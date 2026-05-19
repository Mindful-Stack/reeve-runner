//! Modal state — exclusive overlay views (action menu, edit/new/delete/move
//! prompts, help, daemon-down banner). The App holds one Option<ModalKind>
//! at a time; events.rs routes keys differently when a modal is open.

use reeve_protocol::Column;

#[derive(Debug, Clone)]
pub enum ModalKind {
    /// Centered popup showing context-aware action shortcuts.
    ActionMenu { selected_index: usize },
    /// Help cheatsheet.
    Help,
    // Tasks 11/12 add: MoveColumn, EditBrief, NewCard, DeleteConfirm.
}

#[derive(Debug, Clone)]
pub struct ActionMenuItem {
    pub key_hint: &'static str,
    pub description: &'static str,
    pub action_kind: ActionMenuKind,
    pub enabled: bool,
    pub disabled_reason: Option<&'static str>,
}

#[derive(Debug, Clone, Copy)]
pub enum ActionMenuKind {
    Edit,
    MoveLeft,
    MoveRight,
    MovePicker,
    Attach,
    Delete,
    NewCard,
}

pub fn build_action_menu(card_column: Option<Column>) -> Vec<ActionMenuItem> {
    let in_implementing = card_column == Some(Column::Implementing);
    let has_card = card_column.is_some();
    let at_first = card_column == Some(Column::Backlog);
    let at_last = card_column == Some(Column::Done);

    vec![
        ActionMenuItem {
            key_hint: "e",
            description: "edit brief",
            action_kind: ActionMenuKind::Edit,
            enabled: has_card,
            disabled_reason: if !has_card { Some("no card selected") } else { None },
        },
        ActionMenuItem {
            key_hint: "m",
            description: "move to column...",
            action_kind: ActionMenuKind::MovePicker,
            enabled: has_card,
            disabled_reason: if !has_card { Some("no card selected") } else { None },
        },
        ActionMenuItem {
            key_hint: "H",
            description: "move left",
            action_kind: ActionMenuKind::MoveLeft,
            enabled: has_card && !at_first,
            disabled_reason: if !has_card {
                Some("no card selected")
            } else if at_first {
                Some("already at Backlog")
            } else {
                None
            },
        },
        ActionMenuItem {
            key_hint: "L",
            description: "move right",
            action_kind: ActionMenuKind::MoveRight,
            enabled: has_card && !at_last,
            disabled_reason: if !has_card {
                Some("no card selected")
            } else if at_last {
                Some("already at Done")
            } else {
                None
            },
        },
        ActionMenuItem {
            key_hint: "a",
            description: "attach (open session)",
            action_kind: ActionMenuKind::Attach,
            enabled: in_implementing,
            disabled_reason: if !in_implementing {
                Some("card not in Implementing")
            } else {
                None
            },
        },
        ActionMenuItem {
            key_hint: "d",
            description: "delete",
            action_kind: ActionMenuKind::Delete,
            enabled: has_card,
            disabled_reason: if !has_card { Some("no card selected") } else { None },
        },
        ActionMenuItem {
            key_hint: "n",
            description: "new card",
            action_kind: ActionMenuKind::NewCard,
            enabled: true,
            disabled_reason: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implementing_card_enables_attach() {
        let items = build_action_menu(Some(Column::Implementing));
        let attach = items
            .iter()
            .find(|i| matches!(i.action_kind, ActionMenuKind::Attach))
            .unwrap();
        assert!(attach.enabled);
    }

    #[test]
    fn backlog_card_disables_attach() {
        let items = build_action_menu(Some(Column::Backlog));
        let attach = items
            .iter()
            .find(|i| matches!(i.action_kind, ActionMenuKind::Attach))
            .unwrap();
        assert!(!attach.enabled);
        assert_eq!(attach.disabled_reason, Some("card not in Implementing"));
    }

    #[test]
    fn no_card_disables_move() {
        let items = build_action_menu(None);
        let mv_left = items
            .iter()
            .find(|i| matches!(i.action_kind, ActionMenuKind::MoveLeft))
            .unwrap();
        assert!(!mv_left.enabled);
    }
}
