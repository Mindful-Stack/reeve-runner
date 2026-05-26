---
title: Testing strategy
description: How Reeve tests are organised. Per-crate unit tests with in-memory SQLite and mock ports. Cross-crate integration tests in the tests/ crate. Docker-gated e2e tests for the full container lifecycle. Drift protection for the wire/domain γ boundary.
tags: [general, testing, rust, tdd, mocks]
---

# Testing strategy

Reeve's tests live in four layers, each pinning a different kind of guarantee. Knowing where to put a new test (and where to look when one fails) matters more than how many tests we have.

## The four layers

| Layer | Where | What it pins |
|---|---|---|
| **Domain unit tests** | `<crate>/src/<module>.rs::tests` inline | Pure logic: invariants, policy decisions, value-object conversions. No infra deps. |
| **Adapter unit tests** | `<crate>/src/repo.rs::tests` and `<crate>/src/git.rs::tests` etc. | The SQL/subprocess interface itself. Uses in-memory SQLite (`":memory:"`) and real `git`/`docker` calls (the latter gated on availability). |
| **Handler / integration tests** | `<crate>/src/handler.rs::tests` and `reeved/src/dispatch.rs::tests` | RPC dispatch routing, error code semantics, port composition. Uses mock impls of port traits to assert handler behaviour without real I/O. |
| **End-to-end** | `tests/e2e.rs` (the workspace `tests` crate) | The daemon binary, real Unix socket, real CLI, real (or gated) docker. Pins user-visible behaviour. |

## What lives where

### Domain unit tests — pure

- `reeve-card/src/domain.rs::tests`: `Card::new` invariants, `Card::from_persisted` invariants, round-trip with `protocol::Card`.
- `reeve-card/src/transitions.rs::tests`: every arm of the kanban transition matrix, asserted via a `MockFx` impl of `WorkspaceLifecycle` that records calls.
- `reeve-household/src/manifest.rs::tests`: every validation arm.
- `reeve-workspace/src/domain.rs::tests`: Workspace/Container entity invariants (currently minimal; expand as the entities grow methods).
- `reeve-protocol/src/lib.rs::tests`: wire-string round-trip for `Column` + `ContainerStatus`, RPC envelope serde tests, path helpers.

These tests run in milliseconds. No `tokio` runtime, no SQLite, no subprocess. If a test in this layer is slow, it's mis-categorised.

### Adapter unit tests — real I/O, isolated

- Each `repo.rs::tests` opens `Connection::open_in_memory()`, runs `init_schema`, and exercises the trait surface against a fresh DB per test. Tests can interleave because each owns its connection.
- `reeve-workspace/src/git.rs::tests`: integration tests against a real `git` subprocess on a `tempfile::tempdir`. Tagged docker-availability via `if !git_available() { return; }` — but git is universally available, so they always run.
- `reeve-workspace/src/container.rs::docker_tests::*`: real `docker` calls. Gated on `docker_available()` — they skip if docker is missing (CI without docker-in-docker, fresh dev machines). Each test names a container with a unique sentinel name to avoid colliding with parallel runs.
- `reeve-workspace/src/clones.rs::integration_tests`: real `git clone --reference --dissociate` against fresh tempdirs.

The schema-independence rule (no FK on `containers.card_id`) is pinned by `reeve-workspace/src/repo.rs::tests::no_fk_no_cards_row_needed` — insert a containers row without any cards row and expect success. If a future change accidentally re-introduces the FK, this test catches it.

### Handler / integration tests — port composition

- `reeve-card/src/handler.rs::tests` (sparse — handler bodies are thin enough that domain + repo coverage usually suffices).
- `reeve-household/src/handler.rs::tests`: scenarios like `household_add_via_path_succeeds`, `household_show_classifies_each_entry`, `household_new_refuses_non_empty_target`.
- `reeved/src/dispatch.rs::tests`: routes RPC requests through the full dispatch table with mocked port impls. These are the highest-leverage integration tests we have — they exercise cross-crate routing without booting the daemon.

The dispatch tests build a fresh `DaemonState` with mocked port impls per test. The pattern:

```rust
fn fresh_state() -> DaemonState {
    let conn = app::lifecycle::open_db(":memory:").unwrap();
    let db: DbHandle = Arc::new(Mutex::new(conn));
    DaemonState {
        db: db.clone(),
        cards: reeve_card::make_card_repo(db.clone()),
        workspaces: reeve_workspace::make_workspace_repo(db.clone()),
        household_repo: /* mocked or real */,
        household_lookup: /* same mocked or real */,
        workspace_lifecycle: Arc::new(NoopLifecycle),
        git: Arc::new(NoopGit),
        containers: Arc::new(NoopOps),
        listeners: ListenerRegistry::new(...),
    }
}
```

The mock structs (`NoopLifecycle`, `NoopGit`, `NoopOps`, `NoopDispatcher`) live in `#[cfg(test)] mod tests` blocks within each crate. They're not exported. Promote to `*-test-support` modules only if duplication becomes painful — currently it isn't.

### End-to-end — tests/e2e.rs

The workspace `tests` crate boots the actual `reeved` binary on a temporary socket, runs CLI commands against it, and asserts on observable behaviour. Tests are organised by milestone (`m1_*`, `m2_*`, ..., `m6_*`).

Docker-gated tests (M3, M4, M5, M6) skip when `docker` is unavailable in the environment. CI runs them in a Docker-in-Docker environment; local runs may skip them silently.

The M3.5 lore-subdir test (`m3_5_lore_subdir_lands_in_workspace`) has flaked historically on environments where `devcontainer up` fails for reasons external to Reeve. Treat docker-related failures as environmental until evidence suggests otherwise.

## Mock conventions

- **One mock per port, per crate** (`Noop` for the "doesn't matter" path, `Recording` for tests that need to assert on calls, `Configurable` for tests that simulate specific outcomes).
- Mocks live in the test module of the file that defines the trait (or, for cross-crate mocks, in the test module of the *consumer* file). No `*-mocks` crate yet — the duplication has been ≤2 sites for each mock kind.
- Mocks are `pub` only within `#[cfg(test)] mod tests`; nothing leaks to release builds.

## Drift protection

The wire/domain γ boundary in `reeve-card` (where `domain::Card` and `protocol::Card` coexist) is pinned by:

```rust
#[test]
fn protocol_to_domain_to_protocol_round_trips() {
    let dto = /* sample protocol::Card */;
    let entity = Card::try_from(dto.clone()).unwrap();
    let dto2: reeve_protocol::Card = entity.into();
    assert_eq!(dto, dto2);
}
```

If a field is added to either type, this test (plus `cargo check`) catches missing field handling on either side. Don't delete this test; if you add a Card field, update both types and the round-trip should still pass.

Similar round-trip tests exist for `Manifest` validation in `reeve-household`.

## Running tests

| Goal | Command |
|---|---|
| Domain + adapter + handler unit tests, no docker | `cargo test --workspace --lib` |
| Single crate, including bin tests | `cargo test -p reeved` |
| Single test | `cargo test -p reeve-card -- domain::tests::new_card_in_backlog` |
| Including ignored / gated e2e | `cargo test --workspace -- --include-ignored` |
| Just the M5 e2e (docker required) | `cargo test --test e2e -- m5_attach` |
| Build artefacts only | `cargo build --workspace` |
| Clippy + fmt + isolation lint, the full doctor pass | `make doctor` (runs `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `scripts/check-context-isolation.sh`) |

CI matrices: the per-crate `cargo test -p` runs are split per crate for parallelism; the e2e crate runs last with docker available.

## What we don't test

A short list of tested-but-light areas, deliberately:

- **TUI rendering pixels.** Ratatui draw outputs aren't pinned. Behaviour tests for key bindings (in `tui/events.rs`) cover the binding → Action mapping; the render output is visual review.
- **Subprocess exit codes beyond success/failure.** `RealGitOps` checks `output.status.success()` but doesn't pin specific exit codes per failure mode.
- **Cargo workspace dep graphs.** Trust that Cargo.toml is the source of truth; we don't have a test asserting "reeve-card has no rusqlite dep transitively." The `pub(crate)` discipline + the CI isolation lint are the enforcement.

## See also

- [[general/architecture]] for the rules tests are pinning.
- [[frameworks/rust/repo-port-adapter]] for the trait + adapter pattern that drives the mock shapes.
- [[domain/card]] / [[domain/workspace]] / [[domain/household]] for invariants tests assert.
