use crate::client;
use crate::tui::events::{self, Action};
use crate::tui::render;
use anyhow::Result;
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use reeve_protocol::{Card, LsResult, paths};
use std::io::{Stdout, stdout};
use std::time::{Duration, Instant};

pub type Term = Terminal<CrosstermBackend<Stdout>>;

pub struct App {
    pub cards: Vec<Card>,
    pub last_refresh: Instant,
    pub quit: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            cards: Vec::new(),
            last_refresh: Instant::now(),
            quit: false,
        }
    }
}

pub async fn run_loop() -> Result<()> {
    let mut terminal = setup_terminal()?;
    let result = drive(&mut terminal).await;
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

async fn drive(terminal: &mut Term) -> Result<()> {
    let mut app = App::new();
    app.cards = fetch_cards().await.unwrap_or_default();
    app.last_refresh = Instant::now();

    while !app.quit {
        terminal.draw(|f| render::render(f, &app.cards))?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if let Some(action) = events::translate(key) {
                        handle_action(&mut app, action).await;
                    }
                }
            }
        }

        if app.last_refresh.elapsed() > Duration::from_millis(500) {
            app.cards = fetch_cards().await.unwrap_or_default();
            app.last_refresh = Instant::now();
        }
    }
    Ok(())
}

async fn handle_action(app: &mut App, action: Action) {
    match action {
        Action::Quit => app.quit = true,
        Action::ForceRefresh => {
            app.cards = fetch_cards().await.unwrap_or_default();
            app.last_refresh = Instant::now();
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
