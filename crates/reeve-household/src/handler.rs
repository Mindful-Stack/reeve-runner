//! RPC handlers for the Household aggregate.

use crate::manifest::{self, Manifest};
use crate::repo::{HouseholdRepo, HouseholdRow};
use crate::scaffold::HouseholdScaffold;
use reeve_protocol::{
    CardReference, HouseholdAddParams, HouseholdDetail, HouseholdNewParams, HouseholdSource,
    HouseholdSummary, RepoDetail, RepoOnDisk, RpcError,
};
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_TEMPLATE: &str = "https://github.com/Mindful-Stack/witan-household";

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn internal<E: std::fmt::Display>(e: E) -> RpcError {
    RpcError {
        code: RpcError::INTERNAL,
        message: e.to_string(),
    }
}

fn default_source_dir() -> PathBuf {
    std::env::var("HOME")
        .map(|h| PathBuf::from(h).join("Source"))
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn summary_from(row: &HouseholdRow, m: &Manifest) -> HouseholdSummary {
    HouseholdSummary {
        name: row.name.clone(),
        meta_repo_path: row.meta_repo_path.clone(),
        workspace_name: m.meta_repo.clone(),
        knowledge_base: m.knowledge_base.clone(),
        repo_count: m.repos.len(),
    }
}

#[derive(Deserialize)]
pub struct NameParam {
    pub name: String,
}

pub async fn handle_household_add(
    repo: &dyn HouseholdRepo,
    scaffold: &dyn HouseholdScaffold,
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
            scaffold
                .clone_url(url, &dest)
                .await
                .map_err(|e| RpcError {
                    code: RpcError::INTERNAL,
                    message: format!("clone {url}: {e:#}"),
                })?;
            dest
        }
    };

    let m = manifest::load_manifest(&meta_repo_path).map_err(|e| RpcError {
        code: RpcError::INVALID_ARGUMENT,
        message: format!("manifest invalid: {e:#}"),
    })?;

    let row = HouseholdRow {
        name: params.name.clone(),
        meta_repo_path: meta_repo_path.clone(),
        added_at: now_secs(),
    };
    repo.insert(row.clone()).await.map_err(|e| RpcError {
        code: RpcError::CONFLICT,
        message: format!("insert household: {e:#}"),
    })?;

    serde_json::to_value(summary_from(&row, &m)).map_err(internal)
}

pub async fn handle_household_new(
    repo: &dyn HouseholdRepo,
    scaffold: &dyn HouseholdScaffold,
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

    scaffold
        .clone_template(&template, &dest)
        .await
        .map_err(|e| RpcError {
            code: RpcError::INTERNAL,
            message: format!("clone template {template}: {e:#}"),
        })?;

    scaffold
        .init_repo(&dest)
        .await
        .map_err(|e| RpcError {
            code: RpcError::INTERNAL,
            message: format!("init repo: {e:#}"),
        })?;

    let manifest_path = dest.join("household.json");
    let body = std::fs::read_to_string(&manifest_path).map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("read manifest: {e:#}"),
    })?;
    let mut m: Manifest = serde_json::from_str(&body).map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("parse template manifest: {e:#}"),
    })?;
    let old_workspace = m.meta_repo.clone();
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
    m.meta_repo = params.name.clone();
    let body = serde_json::to_string_pretty(&m).map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("re-serialize manifest: {e:#}"),
    })?;
    std::fs::write(&manifest_path, body).map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("write manifest: {e:#}"),
    })?;

    scaffold
        .commit_all(&dest, "Initial workspace")
        .await
        .map_err(|e| RpcError {
            code: RpcError::INTERNAL,
            message: format!("commit: {e:#}"),
        })?;

    let row = HouseholdRow {
        name: params.name.clone(),
        meta_repo_path: dest.clone(),
        added_at: now_secs(),
    };
    repo.insert(row.clone()).await.map_err(|e| RpcError {
        code: RpcError::CONFLICT,
        message: format!("insert household: {e:#}"),
    })?;

    serde_json::to_value(summary_from(&row, &m)).map_err(internal)
}

pub async fn handle_household_ls(
    repo: &dyn HouseholdRepo,
) -> std::result::Result<Value, RpcError> {
    let rows = repo.list().await.map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("list households: {e:#}"),
    })?;
    let mut out: Vec<HouseholdSummary> = Vec::with_capacity(rows.len());
    for row in rows {
        let m = manifest::load_manifest(&row.meta_repo_path).unwrap_or_else(|_| Manifest {
            meta_repo: row.name.clone(),
            knowledge_base: None,
            repos: vec![],
        });
        out.push(summary_from(&row, &m));
    }
    serde_json::to_value(out).map_err(internal)
}

pub async fn handle_household_rm(
    repo: &dyn HouseholdRepo,
    params: NameParam,
) -> std::result::Result<Value, RpcError> {
    let removed = repo.remove(&params.name).await.map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("remove household: {e:#}"),
    })?;
    if !removed {
        return Err(RpcError {
            code: RpcError::NOT_FOUND,
            message: format!("household '{}' not found", params.name),
        });
    }
    serde_json::to_value(()).map_err(internal)
}

pub async fn handle_household_show(
    repo: &dyn HouseholdRepo,
    cards_for_household: Vec<CardReference>,
    params: NameParam,
) -> std::result::Result<Value, RpcError> {
    let row = repo
        .get(&params.name)
        .await
        .map_err(|e| RpcError {
            code: RpcError::INTERNAL,
            message: format!("get household: {e:#}"),
        })?
        .ok_or_else(|| RpcError {
            code: RpcError::NOT_FOUND,
            message: format!("household '{}' not found", params.name),
        })?;

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
            let is_workspace = entry.name == m.meta_repo;
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

    let detail = HouseholdDetail {
        summary: summary_from(&row, &m),
        workspace_url,
        repos,
        cards: cards_for_household,
    };
    serde_json::to_value(detail).map_err(internal)
}

fn git_remote_url(meta_repo: &std::path::Path) -> Option<String> {
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
