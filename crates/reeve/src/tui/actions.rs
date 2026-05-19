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
        Action::Edit => open_edit_modal(app),
        Action::New => open_new_modal(app).await,
        Action::Delete => open_delete_modal(app),
    }
}

pub async fn handle_modal_key(app: &mut App, key: crossterm::event::KeyEvent) {
    use crossterm::event::{KeyCode, KeyModifiers};
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
        crate::tui::modal::ModalKind::EditBrief { card_id, buffer, cursor, error: _ } => {
            let mut buffer = buffer;
            let mut cursor = cursor;
            match key.code {
                KeyCode::Esc => {
                    app.modal = None;
                }
                KeyCode::Enter => {
                    let params = reeve_protocol::EditParams {
                        id: card_id.clone(),
                        brief: buffer.clone(),
                    };
                    match crate::tui::client_ext::rpc_edit(params).await {
                        Ok(Ok(result)) => {
                            if let Some(c) = app.cards.iter_mut().find(|c| c.id == card_id) {
                                *c = result.card;
                            }
                            app.modal = None;
                            set_status_info(app, "saved".into());
                        }
                        Ok(Err(e)) => {
                            app.modal = Some(crate::tui::modal::ModalKind::EditBrief {
                                card_id,
                                buffer,
                                cursor,
                                error: Some(e.message),
                            });
                        }
                        Err(e) => {
                            app.modal = Some(crate::tui::modal::ModalKind::EditBrief {
                                card_id,
                                buffer,
                                cursor,
                                error: Some(format!("{e:#}")),
                            });
                        }
                    }
                }
                KeyCode::Backspace => {
                    let target = if key.modifiers.contains(KeyModifiers::CONTROL) {
                        word_boundary_before(&buffer, cursor)
                    } else if cursor > 0 {
                        cursor - 1
                    } else {
                        cursor
                    };
                    if target < cursor {
                        let idx = char_byte_index(&buffer, target);
                        let end = char_byte_index(&buffer, cursor);
                        buffer.drain(idx..end);
                        cursor = target;
                    }
                    app.modal = Some(crate::tui::modal::ModalKind::EditBrief {
                        card_id,
                        buffer,
                        cursor,
                        error: None,
                    });
                }
                KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    // Ctrl+W = delete word backward (emacs/vim convention).
                    let target = word_boundary_before(&buffer, cursor);
                    if target < cursor {
                        let idx = char_byte_index(&buffer, target);
                        let end = char_byte_index(&buffer, cursor);
                        buffer.drain(idx..end);
                        cursor = target;
                    }
                    app.modal = Some(crate::tui::modal::ModalKind::EditBrief {
                        card_id,
                        buffer,
                        cursor,
                        error: None,
                    });
                }
                KeyCode::Left => {
                    if cursor > 0 {
                        cursor -= 1;
                    }
                    app.modal = Some(crate::tui::modal::ModalKind::EditBrief {
                        card_id,
                        buffer,
                        cursor,
                        error: None,
                    });
                }
                KeyCode::Right => {
                    if cursor < buffer.chars().count() {
                        cursor += 1;
                    }
                    app.modal = Some(crate::tui::modal::ModalKind::EditBrief {
                        card_id,
                        buffer,
                        cursor,
                        error: None,
                    });
                }
                KeyCode::Home => {
                    cursor = 0;
                    app.modal = Some(crate::tui::modal::ModalKind::EditBrief {
                        card_id,
                        buffer,
                        cursor,
                        error: None,
                    });
                }
                KeyCode::End => {
                    cursor = buffer.chars().count();
                    app.modal = Some(crate::tui::modal::ModalKind::EditBrief {
                        card_id,
                        buffer,
                        cursor,
                        error: None,
                    });
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let byte_idx = char_byte_index(&buffer, cursor);
                    buffer.insert(byte_idx, c);
                    cursor += 1;
                    app.modal = Some(crate::tui::modal::ModalKind::EditBrief {
                        card_id,
                        buffer,
                        cursor,
                        error: None,
                    });
                }
                _ => {}
            }
        }
        crate::tui::modal::ModalKind::DeleteConfirm { card_id } => {
            match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    app.modal = None;
                    let params = reeve_protocol::DeleteParams { id: card_id.clone() };
                    match crate::tui::client_ext::rpc_delete(params).await {
                        Ok(Ok(())) => {
                            app.cards.retain(|c| c.id != card_id);
                            crate::tui::actions::ensure_selection_valid(app);
                            set_status_info(app, "deleted".into());
                        }
                        Ok(Err(e)) => {
                            set_status_error(app, format!("delete failed: {}", e.message));
                        }
                        Err(e) => {
                            set_status_error(app, format!("delete error: {e:#}"));
                        }
                    }
                }
                KeyCode::Char('n') | KeyCode::Esc => {
                    app.modal = None;
                }
                _ => {}
            }
        }
        crate::tui::modal::ModalKind::NewCard {
            household,
            brief,
            cursor,
            household_picker_index,
            error: _,
        } => {
            let mut household = household;
            let mut brief = brief;
            let mut cursor = cursor;
            let mut household_picker_index = household_picker_index;

            // If no household chosen yet and multiple exist → household picker step.
            if household.is_none() && app.households.len() > 1 {
                match key.code {
                    KeyCode::Esc => {
                        app.modal = None;
                    }
                    KeyCode::Up => {
                        if household_picker_index > 0 {
                            household_picker_index -= 1;
                        }
                        app.modal = Some(crate::tui::modal::ModalKind::NewCard {
                            household: None,
                            brief,
                            cursor,
                            household_picker_index,
                            error: None,
                        });
                    }
                    KeyCode::Down => {
                        let max = app.households.len().saturating_sub(1);
                        household_picker_index = (household_picker_index + 1).min(max);
                        app.modal = Some(crate::tui::modal::ModalKind::NewCard {
                            household: None,
                            brief,
                            cursor,
                            household_picker_index,
                            error: None,
                        });
                    }
                    KeyCode::Enter => {
                        if let Some(h) = app.households.get(household_picker_index) {
                            household = Some(h.name.clone());
                        }
                        app.modal = Some(crate::tui::modal::ModalKind::NewCard {
                            household,
                            brief,
                            cursor,
                            household_picker_index,
                            error: None,
                        });
                    }
                    _ => {}
                }
            } else {
                // Brief input step.
                match key.code {
                    KeyCode::Esc => {
                        app.modal = None;
                    }
                    KeyCode::Enter => {
                        if brief.trim().is_empty() {
                            app.modal = Some(crate::tui::modal::ModalKind::NewCard {
                                household,
                                brief,
                                cursor,
                                household_picker_index,
                                error: Some("brief cannot be empty".into()),
                            });
                        } else {
                            let hh = household.clone().unwrap_or_default();
                            let params = reeve_protocol::NewParams {
                                household: hh,
                                brief: brief.clone(),
                                needs_heavy: false,
                            };
                            match crate::tui::client_ext::rpc_new(params).await {
                                Ok(Ok(_)) => {
                                    app.modal = None;
                                    set_status_info(app, "created".into());
                                }
                                Ok(Err(e)) => {
                                    app.modal = Some(crate::tui::modal::ModalKind::NewCard {
                                        household,
                                        brief,
                                        cursor,
                                        household_picker_index,
                                        error: Some(e.message),
                                    });
                                }
                                Err(e) => {
                                    app.modal = Some(crate::tui::modal::ModalKind::NewCard {
                                        household,
                                        brief,
                                        cursor,
                                        household_picker_index,
                                        error: Some(format!("{e:#}")),
                                    });
                                }
                            }
                        }
                    }
                    KeyCode::Backspace => {
                        let target = if key.modifiers.contains(KeyModifiers::CONTROL) {
                            word_boundary_before(&brief, cursor)
                        } else if cursor > 0 {
                            cursor - 1
                        } else {
                            cursor
                        };
                        if target < cursor {
                            let idx = char_byte_index(&brief, target);
                            let end = char_byte_index(&brief, cursor);
                            brief.drain(idx..end);
                            cursor = target;
                        }
                        app.modal = Some(crate::tui::modal::ModalKind::NewCard {
                            household,
                            brief,
                            cursor,
                            household_picker_index,
                            error: None,
                        });
                    }
                    KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        // Ctrl+W = delete word backward.
                        let target = word_boundary_before(&brief, cursor);
                        if target < cursor {
                            let idx = char_byte_index(&brief, target);
                            let end = char_byte_index(&brief, cursor);
                            brief.drain(idx..end);
                            cursor = target;
                        }
                        app.modal = Some(crate::tui::modal::ModalKind::NewCard {
                            household,
                            brief,
                            cursor,
                            household_picker_index,
                            error: None,
                        });
                    }
                    KeyCode::Left => {
                        if cursor > 0 {
                            cursor -= 1;
                        }
                        app.modal = Some(crate::tui::modal::ModalKind::NewCard {
                            household,
                            brief,
                            cursor,
                            household_picker_index,
                            error: None,
                        });
                    }
                    KeyCode::Right => {
                        if cursor < brief.chars().count() {
                            cursor += 1;
                        }
                        app.modal = Some(crate::tui::modal::ModalKind::NewCard {
                            household,
                            brief,
                            cursor,
                            household_picker_index,
                            error: None,
                        });
                    }
                    KeyCode::Home => {
                        cursor = 0;
                        app.modal = Some(crate::tui::modal::ModalKind::NewCard {
                            household,
                            brief,
                            cursor,
                            household_picker_index,
                            error: None,
                        });
                    }
                    KeyCode::End => {
                        cursor = brief.chars().count();
                        app.modal = Some(crate::tui::modal::ModalKind::NewCard {
                            household,
                            brief,
                            cursor,
                            household_picker_index,
                            error: None,
                        });
                    }
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        let byte_idx = char_byte_index(&brief, cursor);
                        brief.insert(byte_idx, c);
                        cursor += 1;
                        app.modal = Some(crate::tui::modal::ModalKind::NewCard {
                            household,
                            brief,
                            cursor,
                            household_picker_index,
                            error: None,
                        });
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Given a string and a character-position cursor, return the byte index.
fn char_byte_index(s: &str, char_pos: usize) -> usize {
    s.char_indices()
        .nth(char_pos)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

/// Find the char-position to the left where a word-delete should stop:
/// skip trailing whitespace, then skip the word. Used by Ctrl+Backspace
/// and Ctrl+W in text-input modals.
fn word_boundary_before(s: &str, cursor: usize) -> usize {
    let chars: Vec<char> = s.chars().collect();
    let mut i = cursor.min(chars.len());
    // Skip whitespace immediately before cursor.
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    // Then skip the word itself.
    while i > 0 && !chars[i - 1].is_whitespace() {
        i -= 1;
    }
    i
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
        ActionMenuKind::Edit => open_edit_modal(app),
        ActionMenuKind::Delete => open_delete_modal(app),
        ActionMenuKind::NewCard => open_new_modal(app).await,
        ActionMenuKind::Attach => {
            // Wired in Task 13/14.
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
    app.set_status(msg, crate::tui::app::StatusLevel::Error);
}

fn set_status_info(app: &mut App, msg: String) {
    app.set_status(msg, crate::tui::app::StatusLevel::Info);
}

fn open_edit_modal(app: &mut App) {
    let Some(card_id) = app.selected_card_id.clone() else {
        set_status_error(app, "no card selected".into());
        return;
    };
    let Some(card) = app.cards.iter().find(|c| c.id == card_id) else {
        return;
    };
    app.modal = Some(crate::tui::modal::ModalKind::EditBrief {
        card_id,
        buffer: card.brief.clone(),
        cursor: card.brief.chars().count(),
        error: None,
    });
}

async fn open_new_modal(app: &mut App) {
    match crate::tui::client_ext::rpc_household_ls().await {
        Ok(Ok(list)) => app.households = list,
        Ok(Err(e)) => {
            set_status_error(app, format!("household.ls failed: {}", e.message));
            return;
        }
        Err(e) => {
            set_status_error(app, format!("household.ls error: {e:#}"));
            return;
        }
    }
    if app.households.is_empty() {
        set_status_error(
            app,
            "no households. run `reeve household add` or `reeve household new` first.".into(),
        );
        return;
    }
    let single = if app.households.len() == 1 {
        Some(app.households[0].name.clone())
    } else {
        None
    };
    app.modal = Some(crate::tui::modal::ModalKind::NewCard {
        household: single,
        brief: String::new(),
        cursor: 0,
        household_picker_index: 0,
        error: None,
    });
}

fn open_delete_modal(app: &mut App) {
    let Some(card_id) = app.selected_card_id.clone() else {
        set_status_error(app, "no card selected".into());
        return;
    };
    app.modal = Some(crate::tui::modal::ModalKind::DeleteConfirm { card_id });
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
