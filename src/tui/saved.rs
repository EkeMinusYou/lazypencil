//! Read-only saved conversation browsing on the shared background task.

use super::checkpoints::{Output, Task};
use crate::{Error, Result, conversation::history, store::Store};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;
use std::borrow::Cow;

const PAGE_BYTES: usize = 32 * 1024;
const MAX_TEXT_PAGES: usize = 8192;

#[derive(Clone, PartialEq, Eq)]
pub enum Location {
    Sessions,
    Turns(String),
    Text {
        session: String,
        turn: String,
        raw: bool,
    },
}

#[derive(Clone)]
pub struct Page {
    pub location: Location,
    pub data: Value,
    pub selected: usize,
    pub scroll: u16,
    pub maximum_scroll: u16,
    pub notice: Option<String>,
    cursor: Option<String>,
    trail: Vec<Option<String>>,
    back: Option<Box<Page>>,
    current: String,
}

pub enum Action {
    Keep,
    Close,
    Show(Box<Page>),
    Read(Box<Page>),
}
impl Action {
    fn read(page: Page) -> Self {
        Self::Read(Box::new(page))
    }
}

impl Page {
    pub fn sessions(current: String) -> Self {
        Self::new(current, Location::Sessions, None, None)
    }
    pub fn current(current: String) -> Self {
        Self::new(current.clone(), Location::Turns(current), None, None)
    }
    fn new(
        current: String,
        location: Location,
        back: Option<Box<Page>>,
        cursor: Option<String>,
    ) -> Self {
        let trail = if matches!(location, Location::Text { .. }) {
            vec![cursor.clone()]
        } else {
            vec![]
        };
        Self {
            location,
            data: Value::Null,
            selected: 0,
            scroll: 0,
            maximum_scroll: 0,
            notice: None,
            cursor,
            trail,
            back,
            current,
        }
    }
    pub fn task(mut self, store: Store) -> Result<Task> {
        Task::spawn("saved-conversation", move |cancel| {
            self.data = match &self.location {
                Location::Sessions => {
                    history::list(&store, None, 50, self.cursor.as_deref(), Some(cancel))?
                }
                Location::Turns(session) => history::list(
                    &store,
                    Some(session),
                    50,
                    self.cursor.as_deref(),
                    Some(cancel),
                )?,
                Location::Text {
                    session,
                    turn,
                    raw: false,
                } => history::view(
                    &store,
                    session,
                    turn,
                    self.cursor.as_deref(),
                    PAGE_BYTES,
                    Some(cancel),
                )?,
                Location::Text {
                    session,
                    turn,
                    raw: true,
                } => {
                    let (offset, revision) = if let Some(cursor) = &self.cursor {
                        let (offset, revision) = cursor.split_once(':').ok_or_else(|| {
                            Error::new("INVALID_ARGUMENT", "Invalid raw transcript cursor")
                        })?;
                        (
                            offset.parse::<u64>().map_err(|_| {
                                Error::new("INVALID_ARGUMENT", "Invalid raw transcript cursor")
                            })?,
                            Some(revision),
                        )
                    } else {
                        (0, None)
                    };
                    history::read(
                        &store,
                        session,
                        turn,
                        offset,
                        PAGE_BYTES,
                        revision,
                        Some(cancel),
                    )?
                }
            };
            if self.location == Location::Sessions
                && self.cursor.is_none()
                && let Some(entries) = self.data["entries"].as_array()
                && let Some(index) = entries.iter().position(|entry| entry["id"] == self.current)
            {
                self.selected = index;
            }
            Ok(Output::Saved(Box::new(self)))
        })
    }
    fn entries(&self) -> &[Value] {
        self.data["entries"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
    fn fresh(&self, cursor: Option<String>) -> Self {
        let mut page = self.clone();
        page.data = Value::Null;
        page.cursor = cursor;
        page.selected = 0;
        page.scroll = 0;
        page.maximum_scroll = 0;
        page.notice = None;
        page
    }
    fn sessions_parent(&self) -> Option<Box<Page>> {
        let mut page = Some(self);
        while let Some(value) = page {
            if value.location == Location::Sessions {
                return Some(Box::new(value.clone()));
            }
            page = value.back.as_deref();
        }
        None
    }
    fn turns_parent(&self) -> Option<Box<Page>> {
        let mut page = Some(self);
        while let Some(value) = page {
            if matches!(value.location, Location::Turns(_)) {
                return Some(Box::new(value.clone()));
            }
            page = value.back.as_deref();
        }
        None
    }
    fn next(&self) -> Option<String> {
        if matches!(self.location, Location::Text { raw: true, .. }) {
            self.data["next_offset"]
                .as_u64()
                .map(|offset| format!("{offset}:{}", self.data["revision"].as_str().unwrap_or("")))
        } else {
            self.data["next_cursor"].as_str().map(str::to_owned)
        }
    }
    pub fn key(&mut self, key: KeyEvent) -> Result<Action> {
        if key.modifiers.intersects(
            KeyModifiers::CONTROL
                | KeyModifiers::ALT
                | KeyModifiers::SUPER
                | KeyModifiers::HYPER
                | KeyModifiers::META,
        ) {
            return Ok(Action::Keep);
        }
        let text = matches!(self.location, Location::Text { .. });
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return Ok(Action::Close),
            KeyCode::Backspace => {
                if let Some(back) = &self.back {
                    return Ok(Action::Show(back.clone()));
                }
                return Ok(if self.location == Location::Sessions {
                    Action::Close
                } else {
                    Action::read(Self::sessions(self.current.clone()))
                });
            }
            KeyCode::Char('s') => return Ok(Action::read(Self::sessions(self.current.clone()))),
            KeyCode::Char('c') => {
                return Ok(Action::read(Self::new(
                    self.current.clone(),
                    Location::Turns(self.current.clone()),
                    self.sessions_parent(),
                    None,
                )));
            }
            KeyCode::Up | KeyCode::Char('k') if !text => {
                self.selected = self.selected.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') if !text => {
                self.selected = (self.selected + 1).min(self.entries().len().saturating_sub(1))
            }
            KeyCode::Enter if !text => {
                let Some(entry) = self.entries().get(self.selected) else {
                    return Ok(Action::Keep);
                };
                if entry["unavailable"] == true {
                    self.notice = Some(super::clean(&entry["error"].to_string()));
                    return Ok(Action::Keep);
                }
                let Some(id) = entry["id"].as_str() else {
                    return Ok(Action::Keep);
                };
                let location = match &self.location {
                    Location::Sessions => Location::Turns(id.to_owned()),
                    Location::Turns(session) => Location::Text {
                        session: session.clone(),
                        turn: id.to_owned(),
                        raw: false,
                    },
                    _ => unreachable!(),
                };
                return Ok(Action::read(Self::new(
                    self.current.clone(),
                    location,
                    Some(Box::new(self.clone())),
                    None,
                )));
            }
            KeyCode::Home if !text => return Ok(Action::read(self.fresh(None))),
            KeyCode::Char('r') | KeyCode::Char('g') => {
                let mut page = self.fresh(None);
                if text {
                    page.trail = vec![None];
                }
                return Ok(Action::read(page));
            }
            KeyCode::Char('n') | KeyCode::Right | KeyCode::PageDown if !text => {
                if let Some(next) = self.next() {
                    return Ok(Action::read(self.fresh(Some(next))));
                }
            }
            KeyCode::Char('p') | KeyCode::Left | KeyCode::PageUp if !text => {
                if let Some(previous) = self.data["previous_cursor"].as_str() {
                    return Ok(Action::read(self.fresh(Some(previous.to_string()))));
                }
            }
            KeyCode::Up | KeyCode::Char('k') if text => self.scroll = self.scroll.saturating_sub(5),
            KeyCode::Down | KeyCode::Char('j') if text => {
                self.scroll = self.scroll.saturating_add(5).min(self.maximum_scroll)
            }
            KeyCode::PageUp if text => self.scroll = self.scroll.saturating_sub(20),
            KeyCode::PageDown if text => {
                self.scroll = self.scroll.saturating_add(20).min(self.maximum_scroll)
            }
            KeyCode::Home if text => self.scroll = 0,
            KeyCode::End if text => self.scroll = self.maximum_scroll,
            KeyCode::Char('n') | KeyCode::Right if text => {
                if let Some(next) = self.next() {
                    // Sanitized output is at most three times the 64 MiB input.
                    // Fixed 32 KiB pages therefore fit within this bound.
                    if self.trail.len() >= MAX_TEXT_PAGES {
                        return Err(Error::new(
                            "AGENT_ERROR",
                            "Saved log exceeds the display page bound",
                        ));
                    }
                    let mut page = self.fresh(Some(next.clone()));
                    page.trail.push(Some(next));
                    return Ok(Action::read(page));
                }
            }
            KeyCode::Char('p') | KeyCode::Left if text && self.trail.len() > 1 => {
                let mut trail = self.trail.clone();
                trail.pop();
                let mut page = self.fresh(trail.last().cloned().flatten());
                page.trail = trail;
                return Ok(Action::read(page));
            }
            KeyCode::Char('v') if text => {
                let Location::Text { session, turn, raw } = &self.location else {
                    unreachable!()
                };
                if *raw {
                    if let Some(back) = &self.back
                        && matches!(back.location, Location::Text { raw: false, .. })
                        && back.data["revision"] == self.data["revision"]
                    {
                        return Ok(Action::Show(back.clone()));
                    }
                    return Ok(Action::read(Self::new(
                        self.current.clone(),
                        Location::Text {
                            session: session.clone(),
                            turn: turn.clone(),
                            raw: false,
                        },
                        self.turns_parent(),
                        None,
                    )));
                }
                let cursor = Some(format!(
                    "0:{}",
                    self.data["revision"].as_str().unwrap_or("")
                ));
                return Ok(Action::read(Self::new(
                    self.current.clone(),
                    Location::Text {
                        session: session.clone(),
                        turn: turn.clone(),
                        raw: true,
                    },
                    Some(Box::new(self.clone())),
                    cursor,
                )));
            }
            _ => {}
        }
        Ok(Action::Keep)
    }
}

pub fn draw(page: &mut Page, frame: &mut ratatui::Frame, area: ratatui::layout::Rect) {
    use ratatui::{
        layout::{Constraint, Layout},
        style::{Color, Style},
        widgets::{List, ListItem, ListState, Paragraph, Wrap},
    };
    let (title, context) = match &page.location {
        Location::Sessions => (
            "Saved conversations".to_string(),
            "Live UUID order · Enter reads saved turns".to_string(),
        ),
        Location::Turns(session) => (
            "Saved turns".to_string(),
            format!("Session {session} · live UUID order"),
        ),
        Location::Text { session, turn, raw } => (
            format!(
                "Saved log · {}",
                if *raw { "raw JSONL" } else { "formatted" }
            ),
            format!(
                "Session {session}\nTurn {turn} · revision {}",
                page.data["revision"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(12)
                    .collect::<String>()
            ),
        ),
    };
    let block = super::panel(format!("{title} · read-only"), true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let raw_records = page.data["raw_records"].as_u64().unwrap_or(0);
    let mut notice = if raw_records > 0 {
        format!("{context}\n{raw_records} structured/unfinished records shown as original text")
    } else {
        format!("{context}\nBrowsing sends no requests · saved data is unchanged")
    };
    if let Some(message) = &page.notice {
        notice.push_str(&format!("\n{} · r refreshes", super::clean(message)));
    }
    let notice = Paragraph::new(notice).wrap(Wrap { trim: false });
    let height = notice
        .line_count(inner.width)
        .min(inner.height.saturating_sub(5) as usize) as u16;
    let regions = Layout::vertical([
        Constraint::Length(height),
        Constraint::Min(1),
        Constraint::Length(3),
    ])
    .split(inner);
    frame.render_widget(notice, regions[0]);
    if matches!(page.location, Location::Text { .. }) {
        let mut text: Cow<'_, str> = Cow::Borrowed(page.data["text"].as_str().unwrap_or(""));
        if matches!(page.location, Location::Text { raw: true, .. })
            && page.data["utf8_valid"] == false
        {
            text = Cow::Owned(format!(
                "Damaged UTF-8 · exact bytes follow as Base64\n{}\n\nBase64:\n{}",
                text,
                page.data["bytes_base64"].as_str().unwrap_or("")
            ));
        }
        if matches!(page.location, Location::Text { raw: true, .. }) {
            let mut visible = String::new();
            for character in text.chars() {
                if character.is_control() && !matches!(character, '\n' | '\t') {
                    visible.extend(character.escape_default());
                } else {
                    visible.push(character);
                }
            }
            text = Cow::Owned(visible);
        }
        // Raw records can contain terminal controls; decode only into cells.
        let clean = super::clean(&text);
        let paragraph = Paragraph::new(clean).wrap(Wrap { trim: false });
        page.maximum_scroll = paragraph
            .line_count(regions[1].width)
            .saturating_sub(regions[1].height as usize)
            .min(u16::MAX as usize) as u16;
        page.scroll = page.scroll.min(page.maximum_scroll);
        frame.render_widget(paragraph.scroll((page.scroll, 0)), regions[1]);
        frame.render_widget(Paragraph::new("↑/↓ PgUp/PgDn scroll · Home/End edges\nn/p or →/← pages · g first · v raw/formatted\nBackspace parent · s sessions · r refresh · Esc closes").wrap(Wrap { trim: false }), regions[2]);
    } else {
        let entries = page.entries();
        let items = entries
            .iter()
            .map(|entry| {
                let id = entry["id"].as_str().unwrap_or("unknown");
                let label = if entry["unavailable"] == true {
                    format!("{id} · unavailable")
                } else if page.location == Location::Sessions {
                    format!(
                        "{} · {} · {id}",
                        entry["agent"].as_str().unwrap_or("unknown"),
                        entry["requested_auth"].as_str().unwrap_or("unknown")
                    )
                } else {
                    format!("{id} · {} bytes", entry["bytes"].as_u64().unwrap_or(0))
                };
                ListItem::new(super::clean(&label))
            })
            .collect::<Vec<_>>();
        if items.is_empty() {
            frame.render_widget(
                Paragraph::new("No saved entries in this page · Home refreshes"),
                regions[1],
            );
        } else {
            page.selected = page.selected.min(items.len() - 1);
            let mut state = ListState::default().with_selected(Some(page.selected));
            frame.render_stateful_widget(
                List::new(items)
                    .highlight_style(Style::default().fg(Color::Black).bg(Color::Cyan))
                    .highlight_symbol("› "),
                regions[1],
                &mut state,
            );
        }
        frame.render_widget(Paragraph::new("↑/↓ select · Enter opens · n/p pages\nHome first · c current conversation · r refresh\nBackspace parent · Esc/q closes · s sessions").wrap(Wrap { trim: false }), regions[2]);
    }
}

impl super::App {
    pub(super) fn open_saved(&mut self, page: Page, back: Option<Box<Page>>) -> Result<()> {
        if self.document_task.is_some() {
            self.modal = back.map(super::checkpoints::Modal::Saved);
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Wait for the active local operation before reading saved logs",
            ));
        }
        let task = match page.task(self.conversation.store().clone()) {
            Ok(task) => task,
            Err(error) => {
                self.modal = back.map(super::checkpoints::Modal::Saved);
                return Err(error);
            }
        };
        self.saved_return = back;
        self.restore_return = None;
        self.document_task = Some(task);
        self.modal = Some(super::checkpoints::Modal::Loading {
            message: "Reading saved logs… · Esc cancels",
        });
        Ok(())
    }
    pub(super) fn saved_key(&mut self, mut page: Box<Page>, key: KeyEvent) -> Result<bool> {
        let action = match page.key(key) {
            Ok(action) => action,
            Err(error) => {
                self.modal = Some(super::checkpoints::Modal::Saved(page));
                return Err(error);
            }
        };
        match action {
            Action::Keep => self.modal = Some(super::checkpoints::Modal::Saved(page)),
            Action::Show(page) => self.modal = Some(super::checkpoints::Modal::Saved(page)),
            Action::Read(next) => self.open_saved(*next, Some(page))?,
            Action::Close => {}
        }
        Ok(false)
    }
}
