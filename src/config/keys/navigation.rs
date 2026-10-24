//! Focus-aware user bindings. Modal acceptance and emergency cancellation stay fixed.

use super::{PromptKeys, Shortcut, invalid, labels, parse, parse_shortcut};
use crate::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const PROMPT: u8 = 1;
const PREVIEW: u8 = 2;
const NODES: u8 = 4;
const ATTACHMENTS: u8 = 8;
const CONVERSATION: u8 = 16;
const ALL: u8 = PROMPT | PREVIEW | NODES | ATTACHMENTS | CONVERSATION;
const VIEWING: u8 = ALL & !PROMPT;

macro_rules! bindings {
    ($( $field:ident => $action:ident ($scope:expr, [$($default:literal),*]) ),* $(,)?) => {
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        #[serde(default, deny_unknown_fields)]
        pub struct NavigationKeys {
            $(#[schemars(length(max = 8))] pub $field: Vec<String>,)*
        }

        impl Default for NavigationKeys {
            fn default() -> Self {
                Self { $($field: vec![$($default.to_owned()),*],)* }
            }
        }

        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum NavigationAction { $($action,)* }

        impl NavigationAction {
            pub fn name(self) -> &'static str {
                match self { $(Self::$action => stringify!($field),)* }
            }
        }

        impl NavigationKeys {
            fn definitions(&self) -> Vec<(NavigationAction, &[String], u8)> {
                vec![$((NavigationAction::$action, &self.$field, $scope),)*]
            }

            fn values(&self, action: NavigationAction) -> &[String] {
                match action { $(NavigationAction::$action => &self.$field,)* }
            }
        }
    };
}

bindings! {
    next_panel => NextPanel(ALL, ["Tab"]),
    previous_panel => PreviousPanel(ALL, ["Shift+Tab"]),
    help => Help(ALL, ["F1", "?"]),
    commands => Commands(ALL, ["Ctrl+p"]),
    quit => Quit(VIEWING, ["q"]),
    refresh => Refresh(VIEWING, ["r"]),
    open_preview => OpenPreview(VIEWING, ["o"]),
    pan_left => PanLeft(PREVIEW, ["Left", "h"]),
    pan_right => PanRight(PREVIEW, ["Right", "l"]),
    pan_up => PanUp(PREVIEW, ["Up", "k"]),
    pan_down => PanDown(PREVIEW, ["Down", "j"]),
    zoom_in => ZoomIn(PREVIEW, ["+", "="]),
    zoom_out => ZoomOut(PREVIEW, ["-"]),
    fit_document => FitDocument(PREVIEW, ["0"]),
    fit_selected => FitSelected(PREVIEW | NODES, ["f"]),
    time_backward => TimeBackward(PREVIEW, ["["]),
    time_forward => TimeForward(PREVIEW, ["]"]),
    play_pause => PlayPause(PREVIEW, ["Space"]),
    previous_item => PreviousItem(NODES | ATTACHMENTS, ["Up", "k"]),
    next_item => NextItem(NODES | ATTACHMENTS, ["Down", "j"]),
    search_nodes => SearchNodes(NODES, ["/"]),
    clear_selection => ClearSelection(NODES, ["Esc"]),
    activate_node => ActivateNode(NODES, ["Enter"]),
    remove_attachment => RemoveAttachment(ATTACHMENTS, ["Delete", "Backspace", "d"]),
    scroll_up => ScrollUp(CONVERSATION, ["PageUp", "Up", "k"]),
    scroll_down => ScrollDown(CONVERSATION, ["PageDown", "Down", "j"]),
    follow_output => FollowOutput(CONVERSATION, ["End", "Shift+g"]),
}

impl NavigationKeys {
    pub fn validate(&self, prompt: &PromptKeys) -> Result<()> {
        let mut used: BTreeMap<Shortcut, u8> = BTreeMap::new();
        for value in prompt.submit.iter().chain(&prompt.paste) {
            used.insert(parse(value)?, PROMPT);
        }
        for (action, values, scope) in self.definitions() {
            if values.len() > 8 {
                return Err(invalid(format!(
                    "Navigation action {} accepts at most 8 shortcuts",
                    action.name()
                )));
            }
            for value in values {
                let shortcut = parse_shortcut(value, true)?;
                let scope = if shortcut.prompt_safe() {
                    scope
                } else {
                    scope & !PROMPT
                };
                let prior = used.entry(shortcut).or_default();
                if *prior & scope != 0 {
                    return Err(invalid(format!(
                        "Navigation action {} has a duplicate or conflicting shortcut in the same panel",
                        action.name()
                    )));
                }
                *prior |= scope;
            }
        }
        if !self
            .next_panel
            .iter()
            .chain(&self.previous_panel)
            .any(|value| parse_shortcut(value, true).is_ok_and(|key| key.prompt_safe()))
        {
            return Err(invalid(
                "Panel switching needs at least one shortcut usable while typing",
            ));
        }
        Ok(())
    }

    pub fn label(&self, action: NavigationAction) -> String {
        let values = self.values(action);
        if values.is_empty() {
            "unbound".into()
        } else {
            labels(values)
        }
    }

    #[cfg(feature = "tui")]
    pub fn matches(&self, action: NavigationAction, event: crossterm::event::KeyEvent) -> bool {
        self.values(action)
            .iter()
            .any(|value| parse_shortcut(value, true).is_ok_and(|shortcut| shortcut.matches(event)))
    }

    /// Plain text shortcuts never intercept prompt or node-search input.
    #[cfg(feature = "tui")]
    pub fn matches_while_typing(
        &self,
        action: NavigationAction,
        event: crossterm::event::KeyEvent,
    ) -> bool {
        self.values(action).iter().any(|value| {
            parse_shortcut(value, true)
                .is_ok_and(|shortcut| shortcut.prompt_safe() && shortcut.matches(event))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_allow_shared_keys_only_in_disjoint_panels() {
        NavigationKeys::default()
            .validate(&PromptKeys::default())
            .unwrap();
        let mut keys = NavigationKeys {
            pan_left: vec!["a".into()],
            previous_item: vec!["a".into()],
            ..Default::default()
        };
        keys.validate(&PromptKeys::default()).unwrap();
        keys.pan_right = vec!["A".into()];
        assert!(keys.validate(&PromptKeys::default()).is_err());
    }

    #[test]
    fn global_prompt_conflicts_and_legacy_aliases_are_rejected() {
        let prompt = PromptKeys {
            submit: vec!["F5".into()],
            paste: vec![],
        };
        let mut keys = NavigationKeys {
            help: vec!["F5".into()],
            ..Default::default()
        };
        assert!(keys.validate(&prompt).is_err());
        keys.help = vec!["x".into()];
        keys.validate(&prompt).unwrap();
        for reserved in [
            "Ctrl+c",
            "Ctrl+Shift+c",
            "Ctrl+h",
            "Ctrl+i",
            "Ctrl+j",
            "Ctrl+m",
        ] {
            keys.pan_left = vec![reserved.into()];
            assert!(keys.validate(&prompt).is_err(), "{reserved}");
        }
    }

    #[test]
    fn canonical_aliases_and_disabled_panel_routes_cannot_hide_conflicts() {
        let prompt = PromptKeys::default();
        let mut keys = NavigationKeys {
            next_panel: vec!["BackTab".into()],
            ..Default::default()
        };
        assert!(keys.validate(&prompt).is_err());
        for invalid in ["F0", "F25", "Ctrl+ArrowUp", "Ctrl+Ctrl+a", "あ", "Space\n"] {
            assert!(parse_shortcut(invalid, true).is_err(), "{invalid}");
        }
        keys.next_panel.clear();
        keys.previous_panel.clear();
        assert!(keys.validate(&prompt).is_err());
        keys.next_panel = vec!["n".into()];
        assert!(keys.validate(&prompt).is_err());
        keys.previous_panel = vec!["F6".into()];
        keys.validate(&prompt).unwrap();
        keys.zoom_in = vec!["+".into(), "Shift+Plus".into()];
        assert!(keys.validate(&prompt).is_err());
        keys.zoom_in = vec!["z".into(); 9];
        assert!(keys.validate(&prompt).is_err());
    }

    #[cfg(feature = "tui")]
    #[test]
    fn enhanced_shifted_symbols_and_legacy_backtab_match_the_same_binding() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
        let keys = NavigationKeys::default();
        for modifiers in [KeyModifiers::NONE, KeyModifiers::SHIFT] {
            assert!(keys.matches(
                NavigationAction::ZoomIn,
                KeyEvent::new(KeyCode::Char('+'), modifiers)
            ));
        }
        assert!(keys.matches(
            NavigationAction::PreviousPanel,
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE)
        ));
        assert!(!keys.matches_while_typing(
            NavigationAction::Help,
            KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE)
        ));
        assert!(keys.matches_while_typing(
            NavigationAction::Help,
            KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE)
        ));
        assert!(!keys.matches(
            NavigationAction::NextPanel,
            KeyEvent {
                kind: KeyEventKind::Release,
                ..KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)
            }
        ));
    }
}
