//! RPC method dispatch table. The phone directory of the daemon — one place
//! to see every supported RPC method and where it routes.

use crate::app::state::DaemonState;
use crate::transport::server::CallerContext;
use reeve_card::make_card_repo;
use reeve_protocol::{
    DeleteParams, EditParams, LsParams, MvParams, MvSelfParams, NewParams, RpcError, RpcRequest,
};
use serde_json::Value;

fn parse_params<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, RpcError> {
    serde_json::from_value(value).map_err(|e| RpcError {
        code: RpcError::INVALID_PARAMS,
        message: format!("invalid params: {e}"),
    })
}

fn require_card(caller: &CallerContext) -> Result<String, RpcError> {
    caller.card_id.clone().ok_or_else(|| RpcError {
        code: RpcError::FORBIDDEN,
        message: "self-scoped methods require per-card socket".into(),
    })
}

fn internal<E: std::fmt::Display>(e: E) -> RpcError {
    RpcError {
        code: RpcError::INTERNAL,
        message: e.to_string(),
    }
}

async fn fetch_container_status(
    state: &DaemonState,
    card_id: &str,
) -> Result<Option<reeve_protocol::ContainerStatus>, RpcError> {
    let row = state
        .workspaces
        .get_container(card_id)
        .await
        .map_err(internal)?;
    Ok(row.map(|r| r.status))
}

pub async fn dispatch(
    req: RpcRequest,
    state: DaemonState,
    caller: CallerContext,
) -> Result<Value, RpcError> {
    match req.method.as_str() {
        // --- card aggregate ---
        "ls" => {
            let cards = make_card_repo(state.db.clone());
            // Empty params (`{}`) round-trip to `LsParams { household: None }`
            // via serde's #[serde(default)] — preserves backwards compat for
            // pre-filter clients that still send `{}`.
            let params = parse_params::<LsParams>(req.params)?;
            reeve_card::handle_ls(&*cards, params).await
        }
        "new" => {
            let cards = make_card_repo(state.db.clone());
            let params = parse_params::<NewParams>(req.params)?;
            reeve_card::handle_new(&*cards, params).await
        }
        "mv" => {
            let params = parse_params::<MvParams>(req.params)?;
            let _card_guard = state.lock_card(&params.id).await;
            let cards = make_card_repo(state.db.clone());
            let households = state.household_lookup.clone();
            let workspace = state.workspace_lifecycle.clone();
            reeve_card::handle_mv(&*cards, &*households, &*workspace, params).await
        }
        "edit" => {
            let cards = make_card_repo(state.db.clone());
            let params = parse_params::<EditParams>(req.params)?;
            reeve_card::handle_edit(&*cards, params).await
        }
        "delete" => {
            let params = parse_params::<DeleteParams>(req.params)?;
            let _card_guard = state.lock_card(&params.id).await;
            let cards = make_card_repo(state.db.clone());
            let workspace = state.workspace_lifecycle.clone();
            reeve_card::handle_delete(&*cards, &*workspace, params).await
        }
        "mv-self" => {
            let card_id = require_card(&caller)?;
            let params = parse_params::<MvSelfParams>(req.params)?;
            let _card_guard = state.lock_card(&card_id).await;
            let cards = make_card_repo(state.db.clone());
            let households = state.household_lookup.clone();
            let workspace = state.workspace_lifecycle.clone();
            reeve_card::handle_mv_self(&*cards, &*households, &*workspace, card_id, params).await
        }
        "status-self" => {
            let card_id = require_card(&caller)?;
            let cards = make_card_repo(state.db.clone());
            let container_status = fetch_container_status(&state, &card_id).await?;
            reeve_card::handle_status_self(&*cards, container_status, card_id).await
        }

        // --- household aggregate ---
        "household.add" => {
            let scaffold = reeve_household::make_scaffold();
            let params = parse_params::<reeve_protocol::HouseholdAddParams>(req.params)?;
            reeve_household::handle_household_add(&*state.household_repo, &*scaffold, params).await
        }
        "household.new" => {
            let scaffold = reeve_household::make_scaffold();
            let params = parse_params::<reeve_protocol::HouseholdNewParams>(req.params)?;
            reeve_household::handle_household_new(&*state.household_repo, &*scaffold, params).await
        }
        "household.ls" => reeve_household::handle_household_ls(&*state.household_repo).await,
        "household.rm" => {
            let params = parse_params::<reeve_household::NameParam>(req.params)?;
            reeve_household::handle_household_rm(&*state.household_repo, params).await
        }
        "household.show" => {
            let params = parse_params::<reeve_household::NameParam>(req.params)?;
            // Pre-resolve referencing cards via CardRepo.
            let cards_repo = reeve_card::make_card_repo(state.db.clone());
            let cards = cards_repo
                .for_household(&params.name)
                .await
                .map_err(|e| RpcError {
                    code: RpcError::INTERNAL,
                    message: e.to_string(),
                })?;
            reeve_household::handle_household_show(&*state.household_repo, cards, params).await
        }

        // --- progress (read-only) ---
        "card_progress" => {
            let params = parse_params::<reeve_protocol::CardProgressParams>(req.params)?;
            let stages = state.progress.get(&params.id).await;
            serde_json::to_value(reeve_protocol::CardProgressResult {
                card_id: params.id,
                stages,
            })
            .map_err(internal)
        }

        // --- workspace aggregate ---
        "attach" => {
            let cards = make_card_repo(state.db.clone());
            let params = parse_params::<reeve_protocol::AttachParams>(req.params)?;
            reeve_workspace::handle_attach(&*cards, &*state.containers, params).await
        }
        "push" => {
            let cards = make_card_repo(state.db.clone());
            let params = parse_params::<reeve_protocol::PushParams>(req.params)?;
            reeve_workspace::handle_push(&*cards, &*state.household_repo, &*state.git, params).await
        }

        other => Err(RpcError {
            code: RpcError::METHOD_NOT_FOUND,
            message: format!("method not found: {other}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::DaemonState;
    use crate::transport::listener::{Dispatcher, ListenerRegistry};
    use crate::transport::server::DbHandle;
    use anyhow::Result;
    use async_trait::async_trait;
    use reeve_card::Card;
    use reeve_protocol::{Column, LsResult, MvResult, MvSelfResult, NewResult, StatusSelfResult};
    use reeve_workspace::{ContainerOps, ContainerOpsHandle};
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

    fn make_workspace_lifecycle(
        workspaces: Arc<dyn reeve_workspace::WorkspaceRepo>,
        household_repo: Arc<dyn reeve_household::HouseholdRepo>,
        git: GitOpsHandle,
        containers: ContainerOpsHandle,
        listeners: &ListenerRegistry,
        progress_handle: reeve_workspace::ProgressHandle,
    ) -> Arc<dyn reeve_card::WorkspaceLifecycle> {
        let listeners_arc: reeve_workspace::ListenerHandle = Arc::new(listeners.clone());
        Arc::new(reeve_workspace::LiveWorkspaceLifecycle::new(
            workspaces,
            household_repo,
            git,
            containers,
            listeners_arc,
            progress_handle,
            std::path::PathBuf::from("/tmp/nonexistent-test-claude-dir"),
            None,
        ))
    }

    fn fresh_state() -> DaemonState {
        let conn = crate::app::lifecycle::open_db(":memory:").unwrap();
        let db: DbHandle = Arc::new(Mutex::new(conn));
        let (household_repo, household_lookup) =
            reeve_household::make_household_components(db.clone());
        let containers: ContainerOpsHandle = Arc::new(NoopOps);
        let git: GitOpsHandle = Arc::new(NoopGit);
        let listeners = ListenerRegistry::new(db.clone(), Arc::new(NoopDispatcher));
        let cards = reeve_card::make_card_repo(db.clone());
        let workspaces = reeve_workspace::make_workspace_repo(db.clone());
        let progress = Arc::new(crate::app::progress::InMemoryProgress::new());
        let workspace_lifecycle = make_workspace_lifecycle(
            workspaces.clone(),
            household_repo.clone(),
            git.clone(),
            containers.clone(),
            &listeners,
            progress.clone() as reeve_workspace::ProgressHandle,
        );
        DaemonState {
            db,
            cards,
            workspaces,
            containers,
            git,
            listeners,
            household_repo,
            household_lookup,
            workspace_lifecycle,
            progress,
            card_locks: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }

    fn fresh_state_with_git(git: GitOpsHandle) -> DaemonState {
        let conn = crate::app::lifecycle::open_db(":memory:").unwrap();
        let db: DbHandle = Arc::new(Mutex::new(conn));
        let (household_repo, household_lookup) =
            reeve_household::make_household_components(db.clone());
        let containers: ContainerOpsHandle = Arc::new(NoopOps);
        let listeners = ListenerRegistry::new(db.clone(), Arc::new(NoopDispatcher));
        let cards = reeve_card::make_card_repo(db.clone());
        let workspaces = reeve_workspace::make_workspace_repo(db.clone());
        let progress = Arc::new(crate::app::progress::InMemoryProgress::new());
        let workspace_lifecycle = make_workspace_lifecycle(
            workspaces.clone(),
            household_repo.clone(),
            git.clone(),
            containers.clone(),
            &listeners,
            progress.clone() as reeve_workspace::ProgressHandle,
        );
        DaemonState {
            db,
            cards,
            workspaces,
            containers,
            git,
            listeners,
            household_repo,
            household_lookup,
            workspace_lifecycle,
            progress,
            card_locks: Arc::new(Mutex::new(std::collections::HashMap::new())),
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
    async fn ls_with_household_filter_returns_only_matching_cards() {
        let state = fresh_state();

        for (h, b) in [("h1", "a"), ("h1", "b"), ("h2", "c")] {
            dispatch(
                RpcRequest {
                    id: 1,
                    method: "new".into(),
                    params: serde_json::json!({
                        "household": h, "brief": b, "needs_heavy": false,
                    }),
                },
                state.clone(),
                CallerContext::host(),
            )
            .await
            .unwrap();
        }

        let value = dispatch(
            RpcRequest {
                id: 99,
                method: "ls".into(),
                params: serde_json::json!({ "household": "h1" }),
            },
            state.clone(),
            CallerContext::host(),
        )
        .await
        .unwrap();
        let result: LsResult = serde_json::from_value(value).unwrap();
        assert_eq!(result.cards.len(), 2);
        assert!(result.cards.iter().all(|c| c.household == "h1"));

        // Unset = every card (back-compat path).
        let value = dispatch(
            RpcRequest {
                id: 100,
                method: "ls".into(),
                params: serde_json::json!({}),
            },
            state,
            CallerContext::host(),
        )
        .await
        .unwrap();
        let result: LsResult = serde_json::from_value(value).unwrap();
        assert_eq!(result.cards.len(), 3);
    }

    #[tokio::test]
    async fn household_add_via_path_succeeds() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let meta = dir.path().join("ws");
        std::fs::create_dir_all(meta.join(".git")).unwrap();
        std::fs::write(
            meta.join("household.json"),
            r#"{ "meta_repo": "ws", "repos": [{ "name": "ws" }] }"#,
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
            r#"{ "meta_repo": "ws", "knowledge_base": "missing", "repos": [{ "name": "ws" }] }"#,
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
            r#"{ "meta_repo": "default", "repos": [{ "name": "default" }] }"#,
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
        let state = fresh_state_with_git(reeve_workspace::make_git_ops() as GitOpsHandle);
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
            manifest_body.contains("\"meta_repo\": \"my-ws\""),
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
                format!(r#"{{ "meta_repo": "{n}", "repos": [{{ "name": "{n}" }}] }}"#),
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
              "meta_repo": "ws",
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
            r#"{ "meta_repo": "ours", "repos": [{ "name": "ours" }] }"#,
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
        let make_card = |id: &str, brief: &str, column: Column, household: &str| {
            Card::from_persisted(
                id.into(),
                household.into(),
                brief.into(),
                column,
                0,
                0,
                false,
                None,
            )
            .unwrap()
        };
        state
            .cards
            .insert(&make_card("card-1", "brief one", Column::Backlog, "ours"))
            .await
            .unwrap();
        state
            .cards
            .insert(&make_card(
                "card-2",
                "brief two",
                Column::Implementing,
                "ours",
            ))
            .await
            .unwrap();
        state
            .cards
            .insert(&make_card(
                "card-3",
                "brief three",
                Column::Backlog,
                "someone-else",
            ))
            .await
            .unwrap();
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
        state
            .household_repo
            .insert(reeve_household::HouseholdRow {
                name: name.to_string(),
                meta_repo_path: meta_repo_path.to_path_buf(),
                added_at: 0,
            })
            .await
            .unwrap();
    }

    /// Write a minimal household.json at `meta_repo_path` AND create sentinel
    /// .git/ markers for the workspace + each separate sibling. The push +
    /// fetch workers skip entries without .git/ (treating them as inline
    /// manifest content — see fix in 59e38f9); without these markers the
    /// push iteration silently filters everything out and yields 0 branches.
    fn write_test_manifest(meta_repo_path: &std::path::Path, workspace: &str, repos: &[&str]) {
        let repos_json: Vec<String> = repos
            .iter()
            .map(|n| format!(r#"{{"name":"{n}"}}"#))
            .collect();
        let body = format!(
            r#"{{"meta_repo":"{workspace}","repos":[{}]}}"#,
            repos_json.join(",")
        );
        std::fs::write(meta_repo_path.join("household.json"), body).unwrap();
        std::fs::create_dir_all(meta_repo_path.join(".git")).unwrap();
        for repo in repos {
            if *repo == workspace {
                continue; // workspace's .git is meta_repo_path/.git itself
            }
            std::fs::create_dir_all(meta_repo_path.join(repo).join(".git")).unwrap();
        }
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
        state
            .cards
            .insert(
                &Card::from_persisted(
                    "card-x".into(),
                    "h".into(),
                    "x".into(),
                    reeve_protocol::Column::Implementing,
                    0,
                    0,
                    false,
                    None,
                )
                .unwrap(),
            )
            .await
            .unwrap();

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
        let conn = crate::app::lifecycle::open_db(":memory:").unwrap();
        let db: crate::transport::server::DbHandle =
            std::sync::Arc::new(tokio::sync::Mutex::new(conn));
        let (household_repo, household_lookup) =
            reeve_household::make_household_components(db.clone());
        let containers: reeve_workspace::ContainerOpsHandle = std::sync::Arc::new(NoopOps);
        let git_arc: crate::clones::GitOpsHandle = git;
        let listeners = crate::transport::listener::ListenerRegistry::new(
            db.clone(),
            std::sync::Arc::new(NoopDispatcher),
        );
        let cards = reeve_card::make_card_repo(db.clone());
        let workspaces = reeve_workspace::make_workspace_repo(db.clone());
        let progress = std::sync::Arc::new(crate::app::progress::InMemoryProgress::new());
        let workspace_lifecycle = make_workspace_lifecycle(
            workspaces.clone(),
            household_repo.clone(),
            git_arc.clone(),
            containers.clone(),
            &listeners,
            progress.clone() as reeve_workspace::ProgressHandle,
        );
        DaemonState {
            db,
            cards,
            workspaces,
            containers,
            git: git_arc,
            listeners,
            household_repo,
            household_lookup,
            workspace_lifecycle,
            progress,
            card_locks: std::sync::Arc::new(tokio::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
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
        state
            .cards
            .insert(
                &Card::from_persisted(
                    "card-y".into(),
                    "h".into(),
                    "y".into(),
                    Column::Implementing,
                    0,
                    0,
                    false,
                    None,
                )
                .unwrap(),
            )
            .await
            .unwrap();

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
        state
            .cards
            .insert(
                &Card::from_persisted(
                    "card-z".into(),
                    "h".into(),
                    "z".into(),
                    Column::Implementing,
                    0,
                    0,
                    false,
                    None,
                )
                .unwrap(),
            )
            .await
            .unwrap();

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
    impl reeve_workspace::ContainerOps for ConfigurableContainers {
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
        let conn = crate::app::lifecycle::open_db(":memory:").unwrap();
        let db: crate::transport::server::DbHandle =
            std::sync::Arc::new(tokio::sync::Mutex::new(conn));
        let (household_repo, household_lookup) =
            reeve_household::make_household_components(db.clone());
        let containers_arc: reeve_workspace::ContainerOpsHandle = containers;
        let git: crate::clones::GitOpsHandle = std::sync::Arc::new(NoopGit);
        let listeners = crate::transport::listener::ListenerRegistry::new(
            db.clone(),
            std::sync::Arc::new(NoopDispatcher),
        );
        let cards = reeve_card::make_card_repo(db.clone());
        let workspaces = reeve_workspace::make_workspace_repo(db.clone());
        let progress = std::sync::Arc::new(crate::app::progress::InMemoryProgress::new());
        let workspace_lifecycle = make_workspace_lifecycle(
            workspaces.clone(),
            household_repo.clone(),
            git.clone(),
            containers_arc.clone(),
            &listeners,
            progress.clone() as reeve_workspace::ProgressHandle,
        );
        DaemonState {
            db,
            cards,
            workspaces,
            containers: containers_arc,
            git,
            listeners,
            household_repo,
            household_lookup,
            workspace_lifecycle,
            progress,
            card_locks: std::sync::Arc::new(tokio::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
        }
    }

    #[tokio::test]
    async fn attach_returns_container_and_session_for_implementing_card() {
        let containers = std::sync::Arc::new(ConfigurableContainers::new());
        containers.set_discover(Some("happy-cool-container".into()));
        let state = fresh_state_with_containers(containers);
        state
            .cards
            .insert(
                &Card::from_persisted(
                    "card-a".into(),
                    "h".into(),
                    "a".into(),
                    Column::Implementing,
                    0,
                    0,
                    false,
                    None,
                )
                .unwrap(),
            )
            .await
            .unwrap();

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
        state
            .cards
            .insert(
                &Card::from_persisted(
                    "card-b".into(),
                    "h".into(),
                    "b".into(),
                    Column::Backlog,
                    0,
                    0,
                    false,
                    None,
                )
                .unwrap(),
            )
            .await
            .unwrap();

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
        state
            .cards
            .insert(
                &Card::from_persisted(
                    "card-c".into(),
                    "h".into(),
                    "c".into(),
                    Column::Implementing,
                    0,
                    0,
                    false,
                    None,
                )
                .unwrap(),
            )
            .await
            .unwrap();

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

    // -----------------------------------------------------------------------
    // Per-card lock / race-condition tests
    // -----------------------------------------------------------------------

    /// A WorkspaceLifecycle mock that:
    /// - Sleeps `ensure_running_delay` BEFORE recording completion in `call_log`,
    ///   simulating the slow clone+devcontainer-up path.
    /// - Records "running_done" AFTER the sleep and "stopped" immediately.
    ///
    /// This means: without per-card serialization, "stopped" can appear in
    /// the log BEFORE "running_done" when both mvs fire concurrently.
    /// With the lock, "running_done" always precedes "stopped".
    struct SlowLifecycle {
        call_log: Arc<Mutex<Vec<&'static str>>>,
        ensure_running_delay: std::time::Duration,
    }

    impl SlowLifecycle {
        fn new(delay_ms: u64) -> (Arc<Self>, Arc<Mutex<Vec<&'static str>>>) {
            let log: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(vec![]));
            let mock = Arc::new(Self {
                call_log: log.clone(),
                ensure_running_delay: std::time::Duration::from_millis(delay_ms),
            });
            (mock, log)
        }
    }

    #[async_trait]
    impl reeve_card::WorkspaceLifecycle for SlowLifecycle {
        async fn ensure_running(&self, _card: &reeve_card::Card) -> anyhow::Result<()> {
            // Sleep FIRST (model slow work), then record completion.
            // Without the per-card lock, ensure_stopped can run and record
            // "stopped" during this sleep, producing an out-of-order log.
            tokio::time::sleep(self.ensure_running_delay).await;
            self.call_log.lock().await.push("running_done");
            Ok(())
        }
        async fn ensure_stopped(&self, _card_id: &str) -> anyhow::Result<()> {
            self.call_log.lock().await.push("stopped");
            Ok(())
        }
    }

    /// Build a DaemonState that uses the given WorkspaceLifecycle (bypassing
    /// LiveWorkspaceLifecycle entirely so we can inject the mock).
    fn fresh_state_with_lifecycle(
        lifecycle: Arc<dyn reeve_card::WorkspaceLifecycle>,
    ) -> DaemonState {
        let conn = crate::app::lifecycle::open_db(":memory:").unwrap();
        let db: DbHandle = Arc::new(Mutex::new(conn));
        let (household_repo, household_lookup) =
            reeve_household::make_household_components(db.clone());
        let containers: ContainerOpsHandle = Arc::new(NoopOps);
        let git: GitOpsHandle = Arc::new(NoopGit);
        let listeners = ListenerRegistry::new(db.clone(), Arc::new(NoopDispatcher));
        let cards = reeve_card::make_card_repo(db.clone());
        let workspaces = reeve_workspace::make_workspace_repo(db.clone());
        DaemonState {
            db,
            cards,
            workspaces,
            containers,
            git,
            listeners,
            household_repo,
            household_lookup,
            workspace_lifecycle: lifecycle,
            progress: Arc::new(crate::app::progress::InMemoryProgress::new()),
            card_locks: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }

    /// Helper: create a card via `new` RPC and return its id.
    async fn create_card(state: &DaemonState, brief: &str) -> String {
        let value = dispatch(
            RpcRequest {
                id: 1,
                method: "new".into(),
                params: serde_json::json!({
                    "household": "h",
                    "brief": brief,
                    "needs_heavy": false,
                }),
            },
            state.clone(),
            CallerContext::host(),
        )
        .await
        .unwrap();
        serde_json::from_value::<NewResult>(value).unwrap().card.id
    }

    /// Sequential mv: the lock must not deadlock and must permit two back-to-back
    /// mv calls for the same card.
    #[tokio::test]
    async fn sequential_mv_with_lock_does_not_deadlock() {
        let (lifecycle, _log) = SlowLifecycle::new(0);
        let state = fresh_state_with_lifecycle(lifecycle);
        let card_id = create_card(&state, "seq-test").await;

        // Backlog → Review (no lifecycle calls, but exercises lock acquire/release)
        let v1 = dispatch(
            RpcRequest {
                id: 2,
                method: "mv".into(),
                params: serde_json::json!({ "id": card_id, "column": "review" }),
            },
            state.clone(),
            CallerContext::host(),
        )
        .await
        .unwrap();
        let c1 = serde_json::from_value::<MvResult>(v1).unwrap().card;
        assert_eq!(c1.column, Column::Review);

        // Review → Planning (another lock acquire for same card; must not deadlock)
        let v2 = dispatch(
            RpcRequest {
                id: 3,
                method: "mv".into(),
                params: serde_json::json!({ "id": card_id, "column": "planning" }),
            },
            state.clone(),
            CallerContext::host(),
        )
        .await
        .unwrap();
        let c2 = serde_json::from_value::<MvResult>(v2).unwrap().card;
        assert_eq!(c2.column, Column::Planning);
    }

    /// Concurrent mv race test: two `mv` RPCs for the same card are fired
    /// concurrently on a multi-threaded runtime. With the per-card lock, they
    /// must execute one after the other — ensure_running finishes completely
    /// before ensure_stopped begins.
    ///
    /// The mock records "running_done" only AFTER its 50 ms sleep, and
    /// "stopped" immediately. Without the lock, "stopped" can appear in the
    /// log before "running_done" because the second task runs during the sleep
    /// window — exactly the production orphan-container race. With the lock,
    /// "running_done" always precedes "stopped".
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_mv_for_same_card_serializes_lifecycle_calls() {
        // 50 ms delay models the slow clone/devcontainer path.
        let (lifecycle, call_log) = SlowLifecycle::new(50);
        let state = fresh_state_with_lifecycle(lifecycle);

        // Register household "h" so mv → implementing passes the household check.
        state
            .household_repo
            .insert(reeve_household::HouseholdRow {
                name: "h".to_string(),
                meta_repo_path: std::path::PathBuf::from("/tmp/test-meta"),
                added_at: 0,
            })
            .await
            .unwrap();

        let card_id = create_card(&state, "race-test").await;

        // First mv: Backlog → Implementing  (triggers ensure_running, slow)
        let state_a = state.clone();
        let id_a = card_id.clone();
        let mv_to_implementing = tokio::spawn(async move {
            dispatch(
                RpcRequest {
                    id: 10,
                    method: "mv".into(),
                    params: serde_json::json!({ "id": id_a, "column": "implementing" }),
                },
                state_a,
                CallerContext::host(),
            )
            .await
        });

        // Second mv: → Backlog  (triggers ensure_stopped)
        // Give the first task a small head-start so it acquires the lock first
        // and the serialization guarantee is exercised (not just luck of scheduling).
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let state_b = state.clone();
        let id_b = card_id.clone();
        let mv_to_backlog = tokio::spawn(async move {
            dispatch(
                RpcRequest {
                    id: 11,
                    method: "mv".into(),
                    params: serde_json::json!({ "id": id_b, "column": "backlog" }),
                },
                state_b,
                CallerContext::host(),
            )
            .await
        });

        let (res_a, res_b) = tokio::join!(mv_to_implementing, mv_to_backlog);
        res_a.unwrap().unwrap(); // no panic, no RpcError
        res_b.unwrap().unwrap();

        // With serialization the mock log must be:
        //   ["running_done", "stopped"]
        // — ensure_running completed in full before ensure_stopped began.
        //
        // Without the lock (multi-threaded runtime, 50 ms sleep), the second
        // task runs during the sleep window and produces:
        //   ["stopped", "running_done"]
        // — exactly the orphan-container state that caused production bugs.
        let log = call_log.lock().await;
        assert_eq!(
            *log,
            vec!["running_done", "stopped"],
            "lifecycle calls must be strictly ordered (running_done before stopped); got: {log:?}"
        );
    }

    #[tokio::test]
    async fn card_progress_unknown_card_returns_empty_stages() {
        let state = fresh_state();
        let req = RpcRequest {
            id: 1,
            method: "card_progress".into(),
            params: serde_json::json!({"id": "no-such-card"}),
        };
        let v = dispatch(req, state, CallerContext::host()).await.unwrap();
        let result: reeve_protocol::CardProgressResult = serde_json::from_value(v).unwrap();
        assert_eq!(result.card_id, "no-such-card");
        assert!(result.stages.is_empty());
    }

    #[tokio::test]
    async fn card_progress_returns_reported_stages() {
        use reeve_protocol::{ProgressKind, ProgressStatus};
        use reeve_workspace::ProgressReporter;
        let state = fresh_state();
        // Report directly into the InMemoryProgress instance held by DaemonState.
        state
            .progress
            .report("c1", ProgressKind::DevcontainerUp, ProgressStatus::Active)
            .await;

        let req = RpcRequest {
            id: 1,
            method: "card_progress".into(),
            params: serde_json::json!({"id": "c1"}),
        };
        let v = dispatch(req, state, CallerContext::host()).await.unwrap();
        let result: reeve_protocol::CardProgressResult = serde_json::from_value(v).unwrap();
        assert_eq!(result.card_id, "c1");
        assert_eq!(result.stages.len(), 1);
        assert_eq!(result.stages[0].kind, ProgressKind::DevcontainerUp);
        assert_eq!(result.stages[0].status, ProgressStatus::Active);
    }

    /// Concurrent mv for DIFFERENT cards must NOT serialize — both lifecycle
    /// calls should complete concurrently (both ensure_running calls overlap).
    /// This test asserts no deadlock and correct final state for each card.
    #[tokio::test]
    async fn concurrent_mv_for_different_cards_are_independent() {
        let (lifecycle, _) = SlowLifecycle::new(0);
        let state = fresh_state_with_lifecycle(lifecycle);
        let card_a = create_card(&state, "card-a").await;
        let card_b = create_card(&state, "card-b").await;

        let state_a = state.clone();
        let id_a = card_a.clone();
        let mv_a = tokio::spawn(async move {
            dispatch(
                RpcRequest {
                    id: 20,
                    method: "mv".into(),
                    params: serde_json::json!({ "id": id_a, "column": "review" }),
                },
                state_a,
                CallerContext::host(),
            )
            .await
            .unwrap()
        });

        let state_b = state.clone();
        let id_b = card_b.clone();
        let mv_b = tokio::spawn(async move {
            dispatch(
                RpcRequest {
                    id: 21,
                    method: "mv".into(),
                    params: serde_json::json!({ "id": id_b, "column": "planning" }),
                },
                state_b,
                CallerContext::host(),
            )
            .await
            .unwrap()
        });

        let (va, vb) = tokio::join!(mv_a, mv_b);
        let ca = serde_json::from_value::<MvResult>(va.unwrap())
            .unwrap()
            .card;
        let cb = serde_json::from_value::<MvResult>(vb.unwrap())
            .unwrap()
            .card;
        assert_eq!(ca.column, Column::Review);
        assert_eq!(cb.column, Column::Planning);
    }
}
