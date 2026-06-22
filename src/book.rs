use crate::html::{chapter_title, parse_chapter};
use image::ImageDecoder;
use rbook::Epub;
use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;

pub use crate::layout::{layout_blocks, total_height};

pub struct TocItem {
    pub label: String,
    pub chapter_index: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpanStyle {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub superscript: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StyledSpan {
    pub text: String,
    pub style: SpanStyle,
}

impl StyledSpan {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: SpanStyle::default(),
        }
    }

    pub fn with_style(text: impl Into<String>, style: SpanStyle) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RichText {
    pub spans: Vec<StyledSpan>,
}

impl RichText {
    pub fn from_plain(text: impl Into<String>) -> Self {
        Self {
            spans: vec![StyledSpan::plain(text)],
        }
    }

    pub fn plain_text(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.plain_text().trim().is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct TableBlock {
    pub caption: Option<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Clone, Debug)]
pub struct ImageBlock {
    pub alt: String,
    pub resource_path: String,
    pub caption: Option<String>,
    pub pixel_width: Option<u32>,
    pub pixel_height: Option<u32>,
}

#[derive(Clone, Debug)]
pub enum ContentBlock {
    Paragraph(RichText),
    Heading { level: u8, text: RichText },
    Table(TableBlock),
    Image(ImageBlock),
}

pub struct Chapter {
    pub title: String,
    pub blocks: Vec<ContentBlock>,
}

pub struct Book {
    pub title: String,
    pub chapters: Vec<Chapter>,
    pub toc: Vec<TocItem>,
}

pub struct EpubBook {
    pub book: Book,
    epub: Epub,
}

impl EpubBook {
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let epub = Epub::open(path)?;
        let book = build_book(&epub, path)?;
        Ok(Self { book, epub })
    }

    pub fn read_resource_bytes(&self, path: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        if let Ok(bytes) = self.epub.read_resource_bytes(path) {
            return Ok(bytes);
        }
        if path.starts_with('/') {
            return Ok(self.epub.read_resource_bytes(path.trim_start_matches('/'))?);
        }
        Ok(self.epub.read_resource_bytes(format!("/{path}"))?)
    }
}

fn build_book(epub: &Epub, path: &Path) -> Result<Book, Box<dyn std::error::Error>> {
    let title = epub
        .metadata()
        .title()
        .map(|t| t.value().to_string())
        .unwrap_or_else(|| path.file_stem().unwrap_or_default().to_string_lossy().to_string());

    let reader = epub.reader();
    let len = reader.len();
    let mut chapters = Vec::with_capacity(len);
    let mut idref_to_index = HashMap::new();
    let mut resource_to_index = HashMap::new();

    for i in 0..len {
        let content = reader.get(i)?;
        let idref = content.spine_entry().idref().to_string();
        let resource_key = resource_key_string(content.manifest_entry().resource().key());
        let html = content.content();
        let mut blocks = parse_chapter(html, &resource_key);
        enrich_image_blocks(epub, &mut blocks);
        let chapter_title = chapter_title(html, &resource_key, &idref);

        idref_to_index.insert(idref.clone(), chapters.len());
        resource_to_index.insert(normalize_href(&resource_key), chapters.len());

        chapters.push(Chapter {
            title: chapter_title,
            blocks,
        });
    }

    let mut toc = Vec::new();
    if let Some(root) = epub.toc().contents() {
        for entry in root.flatten() {
            let label = entry.label().trim();
            if label.is_empty() {
                continue;
            }

            let chapter_index = entry
                .manifest_entry()
                .map(|m| resource_key_string(m.resource().key()))
                .or_else(|| entry.resource().map(|r| resource_key_string(r.key())))
                .and_then(|key| {
                    resource_to_index
                        .get(&normalize_href(&key))
                        .copied()
                        .or_else(|| idref_to_index.get(&key).copied())
                });

            if let Some(idx) = chapter_index {
                toc.push(TocItem {
                    label: label.to_string(),
                    chapter_index: idx,
                });
            }
        }
    }

    if toc.is_empty() {
        for (i, chapter) in chapters.iter().enumerate() {
            toc.push(TocItem {
                label: chapter.title.clone(),
                chapter_index: i,
            });
        }
    }

    Ok(Book {
        title,
        chapters,
        toc,
    })
}

fn resource_key_string(key: &rbook::ebook::resource::ResourceKey<'_>) -> String {
    match key {
        rbook::ebook::resource::ResourceKey::Value(path) => path.to_string(),
        rbook::ebook::resource::ResourceKey::Position(pos) => pos.to_string(),
    }
}

fn normalize_href(href: &str) -> String {
    href.split('#').next().unwrap_or(href).to_string()
}

fn enrich_image_blocks(epub: &Epub, blocks: &mut [ContentBlock]) {
    for block in blocks {
        if let ContentBlock::Image(image) = block
            && let Some((width, height)) = read_image_dimensions(epub, &image.resource_path)
        {
            image.pixel_width = Some(width);
            image.pixel_height = Some(height);
        }
    }
}

fn read_image_dimensions(epub: &Epub, path: &str) -> Option<(u32, u32)> {
    let bytes = epub
        .read_resource_bytes(path)
        .or_else(|_| epub.read_resource_bytes(path.trim_start_matches('/')))
        .or_else(|_| epub.read_resource_bytes(format!("/{path}")))
        .ok()?;
    Some(
        image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .ok()?
            .into_decoder()
            .ok()?
            .dimensions(),
    )
}

pub fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }

    let mut lines = Vec::new();

    for paragraph in text.lines() {
        let trimmed = paragraph.trim();
        if trimmed.is_empty() {
            if lines.last().is_some_and(|l: &String| !l.is_empty()) {
                lines.push(String::new());
            }
            continue;
        }
        wrap_paragraph(trimmed, width, &mut lines);
    }

    if lines.is_empty() {
        lines.push(String::new());
    }

    lines
}

fn wrap_paragraph(text: &str, width: usize, lines: &mut Vec<String>) {
    let mut current = String::new();
    let mut current_width = 0;

    for ch in text.chars() {
        let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if current_width + ch_width > width && !current.is_empty() {
            lines.push(current);
            current = String::new();
            current_width = 0;
        }
        current.push(ch);
        current_width += ch_width;
    }

    if !current.is_empty() {
        lines.push(current);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample_epub() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("マスタリングAPIアーキテクチャ.epub")
    }

    #[test]
    fn loads_epub_with_chapters() {
        let epub_book = EpubBook::load(&sample_epub()).expect("epub should load");
        let book = &epub_book.book;
        assert!(!book.title.is_empty());
        assert!(!book.chapters.is_empty());
        assert!(!book.toc.is_empty());
    }

    #[test]
    fn loads_tables_and_images() {
        let epub_book = EpubBook::load(&sample_epub()).expect("epub should load");
        let has_table = epub_book
            .book
            .chapters
            .iter()
            .any(|chapter| chapter.blocks.iter().any(|block| matches!(block, ContentBlock::Table(_))));
        let has_image = epub_book
            .book
            .chapters
            .iter()
            .any(|chapter| chapter.blocks.iter().any(|block| matches!(block, ContentBlock::Image(_))));
        assert!(has_table);
        assert!(has_image);
    }

    #[test]
    fn wraps_japanese_text() {
        let lines = wrap_text("マスタリングAPIアーキテクチャ", 10);
        assert!(lines.len() > 1);
    }

    #[test]
    fn reads_embedded_images() {
        let epub_book = EpubBook::load(&sample_epub()).expect("epub should load");
        let bytes = epub_book
            .read_resource_bytes("item/image/maar_0101.jpg")
            .expect("image bytes");
        assert!(!bytes.is_empty());
    }

    #[test]
    fn figure_images_have_pixel_dimensions() {
        let epub_book = EpubBook::load(&sample_epub()).expect("epub should load");
        let figure = epub_book
            .book
            .chapters
            .iter()
            .flat_map(|chapter| chapter.blocks.iter())
            .find_map(|block| match block {
                ContentBlock::Image(image) if image.resource_path.contains("maar_") => {
                    Some(image.clone())
                }
                _ => None,
            })
            .expect("figure image");
        assert!(figure.pixel_width.unwrap() > 0);
        assert!(figure.pixel_height.unwrap() > 0);
    }
}
