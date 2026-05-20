use crate::tui::app::App;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use reeve_protocol::{Card, Column};
use tui_term::widget::PseudoTerminal;

pub fn render(f: &mut ratatui::Frame<'_>, app: &App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)])
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
    let hint = match app.layout {
        crate::tui::layout::LayoutMode::KanbanFull => {
            "q quit  r refresh  Enter drill-in  h/j/k/l navigate"
        }
        crate::tui::layout::LayoutMode::FocusedColumnPlusDetail => {
            "Esc close drill-in  j/k navigate"
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
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
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
    let is_focused = col == app.focused_column;
    let mut filtered: Vec<&Card> = app.cards.iter().filter(|c| c.column == col).collect();
    filtered.sort_by_key(|c| c.created_at);
    let items: Vec<ListItem> = filtered
        .iter()
        .map(|c| {
            card_to_listitem(
                c,
                app.selected_card_id.as_deref() == Some(&c.id) && is_focused,
                app.pending_attention.contains(&c.id),
            )
        })
        .collect();
    let count = items.len();
    let title = format!("{} ({count})", col.label());
    let border_style = if is_focused {
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

    if card.column == Column::Implementing {
        if let Some(session) = app.sessions.get(&card.id) {
            if let Ok(parser) = session.parser.lock() {
                let screen = parser.screen();
                let border_style =
                    if app.pane_focus == crate::tui::app::PaneFocus::SessionPane {
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
    let para =
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("detail"));
    f.render_widget(para, area);
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
        crate::tui::modal::ModalKind::EditBrief { buffer, cursor, error, .. } => {
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
    }
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

fn render_action_menu(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    app: &App,
    selected_index: usize,
) {
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
    let list = List::new(list_items)
        .block(Block::default().borders(Borders::ALL).title("actions  (esc to close)"));
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

    let para = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("help  (esc/?/q to close)"));
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

    let para = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("edit brief"));
    f.render_widget(para, popup_area);
}

fn render_delete_confirm(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    _card_id: &str,
    brief: &str,
) {
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
    let para = Paragraph::new(lines)
        .block(
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
                ListItem::new(Line::from(Span::styled(
                    format!("  {}", h.name),
                    style,
                )))
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
        Line::from(vec![
            Span::styled("  brief: ", Style::default().fg(Color::DarkGray)),
        ]),
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

    let para = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("new card"));
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
        spans.push(Span::styled(" ", Style::default().bg(Color::Yellow).fg(Color::Black)));
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
            spans.push(Span::styled(" ", Style::default().bg(Color::Yellow).fg(Color::Black)));
        }
    }

    Line::from(spans)
}

fn card_to_listitem(card: &Card, selected: bool, has_attention: bool) -> ListItem<'_> {
    let id_short = card.id.get(..8).unwrap_or(&card.id);
    let style = if selected {
        Style::default().fg(Color::Black).bg(Color::Yellow)
    } else {
        Style::default()
    };
    let badge = if has_attention {
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
