use crate::book::{
    ContentBlock, ImageBlock, RichText, SpanStyle, StyledSpan, TableBlock,
};
use path_clean::PathClean;
use scraper::{ElementRef, Html, Node, Selector};
use std::path::{Path, PathBuf};

pub fn parse_chapter(html: &str, document_path: &str) -> Vec<ContentBlock> {
    let document = Html::parse_document(html);
    let body_sel = Selector::parse("body").expect("valid selector");
    let body = document
        .select(&body_sel)
        .next()
        .unwrap_or_else(|| document.root_element());

    let mut blocks = Vec::new();
    walk_element(body, &mut blocks, document_path);
    if blocks.is_empty() {
        blocks.push(ContentBlock::Paragraph(
            RichText::from_plain("(このページにテキストコンテンツはありません)"),
        ));
    }
    blocks
}

pub fn chapter_title(html: &str, document_path: &str, fallback: &str) -> String {
    let document = Html::parse_document(html);
    let title_sel = Selector::parse("title").expect("valid selector");
    if let Some(title) = document.select(&title_sel).next() {
        let text = element_text(title);
        if !text.is_empty() {
            return text;
        }
    }

    for selector in ["h1", "h2", "h3", "p.caption", "p.title"] {
        let sel = Selector::parse(selector).expect("valid selector");
        if let Some(el) = document.select(&sel).next() {
            let text = element_text(el);
            if !text.is_empty() {
                return text.chars().take(80).collect();
            }
        }
    }

    Path::new(document_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| fallback.to_string())
}

fn walk_element(element: ElementRef<'_>, blocks: &mut Vec<ContentBlock>, document_path: &str) {
    for child in element.children() {
        if let Node::Element(_) = child.value() {
            let el = ElementRef::wrap(child).expect("element node");
            match el.value().name() {
                "script" | "style" | "meta" | "link" | "head" => continue,
                "table" => blocks.push(ContentBlock::Table(parse_table(el))),
                "img" => blocks.push(ContentBlock::Image(parse_image(el, document_path, None))),
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    let level = el.value().name()[1..].parse().unwrap_or(1);
                    let text = inline_rich_text(el);
                    if !text.is_empty() {
                        blocks.push(ContentBlock::Heading { level, text });
                    }
                }
                "p" => {
                    let text = inline_rich_text(el);
                    if !text.is_empty() {
                        blocks.push(ContentBlock::Paragraph(text));
                    }
                }
                "hr" => blocks.push(ContentBlock::Paragraph(
                    RichText::from_plain("────────────────────────────────"),
                )),
                "ul" | "ol" => {
                    let li_sel = Selector::parse("li").expect("valid selector");
                    for li in el.select(&li_sel) {
                        let text = inline_rich_text(li);
                        if !text.is_empty() {
                            let mut spans = vec![StyledSpan::plain("• ")];
                            spans.extend(text.spans);
                            blocks.push(ContentBlock::Paragraph(RichText { spans }));
                        }
                    }
                }
                "pre" => {
                    let spans = inline_spans(el, SpanStyle {
                        code: true,
                        ..SpanStyle::default()
                    });
                    let text = RichText { spans };
                    if !text.is_empty() {
                        blocks.push(ContentBlock::Paragraph(text));
                    }
                }
                "code" => {
                    let spans = inline_spans(el, SpanStyle {
                        code: true,
                        ..SpanStyle::default()
                    });
                    let text = RichText { spans };
                    if !text.is_empty() {
                        blocks.push(ContentBlock::Paragraph(text));
                    }
                }
                "div" | "section" | "article" | "figure" | "blockquote" | "aside" => {
                    if is_image_container(el)
                        && let Some(block) = parse_image_container(el, document_path)
                    {
                        blocks.push(block);
                        continue;
                    }
                    walk_element(el, blocks, document_path);
                }
                _ => walk_element(el, blocks, document_path),
            }
        }
    }
}

fn is_image_container(el: ElementRef<'_>) -> bool {
    el.value()
        .attr("class")
        .is_some_and(|class| class.split_whitespace().any(|c| c == "image"))
}

fn parse_image_container(el: ElementRef<'_>, document_path: &str) -> Option<ContentBlock> {
    let img_sel = Selector::parse("img").expect("valid selector");
    let img = el.select(&img_sel).next()?;
    let caption_sel = Selector::parse(".caption, p.caption").expect("valid selector");
    let caption = el
        .select(&caption_sel)
        .next()
        .map(|node| inline_rich_text(node).plain_text())
        .filter(|text| !text.is_empty());
    Some(ContentBlock::Image(parse_image(img, document_path, caption)))
}

fn parse_table(table: ElementRef<'_>) -> TableBlock {
    let caption_sel = Selector::parse("caption").expect("valid selector");
    let caption = table
        .select(&caption_sel)
        .next()
        .map(|node| element_text(node))
        .filter(|text| !text.is_empty());

    let row_sel = Selector::parse("tr").expect("valid selector");
    let cell_sel = Selector::parse("th, td").expect("valid selector");
    let mut rows = Vec::new();

    for row in table.select(&row_sel) {
        let cells: Vec<String> = row
            .select(&cell_sel)
            .map(|cell| cell_text(cell))
            .filter(|text| !text.is_empty())
            .collect();
        if !cells.is_empty() {
            rows.push(cells);
        }
    }

    TableBlock { caption, rows }
}

fn parse_image(
    img: ElementRef<'_>,
    document_path: &str,
    caption: Option<String>,
) -> ImageBlock {
    let src = img.value().attr("src").unwrap_or_default();
    let alt = img
        .value()
        .attr("alt")
        .unwrap_or_default()
        .trim()
        .to_string();
    let resource_path = resolve_resource_path(document_path, src);

    ImageBlock {
        alt,
        resource_path,
        caption,
        pixel_width: None,
        pixel_height: None,
    }
}

fn cell_text(cell: ElementRef<'_>) -> String {
    let img_sel = Selector::parse("img").expect("valid selector");
    if let Some(img) = cell.select(&img_sel).next() {
        let alt = img.value().attr("alt").unwrap_or_default().trim();
        if alt.is_empty() {
            return "[画像]".to_string();
        }
        return format!("[画像: {alt}]");
    }
    element_text(cell)
}

fn inline_rich_text(element: ElementRef<'_>) -> RichText {
    let spans = inline_spans(element, SpanStyle::default());
    RichText { spans }
}

fn inline_spans(element: ElementRef<'_>, base_style: SpanStyle) -> Vec<StyledSpan> {
    let mut spans = Vec::new();
    walk_inline(element, base_style, &mut spans);
    merge_adjacent_spans(&mut spans);
    spans
}

fn walk_inline(node: ElementRef<'_>, style: SpanStyle, spans: &mut Vec<StyledSpan>) {
    for child in node.children() {
        match child.value() {
            Node::Text(text) => {
                let value = text.trim();
                if value.is_empty() {
                    continue;
                }
                spans.push(StyledSpan::with_style(value, style));
            }
            Node::Element(_) => {
                let el = ElementRef::wrap(child).expect("element node");
                let child_style = element_span_style(el, style);
                match el.value().name() {
                    "br" => spans.push(StyledSpan::with_style("\n", style)),
                    "img" => {
                        let alt = el.value().attr("alt").unwrap_or_default().trim();
                        let label = if alt.is_empty() {
                            "[画像]".to_string()
                        } else {
                            format!("[画像: {alt}]")
                        };
                        spans.push(StyledSpan::with_style(label, style));
                    }
                    _ => walk_inline(el, child_style, spans),
                }
            }
            _ => {}
        }
    }
}

fn element_span_style(el: ElementRef<'_>, parent: SpanStyle) -> SpanStyle {
    let mut style = parent;
    match el.value().name() {
        "strong" | "b" => style.bold = true,
        "em" | "i" | "cite" | "dfn" => style.italic = true,
        "code" | "kbd" | "samp" | "tt" => style.code = true,
        "sup" => style.superscript = true,
        "sub" => style.superscript = true,
        "span" | "a" | "small" | "mark" | "abbr" => {}
        _ => {}
    }

    if el.value().name() == "span"
        && let Some(class) = el.value().attr("class")
    {
        for token in class.split_whitespace() {
            if token == "bold" {
                style.bold = true;
            }
            if token == "super" {
                style.superscript = true;
            }
        }
    }

    style
}

fn merge_adjacent_spans(spans: &mut Vec<StyledSpan>) {
    if spans.is_empty() {
        return;
    }

    let mut merged = Vec::with_capacity(spans.len());
    let mut current = spans[0].clone();

    for span in spans.iter().skip(1) {
        if span.style == current.style {
            current.text.push_str(&span.text);
        } else {
            merged.push(current);
            current = span.clone();
        }
    }
    merged.push(current);
    *spans = merged;
}

fn element_text(element: ElementRef<'_>) -> String {
    element
        .text()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

pub fn resolve_resource_path(document_path: &str, href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_string();
    }

    let doc_parent = Path::new(document_path).parent().unwrap_or(Path::new(""));
    let joined = doc_parent.join(href);
    normalize_epub_path(joined.clean())
}

fn normalize_epub_path(path: PathBuf) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    normalized.trim_start_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_image_paths() {
        let resolved = resolve_resource_path("item/xhtml/p-003.xhtml", "../image/maar_0101.jpg");
        assert_eq!(resolved, "item/image/maar_0101.jpg");
    }

    #[test]
    fn parses_table_blocks() {
        let html = r#"<html><body>
            <p class="caption">表1-1 サンプル</p>
            <table><tr><td>列A</td><td>列B</td></tr></table>
        </body></html>"#;
        let blocks = parse_chapter(html, "item/xhtml/p-003.xhtml");
        assert!(blocks.iter().any(|b| matches!(b, ContentBlock::Table(_))));
    }

    #[test]
    fn parses_inline_emphasis() {
        let html = r#"<html><body><p>通常<strong>太字</strong>と<em>斜体</em></p></body></html>"#;
        let blocks = parse_chapter(html, "item/xhtml/p.xhtml");
        let paragraph = blocks
            .iter()
            .find_map(|block| match block {
                ContentBlock::Paragraph(text) => Some(text),
                _ => None,
            })
            .expect("paragraph");
        assert!(paragraph.spans.iter().any(|span| span.style.bold));
        assert!(paragraph.spans.iter().any(|span| span.style.italic));
    }
}
