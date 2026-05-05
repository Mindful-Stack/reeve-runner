//! Minimal read-only Ratatui kanban viewer for M1.

use crate::client;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use reeve_protocol::{Card, Column, LsResult, paths};
use std::io::{Stdout, stdout};
use std::time::{Duration, Instant};

pub fn run() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let mut terminal = setup_terminal()?;
    let result = runtime.block_on(run_loop(&mut terminal));
    restore_terminal(&mut terminal)?;
    result
}

type Term = Terminal<CrosstermBackend<Stdout>>;

fn setup_terminal() -> Result<Term> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Term) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

async fn run_loop(terminal: &mut Term) -> Result<()> {
    let mut cards: Vec<Card> = fetch_cards().await.unwrap_or_default();
    let mut last_refresh = Instant::now();

    loop {
        terminal.draw(|f| render(f, &cards))?;

        // Poll for input with a short timeout so we can refresh periodically.
        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('r') => {
                        cards = fetch_cards().await.unwrap_or_default();
                        last_refresh = Instant::now();
                    }
                    _ => {}
                }
            }
        }

        if last_refresh.elapsed() > Duration::from_millis(500) {
            cards = fetch_cards().await.unwrap_or_default();
            last_refresh = Instant::now();
        }
    }
}

async fn fetch_cards() -> Result<Vec<Card>> {
    let socket = paths::socket_path();
    match client::call(&socket, "ls", serde_json::json!({})).await? {
        Ok(value) => {
            let LsResult { cards } = serde_json::from_value(value)?;
            Ok(cards)
        }
        Err(e) => anyhow::bail!("daemon error: {e}"),
    }
}

fn render(f: &mut ratatui::Frame<'_>, cards: &[Card]) {
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
        Span::styled(
            format!("{id_short} "),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw(card.brief.as_str()),
    ]);
    ListItem::new(line)
}
