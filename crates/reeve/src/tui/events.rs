//! Keybinding registry: structured table consumed by both the event
//! matcher (KeyEvent → Action) and the help-overlay renderer.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    ForceRefresh,
    FocusPrevColumn,
    FocusNextColumn,
    SelectUp,
    SelectDown,
    SelectFirst,
    SelectLast,
    OpenDrillIn,
    CloseDrillIn,
    OpenActionMenu,
    OpenHelp,
    CloseModal,
    SelectMenuUp,
    SelectMenuDown,
    ConfirmMenuItem,
    MoveLeft,
    MoveRight,
    OpenMovePicker,
    Edit,
    New,
    Delete,
    FocusKanban,
    ResizePaneLeft,
    ResizePaneRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyMatcher {
    Char(char),
    Code(KeyCode),
    Shift(KeyCode),
    /// Ctrl-modified key. Compared case-insensitively for char codes.
    Ctrl(KeyCode),
    /// Alt-modified key. Compared case-insensitively for char codes.
    Alt(KeyCode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpSection {
    Global,
    Navigation,
    Mutations,
}

#[derive(Debug, Clone, Copy)]
pub struct Binding {
    pub key: KeyMatcher,
    pub action: Action,
    pub description: &'static str,
    pub section: HelpSection,
}

pub const BINDINGS: &[Binding] = &[
    Binding {
        key: KeyMatcher::Char('q'),
        action: Action::Quit,
        description: "quit",
        section: HelpSection::Global,
    },
    Binding {
        key: KeyMatcher::Code(KeyCode::Esc),
        action: Action::CloseDrillIn,
        description: "close drill-in",
        section: HelpSection::Navigation,
    },
    Binding {
        key: KeyMatcher::Code(KeyCode::Enter),
        action: Action::OpenDrillIn,
        description: "open drill-in / focus session",
        section: HelpSection::Navigation,
    },
    Binding {
        key: KeyMatcher::Char('r'),
        action: Action::ForceRefresh,
        description: "force refresh",
        section: HelpSection::Global,
    },
    Binding {
        key: KeyMatcher::Char('h'),
        action: Action::FocusPrevColumn,
        description: "focus previous column",
        section: HelpSection::Navigation,
    },
    Binding {
        key: KeyMatcher::Code(KeyCode::Left),
        action: Action::FocusPrevColumn,
        description: "focus previous column",
        section: HelpSection::Navigation,
    },
    Binding {
        key: KeyMatcher::Char('l'),
        action: Action::FocusNextColumn,
        description: "focus next column",
        section: HelpSection::Navigation,
    },
    Binding {
        key: KeyMatcher::Code(KeyCode::Right),
        action: Action::FocusNextColumn,
        description: "focus next column",
        section: HelpSection::Navigation,
    },
    Binding {
        key: KeyMatcher::Char('j'),
        action: Action::SelectDown,
        description: "select next card",
        section: HelpSection::Navigation,
    },
    Binding {
        key: KeyMatcher::Code(KeyCode::Down),
        action: Action::SelectDown,
        description: "select next card",
        section: HelpSection::Navigation,
    },
    Binding {
        key: KeyMatcher::Char('k'),
        action: Action::SelectUp,
        description: "select previous card",
        section: HelpSection::Navigation,
    },
    Binding {
        key: KeyMatcher::Code(KeyCode::Up),
        action: Action::SelectUp,
        description: "select previous card",
        section: HelpSection::Navigation,
    },
    Binding {
        key: KeyMatcher::Char('g'),
        action: Action::SelectFirst,
        description: "select first card",
        section: HelpSection::Navigation,
    },
    Binding {
        key: KeyMatcher::Code(KeyCode::PageUp),
        action: Action::SelectFirst,
        description: "select first card",
        section: HelpSection::Navigation,
    },
    Binding {
        key: KeyMatcher::Shift(KeyCode::Char('g')),
        action: Action::SelectLast,
        description: "select last card",
        section: HelpSection::Navigation,
    },
    Binding {
        key: KeyMatcher::Code(KeyCode::PageDown),
        action: Action::SelectLast,
        description: "select last card",
        section: HelpSection::Navigation,
    },
    Binding {
        key: KeyMatcher::Char(' '),
        action: Action::OpenActionMenu,
        description: "open action menu",
        section: HelpSection::Navigation,
    },
    Binding {
        key: KeyMatcher::Char('?'),
        action: Action::OpenHelp,
        description: "help",
        section: HelpSection::Global,
    },
    Binding {
        key: KeyMatcher::Shift(KeyCode::Char('h')),
        action: Action::MoveLeft,
        description: "move card left",
        section: HelpSection::Mutations,
    },
    Binding {
        key: KeyMatcher::Shift(KeyCode::Left),
        action: Action::MoveLeft,
        description: "move card left",
        section: HelpSection::Mutations,
    },
    Binding {
        key: KeyMatcher::Shift(KeyCode::Char('l')),
        action: Action::MoveRight,
        description: "move card right",
        section: HelpSection::Mutations,
    },
    Binding {
        key: KeyMatcher::Shift(KeyCode::Right),
        action: Action::MoveRight,
        description: "move card right",
        section: HelpSection::Mutations,
    },
    Binding {
        key: KeyMatcher::Char('m'),
        action: Action::OpenMovePicker,
        description: "move with column picker",
        section: HelpSection::Mutations,
    },
    Binding {
        key: KeyMatcher::Char('e'),
        action: Action::Edit,
        description: "edit brief",
        section: HelpSection::Mutations,
    },
    Binding {
        key: KeyMatcher::Char('n'),
        action: Action::New,
        description: "new card",
        section: HelpSection::Mutations,
    },
    Binding {
        key: KeyMatcher::Char('d'),
        action: Action::Delete,
        description: "delete card",
        section: HelpSection::Mutations,
    },
    // Enter advances inward: in KanbanFull it opens the drill-in; in
    // drill-in with kanban focus it focuses the session pane. The handler
    // for Action::OpenDrillIn dispatches on layout/focus state.
    //
    // Alt+Esc exits the session pane back to kanban focus. Doesn't
    // conflict with anything claude/tmux care about; Ctrl+G / Ctrl+]
    // remain as alternates for terminals that mis-report Alt+Esc.
    Binding {
        key: KeyMatcher::Alt(KeyCode::Esc),
        action: Action::FocusKanban,
        description: "exit session pane",
        section: HelpSection::Navigation,
    },
    Binding {
        key: KeyMatcher::Ctrl(KeyCode::Char('g')),
        action: Action::FocusKanban,
        description: "exit session pane (alternate)",
        section: HelpSection::Navigation,
    },
    Binding {
        key: KeyMatcher::Ctrl(KeyCode::Char(']')),
        action: Action::FocusKanban,
        description: "exit session pane (alternate)",
        section: HelpSection::Navigation,
    },
    // Drill-in split resize. Only useful in FocusedColumnPlusDetail with
    // kanban focus — the session-pane forwarding intercepts only
    // FocusKanban actions, so Ctrl+Left/Right inside the session goes
    // straight to the PTY (where it does word-motion in claude/shells).
    Binding {
        key: KeyMatcher::Ctrl(KeyCode::Left),
        action: Action::ResizePaneLeft,
        description: "shrink kanban / grow session",
        section: HelpSection::Navigation,
    },
    Binding {
        key: KeyMatcher::Ctrl(KeyCode::Right),
        action: Action::ResizePaneRight,
        description: "grow kanban / shrink session",
        section: HelpSection::Navigation,
    },
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
            event.code == KeyCode::Char(c)
                && !event.modifiers.contains(KeyModifiers::SHIFT)
                && !event.modifiers.contains(KeyModifiers::CONTROL)
                && !event.modifiers.contains(KeyModifiers::ALT)
        }
        KeyMatcher::Code(code) => {
            event.code == code
                && !event.modifiers.contains(KeyModifiers::SHIFT)
                && !event.modifiers.contains(KeyModifiers::CONTROL)
                && !event.modifiers.contains(KeyModifiers::ALT)
        }
        KeyMatcher::Shift(code) => {
            if !event.modifiers.contains(KeyModifiers::SHIFT) {
                return false;
            }
            // Crossterm reports Shift+<letter> as KeyCode::Char(<uppercase>)
            // with the SHIFT modifier set. Compare case-insensitively so a
            // binding written as `Shift(Char('g'))` matches both `(Char('G'),
            // SHIFT)` and `(Char('g'), SHIFT)`.
            match (code, event.code) {
                (KeyCode::Char(want), KeyCode::Char(got)) => {
                    want.to_ascii_lowercase() == got.to_ascii_lowercase()
                }
                _ => code == event.code,
            }
        }
        KeyMatcher::Ctrl(code) => {
            if !event.modifiers.contains(KeyModifiers::CONTROL) {
                return false;
            }
            match (code, event.code) {
                (KeyCode::Char(want), KeyCode::Char(got)) => {
                    want.to_ascii_lowercase() == got.to_ascii_lowercase()
                }
                _ => code == event.code,
            }
        }
        KeyMatcher::Alt(code) => {
            if !event.modifiers.contains(KeyModifiers::ALT) {
                return false;
            }
            match (code, event.code) {
                (KeyCode::Char(want), KeyCode::Char(got)) => {
                    want.to_ascii_lowercase() == got.to_ascii_lowercase()
                }
                _ => code == event.code,
            }
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

    #[test]
    fn shift_g_uppercase_translates_to_select_last() {
        // Crossterm reports Shift+G as Char('G') with SHIFT modifier.
        let key = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);
        assert_eq!(translate(key), Some(Action::SelectLast));
    }

    #[test]
    fn shift_g_lowercase_with_modifier_translates_to_select_last() {
        // Some terminals / kitty protocol may report (Char('g'), SHIFT).
        let key = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::SHIFT);
        assert_eq!(translate(key), Some(Action::SelectLast));
    }

    #[test]
    fn plain_g_without_shift_translates_to_select_first() {
        let key = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE);
        assert_eq!(translate(key), Some(Action::SelectFirst));
    }

    #[test]
    fn page_up_translates_to_select_first() {
        let key = KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE);
        assert_eq!(translate(key), Some(Action::SelectFirst));
    }

    #[test]
    fn page_down_translates_to_select_last() {
        let key = KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE);
        assert_eq!(translate(key), Some(Action::SelectLast));
    }

    #[test]
    fn shift_h_uppercase_translates_to_move_left() {
        // Crossterm reports Shift+H as Char('H') with SHIFT modifier.
        let key = KeyEvent::new(KeyCode::Char('H'), KeyModifiers::SHIFT);
        assert_eq!(translate(key), Some(Action::MoveLeft));
    }

    #[test]
    fn shift_h_lowercase_with_modifier_translates_to_move_left() {
        let key = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::SHIFT);
        assert_eq!(translate(key), Some(Action::MoveLeft));
    }

    #[test]
    fn plain_h_without_shift_translates_to_focus_prev_column() {
        let key = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE);
        assert_eq!(translate(key), Some(Action::FocusPrevColumn));
    }

    #[test]
    fn shift_l_uppercase_translates_to_move_right() {
        let key = KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT);
        assert_eq!(translate(key), Some(Action::MoveRight));
    }

    #[test]
    fn plain_l_without_shift_translates_to_focus_next_column() {
        let key = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE);
        assert_eq!(translate(key), Some(Action::FocusNextColumn));
    }

    #[test]
    fn shift_left_translates_to_move_left() {
        let key = KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT);
        assert_eq!(translate(key), Some(Action::MoveLeft));
    }

    #[test]
    fn shift_right_translates_to_move_right() {
        let key = KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT);
        assert_eq!(translate(key), Some(Action::MoveRight));
    }

    #[test]
    fn m_translates_to_open_move_picker() {
        let key = KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE);
        assert_eq!(translate(key), Some(Action::OpenMovePicker));
    }

    #[test]
    fn alt_esc_translates_to_focus_kanban() {
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::ALT);
        assert_eq!(translate(key), Some(Action::FocusKanban));
    }

    #[test]
    fn plain_esc_does_not_match_alt_esc_binding() {
        // Plain Esc → CloseDrillIn (not FocusKanban). Confirms the Alt-aware
        // Code matcher rejects ALT-modified events.
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(translate(key), Some(Action::CloseDrillIn));
    }

    #[test]
    fn tab_is_no_longer_bound() {
        let key = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(translate(key), None);
    }

    #[test]
    fn shift_tab_back_tab_is_no_longer_bound() {
        let key = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(translate(key), None);
        let key = KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE);
        assert_eq!(translate(key), None);
    }

    #[test]
    fn enter_translates_to_open_drill_in() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(translate(key), Some(Action::OpenDrillIn));
    }
}
