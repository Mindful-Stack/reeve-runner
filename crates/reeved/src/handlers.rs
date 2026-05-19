//! Method dispatch and per-method handlers.

use crate::db;
use crate::effects::DaemonState;
use crate::stubs::StubWorkspaceLifecycle;
use crate::transport::server::CallerContext;
use anyhow::Result;
use reeve_card::{make_card_repo, WorkspaceLifecycle};
use reeve_protocol::{
    AttachParams, AttachResult, BranchPushResult, BranchPushStatus, Card, Column,
    MvParams, MvSelfParams, NewParams, PushParams, PushResult, RpcError,
    RpcRequest,
};
use serde_json::Value;
use std::sync::Arc;

pub async fn dispatch(
    req: RpcRequest,
    state: DaemonState,
    caller: CallerContext,
) -> std::result::Result<Value, RpcError> {
    match req.method.as_str() {
        "ls" => {
            let cards = make_card_repo(state.db.clone());
            reeve_card::handle_ls(&*cards).await
        }
        "new" => {
            let cards = make_card_repo(state.db.clone());
            let params = parse_params::<NewParams>(req.params)?;
            reeve_card::handle_new(&*cards, params).await
        }
        "mv" => {
            let cards = make_card_repo(state.db.clone());
            let households = state.household_lookup.clone();
            let workspace = stub_workspace_lifecycle(state.clone());
            let params = parse_params::<MvParams>(req.params)?;
            reeve_card::handle_mv(&*cards, &*households, &*workspace, params).await
        }
        "attach" => {
            let params = parse_params::<AttachParams>(req.params)?;
            handle_attach(&state, params).await
        }
        "mv-self" => {
            let card_id = require_card(&caller)?;
            let cards = make_card_repo(state.db.clone());
            let households = state.household_lookup.clone();
            let workspace = stub_workspace_lifecycle(state.clone());
            let params = parse_params::<MvSelfParams>(req.params)?;
            reeve_card::handle_mv_self(&*cards, &*households, &*workspace, card_id, params).await
        }
        "push" => {
            let params = parse_params::<PushParams>(req.params)?;
            handle_push(&state, params).await
        }
        "status-self" => {
            let card_id = require_card(&caller)?;
            let cards = make_card_repo(state.db.clone());
            let container_status = fetch_container_status(&state, &card_id).await?;
            reeve_card::handle_status_self(&*cards, container_status, card_id).await
        }
        "household.add" => {
            let scaffold = reeve_household::make_scaffold();
            let params = parse_params::<reeve_protocol::HouseholdAddParams>(req.params)?;
            reeve_household::handle_household_add(
                &*state.household_repo,
                &*scaffold,
                params,
            ).await
        }
        "household.new" => {
            let scaffold = reeve_household::make_scaffold();
            let params = parse_params::<reeve_protocol::HouseholdNewParams>(req.params)?;
            reeve_household::handle_household_new(
                &*state.household_repo,
                &*scaffold,
                params,
            ).await
        }
        "household.ls" => {
            reeve_household::handle_household_ls(&*state.household_repo).await
        }
        "household.rm" => {
            let params = parse_params::<reeve_household::NameParam>(req.params)?;
            reeve_household::handle_household_rm(&*state.household_repo, params).await
        }
        "household.show" => {
            let params = parse_params::<reeve_household::NameParam>(req.params)?;
            // pre-resolve referencing cards via CardRepo
            let cards_repo = reeve_card::make_card_repo(state.db.clone());
            let cards = cards_repo.for_household(&params.name).await.map_err(|e| reeve_protocol::RpcError {
                code: reeve_protocol::RpcError::INTERNAL,
                message: e.to_string(),
            })?;
            reeve_household::handle_household_show(&*state.household_repo, cards, params).await
        }
        other => Err(RpcError {
            code: RpcError::METHOD_NOT_FOUND,
            message: format!("method not found: {other}"),
        }),
    }
}

fn stub_workspace_lifecycle(
    state: crate::effects::DaemonState,
) -> Arc<dyn WorkspaceLifecycle> {
    Arc::new(StubWorkspaceLifecycle { state })
}

async fn fetch_container_status(
    state: &crate::effects::DaemonState,
    card_id: &str,
) -> std::result::Result<Option<reeve_protocol::ContainerStatus>, reeve_protocol::RpcError> {
    let db = state.db.clone();
    let id = card_id.to_string();
    let row = tokio::task::spawn_blocking(move || -> anyhow::Result<Option<crate::db::ContainerRow>> {
        let conn = db.blocking_lock();
        crate::db::get_container(&conn, &id)
    })
    .await
    .map_err(|e| reeve_protocol::RpcError {
        code: reeve_protocol::RpcError::INTERNAL,
        message: e.to_string(),
    })?
    .map_err(|e| reeve_protocol::RpcError {
        code: reeve_protocol::RpcError::INTERNAL,
        message: e.to_string(),
    })?;
    Ok(row.map(|r| r.status))
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

fn to_internal<E: std::fmt::Display>(e: E) -> RpcError {
    RpcError {
        code: RpcError::INTERNAL,
        message: e.to_string(),
    }
}

async fn handle_push(
    state: &DaemonState,
    params: PushParams,
) -> std::result::Result<Value, RpcError> {
    // 1. Fetch the card from the DB.
    let card_id = params.id.clone();
    let db = state.db.clone();
    let card = tokio::task::spawn_blocking(move || -> Result<Option<Card>> {
        let conn = db.blocking_lock();
        crate::db::get_card(&conn, &card_id)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?
    .ok_or_else(|| RpcError {
        code: RpcError::NOT_FOUND,
        message: format!("card '{}' not found", params.id),
    })?;

    // 2. Resolve the household from the household_repo.
    let row = state.household_repo.get(&card.household).await
        .map_err(to_internal)?
        .ok_or_else(|| RpcError {
            code: RpcError::INTERNAL,
            message: format!("household '{}' not configured", card.household),
        })?;
    let household = reeve_household::Household {
        name: row.name,
        meta_repo_path: row.meta_repo_path,
    };

    // 3. Load + validate the manifest.
    let manifest = reeve_household::load_household_manifest(&household).map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("load manifest: {e:#}"),
    })?;

    // 4. For each repo, for each branch in the namespace, FF + push.
    let mut branches = Vec::new();
    for (repo_name, host_path) in reeve_household::iter_card_repos(&household, &manifest) {
        let listed = state
            .git
            .list_namespace_refs(&host_path, &card.id)
            .await
            .map_err(|e| RpcError {
                code: RpcError::INTERNAL,
                message: format!(
                    "list refs/reeve/{}/* in {}: {e:#}",
                    card.id,
                    host_path.display()
                ),
            })?;
        for branch in listed {
            let target = format!("refs/heads/{branch}");
            let source = format!("refs/reeve/{}/{}", card.id, branch);
            let status = match state.git.fast_forward(&host_path, &target, &source).await {
                Ok(crate::clones::FfOutcome::Updated) => {
                    match state.git.push(&host_path, &branch).await {
                        Ok(()) => BranchPushStatus::Pushed,
                        Err(e) => BranchPushStatus::PushFailed {
                            message: format!("{e:#}"),
                        },
                    }
                }
                Ok(crate::clones::FfOutcome::NotFastForward) => BranchPushStatus::NotFastForward,
                Err(e) => BranchPushStatus::PushFailed {
                    message: format!("fast-forward check failed: {e:#}"),
                },
            };
            branches.push(BranchPushResult {
                repo: repo_name.to_string(),
                branch,
                status,
            });
        }
    }
    serde_json::to_value(PushResult { branches }).map_err(to_internal)
}

async fn handle_attach(state: &DaemonState, params: AttachParams) -> Result<Value, RpcError> {
    // 1. Fetch the card from the DB.
    let card_id = params.id.clone();
    let db = state.db.clone();
    let card = tokio::task::spawn_blocking(move || -> Result<Option<Card>> {
        let conn = db.blocking_lock();
        crate::db::get_card(&conn, &card_id)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?
    .ok_or_else(|| RpcError {
        code: RpcError::NOT_FOUND,
        message: format!("card '{}' not found", params.id),
    })?;

    // 2. Column check.
    if card.column != Column::Implementing {
        return Err(RpcError {
            code: RpcError::INVALID_ARGUMENT,
            message: format!(
                "card is in {}; attach only works in Implementing",
                card.column.label()
            ),
        });
    }

    // 3. Container discovery by label.
    let container_name = state
        .containers
        .discover_container_name(&card.id)
        .await
        .map_err(|e| RpcError {
            code: RpcError::INTERNAL,
            message: format!("container discovery failed: {e:#}"),
        })?
        .ok_or_else(|| RpcError {
            code: RpcError::INTERNAL,
            message: format!(
                "container for card '{}' not running (try mv {} implementing again)",
                card.id, card.id
            ),
        })?;

    // 4. Return the address.
    let result = AttachResult {
        container_name,
        session_name: "reeve".to_string(),
        in_container_workspace: format!("/workspaces/{}", card.id),
    };
    serde_json::to_value(result).map_err(to_internal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::{ContainerOps, ContainerOpsHandle};
    use crate::effects::DaemonState;
    use crate::transport::listener::{Dispatcher, ListenerRegistry};
    use crate::transport::server::DbHandle;
    use anyhow::Result;
    use async_trait::async_trait;
    use reeve_protocol::{LsResult, MvResult, MvSelfResult, NewResult, StatusSelfResult};
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
        async fn push(&self, _workdir: &std::path::Path, _branch: &str) -> Result<()> {
            Ok(())
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
        async fn exec_in_tmux(
            &self,
            _container_name: &str,
            _session_name: &str,
            _command: &str,
        ) -> Result<()> {
            Ok(())
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
        let (household_repo, household_lookup) =
            reeve_household::make_household_components(db.clone());
        DaemonState {
            db: db.clone(),
            containers: Arc::new(NoopOps) as ContainerOpsHandle,
            git: Arc::new(NoopGit) as GitOpsHandle,
            listeners: ListenerRegistry::new(db, Arc::new(NoopDispatcher)),
            claude_dir: PathBuf::from("/tmp/nonexistent-test-claude-dir"),
            api_key: None,
            household_repo,
            household_lookup,
        }
    }

    fn fresh_state_with_git(git: GitOpsHandle) -> DaemonState {
        let conn = db::open(":memory:").unwrap();
        let db: DbHandle = Arc::new(Mutex::new(conn));
        let (household_repo, household_lookup) =
            reeve_household::make_household_components(db.clone());
        DaemonState {
            db: db.clone(),
            containers: Arc::new(NoopOps) as ContainerOpsHandle,
            git,
            listeners: ListenerRegistry::new(db, Arc::new(NoopDispatcher)),
            claude_dir: PathBuf::from("/tmp/nonexistent-test-claude-dir"),
            api_key: None,
            household_repo,
            household_lookup,
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
        let result = dispatch(
            RpcRequest {
                id: 1,
                method: "household.add".into(),
                params: serde_json::json!({
                    "name": "test",
                    "source": { "kind": "path", "path": meta.to_str().unwrap() },
                }),
            },
            state,
            CallerContext::host(),
        )
        .await
        .unwrap();
        let summary: reeve_protocol::HouseholdSummary = serde_json::from_value(result).unwrap();
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
        let err = dispatch(
            RpcRequest {
                id: 1,
                method: "household.add".into(),
                params: serde_json::json!({
                    "name": "test",
                    "source": { "kind": "path", "path": meta.to_str().unwrap() },
                }),
            },
            state,
            CallerContext::host(),
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
        let result = dispatch(
            RpcRequest {
                id: 1,
                method: "household.new".into(),
                params: serde_json::json!({
                    "name": "my-ws",
                    "path": dest.to_str().unwrap(),
                    "template": format!("file://{}", template.display()),
                }),
            },
            state,
            CallerContext::host(),
        )
        .await
        .unwrap();

        let summary: reeve_protocol::HouseholdSummary = serde_json::from_value(result).unwrap();
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
        // Insert two via add dispatch.
        for n in ["alpha", "beta"] {
            let meta = dir.path().join(n);
            std::fs::create_dir_all(meta.join(".git")).unwrap();
            std::fs::write(
                meta.join("household.json"),
                format!(r#"{{ "workspace": "{n}", "repos": [{{ "name": "{n}" }}] }}"#),
            )
            .unwrap();
            dispatch(
                RpcRequest {
                    id: 1,
                    method: "household.add".into(),
                    params: serde_json::json!({
                        "name": n,
                        "source": { "kind": "path", "path": meta.to_str().unwrap() },
                    }),
                },
                state.clone(),
                CallerContext::host(),
            )
            .await
            .unwrap();
        }
        let list_val = dispatch(
            RpcRequest {
                id: 2,
                method: "household.ls".into(),
                params: serde_json::json!({}),
            },
            state,
            CallerContext::host(),
        )
        .await
        .unwrap();
        let list: Vec<reeve_protocol::HouseholdSummary> = serde_json::from_value(list_val).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "alpha");
        assert_eq!(list[1].name, "beta");
    }

    #[tokio::test]
    async fn household_rm_returns_not_found_for_unknown() {
        let state = fresh_state();
        let err = dispatch(
            RpcRequest {
                id: 1,
                method: "household.rm".into(),
                params: serde_json::json!({ "name": "nope" }),
            },
            state,
            CallerContext::host(),
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
        dispatch(
            RpcRequest {
                id: 1,
                method: "household.add".into(),
                params: serde_json::json!({
                    "name": "test",
                    "source": { "kind": "path", "path": meta.to_str().unwrap() },
                }),
            },
            state.clone(),
            CallerContext::host(),
        )
        .await
        .unwrap();
        let detail_val = dispatch(
            RpcRequest {
                id: 2,
                method: "household.show".into(),
                params: serde_json::json!({ "name": "test" }),
            },
            state,
            CallerContext::host(),
        )
        .await
        .unwrap();
        let detail: reeve_protocol::HouseholdDetail = serde_json::from_value(detail_val).unwrap();
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
        dispatch(
            RpcRequest {
                id: 1,
                method: "household.add".into(),
                params: serde_json::json!({
                    "name": "ours",
                    "source": { "kind": "path", "path": meta.to_str().unwrap() },
                }),
            },
            state.clone(),
            CallerContext::host(),
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
        let detail_val = dispatch(
            RpcRequest {
                id: 2,
                method: "household.show".into(),
                params: serde_json::json!({ "name": "ours" }),
            },
            state,
            CallerContext::host(),
        )
        .await
        .unwrap();
        let detail: reeve_protocol::HouseholdDetail = serde_json::from_value(detail_val).unwrap();
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
        let err = dispatch(
            RpcRequest {
                id: 1,
                method: "household.new".into(),
                params: serde_json::json!({
                    "name": "occupied",
                    "path": dest.to_str().unwrap(),
                    "template": "file:///nonexistent",
                }),
            },
            state,
            CallerContext::host(),
        )
        .await
        .unwrap_err();
        assert!(format!("{err:?}").contains("exists and is non-empty"));
    }

    use reeve_protocol::{BranchPushStatus, PushResult};

    async fn insert_test_household(
        state: &DaemonState,
        name: &str,
        meta_repo_path: &std::path::Path,
    ) {
        state.household_repo.insert(reeve_household::HouseholdRow {
            name: name.to_string(),
            meta_repo_path: meta_repo_path.to_path_buf(),
            added_at: 0,
        }).await.unwrap();
    }

    /// Write a minimal household.json at `meta_repo_path`.
    fn write_test_manifest(meta_repo_path: &std::path::Path, workspace: &str, repos: &[&str]) {
        let repos_json: Vec<String> = repos
            .iter()
            .map(|n| format!(r#"{{"name":"{n}"}}"#))
            .collect();
        let body = format!(
            r#"{{"workspace":"{workspace}","repos":[{}]}}"#,
            repos_json.join(",")
        );
        std::fs::write(meta_repo_path.join("household.json"), body).unwrap();
    }

    #[tokio::test]
    async fn push_with_no_branches_returns_empty_result() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join("ws");
        std::fs::create_dir_all(&meta).unwrap();
        write_test_manifest(&meta, "ws", &["ws"]);

        let state = fresh_state();
        insert_test_household(&state, "h", &meta).await;

        // Insert a card pointing at the household.
        {
            let conn = state.db.lock().await;
            crate::db::insert_card(
                &conn,
                &reeve_protocol::Card {
                    id: "card-x".into(),
                    household: "h".into(),
                    brief: "x".into(),
                    column: reeve_protocol::Column::Implementing,
                    created_at: 0,
                    updated_at: 0,
                    needs_heavy: false,
                    parent_id: None,
                },
            )
            .unwrap();
        }

        let req = RpcRequest {
            id: 1,
            method: "push".into(),
            params: serde_json::json!({ "id": "card-x" }),
        };
        let value = dispatch(req, state, CallerContext::host()).await.unwrap();
        let result: PushResult = serde_json::from_value(value).unwrap();
        assert!(result.branches.is_empty());
    }

    /// Test mock that returns configured branches from list_namespace_refs and
    /// configured FfOutcome per call. Used to exercise handle_push's logic
    /// without a real git.
    struct ConfigurableGit {
        branches_per_repo:
            std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, Vec<String>>>,
        ff_outcomes: std::sync::Mutex<Vec<crate::clones::FfOutcome>>,
        push_failures: std::sync::Mutex<std::collections::HashMap<String, String>>,
        push_call_order: std::sync::Mutex<Vec<(std::path::PathBuf, String)>>,
    }

    impl ConfigurableGit {
        fn new() -> Self {
            Self {
                branches_per_repo: std::sync::Mutex::new(std::collections::HashMap::new()),
                ff_outcomes: std::sync::Mutex::new(vec![]),
                push_failures: std::sync::Mutex::new(std::collections::HashMap::new()),
                push_call_order: std::sync::Mutex::new(vec![]),
            }
        }
        fn set_branches(&self, repo: std::path::PathBuf, branches: Vec<String>) {
            self.branches_per_repo
                .lock()
                .unwrap()
                .insert(repo, branches);
        }
        fn queue_ff_outcome(&self, outcome: crate::clones::FfOutcome) {
            self.ff_outcomes.lock().unwrap().push(outcome);
        }
        #[allow(dead_code)]
        fn fail_push(&self, branch: &str, message: &str) {
            self.push_failures
                .lock()
                .unwrap()
                .insert(branch.to_string(), message.to_string());
        }
    }

    #[async_trait::async_trait]
    impl crate::clones::GitOps for ConfigurableGit {
        async fn clone(&self, _s: &std::path::Path, _d: &std::path::Path) -> Result<()> {
            Ok(())
        }
        async fn clone_url(&self, _u: &str, _d: &std::path::Path) -> Result<()> {
            Ok(())
        }
        async fn fetch(&self, _w: &std::path::Path, _s: &std::path::Path, _n: &str) -> Result<()> {
            Ok(())
        }
        async fn list_namespace_refs(
            &self,
            workdir: &std::path::Path,
            _namespace: &str,
        ) -> Result<Vec<String>> {
            Ok(self
                .branches_per_repo
                .lock()
                .unwrap()
                .get(workdir)
                .cloned()
                .unwrap_or_default())
        }
        async fn fast_forward(
            &self,
            _workdir: &std::path::Path,
            _target: &str,
            _source: &str,
        ) -> Result<crate::clones::FfOutcome> {
            // Pop in order; default to Updated if queue empty.
            let mut q = self.ff_outcomes.lock().unwrap();
            Ok(if q.is_empty() {
                crate::clones::FfOutcome::Updated
            } else {
                q.remove(0)
            })
        }
        async fn push(&self, workdir: &std::path::Path, branch: &str) -> Result<()> {
            self.push_call_order
                .lock()
                .unwrap()
                .push((workdir.to_path_buf(), branch.to_string()));
            if let Some(msg) = self.push_failures.lock().unwrap().get(branch) {
                anyhow::bail!("{msg}");
            }
            Ok(())
        }
    }

    fn fresh_state_with_configurable_git(git: std::sync::Arc<ConfigurableGit>) -> DaemonState {
        let conn = crate::db::open(":memory:").unwrap();
        let db: crate::transport::server::DbHandle = std::sync::Arc::new(tokio::sync::Mutex::new(conn));
        let (household_repo, household_lookup) =
            reeve_household::make_household_components(db.clone());
        DaemonState {
            db: db.clone(),
            containers: std::sync::Arc::new(NoopOps) as crate::container::ContainerOpsHandle,
            git: git as crate::clones::GitOpsHandle,
            listeners: crate::transport::listener::ListenerRegistry::new(
                db,
                std::sync::Arc::new(NoopDispatcher),
            ),
            claude_dir: std::path::PathBuf::from("/tmp/nonexistent-test-claude-dir"),
            api_key: None,
            household_repo,
            household_lookup,
        }
    }

    #[tokio::test]
    async fn push_records_per_branch_status_pushed_and_not_ff() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join("ws");
        std::fs::create_dir_all(&meta).unwrap();
        write_test_manifest(&meta, "ws", &["ws"]);

        let git = std::sync::Arc::new(ConfigurableGit::new());
        git.set_branches(meta.clone(), vec!["feat-a".into(), "feat-b".into()]);
        // First branch: Updated then push OK; Second: NotFastForward.
        git.queue_ff_outcome(crate::clones::FfOutcome::Updated);
        git.queue_ff_outcome(crate::clones::FfOutcome::NotFastForward);

        let state = fresh_state_with_configurable_git(git);
        insert_test_household(&state, "h", &meta).await;
        {
            let conn = state.db.lock().await;
            crate::db::insert_card(
                &conn,
                &Card {
                    id: "card-y".into(),
                    household: "h".into(),
                    brief: "y".into(),
                    column: Column::Implementing,
                    created_at: 0,
                    updated_at: 0,
                    needs_heavy: false,
                    parent_id: None,
                },
            )
            .unwrap();
        }

        let req = RpcRequest {
            id: 1,
            method: "push".into(),
            params: serde_json::json!({ "id": "card-y" }),
        };
        let value = dispatch(req, state, CallerContext::host()).await.unwrap();
        let result: PushResult = serde_json::from_value(value).unwrap();
        assert_eq!(result.branches.len(), 2);
        assert!(matches!(
            result.branches[0].status,
            BranchPushStatus::Pushed
        ));
        assert!(matches!(
            result.branches[1].status,
            BranchPushStatus::NotFastForward
        ));
        assert_eq!(result.branches[0].branch, "feat-a");
        assert_eq!(result.branches[1].branch, "feat-b");
    }

    #[tokio::test]
    async fn push_iterates_meta_repo_then_manifest_repos_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join("ws");
        std::fs::create_dir_all(&meta).unwrap();
        write_test_manifest(&meta, "ws", &["ws", "backend", "lore"]);

        let git = std::sync::Arc::new(ConfigurableGit::new());
        // Each repo path has one branch; assert push_call_order matches.
        git.set_branches(meta.clone(), vec!["feat-meta".into()]);
        git.set_branches(meta.join("backend"), vec!["feat-back".into()]);
        git.set_branches(meta.join("lore"), vec!["feat-lore".into()]);

        let state = fresh_state_with_configurable_git(git.clone());
        insert_test_household(&state, "h", &meta).await;
        {
            let conn = state.db.lock().await;
            crate::db::insert_card(
                &conn,
                &Card {
                    id: "card-z".into(),
                    household: "h".into(),
                    brief: "z".into(),
                    column: Column::Implementing,
                    created_at: 0,
                    updated_at: 0,
                    needs_heavy: false,
                    parent_id: None,
                },
            )
            .unwrap();
        }

        let req = RpcRequest {
            id: 1,
            method: "push".into(),
            params: serde_json::json!({ "id": "card-z" }),
        };
        let value = dispatch(req, state, CallerContext::host()).await.unwrap();
        let result: PushResult = serde_json::from_value(value).unwrap();
        let repos: Vec<&str> = result.branches.iter().map(|b| b.repo.as_str()).collect();
        assert_eq!(repos, vec!["ws", "backend", "lore"]);
        let call_order = git.push_call_order.lock().unwrap();
        let paths: Vec<&std::path::PathBuf> = call_order.iter().map(|(p, _)| p).collect();
        assert_eq!(
            paths,
            vec![&meta, &meta.join("backend"), &meta.join("lore")]
        );
    }

    use reeve_protocol::AttachResult;

    /// ContainerOps test mock that returns a configured discover response.
    struct ConfigurableContainers {
        discover_response: std::sync::Mutex<Option<String>>,
    }
    impl ConfigurableContainers {
        fn new() -> Self {
            Self {
                discover_response: std::sync::Mutex::new(None),
            }
        }
        fn set_discover(&self, name: Option<String>) {
            *self.discover_response.lock().unwrap() = name;
        }
    }
    #[async_trait::async_trait]
    impl crate::container::ContainerOps for ConfigurableContainers {
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
            Ok(self.discover_response.lock().unwrap().clone())
        }
        async fn exec_in_tmux(
            &self,
            _container_name: &str,
            _session_name: &str,
            _command: &str,
        ) -> Result<()> {
            Ok(())
        }
    }

    fn fresh_state_with_containers(
        containers: std::sync::Arc<ConfigurableContainers>,
    ) -> DaemonState {
        let conn = crate::db::open(":memory:").unwrap();
        let db: crate::transport::server::DbHandle = std::sync::Arc::new(tokio::sync::Mutex::new(conn));
        let (household_repo, household_lookup) =
            reeve_household::make_household_components(db.clone());
        DaemonState {
            db: db.clone(),
            containers: containers as crate::container::ContainerOpsHandle,
            git: std::sync::Arc::new(NoopGit) as crate::clones::GitOpsHandle,
            listeners: crate::transport::listener::ListenerRegistry::new(
                db,
                std::sync::Arc::new(NoopDispatcher),
            ),
            claude_dir: std::path::PathBuf::from("/tmp/nonexistent-test-claude-dir"),
            api_key: None,
            household_repo,
            household_lookup,
        }
    }

    #[tokio::test]
    async fn attach_returns_container_and_session_for_implementing_card() {
        let containers = std::sync::Arc::new(ConfigurableContainers::new());
        containers.set_discover(Some("happy-cool-container".into()));
        let state = fresh_state_with_containers(containers);
        {
            let conn = state.db.lock().await;
            crate::db::insert_card(
                &conn,
                &Card {
                    id: "card-a".into(),
                    household: "h".into(),
                    brief: "a".into(),
                    column: Column::Implementing,
                    created_at: 0,
                    updated_at: 0,
                    needs_heavy: false,
                    parent_id: None,
                },
            )
            .unwrap();
        }

        let req = RpcRequest {
            id: 1,
            method: "attach".into(),
            params: serde_json::json!({ "id": "card-a" }),
        };
        let value = dispatch(req, state, CallerContext::host()).await.unwrap();
        let result: AttachResult = serde_json::from_value(value).unwrap();
        assert_eq!(result.container_name, "happy-cool-container");
        assert_eq!(result.session_name, "reeve");
        assert_eq!(result.in_container_workspace, "/workspaces/card-a");
    }

    #[tokio::test]
    async fn attach_returns_not_found_for_unknown_card() {
        let containers = std::sync::Arc::new(ConfigurableContainers::new());
        let state = fresh_state_with_containers(containers);

        let req = RpcRequest {
            id: 1,
            method: "attach".into(),
            params: serde_json::json!({ "id": "missing-card" }),
        };
        let err = dispatch(req, state, CallerContext::host())
            .await
            .unwrap_err();
        assert_eq!(err.code, RpcError::NOT_FOUND);
        assert!(err.message.contains("not found"), "msg: {}", err.message);
    }

    #[tokio::test]
    async fn attach_rejects_non_implementing_card() {
        let containers = std::sync::Arc::new(ConfigurableContainers::new());
        let state = fresh_state_with_containers(containers);
        {
            let conn = state.db.lock().await;
            crate::db::insert_card(
                &conn,
                &Card {
                    id: "card-b".into(),
                    household: "h".into(),
                    brief: "b".into(),
                    column: Column::Backlog,
                    created_at: 0,
                    updated_at: 0,
                    needs_heavy: false,
                    parent_id: None,
                },
            )
            .unwrap();
        }

        let req = RpcRequest {
            id: 1,
            method: "attach".into(),
            params: serde_json::json!({ "id": "card-b" }),
        };
        let err = dispatch(req, state, CallerContext::host())
            .await
            .unwrap_err();
        assert_eq!(err.code, RpcError::INVALID_ARGUMENT);
        assert!(err.message.contains("Implementing"), "msg: {}", err.message);
    }

    #[tokio::test]
    async fn attach_errors_when_no_container_for_card() {
        // Card in Implementing but discover_container_name returns None.
        let containers = std::sync::Arc::new(ConfigurableContainers::new());
        // Don't call set_discover — defaults to None.
        let state = fresh_state_with_containers(containers);
        {
            let conn = state.db.lock().await;
            crate::db::insert_card(
                &conn,
                &Card {
                    id: "card-c".into(),
                    household: "h".into(),
                    brief: "c".into(),
                    column: Column::Implementing,
                    created_at: 0,
                    updated_at: 0,
                    needs_heavy: false,
                    parent_id: None,
                },
            )
            .unwrap();
        }

        let req = RpcRequest {
            id: 1,
            method: "attach".into(),
            params: serde_json::json!({ "id": "card-c" }),
        };
        let err = dispatch(req, state, CallerContext::host())
            .await
            .unwrap_err();
        assert_eq!(err.code, RpcError::INTERNAL);
        assert!(err.message.contains("not running"), "msg: {}", err.message);
    }
}
