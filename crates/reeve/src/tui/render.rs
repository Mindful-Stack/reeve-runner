use crate::tui::app::App;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use reeve_protocol::{Card, Column};
use tui_term::widget::PseudoTerminal;

pub fn render(f: &mut ratatui::Frame<'_>, app: &App) {
    let area = f.area();

    // Daemon-down overlay short-circuits the entire layout — when the
    // daemon is unreachable, none of the kanban state is trustworthy.
    if app.daemon_unreachable {
        render_daemon_down(f, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    f.render_widget(title_line(app), chunks[0]);

    match app.layout {
        crate::tui::layout::LayoutMode::KanbanFull => render_kanban_full(f, chunks[1], app),
        crate::tui::layout::LayoutMode::FocusedColumnPlusDetail => {
            render_focused_plus_detail(f, chunks[1], app)
        }
    }
    render_status_bar(f, chunks[2], app);
    render_modal(f, area, app);
}

fn render_daemon_down(f: &mut ratatui::Frame<'_>, area: Rect) {
    let popup = centered_rect(60, 40, area);
    let lines = vec![
        Line::raw(""),
        Line::from(Span::styled(
            "Cannot connect to the reeve daemon",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
        Line::raw(""),
        Line::from("Start it in another terminal:"),
        Line::raw(""),
        Line::from(Span::styled("  reeved", Style::default().fg(Color::Yellow))),
        Line::raw(""),
        Line::from("then press 'r' to retry."),
        Line::raw(""),
        Line::from(Span::styled(
            "'q' to quit",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let para = Paragraph::new(lines)
        .alignment(ratatui::layout::Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red))
                .title("daemon not running"),
        );
    // Clear the underlying frame so previous content doesn't bleed through.
    f.render_widget(ratatui::widgets::Clear, area);
    f.render_widget(para, popup);
}

fn render_status_bar(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    if let Some(s) = &app.status {
        let color = match s.level {
            crate::tui::app::StatusLevel::Info => Color::White,
            crate::tui::app::StatusLevel::Warn => Color::Yellow,
            crate::tui::app::StatusLevel::Error => Color::Red,
        };
        let p = Paragraph::new(Span::styled(s.text.clone(), Style::default().fg(color)));
        f.render_widget(p, area);
    }
}

fn title_line(app: &App) -> Paragraph<'static> {
    // Session-pane focus overrides layout-based hints — keys are forwarded
    // to the PTY, so kanban bindings don't apply.
    let hint = if app.pane_focus == crate::tui::app::PaneFocus::SessionPane {
        "Alt+Esc back to kanban  (other keys forwarded to session)"
    } else {
        match app.layout {
            crate::tui::layout::LayoutMode::KanbanFull => {
                "q quit  r refresh  Enter drill-in  h/j/k/l navigate"
            }
            crate::tui::layout::LayoutMode::FocusedColumnPlusDetail => {
                "Esc close drill-in  j/k navigate  Enter focus session"
            }
        }
    };
    Paragraph::new(Line::from(vec![
        Span::styled("reeve", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::raw(hint),
    ]))
}

fn render_kanban_full(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    render_all_columns(f, area, app);
}

fn render_focused_plus_detail(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let kanban = app.sidebar_width_pct;
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(kanban),
            Constraint::Percentage(100 - kanban),
        ])
        .split(area);
    render_one_column(f, split[0], app);
    render_detail_pane(f, split[1], app);
}

fn render_all_columns(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let columns = Column::ALL;
    let constraints = vec![Constraint::Percentage(100 / columns.len() as u16); columns.len()];
    let cells = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);
    for (i, col) in columns.iter().enumerate() {
        render_column_into(f, cells[i], app, *col);
    }
}

fn render_one_column(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    render_column_into(f, area, app, app.focused_column);
}

fn render_column_into(f: &mut ratatui::Frame<'_>, area: Rect, app: &App, col: Column) {
    let kanban_has_pane_focus = app.pane_focus == crate::tui::app::PaneFocus::Kanban;
    let is_focused_column = col == app.focused_column;
    let mut filtered: Vec<&Card> = app.cards.iter().filter(|c| c.column == col).collect();
    filtered.sort_by_key(|c| c.created_at);
    let items: Vec<ListItem> = filtered
        .iter()
        .map(|c| {
            card_to_listitem(
                c,
                app.selected_card_id.as_deref() == Some(&c.id) && is_focused_column,
                app.pending_attention.contains(&c.id),
                app.pending_mvs.contains_key(&c.id),
            )
        })
        .collect();
    let count = items.len();
    let title = format!("{} ({count})", col.label());
    // Yellow border only when the kanban actually owns input focus AND this
    // is the active column. Otherwise grey — so Tab-to-session-pane makes
    // the kanban side visibly dim.
    let border_style = if kanban_has_pane_focus && is_focused_column {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .title(title),
        )
        .style(Style::default().fg(Color::White));
    f.render_widget(list, area);
}

fn render_detail_pane(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let Some(card_id) = &app.selected_card_id else {
        let placeholder = Paragraph::new("(no card selected)")
            .block(Block::default().borders(Borders::ALL).title("detail"));
        f.render_widget(placeholder, area);
        return;
    };
    let card = app.cards.iter().find(|c| &c.id == card_id);
    let Some(card) = card else {
        let placeholder = Paragraph::new("(card vanished)")
            .block(Block::default().borders(Borders::ALL).title("detail"));
        f.render_widget(placeholder, area);
        return;
    };

    // 1. Live session takes priority — column-gated (only show when card
    //    is in Implementing AND we have a session).
    if card.column == Column::Implementing {
        if let Some(session) = app.sessions.get(&card.id) {
            // Fit the PTY + parser to the pane's inner area (minus border).
            // Cheap no-op when the size hasn't changed; on actual resizes it
            // issues SIGWINCH so tmux / claude redraw at the right width.
            let inner_rows = area.height.saturating_sub(2);
            let inner_cols = area.width.saturating_sub(2);
            let _ = session.resize(inner_rows, inner_cols);
            if let Ok(parser) = session.parser.lock() {
                let screen = parser.screen();
                let border_style = if app.pane_focus == crate::tui::app::PaneFocus::SessionPane {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                let widget = PseudoTerminal::new(screen).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(border_style)
                        .title(format!(
                            "session ({})",
                            card.id.get(..8).unwrap_or(&card.id)
                        )),
                );
                f.render_widget(widget, area);
                return;
            }
        }
    }

    // 2. Progress data available → loader. NOT gated on card.column or
    //    pending_mvs. A failed spawn whose optimistic column has reverted
    //    still surfaces the Failed checklist while the user is on the card.
    //    Empty-stage entries don't count — those leave the user staring at
    //    a generic "starting…" spinner for a card that isn't doing anything.
    if app
        .card_progress
        .get(&card.id)
        .is_some_and(|s| !s.is_empty())
    {
        render_provisioning_loader(f, area, app, &card.id);
        return;
    }

    // Fallback: card detail block.
    let lines = vec![
        Line::from(vec![
            Span::styled("id:        ", Style::default().fg(Color::DarkGray)),
            Span::raw(card.id.clone()),
        ]),
        Line::from(vec![
            Span::styled("brief:     ", Style::default().fg(Color::DarkGray)),
            Span::raw(card.brief.clone()),
        ]),
        Line::from(vec![
            Span::styled("household: ", Style::default().fg(Color::DarkGray)),
            Span::raw(card.household.clone()),
        ]),
        Line::from(vec![
            Span::styled("column:    ", Style::default().fg(Color::DarkGray)),
            Span::raw(card.column.label().to_string()),
        ]),
        Line::from(vec![
            Span::styled("created:   ", Style::default().fg(Color::DarkGray)),
            Span::raw(card.created_at.to_string()),
        ]),
        Line::from(vec![
            Span::styled("updated:   ", Style::default().fg(Color::DarkGray)),
            Span::raw(card.updated_at.to_string()),
        ]),
    ];
    let para = Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("detail"));
    f.render_widget(para, area);
}

/// Drill-in loader shown while the daemon is provisioning the workspace for a
/// card. Renders a per-stage checklist sourced from `app.card_progress`; falls
/// back to a generic animated spinner when no stages have arrived yet (first
/// ~400 ms after the mv RPC fires).
fn render_provisioning_loader(f: &mut ratatui::Frame<'_>, area: Rect, app: &App, card_id: &str) {
    let stages: &[reeve_protocol::ProgressStage] = app
        .card_progress
        .get(card_id)
        .map(|v| v.as_slice())
        .unwrap_or(&[]);

    let elapsed = elapsed_str_from(&app.pending_mv_started_at, card_id);

    let mut lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Provisioning workspace…",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    if stages.is_empty() {
        // No progress events yet — show a generic animated spinner.
        lines.push(Line::from(Span::styled(
            format!("  {}  starting…", spinner_frame()),
            Style::default().fg(Color::Yellow),
        )));
    } else {
        for stage in stages {
            let label = format_stage(&stage.kind);
            let span = match stage.status {
                reeve_protocol::ProgressStatus::Completed => {
                    Span::styled(format!("  ✓ {label}"), Style::default().fg(Color::Green))
                }
                reeve_protocol::ProgressStatus::Active => Span::styled(
                    format!("  {} {label}", spinner_frame()),
                    Style::default().fg(Color::Yellow),
                ),
                reeve_protocol::ProgressStatus::Failed => {
                    Span::styled(format!("  ✗ {label}"), Style::default().fg(Color::Red))
                }
            };
            lines.push(Line::from(span));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  {elapsed}"),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  First-time provisioning of a card typically takes 30–90 seconds.",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        "  Subsequent reprovisions reuse the cached image.",
        Style::default().fg(Color::DarkGray),
    )));

    let title_id = card_id.get(..8.min(card_id.len())).unwrap_or(card_id);
    let para = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" card {} — provisioning ", title_id)),
    );
    f.render_widget(para, area);
}

fn format_stage(kind: &reeve_protocol::ProgressKind) -> String {
    match kind {
        reeve_protocol::ProgressKind::CloneMeta { name } => format!("Cloning meta-repo {name}"),
        reeve_protocol::ProgressKind::CloneSibling { name } => format!("Cloning sibling {name}"),
        reeve_protocol::ProgressKind::DevcontainerUp => "Building container".into(),
        reeve_protocol::ProgressKind::StartingTmux => "Starting session".into(),
        reeve_protocol::ProgressKind::Done => "Ready".into(),
    }
}

fn spinner_frame() -> &'static str {
    const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    FRAMES[(ms / 80) as usize % FRAMES.len()]
}

fn elapsed_str_from(
    started: &std::collections::HashMap<String, std::time::Instant>,
    card_id: &str,
) -> String {
    match started.get(card_id) {
        Some(t) => {
            let s = t.elapsed().as_secs();
            if s < 60 {
                format!("{s}s elapsed")
            } else {
                format!("{}m {}s elapsed", s / 60, s % 60)
            }
        }
        None => "elapsed: —".into(),
    }
}

pub fn render_modal(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let Some(modal) = &app.modal else {
        return;
    };
    match modal {
        crate::tui::modal::ModalKind::ActionMenu { selected_index } => {
            render_action_menu(f, area, app, *selected_index);
        }
        crate::tui::modal::ModalKind::Help => {
            render_help(f, area);
        }
        crate::tui::modal::ModalKind::MoveColumn { choice, .. } => {
            render_move_picker(f, area, *choice);
        }
        crate::tui::modal::ModalKind::EditBrief {
            buffer,
            cursor,
            error,
            ..
        } => {
            render_edit_modal(f, area, buffer, *cursor, error.as_deref());
        }
        crate::tui::modal::ModalKind::DeleteConfirm { card_id } => {
            let brief = app
                .cards
                .iter()
                .find(|c| &c.id == card_id)
                .map(|c| c.brief.as_str())
                .unwrap_or("");
            render_delete_confirm(f, area, card_id, brief);
        }
        crate::tui::modal::ModalKind::NewCard {
            household,
            brief,
            cursor,
            household_picker_index,
            error,
        } => {
            render_new_modal(
                f,
                area,
                household.as_deref(),
                brief,
                *cursor,
                *household_picker_index,
                error.as_deref(),
                &app.households,
            );
        }
        crate::tui::modal::ModalKind::ConfirmDestructiveMv {
            card_id: _,
            from,
            to,
        } => {
            use reeve_protocol::Column::*;
            // 7 content lines + 2 borders = 9 rows; +2 headroom = 11.
            // Fixed height because percentage-of-area clipped the options
            // hint on smaller terminals.
            let popup = centered_rect_fixed_height(60, 11, area);
            let from_label = from.label();
            let to_label = to.label();
            // Three cases:
            // - Implementing → Testing|Review: container survives (Testing
            //   and Review reuse the same env) but we confirm to protect
            //   active work from accidental Shift+L.
            // - Implementing/Testing/Review → Backlog|Planning|Done: full
            //   teardown — container and claude session both die.
            // - Backlog/Planning → Done: nothing to tear down, just an
            //   archive.
            let (line1, line2, title) = match (*from, *to) {
                (Implementing, Testing | Review) => (
                    "  This ends the implementation phase. The container",
                    "  stays running for testing / review.",
                    " End implementation phase? ",
                ),
                (Implementing | Testing | Review, _) => (
                    "  This will tear down the container and end the",
                    "  claude session for this card.",
                    " Tear down workspace? ",
                ),
                _ => (
                    "  This card has no active container — moving to",
                    "  Done archives it without further action.",
                    " Archive card? ",
                ),
            };
            let lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    format!("  Move from {from_label} to {to_label}?"),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(line1, Style::default().fg(Color::DarkGray))),
                Line::from(Span::styled(line2, Style::default().fg(Color::DarkGray))),
                Line::from(""),
                Line::from(Span::styled(
                    "  Continue?    [y]es / Enter    [n]o / Esc",
                    Style::default().fg(Color::Yellow),
                )),
            ];
            let body = Paragraph::new(lines)
                .alignment(ratatui::layout::Alignment::Center)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Red))
                        .title(title),
                );
            f.render_widget(ratatui::widgets::Clear, popup);
            f.render_widget(body, popup);
        }
    }
}

/// Center a rect with a fixed pixel height (in rows) and a percentage
/// width. For modals whose content is a known fixed line count — using
/// percentage-of-area clips on small terminals.
fn centered_rect_fixed_height(percent_x: u16, rows: u16, area: Rect) -> Rect {
    let target_h = rows.min(area.height);
    let top_h = area.height.saturating_sub(target_h) / 2;
    let bottom_h = area.height.saturating_sub(top_h).saturating_sub(target_h);
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(top_h),
            Constraint::Length(target_h),
            Constraint::Length(bottom_h),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn render_action_menu(f: &mut ratatui::Frame<'_>, area: Rect, app: &App, selected_index: usize) {
    let card_col = app
        .selected_card_id
        .as_ref()
        .and_then(|id| app.cards.iter().find(|c| &c.id == id))
        .map(|c| c.column);
    let items = crate::tui::modal::build_action_menu(card_col);
    let area = centered_rect(50, 50, area);
    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let style = if i == selected_index {
                Style::default().bg(Color::Yellow).fg(Color::Black)
            } else if item.enabled {
                Style::default()
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let label = if let Some(reason) = item.disabled_reason {
                format!(
                    "  {}  {} (disabled: {})",
                    item.key_hint, item.description, reason
                )
            } else {
                format!("  {}  {}", item.key_hint, item.description)
            };
            ListItem::new(Line::from(Span::styled(label, style)))
        })
        .collect();
    let list = List::new(list_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title("actions  (esc to close)"),
    );
    f.render_widget(ratatui::widgets::Clear, area);
    f.render_widget(list, area);
}

fn render_move_picker(f: &mut ratatui::Frame<'_>, area: Rect, choice: Column) {
    let area = centered_rect(30, 40, area);
    let items: Vec<ListItem> = Column::ALL
        .iter()
        .enumerate()
        .map(|(i, col)| {
            let style = if *col == choice {
                Style::default().bg(Color::Yellow).fg(Color::Black)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(Span::styled(
                format!("  {}  {}", i + 1, col.label()),
                style,
            )))
        })
        .collect();
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title("move to column  (1-6 jump · ↑/↓ · enter · esc)"),
    );
    f.render_widget(ratatui::widgets::Clear, area);
    f.render_widget(list, area);
}

fn render_help(f: &mut ratatui::Frame<'_>, area: Rect) {
    let area = centered_rect(60, 70, area);
    let bindings = crate::tui::events::BINDINGS;

    // Group consecutive bindings that share (section, description) into one
    // row, joining their key labels with " / ". This is what surfaces "h / ←"
    // instead of two separate rows.
    let mut lines: Vec<Line> = Vec::new();
    let mut last_section: Option<crate::tui::events::HelpSection> = None;
    let mut i = 0;
    while i < bindings.len() {
        let b = &bindings[i];
        if Some(b.section) != last_section {
            lines.push(Line::from(Span::styled(
                format!("\n{:?}", b.section),
                Style::default().add_modifier(Modifier::BOLD),
            )));
            last_section = Some(b.section);
        }

        // Collect all consecutive bindings with the same description in this
        // section. The BINDINGS const is ordered to group these naturally
        // (e.g. h / Left appear back-to-back for the same FocusPrevColumn).
        let mut keys: Vec<String> = vec![key_label(b.key)];
        let mut j = i + 1;
        while j < bindings.len()
            && bindings[j].section == b.section
            && bindings[j].description == b.description
        {
            keys.push(key_label(bindings[j].key));
            j += 1;
        }
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<14}", keys.join(" / ")),
                Style::default().fg(Color::Cyan),
            ),
            Span::raw(b.description),
        ]));
        i = j;
    }

    let para = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title("help  (esc/?/q to close)"),
    );
    f.render_widget(ratatui::widgets::Clear, area);
    f.render_widget(para, area);
}

fn key_label(matcher: crate::tui::events::KeyMatcher) -> String {
    use crossterm::event::KeyCode;
    match matcher {
        crate::tui::events::KeyMatcher::Char(' ') => "Space".to_string(),
        crate::tui::events::KeyMatcher::Char(c) => c.to_string(),
        crate::tui::events::KeyMatcher::Code(KeyCode::Left) => "←".to_string(),
        crate::tui::events::KeyMatcher::Code(KeyCode::Right) => "→".to_string(),
        crate::tui::events::KeyMatcher::Code(KeyCode::Up) => "↑".to_string(),
        crate::tui::events::KeyMatcher::Code(KeyCode::Down) => "↓".to_string(),
        crate::tui::events::KeyMatcher::Code(KeyCode::PageUp) => "PgUp".to_string(),
        crate::tui::events::KeyMatcher::Code(KeyCode::PageDown) => "PgDn".to_string(),
        crate::tui::events::KeyMatcher::Code(c) => format!("{c:?}"),
        crate::tui::events::KeyMatcher::Shift(KeyCode::Left) => "Shift+←".to_string(),
        crate::tui::events::KeyMatcher::Shift(KeyCode::Right) => "Shift+→".to_string(),
        crate::tui::events::KeyMatcher::Shift(KeyCode::Char(c)) => {
            format!("Shift+{}", c.to_ascii_uppercase())
        }
        crate::tui::events::KeyMatcher::Shift(c) => format!("Shift+{c:?}"),
        crate::tui::events::KeyMatcher::Ctrl(KeyCode::Char(c)) => {
            format!("Ctrl+{}", c.to_ascii_uppercase())
        }
        crate::tui::events::KeyMatcher::Ctrl(c) => format!("Ctrl+{c:?}"),
        crate::tui::events::KeyMatcher::Alt(KeyCode::Char(c)) => {
            format!("Alt+{}", c.to_ascii_uppercase())
        }
        crate::tui::events::KeyMatcher::Alt(c) => format!("Alt+{c:?}"),
    }
}

fn render_edit_modal(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    buffer: &str,
    cursor: usize,
    error: Option<&str>,
) {
    let popup_area = centered_rect(60, 30, area);
    f.render_widget(ratatui::widgets::Clear, popup_area);

    let mut lines: Vec<Line> = vec![Line::from(Span::raw(""))];
    lines.push(build_buffer_line(buffer, cursor));
    if let Some(err) = error {
        lines.push(Line::from(Span::raw("")));
        lines.push(Line::from(Span::styled(
            format!("  error: {err}"),
            Style::default().fg(Color::Red),
        )));
    }
    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(Span::styled(
        "  Enter save  Esc cancel",
        Style::default().fg(Color::DarkGray),
    )));

    let para =
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("edit brief"));
    f.render_widget(para, popup_area);
}

fn render_delete_confirm(f: &mut ratatui::Frame<'_>, area: Rect, _card_id: &str, brief: &str) {
    let popup_area = centered_rect(50, 25, area);
    f.render_widget(ratatui::widgets::Clear, popup_area);

    let lines = vec![
        Line::from(Span::raw("")),
        Line::from(vec![
            Span::styled("  delete: ", Style::default().fg(Color::DarkGray)),
            Span::raw(brief),
        ]),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            "  y/Enter confirm  n/Esc cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let para = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red))
            .title("delete card?"),
    );
    f.render_widget(para, popup_area);
}

fn render_new_modal(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    household: Option<&str>,
    brief: &str,
    cursor: usize,
    household_picker_index: usize,
    error: Option<&str>,
    households: &[reeve_protocol::HouseholdSummary],
) {
    let popup_area = centered_rect(60, 40, area);
    f.render_widget(ratatui::widgets::Clear, popup_area);

    // Household picker step.
    if household.is_none() && households.len() > 1 {
        let items: Vec<ListItem> = households
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let style = if i == household_picker_index {
                    Style::default().bg(Color::Yellow).fg(Color::Black)
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(Span::styled(format!("  {}", h.name), style)))
            })
            .collect();
        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title("new card — pick household  (Enter select, Esc cancel)"),
        );
        f.render_widget(list, popup_area);
        return;
    }

    // Brief input step.
    let hh_label = household.unwrap_or("(none)");
    let mut lines: Vec<Line> = vec![
        Line::from(Span::raw("")),
        Line::from(vec![
            Span::styled("  household: ", Style::default().fg(Color::DarkGray)),
            Span::raw(hh_label),
        ]),
        Line::from(Span::raw("")),
        Line::from(vec![Span::styled(
            "  brief: ",
            Style::default().fg(Color::DarkGray),
        )]),
        build_buffer_line(brief, cursor),
    ];
    if let Some(err) = error {
        lines.push(Line::from(Span::raw("")));
        lines.push(Line::from(Span::styled(
            format!("  error: {err}"),
            Style::default().fg(Color::Red),
        )));
    }
    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(Span::styled(
        "  Enter create  Esc cancel",
        Style::default().fg(Color::DarkGray),
    )));

    let para =
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("new card"));
    f.render_widget(para, popup_area);
}

/// Build a `Line` that shows `buffer` with a yellow-bg cursor cell at `cursor`.
fn build_buffer_line(buffer: &str, cursor: usize) -> Line<'static> {
    let chars: Vec<char> = buffer.chars().collect();
    let len = chars.len();

    let mut spans: Vec<Span<'static>> = Vec::new();
    // Leading spaces for indentation.
    spans.push(Span::raw("  "));

    if len == 0 {
        // Empty buffer: show cursor as a single space.
        spans.push(Span::styled(
            " ",
            Style::default().bg(Color::Yellow).fg(Color::Black),
        ));
    } else {
        // Characters before cursor.
        if cursor > 0 {
            spans.push(Span::raw(chars[..cursor].iter().collect::<String>()));
        }
        if cursor < len {
            // Character under cursor.
            spans.push(Span::styled(
                chars[cursor].to_string(),
                Style::default().bg(Color::Yellow).fg(Color::Black),
            ));
            // Characters after cursor.
            if cursor + 1 < len {
                spans.push(Span::raw(chars[cursor + 1..].iter().collect::<String>()));
            }
        } else {
            // Cursor is past end — append a trailing-space cursor cell.
            spans.push(Span::styled(
                " ",
                Style::default().bg(Color::Yellow).fg(Color::Black),
            ));
        }
    }

    Line::from(spans)
}

fn card_to_listitem(
    card: &Card,
    selected: bool,
    has_attention: bool,
    has_pending_mv: bool,
) -> ListItem<'_> {
    let id_short = card.id.get(..8).unwrap_or(&card.id);
    let style = if selected {
        Style::default().fg(Color::Black).bg(Color::Yellow)
    } else {
        Style::default()
    };
    // Badge priority: pending-mv > attention > none. A mv in flight typically
    // takes 30-90s (container provisioning), so the user needs visible feedback
    // that the move is happening rather than wondering whether their keypress
    // registered.
    let badge = if has_pending_mv {
        Span::styled("⟳ ", Style::default().fg(Color::Yellow))
    } else if has_attention {
        Span::styled("• ", Style::default().fg(Color::Cyan))
    } else {
        Span::raw("  ")
    };
    let line = Line::from(vec![
        badge,
        Span::styled(format!("{id_short} "), Style::default().fg(Color::DarkGray)),
        Span::styled(card.brief.as_str(), style),
    ]);
    ListItem::new(line)
}
