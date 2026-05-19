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

    let title = Paragraph::new(Line::from(vec![
        Span::styled("reeve", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::raw("q quit  r refresh  h/l column  j/k select  g/G first/last"),
    ]));
    f.render_widget(title, chunks[0]);

    render_columns(f, chunks[1], app);
}

fn render_columns(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let columns = Column::ALL;
    let constraints = vec![Constraint::Percentage(100 / columns.len() as u16); columns.len()];
    let cells = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    for (i, col) in columns.iter().enumerate() {
        let focused = *col == app.focused_column;
        let border_color = if focused { Color::Yellow } else { Color::DarkGray };

        let mut sorted: Vec<&Card> = app.cards.iter().filter(|c| c.column == *col).collect();
        sorted.sort_by_key(|c| c.created_at);

        let items: Vec<ListItem> = sorted
            .iter()
            .map(|c| card_to_listitem(c, app.selected_card_id.as_deref()))
            .collect();
        let count = items.len();
        let title = format!("{} ({count})", col.label());
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .border_style(Style::default().fg(border_color)),
            )
            .style(Style::default().fg(Color::White));
        f.render_widget(list, cells[i]);
    }
}

fn card_to_listitem<'a>(card: &'a Card, selected_id: Option<&str>) -> ListItem<'a> {
    let id_short = card.id.get(..8).unwrap_or(&card.id);
    let selected = selected_id == Some(card.id.as_str());
    let line = Line::from(vec![
        Span::styled(
            format!("{id_short} "),
            Style::default().fg(if selected { Color::Black } else { Color::DarkGray }),
        ),
        Span::raw(card.brief.as_str()),
    ]);
    if selected {
        ListItem::new(line).style(Style::default().bg(Color::Yellow).fg(Color::Black))
    } else {
        ListItem::new(line)
    }
}
