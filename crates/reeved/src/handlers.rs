//! Method dispatch and per-method handlers.

use crate::db;
use crate::effects::{DaemonState, LiveEffects};
use crate::manifest;
use crate::server::{CallerContext, DbHandle};
use crate::transitions;
use anyhow::Result;
use reeve_protocol::{
    Card, CardReference, Column, HouseholdAddParams, HouseholdDetail, HouseholdNewParams,
    HouseholdSource, HouseholdSummary, LsResult, MvParams, MvResult, MvSelfParams, MvSelfResult,
    NewParams, NewResult, RepoDetail, RepoOnDisk, RpcError, RpcRequest, StatusSelfResult,
};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_TEMPLATE: &str = "https://github.com/Mindful-Stack/witan-household";

pub async fn dispatch(
    req: RpcRequest,
    state: DaemonState,
    caller: CallerContext,
) -> std::result::Result<Value, RpcError> {
    match req.method.as_str() {
        "ls" => handle_ls(state.db).await,
        "new" => {
            let params = parse_params::<NewParams>(req.params)?;
            handle_new(state.db, params).await
        }
        "mv" => {
            let params = parse_params::<MvParams>(req.params)?;
            handle_mv(state, params).await
        }
        "mv-self" => {
            let card_id = require_card(&caller)?;
            let params = parse_params::<MvSelfParams>(req.params)?;
            handle_mv_self(state, card_id, params).await
        }
        "status-self" => {
            let card_id = require_card(&caller)?;
            handle_status_self(state.db, card_id).await
        }
        "household.add" => {
            let params = parse_params::<HouseholdAddParams>(req.params)?;
            handle_household_add(&state, params).await
        }
        "household.new" => {
            let params = parse_params::<HouseholdNewParams>(req.params)?;
            handle_household_new(&state, params).await
        }
        "household.ls" => handle_household_ls(&state).await,
        "household.rm" => {
            let params = parse_params::<NameParam>(req.params)?;
            handle_household_rm(&state, params).await
        }
        "household.show" => {
            let params = parse_params::<NameParam>(req.params)?;
            handle_household_show(&state, params).await
        }
        other => Err(RpcError {
            code: RpcError::METHOD_NOT_FOUND,
            message: format!("method not found: {other}"),
        }),
    }
}

async fn precheck_household_for_implementing(
    state: &DaemonState,
    household_name: &str,
) -> std::result::Result<(), RpcError> {
    // M3.5 Task 9 will rewrite this to read the meta-repo's manifest:
    //   - <meta_repo_path>/.devcontainer/devcontainer.json must be a file
    //   - For each repos[] entry other than the workspace: the on-disk dir
    //     must exist (either as a sibling clone with .git/ or inline)
    //
    // Query the DB directly rather than the stale in-memory map so that
    // households registered after daemon start (via `household new/add`) are
    // visible immediately.
    let name = household_name.to_string();
    let db = state.db.clone();
    let found = tokio::task::spawn_blocking(move || -> Result<bool> {
        let conn = db.blocking_lock();
        Ok(db::get_household(&conn, &name)?.is_some())
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?;
    if !found {
        return Err(RpcError {
            code: RpcError::INTERNAL,
            message: format!("household '{household_name}' not configured"),
        });
    }
    Ok(())
}

fn require_card(caller: &CallerContext) -> std::result::Result<String, RpcError> {
    caller.card_id.clone().ok_or_else(|| RpcError {
        code: RpcError::FORBIDDEN,
        message: "self-scoped methods require per-card socket".into(),
    })
}

fn parse_params<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, RpcError> {
    serde_json::from_value(value).map_err(|e| RpcError {
        code: RpcError::INVALID_PARAMS,
        message: format!("invalid params: {e}"),
    })
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn to_internal<E: std::fmt::Display>(e: E) -> RpcError {
    RpcError {
        code: RpcError::INTERNAL,
        message: e.to_string(),
    }
}

async fn handle_ls(db: DbHandle) -> std::result::Result<Value, RpcError> {
    let cards = tokio::task::spawn_blocking(move || -> Result<Vec<Card>> {
        let conn = db.blocking_lock();
        db::list_cards(&conn)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?;
    serde_json::to_value(LsResult { cards }).map_err(to_internal)
}

async fn handle_new(db: DbHandle, params: NewParams) -> std::result::Result<Value, RpcError> {
    let now = now_secs();
    let card = Card {
        id: uuid::Uuid::new_v4().to_string(),
        household: params.household,
        brief: params.brief,
        column: Column::Backlog,
        created_at: now,
        updated_at: now,
        needs_heavy: params.needs_heavy,
        parent_id: None,
    };
    let card_for_insert = card.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = db.blocking_lock();
        db::insert_card(&conn, &card_for_insert)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?;
    serde_json::to_value(NewResult { card }).map_err(to_internal)
}

async fn handle_mv(state: DaemonState, params: MvParams) -> std::result::Result<Value, RpcError> {
    let id = params.id.clone();
    let new_column = params.column;
    let now = now_secs();

    // Read the existing card so we know the old column AND the household name.
    let db = state.db.clone();
    let id_for_query = id.clone();
    let existing = tokio::task::spawn_blocking(move || -> Result<Option<Card>> {
        let conn = db.blocking_lock();
        db::get_card(&conn, &id_for_query)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?
    .ok_or_else(|| RpcError {
        code: RpcError::NOT_FOUND,
        message: format!("card not found: {}", params.id),
    })?;
    let old_column = existing.column;

    // Precheck: a move that requires a container must have a configured household.
    // Reject BEFORE the column is mutated so the user can fix their config and retry.
    if new_column == Column::Implementing {
        precheck_household_for_implementing(&state, &existing.household).await?;
    }

    let db = state.db.clone();
    let id_for_update = id.clone();
    let card = tokio::task::spawn_blocking(move || -> Result<Option<Card>> {
        let conn = db.blocking_lock();
        db::update_card_column(&conn, &id_for_update, new_column, now)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?
    .ok_or_else(|| RpcError {
        code: RpcError::NOT_FOUND,
        message: format!("card not found: {}", params.id),
    })?;

    let fx = LiveEffects::new(state);
    transitions::on_column_change(old_column, new_column, &card, &fx)
        .await
        .map_err(|e| RpcError {
            code: RpcError::INTERNAL,
            message: format!("{e:#}"),
        })?;
    serde_json::to_value(MvResult { card }).map_err(to_internal)
}

async fn handle_mv_self(
    state: DaemonState,
    card_id: String,
    params: MvSelfParams,
) -> std::result::Result<Value, RpcError> {
    let new_column = params.column;
    let now = now_secs();

    let db = state.db.clone();
    let id_for_query = card_id.clone();
    let existing = tokio::task::spawn_blocking(move || -> Result<Option<Card>> {
        let conn = db.blocking_lock();
        db::get_card(&conn, &id_for_query)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?
    .ok_or_else(|| RpcError {
        code: RpcError::NOT_FOUND,
        message: format!("card not found: {card_id}"),
    })?;
    let old_column = existing.column;

    if new_column == Column::Implementing {
        precheck_household_for_implementing(&state, &existing.household).await?;
    }

    let db = state.db.clone();
    let id_for_update = card_id.clone();
    let card = tokio::task::spawn_blocking(move || -> Result<Option<Card>> {
        let conn = db.blocking_lock();
        db::update_card_column(&conn, &id_for_update, new_column, now)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?
    .ok_or_else(|| RpcError {
        code: RpcError::NOT_FOUND,
        message: format!("card not found: {card_id}"),
    })?;

    let fx = LiveEffects::new(state);
    transitions::on_column_change(old_column, new_column, &card, &fx)
        .await
        .map_err(|e| RpcError {
            code: RpcError::INTERNAL,
            message: format!("{e:#}"),
        })?;
    serde_json::to_value(MvSelfResult { card }).map_err(to_internal)
}

async fn handle_status_self(db: DbHandle, card_id: String) -> std::result::Result<Value, RpcError> {
    let id = card_id.clone();
    let (card, container) = tokio::task::spawn_blocking(
        move || -> Result<(Option<Card>, Option<db::ContainerRow>)> {
            let conn = db.blocking_lock();
            let card = db::get_card(&conn, &id)?;
            let container = db::get_container(&conn, &id)?;
            Ok((card, container))
        },
    )
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?;
    let card = card.ok_or_else(|| RpcError {
        code: RpcError::NOT_FOUND,
        message: format!("card not found: {card_id}"),
    })?;
    serde_json::to_value(StatusSelfResult {
        card,
        container_status: container.map(|c| c.status),
    })
    .map_err(to_internal)
}

fn default_source_dir() -> PathBuf {
    std::env::var("HOME")
        .map(|h| PathBuf::from(h).join("Source"))
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn summary_from(row: &db::HouseholdRow, m: &manifest::Manifest) -> HouseholdSummary {
    HouseholdSummary {
        name: row.name.clone(),
        meta_repo_path: row.meta_repo_path.clone(),
        workspace_name: m.workspace.clone(),
        knowledge_base: m.knowledge_base.clone(),
        repo_count: m.repos.len(),
    }
}

async fn handle_household_add(
    state: &DaemonState,
    params: HouseholdAddParams,
) -> std::result::Result<Value, RpcError> {
    let meta_repo_path = match &params.source {
        HouseholdSource::Path { path } => {
            if params.clone_into.is_some() {
                return Err(RpcError {
                    code: RpcError::INVALID_PARAMS,
                    message: "--clone-into is only valid for URL sources".into(),
                });
            }
            if !path.exists() {
                return Err(RpcError {
                    code: RpcError::NOT_FOUND,
                    message: format!("path {} does not exist", path.display()),
                });
            }
            if !path.join(".git").exists() {
                return Err(RpcError {
                    code: RpcError::INVALID_ARGUMENT,
                    message: format!("path {} is not a git repository", path.display()),
                });
            }
            path.clone()
        }
        HouseholdSource::Url { url } => {
            let dest = params
                .clone_into
                .clone()
                .unwrap_or_else(|| default_source_dir().join(&params.name));
            if dest.exists() {
                return Err(RpcError {
                    code: RpcError::CONFLICT,
                    message: format!("target directory {} already exists", dest.display()),
                });
            }
            state
                .git
                .clone_url(url, &dest)
                .await
                .map_err(|e| RpcError {
                    code: RpcError::INTERNAL,
                    message: format!("clone {url}: {e:#}"),
                })?;
            // TODO: on manifest or DB-insert failure below, the cloned directory at
            // `dest` is left orphaned (no DB row, no cleanup). Acceptable for M3.5;
            // follow-up issue should add best-effort dest removal on error paths.
            dest
        }
    };

    // Validate the manifest BEFORE inserting into sqlite.
    let m = manifest::load_manifest(&meta_repo_path).map_err(|e| RpcError {
        code: RpcError::INVALID_ARGUMENT,
        message: format!("manifest invalid: {e:#}"),
    })?;

    let added_at = now_secs();
    let row = db::HouseholdRow {
        name: params.name.clone(),
        meta_repo_path: meta_repo_path.clone(),
        added_at,
    };
    let db = state.db.clone();
    let row_clone = row.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = db.blocking_lock();
        db::insert_household(&conn, &row_clone)
    })
    .await
    .map_err(to_internal)?
    .map_err(|e| RpcError {
        code: RpcError::CONFLICT,
        message: format!("insert household: {e:#}"),
    })?;

    serde_json::to_value(summary_from(&row, &m)).map_err(to_internal)
}

async fn git_in(dir: &Path, args: &[&str]) -> anyhow::Result<()> {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .await?;
    if !output.status.success() {
        anyhow::bail!(
            "git {args:?} failed in {} (exit {}): {}",
            dir.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr),
        );
    }
    Ok(())
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

#[derive(serde::Deserialize)]
pub struct NameParam {
    pub name: String,
}

async fn handle_household_ls(state: &DaemonState) -> std::result::Result<Value, RpcError> {
    let conn = state.db.lock().await;
    let rows = db::list_households(&conn).map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("list households: {e:#}"),
    })?;
    drop(conn);
    let mut out: Vec<HouseholdSummary> = Vec::with_capacity(rows.len());
    for row in rows {
        let m =
            manifest::load_manifest(&row.meta_repo_path).unwrap_or_else(|_| manifest::Manifest {
                workspace: row.name.clone(),
                knowledge_base: None,
                repos: vec![],
            });
        out.push(summary_from(&row, &m));
    }
    serde_json::to_value(out).map_err(to_internal)
}

async fn handle_household_rm(
    state: &DaemonState,
    params: NameParam,
) -> std::result::Result<Value, RpcError> {
    let conn = state.db.lock().await;
    let removed = db::remove_household(&conn, &params.name).map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("remove household: {e:#}"),
    })?;
    drop(conn);
    if !removed {
        return Err(RpcError {
            code: RpcError::NOT_FOUND,
            message: format!("household '{}' not found", params.name),
        });
    }
    serde_json::to_value(()).map_err(to_internal)
}

async fn handle_household_show(
    state: &DaemonState,
    params: NameParam,
) -> std::result::Result<Value, RpcError> {
    // First locked section: fetch the row.
    let row = {
        let conn = state.db.lock().await;
        db::get_household(&conn, &params.name)
            .map_err(|e| RpcError {
                code: RpcError::INTERNAL,
                message: format!("get household: {e:#}"),
            })?
            .ok_or_else(|| RpcError {
                code: RpcError::NOT_FOUND,
                message: format!("household '{}' not found", params.name),
            })?
    };

    // Unlocked: file I/O and subprocess work.
    let m = manifest::load_manifest(&row.meta_repo_path).map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("load manifest: {e:#}"),
    })?;

    let workspace_url = m
        .workspace_entry()
        .url
        .clone()
        .or_else(|| git_remote_url(&row.meta_repo_path));

    let repos: Vec<RepoDetail> = m
        .repos
        .iter()
        .map(|entry| {
            let is_workspace = entry.name == m.workspace;
            let is_knowledge_base = m.knowledge_base.as_deref() == Some(entry.name.as_str());
            let on_disk = if is_workspace {
                RepoOnDisk::Workspace
            } else {
                let dir = row.meta_repo_path.join(&entry.name);
                if dir.join(".git").exists() {
                    RepoOnDisk::Sibling
                } else if dir.exists() {
                    RepoOnDisk::Inline
                } else {
                    RepoOnDisk::Missing
                }
            };
            RepoDetail {
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

    // Second locked section: fetch referencing cards.
    let cards = {
        let conn = state.db.lock().await;
        db::cards_for_household(&conn, &params.name).map_err(|e| RpcError {
            code: RpcError::INTERNAL,
            message: format!("list cards: {e:#}"),
        })?
    }
    .into_iter()
    .map(|(id, brief, column)| CardReference { id, brief, column })
    .collect();

    let detail = HouseholdDetail {
        summary: summary_from(&row, &m),
        workspace_url,
        repos,
        cards,
    };
    serde_json::to_value(detail).map_err(to_internal)
}

async fn handle_household_new(
    state: &DaemonState,
    params: HouseholdNewParams,
) -> std::result::Result<Value, RpcError> {
    let dest = params
        .path
        .clone()
        .unwrap_or_else(|| default_source_dir().join(&params.name));

    if dest.exists()
        && dest
            .read_dir()
            .map(|mut i| i.next().is_some())
            .unwrap_or(true)
    {
        return Err(RpcError {
            code: RpcError::CONFLICT,
            message: format!(
                "target directory {} exists and is non-empty",
                dest.display()
            ),
        });
    }

    let template = params
        .template
        .clone()
        .unwrap_or_else(|| DEFAULT_TEMPLATE.into());

    // 1. Shallow-clone the template via URL.
    state
        .git
        .clone_url(&template, &dest)
        .await
        .map_err(|e| RpcError {
            code: RpcError::INTERNAL,
            message: format!("clone template {template}: {e:#}"),
        })?;

    // TODO: from here on, any failure leaves `dest` orphaned (re-init aborted,
    // manifest rewrite failed, git commit failed, or DB insert failed). Mirror
    // the cleanup decision from handle_household_add's URL branch in a follow-up.

    // 2. Detach from the template's history: rm .git, re-init, single commit.
    std::fs::remove_dir_all(dest.join(".git")).map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("remove .git: {e:#}"),
    })?;
    git_in(&dest, &["init", "-b", "main", "--quiet"])
        .await
        .map_err(|e| RpcError {
            code: RpcError::INTERNAL,
            message: format!("git init: {e:#}"),
        })?;

    // 3. Substitute the workspace name in household.json.
    let manifest_path = dest.join("household.json");
    let body = std::fs::read_to_string(&manifest_path).map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("read manifest: {e:#}"),
    })?;
    let mut m: manifest::Manifest = serde_json::from_str(&body).map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("parse template manifest: {e:#}"),
    })?;
    let old_workspace = m.workspace.clone();
    if let Some(entry) = m.repos.iter_mut().find(|r| r.name == old_workspace) {
        entry.name = params.name.clone();
    } else {
        return Err(RpcError {
            code: RpcError::INTERNAL,
            message: format!(
                "template manifest's workspace = '{old_workspace}' has no matching repos[] entry"
            ),
        });
    }
    m.workspace = params.name.clone();
    let body = serde_json::to_string_pretty(&m).map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("re-serialize manifest: {e:#}"),
    })?;
    std::fs::write(&manifest_path, body).map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("write manifest: {e:#}"),
    })?;

    // 4. Initial commit.
    git_in(&dest, &["add", "-A"]).await.map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("git add: {e:#}"),
    })?;
    git_in(&dest, &["commit", "-m", "Initial workspace", "--quiet"])
        .await
        .map_err(|e| RpcError {
            code: RpcError::INTERNAL,
            message: format!("git commit: {e:#}"),
        })?;

    // 5. Register in the database.
    let added_at = now_secs();
    let row = db::HouseholdRow {
        name: params.name.clone(),
        meta_repo_path: dest.clone(),
        added_at,
    };
    let db = state.db.clone();
    let row_clone = row.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = db.blocking_lock();
        db::insert_household(&conn, &row_clone)
    })
    .await
    .map_err(to_internal)?
    .map_err(|e| RpcError {
        code: RpcError::CONFLICT,
        message: format!("insert household: {e:#}"),
    })?;

    serde_json::to_value(summary_from(&row, &m)).map_err(to_internal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::{ContainerOps, ContainerOpsHandle};
    use crate::effects::DaemonState;
    use crate::listeners::{Dispatcher, ListenerRegistry};
    use anyhow::Result;
    use async_trait::async_trait;
    use std::path::PathBuf;
    use std::pin::Pin;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    use crate::clones::{GitOps, GitOpsHandle};

    struct NoopGit;
    #[async_trait]
    impl GitOps for NoopGit {
        async fn clone(&self, _source: &std::path::Path, _dest: &std::path::Path) -> Result<()> {
            std::fs::create_dir_all(_dest).ok();
            Ok(())
        }
        async fn clone_url(&self, _url: &str, _dest: &std::path::Path) -> Result<()> {
            std::fs::create_dir_all(_dest).ok();
            Ok(())
        }
        async fn fetch(
            &self,
            _workdir: &std::path::Path,
            _source: &std::path::Path,
            _namespace: &str,
        ) -> Result<()> {
            Ok(())
        }
        async fn list_namespace_refs(
            &self,
            _workdir: &std::path::Path,
            _namespace: &str,
        ) -> Result<Vec<String>> {
            Ok(vec![])
        }
        async fn fast_forward(
            &self,
            _workdir: &std::path::Path,
            _target_ref: &str,
            _source_ref: &str,
        ) -> Result<crate::clones::FfOutcome> {
            Ok(crate::clones::FfOutcome::Updated)
        }
    }

    /// No-op container backend that pretends nothing exists.
    struct NoopOps;
    #[async_trait]
    impl ContainerOps for NoopOps {
        async fn spawn(&self, _argv: Vec<String>) -> Result<()> {
            Ok(())
        }
        async fn teardown(&self, _name: &str) -> Result<()> {
            Ok(())
        }
        async fn inspect_status(&self, _name: &str) -> Result<Option<String>> {
            Ok(None)
        }
        async fn discover_container_name(&self, _card_id: &str) -> Result<Option<String>> {
            Ok(Some("test-container".to_string()))
        }
    }

    /// Dispatcher stub; transitions don't need to call back here in tests.
    struct NoopDispatcher;
    impl Dispatcher for NoopDispatcher {
        fn dispatch(
            &self,
            _r: RpcRequest,
            _d: DbHandle,
            _c: CallerContext,
        ) -> Pin<
            Box<
                dyn std::future::Future<Output = std::result::Result<serde_json::Value, RpcError>>
                    + Send,
            >,
        > {
            Box::pin(async { Ok(serde_json::Value::Null) })
        }
    }

    fn fresh_state() -> DaemonState {
        let conn = db::open(":memory:").unwrap();
        let db: DbHandle = Arc::new(Mutex::new(conn));
        DaemonState {
            db: db.clone(),
            containers: Arc::new(NoopOps) as ContainerOpsHandle,
            git: Arc::new(NoopGit) as GitOpsHandle,
            listeners: ListenerRegistry::new(db, Arc::new(NoopDispatcher)),
            claude_dir: PathBuf::from("/tmp/nonexistent-test-claude-dir"),
            api_key: None,
        }
    }

    fn fresh_state_with_git(git: GitOpsHandle) -> DaemonState {
        let conn = db::open(":memory:").unwrap();
        let db: DbHandle = Arc::new(Mutex::new(conn));
        DaemonState {
            db: db.clone(),
            containers: Arc::new(NoopOps) as ContainerOpsHandle,
            git,
            listeners: ListenerRegistry::new(db, Arc::new(NoopDispatcher)),
            claude_dir: PathBuf::from("/tmp/nonexistent-test-claude-dir"),
            api_key: None,
        }
    }

    #[tokio::test]
    async fn ls_on_empty_returns_empty_list() {
        let state = fresh_state();
        let req = RpcRequest {
            id: 1,
            method: "ls".into(),
            params: serde_json::json!({}),
        };
        let value = dispatch(req, state, CallerContext::host()).await.unwrap();
        let result: LsResult = serde_json::from_value(value).unwrap();
        assert!(result.cards.is_empty());
    }

    #[tokio::test]
    async fn new_creates_card_then_ls_returns_it() {
        let state = fresh_state();
        let req_new = RpcRequest {
            id: 1,
            method: "new".into(),
            params: serde_json::json!({
                "household": "myarchy",
                "brief": "first card",
                "needs_heavy": false,
            }),
        };
        let new_value = dispatch(req_new, state.clone(), CallerContext::host())
            .await
            .unwrap();
        let new_result: NewResult = serde_json::from_value(new_value).unwrap();
        assert_eq!(new_result.card.brief, "first card");
        assert_eq!(new_result.card.column, Column::Backlog);

        let req_ls = RpcRequest {
            id: 2,
            method: "ls".into(),
            params: serde_json::json!({}),
        };
        let ls_value = dispatch(req_ls, state, CallerContext::host())
            .await
            .unwrap();
        let ls_result: LsResult = serde_json::from_value(ls_value).unwrap();
        assert_eq!(ls_result.cards.len(), 1);
        assert_eq!(ls_result.cards[0], new_result.card);
    }

    #[tokio::test]
    async fn mv_changes_column_and_returns_updated() {
        let state = fresh_state();
        let new_value = dispatch(
            RpcRequest {
                id: 1,
                method: "new".into(),
                params: serde_json::json!({
                    "household": "myarchy",
                    "brief": "x",
                    "needs_heavy": false,
                }),
            },
            state.clone(),
            CallerContext::host(),
        )
        .await
        .unwrap();
        let card = serde_json::from_value::<NewResult>(new_value).unwrap().card;

        let mv_value = dispatch(
            RpcRequest {
                id: 2,
                method: "mv".into(),
                params: serde_json::json!({
                    "id": card.id,
                    "column": "review",
                }),
            },
            state,
            CallerContext::host(),
        )
        .await
        .unwrap();
        let updated = serde_json::from_value::<MvResult>(mv_value).unwrap().card;
        assert_eq!(updated.column, Column::Review);
    }

    #[tokio::test]
    async fn mv_unknown_id_returns_not_found() {
        let state = fresh_state();
        let err = dispatch(
            RpcRequest {
                id: 1,
                method: "mv".into(),
                params: serde_json::json!({
                    "id": "missing",
                    "column": "done",
                }),
            },
            state,
            CallerContext::host(),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, RpcError::NOT_FOUND);
    }

    #[tokio::test]
    async fn unknown_method_returns_method_not_found() {
        let state = fresh_state();
        let err = dispatch(
            RpcRequest {
                id: 1,
                method: "nope".into(),
                params: serde_json::json!({}),
            },
            state,
            CallerContext::host(),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, RpcError::METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn mv_self_on_host_socket_returns_forbidden() {
        let state = fresh_state();
        let req = RpcRequest {
            id: 1,
            method: "mv-self".into(),
            params: serde_json::json!({"column": "review"}),
        };
        let err = dispatch(req, state, CallerContext::host())
            .await
            .unwrap_err();
        assert_eq!(err.code, RpcError::FORBIDDEN);
    }

    #[tokio::test]
    async fn mv_self_on_card_socket_moves_card() {
        let state = fresh_state();
        let new_value = dispatch(
            RpcRequest {
                id: 1,
                method: "new".into(),
                params: serde_json::json!({
                    "household": "h",
                    "brief": "b",
                    "needs_heavy": false,
                }),
            },
            state.clone(),
            CallerContext::host(),
        )
        .await
        .unwrap();
        let card = serde_json::from_value::<NewResult>(new_value).unwrap().card;

        let mv_value = dispatch(
            RpcRequest {
                id: 2,
                method: "mv-self".into(),
                params: serde_json::json!({"column": "review"}),
            },
            state,
            CallerContext::card(&card.id),
        )
        .await
        .unwrap();
        let result: MvSelfResult = serde_json::from_value(mv_value).unwrap();
        assert_eq!(result.card.column, Column::Review);
        assert_eq!(result.card.id, card.id);
    }

    #[tokio::test]
    async fn status_self_on_host_socket_returns_forbidden() {
        let state = fresh_state();
        let req = RpcRequest {
            id: 1,
            method: "status-self".into(),
            params: serde_json::json!({}),
        };
        let err = dispatch(req, state, CallerContext::host())
            .await
            .unwrap_err();
        assert_eq!(err.code, RpcError::FORBIDDEN);
    }

    #[tokio::test]
    async fn status_self_on_card_socket_returns_card_no_container() {
        let state = fresh_state();
        let new_value = dispatch(
            RpcRequest {
                id: 1,
                method: "new".into(),
                params: serde_json::json!({
                    "household": "h",
                    "brief": "b",
                    "needs_heavy": false,
                }),
            },
            state.clone(),
            CallerContext::host(),
        )
        .await
        .unwrap();
        let card = serde_json::from_value::<NewResult>(new_value).unwrap().card;

        let value = dispatch(
            RpcRequest {
                id: 2,
                method: "status-self".into(),
                params: serde_json::json!({}),
            },
            state,
            CallerContext::card(&card.id),
        )
        .await
        .unwrap();
        let result: StatusSelfResult = serde_json::from_value(value).unwrap();
        assert_eq!(result.card.id, card.id);
        assert!(result.container_status.is_none());
    }

    #[tokio::test]
    async fn mv_to_implementing_without_household_leaves_column_unchanged() {
        let state = fresh_state();

        let new_value = dispatch(
            RpcRequest {
                id: 1,
                method: "new".into(),
                params: serde_json::json!({
                    "household": "h",
                    "brief": "x",
                    "needs_heavy": false,
                }),
            },
            state.clone(),
            CallerContext::host(),
        )
        .await
        .unwrap();
        let card = serde_json::from_value::<NewResult>(new_value).unwrap().card;

        let err = dispatch(
            RpcRequest {
                id: 2,
                method: "mv".into(),
                params: serde_json::json!({"id": card.id, "column": "implementing"}),
            },
            state.clone(),
            CallerContext::host(),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, RpcError::INTERNAL);
        assert!(err.message.contains("household 'h' not configured"));

        let ls_value = dispatch(
            RpcRequest {
                id: 3,
                method: "ls".into(),
                params: serde_json::json!({}),
            },
            state,
            CallerContext::host(),
        )
        .await
        .unwrap();
        let ls: LsResult = serde_json::from_value(ls_value).unwrap();
        assert_eq!(ls.cards[0].column, Column::Backlog);
    }

    #[tokio::test]
    async fn household_add_via_path_succeeds() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let meta = dir.path().join("ws");
        std::fs::create_dir_all(meta.join(".git")).unwrap();
        std::fs::write(
            meta.join("household.json"),
            r#"{ "workspace": "ws", "repos": [{ "name": "ws" }] }"#,
        )
        .unwrap();
        let state = fresh_state();
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
        let summary: HouseholdSummary = serde_json::from_value(result).unwrap();
        assert_eq!(summary.name, "test");
        assert_eq!(summary.meta_repo_path, meta);
    }

    #[tokio::test]
    async fn household_add_rejects_invalid_manifest() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let meta = dir.path().join("ws");
        std::fs::create_dir_all(meta.join(".git")).unwrap();
        std::fs::write(
            meta.join("household.json"),
            r#"{ "workspace": "ws", "knowledge_base": "missing", "repos": [{ "name": "ws" }] }"#,
        )
        .unwrap();
        let state = fresh_state();
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
        assert!(err.message.contains("manifest invalid"));
    }

    #[tokio::test]
    async fn household_new_with_explicit_template() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let template = dir.path().join("template");
        std::fs::create_dir_all(&template).unwrap();
        // Initialise a real git repo with one commit.
        std::process::Command::new("git")
            .args(["init", "-b", "main", "--quiet"])
            .current_dir(&template)
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "t@t.com"])
            .current_dir(&template)
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "t"])
            .current_dir(&template)
            .status()
            .unwrap();
        std::fs::write(
            template.join("household.json"),
            r#"{ "workspace": "default", "repos": [{ "name": "default" }] }"#,
        )
        .unwrap();
        std::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(&template)
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "init", "--quiet"])
            .current_dir(&template)
            .status()
            .unwrap();

        let dest = dir.path().join("my-ws");
        let state = fresh_state_with_git(Arc::new(crate::clones::RealGitOps) as GitOpsHandle);
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

        let summary: HouseholdSummary = serde_json::from_value(result).unwrap();
        assert_eq!(
            summary.workspace_name, "my-ws",
            "manifest workspace pointer rewritten"
        );
        assert!(dest.join(".git").exists(), "fresh git history");
        assert!(dest.join("household.json").exists());
        let manifest_body = std::fs::read_to_string(dest.join("household.json")).unwrap();
        assert!(
            manifest_body.contains("\"workspace\": \"my-ws\""),
            "workspace pointer substituted: {manifest_body}"
        );
        assert!(
            manifest_body.contains("\"name\": \"my-ws\""),
            "matching repos[] entry renamed: {manifest_body}"
        );
    }

    #[tokio::test]
    async fn household_ls_returns_inserted_households() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let state = fresh_state();
        // Insert two via add.
        for n in ["alpha", "beta"] {
            let meta = dir.path().join(n);
            std::fs::create_dir_all(meta.join(".git")).unwrap();
            std::fs::write(
                meta.join("household.json"),
                format!(r#"{{ "workspace": "{n}", "repos": [{{ "name": "{n}" }}] }}"#),
            )
            .unwrap();
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
        let list_val = handle_household_ls(&state).await.unwrap();
        let list: Vec<HouseholdSummary> = serde_json::from_value(list_val).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "alpha");
        assert_eq!(list[1].name, "beta");
    }

    #[tokio::test]
    async fn household_rm_returns_not_found_for_unknown() {
        let state = fresh_state();
        let err = handle_household_rm(
            &state,
            NameParam {
                name: "nope".into(),
            },
        )
        .await
        .unwrap_err();
        assert!(format!("{err:?}").contains("not found"));
    }

    #[tokio::test]
    async fn household_show_classifies_each_entry() {
        use reeve_protocol::RepoOnDisk;
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let meta = dir.path().join("ws");
        std::fs::create_dir_all(meta.join(".git")).unwrap();
        std::fs::create_dir_all(meta.join("backend/.git")).unwrap(); // separate sibling, present
        std::fs::create_dir_all(meta.join("lore")).unwrap(); // inline (no .git/)
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
        )
        .unwrap();
        let state = fresh_state();
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
        let detail_val = handle_household_show(
            &state,
            NameParam {
                name: "test".into(),
            },
        )
        .await
        .unwrap();
        let detail: HouseholdDetail = serde_json::from_value(detail_val).unwrap();
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
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let meta = dir.path().join("ws");
        std::fs::create_dir_all(meta.join(".git")).unwrap();
        std::fs::write(
            meta.join("household.json"),
            r#"{ "workspace": "ours", "repos": [{ "name": "ours" }] }"#,
        )
        .unwrap();
        let state = fresh_state();
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
        // Insert cards directly via db.
        let make_card = |id: &str, brief: &str, column: Column, household: &str| Card {
            id: id.into(),
            household: household.into(),
            brief: brief.into(),
            column,
            created_at: 0,
            updated_at: 0,
            needs_heavy: false,
            parent_id: None,
        };
        {
            let conn = state.db.lock().await;
            db::insert_card(
                &conn,
                &make_card("card-1", "brief one", Column::Backlog, "ours"),
            )
            .unwrap();
            db::insert_card(
                &conn,
                &make_card("card-2", "brief two", Column::Implementing, "ours"),
            )
            .unwrap();
            db::insert_card(
                &conn,
                &make_card("card-3", "brief three", Column::Backlog, "someone-else"),
            )
            .unwrap();
        }
        let detail_val = handle_household_show(
            &state,
            NameParam {
                name: "ours".into(),
            },
        )
        .await
        .unwrap();
        let detail: HouseholdDetail = serde_json::from_value(detail_val).unwrap();
        let ids: Vec<&str> = detail.cards.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["card-1", "card-2"]);
    }

    #[tokio::test]
    async fn household_new_refuses_non_empty_target() {
        // `template` is deliberately a bad URL; the guard returns before we touch git.
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let dest = dir.path().join("occupied");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("something"), "x").unwrap();
        let state = fresh_state();
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
}
