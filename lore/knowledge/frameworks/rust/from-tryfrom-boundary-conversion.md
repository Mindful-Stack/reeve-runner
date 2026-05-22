---
title: γ type pattern — domain entity + wire DTO + From/TryFrom boundary
description: How Reeve bridges a typed domain entity (private fields, invariants enforced at construction) with a serde-derived wire DTO. From in one direction always succeeds; TryFrom in the other direction re-runs invariants. Validation lives at the boundary; once you hold the domain type, it's known-valid.
tags: [frameworks, rust, ddd, serde, types, validation]
---

# γ pattern — domain entity vs wire DTO

The Card aggregate is the only one in Reeve that uses the "γ" type pattern: two distinct types (`reeve_card::Card` and `reeve_protocol::Card`) with `From`/`TryFrom` bridging them at the wire boundary. This document explains the mechanics, when to use it, and when not to.

## The pattern

```rust
// crate: reeve-protocol/src/lib.rs (the wire DTO)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    pub id: String,
    pub household: String,
    pub brief: String,
    pub column: Column,
    pub created_at: i64,
    pub updated_at: i64,
    pub needs_heavy: bool,
    pub parent_id: Option<String>,
}
```

```rust
// crate: reeve-card/src/domain.rs (the domain entity)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    id: String,              // ← private
    household: String,       // ← private
    brief: String,           // ← private
    column: Column,          // ← private
    created_at: i64,
    updated_at: i64,
    needs_heavy: bool,
    parent_id: Option<String>,
}

impl Card {
    /// Create a fresh card in Backlog with a UUID + current time.
    pub fn new(household: String, brief: String, needs_heavy: bool) -> Result<Self, CardError> {
        if brief.is_empty() { return Err(CardError::EmptyBrief); }
        let n = brief.chars().count();
        if n > 200 { return Err(CardError::BriefTooLong(n)); }
        let now = now_secs();
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            household, brief,
            column: Column::Backlog,
            created_at: now, updated_at: now,
            needs_heavy,
            parent_id: None,
        })
    }

    /// Construct from an already-validated row (used by the repo when reading SQL).
    pub fn from_persisted(
        id: String,
        household: String,
        brief: String,
        column: Column,
        created_at: i64,
        updated_at: i64,
        needs_heavy: bool,
        parent_id: Option<String>,
    ) -> Result<Self, CardError> {
        if id.is_empty() { return Err(CardError::EmptyId); }
        if brief.is_empty() { return Err(CardError::EmptyBrief); }
        let n = brief.chars().count();
        if n > 200 { return Err(CardError::BriefTooLong(n)); }
        Ok(Self { id, household, brief, column, created_at, updated_at, needs_heavy, parent_id })
    }

    // Getters
    pub fn id(&self) -> &str { &self.id }
    pub fn brief(&self) -> &str { &self.brief }
    pub fn column(&self) -> Column { self.column }
    pub fn household(&self) -> &str { &self.household }
    pub fn created_at(&self) -> i64 { self.created_at }
    pub fn updated_at(&self) -> i64 { self.updated_at }
    pub fn needs_heavy(&self) -> bool { self.needs_heavy }
    pub fn parent_id(&self) -> Option<&str> { self.parent_id.as_deref() }
}

// Domain → wire: always succeeds (domain Card is already valid).
impl From<Card> for reeve_protocol::Card {
    fn from(c: Card) -> Self {
        reeve_protocol::Card {
            id: c.id,
            household: c.household,
            brief: c.brief,
            column: c.column,
            created_at: c.created_at,
            updated_at: c.updated_at,
            needs_heavy: c.needs_heavy,
            parent_id: c.parent_id,
        }
    }
}

// Wire → domain: may fail (re-runs invariants on inbound wire data).
impl TryFrom<reeve_protocol::Card> for Card {
    type Error = CardError;
    fn try_from(d: reeve_protocol::Card) -> Result<Self, CardError> {
        Self::from_persisted(
            d.id, d.household, d.brief, d.column,
            d.created_at, d.updated_at, d.needs_heavy, d.parent_id,
        )
    }
}
```

## Why two types

### The wire DTO needs to be liberal

`reeve_protocol::Card` is what flies over JSON-RPC, gets persisted to SQLite, deserializes from incoming requests, serializes to outgoing responses. It needs:

- `#[derive(Serialize, Deserialize)]` — serde infrastructure.
- Public fields — so serde can write to them during deserialization without builder boilerplate.
- *No* invariant enforcement — serde would fail noisily if `brief` had to be non-empty at deserialize time; instead, deserialize succeeds and validation happens explicitly at the boundary.

### The domain entity needs to be strict

`reeve_card::Card` is what handlers, repos, and the transition policy hold. Once a function holds a `reeve_card::Card`, it should be able to assume the invariants are met (brief is non-empty, id is non-empty, etc.). Private fields + factory constructors guarantee this; no caller can construct an invalid instance.

### The boundary is where validation happens

- **Wire arrives** (an RPC request, a SQL row, a serde-deserialized struct) → `try_from` re-runs invariants → `Result<domain::Card, CardError>`. If invariants fail, we return an `RpcError` to the caller (or fail the SQL row decode). The domain layer never sees an invalid Card.
- **Wire departs** (the handler returns to the dispatcher) → `domain::Card.into() -> reeve_protocol::Card` → serialize. Trivial, always succeeds, because the domain Card was already valid.

The asymmetry (`From` one way, `TryFrom` the other) reflects the directionality of trust: outgoing data has been vetted; incoming data has not.

## Drift protection

The two types must stay in field-sync. Pinned by:

```rust
#[test]
fn protocol_to_domain_to_protocol_round_trips() {
    let dto = reeve_protocol::Card {
        id: "abc".into(), household: "h".into(), brief: "test".into(),
        column: Column::Backlog, created_at: 1000, updated_at: 1000,
        needs_heavy: false, parent_id: None,
    };
    let entity = Card::try_from(dto.clone()).unwrap();
    let dto2: reeve_protocol::Card = entity.into();
    assert_eq!(dto, dto2);
}
```

If you add a field to `reeve_protocol::Card`, this test fails to compile (the `From` impl is missing the field). If you add a field to `reeve_card::Card`, the `TryFrom` impl is missing it. The compiler catches drift; the runtime test pins identity of the round trip.

## When to use γ — and when not

The decision tree:

### Use γ when

- The type is transmitted **whole** over the wire (entire struct serialized in an RPC body or response, like `Card` in `NewResult`, `MvResult`, `LsResult`).
- The type has **non-trivial invariants** worth enforcing at construction (brief length, id non-empty, etc.).
- The domain layer wants **encapsulation** (private fields, methods, ability to refactor internals without breaking the wire contract).

### Don't use γ when

- The type is **never sent whole** — only projections. E.g. `Workspace` is never transmitted; the wire surface is `StatusSelfResult { container_status: Option<ContainerStatus> }`. A read-model projection. No γ needed.
- The type is a **value object with no behaviour** — `Column`, `ContainerStatus`, `RepoOnDisk`. Just put them in `reeve-protocol` with `#[derive(Serialize, Deserialize)]` and use directly.
- The type is **always read-only at the boundary** — like a request DTO that maps to a domain command. Use the wire type directly in the handler.

In Reeve today:

| Aggregate | Domain entity | Wire DTO | Conversion |
|---|---|---|---|
| `Card` | `reeve_card::Card` (private fields, methods) | `reeve_protocol::Card` | `TryFrom` (wire→domain, re-validates), `From` (domain→wire) |
| `Workspace` | `reeve_workspace::Workspace` | none | Projections (`StatusSelfResult.container_status`) built in handlers |
| `Household` | `reeve_household::Household` | none | Projections (`HouseholdSummary`, `HouseholdDetail`) built in handlers |
| `Manifest` | `reeve_household::Manifest` | (same struct, no separate DTO) | Validation in `manifest::validate`; serde just reads JSON |

`Manifest` is a borderline case — same struct, but validation is explicit (`load_manifest` calls `validate`). The single-type-with-validation pattern works when the type isn't shared across crates with different invariant strictness.

## The asymmetry across aggregates

You might ask: "Card has γ, but Workspace and Household have only one struct. Why the inconsistency?"

The answer is in what's transmitted, not in some philosophical preference:

- **Card** flies whole over the wire because the CLI/TUI display the whole entity. So both representations must exist and be reconcilable.
- **Workspace** is never displayed whole — the wire side shows only `container_status: Option<ContainerStatus>`, which is a projection assembled by the handler. There's no `reeve_protocol::Workspace`.
- **Household** is partly visible — `HouseholdSummary` and `HouseholdDetail` are projections, not whole-entity snapshots. There's no `reeve_protocol::Household`.

So γ is applied where the wire-shape and domain-shape genuinely coincide as the same logical entity. For everything else, the wire side is a projection that the handler constructs ad-hoc.

## Common mistakes

- **Public fields on the domain entity.** `pub id: String, pub brief: String, ...` — defeats the encapsulation. Now callers can mutate fields without going through methods, and invariants can be violated post-construction. Keep fields private; add getters.
- **`From` in both directions.** `From<protocol::Card> for domain::Card` (instead of `TryFrom`) — silently accepts invalid wire data. The invariants are now optional. Use `TryFrom` for any direction that needs validation.
- **Re-validating in `From` (domain → wire).** If the domain type is correctly constructed, `From` should be infallible. Adding a validation step there is a code smell — fix the constructor instead.
- **Forgetting to re-export `CardError`.** When `try_from` fails, the caller needs the error type. `reeve-card/src/lib.rs` re-exports `pub use domain::{Card, CardError};`.

## See also

- [[domain/card]] — the only current use of this pattern.
- [[general/architecture]] §"Type strategy" — the rule that motivated γ.
- [[general/testing]] §"Drift protection" — the round-trip test that pins the boundary.
- [[frameworks/rust/repo-port-adapter]] — the repo trait uses `domain::Card`; the adapter converts at the row boundary via `from_persisted`.
