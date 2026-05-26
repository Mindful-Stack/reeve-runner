# Planning Column Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Promote `Planning` to an "active" kanban column so the agent runs in the card's dev container during planning, brainstorms + writes a plan, and self-advances via `mv-self`. Reeve seeds the agent once at SessionStart with a plugin-agnostic phase brief.

**Architecture:** Four bounded-context crates touch this work — `reeve-card` (transitions + new handler + prompt template), `reeve-household` (manifest gains `spec_path`/`plan_path` + `HouseholdLookup` grows a `card_paths` method), `reeve-workspace` (writes `.claude/settings.local.json` at provision), `reeved` (new dispatch arm). Plus one CLI subcommand. Each change respects the inverted-dependency port pattern from `lore/knowledge/general/architecture.md`.

**Tech Stack:** Rust 2021, tokio + async-trait, rusqlite via `spawn_blocking`, serde + serde_json, anyhow.

---

## File Structure

**New files**
- `crates/reeve-card/src/session_init.rs` — prompt template + render function (pure)

**Modified files**
- `crates/reeve-household/src/manifest.rs` — two optional fields on `Manifest`
- `crates/reeve-card/src/transitions.rs` — extend `HouseholdLookup` trait + simplify transition matrix
- `crates/reeve-household/src/repo.rs` — implement new trait method on `SqliteHouseholdRepo`
- `crates/reeve-protocol/src/lib.rs` — `SessionInitSelfParams` + `SessionInitSelfResult`
- `crates/reeve-card/src/handler.rs` — `handle_session_init_self`
- `crates/reeve-card/src/lib.rs` — re-export new handler + new module
- `crates/reeved/src/dispatch.rs` — `session-init-self` arm
- `crates/reeve/src/cli.rs` — `reeve session-init-self` subcommand
- `crates/reeve-workspace/src/lifecycle.rs` — write `.claude/settings.local.json` after clone

**Tests touched**
- `crates/reeve-card/src/transitions.rs::tests` (existing matrix tests change; new ones added)
- `crates/reeve-card/src/handler.rs::tests` (mock `HouseholdLookup` gains the new method; new handler tests)
- `crates/reeve-household/src/manifest.rs::tests` (new field parsing)
- `crates/reeve-household/src/repo.rs::tests` (new trait method)
- `crates/reeve-workspace/src/lifecycle.rs::tests` (settings.local.json write)
- `crates/reeved/tests` or `crates/reeved/src/dispatch.rs::tests` (dispatch arm)

---

## Task 1: Add `spec_path` and `plan_path` to the manifest

**Files:**
- Modify: `crates/reeve-household/src/manifest.rs`

The `Manifest` struct gains two optional string fields. When absent, defaults are `docs/agent/specs` and `docs/agent/plans`. Existing manifests parse unchanged.

- [ ] **Step 1: Write failing test for the new fields**

In `crates/reeve-household/src/manifest.rs::tests`, add:

```rust
#[test]
fn parses_manifest_with_explicit_spec_and_plan_paths() {
    let dir = tempdir().unwrap();
    let root = write_manifest(
        dir.path(),
        r#"{
          "meta_repo": "ws",
          "spec_path": "docs/superpowers/specs",
          "plan_path": "docs/superpowers/plans",
          "repos": [{ "name": "ws" }]
        }"#,
    );
    let m = load_manifest(&root).unwrap();
    assert_eq!(m.spec_path.as_deref(), Some("docs/superpowers/specs"));
    assert_eq!(m.plan_path.as_deref(), Some("docs/superpowers/plans"));
}

#[test]
fn parses_manifest_without_spec_or_plan_paths_yields_none() {
    let dir = tempdir().unwrap();
    let root = write_manifest(
        dir.path(),
        r#"{ "meta_repo": "ws", "repos": [{ "name": "ws" }] }"#,
    );
    let m = load_manifest(&root).unwrap();
    assert!(m.spec_path.is_none());
    assert!(m.plan_path.is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p reeve-household manifest::tests::parses_manifest_with_explicit -- --nocapture`
Expected: FAIL (no such field on `Manifest`).

- [ ] **Step 3: Add the two fields to `Manifest`**

In `crates/reeve-household/src/manifest.rs`, edit the `Manifest` struct:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Manifest {
    /// Name of the repos[] entry that IS the meta-repo.
    /// Renamed from `workspace` in 2026-05; the old field name is no
    /// longer accepted.
    pub meta_repo: String,

    #[serde(default)]
    pub knowledge_base: Option<String>,

    /// Plugin-agnostic path (relative to the meta-repo) where the planning
    /// agent should write spec documents. Defaults to `docs/agent/specs` when
    /// absent; superpowers households override to `docs/superpowers/specs`.
    #[serde(default)]
    pub spec_path: Option<String>,

    /// Plugin-agnostic path (relative to the meta-repo) where the planning
    /// agent should write implementation plans. Defaults to `docs/agent/plans`
    /// when absent.
    #[serde(default)]
    pub plan_path: Option<String>,

    pub repos: Vec<RepoEntry>,
}
```

- [ ] **Step 4: Run the new tests to verify they pass**

Run: `cargo test -p reeve-household manifest::tests -- --nocapture`
Expected: PASS. The pre-existing tests (`parses_full_manifest`, etc.) still pass because the fields are optional.

- [ ] **Step 5: Run all reeve-household tests to confirm no regressions**

Run: `cargo test -p reeve-household`
Expected: All green.

- [ ] **Step 6: Commit**

```bash
git add crates/reeve-household/src/manifest.rs
git commit -m "$(cat <<'EOF'
feat(household): add optional spec_path/plan_path manifest fields

Two new Option<String> fields with serde defaults. Existing manifests
continue to parse; new households can override the plugin-neutral
defaults (docs/agent/{specs,plans}) by setting these explicitly.

Co-Authored-By: Claude
EOF
)"
```

---

## Task 2: Define `CardPaths` and extend `HouseholdLookup` with `card_paths`

**Files:**
- Modify: `crates/reeve-card/src/transitions.rs`
- Modify: `crates/reeve-card/src/handler.rs` (mock impl in tests)

The Card aggregate consumes a new bit of household-shaped data: where to write spec and plan documents. We extend the existing `HouseholdLookup` port with a `card_paths` method that returns resolved paths (defaults applied).

- [ ] **Step 1: Add `CardPaths` struct + extend `HouseholdLookup` trait**

In `crates/reeve-card/src/transitions.rs`, replace the `HouseholdLookup` trait block with:

```rust
/// Resolved per-card output paths for the planning agent. Returned by
/// `HouseholdLookup::card_paths`. Plugin-agnostic defaults are applied
/// here so consumers (the session-init handler) don't need to know
/// the defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardPaths {
    /// Relative to the meta-repo clone — where spec markdown lives.
    /// Default `docs/agent/specs`.
    pub spec_path: String,
    /// Relative to the meta-repo clone — where plan markdown lives.
    /// Default `docs/agent/plans`.
    pub plan_path: String,
}

impl CardPaths {
    pub const DEFAULT_SPEC_PATH: &'static str = "docs/agent/specs";
    pub const DEFAULT_PLAN_PATH: &'static str = "docs/agent/plans";

    /// Apply defaults to optional manifest fields.
    pub fn from_optional(spec: Option<&str>, plan: Option<&str>) -> Self {
        Self {
            spec_path: spec.unwrap_or(Self::DEFAULT_SPEC_PATH).to_string(),
            plan_path: plan.unwrap_or(Self::DEFAULT_PLAN_PATH).to_string(),
        }
    }
}

/// Port the Card aggregate uses to check household existence and read
/// the card-output paths a household declares. Implemented in
/// `reeve-household`.
#[async_trait]
pub trait HouseholdLookup: Send + Sync {
    async fn exists(&self, name: &str) -> Result<bool>;
    /// Resolve the household's `spec_path`/`plan_path` from its manifest,
    /// applying `CardPaths` defaults where the manifest leaves them unset.
    /// Returns Err if the household is not registered or its manifest
    /// cannot be loaded.
    async fn card_paths(&self, name: &str) -> Result<CardPaths>;
}
```

- [ ] **Step 2: Re-export `CardPaths` from `reeve-card/src/lib.rs`**

In `crates/reeve-card/src/lib.rs`, update the transitions re-export line:

```rust
pub use transitions::{CardPaths, HouseholdLookup, WorkspaceLifecycle, on_column_change};
```

- [ ] **Step 3: Update the test mock in `handler.rs`**

In `crates/reeve-card/src/handler.rs::tests`, find `AlwaysExistsHouseholds` and replace its impl block:

```rust
#[async_trait::async_trait]
impl crate::HouseholdLookup for AlwaysExistsHouseholds {
    async fn exists(&self, _name: &str) -> anyhow::Result<bool> {
        Ok(true)
    }
    async fn card_paths(&self, _name: &str) -> anyhow::Result<crate::CardPaths> {
        Ok(crate::CardPaths::from_optional(None, None))
    }
}
```

- [ ] **Step 4: Verify the workspace still compiles**

Run: `cargo build --workspace`
Expected: Compile errors in `reeve-household/src/repo.rs` (its `HouseholdLookup` impl is missing `card_paths`). That's intentional — Task 3 fixes it.

- [ ] **Step 5: Run reeve-card tests to confirm the card crate is internally consistent**

Run: `cargo test -p reeve-card --lib`
Expected: PASS (reeve-card doesn't have a real impl of `HouseholdLookup`; its mock provides the new method).

- [ ] **Step 6: Add a unit test for `CardPaths::from_optional`**

In `crates/reeve-card/src/transitions.rs::tests`, append:

```rust
#[test]
fn card_paths_defaults_when_both_absent() {
    let p = CardPaths::from_optional(None, None);
    assert_eq!(p.spec_path, "docs/agent/specs");
    assert_eq!(p.plan_path, "docs/agent/plans");
}

#[test]
fn card_paths_uses_provided_values() {
    let p = CardPaths::from_optional(Some("docs/superpowers/specs"), Some("docs/superpowers/plans"));
    assert_eq!(p.spec_path, "docs/superpowers/specs");
    assert_eq!(p.plan_path, "docs/superpowers/plans");
}

#[test]
fn card_paths_mixed_provided_and_default() {
    let p = CardPaths::from_optional(Some("custom/specs"), None);
    assert_eq!(p.spec_path, "custom/specs");
    assert_eq!(p.plan_path, "docs/agent/plans");
}
```

- [ ] **Step 7: Run the new tests**

Run: `cargo test -p reeve-card transitions::tests::card_paths -- --nocapture`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/reeve-card/src/transitions.rs crates/reeve-card/src/lib.rs crates/reeve-card/src/handler.rs
git commit -m "$(cat <<'EOF'
feat(card): add CardPaths + extend HouseholdLookup with card_paths

CardPaths carries resolved spec_path/plan_path with plugin-neutral
defaults applied. HouseholdLookup grows a card_paths method; the
existing exists() is unchanged. Mock impl in handler tests updated.

Co-Authored-By: Claude
EOF
)"
```

---

## Task 3: Implement `card_paths` on `SqliteHouseholdRepo`

**Files:**
- Modify: `crates/reeve-household/src/repo.rs`

The supplier implements the new method by looking up the household row, loading the manifest, and applying defaults. This unblocks the workspace build.

- [ ] **Step 1: Write a failing test**

In `crates/reeve-household/src/repo.rs::tests` (or add `#[cfg(test)] mod tests` if absent), add:

```rust
#[tokio::test]
async fn card_paths_reads_manifest_fields() {
    use reeve_card::{CardPaths, HouseholdLookup};
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let meta = dir.path().join("ws");
    std::fs::create_dir_all(&meta).unwrap();
    std::fs::write(
        meta.join("household.json"),
        r#"{
          "meta_repo": "ws",
          "spec_path": "docs/foo/specs",
          "plan_path": "docs/foo/plans",
          "repos": [{ "name": "ws" }]
        }"#,
    ).unwrap();

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::init_schema(&conn).unwrap();
    let (repo, lookup) = crate::make_household_components(
        std::sync::Arc::new(tokio::sync::Mutex::new(conn)),
    );
    repo.upsert(&crate::HouseholdRow {
        name: "h".into(),
        meta_repo_path: meta.clone(),
    }).await.unwrap();

    let paths = lookup.card_paths("h").await.unwrap();
    assert_eq!(paths, CardPaths {
        spec_path: "docs/foo/specs".into(),
        plan_path: "docs/foo/plans".into(),
    });
}

#[tokio::test]
async fn card_paths_returns_defaults_when_manifest_omits_them() {
    use reeve_card::HouseholdLookup;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let meta = dir.path().join("ws");
    std::fs::create_dir_all(&meta).unwrap();
    std::fs::write(
        meta.join("household.json"),
        r#"{ "meta_repo": "ws", "repos": [{ "name": "ws" }] }"#,
    ).unwrap();

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::init_schema(&conn).unwrap();
    let (repo, lookup) = crate::make_household_components(
        std::sync::Arc::new(tokio::sync::Mutex::new(conn)),
    );
    repo.upsert(&crate::HouseholdRow {
        name: "h".into(),
        meta_repo_path: meta.clone(),
    }).await.unwrap();

    let paths = lookup.card_paths("h").await.unwrap();
    assert_eq!(paths.spec_path, "docs/agent/specs");
    assert_eq!(paths.plan_path, "docs/agent/plans");
}

#[tokio::test]
async fn card_paths_errors_when_household_missing() {
    use reeve_card::HouseholdLookup;
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::init_schema(&conn).unwrap();
    let (_repo, lookup) = crate::make_household_components(
        std::sync::Arc::new(tokio::sync::Mutex::new(conn)),
    );
    let err = lookup.card_paths("missing").await.unwrap_err();
    assert!(format!("{err:#}").contains("missing"));
}
```

(If the test module exists already, just append these; check the existing imports — the snippets assume `rusqlite::Connection`, `tokio::sync::Mutex`, `std::sync::Arc`, and `tempfile::tempdir` are usable. `tempfile` is already a dev-dependency for the manifest tests; verify with `grep tempfile crates/reeve-household/Cargo.toml`.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p reeve-household repo::tests::card_paths -- --nocapture`
Expected: FAIL at compile time — `SqliteHouseholdRepo` doesn't implement `card_paths` yet.

- [ ] **Step 3: Implement `card_paths`**

In `crates/reeve-household/src/repo.rs`, replace the existing `HouseholdLookup` impl block:

```rust
#[async_trait]
impl reeve_card::HouseholdLookup for SqliteHouseholdRepo {
    async fn exists(&self, name: &str) -> Result<bool> {
        Ok(self.get(name).await?.is_some())
    }

    async fn card_paths(&self, name: &str) -> Result<reeve_card::CardPaths> {
        let row = self.get(name).await?.ok_or_else(|| {
            anyhow::anyhow!("household '{}' missing — cannot resolve card paths", name)
        })?;
        let household = crate::Household {
            name: row.name,
            meta_repo_path: row.meta_repo_path,
        };
        let manifest = crate::load_household_manifest(&household)?;
        Ok(reeve_card::CardPaths::from_optional(
            manifest.spec_path.as_deref(),
            manifest.plan_path.as_deref(),
        ))
    }
}
```

- [ ] **Step 4: Run the new tests to verify they pass**

Run: `cargo test -p reeve-household repo::tests::card_paths -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Run the full workspace build + tests**

Run: `cargo build --workspace && cargo test --workspace --lib`
Expected: All green. The earlier compile error from Task 2 step 4 is resolved.

- [ ] **Step 6: Commit**

```bash
git add crates/reeve-household/src/repo.rs
git commit -m "$(cat <<'EOF'
feat(household): implement card_paths on SqliteHouseholdRepo

Looks up the household row, loads its manifest, applies the
CardPaths defaults. Returns Err for unregistered households.

Co-Authored-By: Claude
EOF
)"
```

---

## Task 4: Simplify the transition matrix (Planning becomes Active)

**Files:**
- Modify: `crates/reeve-card/src/transitions.rs`

Replace the five-arm matrix with a two-set partition: Active = {Planning, Implementing, Testing, Review}; Inactive = {Backlog, Done}. Inactive→Active triggers `ensure_running`; Active→Inactive triggers `ensure_stopped`; Active→Active and Inactive→Inactive are no-ops.

- [ ] **Step 1: Update the existing `backlog_to_planning_is_noop` test to expect ensure_running**

In `crates/reeve-card/src/transitions.rs::tests`, rename the test and flip the assertion. Replace:

```rust
#[tokio::test]
async fn backlog_to_planning_is_noop() {
    let fx = run_one(Column::Backlog, Column::Planning).await;
    assert!(fx.running.lock().await.is_empty());
    assert!(fx.stopped.lock().await.is_empty());
}
```

with:

```rust
#[tokio::test]
async fn backlog_to_planning_spawns() {
    let fx = run_one(Column::Backlog, Column::Planning).await;
    assert_eq!(*fx.running.lock().await, vec!["c1".to_string()]);
    assert!(fx.stopped.lock().await.is_empty());
}
```

- [ ] **Step 2: Add new tests covering the four-arm matrix**

In the same `tests` module, append:

```rust
#[tokio::test]
async fn planning_to_implementing_is_noop() {
    // Active → Active: container persists, no respawn.
    let fx = run_one(Column::Planning, Column::Implementing).await;
    assert!(fx.running.lock().await.is_empty());
    assert!(fx.stopped.lock().await.is_empty());
}

#[tokio::test]
async fn implementing_to_planning_is_noop() {
    // Backwards Active → Active also persists the container.
    let fx = run_one(Column::Implementing, Column::Planning).await;
    assert!(fx.running.lock().await.is_empty());
    assert!(fx.stopped.lock().await.is_empty());
}

#[tokio::test]
async fn review_to_done_tears_down() {
    let fx = run_one(Column::Review, Column::Done).await;
    assert_eq!(*fx.stopped.lock().await, vec!["c1".to_string()]);
}

#[tokio::test]
async fn planning_to_backlog_tears_down() {
    let fx = run_one(Column::Planning, Column::Backlog).await;
    assert_eq!(*fx.stopped.lock().await, vec!["c1".to_string()]);
}

#[tokio::test]
async fn backlog_to_done_is_noop() {
    // Inactive → Inactive: nothing to start, nothing to stop.
    let fx = run_one(Column::Backlog, Column::Done).await;
    assert!(fx.running.lock().await.is_empty());
    assert!(fx.stopped.lock().await.is_empty());
}

#[tokio::test]
async fn done_to_planning_spawns() {
    // Inactive → Active reprovisions a fresh workspace.
    let fx = run_one(Column::Done, Column::Planning).await;
    assert_eq!(*fx.running.lock().await, vec!["c1".to_string()]);
}
```

- [ ] **Step 3: Run the test set to verify failures**

Run: `cargo test -p reeve-card transitions::tests -- --nocapture`
Expected: The renamed test and the new ones FAIL (matrix still uses old arms); existing passing tests still PASS where they happen to match.

- [ ] **Step 4: Rewrite the matrix**

In `crates/reeve-card/src/transitions.rs`, replace the `on_column_change` body. The new function:

```rust
/// The kanban transition matrix. Decides which `WorkspaceLifecycle`
/// method (if any) to invoke when a card's column changes.
///
/// Active columns hold a container; Inactive columns do not:
///   Active   = { Planning, Implementing, Testing, Review }
///   Inactive = { Backlog, Done }
///
/// Inactive → Active provisions; Active → Inactive tears down; both
/// Active → Active and Inactive → Inactive are no-ops.
pub async fn on_column_change(
    old: Column,
    new: Column,
    card: &Card,
    workspace: &dyn WorkspaceLifecycle,
) -> Result<()> {
    if old == new {
        return Ok(());
    }
    let old_active = is_active(old);
    let new_active = is_active(new);
    match (old_active, new_active) {
        (false, true) => workspace.ensure_running(card).await,
        (true, false) => workspace.ensure_stopped(card.id()).await,
        // Active→Active: container persists; conversation persists.
        // Inactive→Inactive: nothing to start, nothing to stop.
        _ => Ok(()),
    }
}

fn is_active(c: Column) -> bool {
    matches!(
        c,
        Column::Planning | Column::Implementing | Column::Testing | Column::Review,
    )
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p reeve-card transitions::tests -- --nocapture`
Expected: All transition tests PASS (renamed, new, and existing).

- [ ] **Step 6: Run the workspace's full test suite to catch downstream regressions**

Run: `cargo test --workspace --lib && cargo test -p reeved`
Expected: All green. (Any dispatch-layer tests that exercised `* → Planning` as a no-op will need to be revisited; the existing tests assert column updates without checking lifecycle side-effects directly, so they should still pass.)

- [ ] **Step 7: Commit**

```bash
git add crates/reeve-card/src/transitions.rs
git commit -m "$(cat <<'EOF'
refactor(card): partition transition matrix into Active/Inactive sets

Active = {Planning, Implementing, Testing, Review}; Inactive = {Backlog,
Done}. Inactive→Active runs ensure_running; Active→Inactive runs
ensure_stopped; Active↔Active and Inactive↔Inactive are no-ops.

Visible change: Planning is now an Active column. Moving Backlog/Done →
Planning provisions a workspace + container; moving Active → Planning
keeps the container warm (was previously a defensive ensure_stopped).

Co-Authored-By: Claude
EOF
)"
```

---

## Task 5: Add `SessionInitSelfParams` + `SessionInitSelfResult` to the protocol

**Files:**
- Modify: `crates/reeve-protocol/src/lib.rs`

The wire shape for the new RPC. Params are empty (identity from the per-card socket); result carries the rendered prompt.

- [ ] **Step 1: Write a failing serde round-trip test**

In `crates/reeve-protocol/src/lib.rs::tests` (locate the existing test module that has `MvSelfParams` / `StatusSelfResult` round-trips around line 686+), append:

```rust
#[test]
fn session_init_self_round_trip() {
    let p = SessionInitSelfParams {};
    let json = serde_json::to_string(&p).unwrap();
    let _back: SessionInitSelfParams = serde_json::from_str(&json).unwrap();

    let r = SessionInitSelfResult {
        prompt: "hello from reeve".into(),
    };
    let json = serde_json::to_string(&r).unwrap();
    let back: SessionInitSelfResult = serde_json::from_str(&json).unwrap();
    assert_eq!(back.prompt, "hello from reeve");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p reeve-protocol session_init_self_round_trip`
Expected: FAIL — types not defined.

- [ ] **Step 3: Define the DTOs**

In `crates/reeve-protocol/src/lib.rs`, after the `StatusSelfResult` definition (around line 219), add:

```rust
/// Parameters for the `session-init-self` method. Identity comes from
/// the per-card socket; no fields needed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInitSelfParams {}

/// Result for the `session-init-self` method. The rendered prompt
/// text that the SessionStart hook injects into the agent's
/// initial context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInitSelfResult {
    pub prompt: String,
}
```

- [ ] **Step 4: Run to verify passing**

Run: `cargo test -p reeve-protocol session_init_self_round_trip`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/reeve-protocol/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(protocol): add SessionInitSelfParams + SessionInitSelfResult

Wire shape for the new per-card session-init-self RPC. Params are
empty (identity from socket); result carries the rendered prompt
that the SessionStart hook injects.

Co-Authored-By: Claude
EOF
)"
```

---

## Task 6: Build the prompt template + render function

**Files:**
- Create: `crates/reeve-card/src/session_init.rs`
- Modify: `crates/reeve-card/src/lib.rs`

Pure rendering — takes a card and resolved `CardPaths`, returns the prompt string. No I/O.

- [ ] **Step 1: Create the new module with a failing test**

Create `crates/reeve-card/src/session_init.rs`:

```rust
//! SessionStart prompt template + render function for the planning agent.
//! Pure — no I/O, no infra deps. The handler in `handler.rs` calls
//! `render` with a card + resolved CardPaths and ships the result over
//! the wire to be injected into claude's session.

use crate::domain::Card;
use crate::transitions::CardPaths;

/// Render the SessionStart prompt for a card. The prompt is plugin-
/// agnostic: it names workflow phases (brainstorm, plan, execute,
/// test, review) and the household's installed plugin provides the
/// concrete skill bindings.
pub fn render(card: &Card, paths: &CardPaths) -> String {
    format!(
        "You are reeve card {id} in the {household} household.\n\
         Brief: {brief}\n\
         \n\
         Take this card through these phases:\n\
         \x20\x201. Brainstorm the design  \u{2192}  {spec_path}/{id}-design.md\n\
         \x20\x202. Write the implementation plan  \u{2192}  {plan_path}/{id}.md\n\
         \x20\x203. Execute the plan\n\
         \x20\x204. Run integration tests\n\
         \x20\x205. Submit for human review\n\
         \n\
         Your household's installed plugin provides the skills for each\n\
         phase \u{2014} invoke them as they apply.\n\
         \n\
         Signal phase changes to the kanban board as you progress:\n\
         \x20\x20reeve mv-self planning      (during brainstorming + planning)\n\
         \x20\x20reeve mv-self implementing  (when executing the plan)\n\
         \x20\x20reeve mv-self testing       (when running integration tests)\n\
         \x20\x20reeve mv-self review        (when ready for human review)\n\
         \n\
         The user may move your card to a different column to redirect you;\n\
         treat that as a phase-change instruction from them.",
        id = card.id(),
        household = card.household(),
        brief = card.brief(),
        spec_path = paths.spec_path,
        plan_path = paths.plan_path,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use reeve_protocol::Column;

    fn card(id: &str, household: &str, brief: &str) -> Card {
        Card::from_persisted(
            id.into(), household.into(), brief.into(),
            Column::Planning, 0, 0, false, None,
        ).unwrap()
    }

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
        assert!(out.contains("reeve mv-self testing"));
        assert!(out.contains("reeve mv-self review"));
    }

    #[test]
    fn renders_with_custom_paths() {
        let c = card("xyz", "ramudden", "Pipeline refactor");
        let paths = CardPaths::from_optional(
            Some("docs/superpowers/specs"),
            Some("docs/superpowers/plans"),
        );
        let out = render(&c, &paths);
        assert!(out.contains("docs/superpowers/specs/xyz-design.md"));
        assert!(out.contains("docs/superpowers/plans/xyz.md"));
        assert!(out.contains("in the ramudden household"));
    }
}
```

- [ ] **Step 2: Wire the module into lib.rs**

In `crates/reeve-card/src/lib.rs`, add the module declaration and re-export:

```rust
//! Card aggregate: kanban metadata + column-transition policy.

mod domain;
mod handler;
mod repo;
mod session_init;
mod transitions;

pub use domain::{Card, CardError};
pub use handler::{
    handle_delete, handle_edit, handle_ls, handle_mv, handle_mv_self, handle_new,
    handle_status_self,
};
pub use repo::{CardRepo, init_schema, make_card_repo};
pub use session_init::render as render_session_init;
pub use transitions::{CardPaths, HouseholdLookup, WorkspaceLifecycle, on_column_change};
```

- [ ] **Step 3: Run the new tests**

Run: `cargo test -p reeve-card session_init::tests -- --nocapture`
Expected: PASS.

- [ ] **Step 4: Run reeve-card's full test suite**

Run: `cargo test -p reeve-card --lib`
Expected: All green.

- [ ] **Step 5: Commit**

```bash
git add crates/reeve-card/src/session_init.rs crates/reeve-card/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(card): add session_init prompt template + render function

Pure rendering; takes a Card + resolved CardPaths, returns the
plugin-agnostic SessionStart prompt the agent receives. Names
workflow phases (brainstorm/plan/execute/test/review) rather than
plugin-specific skill IDs.

Co-Authored-By: Claude
EOF
)"
```

---

## Task 7: Add `handle_session_init_self` in `reeve-card`

**Files:**
- Modify: `crates/reeve-card/src/handler.rs`
- Modify: `crates/reeve-card/src/lib.rs`

The handler is read-only: read card, resolve paths via `HouseholdLookup::card_paths`, render via `session_init::render`, return.

- [ ] **Step 1: Write a failing test**

In `crates/reeve-card/src/handler.rs::tests`, append:

```rust
#[tokio::test]
async fn session_init_self_renders_prompt_from_card_and_default_paths() {
    let repo = fresh_repo();
    let card = Card::new("witan".into(), "Add OAuth login".into(), false).unwrap();
    let card_id = card.id().to_string();
    repo.insert(&card).await.unwrap();

    let value = handle_session_init_self(
        &*repo,
        &AlwaysExistsHouseholds,
        card_id.clone(),
    )
    .await
    .unwrap();
    let result: reeve_protocol::SessionInitSelfResult =
        serde_json::from_value(value).unwrap();
    assert!(result.prompt.contains(&format!("reeve card {card_id}")));
    assert!(result.prompt.contains("in the witan household"));
    assert!(result.prompt.contains("Brief: Add OAuth login"));
    assert!(result.prompt.contains(&format!("docs/agent/specs/{card_id}-design.md")));
    assert!(result.prompt.contains(&format!("docs/agent/plans/{card_id}.md")));
}

#[tokio::test]
async fn session_init_self_returns_not_found_for_unknown_card() {
    let repo = fresh_repo();
    let err = handle_session_init_self(
        &*repo,
        &AlwaysExistsHouseholds,
        "no-such-card".into(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, RpcError::NOT_FOUND);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p reeve-card session_init_self -- --nocapture`
Expected: FAIL (handle_session_init_self not defined).

- [ ] **Step 3: Implement the handler**

In `crates/reeve-card/src/handler.rs`, after `handle_status_self` and before the `#[cfg(test)] mod tests` block, add:

```rust
pub async fn handle_session_init_self(
    cards: &dyn CardRepo,
    households: &dyn HouseholdLookup,
    card_id: String,
) -> std::result::Result<Value, RpcError> {
    let card = cards
        .get(&card_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found(&card_id))?;
    let paths = households
        .card_paths(card.household())
        .await
        .map_err(internal)?;
    let prompt = crate::session_init::render(&card, &paths);
    serde_json::to_value(reeve_protocol::SessionInitSelfResult { prompt }).map_err(internal)
}
```

- [ ] **Step 4: Re-export the handler from `lib.rs`**

In `crates/reeve-card/src/lib.rs`, update the handler re-export line:

```rust
pub use handler::{
    handle_delete, handle_edit, handle_ls, handle_mv, handle_mv_self, handle_new,
    handle_session_init_self, handle_status_self,
};
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p reeve-card session_init_self -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/reeve-card/src/handler.rs crates/reeve-card/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(card): add handle_session_init_self RPC handler

Reads the card, resolves the household's card paths via the
HouseholdLookup port, renders the prompt template, returns a
SessionInitSelfResult. Read-only; no card-state mutation.

Co-Authored-By: Claude
EOF
)"
```

---

## Task 8: Wire the `session-init-self` dispatch arm in reeved

**Files:**
- Modify: `crates/reeved/src/dispatch.rs`

Self-scoped, read-only — same shape as `status-self`. No per-card lock needed (no state mutation).

- [ ] **Step 1: Write a failing test**

In `crates/reeved/src/dispatch.rs`, locate the existing self-scoped tests (around `status_self_on_card_socket_returns_card_no_container`, ~line 530). Append after that test:

```rust
#[tokio::test]
async fn session_init_self_on_host_socket_returns_forbidden() {
    let env = TestEnv::new().await;
    let req = reeve_protocol::RpcRequest {
        method: "session-init-self".into(),
        params: serde_json::json!({}),
    };
    let caller = CallerContext { card_id: None };
    let err = dispatch(req, env.state.clone(), caller).await.unwrap_err();
    assert_eq!(err.code, RpcError::FORBIDDEN);
}

#[tokio::test]
async fn session_init_self_on_card_socket_returns_prompt() {
    let env = TestEnv::new().await;
    // Create a household + card so card_paths can resolve.
    env.register_household("witan").await;
    let new_resp: reeve_protocol::NewResult = serde_json::from_value(
        dispatch(
            reeve_protocol::RpcRequest {
                method: "new".into(),
                params: serde_json::json!({
                    "household": "witan",
                    "brief": "Add OAuth",
                    "needs_heavy": false,
                }),
            },
            env.state.clone(),
            CallerContext { card_id: None },
        )
        .await
        .unwrap(),
    )
    .unwrap();
    let card_id = new_resp.card.id;

    let req = reeve_protocol::RpcRequest {
        method: "session-init-self".into(),
        params: serde_json::json!({}),
    };
    let caller = CallerContext { card_id: Some(card_id.clone()) };
    let value = dispatch(req, env.state.clone(), caller).await.unwrap();
    let result: reeve_protocol::SessionInitSelfResult =
        serde_json::from_value(value).unwrap();
    assert!(result.prompt.contains(&card_id));
    assert!(result.prompt.contains("in the witan household"));
}
```

Note: this assumes `TestEnv` exposes a `register_household` helper. If it doesn't, inspect the existing tests' setup for the equivalent pattern (likely a direct call to `state.household_repo.upsert(...)` with a temp dir). Match the existing convention exactly.

- [ ] **Step 2: Run to verify failures**

Run: `cargo test -p reeved session_init_self -- --nocapture`
Expected: FAIL — no dispatch arm for `session-init-self`.

- [ ] **Step 3: Add the dispatch arm**

In `crates/reeved/src/dispatch.rs`, locate the `"status-self"` arm (around line 94). Add a new arm immediately after it:

```rust
"session-init-self" => {
    let card_id = require_card(&caller)?;
    let cards = make_card_repo(state.db.clone());
    let households = state.household_lookup.clone();
    reeve_card::handle_session_init_self(&*cards, &*households, card_id).await
}
```

- [ ] **Step 4: Run the dispatch tests**

Run: `cargo test -p reeved session_init_self -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Run the full workspace tests**

Run: `cargo test --workspace --lib && cargo test -p reeved`
Expected: All green.

- [ ] **Step 6: Commit**

```bash
git add crates/reeved/src/dispatch.rs
git commit -m "$(cat <<'EOF'
feat(reeved): dispatch arm for session-init-self

Routes session-init-self to reeve_card::handle_session_init_self.
Self-scoped (per-card socket only); read-only, no card-lock needed.

Co-Authored-By: Claude
EOF
)"
```

---

## Task 9: Add `reeve session-init-self` CLI subcommand

**Files:**
- Modify: `crates/reeve/src/cli.rs`

The hook inside the container calls `reeve session-init-self`; its stdout is injected by claude. The CLI must print just the prompt (not JSON-wrapped).

- [ ] **Step 1: Add the subcommand variant**

In `crates/reeve/src/cli.rs`, find the `Command` enum (around line 30). Add a new variant after `StatusSelf`:

```rust
    /// (in-container only) Print the SessionStart prompt for the calling
    /// card. Consumed by claude's SessionStart hook to seed the agent.
    SessionInitSelf,
```

Then in the import line at the top, add `SessionInitSelfResult` to the protocol re-export:

```rust
use reeve_protocol::{
    AttachResult, BranchPushStatus, Card, Column, HouseholdAddParams, HouseholdDetail,
    HouseholdNewParams, HouseholdSource, HouseholdSummary, LsResult, MvResult, MvSelfResult,
    NewResult, PushResult, RepoOnDisk, SessionInitSelfResult, StatusSelfResult, paths,
};
```

- [ ] **Step 2: Add the match arm in `run()` dispatch**

In the `run()` function (around line 120), add the new arm:

```rust
        Command::SessionInitSelf => session_init_self().await,
```

- [ ] **Step 3: Implement the handler function**

After `status_self()` (around line 276), add:

```rust
async fn session_init_self() -> Result<()> {
    let socket = paths::detect_socket();
    let result = client::call(&socket, "session-init-self", serde_json::json!({}))
        .await
        .context("calling session-init-self")?;
    match result {
        Ok(value) => {
            let r: SessionInitSelfResult = serde_json::from_value(value)?;
            print!("{}", r.prompt);
            Ok(())
        }
        Err(e) => anyhow::bail!("daemon error: {e}"),
    }
}
```

Note: `print!`, not `println!` — we don't want to inject a trailing newline that would render oddly in the agent's injected context. The prompt template itself controls its trailing layout.

- [ ] **Step 4: Build the reeve binary to verify compilation**

Run: `cargo build -p reeve`
Expected: PASS.

- [ ] **Step 5: Verify the subcommand registers in clap**

Run: `cargo run -p reeve -- session-init-self --help`
Expected: Output includes the new subcommand's description.

- [ ] **Step 6: Commit**

```bash
git add crates/reeve/src/cli.rs
git commit -m "$(cat <<'EOF'
feat(reeve): add session-init-self CLI subcommand

Container-side claude SessionStart hook invokes this; the CLI prints
the prompt string verbatim to stdout (no trailing newline; no JSON
framing) for direct injection into the agent's session context.

Co-Authored-By: Claude
EOF
)"
```

---

## Task 10: Write `.claude/settings.local.json` at provision time

**Files:**
- Modify: `crates/reeve-workspace/src/lifecycle.rs`

The provisioning step that creates `<workspace>/.claude/settings.local.json` with the SessionStart hook stanza. Idempotent — re-running provision produces byte-identical output. Runs after `clone_all_for_card` (which creates the workspace directory) and before `exec_in_tmux` (which launches claude).

- [ ] **Step 1: Write a failing test for the helper function**

In `crates/reeve-workspace/src/lifecycle.rs::tests` (if a test module exists; if not, add one at the bottom of the file), append:

```rust
#[test]
fn write_settings_local_json_creates_expected_hook_stanza() {
    use tempfile::tempdir;
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    super::write_settings_local_json(workspace).unwrap();

    let written = std::fs::read_to_string(
        workspace.join(".claude").join("settings.local.json"),
    ).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&written).unwrap();
    let cmd = parsed
        .pointer("/hooks/SessionStart/0/hooks/0/command")
        .and_then(|v| v.as_str())
        .expect("hook command missing");
    assert_eq!(cmd, "reeve session-init-self");
    let matcher = parsed
        .pointer("/hooks/SessionStart/0/matcher")
        .and_then(|v| v.as_str())
        .expect("matcher missing");
    assert_eq!(matcher, "*");
}

#[test]
fn write_settings_local_json_is_idempotent() {
    use tempfile::tempdir;
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    super::write_settings_local_json(workspace).unwrap();
    let first = std::fs::read(
        workspace.join(".claude").join("settings.local.json"),
    ).unwrap();
    super::write_settings_local_json(workspace).unwrap();
    let second = std::fs::read(
        workspace.join(".claude").join("settings.local.json"),
    ).unwrap();
    assert_eq!(first, second, "re-running provision must produce byte-identical output");
}
```

(Verify `tempfile` is already a dev-dependency of `reeve-workspace`; if not, add it: `cargo add --dev -p reeve-workspace tempfile`.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p reeve-workspace lifecycle::tests::write_settings_local_json -- --nocapture`
Expected: FAIL — `write_settings_local_json` not defined.

- [ ] **Step 3: Implement the helper function**

In `crates/reeve-workspace/src/lifecycle.rs`, add a `pub(crate) fn` (or `fn` if not needed outside) near `now_secs`:

```rust
/// Write the SessionStart hook stanza to `<workspace>/.claude/settings.local.json`.
///
/// Idempotent — repeated calls produce byte-identical output. Overwrites any
/// existing file at the same path; this is intentional, the file is gitignored
/// by claude convention and the workspace is Reeve-controlled.
fn write_settings_local_json(workspace_path: &std::path::Path) -> Result<()> {
    let dir = workspace_path.join(".claude");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("create {}", dir.display()))?;
    // Pretty-print so the file is readable if a user inspects it; the order
    // is fixed so byte-identical re-writes are guaranteed.
    let body = r#"{
  "hooks": {
    "SessionStart": [
      {
        "matcher": "*",
        "hooks": [
          { "type": "command", "command": "reeve session-init-self" }
        ]
      }
    ]
  }
}
"#;
    let path = dir.join("settings.local.json");
    std::fs::write(&path, body)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}
```

- [ ] **Step 4: Run the helper tests to verify they pass**

Run: `cargo test -p reeve-workspace lifecycle::tests::write_settings_local_json -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Call the helper from `ensure_running`**

In `crates/reeve-workspace/src/lifecycle.rs::ensure_running`, after the `clone_all_for_card` block and before the `devcontainer_path` check (around line 122), add:

```rust
        // Write the SessionStart hook config into the workspace clone. claude
        // reads .claude/settings.local.json automatically; the hook calls
        // `reeve session-init-self` from inside the container.
        write_settings_local_json(&workspace_path)
            .with_context(|| format!("write settings.local.json for {card_id}"))?;
```

- [ ] **Step 6: Run reeve-workspace's full test suite to confirm no regressions**

Run: `cargo test -p reeve-workspace`
Expected: All green.

- [ ] **Step 7: Run the full workspace tests**

Run: `cargo test --workspace --lib && cargo test -p reeved`
Expected: All green.

- [ ] **Step 8: Commit**

```bash
git add crates/reeve-workspace/src/lifecycle.rs
git commit -m "$(cat <<'EOF'
feat(workspace): write .claude/settings.local.json at provision

ensure_running writes the SessionStart hook stanza to the workspace
clone after clone_all_for_card. Idempotent; re-running provision
produces byte-identical output. Hook calls `reeve session-init-self`
from inside the container to inject the planning prompt into claude.

Co-Authored-By: Claude
EOF
)"
```

---

## Task 11: Verify with `cargo fmt`, `clippy`, and the context-isolation lint

**Files:**
- (none — verification only)

- [ ] **Step 1: Format check**

Run: `cargo fmt --check`
Expected: Clean. If not, run `cargo fmt` and commit any changes:

```bash
cargo fmt
git add -A
git commit -m "chore: cargo fmt"
```

- [ ] **Step 2: Clippy with -D warnings**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: Clean. Fix any new lints in the touched files inline; commit with `chore(clippy): ...`.

- [ ] **Step 3: Context isolation lint**

Run: `scripts/check-context-isolation.sh`
Expected: Clean. The new code adds `pub use session_init::render as render_session_init` (re-export from the crate facade — fine) and `card.session_init::render` calls only from within `reeve-card/src/handler.rs` (same crate — fine). No cross-context internal imports were introduced.

- [ ] **Step 4: Full workspace + daemon tests one final pass**

Run: `cargo test --workspace --lib && cargo test -p reeved`
Expected: All green.

- [ ] **Step 5: Lore validators (if changes touched lore)**

Run: `make validate`
Expected: Clean. (This task didn't edit lore yet — see Task 12 for the lore update.)

---

## Task 12: Update lore for the new transition matrix

**Files:**
- Modify: `lore/knowledge/domain/card.md`

The `domain/card.md` node documents the column-transition policy. Update it to reflect the new four-arm matrix and Planning's active status. Keep the file's existing structure and frontmatter.

- [ ] **Step 1: Update the "Column-transition policy" section**

In `lore/knowledge/domain/card.md`, locate the section starting with `## Column-transition policy` (around line 45). Replace the table that begins with `| Transition | Effect |` with:

```markdown
| Transition | Effect |
|---|---|
| same column | no-op |
| Inactive → Active | `ensure_running(card)` |
| Active → Active | no-op (container + agent persist; conversation continues) |
| Active → Inactive | `ensure_stopped(card_id)` |
| Inactive → Inactive | no-op |

Where Active = {Planning, Implementing, Testing, Review} and Inactive = {Backlog, Done}. The container exists for any Active column; transitioning between active columns is purely a metadata update.
```

- [ ] **Step 2: Add a short paragraph on agent-driven transitions**

Immediately after the matrix table, before the existing "Tests in ... pin each arm" sentence, add:

```markdown
**Agent-driven transitions.** The agent inside the card's container calls `reeve mv-self <column>` at natural phase boundaries (brainstorming + planning → `planning`; executing → `implementing`; tests → `testing`; ready for review → `review`). User-issued `mv` remains available as a corrective override. Both go through the same per-card lock and the same transition matrix; the only difference is the caller-context origin.
```

- [ ] **Step 3: Add `session-init-self` to the RPC surface table**

In the same file, locate the "RPC surface" section (around line 68) and the table that starts with `| Method | Handler | Description |`. Add a new row after `status-self`:

```markdown
| `session-init-self` | `handle_session_init_self` | From the per-card socket: render the SessionStart prompt for the calling card (card identity + brief + household's spec/plan paths). Read-only; consumed by claude's SessionStart hook to inject planning instructions. |
```

Update the "RPC surface (7 methods)" header to "RPC surface (8 methods)".

- [ ] **Step 4: Run lore validators**

Run: `make validate`
Expected: Clean.

- [ ] **Step 5: Commit**

```bash
git add lore/knowledge/domain/card.md
git commit -m "$(cat <<'EOF'
docs(lore): card.md reflects active/inactive transition matrix

Documents the simplified four-arm matrix, Planning's active status,
agent-driven transitions via mv-self, and the new session-init-self
RPC. Pinned by transitions::tests + handler::tests.

Co-Authored-By: Claude
EOF
)"
```

---

## Self-Review (run by the implementing engineer)

After implementing all tasks, look at the spec at `docs/superpowers/specs/2026-05-26-planning-column-design.md` and check the plan covered each piece:

- [ ] Architecture (active/inactive columns) — Task 4
- [ ] Transition matrix simplification — Task 4
- [ ] Agent-driven transitions via `mv-self` — covered by existing `mv-self` infrastructure (Tasks reference it; no code change needed)
- [ ] SessionStart hook in `<workspace>/.claude/settings.local.json` — Task 10
- [ ] `session-init-self` RPC — Tasks 5, 7, 8, 9
- [ ] Plugin-agnostic prompt template — Task 6
- [ ] Manifest `spec_path`/`plan_path` with defaults — Task 1
- [ ] `CardPaths` defaults applied at the port — Task 2
- [ ] `card_paths` implementation reading manifest — Task 3
- [ ] CLI hook target (`reeve session-init-self` prints raw prompt) — Task 9
- [ ] Idempotent settings.local.json write — Task 10
- [ ] Lore updates — Task 12
- [ ] Format / clippy / context-isolation lint clean — Task 11

If any spec requirement isn't covered above, add a task.

---

**Plan complete.** Saved to `docs/superpowers/plans/2026-05-26-planning-column.md`. Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** — Execute tasks in this session using `executing-plans`, batch execution with checkpoints.

Which approach?
