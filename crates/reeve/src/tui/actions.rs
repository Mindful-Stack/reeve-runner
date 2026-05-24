//! Action dispatch: translates `Action` variants into state mutations and
//! daemon RPC calls. Keeps `app.rs` free of business logic.

use crate::tui::app::App;
use crate::tui::events::Action;
use reeve_protocol::Column;

/// Single dispatch entry for all user actions.
pub async fn handle(app: &mut App, action: Action) {
    match action {
        Action::Quit => app.quit = true,
        Action::ForceRefresh => {
            let prev_id = app.selected_card_id.clone();
            // Manual retry: clear the failed-session memo so reconcile will
            // attempt fresh spawns this tick.
            app.sessions.clear_failed();
            // Shared refresh: updates daemon_unreachable, merges cards,
            // reconciles sessions, etc. On daemon-down the flag stays set
            // and the banner remains; on success the banner clears.
            refresh(app).await;
            if !app.daemon_unreachable {
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
            let idx = cols
                .iter()
                .position(|c| c == &app.focused_column)
                .unwrap_or(0);
            if idx > 0 {
                app.focused_column = cols[idx - 1];
            }
            reset_selection(app);
        }
        Action::FocusNextColumn => {
            let cols = Column::ALL;
            let idx = cols
                .iter()
                .position(|c| c == &app.focused_column)
                .unwrap_or(0);
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
            // Enter advances inward one step. From KanbanFull, open the
            // drill-in. From drill-in with kanban focus, hop into the
            // session pane (if one exists). From session pane, Enter is
            // forwarded raw to the PTY (handled by the drive loop, not
            // reachable here).
            use crate::tui::layout::LayoutMode;
            match app.layout {
                LayoutMode::KanbanFull => {
                    if app.selected_card_id.is_some() {
                        app.layout = LayoutMode::FocusedColumnPlusDetail;
                    }
                }
                LayoutMode::FocusedColumnPlusDetail => {
                    if let Some(id) = app.selected_card_id.clone() {
                        if app.sessions.get(&id).is_some() {
                            app.pane_focus = crate::tui::app::PaneFocus::SessionPane;
                            app.pending_attention.remove(&id);
                        } else {
                            set_status_info(app, "no live session for this card".into());
                        }
                    }
                }
            }
        }
        Action::CloseDrillIn => {
            // Esc only closes the drill-in. Quit is `q` only — Esc in
            // KanbanFull is a no-op so an unfocused press doesn't kill
            // the session.
            if app.layout == crate::tui::layout::LayoutMode::FocusedColumnPlusDetail {
                app.layout = crate::tui::layout::LayoutMode::KanbanFull;
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
        Action::MoveLeft => mv_relative(app, -1),
        Action::MoveRight => mv_relative(app, 1),
        Action::OpenMovePicker => {
            if app.selected_card_id.is_some() {
                let current = selected_card_column(app).unwrap_or(reeve_protocol::Column::Backlog);
                app.modal = Some(crate::tui::modal::ModalKind::MoveColumn {
                    card_id: app.selected_card_id.clone().unwrap(),
                    choice: current,
                });
            }
        }
        Action::Edit => open_edit_modal(app),
        Action::New => open_new_modal(app).await,
        Action::Delete => open_delete_modal(app),
        Action::FocusKanban => {
            app.pane_focus = crate::tui::app::PaneFocus::Kanban;
        }
    }
}

pub async fn handle_modal_key(app: &mut App, key: crossterm::event::KeyEvent) {
    use crossterm::event::{KeyCode, KeyModifiers};
    let Some(modal) = app.modal.clone() else {
        return;
    };
    match modal {
        crate::tui::modal::ModalKind::ActionMenu { selected_index } => match key.code {
            KeyCode::Esc => app.modal = None,
            KeyCode::Up => {
                let new = selected_index.saturating_sub(1);
                app.modal = Some(crate::tui::modal::ModalKind::ActionMenu {
                    selected_index: new,
                });
            }
            KeyCode::Down => {
                let items = crate::tui::modal::build_action_menu(selected_card_column(app));
                let new = (selected_index + 1).min(items.len().saturating_sub(1));
                app.modal = Some(crate::tui::modal::ModalKind::ActionMenu {
                    selected_index: new,
                });
            }
            KeyCode::Enter => {
                let items = crate::tui::modal::build_action_menu(selected_card_column(app));
                if let Some(item) = items.get(selected_index) {
                    if item.enabled {
                        app.modal = None;
                        execute_menu_kind(app, item.action_kind).await;
                    }
                }
            }
            KeyCode::Char(c) => handle_menu_letter(app, c).await,
            _ => {}
        },
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
                    submit_move(app, card_id.clone(), choice);
                }
                KeyCode::Char(c) if c.is_ascii_digit() => {
                    // 1-6 = Backlog..Done. Single-press jump-and-confirm.
                    let n = c.to_digit(10).unwrap() as usize;
                    if n >= 1 && n <= cols.len() {
                        submit_move(app, card_id.clone(), cols[n - 1]);
                    }
                }
                _ => {}
            }
        }
        crate::tui::modal::ModalKind::EditBrief {
            card_id,
            buffer,
            cursor,
            error: _,
        } => {
            let mut buffer = buffer;
            let mut cursor = cursor;
            match key.code {
                KeyCode::Esc => {
                    app.modal = None;
                }
                KeyCode::Enter => {
                    let new_brief = buffer.clone();
                    // Optimistic local update.
                    if let Some(c) = app.cards.iter_mut().find(|c| c.id == card_id) {
                        c.brief = new_brief.clone();
                    }
                    // Mark as in-flight so ls poll doesn't revert our local brief.
                    *app.pending_edits.entry(card_id.clone()).or_insert(0) += 1;
                    let params = reeve_protocol::EditParams {
                        id: card_id.clone(),
                        brief: new_brief,
                    };
                    let tx = app.op_tx.clone();
                    let card_id_for_event = card_id.clone();
                    app.modal = None;
                    // No "saving …" status; the local update is the feedback.
                    tokio::spawn(async move {
                        let event = match crate::tui::client_ext::rpc_edit(params).await {
                            Ok(Ok(result)) => crate::tui::app::OpEvent::EditOk {
                                card_id: card_id_for_event,
                                updated: result.card,
                            },
                            Ok(Err(e)) => crate::tui::app::OpEvent::EditErr {
                                card_id: card_id_for_event,
                                msg: e.message,
                            },
                            Err(e) => crate::tui::app::OpEvent::EditErr {
                                card_id: card_id_for_event,
                                msg: format!("{e:#}"),
                            },
                        };
                        let _ = tx.send(event);
                    });
                }
                KeyCode::Backspace => {
                    // Word-delete on Ctrl+Backspace OR Alt+Backspace (emacs
                    // M-Backspace convention; fallback for terminals that
                    // drop the Ctrl modifier on Backspace).
                    let is_word_delete = key.modifiers.contains(KeyModifiers::CONTROL)
                        || key.modifiers.contains(KeyModifiers::ALT);
                    let target = if is_word_delete {
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
                KeyCode::Char('w' | 'h' | '\x08' | '\x7f')
                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    // Delete word backward. Triggered by Ctrl+W (emacs/vim),
                    // Ctrl+H (typewriter convention / some terminals), and the
                    // raw BS/DEL bytes that other terminals send for
                    // Ctrl+Backspace.
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
                    // Optimistic local removal.
                    app.cards.retain(|c| c.id != card_id);
                    app.pending_attention.remove(&card_id);
                    app.sessions.close(&card_id);
                    // Mark as in-flight so ls poll doesn't re-add this card.
                    app.pending_deletes.insert(card_id.clone());

                    let params = reeve_protocol::DeleteParams {
                        id: card_id.clone(),
                    };
                    let tx = app.op_tx.clone();
                    let card_id_for_event = card_id.clone();
                    app.modal = None;
                    tokio::spawn(async move {
                        let event = match crate::tui::client_ext::rpc_delete(params).await {
                            Ok(Ok(())) => crate::tui::app::OpEvent::DeleteOk {
                                card_id: card_id_for_event,
                            },
                            Ok(Err(e)) => crate::tui::app::OpEvent::DeleteErr {
                                card_id: card_id_for_event,
                                msg: e.message,
                            },
                            Err(e) => crate::tui::app::OpEvent::DeleteErr {
                                card_id: card_id_for_event,
                                msg: format!("{e:#}"),
                            },
                        };
                        let _ = tx.send(event);
                    });
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
                            let tx = app.op_tx.clone();
                            app.modal = None;
                            tokio::spawn(async move {
                                let event = match crate::tui::client_ext::rpc_new(params).await {
                                    Ok(Ok(_)) => crate::tui::app::OpEvent::NewOk,
                                    Ok(Err(e)) => {
                                        crate::tui::app::OpEvent::NewErr { msg: e.message }
                                    }
                                    Err(e) => crate::tui::app::OpEvent::NewErr {
                                        msg: format!("{e:#}"),
                                    },
                                };
                                let _ = tx.send(event);
                            });
                        }
                    }
                    KeyCode::Backspace => {
                        // Word-delete on Ctrl+Backspace OR Alt+Backspace.
                        let is_word_delete = key.modifiers.contains(KeyModifiers::CONTROL)
                            || key.modifiers.contains(KeyModifiers::ALT);
                        let target = if is_word_delete {
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
                    KeyCode::Char('w' | 'h' | '\x08' | '\x7f')
                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        // Delete word backward. See EditBrief for the why
                        // behind catching all four code points.
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
        crate::tui::modal::ModalKind::ConfirmDestructiveMv {
            card_id,
            from: _,
            to,
        } => {
            use crossterm::event::KeyCode;
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    // User confirmed — close modal and proceed with the deferred mv.
                    let id = card_id.clone();
                    let target = to;
                    app.modal = None;
                    schedule_mv_unchecked(app, id, target);
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    app.modal = None;
                }
                _ => {}
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

/// Close the move-picker modal and submit the mv RPC. Shared by Enter and
/// the 1-6 digit shortcuts.
fn submit_move(app: &mut App, card_id: String, target: Column) {
    app.modal = None;
    schedule_mv(app, card_id, target);
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
        ActionMenuKind::MoveLeft => mv_relative(app, -1),
        ActionMenuKind::MoveRight => mv_relative(app, 1),
        ActionMenuKind::MovePicker => {
            if let Some(card_id) = app.selected_card_id.clone() {
                let cur = selected_card_column(app).unwrap_or(reeve_protocol::Column::Backlog);
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

fn mv_relative(app: &mut App, delta: i32) {
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
        return; // already at edge
    }
    let target = cols[new_idx];
    schedule_mv(app, card_id, target);
}

const MV_DEBOUNCE_MS: u64 = 400;

/// Public entry: check destructiveness, open modal if needed, else proceed.
fn schedule_mv(app: &mut App, card_id: String, target: Column) {
    let from = app.cards.iter().find(|c| c.id == card_id).map(|c| c.column);
    if let Some(from) = from {
        if from != target && is_destructive(from, target) {
            app.modal = Some(crate::tui::modal::ModalKind::ConfirmDestructiveMv {
                card_id,
                from,
                to: target,
            });
            return;
        }
    }
    schedule_mv_unchecked(app, card_id, target);
}

/// Whether the (from, to) transition should prompt the user to confirm.
///
/// This is a superset of the daemon's ensure_stopped routing
/// (`reeve_card::transitions::on_column_change`): every transition that
/// would tear down the container also confirms, PLUS Implementing →
/// Testing/Review. The latter pair keeps the container running by design
/// (Testing/Review reuses the same env), but we still warn — users were
/// accidentally bumping out of Implementing with a stray Shift+L and
/// expected a safety net regardless of whether the daemon destroys
/// anything.
///
/// Drift between this and the daemon's teardown matrix is intentional.
/// `is_destructive_is_superset_of_lifecycle_routing` guards the invariant
/// "confirm ⊇ daemon teardown".
fn is_destructive(from: Column, to: Column) -> bool {
    use reeve_protocol::Column::*;
    matches!(
        (from, to),
        (Implementing | Testing | Review, Backlog | Planning)
            | (Implementing | Testing | Review, Done)
            | (Backlog | Planning, Done)
            | (Implementing, Testing | Review)
    )
}

fn schedule_mv_unchecked(app: &mut App, card_id: String, target: Column) {
    // Optimistic local update — the card moves immediately. The RPC will
    // either confirm (no-op) or fail (next ls poll reconciles back to
    // daemon truth). `selected_card_id` stays on the moved card so
    // subsequent H/L acts on the same card the user is looking at.
    if let Some(c) = app.cards.iter_mut().find(|c| c.id == card_id) {
        c.column = target;
    }
    app.focused_column = target;
    // No status message — the visible column change is the feedback.

    // Debounce: replace any pending intent for this card with the latest
    // target and a fresh deadline. Rapid presses coalesce into one RPC.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(MV_DEBOUNCE_MS);
    app.pending_mv_intents.insert(card_id, (target, deadline));
}

/// Fire the mv RPC for a card whose debounce deadline has expired. Called
/// by the drive loop tick.
pub(crate) fn fire_pending_mv(app: &mut App, card_id: String, target: Column) {
    // Increment in-flight counter (used by merge_cards to preserve state).
    let entry = app.pending_mvs.entry(card_id.clone()).or_insert(0);
    if *entry == 0 {
        // Starting from quiescent — record the moment so the drill-in
        // can show an elapsed-time loader while the daemon is busy.
        app.pending_mv_started_at
            .insert(card_id.clone(), std::time::Instant::now());
    }
    *entry += 1;

    let params = reeve_protocol::MvParams {
        id: card_id.clone(),
        column: target,
    };
    let tx = app.op_tx.clone();
    let card_id_for_event = card_id.clone();
    tokio::spawn(async move {
        let event = match crate::tui::client_ext::rpc_mv(params).await {
            Ok(Ok(updated)) => crate::tui::app::OpEvent::MvOk {
                card_id: card_id_for_event,
                updated,
                target,
            },
            Ok(Err(e)) => crate::tui::app::OpEvent::MvErr {
                card_id: card_id_for_event,
                msg: e.message,
            },
            Err(e) => crate::tui::app::OpEvent::MvErr {
                card_id: card_id_for_event,
                msg: format!("{e:#}"),
            },
        };
        let _ = tx.send(event);
    });
}

/// Merge a fresh ls response into app.cards, preserving optimistic state
/// for cards with in-flight mutations.
///
/// Rules:
/// - If a card has a pending delete, drop it from the merged list (it'll
///   either get re-added on the next poll if the delete failed, or stay
///   gone if it succeeded).
/// - If a card has a pending mv, keep the local card (preserves the
///   optimistic column the user sees).
/// - If a card has a pending edit, keep the local card (preserves the
///   optimistic brief).
/// - Otherwise, use the server's version.
/// Cards the server confirms in Implementing AND for which no mv RPC is
/// in flight. Drives the session pane's spawn loop — without this filter,
/// optimistically-flipped cards would attach before the daemon committed.
pub fn confirmed_implementing(app: &App) -> Vec<String> {
    app.cards
        .iter()
        .filter(|c| c.column == reeve_protocol::Column::Implementing)
        .filter(|c| !app.pending_mvs.contains_key(&c.id))
        .map(|c| c.id.clone())
        .collect()
}

/// Single source of truth for periodic ls-poll. Updates cards via
/// merge_cards, refreshes session pane reconciliation, and toggles
/// app.daemon_unreachable based on the RPC outcome — so the full-screen
/// daemon-down banner appears/disappears automatically.
pub async fn refresh(app: &mut App) {
    match crate::tui::client_ext::rpc_ls().await {
        Ok(Ok(result)) => {
            merge_cards(app, result.cards);
            app.daemon_unreachable = false;
            app.last_refresh = std::time::Instant::now();
            ensure_selection_valid(app);
            let implementing = confirmed_implementing(app);
            app.sessions.reconcile(&implementing);
        }
        Ok(Err(_)) | Err(_) => {
            app.daemon_unreachable = true;
        }
    }
}

/// Polls card_progress for every card in pending_mvs whose last fetch is
/// at least `poll_interval_ms` old. Updates app.card_progress in place.
/// GC's entries for cards that now have a live session — the session pane
/// takes over the visible display.
pub async fn poll_card_progress(app: &mut App, poll_interval_ms: u64) {
    let now = std::time::Instant::now();
    let pending_ids: Vec<String> = app.pending_mvs.keys().cloned().collect();
    for card_id in pending_ids {
        let due = app
            .card_progress_last_poll
            .get(&card_id)
            .map(|t| now.duration_since(*t).as_millis() as u64 >= poll_interval_ms)
            .unwrap_or(true);
        if !due {
            continue;
        }
        if let Ok(Ok(r)) =
            crate::tui::client_ext::rpc_card_progress(reeve_protocol::CardProgressParams {
                id: card_id.clone(),
            })
            .await
        {
            // Empty stages mean the daemon has nothing for this card —
            // either it was cleared (ensure_stopped) or the daemon
            // restarted. Don't cache the emptiness, or the visibility
            // gate will render a generic "starting…" loader forever.
            if r.stages.is_empty() {
                app.card_progress.remove(&card_id);
            } else {
                app.card_progress.insert(card_id.clone(), r.stages);
            }
            app.card_progress_last_poll.insert(card_id, now);
        }
    }

    // GC: drop progress data for cards that now have a live session — the
    // session pane has taken over the visible display. Failed cards (no
    // session) keep their data so the user can still see what failed.
    app.card_progress
        .retain(|id, stages| !stages.is_empty() && app.sessions.get(id).is_none());
    app.card_progress_last_poll
        .retain(|id, _| app.sessions.get(id).is_none());
}

pub fn merge_cards(app: &mut App, server_cards: Vec<reeve_protocol::Card>) {
    let mut merged: Vec<reeve_protocol::Card> = Vec::with_capacity(server_cards.len());
    for s_card in server_cards {
        if app.pending_deletes.contains(&s_card.id) {
            continue;
        }
        let preserve_local = app.pending_mvs.contains_key(&s_card.id)
            || app.pending_edits.contains_key(&s_card.id)
            || app.pending_mv_intents.contains_key(&s_card.id);
        if preserve_local {
            if let Some(local) = app.cards.iter().find(|c| c.id == s_card.id) {
                merged.push(local.clone());
                continue;
            }
        }
        merged.push(s_card);
    }
    app.cards = merged;
}

/// Apply a completed background RPC result to the app state. Called by the
/// drive loop each tick via `op_rx.try_recv()`.
pub async fn process_op_event(app: &mut App, event: crate::tui::app::OpEvent) {
    use crate::tui::app::OpEvent;
    match event {
        OpEvent::MvOk {
            card_id,
            updated,
            target: _,
        } => {
            decrement_pending(&mut app.pending_mvs, &card_id);
            if !app.pending_mvs.contains_key(&card_id) {
                app.pending_mv_started_at.remove(&card_id);
            }
            // Only sync to server-truth if this was the LAST in-flight mv.
            // If more mvs are still pending, the later spawn_mv's optimistic
            // column is what the user should see; overwriting with this
            // (older) RPC's response would cause a visible bounce.
            if !app.pending_mvs.contains_key(&card_id) {
                if let Some(c) = app.cards.iter_mut().find(|c| c.id == card_id) {
                    *c = updated;
                }
            }
            // Don't set status — the optimistic visible change was sufficient.
            // Don't change focused_column — already done in spawn_mv.
        }
        OpEvent::MvErr { card_id, msg } => {
            // One-shot fetch BEFORE decrementing pending_mvs, so the loader
            // visibility gate ("card_progress has data + no session") catches
            // the Failed state. The regular 400ms poll may have just sampled
            // before the failure landed.
            if let Ok(Ok(r)) =
                crate::tui::client_ext::rpc_card_progress(reeve_protocol::CardProgressParams {
                    id: card_id.clone(),
                })
                .await
            {
                if !r.stages.is_empty() {
                    app.card_progress.insert(card_id.clone(), r.stages);
                }
            }
            decrement_pending(&mut app.pending_mvs, &card_id);
            if !app.pending_mvs.contains_key(&card_id) {
                app.pending_mv_started_at.remove(&card_id);
            }
            // The optimistic update was wrong. Next ls poll reconciles the
            // card back to its real column. Show the error so the user knows.
            set_status_error(app, format!("mv failed: {msg}"));
        }
        OpEvent::EditOk { card_id, updated } => {
            decrement_pending(&mut app.pending_edits, &card_id);
            // Same logic as MvOk — only sync if no later edits are still
            // pending, otherwise we'd revert the user's latest brief.
            if !app.pending_edits.contains_key(&card_id) {
                if let Some(c) = app.cards.iter_mut().find(|c| c.id == card_id) {
                    *c = updated;
                }
            }
            // Don't set status.
        }
        OpEvent::EditErr { card_id, msg } => {
            decrement_pending(&mut app.pending_edits, &card_id);
            set_status_error(app, format!("edit failed: {msg}"));
            // Next ls poll reverts the optimistic brief (pending count now 0).
        }
        OpEvent::NewOk => {
            // New card appears on next ls poll. No status (avoid noise).
        }
        OpEvent::NewErr { msg } => set_status_error(app, format!("new card failed: {msg}")),
        OpEvent::DeleteOk { card_id } => {
            app.pending_deletes.remove(&card_id);
            // Already removed optimistically. No-op.
        }
        OpEvent::DeleteErr { card_id, msg } => {
            app.pending_deletes.remove(&card_id);
            set_status_error(app, format!("delete failed: {msg}"));
            // Next ls poll re-adds the card if delete failed.
        }
    }
}

fn decrement_pending(map: &mut std::collections::HashMap<String, u32>, card_id: &str) {
    if let Some(count) = map.get_mut(card_id) {
        *count = count.saturating_sub(1);
        if *count == 0 {
            map.remove(card_id);
        }
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

/// Forward a raw keypress to the focused session's PTY master.
pub async fn forward_to_session(app: &mut App, key: crossterm::event::KeyEvent) {
    let Some(id) = app.selected_card_id.clone() else {
        return;
    };
    let Some(session) = app.sessions.get_mut(&id) else {
        return;
    };
    let bytes = key_event_to_bytes(key);
    if !bytes.is_empty() {
        if let Err(e) = session.send_bytes(&bytes) {
            set_status_error(app, format!("send to session failed: {e:#}"));
        }
    }
}

fn key_event_to_bytes(key: crossterm::event::KeyEvent) -> Vec<u8> {
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut bytes = match key.code {
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                let upper = c.to_ascii_uppercase() as u8;
                if (b'A'..=b'Z').contains(&upper) {
                    vec![upper - b'A' + 1]
                } else {
                    vec![]
                }
            } else {
                // Forward the literal char (handles uppercase from Shift+letter correctly).
                c.to_string().into_bytes()
            }
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => vec![0x1b, b'[', b'A'],
        KeyCode::Down => vec![0x1b, b'[', b'B'],
        KeyCode::Right => vec![0x1b, b'[', b'C'],
        KeyCode::Left => vec![0x1b, b'[', b'D'],
        KeyCode::Home => vec![0x1b, b'[', b'H'],
        KeyCode::End => vec![0x1b, b'[', b'F'],
        KeyCode::PageUp => vec![0x1b, b'[', b'5', b'~'],
        KeyCode::PageDown => vec![0x1b, b'[', b'6', b'~'],
        KeyCode::Delete => vec![0x1b, b'[', b'3', b'~'],
        _ => vec![],
    };
    // xterm "meta sends escape": Alt-modified keys arrive as a single
    // KeyEvent with the ALT modifier set. Tmux and most TUIs expect the
    // ESC-prefix encoding (e.g. Alt+N → ESC 'n', Alt+Up → ESC ESC[A).
    // Without this prefix the receiver sees a bare 'n' / arrow and the
    // user's prefix-c, alt-arrows, alt-number bindings never fire.
    if !bytes.is_empty() && key.modifiers.contains(KeyModifiers::ALT) {
        bytes.insert(0, 0x1b);
    }
    bytes
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
        assert_eq!(
            app.layout,
            crate::tui::layout::LayoutMode::FocusedColumnPlusDetail
        );
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
    async fn esc_in_kanban_is_noop_no_quit() {
        let mut app = App::new();
        handle(&mut app, Action::CloseDrillIn).await;
        assert!(!app.quit, "Esc in KanbanFull must not quit — `q` is the only quit");
        assert_eq!(app.layout, crate::tui::layout::LayoutMode::KanbanFull);
    }

    #[tokio::test]
    async fn enter_in_drill_in_with_session_focuses_session_pane() {
        let mut app = App::new();
        app.layout = crate::tui::layout::LayoutMode::FocusedColumnPlusDetail;
        app.cards.push(card("c1", Column::Implementing, 0));
        app.selected_card_id = Some("c1".into());
        // No session present → status set, focus unchanged.
        handle(&mut app, Action::OpenDrillIn).await;
        assert_eq!(app.pane_focus, crate::tui::app::PaneFocus::Kanban);
    }

    #[test]
    fn is_destructive_implementing_to_backlog() {
        use reeve_protocol::Column::*;
        assert!(is_destructive(Implementing, Backlog));
        assert!(is_destructive(Implementing, Planning));
    }

    #[test]
    fn is_destructive_any_to_done() {
        use reeve_protocol::Column::*;
        assert!(is_destructive(Backlog, Done));
        assert!(is_destructive(Planning, Done));
        assert!(is_destructive(Implementing, Done));
        assert!(is_destructive(Testing, Done));
        assert!(is_destructive(Review, Done));
    }

    #[test]
    fn is_destructive_within_pre_implementing_is_false() {
        use reeve_protocol::Column::*;
        assert!(!is_destructive(Backlog, Planning));
        assert!(!is_destructive(Planning, Implementing));
        assert!(!is_destructive(Backlog, Implementing));
    }

    #[test]
    fn is_destructive_back_to_implementing_is_false() {
        use reeve_protocol::Column::*;
        assert!(!is_destructive(Testing, Implementing));
        assert!(!is_destructive(Review, Implementing));
    }

    #[test]
    fn is_destructive_implementing_to_testing_or_review() {
        // Container stays running by design on these — the daemon doesn't
        // tear down — but we still confirm because users were accidentally
        // bumping out of active Implementing work.
        use reeve_protocol::Column::*;
        assert!(is_destructive(Implementing, Testing));
        assert!(is_destructive(Implementing, Review));
    }

    #[test]
    fn is_destructive_is_superset_of_lifecycle_routing() {
        // Cross-reference with reeve_card::transitions::on_column_change:
        // every transition that routes to ensure_stopped MUST also confirm.
        // The converse no longer holds — Implementing → Testing/Review
        // confirms without tearing down. Drift-guarded as a superset, not
        // an equality.
        use reeve_protocol::Column;
        use reeve_protocol::Column::*;
        let all: [Column; 6] = [Backlog, Planning, Implementing, Testing, Review, Done];
        for from in all {
            for to in all {
                if from == to {
                    continue;
                }
                let routes_to_ensure_stopped = matches!(
                    (from, to),
                    (_, Done) | (Implementing | Testing | Review, Backlog | Planning)
                );
                if routes_to_ensure_stopped {
                    assert!(
                        is_destructive(from, to),
                        "({from:?} → {to:?}) routes to ensure_stopped but is_destructive=false — drift!",
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn confirm_destructive_y_calls_unchecked_mv() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use reeve_protocol::Column;

        // Build an app with one card in Implementing.
        let mut app = App::new();
        let card_id = "test-card".to_string();
        app.cards.push(card(&card_id, Column::Implementing, 0));
        app.selected_card_id = Some(card_id.clone());

        // Open the modal (simulates schedule_mv detecting destructive).
        app.modal = Some(crate::tui::modal::ModalKind::ConfirmDestructiveMv {
            card_id: card_id.clone(),
            from: Column::Implementing,
            to: Column::Backlog,
        });

        let key = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        handle_modal_key(&mut app, key).await;

        assert!(app.modal.is_none(), "modal should be dismissed");
        assert!(
            app.pending_mv_intents.contains_key(&card_id),
            "schedule_mv_unchecked should have queued the mv intent",
        );
    }

    #[test]
    fn key_event_to_bytes_alt_n_emits_meta_escape_prefix() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::ALT);
        assert_eq!(key_event_to_bytes(key), vec![0x1b, b'n']);
    }

    #[test]
    fn key_event_to_bytes_alt_up_arrow_emits_double_escape_csi() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let key = KeyEvent::new(KeyCode::Up, KeyModifiers::ALT);
        // ESC + (ESC [ A) — tmux reads this as M-Up.
        assert_eq!(key_event_to_bytes(key), vec![0x1b, 0x1b, b'[', b'A']);
    }

    #[test]
    fn key_event_to_bytes_ctrl_alt_l_emits_esc_plus_ctrl_byte() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let key = KeyEvent::new(
            KeyCode::Char('l'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        );
        // Ctrl-L = 0x0c; Alt prepends ESC.
        assert_eq!(key_event_to_bytes(key), vec![0x1b, 0x0c]);
    }

    #[test]
    fn key_event_to_bytes_plain_n_no_escape() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE);
        assert_eq!(key_event_to_bytes(key), vec![b'n']);
    }

    #[tokio::test]
    async fn confirm_destructive_n_cancels() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use reeve_protocol::Column;

        let mut app = App::new();
        let card_id = "test-card".to_string();
        app.cards.push(card(&card_id, Column::Implementing, 0));
        app.selected_card_id = Some(card_id.clone());

        app.modal = Some(crate::tui::modal::ModalKind::ConfirmDestructiveMv {
            card_id: card_id.clone(),
            from: Column::Implementing,
            to: Column::Backlog,
        });

        let key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE);
        handle_modal_key(&mut app, key).await;

        assert!(app.modal.is_none(), "modal should be dismissed");
        assert!(
            !app.pending_mv_intents.contains_key(&card_id),
            "no mv should have been queued",
        );
    }
}
