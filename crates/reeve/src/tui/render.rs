use crate::tui::app::App;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use reeve_protocol::{Card, Column};

pub fn render(f: &mut ratatui::Frame<'_>, app: &App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);

    let title = title_line(app);
    f.render_widget(title, chunks[0]);

    match app.layout {
        crate::tui::layout::LayoutMode::KanbanFull => render_kanban_full(f, chunks[1], app),
        crate::tui::layout::LayoutMode::FocusedColumnPlusDetail => {
            render_focused_plus_detail(f, chunks[1], app)
        }
    }
    render_modal(f, area, app);
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

fn render_help(f: &mut ratatui::Frame<'_>, area: Rect) {
    let area = centered_rect(60, 70, area);
    let bindings = crate::tui::events::BINDINGS;
    let mut lines: Vec<Line> = Vec::new();
    let mut last_section: Option<crate::tui::events::HelpSection> = None;
    for b in bindings {
        if Some(b.section) != last_section {
            lines.push(Line::from(Span::styled(
                format!("\n{:?}", b.section),
                Style::default().add_modifier(Modifier::BOLD),
            )));
            last_section = Some(b.section);
        }
        let key_label = match b.key {
            crate::tui::events::KeyMatcher::Char(c) => format!("{c}"),
            crate::tui::events::KeyMatcher::Code(c) => format!("{c:?}"),
            crate::tui::events::KeyMatcher::Shift(c) => format!("Shift+{c:?}"),
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {key_label:<12}"), Style::default().fg(Color::Cyan)),
            Span::raw(b.description),
        ]));
    }
    let para = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("help  (esc/?/q to close)"));
    f.render_widget(ratatui::widgets::Clear, area);
    f.render_widget(para, area);
}

fn card_to_listitem(card: &Card, selected: bool) -> ListItem<'_> {
    let id_short = card.id.get(..8).unwrap_or(&card.id);
    let style = if selected {
        Style::default().fg(Color::Black).bg(Color::Yellow)
    } else {
        Style::default()
    };
    let line = Line::from(vec![
        Span::styled(
            format!("{id_short} "),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(card.brief.as_str(), style),
    ]);
    ListItem::new(line)
}
