use crate::book::{Book, EpubBook, StyledSpan};
use crate::layout::{
    build_speech_plan, DisplayLine, HighlightState, LayoutSegment, SpeechPlan,
};
use crate::voice::{VoiceSettingsView, VoiceStatus};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;
use ratatui_image::{Image, Resize};
use std::collections::HashMap;

struct LineHighlight<'a> {
    voice_status: &'a VoiceStatus,
    read_line: usize,
    highlight: &'a HighlightState,
    is_table: bool,
}

pub struct RenderAssets {
    pub picker: Picker,
    pub image_cache: HashMap<String, Protocol>,
}

impl RenderAssets {
    pub fn new() -> Self {
        let picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
        Self {
            picker,
            image_cache: HashMap::new(),
        }
    }

    pub fn image_protocol(
        &mut self,
        epub: &EpubBook,
        path: &str,
        cols: u16,
        rows: u16,
    ) -> Option<&Protocol> {
        let cache_key = format!("{path}:{cols}x{rows}");
        if self.image_cache.contains_key(&cache_key) {
            return self.image_cache.get(&cache_key);
        }

        let bytes = epub.read_resource_bytes(path).ok()?;
        let image = image::load_from_memory(&bytes).ok()?;
        let size = ratatui::layout::Size::new(cols.max(1), rows.max(1));
        let protocol = self
            .picker
            .new_protocol(image, size, Resize::Fit(None))
            .ok()?;
        self.image_cache.insert(cache_key.clone(), protocol);
        self.image_cache.get(&cache_key)
    }
}

pub struct ViewState {
    pub chapter: usize,
    pub scroll: usize,
    /// 読み上げ開始行（章内の global line index）
    pub read_line: usize,
    pub show_toc: bool,
    pub toc_selected: usize,
}

impl ViewState {
    pub fn new() -> Self {
        Self {
            chapter: 0,
            scroll: 0,
            read_line: 0,
            show_toc: false,
            toc_selected: 0,
        }
    }

    pub fn chapter_line_count(
        &self,
        book: &Book,
        width: usize,
        font_size: ratatui_image::FontSize,
    ) -> usize {
        let segments = self.chapter_segments(book, width, font_size);
        crate::book::total_height(&segments)
    }

    pub fn clamp_read_line(
        &mut self,
        book: &Book,
        width: usize,
        font_size: ratatui_image::FontSize,
    ) {
        let total = self.chapter_line_count(book, width, font_size);
        if total == 0 {
            self.read_line = 0;
            return;
        }
        self.read_line = self.read_line.min(total - 1);
    }

    pub fn ensure_read_line_visible(
        &mut self,
        book: &Book,
        visible_height: usize,
        width: usize,
        font_size: ratatui_image::FontSize,
    ) {
        if visible_height == 0 {
            self.clamp_scroll(book, visible_height, width, font_size);
            return;
        }
        if self.read_line < self.scroll {
            self.scroll = self.read_line;
        } else if self.read_line >= self.scroll + visible_height {
            self.scroll = self.read_line + 1 - visible_height;
        }
        self.clamp_scroll(book, visible_height, width, font_size);
    }

    pub fn move_read_line(
        &mut self,
        book: &Book,
        delta: isize,
        visible_height: usize,
        width: usize,
        font_size: ratatui_image::FontSize,
    ) {
        if delta.is_negative() {
            self.read_line = self.read_line.saturating_sub(delta.unsigned_abs());
        } else {
            self.read_line = self.read_line.saturating_add(delta as usize);
        }
        self.clamp_read_line(book, width, font_size);
        self.ensure_read_line_visible(book, visible_height, width, font_size);
    }

    pub fn chapter_segments(
        &self,
        book: &Book,
        width: usize,
        font_size: ratatui_image::FontSize,
    ) -> Vec<LayoutSegment> {
        crate::book::layout_blocks(&book.chapters[self.chapter].blocks, width, font_size)
    }

    pub fn max_scroll(
        &self,
        book: &Book,
        visible_height: usize,
        width: usize,
        font_size: ratatui_image::FontSize,
    ) -> usize {
        let segments = self.chapter_segments(book, width, font_size);
        crate::book::total_height(&segments).saturating_sub(visible_height)
    }

    pub fn clamp_scroll(
        &mut self,
        book: &Book,
        visible_height: usize,
        width: usize,
        font_size: ratatui_image::FontSize,
    ) {
        let max = self.max_scroll(book, visible_height, width, font_size);
        self.scroll = self.scroll.min(max);
    }

    pub fn go_to_chapter(&mut self, index: usize) {
        self.chapter = index;
        self.scroll = 0;
        self.read_line = 0;
    }

    pub fn next_chapter(&mut self, book: &Book) {
        if self.chapter + 1 < book.chapters.len() {
            self.chapter += 1;
            self.scroll = 0;
            self.read_line = 0;
        }
    }

    pub fn visible_speech_plan(
        &self,
        book: &Book,
        width: usize,
        font_size: ratatui_image::FontSize,
        visible_height: usize,
    ) -> SpeechPlan {
        let segments = self.chapter_segments(book, width, font_size);
        build_speech_plan(&segments, self.read_line, visible_height)
    }

    pub fn chapter_speech_plan(
        &self,
        book: &Book,
        width: usize,
        font_size: ratatui_image::FontSize,
    ) -> SpeechPlan {
        let segments = self.chapter_segments(book, width, font_size);
        build_speech_plan(&segments, self.read_line, usize::MAX)
    }

    pub fn prev_chapter(&mut self) {
        if self.chapter > 0 {
            self.chapter -= 1;
            self.scroll = 0;
            self.read_line = 0;
        }
    }
}

pub fn draw(
    frame: &mut Frame,
    epub: &EpubBook,
    assets: &mut RenderAssets,
    state: &ViewState,
    voice_status: &VoiceStatus,
    voice_settings: &VoiceSettingsView,
    highlight: &HighlightState,
) {
    let book = &epub.book;
    let area = frame.area();

    let layout = Layout::new(
        Direction::Vertical,
        [Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)],
    )
        .split(area);

    draw_header(frame, layout[0], book, state);
    draw_content(frame, layout[1], epub, assets, state, voice_status, highlight);
    draw_footer(frame, layout[2], state, voice_status, voice_settings, highlight);
}

fn draw_header(frame: &mut Frame, area: Rect, book: &Book, state: &ViewState) {
    let chapter = &book.chapters[state.chapter];
    let chapter_info = format!(
        "第 {} / {} 章: {}",
        state.chapter + 1,
        book.chapters.len(),
        chapter.title
    );

    let header = Line::from(vec![
        Span::styled(book.title.clone(), Style::new().bold()),
        Span::raw("  "),
        Span::styled(chapter_info, Style::new().dim()),
    ]);

    frame.render_widget(Paragraph::new(header), area);
}

fn draw_footer(
    frame: &mut Frame,
    area: Rect,
    state: &ViewState,
    voice_status: &VoiceStatus,
    voice_settings: &VoiceSettingsView,
    highlight: &HighlightState,
) {
    let help = if state.show_toc {
        "↑↓: 選択  Enter: 移動  t/Esc: 閉じる  q: 終了"
    } else {
        "j/k:スクロール v/r:読上 s:停止 [/]:話者 -/=:速度  q:終了"
    };

    let label = truncate_label(&voice_settings.speaker_label, 16);
    let voice_info = format!(
        "話者:{} {} {:.1}x",
        voice_settings.speaker_id,
        label,
        voice_settings.speed
    );

    let status = match voice_status {
        VoiceStatus::Idle => None,
        VoiceStatus::Loading => Some("[読上] 生成中...".to_string()),
        VoiceStatus::Speaking => {
            if let Some(chunk) = highlight.active_chunk {
                Some(format!("[読上] 再生中 #{}/{}", chunk + 1, highlight.chunk_layout_lines.len()))
            } else {
                Some("[読上] 再生中".to_string())
            }
        }
        VoiceStatus::Error(message) => Some(message.clone()),
    };

    let mut line = Line::from(vec![
        Span::raw(help),
        Span::raw("  "),
        Span::styled(voice_info, Style::new().fg(Color::Cyan)),
    ]);
    if let Some(status_text) = status {
        line.spans.push(Span::raw("  "));
        let style = if matches!(voice_status, VoiceStatus::Error(_)) {
            Style::new().fg(Color::Red)
        } else {
            Style::new().fg(Color::Green)
        };
        line.spans.push(Span::styled(status_text, style));
    }

    frame.render_widget(Paragraph::new(line).style(Style::new().dim()), area);
}

fn draw_content(
    frame: &mut Frame,
    area: Rect,
    epub: &EpubBook,
    assets: &mut RenderAssets,
    state: &ViewState,
    voice_status: &VoiceStatus,
    highlight: &HighlightState,
) {
    let book = &epub.book;
    if state.show_toc {
        draw_toc(frame, area, book, state);
        return;
    }

    let inner = Block::new().borders(Borders::ALL).inner(area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let width = inner.width as usize;
    let font_size = assets.picker.font_size();
    let segments = state.chapter_segments(book, width, font_size);
    let mut skip = state.scroll;
    let mut y = inner.y;
    let bottom = inner.bottom();

    let line_style = LineHighlight {
        voice_status,
        read_line: state.read_line,
        highlight,
        is_table: false,
    };

    for segment in segments {
        let height = segment.height();
        if skip >= height {
            skip -= height;
            continue;
        }

        let remaining_rows = bottom.saturating_sub(y);
        if remaining_rows == 0 {
            break;
        }

        let segment_skip = skip;
        skip = 0;

        match segment {
            LayoutSegment::Text(lines) => {
                let visible_len = render_display_lines(
                    frame,
                    inner,
                    y,
                    remaining_rows,
                    &lines,
                    segment_skip,
                    &line_style,
                );
                y += visible_len;
            }
            LayoutSegment::Table {
                caption_lines,
                lines,
            } => {
                let mut table_lines = caption_lines.clone();
                table_lines.extend(lines);
                let table_style = LineHighlight {
                    voice_status,
                    read_line: state.read_line,
                    highlight,
                    is_table: true,
                };
                let visible_len = render_display_lines(
                    frame,
                    inner,
                    y,
                    remaining_rows,
                    &table_lines,
                    segment_skip,
                    &table_style,
                );
                y += visible_len;
            }
            LayoutSegment::Image {
                block,
                caption_lines,
                cols,
                rows,
            } => {
                let caption_skip = segment_skip.min(caption_lines.len());
                let caption_remaining = caption_lines.len().saturating_sub(caption_skip);
                let caption_visible_rows = caption_remaining.min(remaining_rows as usize);
                if caption_visible_rows > 0 {
                    let visible_caption = caption_lines
                        .iter()
                        .skip(caption_skip)
                        .take(caption_visible_rows)
                        .collect::<Vec<_>>();
                    let visible_len = render_display_line_slice(
                        frame,
                        inner,
                        y,
                        caption_visible_rows as u16,
                        &visible_caption,
                        &line_style,
                    );
                    y += visible_len;
                }

                let image_skip = segment_skip.saturating_sub(caption_lines.len());
                let image_rows_available = bottom.saturating_sub(y);
                if image_rows_available == 0 {
                    continue;
                }

                let image_rows = (rows as usize)
                    .saturating_sub(image_skip)
                    .min(image_rows_available as usize) as u16;
                if image_rows == 0 {
                    continue;
                }

                let image_cols = cols.min(inner.width);
                let x = inner.x + (inner.width.saturating_sub(image_cols)) / 2;
                let image_rect = Rect::new(x, y, image_cols, image_rows);
                if let Some(protocol) = assets.image_protocol(
                    epub,
                    &block.resource_path,
                    cols,
                    rows,
                ) {
                    let widget = Image::new(protocol).allow_clipping(true);
                    frame.render_widget(widget, image_rect);
                } else {
                    let fallback = if block.alt.is_empty() {
                        format!("[画像: {}]", block.resource_path)
                    } else {
                        format!("[画像: {}]", block.alt)
                    };
                    frame.render_widget(
                        Paragraph::new(fallback).style(Style::new().fg(Color::Yellow)),
                        image_rect,
                    );
                }
                y += image_rows;
            }
        }
    }

    frame.render_widget(Block::new().borders(Borders::ALL), area);
}

fn render_display_lines(
    frame: &mut Frame,
    inner: Rect,
    y: u16,
    remaining_rows: u16,
    lines: &[DisplayLine],
    skip: usize,
    style: &LineHighlight<'_>,
) -> u16 {
    let visible = lines.iter().skip(skip).take(remaining_rows as usize).collect::<Vec<_>>();
    render_display_line_slice(frame, inner, y, remaining_rows, &visible, style)
}

fn render_display_line_slice(
    frame: &mut Frame,
    inner: Rect,
    y: u16,
    max_rows: u16,
    lines: &[&DisplayLine],
    style: &LineHighlight<'_>,
) -> u16 {
    let visible: Vec<Line> = lines
        .iter()
        .take(max_rows as usize)
        .map(|line| display_line_to_ratatui(line, style))
        .collect();
    let visible_len = visible.len() as u16;
    if visible_len == 0 {
        return 0;
    }
    let rect = Rect::new(inner.x, y, inner.width, visible_len);
    frame.render_widget(Paragraph::new(visible), rect);
    visible_len
}

fn display_line_to_ratatui(line: &DisplayLine, style: &LineHighlight<'_>) -> Line<'static> {
    let line_highlighted = style.highlight.is_line_highlighted(line.global_index)
        || (shows_read_cursor(style.voice_status) && line.global_index == style.read_line);

    if style.is_table && (line.plain.contains('│') || line.plain.contains('┼')) {
        let style = table_line_style(line_highlighted);
        return Line::from(Span::styled(line.plain.clone(), style));
    }

    Line::from(
        line.spans
            .iter()
            .map(|span| styled_span_to_ratatui(span, line_highlighted))
            .collect::<Vec<_>>(),
    )
}

fn shows_read_cursor(voice_status: &VoiceStatus) -> bool {
    !matches!(voice_status, VoiceStatus::Speaking)
}

/// 再生中行・読み上げ開始行: 黄背景＋黒文字で強調する。
fn with_reading_highlight(base: Style, line_highlighted: bool) -> Style {
    if line_highlighted {
        Style::new()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        base
    }
}

fn table_line_style(line_highlighted: bool) -> Style {
    with_reading_highlight(Style::new().fg(Color::Cyan), line_highlighted)
}

fn styled_span_to_ratatui(span: &StyledSpan, line_highlighted: bool) -> Span<'static> {
    let mut style = Style::new();
    if span.style.bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    if span.style.italic {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if span.style.code {
        style = style.fg(Color::Green);
    }
    if span.style.superscript {
        style = style.dim();
    }

    Span::styled(
        span.text.clone(),
        with_reading_highlight(style, line_highlighted),
    )
}

fn draw_toc(frame: &mut Frame, area: Rect, book: &Book, state: &ViewState) {
    let items: Vec<ListItem> = book
        .toc
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let prefix = if i == state.toc_selected {
                Span::styled("▸ ", Style::new().fg(Color::Yellow).bold())
            } else {
                Span::raw("  ")
            };
            let label = if item.chapter_index == state.chapter {
                Span::styled(item.label.clone(), Style::new().fg(Color::Cyan))
            } else {
                Span::raw(item.label.clone())
            };
            ListItem::new(Line::from(vec![prefix, label]))
        })
        .collect();

    let list = List::new(items).block(
        Block::new()
            .title("目次")
            .borders(Borders::ALL)
            .border_style(Style::new().fg(Color::Yellow)),
    );

    frame.render_widget(list, area);
}

fn truncate_label(label: &str, max_chars: usize) -> String {
    if label.chars().count() <= max_chars {
        return label.to_string();
    }
    label.chars().take(max_chars).collect::<String>() + "…"
}
