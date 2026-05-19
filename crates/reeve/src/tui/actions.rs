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
    }
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
}
