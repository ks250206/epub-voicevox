use crate::book::{
    ContentBlock, ImageBlock, RichText, SpanStyle, StyledSpan, TableBlock,
};
use crate::book::wrap_text;
use ratatui_image::FontSize;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub const SPEECH_CHUNK_LEN: usize = 150;

#[derive(Clone, Debug)]
pub struct DisplayLine {
    pub global_index: usize,
    pub spans: Vec<StyledSpan>,
    pub plain: String,
}

#[derive(Clone, Debug)]
pub struct SpeechPlan {
    pub normalized_text: String,
    pub chunks: Vec<String>,
    pub chunk_layout_lines: Vec<Vec<usize>>,
}

#[derive(Clone, Debug, Default)]
pub struct HighlightState {
    pub active_chunk: Option<usize>,
    pub highlighted_lines: Vec<usize>,
    pub chunk_layout_lines: Vec<Vec<usize>>,
}

impl HighlightState {
    pub fn from_plan(plan: &SpeechPlan) -> Self {
        Self {
            active_chunk: None,
            highlighted_lines: Vec::new(),
            chunk_layout_lines: plan.chunk_layout_lines.clone(),
        }
    }

    pub fn set_active_chunk(&mut self, chunk_index: usize) {
        self.active_chunk = Some(chunk_index);
        self.highlighted_lines = self
            .chunk_layout_lines
            .get(chunk_index)
            .cloned()
            .unwrap_or_default();
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn is_line_highlighted(&self, global_index: usize) -> bool {
        self.highlighted_lines.contains(&global_index)
    }
}

pub enum LayoutSegment {
    Text(Vec<DisplayLine>),
    Table {
        caption_lines: Vec<DisplayLine>,
        lines: Vec<DisplayLine>,
    },
    Image {
        block: ImageBlock,
        caption_lines: Vec<DisplayLine>,
        cols: u16,
        rows: u16,
    },
}

impl LayoutSegment {
    pub fn height(&self) -> usize {
        match self {
            Self::Text(lines) => lines.len(),
            Self::Table {
                caption_lines,
                lines,
            } => caption_lines.len() + lines.len(),
            Self::Image {
                caption_lines,
                rows,
                ..
            } => caption_lines.len() + *rows as usize + 1,
        }
    }
}

pub fn layout_blocks(
    blocks: &[ContentBlock],
    width: usize,
    font_size: FontSize,
) -> Vec<LayoutSegment> {
    if width == 0 {
        return Vec::new();
    }

    let mut segments = Vec::new();
    let mut global_line = 0;

    for block in blocks {
        match block {
            ContentBlock::Paragraph(text) => {
                push_rich_segment(&mut segments, text, width, &mut global_line);
            }
            ContentBlock::Heading { level, text } => {
                let prefix = "#".repeat(*level as usize);
                let mut spans = vec![StyledSpan::with_style(
                    format!("{prefix} "),
                    SpanStyle {
                        bold: true,
                        ..SpanStyle::default()
                    },
                )];
                spans.extend(text.spans.clone());
                push_spans_segment(&mut segments, &spans, width, &mut global_line);
            }
            ContentBlock::Table(table) => {
                segments.push(layout_table(table, width, &mut global_line));
            }
            ContentBlock::Image(image) => {
                segments.push(layout_image(image, width, font_size, &mut global_line));
            }
        }
    }
    segments
}

pub fn total_height(segments: &[LayoutSegment]) -> usize {
    segments.iter().map(|segment| segment.height()).sum()
}

pub fn build_speech_plan(
    segments: &[LayoutSegment],
    scroll: usize,
    max_lines: usize,
) -> SpeechPlan {
    let flat_lines = flatten_display_lines(segments);
    let end = scroll.saturating_add(max_lines).min(flat_lines.len());
    let slice = if scroll >= flat_lines.len() {
        &[][..]
    } else {
        &flat_lines[scroll..end]
    };

    let mut speech_paragraphs = Vec::new();
    for line in slice {
        if let Some(cleaned) = clean_line_for_speech(&line.plain) {
            speech_paragraphs.push((line.global_index, cleaned));
        }
    }

    let (chunks, chunk_layout_lines) =
        chunk_paragraphs_with_mapping(&speech_paragraphs, SPEECH_CHUNK_LEN);
    let normalized_text = speech_paragraphs
        .iter()
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    SpeechPlan {
        normalized_text,
        chunks,
        chunk_layout_lines,
    }
}

fn flatten_display_lines(segments: &[LayoutSegment]) -> Vec<DisplayLine> {
    let mut lines = Vec::new();
    for segment in segments {
        match segment {
            LayoutSegment::Text(segment_lines) => lines.extend(segment_lines.clone()),
            LayoutSegment::Table {
                caption_lines,
                lines: table_lines,
            } => {
                lines.extend(caption_lines.clone());
                lines.extend(table_lines.clone());
            }
            LayoutSegment::Image { caption_lines, .. } => lines.extend(caption_lines.clone()),
        }
    }
    lines
}

fn clean_line_for_speech(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed
        .chars()
        .all(|ch| matches!(ch, '─' | '│' | '┼' | '-' | '='))
    {
        return None;
    }
    let without_heading = trimmed.trim_start_matches('#').trim();
    let without_image = if let Some(rest) = without_heading.strip_prefix("[画像:") {
        rest.trim_end_matches(']').trim()
    } else {
        without_heading
    };
    let collapsed: String = without_image
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect();
    if collapsed.is_empty() {
        None
    } else {
        Some(collapsed)
    }
}

fn chunk_paragraphs_with_mapping(
    paragraphs: &[(usize, String)],
    max_len: usize,
) -> (Vec<String>, Vec<Vec<usize>>) {
    let mut chunks = Vec::new();
    let mut chunk_layout_lines = Vec::new();
    let mut current = String::new();
    let mut current_lines = Vec::new();

    for (layout_index, paragraph) in paragraphs {
        for sentence in split_sentences(paragraph) {
            if current.len() + sentence.len() > max_len && !current.is_empty() {
                chunks.push(current.clone());
                chunk_layout_lines.push(current_lines.clone());
                current.clear();
                current_lines.clear();
            }
            current.push_str(&sentence);
            if !current_lines.contains(layout_index) {
                current_lines.push(*layout_index);
            }
        }
    }

    if !current.is_empty() {
        chunks.push(current);
        chunk_layout_lines.push(current_lines);
    }

    (chunks, chunk_layout_lines)
}

fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        current.push(ch);
        if matches!(ch, '。' | '！' | '？' | '!' | '?') {
            if !current.is_empty() {
                sentences.push(current.clone());
            }
            current.clear();
        }
    }

    if !current.is_empty() {
        sentences.push(current);
    }

    if sentences.is_empty() {
        sentences.push(text.to_string());
    }

    sentences
}

fn push_rich_segment(
    segments: &mut Vec<LayoutSegment>,
    text: &RichText,
    width: usize,
    global_line: &mut usize,
) {
    push_spans_segment(segments, &text.spans, width, global_line);
}

fn push_spans_segment(
    segments: &mut Vec<LayoutSegment>,
    spans: &[StyledSpan],
    width: usize,
    global_line: &mut usize,
) {
    let display_lines = make_display_lines(spans, width, global_line);
    if display_lines.is_empty() {
        return;
    }

    if let Some(LayoutSegment::Text(existing)) = segments.last_mut() {
        if existing.last().is_some_and(|line| !line.plain.is_empty()) {
            let spacer = DisplayLine {
                global_index: *global_line,
                spans: vec![StyledSpan::plain("")],
                plain: String::new(),
            };
            *global_line += 1;
            existing.push(spacer);
        }
        existing.extend(display_lines);
    } else {
        segments.push(LayoutSegment::Text(display_lines));
    }
}

fn make_display_lines(
    spans: &[StyledSpan],
    width: usize,
    global_line: &mut usize,
) -> Vec<DisplayLine> {
    wrap_spans(spans, width)
        .into_iter()
        .map(|line_spans| {
            let plain = line_spans.iter().map(|span| span.text.as_str()).collect::<String>();
            let line = DisplayLine {
                global_index: *global_line,
                spans: line_spans,
                plain,
            };
            *global_line += 1;
            line
        })
        .collect()
}

fn wrap_spans(spans: &[StyledSpan], width: usize) -> Vec<Vec<StyledSpan>> {
    if width == 0 {
        return Vec::new();
    }

    let mut lines: Vec<Vec<StyledSpan>> = Vec::new();
    let mut current_line: Vec<StyledSpan> = Vec::new();
    let mut current_width = 0;

    for span in spans {
        for ch in span.text.chars() {
            if ch == '\n' {
                if !current_line.is_empty() {
                    lines.push(current_line.clone());
                    current_line.clear();
                    current_width = 0;
                }
                continue;
            }

            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if current_width + ch_width > width && current_width > 0 {
                lines.push(current_line.clone());
                current_line.clear();
                current_width = 0;
            }

            push_char_to_line(&mut current_line, ch, span.style);
            current_width += ch_width;
        }
    }

    if !current_line.is_empty() {
        lines.push(current_line);
    }

    if lines.is_empty() {
        lines.push(vec![StyledSpan::plain("")]);
    }

    lines
}

fn push_char_to_line(line: &mut Vec<StyledSpan>, ch: char, style: SpanStyle) {
    if let Some(last) = line.last_mut()
        && last.style == style
    {
        last.text.push(ch);
        return;
    }
    line.push(StyledSpan::with_style(ch.to_string(), style));
}

fn layout_table(
    table: &TableBlock,
    width: usize,
    global_line: &mut usize,
) -> LayoutSegment {
    let caption_lines = table
        .caption
        .as_deref()
        .map(|caption| make_display_lines_from_plain(caption, width, global_line))
        .unwrap_or_default();

    let lines = format_table_lines(&table.rows, width, global_line);

    LayoutSegment::Table {
        caption_lines,
        lines,
    }
}

fn layout_image(
    image: &ImageBlock,
    width: usize,
    font_size: FontSize,
    global_line: &mut usize,
) -> LayoutSegment {
    let caption_lines = image
        .caption
        .as_deref()
        .map(|caption| make_display_lines_from_plain(caption, width, global_line))
        .unwrap_or_default();
    let (cols, rows) = image_display_size(image, width, font_size);

    LayoutSegment::Image {
        block: image.clone(),
        caption_lines,
        cols,
        rows,
    }
}

fn make_display_lines_from_plain(
    text: &str,
    width: usize,
    global_line: &mut usize,
) -> Vec<DisplayLine> {
    wrap_text(text, width)
        .into_iter()
        .map(|plain| {
            let line = DisplayLine {
                global_index: *global_line,
                spans: vec![StyledSpan::plain(plain.clone())],
                plain,
            };
            *global_line += 1;
            line
        })
        .collect()
}

/// 画像のピクセルサイズからセル数を算出し、ターミナル幅を超える場合のみ縮小する。
fn image_display_size(image: &ImageBlock, max_cols: usize, font_size: FontSize) -> (u16, u16) {
    let (pixel_width, pixel_height) = match (image.pixel_width, image.pixel_height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => (w, h),
        _ => return (max_cols.min(40) as u16, 6),
    };

    let cell_w = font_size.width.max(1);
    let cell_h = font_size.height.max(1);
    let natural_cols = ((pixel_width as f32) / cell_w as f32).ceil() as u16;
    let natural_rows = ((pixel_height as f32) / cell_h as f32).ceil() as u16;

    let max_cols_u16 = max_cols as u16;
    let max_rows = 24;

    if natural_cols <= max_cols_u16 && natural_rows <= max_rows {
        return (natural_cols.max(1), natural_rows.max(1));
    }

    let max_px_w = max_cols_u16 as u32 * cell_w as u32;
    let max_px_h = max_rows as u32 * cell_h as u32;
    let (fit_w, fit_h) = fit_pixels_proportionally(pixel_width, pixel_height, max_px_w, max_px_h);

    let cols = ((fit_w as f32) / cell_w as f32).ceil() as u16;
    let rows = ((fit_h as f32) / cell_h as f32).ceil() as u16;
    (cols.max(1), rows.max(1))
}

fn fit_pixels_proportionally(
    width: u32,
    height: u32,
    max_width: u32,
    max_height: u32,
) -> (u32, u32) {
    let scale_w = max_width as f64 / width as f64;
    let scale_h = max_height as f64 / height as f64;
    let scale = scale_w.min(scale_h);
    if scale >= 1.0 {
        return (width, height);
    }
    (
        (width as f64 * scale).round().max(1.0) as u32,
        (height as f64 * scale).round().max(1.0) as u32,
    )
}

fn format_table_lines(
    rows: &[Vec<String>],
    width: usize,
    global_line: &mut usize,
) -> Vec<DisplayLine> {
    if rows.is_empty() {
        let line = DisplayLine {
            global_index: *global_line,
            spans: vec![StyledSpan::plain("(空の表)".to_string())],
            plain: "(空の表)".to_string(),
        };
        *global_line += 1;
        return vec![line];
    }

    let column_count = rows.iter().map(|row| row.len()).max().unwrap_or(0);
    if column_count == 0 {
        let line = DisplayLine {
            global_index: *global_line,
            spans: vec![StyledSpan::plain("(空の表)".to_string())],
            plain: "(空の表)".to_string(),
        };
        *global_line += 1;
        return vec![line];
    }

    let mut normalized_rows = Vec::new();
    for row in rows {
        let mut cells = row.clone();
        while cells.len() < column_count {
            cells.push(String::new());
        }
        normalized_rows.push(cells);
    }

    let spacing = column_count.saturating_sub(1);
    let available = width.saturating_sub(spacing * 3);
    let col_widths = distribute_column_widths(&normalized_rows, column_count, available);
    let mut lines = Vec::new();

    for (index, row) in normalized_rows.iter().enumerate() {
        let plain = format_row(row, &col_widths);
        lines.push(DisplayLine {
            global_index: *global_line,
            spans: vec![StyledSpan::plain(plain.clone())],
            plain,
        });
        *global_line += 1;
        if index == 0 && normalized_rows.len() > 1 {
            let divider = format_divider(&col_widths);
            lines.push(DisplayLine {
                global_index: *global_line,
                spans: vec![StyledSpan::plain(divider.clone())],
                plain: divider,
            });
            *global_line += 1;
        }
    }

    lines
}

fn distribute_column_widths(rows: &[Vec<String>], column_count: usize, available: usize) -> Vec<usize> {
    if column_count == 1 {
        return vec![available.max(1)];
    }

    let mut max_widths = vec![1; column_count];
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            let cell_width = UnicodeWidthStr::width(cell.as_str());
            max_widths[index] = max_widths[index].max(cell_width.min(available / column_count));
        }
    }

    if column_count == 2 {
        let left = max_widths[0].clamp(8, available.saturating_sub(12).max(8));
        let right = available.saturating_sub(left).max(8);
        return vec![left, right];
    }

    let total = max_widths.iter().sum::<usize>().max(1);
    max_widths
        .iter()
        .map(|width| {
            let scaled = (*width * available) / total;
            scaled.max(4)
        })
        .collect()
}

fn format_row(cells: &[String], widths: &[usize]) -> String {
    let mut parts = Vec::new();
    for (cell, width) in cells.iter().zip(widths) {
        parts.push(pad_or_truncate(cell, *width));
    }
    parts.join(" │ ")
}

fn format_divider(widths: &[usize]) -> String {
    widths
        .iter()
        .map(|width| "─".repeat(*width))
        .collect::<Vec<_>>()
        .join("─┼─")
}

fn pad_or_truncate(text: &str, width: usize) -> String {
    let current = UnicodeWidthStr::width(text);
    if current <= width {
        format!("{text}{}", " ".repeat(width - current))
    } else {
        truncate_to_width(text, width)
    }
}

fn truncate_to_width(text: &str, width: usize) -> String {
    let mut result = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > width.saturating_sub(1) {
            result.push('…');
            break;
        }
        result.push(ch);
        used += ch_width;
    }
    if UnicodeWidthStr::width(result.as_str()) < width {
        let padding = width - UnicodeWidthStr::width(result.as_str());
        result.push_str(&" ".repeat(padding));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui_image::FontSize;

    #[test]
    fn small_image_uses_natural_cell_size() {
        let image = ImageBlock {
            alt: String::new(),
            resource_path: String::new(),
            caption: None,
            pixel_width: Some(32),
            pixel_height: Some(32),
        };
        let (cols, rows) = image_display_size(&image, 80, FontSize::new(8, 16));
        assert!(cols <= 5);
        assert!(rows <= 3);
    }

    #[test]
    fn large_image_is_scaled_down() {
        let image = ImageBlock {
            alt: String::new(),
            resource_path: String::new(),
            caption: None,
            pixel_width: Some(4000),
            pixel_height: Some(3000),
        };
        let (cols, rows) = image_display_size(&image, 80, FontSize::new(8, 16));
        assert!(cols <= 80);
        assert!(rows <= 24);
    }

    #[test]
    fn speech_plan_maps_chunks_to_layout_lines() {
        let long_text = "これは一文です。これも二文目です。".repeat(12);
        let blocks = vec![ContentBlock::Paragraph(RichText::from_plain(long_text))];
        let segments = layout_blocks(&blocks, 40, FontSize::new(8, 16));
        let plan = build_speech_plan(&segments, 0, usize::MAX);
        assert!(plan.chunks.len() > 1);
        assert_eq!(plan.chunk_layout_lines.len(), plan.chunks.len());
        assert!(!plan.chunk_layout_lines[0].is_empty());
    }

    #[test]
    fn chunks_long_text() {
        let paragraphs = vec![(0, "これは一文です。これも二文目です。三つ目の文です。".to_string())];
        let (chunks, _) = chunk_paragraphs_with_mapping(&paragraphs, 20);
        assert!(chunks.len() > 1);
    }

    #[test]
    fn keeps_short_text_in_one_chunk() {
        let paragraphs = vec![(0, "短いテキスト".to_string())];
        let (chunks, _) = chunk_paragraphs_with_mapping(&paragraphs, SPEECH_CHUNK_LEN);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "短いテキスト");
    }

    #[test]
    fn removes_whitespace_for_speech() {
        let cleaned = clean_line_for_speech("　　先頭に　スペース　がある").unwrap();
        assert!(!cleaned.contains(' '));
        assert!(!cleaned.contains('　'));
        assert_eq!(cleaned, "先頭にスペースがある");
    }
}
