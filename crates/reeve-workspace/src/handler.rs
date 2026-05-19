//! RPC handlers for the Workspace aggregate.

use crate::container::ContainerOps;
use crate::git::GitOps;
use reeve_card::CardRepo;
use reeve_protocol::{
    AttachParams, AttachResult, BranchPushResult, BranchPushStatus, Column, PushParams, PushResult,
    RpcError,
};
use serde_json::Value;

fn internal<E: std::fmt::Display>(e: E) -> RpcError {
    RpcError {
        code: RpcError::INTERNAL,
        message: e.to_string(),
    }
}

pub async fn handle_attach(
    cards: &dyn CardRepo,
    containers: &dyn ContainerOps,
    params: AttachParams,
) -> std::result::Result<Value, RpcError> {
    // 1. Fetch the card.
    let card = cards
        .get(&params.id)
        .await
        .map_err(internal)?
        .ok_or_else(|| RpcError {
            code: RpcError::NOT_FOUND,
            message: format!("card '{}' not found", params.id),
        })?;

    // 2. Column check.
    if card.column() != Column::Implementing {
        return Err(RpcError {
            code: RpcError::INVALID_ARGUMENT,
            message: format!(
                "card is in {}; attach only works in Implementing",
                card.column().label()
            ),
        });
    }

    // 3. Container discovery by label.
    let container_name = containers
        .discover_container_name(card.id())
        .await
        .map_err(|e| RpcError {
            code: RpcError::INTERNAL,
            message: format!("container discovery failed: {e:#}"),
        })?
        .ok_or_else(|| RpcError {
            code: RpcError::INTERNAL,
            message: format!(
                "container for card '{}' not running (try mv {} implementing again)",
                card.id(), card.id()
            ),
        })?;

    // 4. Return the address.
    let result = AttachResult {
        container_name,
        session_name: "reeve".to_string(),
        in_container_workspace: format!("/workspaces/{}", card.id()),
    };
    serde_json::to_value(result).map_err(internal)
}

pub async fn handle_push(
    cards: &dyn CardRepo,
    households: &dyn reeve_household::HouseholdRepo,
    git: &dyn GitOps,
    params: PushParams,
) -> std::result::Result<Value, RpcError> {
    // 1. Fetch the card from the DB.
    let card = cards
        .get(&params.id)
        .await
        .map_err(internal)?
        .ok_or_else(|| RpcError {
            code: RpcError::NOT_FOUND,
            message: format!("card '{}' not found", params.id),
        })?;

    // 2. Resolve the household from the household_repo.
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

    // 3. Load + validate the manifest.
    let manifest =
        reeve_household::load_household_manifest(&household).map_err(|e| RpcError {
            code: RpcError::INTERNAL,
            message: format!("load manifest: {e:#}"),
        })?;

    // 4. For each repo, for each branch in the namespace, FF + push.
    let mut branches = Vec::new();
    for (repo_name, host_path) in reeve_household::iter_card_repos(&household, &manifest) {
        let listed = git
            .list_namespace_refs(&host_path, card.id())
            .await
            .map_err(|e| RpcError {
                code: RpcError::INTERNAL,
                message: format!(
                    "list refs/reeve/{}/* in {}: {e:#}",
                    card.id(),
                    host_path.display()
                ),
            })?;
        for branch in listed {
            let target = format!("refs/heads/{branch}");
            let source = format!("refs/reeve/{}/{}", card.id(), branch);
            let status = match git.fast_forward(&host_path, &target, &source).await {
                Ok(crate::git::FfOutcome::Updated) => {
                    match git.push(&host_path, &branch).await {
                        Ok(()) => BranchPushStatus::Pushed,
                        Err(e) => BranchPushStatus::PushFailed {
                            message: format!("{e:#}"),
                        },
                    }
                }
                Ok(crate::git::FfOutcome::NotFastForward) => BranchPushStatus::NotFastForward,
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
    serde_json::to_value(PushResult { branches }).map_err(internal)
}
