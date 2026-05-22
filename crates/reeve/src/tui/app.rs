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
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneFocus {
    Kanban,
    SessionPane,
}

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

#[derive(Debug)]
pub enum OpEvent {
    MvOk {
        card_id: String,
        updated: reeve_protocol::Card,
        target: reeve_protocol::Column,
    },
    MvErr {
        card_id: String,
        msg: String,
    },
    EditOk {
        card_id: String,
        updated: reeve_protocol::Card,
    },
    EditErr {
        card_id: String,
        msg: String,
    },
    NewOk,
    NewErr {
        msg: String,
    },
    DeleteOk {
        card_id: String,
    },
    DeleteErr {
        card_id: String,
        msg: String,
    },
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
    pub sessions: crate::tui::sessions::SessionManager,
    pub pane_focus: PaneFocus,
    pub pending_attention: std::collections::HashSet<String>,
    pub op_tx: UnboundedSender<OpEvent>,
    pub op_rx: UnboundedReceiver<OpEvent>,
    /// card_id → number of mv RPCs currently in flight for that card.
    /// While > 0, ls poll results don't overwrite the local column for this card.
    pub pending_mvs: std::collections::HashMap<String, u32>,
    /// Same for edits — preserves the local brief.
    pub pending_edits: std::collections::HashMap<String, u32>,
    /// card_ids whose delete RPC is in flight. ls poll won't re-add them.
    pub pending_deletes: std::collections::HashSet<String>,
    /// Debounce buffer: card_id → (final-target-column, deadline).
    /// Each H/L/m on this card replaces the entry. When `Instant::now()` >= deadline,
    /// the drive loop spawns the actual mv RPC with the latest target.
    pub pending_mv_intents:
        std::collections::HashMap<String, (reeve_protocol::Column, std::time::Instant)>,
    /// card_id → when the FIRST in-flight mv RPC started. Used by the
    /// drill-in to show an elapsed-time loader while the daemon is busy
    /// with ensure_running. Cleared when pending_mvs drops to 0.
    pub pending_mv_started_at: std::collections::HashMap<String, std::time::Instant>,
    /// Set when the most recent ls RPC failed (transport or RPC-level).
    /// While true the TUI renders a full-screen daemon-down overlay
    /// instead of the kanban. Cleared by the next successful refresh.
    pub daemon_unreachable: bool,
    /// card_id → most recent CardProgressResult.stages. Survives MvErr;
    /// GC'd when a live session exists for the card.
    pub card_progress: std::collections::HashMap<String, Vec<reeve_protocol::ProgressStage>>,
    /// Last poll instant per card; used to rate-limit the card_progress
    /// fetch to ~400ms while pending_mvs has the card.
    pub card_progress_last_poll: std::collections::HashMap<String, std::time::Instant>,
}

impl App {
    pub fn new() -> Self {
        let (op_tx, op_rx) = mpsc::unbounded_channel();
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
            sessions: crate::tui::sessions::SessionManager::new(),
            pane_focus: PaneFocus::Kanban,
            pending_attention: std::collections::HashSet::new(),
            op_tx,
            op_rx,
            pending_mvs: std::collections::HashMap::new(),
            pending_edits: std::collections::HashMap::new(),
            pending_deletes: std::collections::HashSet::new(),
            pending_mv_intents: std::collections::HashMap::new(),
            pending_mv_started_at: std::collections::HashMap::new(),
            daemon_unreachable: false,
            card_progress: std::collections::HashMap::new(),
            card_progress_last_poll: std::collections::HashMap::new(),
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
        let mut v: Vec<&Card> = self
            .cards
            .iter()
            .filter(|c| c.column == self.focused_column)
            .collect();
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
    crate::tui::actions::refresh(&mut app).await;

    while !app.quit {
        app.clear_expired_status();

        // Drain background-RPC results (non-blocking).
        while let Ok(event) = app.op_rx.try_recv() {
            crate::tui::actions::process_op_event(&mut app, event).await;
        }

        // Fire any debounced mvs whose deadline has passed.
        let now = std::time::Instant::now();
        let to_fire: Vec<(String, reeve_protocol::Column)> = app
            .pending_mv_intents
            .iter()
            .filter(|(_, (_, deadline))| now >= *deadline)
            .map(|(id, (col, _))| (id.clone(), *col))
            .collect();
        for (card_id, target) in to_fire {
            app.pending_mv_intents.remove(&card_id);
            crate::tui::actions::fire_pending_mv(&mut app, card_id, target);
        }

        terminal.draw(|f| render::render(f, &app))?;

        // Apply any debounced PTY resize requests that have settled.
        // 150ms of quiet means the user stopped dragging; safe to SIGWINCH.
        for id in app.sessions.ids() {
            if let Some(s) = app.sessions.get(&id) {
                s.apply_pending_resize_if_due(150);
            }
        }

        // Activity badge: mark cards whose session emitted output since the
        // user last viewed it. Skip the currently-focused session.
        {
            let session_ids: Vec<String> = app.sessions.ids();
            let now = std::time::Instant::now();
            for id in session_ids {
                let is_currently_focused = app.pane_focus
                    == crate::tui::app::PaneFocus::SessionPane
                    && app.selected_card_id.as_ref() == Some(&id);
                if is_currently_focused {
                    continue;
                }
                if let Some(s) = app.sessions.get(&id) {
                    let last = *s.last_output_at.lock().unwrap();
                    if now.saturating_duration_since(last) < std::time::Duration::from_millis(500) {
                        app.pending_attention.insert(id.clone());
                    }
                }
            }
        }

        // Auto-respawn sessions whose reeve attach exited (with guard against tight loops).
        {
            let session_ids: Vec<String> = app.sessions.ids();
            for id in session_ids {
                let exit_info = app.sessions.get_mut(&id).and_then(|s| s.poll_child_exit());
                if let Some(exit) = exit_info {
                    let (should_respawn, error_msg) = {
                        let s = app.sessions.get_mut(&id).expect("session present");
                        // Reset window if >5s since the first respawn in this window.
                        if s.respawn_window_start.elapsed() > std::time::Duration::from_secs(5) {
                            s.respawn_count = 0;
                            s.respawn_window_start = std::time::Instant::now();
                        }
                        if exit == 0 && s.respawn_count < 3 {
                            s.respawn_count += 1;
                            (true, None)
                        } else if exit == 0 {
                            (
                                false,
                                Some(format!(
                                    "session {} respawned 3x in 5s, stopped — press 'r' to retry",
                                    &id[..8.min(id.len())]
                                )),
                            )
                        } else {
                            (
                                false,
                                Some(format!(
                                    "session {} exited with status {}",
                                    &id[..8.min(id.len())],
                                    exit
                                )),
                            )
                        }
                    };
                    app.sessions.close(&id);
                    if should_respawn {
                        if let Err(e) = app.sessions.spawn(&id) {
                            app.sessions.mark_failed(&id);
                            app.set_status(
                                format!("respawn failed: {e:#}"),
                                crate::tui::app::StatusLevel::Error,
                            );
                        }
                    } else if let Some(msg) = error_msg {
                        // Mark as failed so reconcile doesn't keep re-spawning.
                        // The status message fires ONCE per failed session — the
                        // ls-poll-driven reconcile won't re-spawn until the user
                        // manually presses `r`.
                        app.sessions.mark_failed(&id);
                        app.set_status(msg, crate::tui::app::StatusLevel::Error);
                    }
                }
            }
        }

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if app.modal.is_some() {
                        crate::tui::actions::handle_modal_key(&mut app, key).await;
                    } else if app.pane_focus == crate::tui::app::PaneFocus::SessionPane {
                        // In session-pane mode only FocusKanban is intercepted
                        // (Ctrl+G / Ctrl+]). Everything else — including
                        // Shift+Tab, which claude uses for its mode cycle — is
                        // forwarded raw to the PTY.
                        if events::translate(key) == Some(events::Action::FocusKanban) {
                            crate::tui::actions::handle(&mut app, events::Action::FocusKanban)
                                .await;
                        } else {
                            crate::tui::actions::forward_to_session(&mut app, key).await;
                        }
                    } else if let Some(action) = events::translate(key) {
                        crate::tui::actions::handle(&mut app, action).await;
                    }
                }
            }
        }

        if app.last_refresh.elapsed() > Duration::from_millis(500) {
            crate::tui::actions::refresh(&mut app).await;
        }
        // Card-progress poll runs every drive iteration (~50ms tick) but the
        // helper rate-limits to 400ms per card. Cheap when no cards are pending.
        crate::tui::actions::poll_card_progress(&mut app, 400).await;
    }
    Ok(())
}
