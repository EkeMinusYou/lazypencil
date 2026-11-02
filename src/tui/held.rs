//! Navigation driven by key state rather than the OS text-repeat delay.

use crossterm::event::{KeyEvent, KeyEventKind};
use std::time::{Duration, Instant};

const INTERVAL: Duration = Duration::from_millis(33);

#[derive(Default)]
pub struct Held {
    key: Option<KeyEvent>,
    deadline: Option<Instant>,
}

impl Held {
    pub fn clear(&mut self) {
        self.key = None;
        self.deadline = None;
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// Return whether this physical event should also perform an action.
    pub fn input(&mut self, key: KeyEvent, continuous: bool, now: Instant) -> bool {
        if key.kind == KeyEventKind::Release {
            if self.key.is_some_and(|held| held.code == key.code) {
                self.clear();
            }
            return false;
        }
        if !continuous {
            self.clear();
            return true;
        }
        if self.key.is_some_and(|held| held.code == key.code) {
            // OS repeats must not add extra movement or postpone the next tick.
            return false;
        }
        self.key = Some(key);
        self.deadline = Some(now + INTERVAL);
        true
    }

    pub fn tick(&mut self, now: Instant) -> Option<KeyEvent> {
        if self.deadline? > now {
            return None;
        }
        self.deadline = Some(now + INTERVAL);
        self.key.map(|key| KeyEvent {
            kind: KeyEventKind::Repeat,
            ..key
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    #[test]
    fn press_moves_until_release_without_any_os_repeat() {
        let mut held = Held::default();
        let now = Instant::now();
        let key = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE);
        assert!(held.input(key, true, now));
        assert!(held.tick(now + INTERVAL / 2).is_none());
        assert!(held.tick(now + INTERVAL).is_some());
        assert!(!held.input(key, true, now + INTERVAL));
        assert!(held.tick(now + INTERVAL * 2).is_some());
        assert!(!held.input(
            KeyEvent {
                kind: KeyEventKind::Release,
                ..key
            },
            true,
            now
        ));
        assert!(held.tick(now + INTERVAL * 3).is_none());
    }

    #[test]
    fn unsupported_terminals_and_focus_changes_cannot_latch_movement() {
        let mut held = Held::default();
        let now = Instant::now();
        let key = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE);
        assert!(held.input(key, false, now));
        assert!(held.tick(now + INTERVAL).is_none());
        assert!(held.input(key, true, now));
        held.clear();
        assert!(held.tick(now + INTERVAL).is_none());
    }
}
