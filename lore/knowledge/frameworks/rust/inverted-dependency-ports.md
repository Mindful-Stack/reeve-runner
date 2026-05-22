---
title: Inverted-dependency ports — the consumer crate defines the trait
description: Cross-crate dependency direction is architecturally significant. Reeve places port traits in the consumer crate; supplier crates depend on the consumer and implement the trait. The Dependency Inversion Principle in Cargo.toml form.
tags: [frameworks, rust, ddd, hexagonal, dependency-inversion, ports]
---

# Inverted-dependency ports

The most counter-intuitive structural decision in Reeve's refactor: **`reeve-workspace` and `reeve-household` both depend on `reeve-card`**, not the other way around. This document explains why, what the pattern looks like, and when to use it.

## The arrangement

```
reeve-protocol
   ↑           ↑           ↑
reeve-card  ←─ reeve-workspace
   ↑       ←─ reeve-household
   |
reeved (composition root)
```

`reeve-card` is "central" in the dep graph despite being a *consumer* of behaviours that live in other crates. Why? Because `reeve-card` *defines the contracts* — the trait shapes — that those other crates implement. The supplier depends on the consumer's trait definition.

This is the **Dependency Inversion Principle** (one of SOLID's most-misunderstood letters) realised in Rust's Cargo dep graph.

## The conventional (wrong-for-DDD) direction

In a typical "service" architecture, the dep direction goes the obvious way:

```rust
// reeve-card (would have) depended on reeve-workspace:
use reeve_workspace::Workspace;
use reeve_workspace::LiveWorkspaceLifecycle;

pub async fn handle_mv(workspace: &LiveWorkspaceLifecycle, ...) { ... }
```

That looks fine — Card *calls* Workspace, so Card depends on Workspace. Problem:

- Card now drags Workspace's whole compile-graph (rusqlite, docker, git) into its tests.
- Swapping Workspace for a mock requires `LiveWorkspaceLifecycle` to be split into a trait first — but that trait lives in Workspace, so Card can't depend on the trait without depending on Workspace.
- If Workspace's API changes shape, Card's tests break even though Card's behaviour is unchanged.
- If you want to add a *second* aggregate that also calls Workspace (say, a hypothetical "scheduler"), all three crates need synchronised changes.

The fundamental problem: Card's behaviour is *defined in terms of* what it needs from Workspace, but Card doesn't *own* that definition. Workspace does.

## The inverted direction

Move the trait definition to the *consumer*:

```rust
// reeve-card/src/transitions.rs
#[async_trait]
pub trait WorkspaceLifecycle: Send + Sync {
    async fn ensure_running(&self, card: &Card) -> Result<()>;
    async fn ensure_stopped(&self, card_id: &str) -> Result<()>;
}

pub async fn on_column_change(
    old: Column, new: Column,
    card: &Card,
    workspace: &dyn WorkspaceLifecycle,
) -> Result<()> { ... }
```

Now `reeve-workspace` implements it:

```rust
// reeve-workspace/src/lifecycle.rs
use reeve_card::{Card, WorkspaceLifecycle};

pub struct LiveWorkspaceLifecycle { /* state: repo handles, ops, etc. */ }

#[async_trait]
impl WorkspaceLifecycle for LiveWorkspaceLifecycle {
    async fn ensure_running(&self, card: &Card) -> Result<()> { /* ... */ }
    async fn ensure_stopped(&self, card_id: &str) -> Result<()> { /* ... */ }
}
```

Cargo.toml in `reeve-workspace`:

```toml
[dependencies]
reeve-protocol = { path = "../reeve-protocol" }
reeve-card = { path = "../reeve-card" }   # ← the inverted dep
# ... infrastructure deps (rusqlite, tokio::process, etc.)
```

`reeve-card`'s Cargo.toml is *unchanged* — no Workspace dep.

## What this buys

- **Card defines its own needs.** The `WorkspaceLifecycle` trait is the contract Card cares about, owned by Card. Card's tests use a `MockFx` impl of the trait (also in `reeve-card`'s test module); no Workspace import needed.
- **Card stays compile-light.** No rusqlite, no docker, no git transitively. `cargo test -p reeve-card` builds in a second.
- **Workspace serves Card's contract, not its own.** When Workspace's internal architecture changes, the trait it implements stays the same — Card doesn't notice.
- **A second consumer is cheap.** If a hypothetical `reeve-scheduler` crate also needed workspace lifecycle, it would *also* define a `WorkspaceLifecycle` trait (possibly slightly different — only the methods *it* needs), and Workspace would impl both. Workspace's `Cargo.toml` would gain a second `reeve-scheduler` dep.
- **Tests pin behaviour, not coupling.** A mock in `reeve-card` doesn't pretend to be a real Workspace; it pretends to satisfy the contract Card actually depends on.

## The two ports in Reeve

`reeve-card/src/transitions.rs` defines **both** of Card's cross-aggregate ports:

```rust
#[async_trait]
pub trait WorkspaceLifecycle: Send + Sync {
    async fn ensure_running(&self, card: &Card) -> Result<()>;
    async fn ensure_stopped(&self, card_id: &str) -> Result<()>;
}

#[async_trait]
pub trait HouseholdLookup: Send + Sync {
    async fn exists(&self, name: &str) -> Result<bool>;
}
```

| Port | Defined in | Implemented by |
|---|---|---|
| `WorkspaceLifecycle` | `reeve-card/src/transitions.rs` | `reeve_workspace::LiveWorkspaceLifecycle` |
| `HouseholdLookup` | `reeve-card/src/transitions.rs` | `reeve_household::SqliteHouseholdRepo` |

Both supplier crates declare `reeve-card` as a dep. Card's Cargo.toml lists neither.

## Composition at the boundary

The implementations are wired into a single `Arc<dyn Trait>` at the daemon's composition root:

```rust
// reeved/src/main.rs
let workspaces = reeve_workspace::make_workspace_repo(db.clone());
let (households, households_lookup) = reeve_household::make_household_components(db.clone());

let workspace_lifecycle: Arc<dyn reeve_card::WorkspaceLifecycle> = Arc::new(
    reeve_workspace::LiveWorkspaceLifecycle::new(
        workspaces.clone(),
        households.clone(),
        git.clone(),
        containers.clone(),
        listeners_arc,
        claude_dir.clone(),
        api_key.clone(),
    )
);

// DaemonState holds both:
let state = DaemonState {
    cards, workspaces, households,
    household_lookup: households_lookup,
    workspace_lifecycle,
    // ...
};
```

The dispatch table then passes these into Card's handlers:

```rust
"mv" => {
    let p = parse_params::<MvParams>(req.params)?;
    reeve_card::handle_mv(
        &*state.cards,
        &*state.household_lookup,
        &*state.workspace_lifecycle,
        p,
    ).await
}
```

`reeve_card::handle_mv`'s signature names exactly what it depends on:

```rust
pub async fn handle_mv(
    cards: &dyn CardRepo,
    households: &dyn HouseholdLookup,
    workspace: &dyn WorkspaceLifecycle,
    params: MvParams,
) -> Result<Value, RpcError>;
```

## When to use this pattern

When a context needs to call into a sibling context (cross-aggregate work), and:

1. There's a clean, narrow contract the consumer cares about (not a full repo or a big API surface).
2. You can imagine a useful mock — i.e., the contract is small enough that a test impl fits in 10 lines.
3. The supplier crate is already free to depend on the consumer (no cyclic risk).

If any of those fail, consider:

- **Trait too wide?** Split into smaller traits, each with one consumer.
- **No useful mock?** Maybe the contract is too tightly coupled to implementation details — extract pure value-types into the consumer's domain, push impl-specific work back into the supplier.
- **Cycle risk?** Reconsider the bounded-context boundary. Two aggregates that need each other might be one aggregate with internal collaboration.

## Common mistakes

- **Defining the trait in the supplier crate** — this is the conventional direction. Tempting because the impl is "naturally" in that crate. Resist; move the trait to the consumer.
- **Re-defining the trait in `reeve-protocol`** — protocol is for *wire shapes*, not for domain contracts. Putting `WorkspaceLifecycle` there leaks daemon-internal abstractions into the CLI.
- **Holding the supplier crate's concrete type in the consumer** — defeats the inversion. The consumer must hold `&dyn Trait` or `Arc<dyn Trait>`, never `LiveWorkspaceLifecycle`.

## See also

- [[general/architecture]] for the broader rules; this pattern is rule #1 ("cross-context coupling only through ports").
- [[frameworks/rust/repo-port-adapter]] for the within-crate version of this (where the port and impl live in the same crate).
- [[domain/card]] §"Key entities" for the actual ports Card consumes.
