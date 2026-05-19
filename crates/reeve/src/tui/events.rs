//! Keybinding registry: structured table consumed by both the event
//! matcher (KeyEvent → Action) and the help-overlay renderer.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    ForceRefresh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyMatcher {
    Char(char),
    Code(KeyCode),
    Shift(KeyCode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpSection {
    Global,
}

#[derive(Debug, Clone, Copy)]
pub struct Binding {
    pub key: KeyMatcher,
    pub action: Action,
    pub description: &'static str,
    pub section: HelpSection,
}

pub const BINDINGS: &[Binding] = &[
    Binding { key: KeyMatcher::Char('q'), action: Action::Quit, description: "quit", section: HelpSection::Global },
    Binding { key: KeyMatcher::Code(KeyCode::Esc), action: Action::Quit, description: "quit", section: HelpSection::Global },
    Binding { key: KeyMatcher::Char('r'), action: Action::ForceRefresh, description: "force refresh", section: HelpSection::Global },
];

pub fn translate(event: KeyEvent) -> Option<Action> {
    for b in BINDINGS {
        if matches(b.key, event) {
            return Some(b.action);
        }
    }
    None
}

fn matches(matcher: KeyMatcher, event: KeyEvent) -> bool {
    match matcher {
        KeyMatcher::Char(c) => {
            event.code == KeyCode::Char(c) && !event.modifiers.contains(KeyModifiers::SHIFT)
        }
        KeyMatcher::Code(code) => event.code == code,
        KeyMatcher::Shift(code) => {
            event.code == code && event.modifiers.contains(KeyModifiers::SHIFT)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn q_translates_to_quit() {
        let key = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(translate(key), Some(Action::Quit));
    }

    #[test]
    fn r_translates_to_force_refresh() {
        let key = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE);
        assert_eq!(translate(key), Some(Action::ForceRefresh));
    }

    #[test]
    fn unbound_key_returns_none() {
        let key = KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE);
        assert_eq!(translate(key), None);
    }
}
