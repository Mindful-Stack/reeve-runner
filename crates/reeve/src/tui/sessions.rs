//! Session manager: PTY pool keyed by card_id. Each entry spawns
//! `reeve attach <card>` connected to a portable-pty; vt100::Parser
//! consumes output bytes in a background blocking task; tui-term renders
//! the parser's screen.

use anyhow::{Context, Result};
use portable_pty::{CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use vt100::Parser;

/// One embedded session per Implementing card. Owns the PTY pair,
/// the spawned `reeve attach` child, the vt100 parser, and a drainer
/// task that feeds bytes from the master into the parser.
pub struct Session {
    pub card_id: String,
    pub parser: Arc<Mutex<Parser>>,
    /// Held so we can issue SIGWINCH via `master.resize()` when the pane
    /// is rendered at a new size. Kept alongside `master_writer` which
    /// is the byte-write half taken from the same master.
    pub master: Box<dyn MasterPty + Send>,
    pub master_writer: Box<dyn Write + Send>,
    pub child: Box<dyn portable_pty::Child + Send + Sync>,
    /// Most recent PTY rows × cols. Tracked so we don't issue a SIGWINCH
    /// every frame when nothing has changed. Behind a Mutex so the render
    /// path (which holds only `&Session`) can drive size changes.
    pub current_size: Arc<Mutex<(u16, u16)>>,
    /// Wall-clock of the most recent output byte (used by Task 14 activity badge).
    pub last_output_at: Arc<Mutex<Instant>>,
    /// Respawn counter (used by Task 14 loop guard).
    pub respawn_count: u32,
    pub respawn_window_start: Instant,
}

impl Session {
    /// Spawn `reeve attach <card_id>` on a fresh PTY with the given size.
    pub fn spawn(card_id: &str, rows: u16, cols: u16) -> Result<Self> {
        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .context("openpty")?;

        let reeve_path = std::env::current_exe().context("locate reeve binary")?;
        let mut cmd = CommandBuilder::new(reeve_path);
        cmd.args(["attach", card_id]);
        // Hint a sensible TERM in case the inheriting environment doesn't
        // forward one. xterm-256color avoids legacy ACS line-drawing paths
        // that some apps (tmux, ncurses) fall back to under bare 'screen'.
        cmd.env("TERM", "xterm-256color");
        let child = pair.slave.spawn_command(cmd).context("spawn reeve attach")?;

        // Drop slave end now that the child holds it.
        drop(pair.slave);

        let mut master_reader = pair.master.try_clone_reader().context("clone master reader")?;
        let master_writer = pair.master.take_writer().context("take master writer")?;

        let parser = Arc::new(Mutex::new(Parser::new(rows, cols, 1000)));
        let last_output_at = Arc::new(Mutex::new(Instant::now()));

        let parser_clone = Arc::clone(&parser);
        let last_output_clone = Arc::clone(&last_output_at);
        tokio::task::spawn_blocking(move || {
            let mut buf = [0u8; 4096];
            loop {
                match master_reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if let Ok(mut p) = parser_clone.lock() {
                            p.process(&buf[..n]);
                        }
                        if let Ok(mut t) = last_output_clone.lock() {
                            *t = Instant::now();
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            card_id: card_id.to_string(),
            parser,
            master: pair.master,
            master_writer,
            child,
            current_size: Arc::new(Mutex::new((rows, cols))),
            last_output_at,
            respawn_count: 0,
            respawn_window_start: Instant::now(),
        })
    }

    pub fn send_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.master_writer.write_all(bytes).context("write to PTY")?;
        self.master_writer.flush().ok();
        Ok(())
    }

    /// Resize the PTY (sending SIGWINCH to the child) and the vt100 parser.
    /// No-op if the size hasn't changed. Takes `&self` so the render path
    /// can fit the embedded terminal to the actual pane area each frame.
    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        if rows == 0 || cols == 0 {
            return Ok(());
        }
        {
            let cur = self.current_size.lock().unwrap();
            if (rows, cols) == *cur {
                return Ok(());
            }
        }
        self.master
            .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .context("resize pty master")?;
        if let Ok(mut p) = self.parser.lock() {
            p.set_size(rows, cols);
        }
        *self.current_size.lock().unwrap() = (rows, cols);
        Ok(())
    }

    pub fn poll_child_exit(&mut self) -> Option<i32> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(status.exit_code() as i32),
            _ => None,
        }
    }
}

pub struct SessionManager {
    sessions: HashMap<String, Session>,
    default_rows: u16,
    default_cols: u16,
    /// card_ids whose last spawn-or-attach failed. While present here,
    /// `reconcile` will NOT auto-spawn a session for that card. Cleared
    /// when the user manually retries (via the ForceRefresh action — `r`)
    /// or when the card leaves Implementing.
    failed_sessions: std::collections::HashSet<String>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            default_rows: 40,
            default_cols: 120,
            failed_sessions: std::collections::HashSet::new(),
        }
    }

    /// Mark a card's session as failed so reconcile won't auto-respawn.
    pub fn mark_failed(&mut self, card_id: &str) {
        self.failed_sessions.insert(card_id.to_string());
    }

    /// Clear all failed markers. Called on ForceRefresh (`r`) so the user
    /// can retry manually.
    pub fn clear_failed(&mut self) {
        self.failed_sessions.clear();
    }

    /// Whether a card is currently in the failed set.
    pub fn is_failed(&self, card_id: &str) -> bool {
        self.failed_sessions.contains(card_id)
    }

    pub fn set_default_size(&mut self, rows: u16, cols: u16) {
        self.default_rows = rows;
        self.default_cols = cols;
    }

    pub fn get(&self, card_id: &str) -> Option<&Session> {
        self.sessions.get(card_id)
    }

    pub fn get_mut(&mut self, card_id: &str) -> Option<&mut Session> {
        self.sessions.get_mut(card_id)
    }

    pub fn spawn(&mut self, card_id: &str) -> Result<()> {
        let session = Session::spawn(card_id, self.default_rows, self.default_cols)?;
        self.sessions.insert(card_id.to_string(), session);
        Ok(())
    }

    pub fn close(&mut self, card_id: &str) {
        if let Some(mut session) = self.sessions.remove(card_id) {
            let _ = session.child.kill();
        }
    }

    pub fn ids(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }

    /// Reconcile against a fresh set of Implementing card ids: spawn new,
    /// close removed.
    pub fn reconcile(&mut self, implementing: &[String]) {
        let current: Vec<String> = self.ids();
        for id in &current {
            if !implementing.contains(id) {
                self.close(id);
            }
        }
        // Also clear failed-state for cards no longer in Implementing — they
        // get a fresh chance if/when they return.
        let still_implementing: std::collections::HashSet<&String> = implementing.iter().collect();
        self.failed_sessions.retain(|id| still_implementing.contains(id));

        for id in implementing {
            if !self.sessions.contains_key(id) && !self.failed_sessions.contains(id) {
                if let Err(e) = self.spawn(id) {
                    tracing::warn!("failed to spawn session for {id}: {e:#}");
                    self.failed_sessions.insert(id.clone());
                }
            }
        }
    }
}
