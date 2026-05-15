# Reeve M4 — Namespaced refs + `reeve push` + Lorekeeper KB

**Status:** Design (2026-05-14)
**Predecessor:** M3 clones + `devcontainer up` (merged to `main` as `3f3aeaa`, 2026-05-13/14)
**Parent design:** `docs/superpowers/specs/2026-04-29-reeve-design.md`

## Context

M3 wired the agent-can-see-code half of the kanban loop: when a card moves to `Implementing`, the daemon clones the household's declared repos into `~/reeve/cards/<id>/<repo>/`, spawns the container via `devcontainer up`, and the agent works inside `/workspaces/<id>/`. But the host side has no view of the agent's commits — the only place they exist is in the per-card clone, invisible to the user's git tools.

M4 closes the user-can-review-code half:

1. **Continuous fetch** — the daemon periodically `git fetch`es each card's clones back into the host repo, into a private `refs/reeve/<card-id>/*` namespace. GitKraken/lazygit/sublime-merge pick the refs up automatically.
2. **`reeve push <id>`** — explicit promotion. Fast-forwards `refs/heads/<branch>` from the namespaced ref, then `git push origin <branch>`. Nothing reaches GitHub until the user runs it.
3. **Knowledge-base clone** — households gain an optional `[knowledge_base]` entry; on `→ Implementing` the KB is cloned alongside code repos using the same `--reference --dissociate` mechanism. The KB participates in fetch and push.

The fetch + push pair is what lets the user actually use the cards as a kanban: the agent writes code, the user reviews via their normal git workflow, the user promotes when satisfied. Without this, M3's containers are just a sandbox with no integration point back to the user's repo.

**Lorekeeper, plugin vs. KB:** The parent design (line 321 onward) frames Lorekeeper as a single thing reeve hard-requires. M4 treats it as two separable concerns:

- The **knowledge-base repo** (markdown / structured-knowledge files the agent reads and writes) is reeve's responsibility, provisioned via `[knowledge_base]` and cloned per-card.
- The **CC plugin** (the in-container `/lore` commands and supporting code) is the household's responsibility, provisioned via the household's devcontainer.json — base image with Lorekeeper baked in, a `postCreateCommand`, or a devcontainer feature.

Reeve never reads CC's plugin registry, never bind-mounts `~/.claude/plugins/`, and never gates startup on plugin presence. This decouples reeve from CC internals (plugin paths, `installed_plugins.json` shape, marketplace conventions) and preserves the M2 sandbox property that nothing under `~/.claude/` except the explicit allowlist (`skills/`, `settings.json`, `CLAUDE.md`) reaches the container.

M4 deliberately does *not* yet:

- Configure fetch cadence per household (hardcoded 5 seconds in v1).
- Add `reeve fetch <id>` on-demand RPC (the worker is enough).
- Add `reeve push <id> --branch <name>` (push all branches in the namespace; UX polish later).
- Clean up namespaced refs after `→ Done` (refs persist for inspection).
- Implement service categories (`shared` / `shared-exclusive`).
- Implement the heavy-slot scheduler.
- TUI mutations or fork operations.
- Address the M2/M3 deferred follow-ups (`--user agent`, M3-F1 reconcile-pending, M3-F5 override-config, etc.).
- Provision the Lorekeeper CC plugin (delegated to the household's devcontainer.json — see *Context*).

## Summary

The daemon spawns a single background `FetchWorker` task at startup. Every 5 seconds the worker queries the `containers` table for `Running`/`Spawning` rows, and for each card it iterates the household's declared repos + KB, running `git fetch <card-clone> '+refs/heads/*:refs/reeve/<card-id>/*'` in the host repo's working directory. Failures log at `warn`; the loop never panics.

When the user runs `reeve push <card-id>`, the daemon iterates the same set of host repos. For each, it lists `refs/reeve/<card-id>/*`, fast-forwards `refs/heads/<branch>` from each namespaced ref (failing-soft per-branch on non-FF), then runs `git push origin <branch>`. Output is a per-branch result table.

The `HouseholdProfile` schema gains a top-level optional `[knowledge_base]` table with one field, `path`. The KB clones into `<workspace>/<basename-of-path>/` alongside the code repos. The agent has read+write access; updates flow back to the host via the same namespaced-fetch mechanism.

Lorekeeper plugin provisioning is out of reeve's scope — see *Context*.

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

- `crates/reeved/src/household.rs` — `HouseholdProfile` gains `knowledge_base: Option<KbSpec>`. New `KbSpec { path: PathBuf }`. Tilde-expanded at load time alongside repo paths. Schema:
  ```toml
  devcontainer = "~/Source/myarchy-ops/.devcontainer/devcontainer.json"

  [[repos]]
  name = "backend"
  path = "~/Source/myarchy-backend"

  [knowledge_base]
  path = "~/Source/myarchy-kb"
  ```

- `crates/reeved/src/effects.rs::ensure_running` — `clone_all_repos` is extended (or paired with a `clone_kb` call) so the KB lands at `<workspace>/<basename-of-kb-path>/` alongside the code repos. The basename derivation strips trailing slashes; `~/Source/myarchy-kb` → `myarchy-kb`.

- `crates/reeved/src/main.rs` — at startup, after daemon state is built:
  1. Spawn `FetchWorker` via `tokio::spawn`, retaining a oneshot sender for shutdown.
  2. On shutdown (after `server::run` returns), send on the worker's shutdown channel and await its join handle.

- `crates/reeved/src/handlers.rs` — new `handle_push(state, params: PushParams) -> Result<Value, RpcError>`. Dispatched on `"push"`. Builds the per-branch result by walking the household's repos + KB, calling `list_namespace_refs` then `fast_forward` then `push` for each branch.

- `crates/reeve-protocol/src/lib.rs` — new types:
  - `PushParams { id: String }`
  - `PushResult { branches: Vec<BranchPushResult> }`
  - `BranchPushResult { repo: String, branch: String, status: BranchPushStatus }`
  - `BranchPushStatus { Pushed, NotFastForward, PushFailed { message: String } }` — serde-tagged enum so the CLI can render and exit-code appropriately.

- `crates/reeve/src/cli.rs` — new `Push { id: String }` subcommand. Calls the `push` RPC, prints a per-branch table, exits non-zero if any branch is not `Pushed`.

### Refspec topology

```
~/Source/myarchy-backend/.git/refs/reeve/<card-id>/
                                       ├── feat-auth
                                       └── main

~/Source/myarchy-kb/.git/refs/reeve/<card-id>/
                                       └── main
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
│           for each host repo (code + KB):
│               state.git.fetch(host_repo, card_clone_path, card_id).await
│                 .unwrap_or_else(|e| warn!("fetch {card}/{repo}: {e:#}"))
│   })
```

The shutdown oneshot is held by `main.rs` and fires before the daemon exits. The select inside the loop means a pending sleep is interrupted by shutdown; total worker-exit latency is at most one in-flight fetch.

The fetch interval (5s) is hardcoded as `const FETCH_INTERVAL: Duration = Duration::from_secs(5)`. Easy to make configurable later.

### `reeve push <id>` semantics

Pseudocode for `handle_push`:

```
let household = state.households.get(card.household)?;
let host_repos = household.repos.iter().map(|r| (&r.name, &r.path))
    .chain(household.knowledge_base.iter().map(|kb| (basename(&kb.path), &kb.path)))
    .collect();

let mut results = Vec::new();
for (repo_name, host_repo_path) in host_repos {
    let branches = git.list_namespace_refs(host_repo_path, &card.id).await?;
    for branch in branches {
        let target = format!("refs/heads/{branch}");
        let source = format!("refs/reeve/{}/{}", card.id, branch);
        let ff = git.fast_forward(host_repo_path, &target, &source).await;
        match ff {
            Ok(FfOutcome::Updated) => match git.push(host_repo_path, &branch).await {
                Ok(()) => results.push(BranchPushResult { repo, branch, status: Pushed }),
                Err(e) => results.push(BranchPushResult { repo, branch, status: PushFailed { message: e.to_string() } }),
            },
            Ok(FfOutcome::NotFastForward) => results.push(BranchPushResult { repo, branch, status: NotFastForward }),
            Err(e) => results.push(BranchPushResult { repo, branch, status: PushFailed { message: e.to_string() } }),
        }
    }
}
return PushResult { branches: results };
```

The handler returns a successful RPC result even when individual branches failed; the CLI side decides exit code by inspecting the branch statuses (any non-`Pushed` → exit 1). This keeps the RPC layer free of "partial success" complexity and lets the CLI render a meaningful table.

If the card has zero branches across all repos in its namespace (no work done yet, or fetch worker hasn't run), `results` is empty. CLI prints "(no branches in `refs/reeve/<id>/*` — has the agent committed anything yet?)" and exits 0.

### Lorekeeper plugin provisioning (not implemented by reeve)

The household's devcontainer.json is responsible for getting the Lorekeeper CC plugin into the container. Three workable patterns; reeve takes no opinion:

1. **Base image.** The household uses an image that has Lorekeeper baked in.
2. **`postCreateCommand`.** The household's devcontainer.json runs an install step on first creation (e.g., `curl … | sh`, `npm install -g @witan/lorekeeper`, or whatever the canonical installer is).
3. **Devcontainer feature.** If Lorekeeper publishes a devcontainer feature, the household's devcontainer.json includes it.

The KB content side IS reeve's responsibility (the `[knowledge_base]` field on `HouseholdProfile`, cloned by `effects::ensure_running` alongside code repos) — the agent inside the container reads and writes `/workspaces/<id>/<kb-name>/` regardless of whether the Lorekeeper plugin is installed; the plugin is just the CC-side UX for navigating that content.

### Continuous-fetch interaction with the M3 directory mount

The cards-socket directory is mounted into containers at `/var/run/reeve-cards/` (M2/M3). The card workspace (`$REEVE_CARDS_DIR/<card-id>/`) is mounted via devcontainer's `--workspace-folder` to `/workspaces/<card-id>/`. The fetch worker doesn't interact with the container — it reads from `<card-clone-path>/.git/` on the host, which is the same directory the agent commits into via the container's bind-mount. Git's locking (`.git/index.lock`, etc.) handles concurrent reads safely; the agent writing while the daemon fetches is a normal git-from-two-processes case.

### Error semantics

- **Fetch error (worker)**: `tracing::warn!` and continue. Next iteration retries. Never panics, never blocks the loop.
- **Push error (per-branch)**: captured in `BranchPushResult` with the git stderr; CLI prints it.
- **Push error (per-repo)**: e.g., `list_namespace_refs` fails because the host repo path is gone. Returned to the CLI as an `INTERNAL` RpcError; the CLI exits non-zero with the message.
- **Card not found**: `NOT_FOUND` RpcError; CLI prints "card X not found" and exits non-zero.
- **Household no longer in the loaded map** (renamed config since daemon start): `INTERNAL "household '<name>' not configured"`. User restarts daemon to re-load.

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

- `FetchWorker::tick(state)` extracted as a non-looping function that runs one iteration. Test with a mock `GitOps` and a hand-rolled containers/cards/household state. Assert: one fetch per (running card × declared repo).

### Unit (handlers.rs)

- New `mv_to_implementing_then_push_with_no_branches_returns_empty_result`. Uses NoopGit recording no branches; push returns empty.
- New `push_records_per_branch_status`. Mock returns Updated for one branch and NotFastForward for another; push response distinguishes.

### E2E (Docker-gated, in tests/e2e.rs)

- `m4_fetch_and_push_round_trip` (REEVE_TESTS_STRICT-respecting):
  1. Set up temp host repo (with a bare-repo "origin" remote at a second temp path).
  2. Write devcontainer.json + household TOML (one `[[repos]]` pointing at the host repo).
  3. `reeve mv <id> implementing`.
  4. `docker exec` into the container, set git config user.email/name, switch to a new branch `feat-m4-smoke`, write a file, commit.
  5. Wait for the fetch worker tick (up to 6s).
  6. Assert host repo has `refs/reeve/<id>/feat-m4-smoke` pointing at the new commit.
  7. `reeve push <id>` from the host.
  8. Assert `refs/heads/feat-m4-smoke` exists on the host AND on the bare remote.
  9. `reeve mv <id> done` (container teardown but refs persist).
  10. Assert `refs/reeve/<id>/feat-m4-smoke` still on the host post-Done.

### KB-specific tests

- Unit (household): `load_profile_with_knowledge_base_parses_kb_path`, `kb_path_tilde_expands`, `kb_path_optional_when_absent`.
- Integration (clones): `clone_all_with_kb_lands_kb_alongside_repos` (verifies the `<basename>` subdir gets the KB clone).
- E2E gets a follow-up case in the round-trip that adds a `[knowledge_base]` entry, asserts the KB lands inside the workspace and the agent can read+write it (via `docker exec`).

## Open items deferred to writing-plans

- Whether `fast_forward` should be a single trait method or split into `is_ancestor` + `update_ref`. Affects mockability; pick at implementation.
- How `BranchPushStatus::PushFailed` carries the git stderr — likely a `String`, but could be more structured if useful for the CLI's output rendering.
- Whether to parallelize the per-repo fetches inside one iteration. Default to serial for predictability (mirrors M3's `clone_all_repos` choice).
- Whether the fetch worker should hold the `DaemonState` directly or just the fields it needs (DB handle + GitOps handle + households map). Hold the full state; smaller surface to plumb.

## Out of scope (do not implement in M4)

- Anything in the parent design's M5+ deferred list.
- Configurable fetch cadence.
- `reeve fetch <id>` on-demand RPC.
- `reeve push <id> --branch <name>`.
- Auto-cleanup of `refs/reeve/<id>/*` after `→ Done`.
- Auto-archive of card workspaces.
- M2/M3 deferred follow-ups (M3-F1, M3-F5, etc.).
- Service categories and the heavy-slot scheduler.
- Replacing M3's `--config <user-path>` with `--override-config <synthesized>` (M3-F5).

## Verification

A merged M4 must:

1. Pass all M1+M2+M3 tests untouched.
2. Pass the new unit, integration, and Docker-gated e2e tests.
3. `cargo test --workspace` clean; `cargo fmt --check` clean; `cargo clippy` clean.
4. Manually (with `REEVE_TESTS_STRICT=1` exported during development):
   1. Add a `[knowledge_base]` entry to the `reeve-self` household pointing at any local git repo.
   2. `reeve daemon start`.
   3. `reeve new --household reeve-self --brief "m4 smoke"` → `<id>`.
   4. `reeve mv <id> implementing` → both the code repo AND the KB clone land in `~/reeve/cards/<id>/`.
   5. `docker exec -e REEVE_CARD_ID=<id> <container> bash -lc "cd /workspaces/<id>/reeve && git checkout -b feat-m4-smoke && echo hi > smoke.txt && git add smoke.txt && git commit -m smoke"`.
   6. Wait 6 seconds.
   7. `git -C ~/Source/reeve log refs/reeve/<id>/feat-m4-smoke -1 --oneline` → shows the commit.
   8. `reeve push <id>` → per-branch result table. With no `origin` remote on the test repo, the push will fail and that's the expected output; reporting matches.
   9. `reeve mv <id> done` → container down; `refs/reeve/<id>/feat-m4-smoke` still on disk; workspace still on disk.
