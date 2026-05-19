//! Action dispatch: translates `Action` variants into state mutations and
//! daemon RPC calls. Keeps `app.rs` free of business logic.

use crate::tui::app::App;
use crate::tui::client_ext;
use crate::tui::events::Action;
use reeve_protocol::Column;

/// Single dispatch entry for all user actions.
pub async fn handle(app: &mut App, action: Action) {
    match action {
        Action::Quit => app.quit = true,
        Action::ForceRefresh => {
            let prev_id = app.selected_card_id.clone();
            if let Ok(Ok(result)) = client_ext::rpc_ls().await {
                app.cards = result.cards;
                app.last_refresh = std::time::Instant::now();
                let still_present = prev_id
                    .as_ref()
                    .map(|id| app.focused_cards().iter().any(|c| &c.id == id))
                    .unwrap_or(false);
                if still_present {
                    app.selected_card_id = prev_id;
                } else {
                    reset_selection(app);
                }
            }
        }
        Action::FocusPrevColumn => {
            let cols = Column::ALL;
            let idx = cols.iter().position(|c| c == &app.focused_column).unwrap_or(0);
            if idx > 0 {
                app.focused_column = cols[idx - 1];
            }
            reset_selection(app);
        }
        Action::FocusNextColumn => {
            let cols = Column::ALL;
            let idx = cols.iter().position(|c| c == &app.focused_column).unwrap_or(0);
            if idx + 1 < cols.len() {
                app.focused_column = cols[idx + 1];
            }
            reset_selection(app);
        }
        Action::SelectUp => {
            let cards = app.focused_cards();
            if cards.is_empty() {
                return;
            }
            match app.selected_index() {
                Some(0) | None => {
                    app.selected_card_id = Some(cards[0].id.clone());
                }
                Some(i) => {
                    app.selected_card_id = Some(cards[i - 1].id.clone());
                }
            }
        }
        Action::SelectDown => {
            let cards = app.focused_cards();
            if cards.is_empty() {
                return;
            }
            let last = cards.len() - 1;
            match app.selected_index() {
                None => {
                    app.selected_card_id = Some(cards[0].id.clone());
                }
                Some(i) if i >= last => {
                    app.selected_card_id = Some(cards[last].id.clone());
                }
                Some(i) => {
                    app.selected_card_id = Some(cards[i + 1].id.clone());
                }
            }
        }
        Action::SelectFirst => {
            let cards = app.focused_cards();
            app.selected_card_id = cards.first().map(|c| c.id.clone());
        }
        Action::SelectLast => {
            let cards = app.focused_cards();
            app.selected_card_id = cards.last().map(|c| c.id.clone());
        }
        Action::OpenDrillIn => {
            if app.selected_card_id.is_some() {
                app.layout = crate::tui::layout::LayoutMode::FocusedColumnPlusDetail;
            }
        }
        Action::CloseDrillIn => {
            if app.layout == crate::tui::layout::LayoutMode::FocusedColumnPlusDetail {
                app.layout = crate::tui::layout::LayoutMode::KanbanFull;
            } else {
                app.quit = true; // Esc in KanbanFull = quit
            }
        }
        Action::OpenActionMenu => {
            app.modal = Some(crate::tui::modal::ModalKind::ActionMenu { selected_index: 0 });
        }
        Action::OpenHelp => {
            app.modal = Some(crate::tui::modal::ModalKind::Help);
        }
        Action::CloseModal => {
            app.modal = None;
        }
        Action::SelectMenuUp | Action::SelectMenuDown | Action::ConfirmMenuItem => {
            // Routed via handle_modal_key; non-modal context is no-op.
        }
        Action::MoveLeft => mv_relative(app, -1).await,
        Action::MoveRight => mv_relative(app, 1).await,
        Action::OpenMovePicker => {
            if app.selected_card_id.is_some() {
                let current =
                    selected_card_column(app).unwrap_or(reeve_protocol::Column::Backlog);
                app.modal = Some(crate::tui::modal::ModalKind::MoveColumn {
                    card_id: app.selected_card_id.clone().unwrap(),
                    choice: current,
                });
            }
        }
    }
}

pub async fn handle_modal_key(app: &mut App, key: crossterm::event::KeyEvent) {
    use crossterm::event::KeyCode;
    let Some(modal) = app.modal.clone() else {
        return;
    };
    match modal {
        crate::tui::modal::ModalKind::ActionMenu { selected_index } => {
            match key.code {
                KeyCode::Esc => app.modal = None,
                KeyCode::Up => {
                    let new = selected_index.saturating_sub(1);
                    app.modal =
                        Some(crate::tui::modal::ModalKind::ActionMenu { selected_index: new });
                }
                KeyCode::Down => {
                    let items =
                        crate::tui::modal::build_action_menu(selected_card_column(app));
                    let new = (selected_index + 1).min(items.len().saturating_sub(1));
                    app.modal =
                        Some(crate::tui::modal::ModalKind::ActionMenu { selected_index: new });
                }
                KeyCode::Enter => {
                    let items =
                        crate::tui::modal::build_action_menu(selected_card_column(app));
                    if let Some(item) = items.get(selected_index) {
                        if item.enabled {
                            app.modal = None;
                            execute_menu_kind(app, item.action_kind).await;
                        }
                    }
                }
                KeyCode::Char(c) => handle_menu_letter(app, c).await,
                _ => {}
            }
        }
        crate::tui::modal::ModalKind::Help => {
            if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?')
            ) {
                app.modal = None;
            }
        }
        crate::tui::modal::ModalKind::MoveColumn { card_id, choice } => {
            use reeve_protocol::Column;
            let cols = Column::ALL;
            let cur_idx = cols.iter().position(|c| *c == choice).unwrap_or(0);
            match key.code {
                KeyCode::Esc => app.modal = None,
                KeyCode::Up => {
                    let new = cur_idx.saturating_sub(1);
                    app.modal = Some(crate::tui::modal::ModalKind::MoveColumn {
                        card_id: card_id.clone(),
                        choice: cols[new],
                    });
                }
                KeyCode::Down => {
                    let new = (cur_idx + 1).min(cols.len() - 1);
                    app.modal = Some(crate::tui::modal::ModalKind::MoveColumn {
                        card_id: card_id.clone(),
                        choice: cols[new],
                    });
                }
                KeyCode::Enter => {
                    app.modal = None;
                    let params =
                        reeve_protocol::MvParams { id: card_id.clone(), column: choice };
                    match crate::tui::client_ext::rpc_mv(params).await {
                        Ok(Ok(updated)) => {
                            if let Some(c) = app.cards.iter_mut().find(|c| c.id == card_id) {
                                *c = updated;
                            }
                            app.focused_column = choice;
                        }
                        Ok(Err(e)) => set_status_error(app, format!("mv failed: {}", e.message)),
                        Err(e) => set_status_error(app, format!("mv error: {e:#}")),
                    }
                }
                _ => {}
            }
        }
    }
}

fn selected_card_column(app: &App) -> Option<Column> {
    let id = app.selected_card_id.as_ref()?;
    app.cards.iter().find(|c| &c.id == id).map(|c| c.column)
}

async fn handle_menu_letter(app: &mut App, c: char) {
    let items = crate::tui::modal::build_action_menu(selected_card_column(app));
    if let Some(item) = items.iter().find(|i| i.key_hint.chars().next() == Some(c)) {
        if item.enabled {
            app.modal = None;
            execute_menu_kind(app, item.action_kind).await;
        }
    }
}

async fn execute_menu_kind(app: &mut App, kind: crate::tui::modal::ActionMenuKind) {
    use crate::tui::modal::ActionMenuKind;
    match kind {
        ActionMenuKind::MoveLeft => mv_relative(app, -1).await,
        ActionMenuKind::MoveRight => mv_relative(app, 1).await,
        ActionMenuKind::MovePicker => {
            if let Some(card_id) = app.selected_card_id.clone() {
                let cur =
                    selected_card_column(app).unwrap_or(reeve_protocol::Column::Backlog);
                app.modal = Some(crate::tui::modal::ModalKind::MoveColumn {
                    card_id,
                    choice: cur,
                });
            }
        }
        ActionMenuKind::Edit
        | ActionMenuKind::Attach
        | ActionMenuKind::Delete
        | ActionMenuKind::NewCard => {
            // Wired in Task 12.
        }
    }
}

async fn mv_relative(app: &mut App, delta: i32) {
    let Some(card_id) = app.selected_card_id.clone() else {
        return;
    };
    let Some(card) = app.cards.iter().find(|c| c.id == card_id).cloned() else {
        return;
    };
    let cols = reeve_protocol::Column::ALL;
    let cur = cols.iter().position(|c| *c == card.column).unwrap_or(0);
    let new_idx = (cur as i32 + delta).clamp(0, cols.len() as i32 - 1) as usize;
    if cols[new_idx] == card.column {
        return; // no-op (already at edge)
    }
    let target = cols[new_idx];
    let params = reeve_protocol::MvParams { id: card_id.clone(), column: target };
    match crate::tui::client_ext::rpc_mv(params).await {
        Ok(Ok(updated)) => {
            if let Some(c) = app.cards.iter_mut().find(|c| c.id == card_id) {
                *c = updated;
            }
            app.focused_column = target;
        }
        Ok(Err(e)) => set_status_error(app, format!("mv failed: {}", e.message)),
        Err(e) => set_status_error(app, format!("mv error: {e:#}")),
    }
}

fn set_status_error(app: &mut App, msg: String) {
    let _ = (app, msg); // wired up in Task 12 (status bar)
}

/// Pick first card in focused column as the selection. Called after column
/// switches and after the periodic refresh replaces `app.cards`.
fn reset_selection(app: &mut App) {
    app.selected_card_id = app.focused_cards().first().map(|c| c.id.clone());
}

/// Validate the current selection against the live card list. Called by the
/// periodic refresh in `app::drive` after replacing `app.cards`.
///
/// - If the selected card is still present, keep it.
/// - If the selection is gone (deleted externally), pick the first card in the
///   focused column.
/// - If there was no selection but there are cards, pick the first.
pub fn ensure_selection_valid(app: &mut App) {
    let still_present = app
        .selected_card_id
        .as_ref()
        .map(|id| app.focused_cards().iter().any(|c| &c.id == id))
        .unwrap_or(false);

    if !still_present {
        reset_selection(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::App;
    use crate::tui::events::Action;
    use reeve_protocol::{Card, Column};

    fn card(id: &str, column: Column, created_at: i64) -> Card {
        Card {
            id: id.into(),
            household: "h".into(),
            brief: "x".into(),
            column,
            created_at,
            updated_at: created_at,
            needs_heavy: false,
            parent_id: None,
        }
    }

    #[tokio::test]
    async fn select_down_clamps_at_bottom() {
        let mut app = App::new();
        app.cards = vec![
            card("aaa", Column::Backlog, 1),
            card("bbb", Column::Backlog, 2),
        ];
        app.selected_card_id = Some("bbb".into());

        handle(&mut app, Action::SelectDown).await;

        assert_eq!(app.selected_card_id.as_deref(), Some("bbb"));
    }

    #[tokio::test]
    async fn select_up_clamps_at_top() {
        let mut app = App::new();
        app.cards = vec![
            card("aaa", Column::Backlog, 1),
            card("bbb", Column::Backlog, 2),
        ];
        app.selected_card_id = Some("aaa".into());

        handle(&mut app, Action::SelectUp).await;

        assert_eq!(app.selected_card_id.as_deref(), Some("aaa"));
    }

    #[tokio::test]
    async fn focus_next_column_resets_selection_to_first_in_new_column() {
        let mut app = App::new();
        app.cards = vec![
            card("aaa", Column::Backlog, 1),
            card("bbb", Column::Planning, 2),
            card("ccc", Column::Planning, 3),
        ];
        app.focused_column = Column::Backlog;
        app.selected_card_id = Some("aaa".into());

        handle(&mut app, Action::FocusNextColumn).await;

        assert_eq!(app.focused_column, Column::Planning);
        // first card in Planning by created_at is "bbb"
        assert_eq!(app.selected_card_id.as_deref(), Some("bbb"));
    }

    #[tokio::test]
    async fn focus_prev_column_clamps_at_backlog() {
        let mut app = App::new();
        app.focused_column = Column::Backlog;

        handle(&mut app, Action::FocusPrevColumn).await;

        assert_eq!(app.focused_column, Column::Backlog);
    }

    #[tokio::test]
    async fn select_last_jumps_to_bottom() {
        let mut app = App::new();
        app.cards = vec![
            card("aaa", Column::Backlog, 1),
            card("bbb", Column::Backlog, 2),
            card("ccc", Column::Backlog, 3),
        ];
        app.selected_card_id = Some("aaa".into());

        handle(&mut app, Action::SelectLast).await;

        assert_eq!(app.selected_card_id.as_deref(), Some("ccc"));
    }

    #[tokio::test]
    async fn ensure_selection_valid_clears_when_selected_card_deleted() {
        let mut app = App::new();
        app.cards = vec![card("a", Column::Backlog, 0)];
        app.selected_card_id = Some("b".into()); // not in cards
        ensure_selection_valid(&mut app);
        assert_eq!(app.selected_card_id.as_deref(), Some("a"));
    }

    #[tokio::test]
    async fn enter_opens_drill_in_when_card_selected() {
        let mut app = App::new();
        app.cards = vec![card("a", Column::Backlog, 0)];
        app.selected_card_id = Some("a".into());
        handle(&mut app, Action::OpenDrillIn).await;
        assert_eq!(app.layout, crate::tui::layout::LayoutMode::FocusedColumnPlusDetail);
    }

    #[tokio::test]
    async fn enter_does_not_open_drill_in_when_no_selection() {
        let mut app = App::new();
        handle(&mut app, Action::OpenDrillIn).await;
        assert_eq!(app.layout, crate::tui::layout::LayoutMode::KanbanFull);
    }

    #[tokio::test]
    async fn esc_closes_drill_in_when_open() {
        let mut app = App::new();
        app.layout = crate::tui::layout::LayoutMode::FocusedColumnPlusDetail;
        handle(&mut app, Action::CloseDrillIn).await;
        assert_eq!(app.layout, crate::tui::layout::LayoutMode::KanbanFull);
        assert!(!app.quit);
    }

    #[tokio::test]
    async fn esc_quits_when_drill_in_closed() {
        let mut app = App::new();
        handle(&mut app, Action::CloseDrillIn).await;
        assert!(app.quit);
    }
}
