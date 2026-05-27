# Review and PR Flow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reorder the back-half kanban flow (Implementing → Review → Testing → PR → Done), introduce the new `Pr` active column, and implement the host-mediated PR collaboration bridge that lets the in-container agent read and reply to PR comments via the daemon without ever holding GitHub credentials inside the devcontainer.

**Architecture:** Add `Pr` variant to the `Column` enum and reorder `Column::ALL`. Extend the `WorkspaceLifecycle` port with a `prepare_pr` method whose impl on `LiveWorkspaceLifecycle` pushes affected branches and creates GitHub PRs via a new `GhOps` port (defined in `reeve-card`, implemented in `reeve-workspace`). Track per-card PR records in a new `card_prs` satellite table owned by `reeve-card`. Add a read-only `review-info` RPC (in `reeve-workspace`) for the Review-column tool integration. Add five self-scoped PR RPCs (in `reeve-card`) the in-container agent uses to discuss the PR. Update the SessionStart prompt template, the TUI, and the lore docs to match.

**Tech Stack:** Rust 2021, tokio, async-trait, rusqlite, serde, `tokio::process::Command` for `gh` shell-out, ratatui for the TUI.

**Reference spec:** `docs/superpowers/specs/2026-05-27-review-and-pr-flow-design.md`. Read it before starting — every task here implements something the spec defines.

---

## File map

NEW FILES:

- `crates/reeve-card/src/pr_repo.rs` — `CardPrRepo` trait + `SqliteCardPrRepo` adapter + `init_schema` + `make_card_pr_repo` factory.
- `crates/reeve-workspace/src/gh.rs` — `GhOps` port (trait), `RealGhOps` adapter, pure argv builders, `make_gh_ops` factory.

MODIFIED FILES (grouped by aggregate):

- **protocol:** `crates/reeve-protocol/src/lib.rs` — add `Pr` variant, reorder `Column::ALL`, update `label`/`as_wire_str`/`from_wire_str`/`is_active`; add new wire DTOs for `RepoReviewInfo`, `PrSummary`, `PrStatus`, `PrComment`, `PrReplyParams/Result`, `PrEditParams/Result`, `PrCreated`.
- **card:** `crates/reeve-card/src/transitions.rs` (add `prepare_pr` to `WorkspaceLifecycle`, `PrCreated` value type, new tests), `crates/reeve-card/src/handler.rs` (call `prepare_pr` on entering `Pr`; five new `handle_pr_*` handlers), `crates/reeve-card/src/lib.rs` (re-exports), `crates/reeve-card/src/session_init.rs` (prompt template).
- **workspace:** `crates/reeve-workspace/src/lifecycle.rs` (implement `prepare_pr`, hold `CardPrRepo` + `GhOps`), `crates/reeve-workspace/src/handler.rs` (`handle_review_info`), `crates/reeve-workspace/src/lib.rs` (re-exports).
- **reeved:** `crates/reeved/src/dispatch.rs` (dispatch arms for the new RPCs), `crates/reeved/src/app/state.rs` (state holds `gh: GhOpsHandle`), `crates/reeved/src/app/lifecycle.rs` (call `pr_repo::init_schema` in `migrate`).
- **reeve (CLI/TUI):** `crates/reeve/src/cli.rs` (`Review` + `PrStatus` subcommands), `crates/reeve/src/tui/modal.rs` (at-first/at-last with new column order), `crates/reeve/src/tui/render.rs` (any Pr-specific arms).
- **lore:** `lore/knowledge/domain/card.md`, `lore/knowledge/domain/workspace.md`.

---

## Phase 1 — Column enum extension and reorder

### Task 1: Add `Pr` variant to `Column` and reorder `Column::ALL`

**Files:**
- Modify: `crates/reeve-protocol/src/lib.rs`

- [ ] **Step 1: Write the failing tests**

Append to `crates/reeve-protocol/src/lib.rs` inside the existing `#[cfg(test)] mod tests` (or create one if none — search for `mod tests` first; if absent, add at the bottom of the file):

```rust
#[cfg(test)]
mod column_tests {
    use super::*;

    #[test]
    fn pr_round_trips_via_wire() {
        assert_eq!(Column::Pr.as_wire_str(), "pr");
        assert_eq!(Column::from_wire_str("pr"), Some(Column::Pr));
        assert_eq!(Column::Pr.label(), "PR");
    }

    #[test]
    fn pr_is_active() {
        assert!(Column::Pr.is_active());
    }

    #[test]
    fn column_all_in_new_canonical_order() {
        assert_eq!(
            Column::ALL,
            [
                Column::Backlog,
                Column::Planning,
                Column::Implementing,
                Column::Review,
                Column::Testing,
                Column::Pr,
                Column::Done,
            ]
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p reeve-protocol column_tests
```
Expected: compilation errors — `Pr` variant does not exist.

- [ ] **Step 3: Update the enum, ALL[], and all match arms**

In `crates/reeve-protocol/src/lib.rs`, change the `Column` enum, `ALL`, and the three impl methods to:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Column {
    Backlog,
    Planning,
    Implementing,
    Review,
    Testing,
    Pr,
    Done,
}

impl Column {
    pub const ALL: [Column; 7] = [
        Column::Backlog,
        Column::Planning,
        Column::Implementing,
        Column::Review,
        Column::Testing,
        Column::Pr,
        Column::Done,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Column::Backlog => "Backlog",
            Column::Planning => "Planning",
            Column::Implementing => "Implementing",
            Column::Review => "Review",
            Column::Testing => "Testing",
            Column::Pr => "PR",
            Column::Done => "Done",
        }
    }

    pub fn as_wire_str(&self) -> &'static str {
        match self {
            Column::Backlog => "backlog",
            Column::Planning => "planning",
            Column::Implementing => "implementing",
            Column::Review => "review",
            Column::Testing => "testing",
            Column::Pr => "pr",
            Column::Done => "done",
        }
    }

    pub fn from_wire_str(s: &str) -> Option<Self> {
        match s {
            "backlog" => Some(Column::Backlog),
            "planning" => Some(Column::Planning),
            "implementing" => Some(Column::Implementing),
            "review" => Some(Column::Review),
            "testing" => Some(Column::Testing),
            "pr" => Some(Column::Pr),
            "done" => Some(Column::Done),
            _ => None,
        }
    }

    pub fn is_active(self) -> bool {
        matches!(
            self,
            Column::Planning
                | Column::Implementing
                | Column::Testing
                | Column::Review
                | Column::Pr,
        )
    }
}
```

- [ ] **Step 4: Run all tests in the workspace; expect compile errors elsewhere**

```bash
cargo test --workspace --lib
```
Expected: compile errors in `reeve-card`, `reeve-workspace`, `reeve` from non-exhaustive `match` on `Column`. Note each error site for the next task.

- [ ] **Step 5: Commit**

```bash
git add crates/reeve-protocol/src/lib.rs
git commit -m "feat(protocol): add Pr column variant and reorder Column::ALL"
```

---

### Task 2: Fix non-exhaustive `match Column` arms across the workspace

**Files:**
- Modify: `crates/reeve-card/src/transitions.rs` (the `#[cfg(test)] mod tests`)
- Modify: `crates/reeve/src/tui/render.rs:432` (and any nearby exhaustive matches found in Task 1's compile errors)
- Modify: any other file the compiler points at.

This task is mechanical: extend every non-exhaustive `match` to handle `Column::Pr`. Pr is active and behaves like Implementing/Testing/Review for default rendering and lifecycle.

- [ ] **Step 1: Compile to enumerate sites**

```bash
cargo build --workspace 2>&1 | grep -E "non-exhaustive|missing pattern" | head -40
```
Expected: a list of file:line locations.

- [ ] **Step 2: Patch each site**

For each location, add a `Column::Pr => ...` arm following the pattern of the surrounding code. Examples:

In `crates/reeve/src/tui/render.rs` around line 432, the existing block does color/style by column:
```rust
use reeve_protocol::Column::*;
match column {
    Backlog => ...,
    Planning => ...,
    Implementing => ...,
    Review => ...,
    Testing => ...,
    Pr => ...,           // add — colour same family as Review
    Done => ...,
}
```

Use the same style as `Review` (it's the prior active-human column) unless the file's pattern suggests otherwise.

- [ ] **Step 3: Build to verify all sites covered**

```bash
cargo build --workspace 2>&1 | grep -E "non-exhaustive|missing pattern"
```
Expected: empty output.

- [ ] **Step 4: Run the full test suite**

```bash
cargo test --workspace --lib && cargo test -p reeved
```
Expected: PASS (existing tests do not assert column order, just behaviour).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "fix: extend Column match arms across workspace for new Pr variant"
```

---

### Task 3: Pin new transition tests for the `Pr` column

**Files:**
- Modify: `crates/reeve-card/src/transitions.rs:90-205` (the `tests` module)

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module in `crates/reeve-card/src/transitions.rs`:

```rust
#[tokio::test]
async fn testing_to_pr_is_noop_for_container() {
    // Active → Active: container persists, no respawn.
    let fx = run_one(Column::Testing, Column::Pr).await;
    assert!(fx.running.lock().await.is_empty());
    assert!(fx.stopped.lock().await.is_empty());
}

#[tokio::test]
async fn pr_to_done_tears_down() {
    let fx = run_one(Column::Pr, Column::Done).await;
    assert_eq!(*fx.stopped.lock().await, vec!["c1".to_string()]);
}

#[tokio::test]
async fn pr_to_implementing_is_noop() {
    // Backwards Active → Active also persists the container.
    let fx = run_one(Column::Pr, Column::Implementing).await;
    assert!(fx.running.lock().await.is_empty());
    assert!(fx.stopped.lock().await.is_empty());
}

#[tokio::test]
async fn backlog_to_pr_spawns() {
    let fx = run_one(Column::Backlog, Column::Pr).await;
    assert_eq!(*fx.running.lock().await, vec!["c1".to_string()]);
}
```

- [ ] **Step 2: Run tests; expect PASS**

```bash
cargo test -p reeve-card --lib transitions::tests
```
Expected: all four new tests PASS (they exercise the existing `is_active`-based matrix, which already covers Pr after Task 2's `is_active` update).

- [ ] **Step 3: Commit**

```bash
git add crates/reeve-card/src/transitions.rs
git commit -m "test(card): pin Pr column transitions through the active/inactive matrix"
```

---

## Phase 2 — `card_prs` satellite table

### Task 4: Define `PrCreated` value type and `CardPrRepo` trait

**Files:**
- Create: `crates/reeve-card/src/pr_repo.rs`
- Modify: `crates/reeve-card/src/lib.rs` (add `pub mod pr_repo;` and re-exports)

- [ ] **Step 1: Write `pr_repo.rs` skeleton + the failing test**

Create `crates/reeve-card/src/pr_repo.rs`:

```rust
//! Persistence port + SQLite adapter for the `card_prs` satellite table.
//! A card can open one PR per affected repo; this table records the
//! (card, repo) → PR-number mapping used by the in-container PR RPCs.

use anyhow::{Context, Result};
use async_trait::async_trait;
use rusqlite::{Connection, params};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardPr {
    pub card_id: String,
    pub repo_name: String,
    pub pr_number: u64,
    pub pr_url: String,
    pub opened_at: i64,
}

/// What `WorkspaceLifecycle::prepare_pr` returns per affected repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrCreated {
    pub repo_name: String,
    pub pr_number: u64,
    pub pr_url: String,
}

#[async_trait]
pub trait CardPrRepo: Send + Sync {
    async fn upsert(&self, row: &CardPr) -> Result<()>;
    async fn list_for_card(&self, card_id: &str) -> Result<Vec<CardPr>>;
    async fn get(&self, card_id: &str, repo_name: &str) -> Result<Option<CardPr>>;
}

pub(crate) struct SqliteCardPrRepo {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteCardPrRepo {
    pub(crate) fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl CardPrRepo for SqliteCardPrRepo {
    async fn upsert(&self, row: &CardPr) -> Result<()> {
        let conn = self.conn.clone();
        let row = row.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT INTO card_prs (card_id, repo_name, pr_number, pr_url, opened_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(card_id, repo_name) DO UPDATE SET
                   pr_number = excluded.pr_number,
                   pr_url    = excluded.pr_url,
                   opened_at = excluded.opened_at",
                params![row.card_id, row.repo_name, row.pr_number as i64, row.pr_url, row.opened_at],
            )?;
            Ok(())
        })
        .await?
    }

    async fn list_for_card(&self, card_id: &str) -> Result<Vec<CardPr>> {
        let conn = self.conn.clone();
        let card_id = card_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<Vec<CardPr>> {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare(
                "SELECT card_id, repo_name, pr_number, pr_url, opened_at
                 FROM card_prs WHERE card_id = ?1 ORDER BY repo_name",
            )?;
            let rows = stmt.query_map(params![card_id], |r| {
                Ok(CardPr {
                    card_id: r.get(0)?,
                    repo_name: r.get(1)?,
                    pr_number: r.get::<_, i64>(2)? as u64,
                    pr_url: r.get(3)?,
                    opened_at: r.get(4)?,
                })
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .await?
    }

    async fn get(&self, card_id: &str, repo_name: &str) -> Result<Option<CardPr>> {
        let conn = self.conn.clone();
        let card_id = card_id.to_string();
        let repo_name = repo_name.to_string();
        tokio::task::spawn_blocking(move || -> Result<Option<CardPr>> {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare(
                "SELECT card_id, repo_name, pr_number, pr_url, opened_at
                 FROM card_prs WHERE card_id = ?1 AND repo_name = ?2",
            )?;
            let row = stmt
                .query_row(params![card_id, repo_name], |r| {
                    Ok(CardPr {
                        card_id: r.get(0)?,
                        repo_name: r.get(1)?,
                        pr_number: r.get::<_, i64>(2)? as u64,
                        pr_url: r.get(3)?,
                        opened_at: r.get(4)?,
                    })
                })
                .ok();
            Ok(row)
        })
        .await?
    }
}

pub fn make_card_pr_repo(conn: Arc<Mutex<Connection>>) -> Arc<dyn CardPrRepo> {
    Arc::new(SqliteCardPrRepo::new(conn))
}

pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS card_prs (
            card_id   TEXT NOT NULL,
            repo_name TEXT NOT NULL,
            pr_number INTEGER NOT NULL,
            pr_url    TEXT NOT NULL,
            opened_at INTEGER NOT NULL,
            PRIMARY KEY (card_id, repo_name)
         );
         CREATE INDEX IF NOT EXISTS idx_card_prs_card ON card_prs(card_id);",
    )
    .context("init_schema for card_prs")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_repo() -> Arc<dyn CardPrRepo> {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        make_card_pr_repo(Arc::new(Mutex::new(conn)))
    }

    fn sample(card: &str, repo: &str, num: u64) -> CardPr {
        CardPr {
            card_id: card.into(),
            repo_name: repo.into(),
            pr_number: num,
            pr_url: format!("https://github.com/owner/{repo}/pull/{num}"),
            opened_at: 1_000,
        }
    }

    #[tokio::test]
    async fn upsert_then_list() {
        let repo = fresh_repo();
        repo.upsert(&sample("c1", "meta", 42)).await.unwrap();
        repo.upsert(&sample("c1", "siblA", 7)).await.unwrap();
        repo.upsert(&sample("c2", "meta", 9)).await.unwrap();

        let c1 = repo.list_for_card("c1").await.unwrap();
        assert_eq!(c1.len(), 2);
        let names: Vec<&str> = c1.iter().map(|r| r.repo_name.as_str()).collect();
        assert_eq!(names, vec!["meta", "siblA"]);
    }

    #[tokio::test]
    async fn upsert_is_idempotent_and_updates_in_place() {
        let repo = fresh_repo();
        repo.upsert(&sample("c1", "meta", 42)).await.unwrap();
        // Same (card, repo) → new pr_number/url overwrites.
        let mut updated = sample("c1", "meta", 99);
        updated.pr_url = "new-url".into();
        repo.upsert(&updated).await.unwrap();

        let rows = repo.list_for_card("c1").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].pr_number, 99);
        assert_eq!(rows[0].pr_url, "new-url");
    }

    #[tokio::test]
    async fn get_returns_specific_row() {
        let repo = fresh_repo();
        repo.upsert(&sample("c1", "meta", 42)).await.unwrap();
        let row = repo.get("c1", "meta").await.unwrap().unwrap();
        assert_eq!(row.pr_number, 42);
        assert!(repo.get("c1", "missing").await.unwrap().is_none());
    }
}
```

- [ ] **Step 2: Wire the module into the crate**

In `crates/reeve-card/src/lib.rs`, add at the top with other `pub mod` declarations:

```rust
pub mod pr_repo;
```

Then add to the re-export block:
```rust
pub use pr_repo::{CardPr, CardPrRepo, PrCreated, init_schema as init_pr_schema, make_card_pr_repo};
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p reeve-card --lib pr_repo
```
Expected: all three tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/reeve-card/src/pr_repo.rs crates/reeve-card/src/lib.rs
git commit -m "feat(card): add CardPrRepo + card_prs satellite table"
```

---

### Task 5: Wire `card_prs` schema migration into reeved

**Files:**
- Modify: `crates/reeved/src/app/lifecycle.rs` (find the `migrate` function — call `reeve_card::init_pr_schema(&conn)` alongside the existing `reeve_card::init_schema` / `reeve_workspace::init_schema` calls)

- [ ] **Step 1: Locate the migration function**

```bash
grep -n "init_schema" crates/reeved/src/app/lifecycle.rs
```
Note the existing schema-init call pattern.

- [ ] **Step 2: Add the new init call**

Add `reeve_card::init_pr_schema(&conn)?;` next to the existing `reeve_card::init_schema(&conn)?;` line in the migrate function.

- [ ] **Step 3: Run reeved's tests**

```bash
cargo test -p reeved --lib
```
Expected: PASS. (No tests assert on `card_prs` yet; this is purely additive.)

- [ ] **Step 4: Commit**

```bash
git add crates/reeved/src/app/lifecycle.rs
git commit -m "feat(reeved): migrate the card_prs table on daemon startup"
```

---

## Phase 3 — `GhOps` port (in card) and `RealGhOps` adapter (in workspace)

Per the spec: the consumer defines the port. `reeve-card` is the only consumer (the PR self-scoped RPCs all live there); `reeve-workspace` only supplies the shell-out adapter. This mirrors how `WorkspaceLifecycle` is defined in `reeve-card` and implemented in `reeve-workspace`.

### Task 6a: Define `GhOps` trait + types in `reeve-card`

**Files:**
- Create: `crates/reeve-card/src/gh_ops.rs`
- Modify: `crates/reeve-card/src/lib.rs` (`pub mod gh_ops;` + re-exports)

- [ ] **Step 1: Create the trait file**

Create `crates/reeve-card/src/gh_ops.rs`:

```rust
//! `gh` CLI port. Defined here (the consumer crate); implemented in
//! `reeve-workspace` (`RealGhOps`). The container never has `gh`;
//! only the daemon on the host does. The agent inside a container
//! invokes self-scoped PR RPCs that route through this trait.

use anyhow::Result;
use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhPrView {
    pub state: String,         // "OPEN" | "CLOSED" | "MERGED"
    pub mergeable: String,     // "MERGEABLE" | "CONFLICTING" | "UNKNOWN"
    pub checks_status: String, // "PENDING" | "SUCCESS" | "FAILURE" | "NONE"
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhComment {
    pub id: u64,
    pub author: String,
    pub path: Option<String>,
    pub line: Option<u32>,
    pub body: String,
    pub in_reply_to: Option<u64>,
}

#[async_trait]
pub trait GhOps: Send + Sync {
    /// Run `gh pr create --base <base> --head <branch> --title ... --body ...`.
    /// Returns (pr_number, pr_url). Idempotent: if a PR for `<branch>` already
    /// exists, return that instead of failing.
    async fn create_pr(
        &self,
        repo_path: &Path,
        branch: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<(u64, String)>;

    /// `gh pr view <num> --json state,mergeable,statusCheckRollup`.
    async fn view_pr(&self, repo_path: &Path, pr_number: u64) -> Result<GhPrView>;

    /// `gh api repos/{owner}/{repo}/pulls/{num}/comments` (review comments).
    async fn list_comments(&self, repo_path: &Path, pr_number: u64) -> Result<Vec<GhComment>>;

    /// `gh api -X POST repos/{owner}/{repo}/pulls/{num}/comments/{parent}/replies`.
    /// Returns the posted comment's id.
    async fn post_reply(
        &self,
        repo_path: &Path,
        pr_number: u64,
        parent_comment_id: u64,
        body: &str,
    ) -> Result<u64>;

    /// `gh pr edit <num> --title ... --body ...`. Either field optional.
    async fn edit_pr(
        &self,
        repo_path: &Path,
        pr_number: u64,
        title: Option<&str>,
        body: Option<&str>,
    ) -> Result<()>;

    /// `gh pr list --head <branch> --state all --json number` — returns
    /// numbers of any PRs whose head branch matches. Empty when none exist.
    async fn list_prs_for_branch(&self, repo_path: &Path, branch: &str) -> Result<Vec<u64>>;
}

pub type GhOpsHandle = Arc<dyn GhOps>;
```

- [ ] **Step 2: Re-export from `lib.rs`**

In `crates/reeve-card/src/lib.rs`:
```rust
pub mod gh_ops;
pub use gh_ops::{GhComment, GhOps, GhOpsHandle, GhPrView};
```

- [ ] **Step 3: Build the crate**

```bash
cargo build -p reeve-card
```
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/reeve-card/src/gh_ops.rs crates/reeve-card/src/lib.rs
git commit -m "feat(card): define GhOps port (consumer-defined, impl in workspace)"
```

---

### Task 6b: Implement `RealGhOps` adapter + pure argv builders in `reeve-workspace`

**Files:**
- Create: `crates/reeve-workspace/src/gh.rs`
- Modify: `crates/reeve-workspace/src/lib.rs` (`pub mod gh;` + re-exports)

- [ ] **Step 1: Write `gh.rs` with the adapter + pure argv builders + failing tests**

Create `crates/reeve-workspace/src/gh.rs`:

```rust
//! `RealGhOps` adapter for `reeve_card::GhOps` + pure argv builders.
//!
//! The trait lives in `reeve-card` (the consumer). This file owns the
//! `gh` subprocess implementation and the testable argv builders.

use anyhow::{Context, Result};
use async_trait::async_trait;
use reeve_card::{GhComment, GhOps, GhPrView};
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;
use tokio::process::Command;

// --- pure argv builders (testable without subprocess) ---

pub fn create_pr_argv(branch: &str, base: &str, title: &str, body: &str) -> Vec<String> {
    vec![
        "pr".into(), "create".into(),
        "--base".into(), base.into(),
        "--head".into(), branch.into(),
        "--title".into(), title.into(),
        "--body".into(), body.into(),
    ]
}

pub fn view_pr_argv(pr_number: u64) -> Vec<String> {
    vec![
        "pr".into(), "view".into(), pr_number.to_string(),
        "--json".into(), "state,mergeable,statusCheckRollup".into(),
    ]
}

pub fn list_prs_argv(branch: &str) -> Vec<String> {
    vec![
        "pr".into(), "list".into(),
        "--head".into(), branch.into(),
        "--state".into(), "all".into(),
        "--json".into(), "number".into(),
    ]
}

pub fn edit_pr_argv(pr_number: u64, title: Option<&str>, body: Option<&str>) -> Vec<String> {
    let mut v = vec!["pr".into(), "edit".into(), pr_number.to_string()];
    if let Some(t) = title {
        v.push("--title".into());
        v.push(t.into());
    }
    if let Some(b) = body {
        v.push("--body".into());
        v.push(b.into());
    }
    v
}

// --- RealGhOps adapter ---

pub(crate) struct RealGhOps;

#[async_trait]
impl GhOps for RealGhOps {
    async fn create_pr(
        &self,
        repo_path: &Path,
        branch: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<(u64, String)> {
        // Idempotency: if a PR already exists for this branch, return it.
        let existing = self.list_prs_for_branch(repo_path, branch).await?;
        if let Some(&num) = existing.first() {
            let url = pr_url(repo_path, num).await?;
            return Ok((num, url));
        }
        let argv = create_pr_argv(branch, base, title, body);
        let output = Command::new("gh")
            .args(&argv)
            .current_dir(repo_path)
            .output()
            .await
            .context("exec gh pr create")?;
        if !output.status.success() {
            anyhow::bail!(
                "gh pr create failed (exit {}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let num = url
            .rsplit('/')
            .next()
            .and_then(|s| s.parse::<u64>().ok())
            .context("could not parse PR number from gh stdout")?;
        Ok((num, url))
    }

    async fn view_pr(&self, repo_path: &Path, pr_number: u64) -> Result<GhPrView> {
        let argv = view_pr_argv(pr_number);
        let output = Command::new("gh")
            .args(&argv)
            .current_dir(repo_path)
            .output()
            .await
            .context("exec gh pr view")?;
        if !output.status.success() {
            anyhow::bail!(
                "gh pr view failed (exit {}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        #[derive(Deserialize)]
        struct Raw {
            state: String,
            mergeable: String,
            #[serde(rename = "statusCheckRollup", default)]
            status_check_rollup: Vec<RollupEntry>,
        }
        let raw: Raw = serde_json::from_slice(&output.stdout).context("parse gh pr view json")?;
        let checks_status = derive_checks_status(&raw.status_check_rollup);
        Ok(GhPrView {
            state: raw.state,
            mergeable: raw.mergeable,
            checks_status,
        })
    }

    async fn list_comments(&self, repo_path: &Path, pr_number: u64) -> Result<Vec<GhComment>> {
        let endpoint = format!("repos/{{owner}}/{{repo}}/pulls/{pr_number}/comments");
        let output = Command::new("gh")
            .args(["api", &endpoint, "--paginate"])
            .current_dir(repo_path)
            .output()
            .await
            .context("exec gh api comments")?;
        if !output.status.success() {
            anyhow::bail!(
                "gh api comments failed (exit {}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        #[derive(Deserialize)]
        struct Raw {
            id: u64,
            user: User,
            #[serde(default)]
            path: Option<String>,
            #[serde(default)]
            line: Option<u32>,
            body: String,
            #[serde(default)]
            in_reply_to_id: Option<u64>,
        }
        #[derive(Deserialize)]
        struct User {
            login: String,
        }
        let raws: Vec<Raw> = serde_json::from_slice(&output.stdout).unwrap_or_default();
        Ok(raws
            .into_iter()
            .map(|r| GhComment {
                id: r.id,
                author: r.user.login,
                path: r.path,
                line: r.line,
                body: r.body,
                in_reply_to: r.in_reply_to_id,
            })
            .collect())
    }

    async fn post_reply(
        &self,
        repo_path: &Path,
        pr_number: u64,
        parent_comment_id: u64,
        body: &str,
    ) -> Result<u64> {
        let endpoint = format!(
            "repos/{{owner}}/{{repo}}/pulls/{pr_number}/comments/{parent_comment_id}/replies"
        );
        let field = format!("body={body}");
        let output = Command::new("gh")
            .args(["api", "-X", "POST", &endpoint, "-f", &field])
            .current_dir(repo_path)
            .output()
            .await
            .context("exec gh api reply")?;
        if !output.status.success() {
            anyhow::bail!(
                "gh api reply failed (exit {}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        #[derive(Deserialize)]
        struct Raw {
            id: u64,
        }
        let raw: Raw = serde_json::from_slice(&output.stdout).context("parse reply json")?;
        Ok(raw.id)
    }

    async fn edit_pr(
        &self,
        repo_path: &Path,
        pr_number: u64,
        title: Option<&str>,
        body: Option<&str>,
    ) -> Result<()> {
        if title.is_none() && body.is_none() {
            return Ok(());
        }
        let argv = edit_pr_argv(pr_number, title, body);
        let output = Command::new("gh")
            .args(&argv)
            .current_dir(repo_path)
            .output()
            .await
            .context("exec gh pr edit")?;
        if !output.status.success() {
            anyhow::bail!(
                "gh pr edit failed (exit {}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }

    async fn list_prs_for_branch(&self, repo_path: &Path, branch: &str) -> Result<Vec<u64>> {
        let argv = list_prs_argv(branch);
        let output = Command::new("gh")
            .args(&argv)
            .current_dir(repo_path)
            .output()
            .await
            .context("exec gh pr list")?;
        if !output.status.success() {
            anyhow::bail!(
                "gh pr list failed (exit {}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        #[derive(Deserialize)]
        struct Raw {
            number: u64,
        }
        let raws: Vec<Raw> = serde_json::from_slice(&output.stdout).unwrap_or_default();
        Ok(raws.into_iter().map(|r| r.number).collect())
    }
}

async fn pr_url(repo_path: &Path, pr_number: u64) -> Result<String> {
    let output = Command::new("gh")
        .args(["pr", "view", &pr_number.to_string(), "--json", "url", "-q", ".url"])
        .current_dir(repo_path)
        .output()
        .await
        .context("exec gh pr view --json url")?;
    if !output.status.success() {
        anyhow::bail!(
            "gh pr view --json url failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn derive_checks_status(entries: &[RollupEntry]) -> String {
    if entries.is_empty() {
        return "NONE".into();
    }
    let any_failure = entries.iter().any(|e| e.conclusion.eq_ignore_ascii_case("FAILURE"));
    let any_pending = entries.iter().any(|e| e.conclusion.is_empty() || e.conclusion.eq_ignore_ascii_case("PENDING"));
    if any_failure { "FAILURE".into() }
    else if any_pending { "PENDING".into() }
    else { "SUCCESS".into() }
}

#[derive(Deserialize, Clone)]
struct RollupEntry {
    #[serde(default)]
    conclusion: String,
}

pub fn make_gh_ops() -> Arc<dyn GhOps> {
    Arc::new(RealGhOps)
}

#[cfg(test)]
mod argv_tests {
    use super::*;

    #[test]
    fn create_pr_argv_includes_all_fields() {
        let argv = create_pr_argv("feat-x", "main", "Title", "Body");
        assert_eq!(
            argv,
            vec![
                "pr".to_string(),
                "create".into(),
                "--base".into(),
                "main".into(),
                "--head".into(),
                "feat-x".into(),
                "--title".into(),
                "Title".into(),
                "--body".into(),
                "Body".into(),
            ]
        );
    }

    #[test]
    fn view_pr_argv_requests_state_mergeable_and_rollup() {
        let argv = view_pr_argv(42);
        assert!(argv.contains(&"42".to_string()));
        assert!(argv.iter().any(|s| s.contains("statusCheckRollup")));
    }

    #[test]
    fn list_prs_argv_uses_head_filter() {
        let argv = list_prs_argv("feat-y");
        assert!(argv.windows(2).any(|w| w == ["--head", "feat-y"]));
        assert!(argv.windows(2).any(|w| w == ["--state", "all"]));
    }

    #[test]
    fn edit_pr_argv_omits_unset_fields() {
        let argv = edit_pr_argv(5, Some("new title"), None);
        assert!(argv.windows(2).any(|w| w == ["--title", "new title"]));
        assert!(!argv.iter().any(|s| s == "--body"));

        let argv = edit_pr_argv(5, None, Some("new body"));
        assert!(argv.windows(2).any(|w| w == ["--body", "new body"]));
        assert!(!argv.iter().any(|s| s == "--title"));
    }
}
```

Note: the `derive_checks_status` helper above takes `&[serde_json::Value]` but the call site has a typed slice — replace its body with an explicit scan over typed entries when wiring up. For the v1 cut, "PENDING"/"SUCCESS"/"FAILURE"/"NONE" is a coarse heuristic; refine later if needed.

- [ ] **Step 2: Wire the module into the crate**

In `crates/reeve-workspace/src/lib.rs` add:
```rust
pub mod gh;
pub use gh::make_gh_ops;
```

(The trait + types are re-exported from `reeve-card`; consumers depend on the card crate for the API and on the workspace crate only for the factory.)

- [ ] **Step 3: Run the argv-builder tests**

```bash
cargo test -p reeve-workspace --lib gh::argv_tests
```
Expected: all four PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/reeve-workspace/src/gh.rs crates/reeve-workspace/src/lib.rs
git commit -m "feat(workspace): RealGhOps adapter + pure argv builders for gh"
```

---

## Phase 4 — `prepare_pr` lifecycle method

### Task 7: Extend `WorkspaceLifecycle` with `prepare_pr`

**Files:**
- Modify: `crates/reeve-card/src/transitions.rs:9-15` (the trait)
- Modify: `crates/reeve-card/src/transitions.rs:96-112` (the `MockFx` impl in `tests`)

- [ ] **Step 1: Write the failing test**

Add to `crates/reeve-card/src/transitions.rs::tests`:

```rust
#[tokio::test]
async fn mock_records_prepare_pr_call() {
    let fx = Arc::new(MockFx::default());
    let card = sample("c1", Column::Testing);
    fx.prepare_pr(&card).await.unwrap();
    let prepared = fx.prepared.lock().await;
    assert_eq!(*prepared, vec!["c1".to_string()]);
}
```

And extend `MockFx`:

```rust
#[derive(Default)]
struct MockFx {
    running: Mutex<Vec<String>>,
    stopped: Mutex<Vec<String>>,
    prepared: Mutex<Vec<String>>,
}
```

- [ ] **Step 2: Run; expect compile error from missing trait method**

```bash
cargo test -p reeve-card --lib transitions::tests::mock_records_prepare_pr_call
```
Expected: compile error — `prepare_pr` not in trait.

- [ ] **Step 3: Add the trait method and update the mock impl**

In `crates/reeve-card/src/transitions.rs`:

```rust
use crate::pr_repo::PrCreated;

#[async_trait]
pub trait WorkspaceLifecycle: Send + Sync {
    async fn ensure_running(&self, card: &Card) -> Result<()>;
    async fn ensure_stopped(&self, card_id: &str) -> Result<()>;
    /// Push affected branches + open GitHub PRs for any branch that doesn't
    /// have one yet. Records the resulting (card, repo, pr_number) tuples
    /// via the consumer's `CardPrRepo`. Idempotent: re-running detects
    /// existing PRs by branch and reuses them.
    async fn prepare_pr(&self, card: &Card) -> Result<Vec<PrCreated>>;
}
```

Update the `impl WorkspaceLifecycle for MockFx`:

```rust
#[async_trait]
impl WorkspaceLifecycle for MockFx {
    async fn ensure_running(&self, card: &Card) -> Result<()> {
        self.running.lock().await.push(card.id().to_string());
        Ok(())
    }
    async fn ensure_stopped(&self, card_id: &str) -> Result<()> {
        self.stopped.lock().await.push(card_id.into());
        Ok(())
    }
    async fn prepare_pr(&self, card: &Card) -> Result<Vec<PrCreated>> {
        self.prepared.lock().await.push(card.id().to_string());
        Ok(vec![])
    }
}
```

- [ ] **Step 4: Run all transition tests**

```bash
cargo test -p reeve-card --lib transitions
```
Expected: PASS, including the new mock test.

- [ ] **Step 5: Commit**

```bash
git add crates/reeve-card/src/transitions.rs
git commit -m "feat(card): extend WorkspaceLifecycle with prepare_pr port method"
```

---

### Task 8: Implement `LiveWorkspaceLifecycle::prepare_pr`

**Files:**
- Modify: `crates/reeve-workspace/src/lifecycle.rs`

Read the existing `LiveWorkspaceLifecycle::new` constructor and `impl WorkspaceLifecycle` block first to know the field layout. The new field is `pr_repo: Arc<dyn CardPrRepo>` plus `gh: GhOpsHandle`. Update both the struct, the constructor, and the dispatch-test factory function `make_workspace_lifecycle` in `dispatch.rs::tests`.

- [ ] **Step 1: Add the fields + constructor params**

In `crates/reeve-workspace/src/lifecycle.rs`, locate the `LiveWorkspaceLifecycle` struct definition and its `new` constructor. Add fields and ctor params:

```rust
pub struct LiveWorkspaceLifecycle {
    // ... existing fields ...
    pr_repo: Arc<dyn reeve_card::CardPrRepo>,
    gh: reeve_card::GhOpsHandle,
}

impl LiveWorkspaceLifecycle {
    pub fn new(
        // ... existing params ...
        pr_repo: Arc<dyn reeve_card::CardPrRepo>,
        gh: reeve_card::GhOpsHandle,
    ) -> Self {
        Self {
            // ... existing field inits ...
            pr_repo,
            gh,
        }
    }
}
```

- [ ] **Step 2: Implement `prepare_pr`**

Add to `impl WorkspaceLifecycle for LiveWorkspaceLifecycle`:

```rust
async fn prepare_pr(
    &self,
    card: &reeve_card::Card,
) -> anyhow::Result<Vec<reeve_card::PrCreated>> {
    use reeve_card::{CardPr, PrCreated};

    let row = self
        .household_repo
        .get(card.household())
        .await?
        .ok_or_else(|| anyhow::anyhow!("household '{}' not configured", card.household()))?;
    let household = reeve_household::Household {
        name: row.name,
        meta_repo_path: row.meta_repo_path,
    };
    let manifest = reeve_household::load_household_manifest(&household)?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let mut created: Vec<PrCreated> = Vec::new();

    for (repo_name, host_path) in reeve_household::iter_card_repos(&household, &manifest) {
        if !host_path.join(".git").exists() {
            continue;
        }
        let branches = self.git.list_namespace_refs(&host_path, card.id()).await?;
        for branch in branches {
            // 1. Fast-forward refs/heads/<branch> from the namespaced ref.
            let target = format!("refs/heads/{branch}");
            let source = format!("refs/reeve/{}/{}", card.id(), branch);
            match self.git.fast_forward(&host_path, &target, &source).await? {
                crate::git::FfOutcome::Updated => {}
                crate::git::FfOutcome::NotFastForward => anyhow::bail!(
                    "refs/heads/{branch} in {} is not fast-forward from card branch",
                    host_path.display()
                ),
            }
            // 2. Push to origin.
            self.git.push(&host_path, &branch).await?;
            // 3. Find or create the PR.
            let title = card.brief().chars().take(72).collect::<String>();
            let body = format!(
                "Card {} from the {} household.\n\nBrief: {}\n",
                card.id(),
                card.household(),
                card.brief()
            );
            let (pr_number, pr_url) = self
                .gh
                .create_pr(&host_path, &branch, "main", &title, &body)
                .await?;
            // 4. Record in card_prs.
            self.pr_repo
                .upsert(&CardPr {
                    card_id: card.id().to_string(),
                    repo_name: repo_name.to_string(),
                    pr_number,
                    pr_url: pr_url.clone(),
                    opened_at: now,
                })
                .await?;
            created.push(PrCreated {
                repo_name: repo_name.to_string(),
                pr_number,
                pr_url,
            });
        }
    }
    Ok(created)
}
```

- [ ] **Step 3: Build the workspace + run`-lib` tests**

```bash
cargo build -p reeve-workspace
cargo test -p reeve-workspace --lib
```
Expected: PASS. (The new method is exercised end-to-end in Phase-9 e2e; lifecycle.rs's unit tests don't pin gh-specific behaviour.)

- [ ] **Step 4: Commit**

```bash
git add crates/reeve-workspace/src/lifecycle.rs
git commit -m "feat(workspace): implement prepare_pr lifecycle method (push + gh + record)"
```

---

### Task 9: Wire `gh` and `pr_repo` into `DaemonState`

**Files:**
- Modify: `crates/reeved/src/app/state.rs` (add `gh: GhOpsHandle` and `pr_repo: Arc<dyn CardPrRepo>`)
- Modify: `crates/reeved/src/app/lifecycle.rs` (wire them on construction)
- Modify: `crates/reeved/src/dispatch.rs` (the `make_workspace_lifecycle` factory in tests)

- [ ] **Step 1: Locate state construction**

```bash
grep -n "DaemonState {" crates/reeved/src/app/lifecycle.rs crates/reeved/src/app/state.rs | head
```

- [ ] **Step 2: Add the fields and construction**

In `state.rs`, add to the `DaemonState` struct:
```rust
pub gh: reeve_card::GhOpsHandle,
pub pr_repo: Arc<dyn reeve_card::CardPrRepo>,
```

In `lifecycle.rs` at the construction site (next to existing `git`/`containers` lines):
```rust
let gh: reeve_card::GhOpsHandle = reeve_workspace::make_gh_ops();
let pr_repo = reeve_card::make_card_pr_repo(db.clone());
```

And pass `pr_repo.clone()` and `gh.clone()` to `LiveWorkspaceLifecycle::new(...)`. Include both new fields in the `DaemonState { ... }` literal.

- [ ] **Step 3: Update `dispatch.rs::tests::make_workspace_lifecycle`**

In `crates/reeved/src/dispatch.rs`, the test helper `make_workspace_lifecycle` must pass a `pr_repo` and `gh` too. Stub `gh` with a no-op:

```rust
struct NoopGh;
#[async_trait]
impl reeve_card::GhOps for NoopGh {
    async fn create_pr(&self, _p: &std::path::Path, _b: &str, _ba: &str, _t: &str, _bo: &str)
        -> anyhow::Result<(u64, String)> { Ok((0, String::new())) }
    async fn view_pr(&self, _p: &std::path::Path, _n: u64) -> anyhow::Result<reeve_card::GhPrView> {
        Ok(reeve_card::GhPrView { state: "OPEN".into(), mergeable: "UNKNOWN".into(), checks_status: "NONE".into() })
    }
    async fn list_comments(&self, _p: &std::path::Path, _n: u64) -> anyhow::Result<Vec<reeve_card::GhComment>> { Ok(vec![]) }
    async fn post_reply(&self, _p: &std::path::Path, _n: u64, _pid: u64, _b: &str) -> anyhow::Result<u64> { Ok(0) }
    async fn edit_pr(&self, _p: &std::path::Path, _n: u64, _t: Option<&str>, _b: Option<&str>) -> anyhow::Result<()> { Ok(()) }
    async fn list_prs_for_branch(&self, _p: &std::path::Path, _b: &str) -> anyhow::Result<Vec<u64>> { Ok(vec![]) }
}
```

Pass `Arc::new(NoopGh) as reeve_card::GhOpsHandle` and `reeve_card::make_card_pr_repo(db.clone())` into both `make_workspace_lifecycle` and the `DaemonState { ... }` literals (`fresh_state`, `fresh_state_with_git`, `fresh_state_with_configurable_git`, `fresh_state_with_containers`, `fresh_state_with_lifecycle`).

- [ ] **Step 4: Build everything**

```bash
cargo build --workspace
```
Expected: clean.

- [ ] **Step 5: Run all tests**

```bash
cargo test --workspace --lib && cargo test -p reeved
```
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/reeved/src/app/state.rs crates/reeved/src/app/lifecycle.rs crates/reeved/src/dispatch.rs
git commit -m "feat(reeved): wire GhOps + CardPrRepo into DaemonState"
```

---

### Task 10: Call `prepare_pr` from `handle_mv` and `handle_mv_self`

**Files:**
- Modify: `crates/reeve-card/src/handler.rs` (`handle_mv`, `handle_mv_self`)

Read the existing flow at `handler.rs:60-117` (handle_mv) and `handler.rs:119-174` (handle_mv_self). The new step runs after the household-exists gate and before `on_column_change`.

- [ ] **Step 1: Write the failing test**

Add to `crates/reeved/src/dispatch.rs::tests` (this is where end-to-end mv tests already live; use a `prepare_pr`-recording lifecycle mock):

```rust
#[tokio::test]
async fn mv_testing_to_pr_invokes_prepare_pr() {
    use std::sync::Mutex as StdMutex;
    struct RecordingLifecycle {
        prepare_calls: StdMutex<Vec<String>>,
    }
    #[async_trait]
    impl reeve_card::WorkspaceLifecycle for RecordingLifecycle {
        async fn ensure_running(&self, _card: &reeve_card::Card) -> Result<()> { Ok(()) }
        async fn ensure_stopped(&self, _id: &str) -> Result<()> { Ok(()) }
        async fn prepare_pr(&self, card: &reeve_card::Card) -> Result<Vec<reeve_card::PrCreated>> {
            self.prepare_calls.lock().unwrap().push(card.id().to_string());
            Ok(vec![])
        }
    }
    let lifecycle = Arc::new(RecordingLifecycle { prepare_calls: StdMutex::new(vec![]) });
    let state = fresh_state_with_lifecycle(lifecycle.clone());
    // Register household so the existence gate passes.
    state.household_repo.insert(reeve_household::HouseholdRow {
        name: "h".into(),
        meta_repo_path: std::path::PathBuf::from("/tmp/test-meta"),
        added_at: 0,
    }).await.unwrap();
    let card_id = create_card(&state, "pr-test").await;
    // Walk Backlog → Testing first (no PR side-effect on those moves).
    for col in ["planning", "implementing", "review", "testing"] {
        dispatch(RpcRequest { id: 1, method: "mv".into(),
            params: serde_json::json!({"id": card_id, "column": col})},
            state.clone(), CallerContext::host()).await.unwrap();
    }
    assert!(lifecycle.prepare_calls.lock().unwrap().is_empty(),
        "prepare_pr must not be called for non-Pr targets");

    // Now Testing → Pr should invoke prepare_pr exactly once.
    dispatch(RpcRequest { id: 99, method: "mv".into(),
        params: serde_json::json!({"id": card_id, "column": "pr"})},
        state, CallerContext::host()).await.unwrap();
    let calls = lifecycle.prepare_calls.lock().unwrap().clone();
    assert_eq!(calls, vec![card_id]);
}
```

- [ ] **Step 2: Run; expect failure**

```bash
cargo test -p reeved --lib mv_testing_to_pr_invokes_prepare_pr
```
Expected: FAIL — `prepare_pr` never called.

- [ ] **Step 3: Update `handle_mv` and `handle_mv_self` in `reeve-card/src/handler.rs`**

In both handlers, after the household-exists gate and *before* the call to `on_column_change`, add:

```rust
if new_column == reeve_protocol::Column::Pr && old_column != reeve_protocol::Column::Pr {
    workspace.prepare_pr(&card).await.map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("prepare_pr: {e:#}"),
    })?;
}
```

(Read the existing variable names for `new_column`/`old_column` and adapt — handler.rs already uses `card.column()` for the old column and `params.column` for the new.)

- [ ] **Step 4: Run the test**

```bash
cargo test -p reeved --lib mv_testing_to_pr_invokes_prepare_pr
```
Expected: PASS.

- [ ] **Step 5: Run the full suite — verify nothing regressed**

```bash
cargo test --workspace --lib && cargo test -p reeved
```
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/reeve-card/src/handler.rs crates/reeved/src/dispatch.rs
git commit -m "feat(card): invoke prepare_pr when card enters the Pr column"
```

---

## Phase 5 — `review-info` RPC and CLI

### Task 11: Add wire DTOs for review and PR types

**Files:**
- Modify: `crates/reeve-protocol/src/lib.rs`

- [ ] **Step 1: Append types to the protocol crate**

In `crates/reeve-protocol/src/lib.rs`, append (anywhere among the existing DTOs):

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewInfoParams { pub id: String }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewInfoResult {
    pub card: Card,
    pub repos: Vec<RepoReviewInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoReviewInfo {
    pub repo_name: String,
    pub ref_path: String,
    pub branch: String,
    pub head_sha: String,
    pub head_subject: String,
    pub commits_ahead: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrSummary {
    pub repo_name: String,
    pub pr_number: u64,
    pub pr_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrCreateSelfResult { pub prs: Vec<PrSummary> }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrStatusEntry {
    pub repo_name: String,
    pub pr_number: u64,
    pub state: String,
    pub mergeable: String,
    pub checks_status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrStatusSelfResult { pub statuses: Vec<PrStatusEntry> }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrCommentEntry {
    pub repo_name: String,
    pub pr_number: u64,
    pub comment_id: u64,
    pub author: String,
    pub path: Option<String>,
    pub line: Option<u32>,
    pub body: String,
    pub in_reply_to: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrCommentsSelfResult { pub comments: Vec<PrCommentEntry> }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrReplySelfParams {
    pub repo_name: String,
    pub parent_comment_id: u64,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrReplySelfResult { pub comment_id: u64 }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrEditSelfParams {
    pub repo_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrEditSelfResult { pub ok: bool }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrStatusParams { pub id: String }
```

- [ ] **Step 2: Build the workspace**

```bash
cargo build --workspace
```
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add crates/reeve-protocol/src/lib.rs
git commit -m "feat(protocol): wire DTOs for review-info and pr-* RPCs"
```

---

### Task 12: Implement `handle_review_info` in `reeve-workspace`

**Files:**
- Modify: `crates/reeve-workspace/src/handler.rs`
- Modify: `crates/reeve-workspace/src/lib.rs`

The handler needs git-log/diff data. Use `GitOps` for ref listing + add a small helper that shells out for `head_sha`, `head_subject`, and `commits_ahead`. For v1 keep the helper a small private function inside `handler.rs` that calls `tokio::process::Command` directly — extracting into `GitOps` is future work.

- [ ] **Step 1: Write the failing test**

Append to `crates/reeve-workspace/src/handler.rs::tests`:

```rust
#[tokio::test]
async fn review_info_returns_empty_when_no_branches() {
    use std::path::Path;
    struct EmptyGit;
    #[async_trait]
    impl GitOps for EmptyGit {
        async fn clone(&self, _s: &Path, _d: &Path) -> Result<()> { Ok(()) }
        async fn fetch(&self, _w: &Path, _s: &Path, _n: &str) -> Result<()> { Ok(()) }
        async fn list_namespace_refs(&self, _w: &Path, _n: &str) -> Result<Vec<String>> { Ok(vec![]) }
        async fn fast_forward(&self, _w: &Path, _t: &str, _s: &str) -> Result<crate::git::FfOutcome> { Ok(crate::git::FfOutcome::Updated) }
        async fn push(&self, _w: &Path, _b: &str) -> Result<()> { Ok(()) }
    }
    let dir = tempfile::tempdir().unwrap();
    let meta = dir.path().join("ws");
    std::fs::create_dir_all(&meta.join(".git")).unwrap();
    std::fs::write(meta.join("household.json"),
        r#"{"meta_repo":"ws","repos":[{"name":"ws"}]}"#).unwrap();

    let card = Card::from_persisted(
        "card-r".into(), "h".into(), "review me".into(),
        Column::Review, 0, 0, false, None,
    ).unwrap();
    let cards = OneCardRepo::new(card);
    let households = stub_household("h", &meta);   // helper: see Step 3.
    let git = EmptyGit;
    let params = reeve_protocol::ReviewInfoParams { id: "card-r".into() };
    let value = handle_review_info(&cards, &households, &git, params).await.unwrap();
    let result: reeve_protocol::ReviewInfoResult = serde_json::from_value(value).unwrap();
    assert_eq!(result.card.id, "card-r");
    assert!(result.repos.is_empty());
}
```

- [ ] **Step 2: Add the helper `stub_household` to the same test module**

Append to `crates/reeve-workspace/src/handler.rs::tests`:

```rust
fn stub_household(name: &str, meta_repo_path: &std::path::Path) -> impl reeve_household::HouseholdRepo {
    struct OneHousehold { row: reeve_household::HouseholdRow }
    #[async_trait]
    impl reeve_household::HouseholdRepo for OneHousehold {
        async fn insert(&self, _row: reeve_household::HouseholdRow) -> Result<()> { Ok(()) }
        async fn get(&self, name: &str) -> Result<Option<reeve_household::HouseholdRow>> {
            if name == self.row.name { Ok(Some(self.row.clone())) } else { Ok(None) }
        }
        async fn list(&self) -> Result<Vec<reeve_household::HouseholdRow>> { Ok(vec![self.row.clone()]) }
        async fn delete(&self, _name: &str) -> Result<bool> { Ok(true) }
    }
    OneHousehold {
        row: reeve_household::HouseholdRow {
            name: name.into(),
            meta_repo_path: meta_repo_path.to_path_buf(),
            added_at: 0,
        }
    }
}
```

If `HouseholdRepo`'s trait has a different signature than what's listed here (insert/get/list/delete), `grep -n "trait HouseholdRepo" crates/reeve-household/src/repo.rs` and conform.

- [ ] **Step 3: Implement `handle_review_info`**

Append to `crates/reeve-workspace/src/handler.rs`:

```rust
use reeve_protocol::{RepoReviewInfo, ReviewInfoParams, ReviewInfoResult};

pub async fn handle_review_info(
    cards: &dyn CardRepo,
    households: &dyn reeve_household::HouseholdRepo,
    git: &dyn GitOps,
    params: ReviewInfoParams,
) -> std::result::Result<Value, RpcError> {
    let card = cards
        .get(&params.id)
        .await
        .map_err(internal)?
        .ok_or_else(|| RpcError {
            code: RpcError::NOT_FOUND,
            message: format!("card '{}' not found", params.id),
        })?;
    let row = households
        .get(card.household())
        .await
        .map_err(internal)?
        .ok_or_else(|| RpcError {
            code: RpcError::INTERNAL,
            message: format!("household '{}' not configured", card.household()),
        })?;
    let household = reeve_household::Household {
        name: row.name,
        meta_repo_path: row.meta_repo_path,
    };
    let manifest = reeve_household::load_household_manifest(&household).map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("load manifest: {e:#}"),
    })?;

    let mut repos: Vec<RepoReviewInfo> = Vec::new();
    for (repo_name, host_path) in reeve_household::iter_card_repos(&household, &manifest) {
        if !host_path.join(".git").exists() {
            continue;
        }
        let branches = git
            .list_namespace_refs(&host_path, card.id())
            .await
            .map_err(internal)?;
        for branch in branches {
            let ref_path = format!("refs/reeve/{}/{}", card.id(), branch);
            let head_sha = run_git_oneline(&host_path, &["rev-parse", &ref_path]).await.unwrap_or_default();
            let head_subject = run_git_oneline(&host_path, &["log", "-1", "--format=%s", &ref_path]).await.unwrap_or_default();
            let count = run_git_oneline(&host_path, &["rev-list", "--count", &format!("main..{ref_path}")]).await
                .unwrap_or_default()
                .parse::<u32>()
                .unwrap_or(0);
            repos.push(RepoReviewInfo {
                repo_name: repo_name.to_string(),
                ref_path,
                branch,
                head_sha,
                head_subject,
                commits_ahead: count,
            });
        }
    }
    serde_json::to_value(ReviewInfoResult {
        card: card.into(),
        repos,
    })
    .map_err(internal)
}

async fn run_git_oneline(workdir: &std::path::Path, args: &[&str]) -> Option<String> {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(workdir)
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
```

In `crates/reeve-workspace/src/lib.rs`:
```rust
pub use handler::{handle_attach, handle_push, handle_review_info};
```

- [ ] **Step 4: Run the test**

```bash
cargo test -p reeve-workspace --lib handler::tests::review_info_returns_empty_when_no_branches
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/reeve-workspace/src/handler.rs crates/reeve-workspace/src/lib.rs
git commit -m "feat(workspace): add review-info RPC handler"
```

---

### Task 13: Dispatch `review-info` from reeved

**Files:**
- Modify: `crates/reeved/src/dispatch.rs`

- [ ] **Step 1: Add the dispatch arm**

In the `match req.method.as_str()` block of `dispatch.rs`, add (next to the existing `"attach"` and `"push"` arms):

```rust
"review-info" => {
    let cards = make_card_repo(state.db.clone());
    let params = parse_params::<reeve_protocol::ReviewInfoParams>(req.params)?;
    reeve_workspace::handle_review_info(
        &*cards,
        &*state.household_repo,
        &*state.git,
        params,
    ).await
}
```

- [ ] **Step 2: Build + run dispatch tests**

```bash
cargo test -p reeved --lib dispatch
```
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/reeved/src/dispatch.rs
git commit -m "feat(reeved): dispatch review-info RPC"
```

---

### Task 14: CLI subcommand `reeve review <id>`

**Files:**
- Modify: `crates/reeve/src/cli.rs`

- [ ] **Step 1: Add the subcommand**

In `crates/reeve/src/cli.rs`, locate the `pub enum Command { ... }` block and add (alongside `Push`):

```rust
/// Print a summary of the card's branches across affected repos, so the
/// user can open them in their preferred diff tool for review.
Review { id: String },
```

In `pub async fn run(cmd: Command)`, add the arm:

```rust
Command::Review { id } => review(id).await,
```

Then append the function:

```rust
async fn review(id: String) -> Result<()> {
    let socket = paths::host_socket();
    let value = client::call(&socket, "review-info", serde_json::json!({"id": id}))
        .await
        .context("calling review-info")?;
    let result: reeve_protocol::ReviewInfoResult = serde_json::from_value(value)?;
    println!("Card {}  brief={:?}  household={}",
        result.card.id, result.card.brief, result.card.household);
    println!("Status: in {} ({})",
        result.card.column.label(), result.card.column.as_wire_str());
    if result.repos.is_empty() {
        println!("\nNo affected repos yet — the card has no commits on top of main.");
        return Ok(());
    }
    println!("\nAffected repos:");
    for r in &result.repos {
        println!("  {}", r.repo_name);
        println!("    ref:     {}", r.ref_path);
        println!("    head:    {}  {}", &r.head_sha[..r.head_sha.len().min(8)], r.head_subject);
        println!("    commits ahead of main: {}", r.commits_ahead);
    }
    println!("\nOpen in your tool of choice. To proceed:");
    println!("  reeve mv {} testing       # approve, move to local testing", result.card.id);
    println!("  reeve mv {} implementing  # send back for changes", result.card.id);
    Ok(())
}
```

- [ ] **Step 2: Build the CLI**

```bash
cargo build -p reeve
```
Expected: clean.

- [ ] **Step 3: Smoke-test on a real card (manual; non-blocking)**

```bash
cargo run -p reeve -- review <some-card-id>
```
Expected: human-readable output. (Skip if no daemon running.)

- [ ] **Step 4: Commit**

```bash
git add crates/reeve/src/cli.rs
git commit -m "feat(reeve): add `reeve review <id>` CLI subcommand"
```

---

## Phase 6 — Self-scoped PR RPCs

Each of Tasks 15–19 follows the same TDD shape: failing dispatch test → handler skeleton → dispatch arm → passing test → commit. The handlers all live in `reeve-card/src/handler.rs` (they reference card identity via `caller_context.card_id` and may use `GhOps` and `CardPrRepo`).

### Task 15: `pr-create-self` — list recorded PRs for the calling card

**Files:**
- Modify: `crates/reeve-card/src/handler.rs`, `crates/reeve-card/src/lib.rs`
- Modify: `crates/reeved/src/dispatch.rs`

- [ ] **Step 1: Add the handler**

Append to `crates/reeve-card/src/handler.rs`:

```rust
pub async fn handle_pr_create_self(
    pr_repo: &dyn crate::pr_repo::CardPrRepo,
    card_id: String,
) -> std::result::Result<serde_json::Value, reeve_protocol::RpcError> {
    let rows = pr_repo.list_for_card(&card_id).await.map_err(internal)?;
    let prs = rows
        .into_iter()
        .map(|r| reeve_protocol::PrSummary {
            repo_name: r.repo_name,
            pr_number: r.pr_number,
            pr_url: r.pr_url,
        })
        .collect();
    serde_json::to_value(reeve_protocol::PrCreateSelfResult { prs }).map_err(internal)
}
```

Re-export from `lib.rs`. Add the dispatch arm:

```rust
"pr-create-self" => {
    let card_id = require_card(&caller)?;
    reeve_card::handle_pr_create_self(&*state.pr_repo, card_id).await
}
```

- [ ] **Step 2: Add a test in `dispatch.rs::tests`**

```rust
#[tokio::test]
async fn pr_create_self_returns_empty_when_no_prs_recorded() {
    let state = fresh_state();
    state.cards.insert(&Card::from_persisted(
        "c1".into(), "h".into(), "x".into(), Column::Pr, 0, 0, false, None
    ).unwrap()).await.unwrap();
    let value = dispatch(
        RpcRequest { id: 1, method: "pr-create-self".into(), params: serde_json::json!({}) },
        state, CallerContext::card("c1"),
    ).await.unwrap();
    let r: reeve_protocol::PrCreateSelfResult = serde_json::from_value(value).unwrap();
    assert!(r.prs.is_empty());
}
```

- [ ] **Step 3: Run + commit**

```bash
cargo test -p reeved --lib pr_create_self
git add -A
git commit -m "feat(card): pr-create-self RPC (list recorded PRs for the calling card)"
```

---

### Task 16: `pr-status-self` — query GitHub for PR state

**Files:** same as Task 15.

- [ ] **Step 1: Add handler**

```rust
pub async fn handle_pr_status_self(
    pr_repo: &dyn crate::pr_repo::CardPrRepo,
    households: &dyn HouseholdLookup, // needs an extension - see note
    gh: &dyn reeve_card::GhOps,
    card_id: String,
) -> std::result::Result<serde_json::Value, reeve_protocol::RpcError> {
    let prs = pr_repo.list_for_card(&card_id).await.map_err(internal)?;
    let mut statuses = Vec::new();
    for pr in prs {
        let repo_path = households
            .repo_path(&card_id, &pr.repo_name)
            .await
            .map_err(internal)?
            .ok_or_else(|| reeve_protocol::RpcError {
                code: reeve_protocol::RpcError::INTERNAL,
                message: format!("repo '{}' not in household", pr.repo_name),
            })?;
        let view = gh.view_pr(&repo_path, pr.pr_number).await.map_err(internal)?;
        statuses.push(reeve_protocol::PrStatusEntry {
            repo_name: pr.repo_name,
            pr_number: pr.pr_number,
            state: view.state,
            mergeable: view.mergeable,
            checks_status: view.checks_status,
        });
    }
    serde_json::to_value(reeve_protocol::PrStatusSelfResult { statuses }).map_err(internal)
}
```

**Important note:** `HouseholdLookup` does NOT currently expose `repo_path`. Extend the trait in `transitions.rs`:

```rust
#[async_trait]
pub trait HouseholdLookup: Send + Sync {
    async fn exists(&self, name: &str) -> Result<bool>;
    async fn card_paths(&self, name: &str) -> Result<CardPaths>;
    /// Resolve the on-disk path of a specific repo within a card's household.
    /// Returns Ok(None) if the household exists but doesn't declare the repo.
    async fn repo_path(&self, card_id: &str, repo_name: &str) -> anyhow::Result<Option<std::path::PathBuf>>;
}
```

Update the impl in `reeve-household` (mirror the existing `card_paths` impl pattern — load the manifest, find the entry, compute `meta_repo_path` for the meta-repo entry or `meta_repo_path/<sibling-name>` for siblings).

- [ ] **Step 2: Dispatch arm**

```rust
"pr-status-self" => {
    let card_id = require_card(&caller)?;
    reeve_card::handle_pr_status_self(
        &*state.pr_repo, &*state.household_lookup, &*state.gh, card_id,
    ).await
}
```

- [ ] **Step 3: Test** (using `NoopGh` from Task 9; assert empty status list for a card with no PRs)

```rust
#[tokio::test]
async fn pr_status_self_returns_empty_for_card_with_no_prs() {
    let state = fresh_state();
    state.cards.insert(&Card::from_persisted(
        "c1".into(), "h".into(), "x".into(), Column::Pr, 0, 0, false, None
    ).unwrap()).await.unwrap();
    let v = dispatch(RpcRequest { id: 1, method: "pr-status-self".into(),
        params: serde_json::json!({})}, state, CallerContext::card("c1")).await.unwrap();
    let r: reeve_protocol::PrStatusSelfResult = serde_json::from_value(v).unwrap();
    assert!(r.statuses.is_empty());
}
```

- [ ] **Step 4: Run + commit**

```bash
cargo test -p reeved --lib pr_status_self
git add -A
git commit -m "feat(card): pr-status-self RPC + HouseholdLookup::repo_path"
```

---

### Task 17: `pr-comments-self` — list review comments across all card PRs

**Files:**
- Modify: `crates/reeve-card/src/handler.rs`, `crates/reeve-card/src/lib.rs`
- Modify: `crates/reeved/src/dispatch.rs`

- [ ] **Step 1: Add the handler**

Append to `crates/reeve-card/src/handler.rs`:

```rust
pub async fn handle_pr_comments_self(
    pr_repo: &dyn crate::pr_repo::CardPrRepo,
    households: &dyn HouseholdLookup,
    gh: &dyn reeve_card::GhOps,
    card_id: String,
) -> std::result::Result<serde_json::Value, reeve_protocol::RpcError> {
    let prs = pr_repo.list_for_card(&card_id).await.map_err(internal)?;
    let mut comments = Vec::new();
    for pr in prs {
        let repo_path = households
            .repo_path(&card_id, &pr.repo_name)
            .await
            .map_err(internal)?
            .ok_or_else(|| reeve_protocol::RpcError {
                code: reeve_protocol::RpcError::INTERNAL,
                message: format!("repo '{}' not in household", pr.repo_name),
            })?;
        let raw = gh.list_comments(&repo_path, pr.pr_number).await.map_err(internal)?;
        for c in raw {
            comments.push(reeve_protocol::PrCommentEntry {
                repo_name: pr.repo_name.clone(),
                pr_number: pr.pr_number,
                comment_id: c.id,
                author: c.author,
                path: c.path,
                line: c.line,
                body: c.body,
                in_reply_to: c.in_reply_to,
            });
        }
    }
    serde_json::to_value(reeve_protocol::PrCommentsSelfResult { comments }).map_err(internal)
}
```

Re-export from `lib.rs`.

- [ ] **Step 2: Add dispatch arm**

In `crates/reeved/src/dispatch.rs`:

```rust
"pr-comments-self" => {
    let card_id = require_card(&caller)?;
    reeve_card::handle_pr_comments_self(
        &*state.pr_repo, &*state.household_lookup, &*state.gh, card_id,
    ).await
}
```

- [ ] **Step 3: Empty-list test**

Add to `crates/reeved/src/dispatch.rs::tests`:

```rust
#[tokio::test]
async fn pr_comments_self_returns_empty_for_card_with_no_prs() {
    let state = fresh_state();
    state.cards.insert(&Card::from_persisted(
        "c1".into(), "h".into(), "x".into(), Column::Pr, 0, 0, false, None
    ).unwrap()).await.unwrap();
    let v = dispatch(
        RpcRequest { id: 1, method: "pr-comments-self".into(), params: serde_json::json!({}) },
        state, CallerContext::card("c1"),
    ).await.unwrap();
    let r: reeve_protocol::PrCommentsSelfResult = serde_json::from_value(v).unwrap();
    assert!(r.comments.is_empty());
}
```

- [ ] **Step 4: Run + commit**

```bash
cargo test -p reeved --lib pr_comments_self
git add -A
git commit -m "feat(card): pr-comments-self RPC"
```

---

### Task 18: `pr-reply-self` — post a reply to a review comment

**Files:**
- Modify: `crates/reeve-card/src/handler.rs`, `crates/reeve-card/src/lib.rs`
- Modify: `crates/reeved/src/dispatch.rs`

**Card-scoping invariant:** the handler must verify that the (repo_name, parent_comment_id) belongs to *this* card's recorded PRs before calling `gh.post_reply`. Reject with `INVALID_ARGUMENT` otherwise. The agent can only reply to comments under its own PRs.

- [ ] **Step 1: Add the handler**

Append to `crates/reeve-card/src/handler.rs`:

```rust
pub async fn handle_pr_reply_self(
    pr_repo: &dyn crate::pr_repo::CardPrRepo,
    households: &dyn HouseholdLookup,
    gh: &dyn reeve_card::GhOps,
    card_id: String,
    params: reeve_protocol::PrReplySelfParams,
) -> std::result::Result<serde_json::Value, reeve_protocol::RpcError> {
    // 1. Find this card's PR record for the named repo.
    let pr = pr_repo
        .get(&card_id, &params.repo_name)
        .await
        .map_err(internal)?
        .ok_or_else(|| reeve_protocol::RpcError {
            code: reeve_protocol::RpcError::INVALID_ARGUMENT,
            message: format!(
                "no PR registered for card {} in repo {}",
                card_id, params.repo_name
            ),
        })?;
    // 2. Resolve the repo's host path.
    let repo_path = households
        .repo_path(&card_id, &params.repo_name)
        .await
        .map_err(internal)?
        .ok_or_else(|| reeve_protocol::RpcError {
            code: reeve_protocol::RpcError::INTERNAL,
            message: format!("repo '{}' not in household", params.repo_name),
        })?;
    // 3. Authorize the parent comment id by listing the PR's comments and
    //    rejecting if the id is not present.
    let existing = gh
        .list_comments(&repo_path, pr.pr_number)
        .await
        .map_err(internal)?;
    if !existing.iter().any(|c| c.id == params.parent_comment_id) {
        return Err(reeve_protocol::RpcError {
            code: reeve_protocol::RpcError::INVALID_ARGUMENT,
            message: format!(
                "comment {} does not belong to card {}'s PR #{}",
                params.parent_comment_id, card_id, pr.pr_number
            ),
        });
    }
    // 4. Post the reply.
    let new_id = gh
        .post_reply(&repo_path, pr.pr_number, params.parent_comment_id, &params.body)
        .await
        .map_err(internal)?;
    serde_json::to_value(reeve_protocol::PrReplySelfResult { comment_id: new_id })
        .map_err(internal)
}
```

Re-export from `lib.rs`.

- [ ] **Step 2: Add dispatch arm**

In `crates/reeved/src/dispatch.rs`:

```rust
"pr-reply-self" => {
    let card_id = require_card(&caller)?;
    let params = parse_params::<reeve_protocol::PrReplySelfParams>(req.params)?;
    reeve_card::handle_pr_reply_self(
        &*state.pr_repo, &*state.household_lookup, &*state.gh, card_id, params,
    ).await
}
```

- [ ] **Step 3: Card-scoping test**

Add to `crates/reeved/src/dispatch.rs::tests`:

```rust
#[tokio::test]
async fn pr_reply_self_rejects_when_no_pr_registered_for_repo() {
    let state = fresh_state();
    state.cards.insert(&Card::from_persisted(
        "c1".into(), "h".into(), "x".into(), Column::Pr, 0, 0, false, None
    ).unwrap()).await.unwrap();
    // Note: no pr_repo.upsert — the card has no PR records.
    let err = dispatch(
        RpcRequest { id: 1, method: "pr-reply-self".into(),
            params: serde_json::json!({
                "repo_name": "meta", "parent_comment_id": 7, "body": "thanks"
            }),
        },
        state, CallerContext::card("c1"),
    ).await.unwrap_err();
    assert_eq!(err.code, RpcError::INVALID_ARGUMENT);
    assert!(err.message.contains("no PR registered"), "msg: {}", err.message);
}
```

- [ ] **Step 4: Run + commit**

```bash
cargo test -p reeved --lib pr_reply_self
git add -A
git commit -m "feat(card): pr-reply-self RPC with card-scoped comment authorization"
```

---

### Task 19: `pr-edit-self` — update PR title/body

**Files:**
- Modify: `crates/reeve-card/src/handler.rs`, `crates/reeve-card/src/lib.rs`
- Modify: `crates/reeved/src/dispatch.rs`

- [ ] **Step 1: Add the handler**

Append to `crates/reeve-card/src/handler.rs`:

```rust
pub async fn handle_pr_edit_self(
    pr_repo: &dyn crate::pr_repo::CardPrRepo,
    households: &dyn HouseholdLookup,
    gh: &dyn reeve_card::GhOps,
    card_id: String,
    params: reeve_protocol::PrEditSelfParams,
) -> std::result::Result<serde_json::Value, reeve_protocol::RpcError> {
    let pr = pr_repo
        .get(&card_id, &params.repo_name)
        .await
        .map_err(internal)?
        .ok_or_else(|| reeve_protocol::RpcError {
            code: reeve_protocol::RpcError::INVALID_ARGUMENT,
            message: format!(
                "no PR registered for card {} in repo {}",
                card_id, params.repo_name
            ),
        })?;
    let repo_path = households
        .repo_path(&card_id, &params.repo_name)
        .await
        .map_err(internal)?
        .ok_or_else(|| reeve_protocol::RpcError {
            code: reeve_protocol::RpcError::INTERNAL,
            message: format!("repo '{}' not in household", params.repo_name),
        })?;
    gh.edit_pr(&repo_path, pr.pr_number, params.title.as_deref(), params.body.as_deref())
        .await
        .map_err(internal)?;
    serde_json::to_value(reeve_protocol::PrEditSelfResult { ok: true }).map_err(internal)
}
```

Re-export from `lib.rs`.

- [ ] **Step 2: Add dispatch arm**

In `crates/reeved/src/dispatch.rs`:

```rust
"pr-edit-self" => {
    let card_id = require_card(&caller)?;
    let params = parse_params::<reeve_protocol::PrEditSelfParams>(req.params)?;
    reeve_card::handle_pr_edit_self(
        &*state.pr_repo, &*state.household_lookup, &*state.gh, card_id, params,
    ).await
}
```

- [ ] **Step 3: Card-scoping test**

Add to `crates/reeved/src/dispatch.rs::tests`:

```rust
#[tokio::test]
async fn pr_edit_self_rejects_when_no_pr_registered_for_repo() {
    let state = fresh_state();
    state.cards.insert(&Card::from_persisted(
        "c1".into(), "h".into(), "x".into(), Column::Pr, 0, 0, false, None
    ).unwrap()).await.unwrap();
    let err = dispatch(
        RpcRequest { id: 1, method: "pr-edit-self".into(),
            params: serde_json::json!({
                "repo_name": "meta", "title": "new", "body": null
            }),
        },
        state, CallerContext::card("c1"),
    ).await.unwrap_err();
    assert_eq!(err.code, RpcError::INVALID_ARGUMENT);
    assert!(err.message.contains("no PR registered"));
}
```

- [ ] **Step 4: Run + commit**

```bash
cargo test -p reeved --lib pr_edit_self
git add -A
git commit -m "feat(card): pr-edit-self RPC with card-scoped PR authorization"
```

---

## Phase 7 — CLI convenience

### Task 20: `reeve pr-status <id>` host-callable command

**Files:**
- Modify: `crates/reeve-card/src/handler.rs`, `crates/reeve-card/src/lib.rs`
- Modify: `crates/reeved/src/dispatch.rs`
- Modify: `crates/reeve/src/cli.rs`

- [ ] **Step 1: Host-side handler in `reeve-card/src/handler.rs`**

```rust
pub async fn handle_pr_status(
    pr_repo: &dyn crate::pr_repo::CardPrRepo,
    households: &dyn HouseholdLookup,
    gh: &dyn reeve_card::GhOps,
    params: reeve_protocol::PrStatusParams,
) -> std::result::Result<serde_json::Value, reeve_protocol::RpcError> {
    let prs = pr_repo.list_for_card(&params.id).await.map_err(internal)?;
    let mut statuses = Vec::new();
    for pr in prs {
        let repo_path = households
            .repo_path(&params.id, &pr.repo_name)
            .await
            .map_err(internal)?
            .ok_or_else(|| reeve_protocol::RpcError {
                code: reeve_protocol::RpcError::INTERNAL,
                message: format!("repo '{}' not in household", pr.repo_name),
            })?;
        let view = gh.view_pr(&repo_path, pr.pr_number).await.map_err(internal)?;
        statuses.push(reeve_protocol::PrStatusEntry {
            repo_name: pr.repo_name,
            pr_number: pr.pr_number,
            state: view.state,
            mergeable: view.mergeable,
            checks_status: view.checks_status,
        });
    }
    serde_json::to_value(reeve_protocol::PrStatusSelfResult { statuses }).map_err(internal)
}
```

Re-export from `lib.rs`.

- [ ] **Step 2: Dispatch arm**

In `crates/reeved/src/dispatch.rs`:

```rust
"pr-status" => {
    let params = parse_params::<reeve_protocol::PrStatusParams>(req.params)?;
    reeve_card::handle_pr_status(
        &*state.pr_repo, &*state.household_lookup, &*state.gh, params,
    ).await
}
```

- [ ] **Step 3: CLI subcommand**

In `crates/reeve/src/cli.rs`, add to `pub enum Command`:

```rust
/// Print PR state + CI status for a card's open PRs.
PrStatus { id: String },
```

In `pub async fn run`, add the arm `Command::PrStatus { id } => pr_status(id).await,` then append:

```rust
async fn pr_status(id: String) -> Result<()> {
    let socket = paths::host_socket();
    let value = client::call(&socket, "pr-status", serde_json::json!({"id": id}))
        .await
        .context("calling pr-status")?;
    let result: reeve_protocol::PrStatusSelfResult = serde_json::from_value(value)?;
    if result.statuses.is_empty() {
        println!("No PRs registered for this card.");
        return Ok(());
    }
    for s in &result.statuses {
        println!("{}  PR #{}  state={}  mergeable={}  checks={}",
            s.repo_name, s.pr_number, s.state, s.mergeable, s.checks_status);
    }
    Ok(())
}
```

- [ ] **Step 4: Build + smoke test**

```bash
cargo build -p reeve
cargo test --workspace --lib && cargo test -p reeved
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(reeve): add \`reeve pr-status <id>\` CLI command"
```

---

## Phase 8 — SessionStart prompt update

### Task 21: Update `session_init.rs` template

**Files:**
- Modify: `crates/reeve-card/src/session_init.rs`

- [ ] **Step 1: Write the failing tests**

Update the existing `renders_with_default_paths` test:

```rust
#[test]
fn renders_with_default_paths() {
    let c = card("abc123", "witan", "Add OAuth login");
    let paths = CardPaths::from_optional(None, None);
    let out = render(&c, &paths);
    assert!(out.contains("reeve card abc123"));
    assert!(out.contains("in the witan household"));
    assert!(out.contains("Brief: Add OAuth login"));
    assert!(out.contains("docs/agent/specs/abc123-design.md"));
    assert!(out.contains("docs/agent/plans/abc123.md"));
    assert!(out.contains("reeve mv-self planning"));
    assert!(out.contains("reeve mv-self implementing"));
    assert!(out.contains("reeve mv-self review"));
    // The human runs the testing and ship steps; the agent doesn't mv-self past review.
    assert!(!out.contains("reeve mv-self testing"));
    // PR-collaboration verbs the agent uses in the Pr column:
    assert!(out.contains("reeve pr-status-self"));
    assert!(out.contains("reeve pr-comments-self"));
    assert!(out.contains("reeve pr-reply-self"));
    assert!(out.contains("reeve pr-edit-self"));
}
```

- [ ] **Step 2: Run; expect failure**

```bash
cargo test -p reeve-card --lib session_init
```
Expected: FAIL on the new asserts.

- [ ] **Step 3: Update `render` to match the new column flow**

Replace the body of `render` with the new template (closely matches the spec's "SessionStart prompt update" section — verify against the spec when editing):

```rust
pub fn render(card: &Card, paths: &CardPaths) -> String {
    format!(
        "You are reeve card {id} in the {household} household.\n\
         Brief: {brief}\n\
         \n\
         Take this card through these phases:\n\
         \x20\x201. Brainstorm the design  \u{2192}  {spec_path}/{id}-design.md\n\
         \x20\x202. Write the implementation plan  \u{2192}  {plan_path}/{id}.md\n\
         \x20\x203. Execute the plan\n\
         \x20\x204. Submit for human review\n\
         \n\
         Your household's installed plugin provides the skills for each\n\
         phase \u{2014} invoke them as they apply.\n\
         \n\
         Signal phase changes to the kanban board as you progress:\n\
         \x20\x20reeve mv-self planning      (during brainstorming + planning)\n\
         \x20\x20reeve mv-self implementing  (when executing the plan)\n\
         \x20\x20reeve mv-self review        (when you believe the implementation is done)\n\
         \n\
         The human reviews your code and tests it locally; if they ship it,\n\
         they'll open a PR and move you to the PR column. There you can:\n\
         \x20\x20reeve pr-status-self    (check PR state + CI)\n\
         \x20\x20reeve pr-comments-self  (read review comments)\n\
         \x20\x20reeve pr-reply-self     (post a reply)\n\
         \x20\x20reeve pr-edit-self      (update PR title/body)\n\
         \n\
         If the human moves you back to Implementing, address their feedback\n\
         and call mv-self review again when ready.",
        id = card.id(),
        household = card.household(),
        brief = card.brief(),
        spec_path = paths.spec_path,
        plan_path = paths.plan_path,
    )
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p reeve-card --lib session_init
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/reeve-card/src/session_init.rs
git commit -m "feat(card): update SessionStart prompt for the new review/PR flow"
```

---

## Phase 9 — TUI and container template

### Task 22: TUI column-specific arms

After Task 2 the TUI builds, but a few sites special-case columns (e.g. `modal.rs::at_first` checks `== Backlog`; that still works). Verify the kanban column ordering renders correctly.

- [ ] **Step 1: Run the TUI test suite**

```bash
cargo test -p reeve --lib tui
```
Expected: PASS.

- [ ] **Step 2: Visual smoke-test**

```bash
cargo run -p reeve -- tui
```
Verify: 7 columns appear left-to-right in the order Backlog, Planning, Implementing, Review, Testing, PR, Done.

- [ ] **Step 3: If any column is missing or out of order**

Likely cause: `render.rs:580` uses `Column::ALL` (correct, no action needed) but `render.rs:432` may have hand-written ordering. Patch any list literals to use `Column::ALL` instead of hardcoded vectors.

- [ ] **Step 4: Commit (if changes needed)**

```bash
git add -A
git commit -m "fix(tui): render the PR column in canonical order"
```

---

### Task 23: Verify the devcontainer template does not install `gh`

**Files:**
- Inspect: `.devcontainer/devcontainer.json` (and any household scaffolding templates in `crates/reeve-household/src/scaffold.rs`)

- [ ] **Step 1: Grep for gh installation directives**

```bash
grep -rn "gh\b\|github-cli\|github_cli" .devcontainer/ crates/reeve-household/src/scaffold.rs 2>/dev/null | grep -v binary
```
Expected: no matches that install gh inside the container. Document this absence in the workspace lore (Task 25).

- [ ] **Step 2: No-op task — no commit needed if there were no matches**

If `gh` is being installed by the existing template, remove it and commit with `chore(devcontainer): keep gh out of the card container (host-mediated)`.

---

## Phase 10 — Documentation

### Task 24: Update `lore/knowledge/domain/card.md`

Reflect the column reorder, the new Pr column, the new RPCs (`pr-create-self`, `pr-status-self`, `pr-comments-self`, `pr-reply-self`, `pr-edit-self`, plus `pr-status`), the new `card_prs` satellite table, and the `prepare_pr` port method that Card consumes from Workspace.

- [ ] **Step 1: Edit the file.** Specific updates:

  **Purpose** (§"Purpose"): change `(Backlog → Planning → Implementing → Testing → Review → Done)` to `(Backlog → Planning → Implementing → Review → Testing → PR → Done)`. Mention that PR is an active column whose entry has a host-side side effect (push + `gh pr create`).

  **Key entities** (§"Key entities"): in the `Column` value-enum bullet, list the 7 variants in the new order. In the bullet for "what Card needs from Workspace," extend `WorkspaceLifecycle` to include `prepare_pr(&Card) -> Vec<PrCreated>`. Add a new bullet for `CardPrRepo` — the satellite table the agent uses to look up its own card's PRs.

  **Ubiquitous language** (§"Ubiquitous language"): add a *PR column* glossary entry — the kanban column representing "the card has an open PR." Note the explicit "PR" capitalisation (label, not wire). Add a *prepare_pr* entry — the lifecycle method that pushes affected branches and opens GitHub PRs (idempotent on retry).

  **Invariants** (§"Invariants"): keep the existing four invariants. Add a fifth: *prepare_pr runs before the column commit on entering PR.* If push/create fails, the card stays in its source column. Idempotency: re-entering PR detects existing PRs via `gh.list_prs_for_branch` and reuses them.

  **Column-transition policy** (§"Column-transition policy"): the transition matrix is unchanged (still 4 arms). Add an explicit sentence: *entering PR from any non-PR column also calls `WorkspaceLifecycle::prepare_pr(&card)` before the column commit.* Update the agent-driven-transitions paragraph: agent only `mv-self`s to planning/implementing/review now; testing and PR are human-driven.

  **RPC surface** (§"RPC surface"): the table grows by 5 self-scoped RPCs (`pr-create-self`, `pr-status-self`, `pr-comments-self`, `pr-reply-self`, `pr-edit-self`) and 1 host RPC (`pr-status`). Add one row per RPC with a one-sentence description matching the spec.

  **Integration points** (§"Integration points"): in the outbound section, mention the new `GhOps` port (defined in `reeve-card` per the inverted-dep rule — consumer defines what it needs — and implemented in `reeve-workspace::RealGhOps`). Mention the new `CardPrRepo` port + `card_prs` satellite table (both owned by Card).
- [ ] **Step 2: Run lore validators**

```bash
make validate
```
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add lore/knowledge/domain/card.md
git commit -m "docs(lore): card.md reflects review/PR flow + Pr column + new RPCs"
```

---

### Task 25: Update `lore/knowledge/domain/workspace.md`

Add the new RPC (`review-info`), the new `GhOps` port and adapter, the new `prepare_pr` implementation, and the "gh on host, never in container" property.

- [ ] **Step 1: Edit the file**
- [ ] **Step 2: Run validators**

```bash
make doctor
```
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add lore/knowledge/domain/workspace.md
git commit -m "docs(lore): workspace.md reflects GhOps port + prepare_pr + review-info"
```

---

## Final verification

Before declaring the plan complete, run the full battery from `CLAUDE.md`:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib
cargo test -p reeved
scripts/check-context-isolation.sh
make validate
```

All must pass. If any step fails, fix and amend the affected task's commit (or add a follow-up commit `chore: fmt + lint after Tasks N–M`).

---

## Notes for the executing engineer

- **Don't skip TDD.** Every handler has a happy-path test and at least one negative test (card-scoping for `pr-reply-self`, `pr-edit-self`). Add more if you see gaps.
- **Don't break existing dispatch tests.** Phase 4 Task 9 is the riskiest — it changes the construction of `DaemonState` across five test factories. If a test fails compile, it's almost always because the factory wasn't updated.
- **Read the spec** (`docs/superpowers/specs/2026-05-27-review-and-pr-flow-design.md`) at the start of each phase. It has the "why" for choices the plan only states.
- **`gh` is a runtime prereq** for end-to-end PR ops. Unit and dispatch tests use `NoopGh`. E2E PR coverage is documented in the spec's "Testing" section as gated by `REEVE_E2E_GH=1` (or similar) — implementing that gate is future work, not part of this plan.
