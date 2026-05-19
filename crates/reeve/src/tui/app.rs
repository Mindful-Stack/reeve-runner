use crate::tui::client_ext;
use crate::tui::events;
use crate::tui::render;
use anyhow::Result;
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use reeve_protocol::{Card, Column};
use std::io::{Stdout, stdout};
use std::time::{Duration, Instant};

pub type Term = Terminal<CrosstermBackend<Stdout>>;

#[derive(Debug, Clone, Copy)]
pub enum StatusLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub struct StatusMessage {
    pub text: String,
    pub level: StatusLevel,
    pub expires_at: Instant,
}

pub struct App {
    pub cards: Vec<Card>,
    pub last_refresh: Instant,
    pub quit: bool,
    pub focused_column: Column,
    pub selected_card_id: Option<String>,
    pub layout: crate::tui::layout::LayoutMode,
    pub modal: Option<crate::tui::modal::ModalKind>,
    pub status: Option<StatusMessage>,
    pub households: Vec<reeve_protocol::HouseholdSummary>,
}

impl App {
    pub fn new() -> Self {
        Self {
            cards: Vec::new(),
            last_refresh: Instant::now(),
            quit: false,
            focused_column: Column::Backlog,
            selected_card_id: None,
            layout: crate::tui::layout::LayoutMode::default(),
            modal: None,
            status: None,
            households: Vec::new(),
        }
    }

    pub fn set_status(&mut self, text: String, level: StatusLevel) {
        self.status = Some(StatusMessage {
            text,
            level,
            expires_at: Instant::now() + Duration::from_secs(3),
        });
    }

    pub fn clear_expired_status(&mut self) {
        if let Some(s) = &self.status {
            if s.expires_at <= Instant::now() {
                self.status = None;
            }
        }
    }

    pub fn focused_cards(&self) -> Vec<&Card> {
        let mut v: Vec<&Card> = self.cards.iter().filter(|c| c.column == self.focused_column).collect();
        v.sort_by_key(|c| c.created_at);
        v
    }

    pub fn selected_index(&self) -> Option<usize> {
        let id = self.selected_card_id.as_ref()?;
        self.focused_cards().iter().position(|c| &c.id == id)
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
    if let Ok(Ok(result)) = client_ext::rpc_ls().await {
        app.cards = result.cards;
        app.last_refresh = Instant::now();
        crate::tui::actions::ensure_selection_valid(&mut app);
    }

    while !app.quit {
        app.clear_expired_status();
        terminal.draw(|f| render::render(f, &app))?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if app.modal.is_some() {
                        crate::tui::actions::handle_modal_key(&mut app, key).await;
                    } else if let Some(action) = events::translate(key) {
                        crate::tui::actions::handle(&mut app, action).await;
                    }
                }
            }
        }

        if app.last_refresh.elapsed() > Duration::from_millis(500) {
            if let Ok(Ok(result)) = client_ext::rpc_ls().await {
                app.cards = result.cards;
                app.last_refresh = Instant::now();
                crate::tui::actions::ensure_selection_valid(&mut app);
            }
        }
    }
    Ok(())
}
