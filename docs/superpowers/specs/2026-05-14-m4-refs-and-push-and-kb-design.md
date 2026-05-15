# Reeve M4 — Namespaced refs + `reeve push`

**Status:** Design (2026-05-14, trimmed 2026-05-15)
**Predecessor:** M3.5 households-as-meta-repos (`2026-05-15-m3-5-households-as-meta-repos-design.md`)
**Parent design:** `docs/superpowers/specs/2026-04-29-reeve-design.md`

> **Trim note (2026-05-15):** This spec was originally titled "Namespaced refs + `reeve push` + Lorekeeper KB" and folded the KB-clone integration into M4. Brainstorming on 2026-05-15 split that integration into M3.5 (households-as-meta-repos), where the KB became a peer of code repos in a unified manifest. M4 is now refs + push only — the KB participates as just another repo, no special case, no `[knowledge_base]` schema, no Lorekeeper-presence warning. The original framing is preserved in git history.

## Context

M3.5 unified the household schema: a household is a user-cloned (or Reeve-scaffolded) meta-repo, with siblings as gitignored subdirs and the KB as a sibling named by the manifest's `knowledge_base` field. Cards clone the meta-repo + every sibling declared in its manifest into `~/reeve/cards/<id>/`, mounting that as the container workspace. M3.5 wired the agent-can-see-code half of the kanban loop.

M4 closes the user-can-review-code half:

1. **Continuous fetch** — the daemon periodically `git fetch`es each card's clones back into the host repos, into a private `refs/reeve/<card-id>/*` namespace. GitKraken/lazygit/sublime-merge pick the refs up automatically.
2. **`reeve push <id>`** — explicit promotion. Fast-forwards `refs/heads/<branch>` from the namespaced ref, then `git push origin <branch>`. Nothing reaches GitHub until the user runs it.
3. **Meta-repo as repo zero** — the meta-repo itself participates in fetch + push. Agent commits to `repos.json`, `.devcontainer/devcontainer.json`, `CLAUDE.md` flow back via the same mechanism. No special code path; the meta-repo is just another git repo in the iteration.

The fetch + push pair is what lets the user actually use the cards as a kanban: the agent writes code, the user reviews via their normal git workflow, the user promotes when satisfied. Without this, M3/M3.5's containers are just a sandbox with no integration point back to the user's repos.

M4 deliberately does *not* yet:

- Configure fetch cadence per household (hardcoded 5 seconds in v1).
- Add `reeve fetch <id>` on-demand RPC (the worker is enough).
- Add `reeve push <id> --branch <name>` (push all branches in the namespace; UX polish later).
- Clean up namespaced refs after `→ Done` (refs persist for inspection).
- Implement service categories (`shared` / `shared-exclusive`).
- Implement the heavy-slot scheduler.
- TUI mutations or fork operations.
- Address the M2/M3/M3.5 deferred follow-ups (`--user agent`, M3-F1 reconcile-pending, M3-F5 override-config, M3.5 household-update, etc.).

## Summary

The daemon spawns a single background `FetchWorker` task at startup. Every 5 seconds the worker queries the `containers` table for `Running`/`Spawning` rows, and for each card it iterates the meta-repo + every repo declared in the household's manifest, running `git fetch <card-clone> '+refs/heads/*:refs/reeve/<card-id>/*'` in the host repo's working directory. Failures log at `warn`; the loop never panics.

When the user runs `reeve push <card-id>`, the daemon iterates the same set of host repos (meta-repo + manifest repos). For each, it lists `refs/reeve/<card-id>/*`, fast-forwards `refs/heads/<branch>` from each namespaced ref (failing-soft per-branch on non-FF), then runs `git push origin <branch>`. Output is a per-branch result table.

There is **no KB-specific code path**. The repo identified by the manifest's `knowledge_base` field is fetched and pushed identically to every other repo. If the user has wired the KB to a remote (likely), `reeve push` pushes lore-edits to that remote alongside code commits. If they have not, the per-branch result reports the push failure and exits non-zero.

## Architecture

### New modules

- `crates/reeved/src/fetch.rs` — `FetchWorker` struct and its tokio loop. Holds a clone of `DaemonState` to read the DB and call into `GitOps`. Started in `main.rs` after the daemon state is built; stopped via a oneshot channel on shutdown.

### Modified modules

- `crates/reeved/src/clones.rs` — `GitOps` trait gains four methods:
  - `fetch(workdir: &Path, source: &Path, namespace: &str) -> Result<()>`. Shells `git fetch <source> '+refs/heads/*:refs/reeve/<namespace>/*'` in `workdir`.
  - `list_namespace_refs(workdir: &Path, namespace: &str) -> Result<Vec<String>>`. Returns branch names under `refs/reeve/<namespace>/`. Shells `git for-each-ref --format='%(refname:short)' refs/reeve/<namespace>/`, strips the namespace prefix.
  - `fast_forward(workdir: &Path, target_ref: &str, source_ref: &str) -> Result<FfOutcome>`. Updates `target_ref` to point at `source_ref` only if the move is a fast-forward. Returns `Updated` or `NotFastForward`. Implementation: `git merge-base --is-ancestor <target> <source>` to test FF, then `git update-ref <target> <source>`.
  - `push(workdir: &Path, branch: &str) -> Result<()>`. Shells `git push origin <branch>` in `workdir`.

  `RealGitOps` implements all four. `MockGitOps` records calls and supports configurable per-method failure for tests.

- `crates/reeved/src/main.rs` — at startup, after daemon state is built:
  1. Spawn `FetchWorker` via `tokio::spawn`, retaining a oneshot sender for shutdown.
  2. On shutdown (after `server::run` returns), send on the worker's shutdown channel and await its join handle.

  *No Lorekeeper-presence check.* (Removed during 2026-05-15 trim — host-side Lorekeeper detection was the wrong layer; the plugin is a container-side concern owned by the user's devcontainer.json.)

- `crates/reeved/src/handlers.rs` — new `handle_push(state, params: PushParams) -> Result<Value, RpcError>`. Dispatched on `"push"`. Builds the per-branch result by walking the meta-repo + manifest repos, calling `list_namespace_refs` then `fast_forward` then `push` for each branch.

- `crates/reeve-protocol/src/lib.rs` — new types:
  - `PushParams { id: String }`
  - `PushResult { branches: Vec<BranchPushResult> }`
  - `BranchPushResult { repo: String, branch: String, status: BranchPushStatus }`
  - `BranchPushStatus { Pushed, NotFastForward, PushFailed { message: String } }` — serde-tagged enum so the CLI can render and exit-code appropriately.

- `crates/reeve/src/cli.rs` — new `Push { id: String }` subcommand. Calls the `push` RPC, prints a per-branch table, exits non-zero if any branch is not `Pushed`.

### Repo iteration helper

A small helper in `manifest.rs` (or `household.rs`):

```rust
/// Yields (repo_name, host_path) for every repo participating in a card:
/// the meta-repo itself (workspace-named, returned first), then each
/// entry in the manifest's `repos` array. Used by fetch + push.
pub fn iter_card_repos<'a>(
    household: &'a Household,
    manifest: &'a Manifest,
) -> impl Iterator<Item = (&'a str, PathBuf)> + 'a;
```

The meta-repo's "name" for fetch/push purposes is `manifest.name` (the workspace name) — that's what shows up in `BranchPushResult.repo` for commits to the meta-repo. Code-repo and KB-repo names come from `repos[].name`.

### Refspec topology

```
~/Source/myarchy-workspace/.git/refs/reeve/<card-id>/        ← meta-repo itself (repo zero)
                                       └── feat-update-manifest

~/Source/myarchy-workspace/backend/.git/refs/reeve/<card-id>/
                                       ├── feat-auth
                                       └── main

~/Source/myarchy-workspace/lore/.git/refs/reeve/<card-id>/   ← KB; no special case
                                       └── feat-document-auth-flow
```

The user's `refs/heads/` namespace is untouched until `reeve push` runs. Card branches survive `→ Done` (we don't auto-prune the namespace).

### Fetch worker

```
FetchWorker
├── tokio::task::spawn(loop {
│       tokio::select! {
│           _ = shutdown.recv() => break,
│           _ = tokio::time::sleep(Duration::from_secs(5)) => {},
│       }
│       for each Running/Spawning card:
│           let manifest = load_manifest(card.household.meta_repo_path)?;
│           for (repo_name, host_path) in iter_card_repos(&card.household, &manifest):
│               let card_clone = card_workspace.join_or_workspace_root(repo_name);
│               state.git.fetch(host_path, card_clone, card_id).await
│                 .unwrap_or_else(|e| warn!("fetch {card}/{repo_name}: {e:#}"))
│   })
```

The shutdown oneshot is held by `main.rs` and fires before the daemon exits. The select inside the loop means a pending sleep is interrupted by shutdown; total worker-exit latency is at most one in-flight fetch.

The fetch interval (5s) is hardcoded as `const FETCH_INTERVAL: Duration = Duration::from_secs(5)`. Easy to make configurable later.

### `reeve push <id>` semantics

Pseudocode for `handle_push`:

```
let household = state.households.get(&card.household_name)?;
let manifest = load_manifest(&household.meta_repo_path)?;
let mut results = Vec::new();

for (repo_name, host_repo_path) in iter_card_repos(&household, &manifest) {
    let branches = git.list_namespace_refs(&host_repo_path, &card.id).await?;
    for branch in branches {
        let target = format!("refs/heads/{branch}");
        let source = format!("refs/reeve/{}/{}", card.id, branch);
        let ff = git.fast_forward(&host_repo_path, &target, &source).await;
        match ff {
            Ok(FfOutcome::Updated) => match git.push(&host_repo_path, &branch).await {
                Ok(()) => results.push(BranchPushResult { repo: repo_name.into(), branch, status: Pushed }),
                Err(e) => results.push(BranchPushResult { repo: repo_name.into(), branch, status: PushFailed { message: e.to_string() } }),
            },
            Ok(FfOutcome::NotFastForward) => results.push(BranchPushResult { repo: repo_name.into(), branch, status: NotFastForward }),
            Err(e) => results.push(BranchPushResult { repo: repo_name.into(), branch, status: PushFailed { message: e.to_string() } }),
        }
    }
}
return PushResult { branches: results };
```

The handler returns a successful RPC result even when individual branches failed; the CLI side decides exit code by inspecting the branch statuses (any non-`Pushed` → exit 1). This keeps the RPC layer free of "partial success" complexity and lets the CLI render a meaningful table.

If the card has zero branches across all repos in its namespace (no work done yet, or fetch worker hasn't run), `results` is empty. CLI prints "(no branches in `refs/reeve/<id>/*` — has the agent committed anything yet?)" and exits 0.

### Continuous-fetch interaction with the M3 directory mount

The cards-socket directory is mounted into containers at `/var/run/reeve-cards/` (M2/M3). The card workspace (`$REEVE_CARDS_DIR/<card-id>/`) is mounted via devcontainer's `--workspace-folder` to `/workspaces/<card-id>/`. The fetch worker doesn't interact with the container — it reads from `<card-workspace>/<repo-or-empty>/.git/` on the host, which is the same directory the agent commits into via the container's bind-mount. Git's locking (`.git/index.lock`, etc.) handles concurrent reads safely; the agent writing while the daemon fetches is a normal git-from-two-processes case.

### Error semantics

- **Fetch error (worker)**: `tracing::warn!` and continue. Next iteration retries. Never panics, never blocks the loop.
- **Push error (per-branch)**: captured in `BranchPushResult` with the git stderr; CLI prints it.
- **Push error (per-repo)**: e.g., `list_namespace_refs` fails because the host repo path is gone. Returned to the CLI as an `INTERNAL` RpcError; the CLI exits non-zero with the message.
- **Card not found**: `NOT_FOUND` RpcError; CLI prints "card X not found" and exits non-zero.
- **Household no longer in the loaded map** (renamed since daemon start): `INTERNAL "household '<name>' not configured"`. User restarts daemon to re-load.
- **Manifest re-read fails mid-fetch** (user broke `repos.json` while the daemon is running): warn-log per fetch tick; worker skips the affected card until the manifest parses again.

## Testing

### Unit (clones.rs)

- `MockGitOps` extended with recording for `fetch`, `list_namespace_refs`, `fast_forward`, `push`. Configurable per-method failure.
- Tests for the new methods at the mock level: argv check, return-value mapping.

### Integration (clones.rs)

- Real-git tests using `tempfile::tempdir`:
  - `real_fetch_populates_namespaced_refs`: init host repo, init card clone (copy host), add commit to card, run `fetch`, assert host has `refs/reeve/<id>/<branch>` matching the new commit.
  - `real_fast_forward_succeeds_for_ff_case` + `real_fast_forward_rejects_non_ff`: standard FF safety.
  - `real_push_to_bare_remote`: init bare remote, push from host repo, assert bare remote has the branch.

### Unit (fetch.rs)

- `FetchWorker::tick(state)` extracted as a non-looping function that runs one iteration. Test with a mock `GitOps` and a hand-rolled containers/cards/household state. Assert: one fetch per (running card × (meta-repo + each manifest repo)). The meta-repo participates equivalently — no skip, no special case.

### Unit (handlers.rs)

- `mv_to_implementing_then_push_with_no_branches_returns_empty_result`. Uses NoopGit recording no branches; push returns empty.
- `push_records_per_branch_status`. Mock returns Updated for one branch and NotFastForward for another; push response distinguishes.
- `push_iterates_meta_repo_then_manifest_repos_in_order`. Asserts the meta-repo (repo zero) is the first entry in the result table.

### E2E (Docker-gated, in tests/e2e.rs)

- `m4_fetch_and_push_round_trip` (REEVE_TESTS_STRICT-respecting):
  1. Set up temp host meta-repo (with bare-repo "origin" remotes for both the meta-repo and one declared sibling at additional temp paths).
  2. Populate the sibling under `<meta_repo>/<sibling>/`.
  3. `reeve household add` the meta-repo.
  4. `reeve new --household ... --brief "smoke"` → `<id>`.
  5. `reeve mv <id> implementing`.
  6. `docker exec` into the container, set git config user.email/name, switch to a new branch `feat-m4-smoke` in the sibling repo, write a file, commit. Also make a commit on the meta-repo (`echo // note >> CLAUDE.md && git commit`) to exercise repo-zero participation.
  7. Wait for the fetch worker tick (up to 6s).
  8. Assert both host meta-repo and host sibling have `refs/reeve/<id>/feat-m4-smoke` (or the relevant branch name) pointing at the new commits.
  9. `reeve push <id>` from the host.
  10. Assert `refs/heads/feat-m4-smoke` exists on both the host AND the bare remote, for both repos.
  11. `reeve mv <id> done` (container teardown but refs persist).
  12. Assert `refs/reeve/<id>/feat-m4-smoke` still on the host post-Done.

- `m4_kb_repo_participates_as_peer`: extension of the round-trip with a `knowledge_base: "lore"` entry in the manifest. Agent commits to `<workspace>/lore/`, fetch picks it up under `refs/reeve/<id>/*` in the host `lore/` repo, push (against a bare remote for the lore repo) succeeds. Asserts no KB-specific code path executed.

## Open items deferred to writing-plans

- Whether `fast_forward` should be a single trait method or split into `is_ancestor` + `update_ref`. Affects mockability; pick at implementation.
- How `BranchPushStatus::PushFailed` carries the git stderr — likely a `String`, but could be more structured if useful for the CLI's output rendering.
- Whether to parallelize the per-repo fetches inside one iteration. Default to serial for predictability (mirrors M3's `clone_all_repos` choice).
- Whether the fetch worker should hold the `DaemonState` directly or just the fields it needs (DB handle + GitOps handle + households map). Hold the full state; smaller surface to plumb.
- Whether `iter_card_repos` lives in `manifest.rs` or `household.rs`. Lean: `household.rs` (it's a card-level concern that needs both the household record and the manifest).

## Out of scope (do not implement in M4)

- Anything in the parent design's M5+ deferred list.
- Configurable fetch cadence.
- `reeve fetch <id>` on-demand RPC.
- `reeve push <id> --branch <name>`.
- Auto-cleanup of `refs/reeve/<id>/*` after `→ Done`.
- Auto-archive of card workspaces.
- M2/M3/M3.5 deferred follow-ups (M3-F1, M3-F5, M3.5 household-update, etc.).
- Service categories and the heavy-slot scheduler.
- Replacing M3's `--config <user-path>` with `--override-config <synthesized>` (M3-F5).
- Lorekeeper-presence check on the host (removed during 2026-05-15 trim; not Reeve's layer).

## Verification

A merged M4 must:

1. Pass all M1+M2+M3+M3.5 tests untouched.
2. Pass the new unit, integration, and Docker-gated e2e tests.
3. `cargo test --workspace` clean (with `REEVE_TESTS_STRICT=1`); `cargo fmt --check` clean; `cargo clippy` clean.
4. Manually (with `REEVE_TESTS_STRICT=1` exported during development):
   1. Pick or scaffold a household with a `knowledge_base: "lore"` manifest and a populated `lore/` sibling (e.g., reuse the M3.5 smoke setup).
   2. `reeve daemon start`.
   3. `reeve new --household reeve-self --brief "m4 smoke"` → `<id>`.
   4. `reeve mv <id> implementing` → meta-repo + every sibling lands in `~/reeve/cards/<id>/`.
   5. `docker exec -e REEVE_CARD_ID=<id> <container> bash -lc "cd /workspaces/<id>/reeve && git checkout -b feat-m4-smoke && echo hi > smoke.txt && git add smoke.txt && git commit -m smoke"`. Also a `cd /workspaces/<id>/lore && ... && git commit -m doc` and a meta-repo commit (`cd /workspaces/<id> && echo // note >> CLAUDE.md && git commit -am note`).
   6. Wait 6 seconds.
   7. `git -C ~/Source/reeve-household log refs/reeve/<id>/feat-m4-smoke -1 --oneline` shows the meta-repo commit; same for `~/Source/reeve-household/reeve/` and `~/Source/reeve-household/lore/`.
   8. `reeve push <id>` → per-branch result table covering all three repos. With no `origin` remotes on the test repos, the pushes fail and that's the expected output; reporting matches.
   9. `reeve mv <id> done` → container down; all three `refs/reeve/<id>/feat-m4-smoke` still on disk; workspace still on disk.
