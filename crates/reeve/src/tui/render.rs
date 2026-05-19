use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use reeve_protocol::{Card, Column};

pub fn render(f: &mut ratatui::Frame<'_>, cards: &[Card]) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);

    let title = Paragraph::new(Line::from(vec![
        Span::styled("reeve", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::raw("q quit  r refresh"),
    ]));
    f.render_widget(title, chunks[0]);

    render_columns(f, chunks[1], cards);
}

fn render_columns(f: &mut ratatui::Frame<'_>, area: Rect, cards: &[Card]) {
    let columns = Column::ALL;
    let constraints = vec![Constraint::Percentage(100 / columns.len() as u16); columns.len()];
    let cells = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    for (i, col) in columns.iter().enumerate() {
        let items: Vec<ListItem> = cards
            .iter()
            .filter(|c| c.column == *col)
            .map(card_to_listitem)
            .collect();
        let count = items.len();
        let title = format!("{} ({count})", col.label());
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .style(Style::default().fg(Color::White));
        f.render_widget(list, cells[i]);
    }
}

fn card_to_listitem(card: &Card) -> ListItem<'_> {
    let id_short = card.id.get(..8).unwrap_or(&card.id);
    let line = Line::from(vec![
        Span::styled(format!("{id_short} "), Style::default().fg(Color::DarkGray)),
        Span::raw(card.brief.as_str()),
    ]);
    ListItem::new(line)
}
