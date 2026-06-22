mod book;
mod html;
mod layout;
mod ui;
mod voice;

use book::EpubBook;
use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use ui::{draw, RenderAssets, ViewState};
use voice::{fetch_speakers, VoiceReader, SPEED_STEP};

#[derive(Parser)]
#[command(name = "bk", about = "TUI EPUB reader with VOICEVOX")]
struct Cli {
    /// EPUB file path
    path: PathBuf,

    /// VOICEVOX ENGINE の URL
    #[arg(long, default_value = "http://127.0.0.1:50021")]
    voicevox_url: String,

    /// VOICEVOX の話者 ID（スタイル ID）
    #[arg(long, default_value_t = 1)]
    speaker: u32,

    /// 読み上げ速度（VOICEVOX speedScale、0.5〜2.0）
    #[arg(long, default_value_t = 1.0)]
    speech_speed: f64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let epub_book = EpubBook::load(&cli.path)?;

    if epub_book.book.chapters.is_empty() {
        eprintln!("この EPUB には読み取り可能な章がありません。");
        return Ok(());
    }

    ratatui::run(|terminal| run_app(terminal, epub_book, &cli))?;
    Ok(())
}

fn run_app(
    terminal: &mut ratatui::DefaultTerminal,
    epub_book: EpubBook,
    cli: &Cli,
) -> Result<(), Box<dyn std::error::Error>> {
    let speakers = fetch_speakers(&cli.voicevox_url).unwrap_or_else(|error| {
        eprintln!("話者一覧の取得に失敗しました: {error}");
        Vec::new()
    });

    let mut state = ViewState::new();
    let mut assets = RenderAssets::new();
    let mut voice = VoiceReader::new(
        &cli.voicevox_url,
        cli.speaker,
        cli.speech_speed,
        speakers,
    );
    let mut last_tick = Instant::now();
    let tick_rate = Duration::from_millis(250);
    let book = &epub_book.book;

    loop {
        let size = terminal.size()?;
        let visible_height = content_height(size.height);
        let width = content_width(size.width);
        let font_size = assets.picker.font_size();
        state.clamp_scroll(book, visible_height, width, font_size);
        state.clamp_read_line(book, width, font_size);

        let voice_status = voice.status();
        let voice_settings = voice.settings_view();
        let highlight = voice.highlight_state();
        terminal.draw(|frame| {
            draw(
                frame,
                &epub_book,
                &mut assets,
                &state,
                &voice_status,
                &voice_settings,
                &highlight,
            )
        })?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or(Duration::ZERO);

        if event::poll(timeout)?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            let content_height = content_height(size.height);
            let content_width = content_width(size.width);
            let font_size = assets.picker.font_size();
            let layout = ContentLayout {
                visible_height: content_height,
                width: content_width,
                font_size,
            };

            if state.show_toc {
                    handle_toc_key(&mut state, book, key.code, key.modifiers);
                } else {
                    match key.code {
                        KeyCode::Char('s') => voice.stop(),
                        KeyCode::Char('v') => {
                            let plan = state.visible_speech_plan(
                                book,
                                content_width,
                                font_size,
                                content_height,
                            );
                            voice.speak_plan(plan);
                        }
                        KeyCode::Char('r') => {
                            let plan =
                                state.chapter_speech_plan(book, content_width, font_size);
                            voice.speak_plan(plan);
                        }
                        KeyCode::Char('[') => voice.prev_speaker(),
                        KeyCode::Char(']') => voice.next_speaker(),
                        KeyCode::Char('-') | KeyCode::Char('_') => {
                            voice.adjust_speed(-SPEED_STEP);
                        }
                        KeyCode::Char('=') | KeyCode::Char('+') => {
                            voice.adjust_speed(SPEED_STEP);
                        }
                        _ => {
                            let voice_nav = matches!(
                                voice.status(),
                                voice::VoiceStatus::Loading | voice::VoiceStatus::Speaking
                            );
                            handle_read_key(
                                &mut state,
                                book,
                                key.code,
                                key.modifiers,
                                layout,
                                voice_nav,
                            );
                        }
                    }
                }

                if key.code == KeyCode::Char('q')
                    || key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    voice.stop();
                    break;
                }
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }
    }

    Ok(())
}

fn content_height(terminal_height: u16) -> usize {
    let area = ratatui::layout::Rect::new(0, 0, 80, terminal_height);
    let layout = Layout::new(
        Direction::Vertical,
        [Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)],
    )
        .split(area);
    layout[1].height.saturating_sub(2) as usize
}

fn content_width(terminal_width: u16) -> usize {
    terminal_width.saturating_sub(2) as usize
}

struct ContentLayout {
    visible_height: usize,
    width: usize,
    font_size: ratatui_image::FontSize,
}

fn handle_read_key(
    state: &mut ViewState,
    book: &book::Book,
    code: KeyCode,
    modifiers: KeyModifiers,
    layout: ContentLayout,
    voice_nav: bool,
) {
    let visible_height = layout.visible_height;
    let width = layout.width;
    let font_size = layout.font_size;
    let scroll_step = if modifiers.contains(KeyModifiers::CONTROL) {
        visible_height / 2
    } else {
        1
    };

    if voice_nav {
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                state.scroll = state.scroll.saturating_add(scroll_step);
                state.clamp_scroll(book, visible_height, width, font_size);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                state.scroll = state.scroll.saturating_sub(scroll_step);
            }
            KeyCode::Char('d') if modifiers.contains(KeyModifiers::CONTROL) => {
                state.scroll = state.scroll.saturating_add(visible_height / 2);
                state.clamp_scroll(book, visible_height, width, font_size);
            }
            KeyCode::Char('u') if modifiers.contains(KeyModifiers::CONTROL) => {
                state.scroll = state.scroll.saturating_sub(visible_height / 2);
            }
            KeyCode::PageDown => {
                state.scroll = state.scroll.saturating_add(visible_height);
                state.clamp_scroll(book, visible_height, width, font_size);
            }
            KeyCode::PageUp => {
                state.scroll = state.scroll.saturating_sub(visible_height);
            }
            KeyCode::Char('n') | KeyCode::Right => state.next_chapter(book),
            KeyCode::Char('p') | KeyCode::Left => state.prev_chapter(),
            KeyCode::Char('g') => state.scroll = 0,
            KeyCode::Char('G') => {
                state.scroll = state.max_scroll(book, visible_height, width, font_size);
            }
            KeyCode::Char('t') => {
                state.show_toc = true;
                state.toc_selected = book
                    .toc
                    .iter()
                    .position(|item| item.chapter_index == state.chapter)
                    .unwrap_or(0);
            }
            _ => {}
        }
        return;
    }

    match code {
        KeyCode::Char('j') | KeyCode::Down => {
            state.move_read_line(book, scroll_step as isize, visible_height, width, font_size);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.move_read_line(book, -(scroll_step as isize), visible_height, width, font_size);
        }
        KeyCode::Char('d') if modifiers.contains(KeyModifiers::CONTROL) => {
            state.move_read_line(
                book,
                (visible_height / 2) as isize,
                visible_height,
                width,
                font_size,
            );
        }
        KeyCode::Char('u') if modifiers.contains(KeyModifiers::CONTROL) => {
            state.move_read_line(
                book,
                -(visible_height as isize / 2),
                visible_height,
                width,
                font_size,
            );
        }
        KeyCode::PageDown => {
            state.move_read_line(
                book,
                visible_height as isize,
                visible_height,
                width,
                font_size,
            );
        }
        KeyCode::PageUp => {
            state.move_read_line(
                book,
                -(visible_height as isize),
                visible_height,
                width,
                font_size,
            );
        }
        KeyCode::Char('n') | KeyCode::Right => state.next_chapter(book),
        KeyCode::Char('p') | KeyCode::Left => state.prev_chapter(),
        KeyCode::Char('g') => {
            state.read_line = 0;
            state.ensure_read_line_visible(book, visible_height, width, font_size);
        }
        KeyCode::Char('G') => {
            let last = state.chapter_line_count(book, width, font_size).saturating_sub(1);
            state.read_line = last;
            state.ensure_read_line_visible(book, visible_height, width, font_size);
        }
        KeyCode::Char('t') => {
            state.show_toc = true;
            state.toc_selected = book
                .toc
                .iter()
                .position(|item| item.chapter_index == state.chapter)
                .unwrap_or(0);
        }
        _ => {}
    }
}

fn handle_toc_key(
    state: &mut ViewState,
    book: &book::Book,
    code: KeyCode,
    modifiers: KeyModifiers,
) {
    match code {
        KeyCode::Esc | KeyCode::Char('t') => state.show_toc = false,
        KeyCode::Up | KeyCode::Char('k') if state.toc_selected > 0 => {
            state.toc_selected -= 1;
        }
        KeyCode::Down | KeyCode::Char('j')
            if state.toc_selected + 1 < book.toc.len() =>
        {
            state.toc_selected += 1;
        }
        KeyCode::Enter => {
            let chapter = book.toc[state.toc_selected].chapter_index;
            state.go_to_chapter(chapter);
            state.show_toc = false;
        }
        KeyCode::Char('q')
            | KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {}
        _ => {}
    }
}
