//! Startup document picker. No document or agent runtime starts during discovery.
use crate::{
    Error, Result,
    documents::{Limits, Listing, Scan},
    store::Store,
};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    style::{Color, Style},
    widgets::{Block, List, ListItem, ListState, Paragraph},
};
use std::{
    io::{self, IsTerminal},
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), LeaveAlternateScreen, cursor::Show);
        let _ = terminal::disable_raw_mode();
    }
}
struct Search {
    cancel: CancellationToken,
    results: mpsc::Receiver<Result<Listing>>,
}
impl Search {
    fn start(root: PathBuf) -> Self {
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let (sender, results) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let result = Scan::new(&root, true, Limits::default());
            match result {
                Err(error) => {
                    let _ = sender.send(Err(error));
                }
                Ok(mut scan) => {
                    let mut published = Instant::now();
                    loop {
                        let done = scan.step(&token);
                        if done {
                            if !token.is_cancelled() {
                                let _ = sender.send(Ok(scan.listing));
                            }
                            break;
                        }
                        if published.elapsed() >= Duration::from_millis(50) {
                            // Coalesce progress updates and bound queued memory.
                            let _ = sender.try_send(Ok(scan.listing.clone()));
                            published = Instant::now();
                        }
                        if token.is_cancelled() {
                            break;
                        }
                    }
                }
            }
        });
        Self { cancel, results }
    }
}
impl Drop for Search {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

pub fn select(data: Option<PathBuf>) -> Result<Option<PathBuf>> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "TUI requires a terminal on stdin and stdout",
        ));
    }
    let root = std::env::current_dir().map_err(super::terminal::io)?;
    terminal::enable_raw_mode().map_err(super::terminal::io)?;
    let _restore = Restore;
    execute!(io::stdout(), EnterAlternateScreen, cursor::Hide).map_err(super::terminal::io)?;
    let mut terminal =
        Terminal::new(CrosstermBackend::new(io::stdout())).map_err(super::terminal::io)?;
    let mut search = Search::start(root.clone());
    let mut listing = Listing {
        root,
        documents: vec![],
        complete: false,
        stop_reason: None,
        entries_scanned: 0,
        unreadable_directories: 0,
    };
    let mut filter = String::new();
    let mut filtering = false;
    let mut path_input: Option<String> = None;
    let mut selected = 0usize;
    let mut notice = String::new();
    let mut done = false;
    let mut dirty = true;
    let mut visible: Vec<PathBuf> = vec![];
    let mut list_state = ListState::default();
    loop {
        while let Ok(result) = search.results.try_recv() {
            dirty = true;
            match result {
                Ok(mut found) => {
                    let needle = filter.to_lowercase();
                    let previous = listing
                        .documents
                        .iter()
                        .filter(|path| path.to_string_lossy().to_lowercase().contains(&needle))
                        .nth(selected)
                        .cloned();
                    found.documents.sort();
                    if let Some(previous) = previous
                        && let Some(index) = found
                            .documents
                            .iter()
                            .filter(|path| path.to_string_lossy().to_lowercase().contains(&needle))
                            .position(|path| *path == previous)
                    {
                        selected = index;
                    }
                    listing = found;
                    done = listing.complete || listing.stop_reason.is_some();
                }
                Err(error) => {
                    notice = error.to_string();
                    listing.stop_reason = Some("search_error");
                    done = true;
                }
            }
        }
        if dirty {
            let needle = filter.to_lowercase();
            visible = listing
                .documents
                .iter()
                .filter(|path| path.to_string_lossy().to_lowercase().contains(&needle))
                .cloned()
                .collect();
            selected = selected.min(visible.len().saturating_sub(1));
            terminal
                .draw(|frame| {
                    let areas = Layout::vertical([
                        Constraint::Length(3),
                        Constraint::Min(1),
                        Constraint::Length(3),
                    ])
                    .split(frame.area());
                    frame.render_widget(
                        Paragraph::new(listing.root.display().to_string())
                            .block(Block::bordered().title("Open a .pen file")),
                        areas[0],
                    );
                    let items: Vec<_> = visible
                        .iter()
                        .map(|p| ListItem::new(p.to_string_lossy().into_owned()))
                        .collect();
                    list_state.select((!items.is_empty()).then_some(selected));
                    frame.render_stateful_widget(
                        List::new(items)
                            .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White)),
                        areas[1],
                        &mut list_state,
                    );
                    let status = if let Some(reason) = listing.stop_reason {
                        format!(
                            "Search stopped: {reason}; {} files found; unsearched locations remain",
                            listing.documents.len()
                        )
                    } else if done {
                        format!("Search complete: {} files", listing.documents.len())
                    } else {
                        format!("Searching… {} files found", listing.documents.len())
                    };
                    let input = path_input
                        .as_ref()
                        .map(|p| format!("Path: {p}"))
                        .unwrap_or_else(|| {
                            format!(
                                "Filter: {filter}  |  Enter open · Ctrl+L path/root · Ctrl+C quit"
                            )
                        });
                    let message = if notice.is_empty() {
                        status
                    } else {
                        format!("{status} | {notice}")
                    };
                    frame.render_widget(Paragraph::new(format!("{input}\n{message}")), areas[2]);
                    if areas[2].width > 0 && areas[2].height > 0 {
                        let editing = path_input
                            .as_ref()
                            .map(|path| format!("Path: {path}"))
                            .unwrap_or_else(|| format!("Filter: {filter}"));
                        let width = ratatui::text::Line::raw(editing).width();
                        frame.set_cursor_position((
                            areas[2].x
                                + width.min(areas[2].width.saturating_sub(1) as usize) as u16,
                            areas[2].y,
                        ));
                    }
                })
                .map_err(super::terminal::io)?;
            dirty = false;
        }
        if !event::poll(Duration::from_millis(50)).map_err(super::terminal::io)? {
            continue;
        }
        let key = match event::read().map_err(super::terminal::io)? {
            Event::Key(key) => key,
            Event::Resize(_, _) => {
                dirty = true;
                continue;
            }
            _ => continue,
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        dirty = true;
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('c') => return Ok(None),
                KeyCode::Char('l') => {
                    path_input = Some(String::new());
                    notice.clear();
                }
                KeyCode::Char('u') => {
                    if let Some(path) = &mut path_input {
                        path.clear();
                    } else {
                        filter.clear();
                    }
                }
                _ => {}
            }
            continue;
        }
        match key.code {
            KeyCode::Esc => {
                path_input = None;
                filter.clear();
                filtering = false;
                notice.clear();
            }
            KeyCode::Up => selected = selected.saturating_sub(1),
            KeyCode::Down => selected = (selected + 1).min(visible.len().saturating_sub(1)),
            KeyCode::Char('j') if path_input.is_none() && !filtering => {
                selected = (selected + 1).min(visible.len().saturating_sub(1))
            }
            KeyCode::Char('k') if path_input.is_none() && !filtering => {
                selected = selected.saturating_sub(1)
            }
            KeyCode::Backspace => {
                if let Some(path) = &mut path_input {
                    path.pop();
                } else {
                    filter.pop();
                }
                selected = 0;
            }
            KeyCode::Char('/') if path_input.is_none() && !filtering => {
                filtering = true;
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::SUPER) =>
            {
                if let Some(path) = &mut path_input {
                    path.push(c);
                } else {
                    filtering = true;
                    filter.push(c);
                }
                selected = 0;
            }
            KeyCode::Enter => {
                let path = if let Some(input) = &path_input {
                    let path = Path::new(input);
                    if path.is_absolute() {
                        path.to_owned()
                    } else {
                        listing.root.join(path)
                    }
                } else if let Some(path) = visible.get(selected) {
                    listing.root.join(path)
                } else {
                    continue;
                };
                if path.is_dir() {
                    search = Search::start(path.clone());
                    listing = Listing {
                        root: path,
                        documents: vec![],
                        complete: false,
                        stop_reason: None,
                        entries_scanned: 0,
                        unreadable_directories: 0,
                    };
                    path_input = None;
                    filter.clear();
                    filtering = false;
                    notice.clear();
                    done = false;
                    selected = 0;
                    continue;
                }
                match crate::documents::require_pen(&path)
                    .and_then(|()| Store::open(&path, data.clone()))
                    .and_then(|store| store.read().map(|_| store.path().to_owned()))
                {
                    Ok(path) => return Ok(Some(path)),
                    Err(error) => notice = error.to_string(),
                }
            }
            _ => {}
        }
    }
}
