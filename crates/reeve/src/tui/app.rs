use crate::client;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use reeve_protocol::{Card, LsResult, paths};
use std::io::{Stdout, stdout};
use std::time::{Duration, Instant};

pub(super) type Term = Terminal<CrosstermBackend<Stdout>>;

pub fn run() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let mut terminal = setup_terminal()?;
    let result = runtime.block_on(run_loop(&mut terminal));
    restore_terminal(&mut terminal)?;
    result
}

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

pub(super) async fn run_loop(terminal: &mut Term) -> Result<()> {
    let mut cards: Vec<Card> = fetch_cards().await.unwrap_or_default();
    let mut last_refresh = Instant::now();

    loop {
        terminal.draw(|f| super::render::render(f, &cards))?;

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
