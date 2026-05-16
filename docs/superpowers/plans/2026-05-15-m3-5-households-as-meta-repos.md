# Reeve M3.5 — Households as meta-repos: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the per-file TOML household profile (M3) with a meta-repo-as-household model: a user-cloned (or Reeve-scaffolded) git repo at the head of a workspace that carries a JSON manifest, `.devcontainer/`, and sibling code repos as gitignored subdirs.

**Architecture:** A new `crates/reeved/src/manifest.rs` module owns the schema and validation; `household.rs` shrinks to a thin sqlite-backed registry mapping `name → meta_repo_path`; `clones.rs` rewrites `clone_all_repos` to also clone the meta-repo (as "repo zero") and iterate the manifest's repos; five new RPC handlers expose `household add/new/ls/rm/show`; the CLI grows a `household` subcommand tree. Spec: `docs/superpowers/specs/2026-05-15-m3-5-households-as-meta-repos-design.md`.

**Tech Stack:** Rust 2024, tokio async, anyhow error handling, serde JSON, `async_trait`, `tempfile` for tests, `rusqlite` for the households table.

---

## File structure

| Path | Action | Responsibility |
|---|---|---|
| `crates/reeved/src/manifest.rs` | **Create** | `Manifest` / `RepoEntry` structs, `load_manifest`, 5 validation rules, all manifest tests |
| `crates/reeved/src/household.rs` | **Rewrite** | `Household` struct (`name`, `meta_repo_path`); sqlite-backed `load_households`/`insert_household`/`remove_household`. Old `HouseholdProfile` and `load_all` removed. |
| `crates/reeved/src/db.rs` | **Modify** | Add `households` table to schema migrations |
| `crates/reeved/src/clones.rs` | **Modify** | Rewrite `clone_all_repos` signature to take meta-repo + workspace + manifest; clone meta-repo first, then iterate manifest entries; emit a structured error for missing sibling subdirs |
| `crates/reeved/src/effects.rs` | **Modify** | `ensure_running` reads devcontainer path from `<workspace>/.devcontainer/devcontainer.json` (convention) instead of the removed `household.devcontainer` field |
| `crates/reeved/src/handlers.rs` | **Modify** | Add `handle_household_add`, `handle_household_new`, `handle_household_ls`, `handle_household_rm`, `handle_household_show`; wire into the dispatch table |
| `crates/reeved/src/main.rs` | **Modify** | On startup, if `~/.config/reeve/households/*.toml` exists, log a `warn` pointing the user at `reeve household add` (no auto-import) |
| `crates/reeve-protocol/src/lib.rs` | **Modify** | Add `HouseholdAddParams`, `HouseholdSource`, `HouseholdNewParams`, `HouseholdSummary`, `HouseholdDetail`, `RepoDetail`, `CardReference` |
| `crates/reeve/src/cli.rs` | **Modify** | New `household` clap subcommand tree: `add`, `new`, `ls`, `rm`, `show` |
| `crates/reeve/src/client.rs` | **Modify** | Typed wrappers for the 5 new RPCs |
| `tests/e2e.rs` | **Modify** | Three new `REEVE_TESTS_STRICT`-respecting docker-gated scenarios |

---

## Task 1: Manifest module — schema and parsing

**Files:**
- Create: `crates/reeved/src/manifest.rs`
- Modify: `crates/reeved/src/lib.rs` (or equivalent — add `pub mod manifest;`)

- [ ] **Step 1: Create the manifest module skeleton**

```rust
// crates/reeved/src/manifest.rs
//! Workspace manifest loader. A `household.json` at the meta-repo root declares the
//! workspace pointer, an optional knowledge-base pointer, and a unified list of
//! every repo (workspace meta-repo included). The schema lives here; consumers
//! (clones.rs, handlers.rs) read parsed `Manifest` values, not raw JSON.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

const MANIFEST_FILENAME: &str = "household.json";

/// Reserved directory names that cannot be used as a repo entry's `name`.
/// These collide with meta-repo internals or git itself.
const RESERVED_NAMES: &[&str] = &[".git", ".devcontainer"];

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Manifest {
    /// Required. Name of the `repos[]` entry that IS the meta-repo itself.
    /// Its on-disk location is the meta-repo root.
    pub workspace: String,

    /// Optional. Name of the `repos[]` entry that holds the knowledge base.
    /// May equal `workspace` for inline-KB single-repo setups.
    #[serde(default)]
    pub knowledge_base: Option<String>,

    /// Every repo participating in the workspace, the meta-repo included.
    /// Non-empty (the workspace entry must exist).
    pub repos: Vec<RepoEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RepoEntry {
    /// Manifest identifier; also the on-disk directory name for non-workspace entries.
    pub name: String,

    /// Canonical git URL. Optional; surfaced in `household show` and consumed by
    /// user-owned bootstrap scripts. NOT used by Reeve at card-spawn time.
    #[serde(default)]
    pub url: Option<String>,

    /// Free-form categorisation (`backend`, `core`, `docs`, ...). Neither the KB
    /// marker nor the workspace marker is a tag — both live in top-level fields.
    #[serde(default)]
    pub tags: Vec<String>,

    /// Human-readable; surfaced in `household show`.
    #[serde(default)]
    pub description: Option<String>,
}

impl Manifest {
    /// The entry pointed to by `workspace`. Validated at load, so unwrap is safe here.
    pub fn workspace_entry(&self) -> &RepoEntry {
        self.repos
            .iter()
            .find(|r| r.name == self.workspace)
            .expect("workspace_entry: validation ensures workspace matches a repos[].name")
    }

    /// The entry pointed to by `knowledge_base`, if set.
    pub fn knowledge_base_entry(&self) -> Option<&RepoEntry> {
        self.knowledge_base
            .as_ref()
            .and_then(|kb| self.repos.iter().find(|r| &r.name == kb))
    }

    /// Every non-workspace entry, in declaration order.
    pub fn siblings(&self) -> impl Iterator<Item = &RepoEntry> {
        self.repos.iter().filter(|r| r.name != self.workspace)
    }
}

/// Read and validate `<meta_repo_path>/household.json`.
pub fn load_manifest(meta_repo_path: &Path) -> Result<Manifest> {
    let path = meta_repo_path.join(MANIFEST_FILENAME);
    let body = std::fs::read_to_string(&path)
        .with_context(|| format!("read manifest at {}", path.display()))?;
    let manifest: Manifest = serde_json::from_str(&body)
        .with_context(|| format!("parse manifest at {}", path.display()))?;
    validate(&manifest)?;
    Ok(manifest)
}

fn validate(m: &Manifest) -> Result<()> {
    if m.repos.is_empty() {
        bail!("manifest: repos[] must be non-empty (must contain at least the workspace entry)");
    }
    if m.workspace.is_empty() {
        bail!("manifest: workspace must be non-empty");
    }
    // Names non-empty + unique + not reserved.
    let mut seen = std::collections::HashSet::new();
    for entry in &m.repos {
        if entry.name.is_empty() {
            bail!("manifest: repos[] entry has empty name");
        }
        if RESERVED_NAMES.contains(&entry.name.as_str()) {
            bail!("manifest: repos[].name '{}' is reserved", entry.name);
        }
        if !seen.insert(&entry.name) {
            bail!("manifest: duplicate repos[].name '{}'", entry.name);
        }
    }
    // Workspace points at exactly one repos[].name.
    if !m.repos.iter().any(|r| r.name == m.workspace) {
        bail!(
            "manifest: workspace = '{}' does not match any repos[].name",
            m.workspace
        );
    }
    // knowledge_base, if set, points at exactly one repos[].name.
    if let Some(kb) = &m.knowledge_base {
        if !m.repos.iter().any(|r| &r.name == kb) {
            bail!(
                "manifest: knowledge_base = '{}' does not match any repos[].name",
                kb
            );
        }
    }
    Ok(())
}
```

- [ ] **Step 2: Wire the module into the crate**

Open `crates/reeved/src/lib.rs` (or `main.rs`'s `mod` declarations — match whichever pattern the crate uses). Add:

```rust
pub mod manifest;
```

- [ ] **Step 3: Run `cargo check -p reeved` to confirm the module compiles**

Run: `cargo check -p reeved`
Expected: PASS, no warnings about unused `Manifest`/`RepoEntry` (those will be consumed in later tasks).

- [ ] **Step 4: Write the parse-success test**

Append to `crates/reeved/src/manifest.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_manifest(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join(MANIFEST_FILENAME);
        std::fs::write(&path, body).unwrap();
        dir.to_path_buf()
    }

    #[test]
    fn parses_full_manifest() {
        let dir = tempdir().unwrap();
        let root = write_manifest(
            dir.path(),
            r#"{
              "workspace": "myarchy-workspace",
              "knowledge_base": "lore",
              "repos": [
                { "name": "myarchy-workspace", "url": "git@github.com:Mindful-Stack/myarchy-workspace.git" },
                { "name": "backend",  "url": "git@github.com:Mindful-Stack/myarchy-backend.git", "tags": ["backend"] },
                { "name": "frontend", "url": "git@github.com:Mindful-Stack/myarchy-frontend.git", "tags": ["frontend"] },
                { "name": "lore",     "tags": ["docs"] }
              ]
            }"#,
        );
        let m = load_manifest(&root).unwrap();
        assert_eq!(m.workspace, "myarchy-workspace");
        assert_eq!(m.knowledge_base.as_deref(), Some("lore"));
        assert_eq!(m.repos.len(), 4);
        assert_eq!(m.workspace_entry().name, "myarchy-workspace");
        assert_eq!(m.workspace_entry().url.as_deref(), Some("git@github.com:Mindful-Stack/myarchy-workspace.git"));
        assert_eq!(m.knowledge_base_entry().unwrap().name, "lore");
        let sibs: Vec<&str> = m.siblings().map(|r| r.name.as_str()).collect();
        assert_eq!(sibs, vec!["backend", "frontend", "lore"]);
    }

    #[test]
    fn parses_single_repo_inline_kb_manifest() {
        // The witan-household starter shape.
        let dir = tempdir().unwrap();
        let root = write_manifest(
            dir.path(),
            r#"{
              "workspace": "my-ws",
              "knowledge_base": "lore",
              "repos": [
                { "name": "my-ws", "url": "git@github.com:you/my-ws.git" },
                { "name": "lore",  "description": "Inline KB" }
              ]
            }"#,
        );
        let m = load_manifest(&root).unwrap();
        assert_eq!(m.workspace, "my-ws");
        assert_eq!(m.workspace_entry().name, "my-ws");
        assert_eq!(m.knowledge_base_entry().unwrap().name, "lore");
        let sibs: Vec<&str> = m.siblings().map(|r| r.name.as_str()).collect();
        assert_eq!(sibs, vec!["lore"]);
    }

    #[test]
    fn allows_knowledge_base_to_equal_workspace() {
        // A single-repo workspace where the KB IS the workspace itself.
        let dir = tempdir().unwrap();
        let root = write_manifest(
            dir.path(),
            r#"{
              "workspace": "kb-only",
              "knowledge_base": "kb-only",
              "repos": [{ "name": "kb-only" }]
            }"#,
        );
        let m = load_manifest(&root).unwrap();
        assert_eq!(m.workspace, "kb-only");
        assert!(std::ptr::eq(
            m.workspace_entry() as *const _,
            m.knowledge_base_entry().unwrap() as *const _,
        ));
    }
}
```

- [ ] **Step 5: Run the test, verify it passes**

Run: `cargo test -p reeved manifest::tests::parses_full_manifest -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Write the validation-failure tests**

Append in the same `mod tests`:

```rust
    // Note: append these inside the same `mod tests` block as the parse tests above.

    #[test]
    fn requires_workspace_to_match_a_repos_entry() {
        let dir = tempdir().unwrap();
        let root = write_manifest(
            dir.path(),
            r#"{ "workspace": "ghost", "repos": [{ "name": "real" }] }"#,
        );
        let err = load_manifest(&root).unwrap_err();
        assert!(format!("{err:#}").contains("workspace = 'ghost' does not match"));
    }

    #[test]
    fn requires_knowledge_base_to_match_a_repos_entry() {
        let dir = tempdir().unwrap();
        let root = write_manifest(
            dir.path(),
            r#"{ "workspace": "ws", "knowledge_base": "ghost", "repos": [{ "name": "ws" }] }"#,
        );
        let err = load_manifest(&root).unwrap_err();
        assert!(format!("{err:#}").contains("knowledge_base = 'ghost' does not match"));
    }

    #[test]
    fn rejects_duplicate_repo_names() {
        let dir = tempdir().unwrap();
        let root = write_manifest(
            dir.path(),
            r#"{ "workspace": "a", "repos": [{ "name": "a" }, { "name": "a" }] }"#,
        );
        let err = load_manifest(&root).unwrap_err();
        assert!(format!("{err:#}").contains("duplicate repos[].name 'a'"));
    }

    #[test]
    fn rejects_reserved_repo_names() {
        let dir = tempdir().unwrap();
        let root = write_manifest(
            dir.path(),
            r#"{ "workspace": "ws", "repos": [{ "name": "ws" }, { "name": ".git" }] }"#,
        );
        let err = load_manifest(&root).unwrap_err();
        assert!(format!("{err:#}").contains("is reserved"));
    }

    #[test]
    fn rejects_empty_repos_array() {
        let dir = tempdir().unwrap();
        let root = write_manifest(
            dir.path(),
            r#"{ "workspace": "ws", "repos": [] }"#,
        );
        let err = load_manifest(&root).unwrap_err();
        assert!(format!("{err:#}").contains("repos[] must be non-empty"));
    }

    #[test]
    fn rejects_missing_workspace_field() {
        let dir = tempdir().unwrap();
        let root = write_manifest(
            dir.path(),
            r#"{ "repos": [{ "name": "a" }] }"#,
        );
        let err = load_manifest(&root).unwrap_err();
        // serde will fail before validate() because workspace is required.
        assert!(format!("{err:#}").contains("workspace"));
    }

    #[test]
    fn missing_manifest_file_errors() {
        let dir = tempdir().unwrap();
        let err = load_manifest(dir.path()).unwrap_err();
        assert!(format!("{err:#}").contains("read manifest at"));
    }

    #[test]
    fn malformed_json_errors() {
        let dir = tempdir().unwrap();
        let root = write_manifest(dir.path(), r#"{ this is not json"#);
        let err = load_manifest(&root).unwrap_err();
        assert!(format!("{err:#}").contains("parse manifest at"));
    }
```

- [ ] **Step 7: Run the full manifest test suite**

Run: `cargo test -p reeved manifest::tests`
Expected: all 9 tests PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/reeved/src/manifest.rs crates/reeved/src/lib.rs
git commit -m "feat(m3.5): workspace manifest schema and validation"
```

---

## Task 2: DB schema — `households` table

**Files:**
- Modify: `crates/reeved/src/db.rs` (add table to schema migration list + CRUD helpers)

- [ ] **Step 1: Add the table to the schema migration**

Open `crates/reeved/src/db.rs`. Locate the migration runner (the function that issues `CREATE TABLE` statements during daemon init). Add a new migration alongside the existing ones:

```rust
// In the migrations vector (or equivalent ordered list of CREATE TABLE statements):
"CREATE TABLE IF NOT EXISTS households (
    name           TEXT PRIMARY KEY,
    meta_repo_path TEXT NOT NULL,
    added_at       INTEGER NOT NULL
);",
```

If the existing migration mechanism uses a versioned approach (separate up/down migrations), add it as the next version increment matching the existing pattern.

- [ ] **Step 2: Add CRUD helpers**

Append to `db.rs`:

```rust
use crate::manifest::Manifest;

#[derive(Debug, Clone)]
pub struct HouseholdRow {
    pub name: String,
    pub meta_repo_path: PathBuf,
    pub added_at: i64,
}

pub fn insert_household(conn: &rusqlite::Connection, row: &HouseholdRow) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO households (name, meta_repo_path, added_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![row.name, row.meta_repo_path.to_string_lossy(), row.added_at],
    )?;
    Ok(())
}

pub fn remove_household(conn: &rusqlite::Connection, name: &str) -> anyhow::Result<bool> {
    let n = conn.execute("DELETE FROM households WHERE name = ?1", [name])?;
    Ok(n > 0)
}

pub fn list_households(conn: &rusqlite::Connection) -> anyhow::Result<Vec<HouseholdRow>> {
    let mut stmt = conn.prepare(
        "SELECT name, meta_repo_path, added_at FROM households ORDER BY name",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(HouseholdRow {
                name: r.get(0)?,
                meta_repo_path: PathBuf::from(r.get::<_, String>(1)?),
                added_at: r.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_household(
    conn: &rusqlite::Connection,
    name: &str,
) -> anyhow::Result<Option<HouseholdRow>> {
    let result = conn
        .query_row(
            "SELECT name, meta_repo_path, added_at FROM households WHERE name = ?1",
            [name],
            |r| {
                Ok(HouseholdRow {
                    name: r.get(0)?,
                    meta_repo_path: PathBuf::from(r.get::<_, String>(1)?),
                    added_at: r.get(2)?,
                })
            },
        )
        .optional()?;
    Ok(result)
}

/// Return all cards whose `household_name` column equals `name`. Used by
/// `handle_household_show` to surface the cards-referencing-this-household list.
pub fn cards_for_household(
    conn: &rusqlite::Connection,
    name: &str,
) -> anyhow::Result<Vec<(String, String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, brief, column FROM cards WHERE household_name = ?1 ORDER BY id",
    )?;
    let rows = stmt
        .query_map([name], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}
```

Adjust imports at the top of `db.rs` to include `std::path::PathBuf` and `rusqlite::OptionalExtension` (for `.optional()` on `query_row`).

- [ ] **Step 3: Write CRUD tests**

In `crates/reeved/src/db.rs`'s test module (or a new `#[cfg(test)] mod households_tests` block), add:

```rust
#[cfg(test)]
mod households_tests {
    use super::*;
    use tempfile::tempdir;

    fn open_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        // Run the daemon's schema-init on this connection. Use whatever
        // function the existing test code uses; this assumes `init_schema`.
        init_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn insert_and_get_household() {
        let conn = open_db();
        let dir = tempdir().unwrap();
        let row = HouseholdRow {
            name: "test".into(),
            meta_repo_path: dir.path().to_path_buf(),
            added_at: 1234,
        };
        insert_household(&conn, &row).unwrap();
        let got = get_household(&conn, "test").unwrap().unwrap();
        assert_eq!(got.name, "test");
        assert_eq!(got.meta_repo_path, dir.path());
    }

    #[test]
    fn duplicate_household_name_errors() {
        let conn = open_db();
        let row = HouseholdRow {
            name: "test".into(),
            meta_repo_path: PathBuf::from("/x"),
            added_at: 1,
        };
        insert_household(&conn, &row).unwrap();
        assert!(insert_household(&conn, &row).is_err());
    }

    #[test]
    fn remove_household_returns_true_if_existed() {
        let conn = open_db();
        let row = HouseholdRow {
            name: "test".into(),
            meta_repo_path: PathBuf::from("/x"),
            added_at: 1,
        };
        insert_household(&conn, &row).unwrap();
        assert!(remove_household(&conn, "test").unwrap());
        assert!(!remove_household(&conn, "test").unwrap()); // gone now
    }

    #[test]
    fn list_households_sorted() {
        let conn = open_db();
        for name in ["c", "a", "b"] {
            insert_household(
                &conn,
                &HouseholdRow {
                    name: name.into(),
                    meta_repo_path: PathBuf::from(format!("/{name}")),
                    added_at: 0,
                },
            )
            .unwrap();
        }
        let rows = list_households(&conn).unwrap();
        assert_eq!(rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["a", "b", "c"]);
    }
}
```

Note: if the existing `cards` table doesn't have a `household_name` column yet (M3 may have stored household differently — verify), defer the `cards_for_household` test to Task 9 where the migration adds it. Otherwise include it here.

- [ ] **Step 4: Run the test suite**

Run: `cargo test -p reeved db::households_tests`
Expected: 4 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/reeved/src/db.rs
git commit -m "feat(m3.5): households table — CRUD + schema migration"
```

---

## Task 3: Household struct rewrite

**Files:**
- Modify: `crates/reeved/src/household.rs` (full rewrite — remove `HouseholdProfile`/`load_all`)
- Modify: anywhere in the crate that imports `HouseholdProfile` (likely `effects.rs`, `handlers.rs`, `main.rs`)

- [ ] **Step 1: Replace the file contents**

Rewrite `crates/reeved/src/household.rs` end-to-end:

```rust
//! M3.5 household: a thin sqlite-backed registry mapping `name → meta_repo_path`.
//! The manifest is read from the meta-repo on demand (see `manifest.rs`); no
//! cached state lives in this module. M3's TOML-based `HouseholdProfile` is gone.

use crate::db;
use crate::manifest::Manifest;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Household {
    pub name: String,
    pub meta_repo_path: PathBuf,
}

pub type HouseholdMap = Arc<HashMap<String, Household>>;

pub fn load_households(conn: &rusqlite::Connection) -> Result<HouseholdMap> {
    let rows = db::list_households(conn).context("list households from db")?;
    let mut map = HashMap::new();
    for row in rows {
        map.insert(
            row.name.clone(),
            Household {
                name: row.name,
                meta_repo_path: row.meta_repo_path,
            },
        );
    }
    Ok(Arc::new(map))
}

/// Load and validate the meta-repo's manifest. Convenience wrapper over
/// `manifest::load_manifest`. Used by handlers that want the manifest
/// in their hot path.
pub fn load_household_manifest(household: &Household) -> Result<Manifest> {
    crate::manifest::load_manifest(&household.meta_repo_path)
        .with_context(|| format!("load manifest for household '{}'", household.name))
}
```

- [ ] **Step 2: Fix downstream import sites**

Run: `cargo check -p reeved 2>&1 | head -40`
Expected: a list of compile errors at every site that imports the removed `HouseholdProfile` or `RepoSpec` (which moves out of `clones.rs` in Task 4) or `load_all`. Fix each:

- `effects.rs`: replace `HouseholdProfile` references with `Household` + `load_household_manifest` calls (full body change happens in Task 11).
- `handlers.rs`: same — body changes happen across Tasks 5–9.
- `main.rs`: replace `household::load_all(...)` with `household::load_households(&conn)`.

For this task, make compilation pass by stubbing out the changed bodies with `todo!()` markers that explicit reference the task they belong to:

```rust
fn old_function(...) -> ... {
    // M3.5 Task 9: rewrite to use new Household + manifest.
    todo!("M3.5 Task 9")
}
```

- [ ] **Step 3: Run `cargo check`**

Run: `cargo check -p reeved`
Expected: PASS with no errors. The `todo!()` macros compile.

- [ ] **Step 4: Run existing tests; expect failures from removed types**

Run: `cargo test -p reeved 2>&1 | tail -30`
Expected: tests that constructed `HouseholdProfile` (M3's `household::tests`) fail to compile. **Remove those tests entirely** — they're testing a removed schema, not a refactored one. Manifest tests from Task 1 stay.

- [ ] **Step 5: Commit**

```bash
git add crates/reeved/src/household.rs crates/reeved/src/effects.rs crates/reeved/src/handlers.rs crates/reeved/src/main.rs
git commit -m "refactor(m3.5): replace HouseholdProfile with sqlite Household; M3 tests removed"
```

---

## Task 4: Clones rewrite — meta-repo + manifest iteration

**Files:**
- Modify: `crates/reeved/src/clones.rs` (rewrite `clone_all_repos` signature and body; keep `RepoSpec` if other code references it, otherwise remove)

- [ ] **Step 1: Add the new clone-all entrypoint**

In `crates/reeved/src/clones.rs`, alongside the existing `clone_all_repos`, add:

```rust
use crate::household::Household;
use crate::manifest::Manifest;

/// Clone the workspace meta-repo plus every separate-sibling repo declared in
/// the manifest into the card workspace, using `--reference --dissociate` per
/// the parent design's poly-repo topology. Inline entries (directories tracked
/// inside the meta-repo without their own `.git/`) require no separate clone —
/// they arrive with the workspace clone.
///
/// The card workspace dir is created by `git clone` itself in step 1 (git
/// refuses to clone into a non-empty directory). The caller must NOT
/// `create_dir_all(workspace)` before calling.
///
/// On any failure, the partial workspace is removed (best-effort) and the
/// error is returned.
pub async fn clone_all_for_card(
    ops: &dyn GitOps,
    household: &Household,
    manifest: &Manifest,
    workspace: &Path,
) -> Result<()> {
    let meta = &household.meta_repo_path;

    // Pre-flight: validate every NON-workspace entry exists on disk in some
    // form. Missing entirely => error. Existing without .git/ => inline.
    // Existing with .git/ => sibling. Workspace entry doesn't need this check.
    for entry in manifest.siblings() {
        let dir = meta.join(&entry.name);
        if !dir.exists() {
            anyhow::bail!(
                "sibling '{}' declared in household.json but not cloned at {} — \
                 run your workspace's bootstrap (e.g. `make setup`) first",
                entry.name,
                dir.display(),
            );
        }
    }

    // Step 1: clone the workspace entry. Its on-disk source is the meta-repo
    // root itself. `git clone` creates `<workspace>/`.
    if let Err(e) = ops.clone(meta, workspace).await {
        // Workspace dir may have been partially created by git; clean it up.
        cleanup(workspace);
        return Err(e.context("clone workspace meta-repo"));
    }

    // Step 2: for each non-workspace entry, dispatch on disk state.
    for entry in manifest.siblings() {
        let source = meta.join(&entry.name);
        let dest = workspace.join(&entry.name);
        if source.join(".git").exists() {
            // Separate sibling repo.
            if let Err(e) = ops.clone(&source, &dest).await {
                cleanup(workspace);
                return Err(e.context(format!("clone sibling '{}'", entry.name)));
            }
        } else {
            // Inline directory. No clone needed — it travelled with the
            // workspace clone. Verify it actually arrived (defensive).
            if !dest.exists() {
                cleanup(workspace);
                anyhow::bail!(
                    "inline entry '{}' is missing from workspace clone — \
                     is it correctly tracked in the meta-repo?",
                    entry.name,
                );
            }
        }
    }
    Ok(())
}

fn cleanup(workspace: &Path) {
    if workspace.exists() {
        if let Err(e) = std::fs::remove_dir_all(workspace) {
            warn!(
                "failed to clean partial workspace {}: {e:#}",
                workspace.display()
            );
        }
    }
}
```

- [ ] **Step 2: Write the unit test using `MockGitOps`**

Append in the same `#[cfg(test)] mod tests` block in `clones.rs`:

```rust
    use crate::household::Household;
    use crate::manifest::{Manifest, RepoEntry};

    fn host_meta(dir: &Path, sibling_names: &[&str]) -> PathBuf {
        let meta = dir.join("meta");
        std::fs::create_dir_all(meta.join(".git")).unwrap();
        for name in sibling_names {
            std::fs::create_dir_all(meta.join(name).join(".git")).unwrap();
        }
        meta
    }

    /// Build a host meta-repo with workspace `.git/` + each named sibling.
    /// `siblings_with_git` get their own `.git/`; `inline_dirs` get a directory
    /// but no `.git/`.
    fn host_meta(dir: &Path, siblings_with_git: &[&str], inline_dirs: &[&str]) -> PathBuf {
        let meta = dir.join("meta");
        std::fs::create_dir_all(meta.join(".git")).unwrap();
        for name in siblings_with_git {
            std::fs::create_dir_all(meta.join(name).join(".git")).unwrap();
        }
        for name in inline_dirs {
            std::fs::create_dir_all(meta.join(name)).unwrap();
        }
        meta
    }

    fn workspace_manifest(workspace_name: &str, repos: &[&str], kb: Option<&str>) -> Manifest {
        Manifest {
            workspace: workspace_name.into(),
            knowledge_base: kb.map(str::to_string),
            repos: repos
                .iter()
                .map(|n| RepoEntry {
                    name: (*n).into(),
                    url: None,
                    tags: vec![],
                    description: None,
                })
                .collect(),
        }
    }

    #[tokio::test]
    async fn clone_all_for_card_clones_workspace_then_separate_siblings() {
        let dir = tempdir().unwrap();
        let meta = host_meta(dir.path(), &["backend", "lore"], &[]);
        let workspace = dir.path().join("ws"); // do NOT pre-create
        let household = Household {
            name: "test".into(),
            meta_repo_path: meta.clone(),
        };
        let manifest = workspace_manifest("test", &["test", "backend", "lore"], Some("lore"));
        let ops = MockGitOps::new();
        clone_all_for_card(&ops, &household, &manifest, &workspace)
            .await
            .unwrap();
        let calls = ops.calls.lock().unwrap();
        assert_eq!(calls.len(), 3, "workspace + 2 separate siblings");
        assert_eq!(calls[0].0, meta, "workspace cloned first from meta path");
        assert_eq!(calls[0].1, workspace);
        assert_eq!(calls[1].0, meta.join("backend"));
        assert_eq!(calls[1].1, workspace.join("backend"));
        assert_eq!(calls[2].0, meta.join("lore"));
        assert_eq!(calls[2].1, workspace.join("lore"));
    }

    #[tokio::test]
    async fn clone_all_for_card_skips_inline_entries() {
        // Inline `lore/` exists in the meta-repo but has no .git/.
        // The mock's clone of the workspace creates the dest dir;
        // we have to seed it with the inline `lore/` to mimic the
        // real "it travels with the meta-repo clone" behaviour.
        let dir = tempdir().unwrap();
        let meta = host_meta(dir.path(), &[], &["lore"]);
        let workspace = dir.path().join("ws");
        let household = Household {
            name: "test".into(),
            meta_repo_path: meta.clone(),
        };
        let manifest = workspace_manifest("test", &["test", "lore"], Some("lore"));

        // Custom mock that creates the workspace AND copies inline dirs.
        struct InlineSimulatingOps {
            inner: MockGitOps,
            inline_to_seed: Vec<(PathBuf, PathBuf)>, // (src, dest-under-workspace)
        }
        #[async_trait]
        impl GitOps for InlineSimulatingOps {
            async fn clone(&self, source: &Path, dest: &Path) -> anyhow::Result<()> {
                self.inner.clone(source, dest).await?;
                // After the workspace clone, the real `git clone` would have
                // brought every tracked subdirectory along. Simulate that for
                // the workspace-level clone only.
                if dest == self.inline_to_seed.first().map(|(_, d)| d.parent().unwrap()).unwrap_or(dest) {
                    for (src_rel, _) in &self.inline_to_seed {
                        let abs_src = source.join(src_rel.file_name().unwrap());
                        let abs_dest = dest.join(src_rel.file_name().unwrap());
                        if abs_src.exists() {
                            std::fs::create_dir_all(&abs_dest)?;
                        }
                    }
                }
                Ok(())
            }
        }
        let ops = InlineSimulatingOps {
            inner: MockGitOps::new(),
            inline_to_seed: vec![(meta.join("lore"), workspace.join("lore"))],
        };
        clone_all_for_card(&ops, &household, &manifest, &workspace)
            .await
            .unwrap();
        let calls = ops.inner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "only the workspace was cloned; lore is inline");
        assert_eq!(calls[0].0, meta);
        assert!(workspace.join("lore").exists(), "inline lore/ landed via workspace clone");
    }

    #[tokio::test]
    async fn clone_all_for_card_errors_on_missing_sibling() {
        let dir = tempdir().unwrap();
        let meta = host_meta(dir.path(), &[], &[]); // declare "backend" but don't create it
        let workspace = dir.path().join("ws");
        let household = Household {
            name: "test".into(),
            meta_repo_path: meta,
        };
        let manifest = workspace_manifest("test", &["test", "backend"], None);
        let ops = MockGitOps::new();
        let err = clone_all_for_card(&ops, &household, &manifest, &workspace)
            .await
            .unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("declared in household.json but not cloned"),
            "unexpected error: {msg}",
        );
        assert_eq!(ops.calls.lock().unwrap().len(), 0, "no clones issued");
        assert!(!workspace.exists(), "workspace not created on pre-flight failure");
    }

    #[tokio::test]
    async fn clone_all_for_card_with_workspace_only_clones_workspace() {
        let dir = tempdir().unwrap();
        let meta = host_meta(dir.path(), &[], &[]);
        let workspace = dir.path().join("ws");
        let household = Household {
            name: "test".into(),
            meta_repo_path: meta.clone(),
        };
        let manifest = workspace_manifest("test", &["test"], None);
        let ops = MockGitOps::new();
        clone_all_for_card(&ops, &household, &manifest, &workspace)
            .await
            .unwrap();
        let calls = ops.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, meta);
        assert_eq!(calls[0].1, workspace);
    }
```

- [ ] **Step 3: Run the unit tests**

Run: `cargo test -p reeved clones::tests::clone_all_for_card`
Expected: 3 tests PASS.

- [ ] **Step 4: Add a real-git integration test**

In the `#[cfg(test)] mod integration_tests` block (same file), add:

```rust
    use crate::household::Household;
    use crate::manifest::{Manifest, RepoEntry};

    /// Init a meta-repo with one sibling, both real git repos with one commit each.
    fn full_workspace(dir: &Path) -> (PathBuf, Household, Manifest) {
        let meta = host_repo(dir, "meta");
        // Carve out a sibling inside the meta-repo dir.
        let _sibling = host_repo(&meta, "backend");
        let household = Household {
            name: "test".into(),
            meta_repo_path: meta.clone(),
        };
        let manifest = Manifest {
            workspace: "meta".into(),
            knowledge_base: None,
            repos: vec![
                RepoEntry {
                    name: "meta".into(),
                    url: None,
                    tags: vec![],
                    description: None,
                },
                RepoEntry {
                    name: "backend".into(),
                    url: None,
                    tags: vec![],
                    description: None,
                },
            ],
        };
        (meta, household, manifest)
    }

    #[tokio::test]
    async fn real_clone_all_for_card_lands_workspace_and_siblings() {
        let dir = tempdir().unwrap();
        let (_meta, household, manifest) = full_workspace(dir.path());
        let workspace = dir.path().join("workspace"); // do NOT pre-create

        let ops = RealGitOps;
        clone_all_for_card(&ops, &household, &manifest, &workspace)
            .await
            .unwrap();

        assert!(workspace.join(".git").exists(), "workspace meta-repo cloned");
        assert!(workspace.join("backend/.git").exists(), "sibling cloned");
    }
```

- [ ] **Step 5: Run the integration test**

Run: `cargo test -p reeved clones::integration_tests::real_clone_all_for_card_lands_meta_and_siblings`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/reeved/src/clones.rs
git commit -m "feat(m3.5): clone_all_for_card — meta-repo + manifest iteration"
```

---

## Task 5: Protocol — household RPC types

**Files:**
- Modify: `crates/reeve-protocol/src/lib.rs`

- [ ] **Step 1: Append the new types**

Open `crates/reeve-protocol/src/lib.rs`. Locate the existing `params` / `result` types (probably near the bottom). Append:

```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HouseholdSource {
    Path { path: PathBuf },
    Url { url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HouseholdAddParams {
    pub name: String,
    pub source: HouseholdSource,
    /// Override default clone location for URL sources. Ignored for path sources.
    #[serde(default)]
    pub clone_into: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HouseholdNewParams {
    pub name: String,
    pub path: Option<PathBuf>,
    pub template: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HouseholdSummary {
    pub name: String,
    pub meta_repo_path: PathBuf,
    pub workspace_name: String,
    pub knowledge_base: Option<String>,
    pub repo_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoOnDisk {
    /// Directory exists with its own .git/ — separate sibling repo.
    Sibling,
    /// Directory exists without .git/ — tracked as part of the workspace meta-repo.
    Inline,
    /// Directory doesn't exist on the host. Sibling that wasn't bootstrapped.
    Missing,
    /// The workspace entry itself — lives at the meta-repo root, always "present".
    Workspace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoDetail {
    pub name: String,
    pub url: Option<String>,
    pub tags: Vec<String>,
    pub description: Option<String>,
    /// True iff this entry is the manifest's `workspace`.
    pub is_workspace: bool,
    /// True iff this entry is the manifest's `knowledge_base`.
    pub is_knowledge_base: bool,
    /// On-disk classification, derived at `show` time.
    pub on_disk: RepoOnDisk,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardReference {
    pub id: String,
    pub brief: String,
    pub column: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HouseholdDetail {
    pub summary: HouseholdSummary,
    pub workspace_url: Option<String>,
    pub repos: Vec<RepoDetail>,
    pub cards: Vec<CardReference>,
}
```

- [ ] **Step 2: Compile the protocol crate**

Run: `cargo check -p reeve-protocol`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/reeve-protocol/src/lib.rs
git commit -m "feat(m3.5): protocol types for household add/new/ls/rm/show"
```

---

## Task 6: Handler — `household add`

**Files:**
- Modify: `crates/reeved/src/handlers.rs`

- [ ] **Step 1: Implement `handle_household_add`**

Open `crates/reeved/src/handlers.rs`. Add (alongside existing handlers, matching their signature pattern — likely `async fn handle_X(state: &DaemonState, params: XParams) -> Result<XResult, RpcError>`):

```rust
use reeve_protocol::{HouseholdAddParams, HouseholdSource, HouseholdSummary};
use crate::db::{self, HouseholdRow};
use crate::manifest;
use std::time::{SystemTime, UNIX_EPOCH};

pub async fn handle_household_add(
    state: &DaemonState,
    params: HouseholdAddParams,
) -> Result<HouseholdSummary, RpcError> {
    let meta_repo_path = match &params.source {
        HouseholdSource::Path { path } => {
            if params.clone_into.is_some() {
                return Err(RpcError::invalid_params(
                    "--path is only valid for URL sources",
                ));
            }
            if !path.exists() || !path.join(".git").exists() {
                return Err(RpcError::not_found(format!(
                    "path {} is not a git repository",
                    path.display()
                )));
            }
            path.clone()
        }
        HouseholdSource::Url { url } => {
            let dest = params
                .clone_into
                .clone()
                .unwrap_or_else(|| default_source_dir().join(&params.name));
            if dest.exists() {
                return Err(RpcError::conflict(format!(
                    "target directory {} already exists",
                    dest.display()
                )));
            }
            state
                .git
                .clone(std::path::Path::new(url), &dest)
                .await
                .map_err(|e| RpcError::internal(format!("clone {url}: {e:#}")))?;
            dest
        }
    };

    // Validate the manifest BEFORE inserting into sqlite.
    let m = manifest::load_manifest(&meta_repo_path)
        .map_err(|e| RpcError::invalid_argument(format!("manifest invalid: {e:#}")))?;

    let added_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let row = HouseholdRow {
        name: params.name.clone(),
        meta_repo_path: meta_repo_path.clone(),
        added_at,
    };
    db::insert_household(&state.conn(), &row)
        .map_err(|e| RpcError::conflict(format!("insert household: {e:#}")))?;

    Ok(summary_from(&row, &m))
}

fn default_source_dir() -> PathBuf {
    std::env::var("HOME")
        .map(|h| PathBuf::from(h).join("Source"))
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn summary_from(row: &HouseholdRow, m: &manifest::Manifest) -> HouseholdSummary {
    HouseholdSummary {
        name: row.name.clone(),
        meta_repo_path: row.meta_repo_path.clone(),
        workspace_name: m.workspace.clone(),
        knowledge_base: m.knowledge_base.clone(),
        repo_count: m.repos.len(),
    }
}
```

The `state.conn()` accessor and the `RpcError::invalid_params/conflict/internal/invalid_argument/not_found` constructors must already exist in the codebase from M1/M2/M3. If a needed variant is missing, add it in this task with a matching one-line change in `handlers.rs`'s error module.

- [ ] **Step 2: Add it to the RPC dispatcher**

Find the dispatcher (likely a `match method_name` table in `server.rs` or `handlers.rs`). Add:

```rust
"household.add" => dispatch_json::<HouseholdAddParams, _>(params, |p| handle_household_add(state, p)).await,
```

Match the existing dispatch pattern exactly — argument plumbing varies between projects.

- [ ] **Step 3: Write a unit test**

Append in the handlers module's tests:

```rust
    use tempfile::tempdir;
    use reeve_protocol::{HouseholdAddParams, HouseholdSource};

    /// Build a state with an in-memory DB and the test MockGitOps.
    /// Reuse whatever `test_state()` helper exists in handlers tests today.

    #[tokio::test]
    async fn household_add_via_path_succeeds() {
        let dir = tempdir().unwrap();
        let meta = dir.path().join("ws");
        std::fs::create_dir_all(meta.join(".git")).unwrap();
        std::fs::write(
            meta.join("household.json"),
            r#"{ "workspace": "ws", "repos": [{ "name": "ws" }] }"#,
        ).unwrap();
        let state = test_state();
        let result = handle_household_add(
            &state,
            HouseholdAddParams {
                name: "test".into(),
                source: HouseholdSource::Path { path: meta.clone() },
                clone_into: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(result.name, "test");
        assert_eq!(result.meta_repo_path, meta);
    }

    #[tokio::test]
    async fn household_add_rejects_invalid_manifest() {
        let dir = tempdir().unwrap();
        let meta = dir.path().join("ws");
        std::fs::create_dir_all(meta.join(".git")).unwrap();
        std::fs::write(
            meta.join("household.json"),
            r#"{ "workspace": "ws", "knowledge_base": "missing", "repos": [{ "name": "ws" }] }"#,
        ).unwrap();
        let state = test_state();
        let err = handle_household_add(
            &state,
            HouseholdAddParams {
                name: "test".into(),
                source: HouseholdSource::Path { path: meta },
                clone_into: None,
            },
        )
        .await
        .unwrap_err();
        assert!(format!("{err:?}").contains("manifest invalid"));
    }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p reeved handlers::tests::household_add`
Expected: 2 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/reeved/src/handlers.rs
git commit -m "feat(m3.5): handle_household_add — path or URL source"
```

---

## Task 7: Handler — `household new`

**Files:**
- Modify: `crates/reeved/src/handlers.rs`

- [ ] **Step 1: Implement `handle_household_new`**

```rust
use reeve_protocol::HouseholdNewParams;

const DEFAULT_TEMPLATE: &str = "https://github.com/Mindful-Stack/witan-household";

pub async fn handle_household_new(
    state: &DaemonState,
    params: HouseholdNewParams,
) -> Result<HouseholdSummary, RpcError> {
    let dest = params
        .path
        .unwrap_or_else(|| default_source_dir().join(&params.name));
    if dest.exists() && dest.read_dir().map(|mut i| i.next().is_some()).unwrap_or(false) {
        return Err(RpcError::conflict(format!(
            "target directory {} exists and is non-empty",
            dest.display()
        )));
    }
    let template = params.template.unwrap_or_else(|| DEFAULT_TEMPLATE.into());

    // 1. Shallow-clone the template.
    state
        .git
        .clone(std::path::Path::new(&template), &dest)
        .await
        .map_err(|e| RpcError::internal(format!("clone template {template}: {e:#}")))?;

    // 2. Detach from the template's history: rm .git, re-init, single commit.
    std::fs::remove_dir_all(dest.join(".git"))
        .map_err(|e| RpcError::internal(format!("remove .git: {e:#}")))?;
    git_in(&dest, &["init", "-b", "main", "--quiet"])
        .map_err(|e| RpcError::internal(format!("git init: {e:#}")))?;

    // 3. Substitute the workspace name in BOTH places: the top-level `workspace`
    //    pointer AND the `repos[]` entry it points at.
    let manifest_path = dest.join("household.json");
    let body = std::fs::read_to_string(&manifest_path)
        .map_err(|e| RpcError::internal(format!("read manifest: {e:#}")))?;
    let mut m: manifest::Manifest = serde_json::from_str(&body)
        .map_err(|e| RpcError::internal(format!("parse template manifest: {e:#}")))?;
    let old_workspace = m.workspace.clone();
    // Rename the repos[] entry that the template pointed at.
    if let Some(entry) = m.repos.iter_mut().find(|r| r.name == old_workspace) {
        entry.name = params.name.clone();
    } else {
        return Err(RpcError::internal(format!(
            "template manifest's workspace = '{old_workspace}' has no matching repos[] entry"
        )));
    }
    m.workspace = params.name.clone();
    let body = serde_json::to_string_pretty(&m)
        .map_err(|e| RpcError::internal(format!("re-serialize manifest: {e:#}")))?;
    std::fs::write(&manifest_path, body)
        .map_err(|e| RpcError::internal(format!("write manifest: {e:#}")))?;

    // 4. Initial commit.
    git_in(&dest, &["add", "-A"])
        .map_err(|e| RpcError::internal(format!("git add: {e:#}")))?;
    git_in(&dest, &["commit", "-m", "Initial workspace", "--quiet"])
        .map_err(|e| RpcError::internal(format!("git commit: {e:#}")))?;

    // 5. Register.
    let added_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let row = HouseholdRow {
        name: params.name,
        meta_repo_path: dest.clone(),
        added_at,
    };
    db::insert_household(&state.conn(), &row)
        .map_err(|e| RpcError::conflict(format!("insert household: {e:#}")))?;

    Ok(summary_from(&row, &m))
}

fn git_in(dir: &Path, args: &[&str]) -> anyhow::Result<()> {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()?;
    if !status.success() {
        anyhow::bail!("git {args:?} failed in {}", dir.display());
    }
    Ok(())
}
```

- [ ] **Step 2: Add it to the dispatcher**

```rust
"household.new" => dispatch_json::<HouseholdNewParams, _>(params, |p| handle_household_new(state, p)).await,
```

- [ ] **Step 3: Test it (path-source variant, no network)**

Add to the handlers tests:

```rust
    #[tokio::test]
    async fn household_new_uses_default_template_path_when_provided() {
        // Build a fake template on the local FS to avoid network.
        let dir = tempdir().unwrap();
        let template = dir.path().join("template");
        std::fs::create_dir_all(template.join(".git/objects/info")).unwrap();
        // Make it a valid git repo with one commit.
        std::process::Command::new("git").args(["init", "-b", "main", "--quiet"]).current_dir(&template).status().unwrap();
        std::process::Command::new("git").args(["config", "user.email", "t@t.com"]).current_dir(&template).status().unwrap();
        std::process::Command::new("git").args(["config", "user.name", "t"]).current_dir(&template).status().unwrap();
        std::fs::write(
            template.join("household.json"),
            r#"{ "workspace": "default", "repos": [{ "name": "default" }] }"#,
        ).unwrap();
        std::process::Command::new("git").args(["add", "-A"]).current_dir(&template).status().unwrap();
        std::process::Command::new("git").args(["commit", "-m", "init", "--quiet"]).current_dir(&template).status().unwrap();

        let dest = dir.path().join("my-ws");
        let state = test_state();
        let result = handle_household_new(
            &state,
            HouseholdNewParams {
                name: "my-ws".into(),
                path: Some(dest.clone()),
                template: Some(format!("file://{}", template.display())),
            },
        )
        .await
        .unwrap();

        assert_eq!(result.workspace_name, "my-ws", "manifest workspace pointer rewritten");
        assert!(dest.join(".git").exists(), "fresh git history");
        assert!(dest.join("household.json").exists());
        let manifest_body = std::fs::read_to_string(dest.join("household.json")).unwrap();
        assert!(manifest_body.contains("\"workspace\": \"my-ws\""), "workspace pointer substituted: {manifest_body}");
        assert!(manifest_body.contains("\"name\": \"my-ws\""), "matching repos[] entry renamed: {manifest_body}");
    }

    #[tokio::test]
    async fn household_new_refuses_non_empty_target() {
        let dir = tempdir().unwrap();
        let dest = dir.path().join("occupied");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("something"), "x").unwrap();
        let state = test_state();
        let err = handle_household_new(
            &state,
            HouseholdNewParams {
                name: "occupied".into(),
                path: Some(dest),
                template: Some("file:///nonexistent".into()),
            },
        )
        .await
        .unwrap_err();
        assert!(format!("{err:?}").contains("exists and is non-empty"));
    }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p reeved handlers::tests::household_new`
Expected: 2 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/reeved/src/handlers.rs
git commit -m "feat(m3.5): handle_household_new — scaffold from template"
```

---

## Task 8: Handlers — `household ls`, `rm`, `show`

**Files:**
- Modify: `crates/reeved/src/handlers.rs`

- [ ] **Step 1: Implement the three handlers**

```rust
pub async fn handle_household_ls(state: &DaemonState) -> Result<Vec<HouseholdSummary>, RpcError> {
    let rows = db::list_households(&state.conn())
        .map_err(|e| RpcError::internal(format!("list households: {e:#}")))?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let m = manifest::load_manifest(&row.meta_repo_path).unwrap_or_else(|_| manifest::Manifest {
            name: Some(row.name.clone()),
            url: None,
            knowledge_base: None,
            repos: vec![],
        });
        out.push(summary_from(&row, &m));
    }
    Ok(out)
}

#[derive(serde::Deserialize)]
pub struct NameParam {
    pub name: String,
}

pub async fn handle_household_rm(
    state: &DaemonState,
    params: NameParam,
) -> Result<(), RpcError> {
    let removed = db::remove_household(&state.conn(), &params.name)
        .map_err(|e| RpcError::internal(format!("remove household: {e:#}")))?;
    if !removed {
        return Err(RpcError::not_found(format!("household '{}' not found", params.name)));
    }
    Ok(())
}

pub async fn handle_household_show(
    state: &DaemonState,
    params: NameParam,
) -> Result<reeve_protocol::HouseholdDetail, RpcError> {
    let row = db::get_household(&state.conn(), &params.name)
        .map_err(|e| RpcError::internal(format!("get household: {e:#}")))?
        .ok_or_else(|| RpcError::not_found(format!("household '{}' not found", params.name)))?;

    let m = manifest::load_manifest(&row.meta_repo_path)
        .map_err(|e| RpcError::internal(format!("load manifest: {e:#}")))?;

    let workspace_url = m
        .workspace_entry()
        .url
        .clone()
        .or_else(|| git_remote_url(&row.meta_repo_path));

    let repos = m
        .repos
        .iter()
        .map(|entry| {
            let is_workspace = entry.name == m.workspace;
            let is_knowledge_base = m.knowledge_base.as_deref() == Some(entry.name.as_str());
            let on_disk = if is_workspace {
                reeve_protocol::RepoOnDisk::Workspace
            } else {
                let dir = row.meta_repo_path.join(&entry.name);
                if dir.join(".git").exists() {
                    reeve_protocol::RepoOnDisk::Sibling
                } else if dir.exists() {
                    reeve_protocol::RepoOnDisk::Inline
                } else {
                    reeve_protocol::RepoOnDisk::Missing
                }
            };
            reeve_protocol::RepoDetail {
                name: entry.name.clone(),
                url: entry.url.clone(),
                tags: entry.tags.clone(),
                description: entry.description.clone(),
                is_workspace,
                is_knowledge_base,
                on_disk,
            }
        })
        .collect();

    let cards = db::cards_for_household(&state.conn(), &params.name)
        .map_err(|e| RpcError::internal(format!("list cards: {e:#}")))?
        .into_iter()
        .map(|(id, brief, column)| reeve_protocol::CardReference { id, brief, column })
        .collect();

    Ok(reeve_protocol::HouseholdDetail {
        summary: summary_from(&row, &m),
        workspace_url,
        repos,
        cards,
    })
}

fn git_remote_url(meta_repo: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(meta_repo)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
```

- [ ] **Step 2: Wire into the dispatcher**

```rust
"household.ls"   => dispatch_json::<(), _>(params, |_| handle_household_ls(state)).await,
"household.rm"   => dispatch_json::<NameParam, _>(params, |p| handle_household_rm(state, p)).await,
"household.show" => dispatch_json::<NameParam, _>(params, |p| handle_household_show(state, p)).await,
```

- [ ] **Step 3: Tests**

```rust
    #[tokio::test]
    async fn household_ls_returns_inserted_households() {
        let dir = tempdir().unwrap();
        let state = test_state();
        // Insert two via add.
        for n in ["alpha", "beta"] {
            let meta = dir.path().join(n);
            std::fs::create_dir_all(meta.join(".git")).unwrap();
            std::fs::write(
                meta.join("household.json"),
                format!(r#"{{ "workspace": "{n}", "repos": [{{ "name": "{n}" }}] }}"#),
            ).unwrap();
            handle_household_add(
                &state,
                HouseholdAddParams {
                    name: n.into(),
                    source: HouseholdSource::Path { path: meta },
                    clone_into: None,
                },
            )
            .await
            .unwrap();
        }
        let list = handle_household_ls(&state).await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "alpha");
        assert_eq!(list[1].name, "beta");
    }

    #[tokio::test]
    async fn household_rm_returns_not_found_for_unknown() {
        let state = test_state();
        let err = handle_household_rm(&state, NameParam { name: "nope".into() }).await.unwrap_err();
        assert!(format!("{err:?}").contains("not found"));
    }

    #[tokio::test]
    async fn household_show_classifies_each_entry() {
        use reeve_protocol::RepoOnDisk;
        let dir = tempdir().unwrap();
        let meta = dir.path().join("ws");
        std::fs::create_dir_all(meta.join(".git")).unwrap();
        std::fs::create_dir_all(meta.join("backend/.git")).unwrap();   // separate sibling, present
        std::fs::create_dir_all(meta.join("lore")).unwrap();           // inline (no .git/)
        // "frontend" declared but NOT created -> Missing
        std::fs::write(
            meta.join("household.json"),
            r#"{
              "workspace": "ws",
              "knowledge_base": "lore",
              "repos": [
                { "name": "ws" },
                { "name": "backend" },
                { "name": "lore" },
                { "name": "frontend" }
              ]
            }"#,
        ).unwrap();
        let state = test_state();
        handle_household_add(
            &state,
            HouseholdAddParams {
                name: "test".into(),
                source: HouseholdSource::Path { path: meta },
                clone_into: None,
            },
        )
        .await
        .unwrap();
        let detail = handle_household_show(&state, NameParam { name: "test".into() }).await.unwrap();
        assert_eq!(detail.repos.len(), 4);
        let by = |n: &str| detail.repos.iter().find(|r| r.name == n).unwrap();
        assert!(by("ws").is_workspace);
        assert_eq!(by("ws").on_disk, RepoOnDisk::Workspace);
        assert!(by("lore").is_knowledge_base);
        assert_eq!(by("lore").on_disk, RepoOnDisk::Inline);
        assert_eq!(by("backend").on_disk, RepoOnDisk::Sibling);
        assert_eq!(by("frontend").on_disk, RepoOnDisk::Missing);
    }

    #[tokio::test]
    async fn household_show_lists_referencing_cards() {
        // Requires the `cards` table to exist; M3 set this up. Insert two cards
        // referencing the household, plus one referencing a different household.
        // Then assert detail.cards has exactly the two.
        let dir = tempdir().unwrap();
        let meta = dir.path().join("ws");
        std::fs::create_dir_all(meta.join(".git")).unwrap();
        std::fs::write(
            meta.join("household.json"),
            r#"{ "workspace": "ours", "repos": [{ "name": "ours" }] }"#,
        ).unwrap();
        let state = test_state();
        handle_household_add(
            &state,
            HouseholdAddParams {
                name: "ours".into(),
                source: HouseholdSource::Path { path: meta },
                clone_into: None,
            },
        )
        .await
        .unwrap();
        // Insert cards directly via db (cards table schema is from M3).
        // Use whatever insert-card helper the existing tests use; the example
        // here assumes a `crate::db::insert_card` with positional args.
        crate::db::insert_card(&state.conn(), "card-1", "brief one", "Backlog", "ours").unwrap();
        crate::db::insert_card(&state.conn(), "card-2", "brief two", "Implementing", "ours").unwrap();
        crate::db::insert_card(&state.conn(), "card-3", "brief three", "Backlog", "someone-else").unwrap();
        let detail = handle_household_show(&state, NameParam { name: "ours".into() }).await.unwrap();
        let ids: Vec<&str> = detail.cards.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["card-1", "card-2"]);
    }
```

If `db::insert_card` doesn't exist as written, substitute the actual insert path used by M3's tests.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p reeved handlers::tests -- household_`
Expected: all household_* tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/reeved/src/handlers.rs
git commit -m "feat(m3.5): handle_household_ls/rm/show"
```

---

## Task 9: Effects — devcontainer path convention

**Files:**
- Modify: `crates/reeved/src/effects.rs`

- [ ] **Step 1: Replace the `todo!("M3.5 Task 9")` stub in `ensure_running`**

Open `effects.rs`. Find `ensure_running` (the function that transitions a card to `Implementing` and spawns the container). The M3 version:
- looked up `state.households.get(&card.household_name)`
- called `clone_all_repos(ops, &workspace, &profile.repos)`
- passed `profile.devcontainer` to the devcontainer-up call

Rewrite to:
- look up the `Household` from `state.households`
- load the manifest via `household::load_household_manifest(&household)`
- call `clones::clone_all_for_card(ops, &household, &manifest, &workspace)`
- pass `workspace.join(".devcontainer/devcontainer.json")` to the devcontainer-up call

```rust
use crate::clones;
use crate::household::{self, Household};

async fn ensure_running(state: &DaemonState, card_id: &str) -> Result<()> {
    // ...prior preconditions unchanged...

    let card = /* fetch card row by id, same as M3 */;
    let household = state
        .households
        .get(&card.household_name)
        .with_context(|| format!("household '{}' not configured", card.household_name))?
        .clone();

    let manifest = household::load_household_manifest(&household)?;
    let workspace = state.cards_dir.join(card_id);
    std::fs::create_dir_all(&workspace)
        .with_context(|| format!("mkdir workspace {}", workspace.display()))?;

    clones::clone_all_for_card(&*state.git, &household, &manifest, &workspace).await?;

    let devcontainer_path = workspace.join(".devcontainer").join("devcontainer.json");
    if !devcontainer_path.exists() {
        anyhow::bail!(
            "devcontainer.json missing at {} — every witan-household must ship one at the conventional location",
            devcontainer_path.display(),
        );
    }

    /* call into existing devcontainer-up code with `devcontainer_path` and `workspace` */
    Ok(())
}
```

Adapt argument plumbing to whatever signatures M3's effects.rs already uses; the new lines are the ones interacting with `manifest` and the convention-based devcontainer path.

- [ ] **Step 2: Run all existing effects tests; expect green**

Run: `cargo test -p reeved effects`
Expected: PASS (mock-based effects tests should still work; they don't depend on `HouseholdProfile` if the test scaffolding got migrated when Task 3 stubbed.)

- [ ] **Step 3: Commit**

```bash
git add crates/reeved/src/effects.rs
git commit -m "feat(m3.5): ensure_running uses manifest + convention devcontainer path"
```

---

## Task 10: CLI — `household` subcommand tree

**Files:**
- Modify: `crates/reeve/src/cli.rs`
- Modify: `crates/reeve/src/client.rs` (if it's where typed RPC wrappers live)

- [ ] **Step 1: Add the clap subcommand definitions**

Open `crates/reeve/src/cli.rs`. Find the top-level subcommand enum (clap derives). Add:

```rust
use reeve_protocol::{HouseholdAddParams, HouseholdNewParams, HouseholdSource};
use std::path::PathBuf;

#[derive(clap::Subcommand)]
pub enum Commands {
    // ... existing variants ...
    Household {
        #[command(subcommand)]
        cmd: HouseholdCommand,
    },
}

#[derive(clap::Subcommand)]
pub enum HouseholdCommand {
    /// Register an existing meta-repo (path) or clone-then-register one (URL).
    Add {
        name: String,
        /// Filesystem path OR git URL.
        source: String,
        /// Where to clone (only meaningful for URL sources). Default ~/Source/<name>/.
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Scaffold a fresh household from the witan-household template.
    New {
        name: String,
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long)]
        template: Option<String>,
    },
    /// List registered households.
    Ls,
    /// Unregister a household (does not delete the meta-repo).
    Rm { name: String },
    /// Show manifest + sibling presence + referencing cards.
    Show { name: String },
}
```

- [ ] **Step 2: Implement the dispatch in `main.rs` (or wherever subcommands route)**

```rust
Commands::Household { cmd } => match cmd {
    HouseholdCommand::Add { name, source, path } => {
        let src = if is_git_url(&source) {
            HouseholdSource::Url { url: source }
        } else {
            HouseholdSource::Path { path: PathBuf::from(source) }
        };
        let result = client.household_add(HouseholdAddParams { name, source: src, clone_into: path }).await?;
        println!("Registered {} at {}", result.name, result.meta_repo_path.display());
        println!("  workspace: {}", result.workspace_name);
        if let Some(kb) = result.knowledge_base {
            println!("  knowledge_base: {kb}");
        }
        println!("  repos: {}", result.repo_count);
    }
    HouseholdCommand::New { name, path, template } => {
        let result = client.household_new(HouseholdNewParams { name, path, template }).await?;
        println!("Scaffolded {} at {}", result.name, result.meta_repo_path.display());
        println!("\nNext steps:");
        println!("  cd {}", result.meta_repo_path.display());
        println!("  $EDITOR household.json   # declare your sibling repos");
        println!("  git remote add origin <url>   # if you want to push it to a remote");
    }
    HouseholdCommand::Ls => {
        let list = client.household_ls().await?;
        if list.is_empty() {
            println!("(no households registered — run `reeve household add` or `reeve household new`)");
        } else {
            for h in list {
                println!("{:<24} {} ({} repos)", h.name, h.meta_repo_path.display(), h.repo_count);
            }
        }
    }
    HouseholdCommand::Rm { name } => {
        client.household_rm(name.clone()).await?;
        println!("Unregistered {} (meta-repo on disk untouched)", name);
    }
    HouseholdCommand::Show { name } => {
        let detail = client.household_show(name).await?;
        println!("{} ({})", detail.summary.name, detail.summary.meta_repo_path.display());
        println!("  workspace: {}", detail.summary.workspace_name);
        if let Some(url) = detail.workspace_url {
            println!("  url: {}", url);
        }
        if let Some(kb) = &detail.summary.knowledge_base {
            println!("  knowledge_base: {} (→ repo entry of the same name)", kb);
        }
        println!("\n  Repos ({}):", detail.repos.len());
        for r in detail.repos {
            let glyph = match r.on_disk {
                reeve_protocol::RepoOnDisk::Workspace => "W",
                reeve_protocol::RepoOnDisk::Sibling => "✓",
                reeve_protocol::RepoOnDisk::Inline => "·",
                reeve_protocol::RepoOnDisk::Missing => "✗",
            };
            let kb_marker = if r.is_knowledge_base { " [KB]" } else { "" };
            println!("    {glyph} {}{kb_marker}", r.name);
        }
        if detail.cards.is_empty() {
            println!("\n  No cards reference this household.");
        } else {
            println!("\n  Cards referencing this household:");
            for c in detail.cards {
                println!("    {} [{}] {}", c.id, c.column, c.brief);
            }
        }
    }
},
```

Helper:

```rust
fn is_git_url(s: &str) -> bool {
    s.starts_with("git@") || s.starts_with("https://") || s.starts_with("http://") || s.starts_with("file://") || s.starts_with("ssh://")
}
```

- [ ] **Step 3: Implement the `client.household_*` typed wrappers**

Open `crates/reeve/src/client.rs`. Add methods that match the existing RPC-call pattern (probably `pub async fn call<P: Serialize, R: DeserializeOwned>(&self, method: &str, params: P) -> Result<R>`):

```rust
use reeve_protocol::{
    HouseholdAddParams, HouseholdDetail, HouseholdNewParams, HouseholdSummary,
};

impl Client {
    pub async fn household_add(&self, params: HouseholdAddParams) -> Result<HouseholdSummary> {
        self.call("household.add", params).await
    }
    pub async fn household_new(&self, params: HouseholdNewParams) -> Result<HouseholdSummary> {
        self.call("household.new", params).await
    }
    pub async fn household_ls(&self) -> Result<Vec<HouseholdSummary>> {
        self.call("household.ls", ()).await
    }
    pub async fn household_rm(&self, name: String) -> Result<()> {
        self.call(
            "household.rm",
            serde_json::json!({ "name": name }),
        )
        .await
    }
    pub async fn household_show(&self, name: String) -> Result<HouseholdDetail> {
        self.call(
            "household.show",
            serde_json::json!({ "name": name }),
        )
        .await
    }
}
```

- [ ] **Step 4: Run `cargo build --workspace`**

Run: `cargo build --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/reeve/src/cli.rs crates/reeve/src/client.rs crates/reeve/src/main.rs
git commit -m "feat(m3.5): reeve household add|new|ls|rm|show CLI"
```

---

## Task 11: Migration warning for old TOML

**Files:**
- Modify: `crates/reeved/src/main.rs`

- [ ] **Step 1: Add the warn**

In `main.rs`, after sqlite migration runs and before the daemon serves requests:

```rust
fn warn_if_old_toml_exists() {
    let dir = std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".config/reeve/households"))
        .ok();
    let Some(dir) = dir else { return };
    if !dir.is_dir() {
        return;
    }
    let mut found_toml = false;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if entry.path().extension().and_then(|s| s.to_str()) == Some("toml") {
                found_toml = true;
                break;
            }
        }
    }
    if found_toml {
        tracing::warn!(
            "Old-style household TOML files found in {} — M3.5 uses sqlite-backed households, \
             not TOML. Re-register each household via `reeve household add <name> <path>` \
             pointing at its meta-repo. The TOML files are otherwise ignored.",
            dir.display(),
        );
    }
}
```

Call `warn_if_old_toml_exists()` once during daemon startup, after tracing-subscriber init.

- [ ] **Step 2: Commit**

```bash
git add crates/reeved/src/main.rs
git commit -m "feat(m3.5): warn on legacy ~/.config/reeve/households/*.toml at startup"
```

---

## Task 12: E2E — `household new` + `→ Implementing` round trip

**Files:**
- Modify: `tests/e2e.rs`

- [ ] **Step 1: Add the test**

```rust
#[tokio::test]
#[ignore = "docker-gated; opt in via REEVE_TESTS_STRICT=1 or `cargo test --features docker-e2e`"]
async fn m3_5_household_new_then_implementing_lands_workspace() {
    skip_if_not_strict_and_missing("docker", || which("docker").is_some());
    skip_if_not_strict_and_missing("devcontainer", || which("devcontainer").is_some());

    let dir = tempfile::tempdir().unwrap();

    // Build a local copy of the template so we don't hit the network.
    let template_src = dir.path().join("template-source");
    build_test_template(&template_src);

    let dest = dir.path().join("my-household");

    // Bring up daemon.
    let daemon = test_daemon().await;

    // Scaffold via `reeve household new --template file://...`.
    daemon
        .client()
        .household_new(reeve_protocol::HouseholdNewParams {
            name: "test-household".into(),
            path: Some(dest.clone()),
            template: Some(format!("file://{}", template_src.display())),
        })
        .await
        .unwrap();

    // Populate `lore/` (the only manifest entry from the template) and create
    // one extra sibling so we test the manifest-iteration path.
    init_sibling(&dest.join("lore"));
    // Add a sibling entry to the manifest by hand, then init that sibling.
    write_manifest(
        &dest.join("household.json"),
        r#"{
          "workspace": "test-household",
          "knowledge_base": "lore",
          "repos": [
            { "name": "test-household" },
            { "name": "lore" },
            { "name": "extra" }
          ]
        }"#,
    );
    init_sibling(&dest.join("extra"));
    commit_meta_changes(&dest);

    // Create + transition a card.
    let card_id = daemon
        .client()
        .new_card(/* household: "test-household", brief: "m3.5 smoke" */)
        .await
        .unwrap();
    daemon
        .client()
        .move_card(&card_id, "Implementing")
        .await
        .unwrap();

    let workspace = daemon.cards_dir().join(&card_id);
    assert!(workspace.join(".git").exists(), "meta-repo cloned");
    assert!(workspace.join("lore/.git").exists(), "lore cloned");
    assert!(workspace.join("extra/.git").exists(), "extra cloned");
    assert!(
        workspace.join(".devcontainer/devcontainer.json").exists(),
        "devcontainer.json travelled with the meta-repo clone",
    );
    assert!(
        daemon.container_running_for(&card_id),
        "container is up",
    );
}

fn build_test_template(dst: &Path) {
    // Mirror the witan-household template's minimal essentials.
    std::fs::create_dir_all(dst.join(".devcontainer")).unwrap();
    std::fs::create_dir_all(dst.join("lore/knowledge/general")).unwrap();
    std::fs::write(
        dst.join("household.json"),
        r#"{
          "workspace": "template",
          "knowledge_base": "lore",
          "repos": [
            { "name": "template" },
            { "name": "lore" }
          ]
        }"#,
    ).unwrap();
    std::fs::write(
        dst.join(".devcontainer/devcontainer.json"),
        r#"{ "name": "test", "image": "mcr.microsoft.com/devcontainers/base:ubuntu" }"#,
    ).unwrap();
    std::fs::write(dst.join("CLAUDE.md"), "test workspace").unwrap();
    std::fs::write(dst.join(".gitignore"), "").unwrap();
    std::fs::write(dst.join("lore/knowledge/general/_starter.md"), "---\n").unwrap();
    git_in(dst, &["init", "-b", "main", "--quiet"]);
    git_in(dst, &["config", "user.email", "t@t.com"]);
    git_in(dst, &["config", "user.name", "t"]);
    git_in(dst, &["add", "-A"]);
    git_in(dst, &["commit", "-m", "template", "--quiet"]);
}

fn init_sibling(path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    git_in(path, &["init", "-b", "main", "--quiet"]);
    git_in(path, &["config", "user.email", "t@t.com"]);
    git_in(path, &["config", "user.name", "t"]);
    std::fs::write(path.join("README.md"), "hi\n").unwrap();
    git_in(path, &["add", "-A"]);
    git_in(path, &["commit", "-m", "initial", "--quiet"]);
}

fn commit_meta_changes(repo: &Path) {
    git_in(repo, &["add", "-A"]);
    git_in(repo, &["commit", "-m", "manifest update", "--quiet"]);
}

fn write_manifest(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
}

fn git_in(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git").args(args).current_dir(dir).status().unwrap();
    assert!(status.success(), "git {args:?} failed");
}
```

The exact `test_daemon()`, `new_card`, `move_card`, `cards_dir`, `container_running_for`, and `skip_if_not_strict_and_missing` helpers come from M3's existing e2e tests; reuse them rather than reinventing.

- [ ] **Step 2: Run the test under strict mode**

Run: `REEVE_TESTS_STRICT=1 cargo test --test e2e m3_5_household_new_then_implementing_lands_workspace -- --ignored`
Expected: PASS (or skip with a printed reason if docker/devcontainer is missing AND `REEVE_TESTS_STRICT=0`).

- [ ] **Step 3: Commit**

```bash
git add tests/e2e.rs
git commit -m "test(m3.5): e2e household new + implementing round-trip"
```

---

## Task 13: E2E — missing sibling blocks `→ Implementing`

**Files:**
- Modify: `tests/e2e.rs`

- [ ] **Step 1: Add the test**

```rust
#[tokio::test]
#[ignore = "docker-gated"]
async fn m3_5_missing_sibling_blocks_implementing() {
    skip_if_not_strict_and_missing("docker", || which("docker").is_some());

    let dir = tempfile::tempdir().unwrap();
    let template_src = dir.path().join("template-source");
    build_test_template(&template_src);
    let dest = dir.path().join("incomplete");

    let daemon = test_daemon().await;
    daemon
        .client()
        .household_new(reeve_protocol::HouseholdNewParams {
            name: "incomplete".into(),
            path: Some(dest.clone()),
            template: Some(format!("file://{}", template_src.display())),
        })
        .await
        .unwrap();

    // Declare a sibling that we deliberately do NOT populate.
    write_manifest(
        &dest.join("household.json"),
        r#"{
          "workspace": "incomplete",
          "knowledge_base": "lore",
          "repos": [
            { "name": "incomplete" },
            { "name": "lore" },
            { "name": "phantom" }
          ]
        }"#,
    );
    init_sibling(&dest.join("lore"));
    // do NOT init_sibling("phantom")
    commit_meta_changes(&dest);

    let card_id = daemon.client().new_card_for("incomplete", "smoke").await.unwrap();
    let err = daemon
        .client()
        .move_card(&card_id, "Implementing")
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("phantom") && msg.contains("not cloned"),
        "unexpected error: {msg}",
    );
    assert!(
        !daemon.container_running_for(&card_id),
        "container should NOT be running",
    );
}
```

- [ ] **Step 2: Run**

Run: `REEVE_TESTS_STRICT=1 cargo test --test e2e m3_5_missing_sibling_blocks_implementing -- --ignored`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e.rs
git commit -m "test(m3.5): missing sibling blocks → Implementing with clear error"
```

---

## Task 14: E2E — `lore/` lands in workspace and is accessible inside container

**Files:**
- Modify: `tests/e2e.rs`

- [ ] **Step 1: Add the test**

```rust
#[tokio::test]
#[ignore = "docker-gated"]
async fn m3_5_lore_subdir_lands_in_workspace() {
    skip_if_not_strict_and_missing("docker", || which("docker").is_some());

    let dir = tempfile::tempdir().unwrap();
    let template_src = dir.path().join("template-source");
    build_test_template(&template_src);
    let dest = dir.path().join("with-kb");

    let daemon = test_daemon().await;
    daemon
        .client()
        .household_new(reeve_protocol::HouseholdNewParams {
            name: "with-kb".into(),
            path: Some(dest.clone()),
            template: Some(format!("file://{}", template_src.display())),
        })
        .await
        .unwrap();

    init_sibling(&dest.join("lore"));
    commit_meta_changes(&dest);

    let card_id = daemon.client().new_card_for("with-kb", "kb smoke").await.unwrap();
    daemon.client().move_card(&card_id, "Implementing").await.unwrap();

    let workspace = daemon.cards_dir().join(&card_id);
    assert!(workspace.join("lore/.git").exists());

    // Verify the agent (had it been running with Lorekeeper) could resolve the
    // KB via sibling-fallback by checking the in-container path resolves.
    let container = format!("reeve-{}", card_id);
    let output = std::process::Command::new("docker")
        .args(["exec", &container, "test", "-d", "/workspaces/lore"])
        .output()
        .unwrap();
    assert!(output.status.success(), "lore/ accessible inside container");
}
```

- [ ] **Step 2: Run**

Run: `REEVE_TESTS_STRICT=1 cargo test --test e2e m3_5_lore_subdir_lands_in_workspace -- --ignored`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e.rs
git commit -m "test(m3.5): lore/ sibling lands in workspace and is accessible in container"
```

---

## Task 15: Workspace-wide verification + manual smoke

**Files:** none (verification only)

- [ ] **Step 1: Run the full workspace test suite under strict mode**

Run: `REEVE_TESTS_STRICT=1 cargo test --workspace`
Expected: PASS.

- [ ] **Step 2: Run formatting + clippy**

Run: `cargo fmt --check && cargo clippy --workspace -- -D warnings`
Expected: both PASS clean.

- [ ] **Step 3: Manual smoke per the spec's Verification section**

Follow the spec's Verification section steps 4.1 through 4.9 exactly:

```sh
# 1. scaffold
reeve household new reeve-self --path ~/Source/reeve-household --template https://github.com/Mindful-Stack/witan-household

# 2-3. hand-edit household.json and populate siblings
$EDITOR ~/Source/reeve-household/household.json
# Add a `reeve` entry (existing local reeve clone)
# Add a `lore` entry pointing at any local KB

# manually populate:
cd ~/Source/reeve-household
git clone --depth=1 file://$HOME/Source/reeve reeve
git clone --depth=1 file://$HOME/Source/witan-household lore  # or any KB

# 4. show
reeve household show reeve-self
# Confirm: workspace name, KB pointer, both siblings show populated_on_host=true

# 5-7. new card, mv to implementing, inspect workspace
reeve new --household reeve-self --brief "m3.5 smoke" --no-plan
# capture the printed card id as $CARD
reeve mv $CARD implementing
ls ~/reeve/cards/$CARD/   # expect: .devcontainer/, household.json, lore/, reeve/, CLAUDE.md
docker exec reeve-$CARD ls /workspaces/$CARD/  # same

# 8. (optional) install Lorekeeper inside the container and verify /lore:help
docker exec reeve-$CARD bash -lc 'claude plugin marketplace add Mindful-Stack/witan && claude plugin install lorekeeper@witan'
docker exec reeve-$CARD bash -lc 'cd /workspaces/'$CARD' && claude /lore:help'
# Expect: "Knowledge base found" with the workspace's lore/ path

# 9. mv to done
reeve mv $CARD done
# Container down; workspace stays on disk
```

If any step diverges from the spec, file a follow-up issue rather than blocking the milestone — but flag it in the PR description.

- [ ] **Step 4: Commit (workspace lockfile or version bumps if needed)**

If `Cargo.lock` changed during the milestone:

```bash
git add Cargo.lock
git commit -m "chore(m3.5): bump lockfile"
```

Otherwise, no commit.

---

## Final self-review checklist

After running through every task:

- [ ] **Spec coverage:** every section of `docs/superpowers/specs/2026-05-15-m3-5-households-as-meta-repos-design.md` maps to at least one task above.
  - Manifest schema → Task 1
  - Host-side topology → enforced by `clone_all_for_card`'s sibling-presence check (Task 4) + manual smoke (Task 15)
  - Card-side topology → Task 4 + Task 9 + Task 12
  - Lorekeeper interaction (no Reeve mechanism) → no code; documented in spec; verified in Task 14
  - DB schema → Task 2
  - Household struct rewrite → Task 3
  - Clones rewrite → Task 4
  - Effects → Task 9
  - Handlers (5) → Tasks 6, 7, 8
  - Protocol types → Task 5
  - CLI → Task 10
  - Migration warn → Task 11
  - All testing categories (unit, integration, e2e) → Tasks 1, 2, 4, 6–8, 12–14

- [ ] **Placeholder scan:** no "TBD", "TODO", "implement later", "similar to Task N" in the steps above. Code blocks are concrete.

- [ ] **Type consistency check:**
  - `Manifest { workspace: String, knowledge_base: Option<String>, repos: Vec<RepoEntry> }` — same across Tasks 1, 5, 8. Helper methods `workspace_entry()`, `knowledge_base_entry()`, `siblings()` used in Tasks 4, 8.
  - `RepoEntry { name, url, tags, description }` — fields consistent in Tasks 1, 4, 5, 8.
  - `Household { name, meta_repo_path }` — same in Tasks 3, 4, 9.
  - `HouseholdRow { name, meta_repo_path, added_at }` — same in Tasks 2, 6, 7, 8.
  - `HouseholdSummary { name, meta_repo_path, workspace_name, knowledge_base, repo_count }` — same in Tasks 5, 6, 7, 8. `workspace_name` sourced from `manifest.workspace`.
  - `RepoDetail { name, url, tags, description, is_workspace, is_knowledge_base, on_disk }`, `RepoOnDisk { Workspace, Sibling, Inline, Missing }` — same in Tasks 5, 8, 10.
  - `HouseholdDetail { summary, workspace_url, repos, cards }` — same in Tasks 5, 8. `workspace_url` sourced from the workspace entry's `url`.
  - RPC methods: `household.add`, `household.new`, `household.ls`, `household.rm`, `household.show` — same in Tasks 6–8, 10.

- [ ] **Open items deferred to writing-plans were resolved:**
  - Default clone-location flag name → `--path` (Task 10)
  - `household add <url>` does NOT auto-populate siblings (Task 6 — user runs their own bootstrap)
  - `template_used` sqlite column not added (Task 2 — `name, meta_repo_path, added_at` only)
  - `household show` lists referencing cards (Task 8)
  - Template content not bundled in binary (Task 7 — clones from URL; `--template file://...` for offline)
