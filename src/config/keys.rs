//! User-owned prompt shortcuts, validated without terminal or rendering dependencies.

use super::{UserSource, invalid, user_config_path};
use crate::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const TEMPLATE: &str = "\n[tui.keys]\n# Each explicit list replaces that action's defaults.\n# submit = [\"Ctrl+Enter\", \"Alt+Enter\", \"Ctrl+s\"]\n# paste = [\"Ctrl+v\", \"Alt+v\"]\n";

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct TuiSettings {
    pub keys: PromptKeys,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PromptKeys {
    /// Shortcuts active only in the idle prompt. Enter alone remains a newline.
    #[schemars(length(min = 1, max = 8))]
    pub submit: Vec<String>,
    /// An empty list disables clipboard shortcuts; /paste-image remains available.
    #[schemars(length(max = 8))]
    pub paste: Vec<String>,
}

impl Default for PromptKeys {
    fn default() -> Self {
        Self {
            submit: ["Ctrl+Enter", "Alt+Enter", "Ctrl+s"]
                .map(str::to_owned)
                .into(),
            paste: ["Ctrl+v", "Alt+v"].map(str::to_owned).into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Enter,
    Letter(char),
    Function(u8),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Shortcut {
    key: Key,
    ctrl: bool,
    alt: bool,
    shift: bool,
}

fn parse(value: &str) -> Result<Shortcut> {
    if value.is_empty() || value.len() > 64 || !value.is_ascii() {
        return Err(invalid("A prompt shortcut requires 1–64 ASCII bytes"));
    }
    let mut parts = value.split('+');
    let mut shortcut = Shortcut {
        key: Key::Enter,
        ctrl: false,
        alt: false,
        shift: false,
    };
    let mut token = parts.next().unwrap();
    for next in parts {
        let modifier = match token.to_ascii_lowercase().as_str() {
            "ctrl" => &mut shortcut.ctrl,
            "alt" => &mut shortcut.alt,
            "shift" => &mut shortcut.shift,
            _ => return Err(invalid("Shortcut modifiers are Ctrl, Alt and Shift")),
        };
        if *modifier {
            return Err(invalid("Duplicate shortcut modifier"));
        }
        *modifier = true;
        token = next;
    }
    let key = token.to_ascii_lowercase();
    shortcut.key = if key == "enter" {
        Key::Enter
    } else if key.len() == 1 && key.as_bytes()[0].is_ascii_lowercase() {
        Key::Letter(key.as_bytes()[0] as char)
    } else if let Some(number) = key
        .strip_prefix('f')
        .and_then(|value| value.parse::<u8>().ok())
    {
        if !(2..=24).contains(&number) || key != format!("f{number}") {
            return Err(invalid(
                "Prompt function shortcuts support F2–F24; F1 opens help",
            ));
        }
        Key::Function(number)
    } else {
        return Err(invalid(
            "Prompt shortcuts support Enter, ASCII letters, or F2–F24",
        ));
    };
    if !matches!(shortcut.key, Key::Function(_)) && !shortcut.ctrl && !shortcut.alt {
        return Err(invalid(
            "Letter and Enter shortcuts require Ctrl or Alt to preserve text input",
        ));
    }
    // Legacy terminals encode these as cancellation, palette, Tab, newline or
    // Backspace. They must remain reachable regardless of keyboard protocol.
    if shortcut.ctrl && matches!(shortcut.key, Key::Letter('c' | 'p' | 'i' | 'j' | 'm' | 'h')) {
        return Err(invalid(
            "Reserved Ctrl shortcut conflicts with global or legacy terminal input",
        ));
    }
    Ok(shortcut)
}

impl PromptKeys {
    pub fn validate(&self) -> Result<()> {
        if self.submit.is_empty() || self.submit.len() > 8 || self.paste.len() > 8 {
            return Err(invalid(
                "Prompt keys require 1–8 submit shortcuts and 0–8 paste shortcuts",
            ));
        }
        let mut seen = BTreeSet::new();
        for value in self.submit.iter().chain(&self.paste) {
            if !seen.insert(parse(value)?) {
                return Err(invalid("Duplicate or conflicting prompt shortcuts"));
            }
        }
        Ok(())
    }

    pub fn submit_label(&self) -> String {
        labels(&self.submit)
    }

    pub fn paste_label(&self) -> String {
        if self.paste.is_empty() {
            "/paste-image".into()
        } else {
            labels(&self.paste)
        }
    }

    #[cfg(feature = "tui")]
    pub fn submits(&self, event: crossterm::event::KeyEvent) -> bool {
        self.submit.iter().any(|value| matches_event(value, event))
    }

    #[cfg(feature = "tui")]
    pub fn pastes(&self, event: crossterm::event::KeyEvent) -> bool {
        self.paste.iter().any(|value| matches_event(value, event))
    }
}

fn labels(values: &[String]) -> String {
    values
        .iter()
        .map(|value| {
            let Ok(shortcut) = parse(value) else {
                return String::new();
            };
            let key = match shortcut.key {
                Key::Enter => "Enter".into(),
                Key::Letter(letter) => letter.to_ascii_uppercase().to_string(),
                Key::Function(number) => format!("F{number}"),
            };
            format!(
                "{}{}{}{key}",
                if shortcut.ctrl { "Ctrl+" } else { "" },
                if shortcut.alt { "Alt+" } else { "" },
                if shortcut.shift { "Shift+" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join(" / ")
}

#[cfg(feature = "tui")]
fn matches_event(value: &str, event: crossterm::event::KeyEvent) -> bool {
    use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
    let Ok(shortcut) = parse(value) else {
        return false;
    };
    if event.kind == KeyEventKind::Release {
        return false;
    }
    let mut modifiers = KeyModifiers::NONE;
    if shortcut.ctrl {
        modifiers |= KeyModifiers::CONTROL;
    }
    if shortcut.alt {
        modifiers |= KeyModifiers::ALT;
    }
    if shortcut.shift {
        modifiers |= KeyModifiers::SHIFT;
    }
    event.modifiers == modifiers
        && match (shortcut.key, event.code) {
            (Key::Enter, KeyCode::Enter) => true,
            (Key::Letter(letter), KeyCode::Char(character)) => {
                character.to_ascii_lowercase() == letter
            }
            (Key::Function(expected), KeyCode::F(actual)) => expected == actual,
            _ => false,
        }
}

/// Read only user shortcuts without discovering fonts or opening a document.
pub fn load() -> Result<PromptKeys> {
    let source = UserSource::load(Some(user_config_path()?), true)?;
    source.verify()?;
    Ok(source.settings.tui.keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_alternate_terminal_keys_are_valid() {
        PromptKeys::default().validate().unwrap();
        let mut keys = PromptKeys {
            submit: vec!["F2".into(), "Alt+Shift+Enter".into()],
            paste: vec!["Ctrl+Alt+v".into()],
        };
        keys.validate().unwrap();
        keys.paste.clear();
        keys.validate().unwrap();
        assert_eq!(keys.paste_label(), "/paste-image");
    }

    #[test]
    fn reserved_and_ambiguous_input_is_rejected() {
        for value in [
            "Enter",
            "Shift+Enter",
            "q",
            "Shift+q",
            "Ctrl+c",
            "Ctrl+Shift+p",
            "Ctrl+i",
            "Ctrl+j",
            "Ctrl+m",
            "Ctrl+h",
            "F1",
            "F25",
            "F02",
            "Ctrl+Ctrl+s",
            "Super+v",
            "Ctrl+",
            "Ctrl+ArrowUp",
            "Alt+あ",
            "Alt+v\n",
        ] {
            assert!(
                parse(value).is_err(),
                "Unexpected accepted shortcut: {value:?}"
            );
        }
        let mut keys = PromptKeys {
            submit: vec!["Ctrl+Enter".into()],
            paste: vec!["ctrl+enter".into()],
        };
        assert!(keys.validate().is_err());
        keys.paste = vec![];
        keys.submit = vec!["Alt+Shift+s".into(), "shift+alt+S".into()];
        assert!(keys.validate().is_err());
        keys.submit.clear();
        assert!(keys.validate().is_err());
        keys.submit = vec!["F2".into(); 9];
        assert!(keys.validate().is_err());
    }

    #[cfg(feature = "tui")]
    #[test]
    fn matching_requires_exact_modifiers_and_ignores_key_release() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
        let keys = PromptKeys {
            submit: vec!["Ctrl+Shift+s".into()],
            paste: vec!["F12".into()],
        };
        let event = KeyEvent::new(
            KeyCode::Char('S'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert!(keys.submits(event));
        assert!(!keys.submits(KeyEvent {
            modifiers: KeyModifiers::CONTROL,
            ..event
        }));
        assert!(!keys.submits(KeyEvent {
            kind: KeyEventKind::Release,
            ..event
        }));
        assert!(!keys.submits(KeyEvent {
            code: KeyCode::Char('x'),
            ..event
        }));
        assert!(keys.pastes(KeyEvent::new(KeyCode::F(12), KeyModifiers::NONE)));
        assert!(!keys.pastes(KeyEvent::new(KeyCode::F(12), KeyModifiers::ALT)));
    }
}
