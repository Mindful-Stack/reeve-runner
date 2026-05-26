---
title: Household — registered project bundle (meta-repo + manifest + repos)
description: The Household bounded context. Manages the named project bundles Reeve operates on — a meta-repo with a household.json manifest declaring sibling repos and a knowledge base. Implements HouseholdLookup for Card.
tags: [domain, ddd, core, household, manifest, witan-household]
---

# Household

## Purpose

A Household is a *registered project bundle* — a named entry in Reeve's database pointing at a meta-repo on disk. The meta-repo follows the [[https://github.com/Mindful-Stack/witan-household|witan-household]] pattern: a single git repository containing a `household.json` manifest that declares the project's structure (which sibling repos belong, where the knowledge base lives, what devcontainer to use).

Households are how Reeve groups Cards. Every Card belongs to exactly one Household (by name reference). Multiple Cards can target the same Household (each gets its own Workspace materialization). One Household can host many Cards over time.

The Household context owns:
- The Household entity (`name`, `meta_repo_path`).
- The Manifest value object (parsed `household.json` with validation).
- The `HouseholdScaffold` port for creating a fresh household from a template (`household new`).
- The implementation of [[domain/card]]'s `HouseholdLookup::exists` port.
- The five `household.*` RPCs.

## Key entities

- **Household** — root aggregate. Identity is `name` (string, the user-chosen handle: `myarchy`, `myproject`, etc.). Holds `meta_repo_path: PathBuf` (where the meta-repo lives on disk).
- **Manifest** — parsed `household.json`. Fields:
  - `meta_repo: String` — name of the `repos[]` entry that IS the meta-repo. (Renamed from `workspace` in 2026-05; the old field name is no longer accepted.)
  - `knowledge_base: Option<String>` — name of the `repos[]` entry that holds the KB (the `lore/` directory).
  - `repos: Vec<RepoEntry>` — every repo in the workspace, including the meta-repo itself.
- **RepoEntry** — value object inside Manifest. Fields: `name` (manifest identifier + on-disk dir for siblings), `url` (informational; surfaced in `household show`), `tags` (free-form), `description`.
- **`RepoOnDisk`** (value enum, defined in `reeve-protocol`) — `Workspace` (the meta-repo entry), `Sibling` (has its own `.git/`), `Inline` (directory tracked in the meta-repo without own `.git/`), `Missing` (declared but absent on disk). Derived at `show` time; not stored.
- **`HouseholdScaffold`** — port for the git operations needed to scaffold a fresh household: `clone_template`, `init_repo`, `commit_all`, `clone_url`. Implemented by `pub(crate) RealHouseholdScaffold`.

## Ubiquitous language

- *Household* — the registered project bundle (this aggregate). The unit a Card belongs to. Not *project* (overloaded), not *workspace* (that's the per-card materialization in [[domain/workspace]]).
- *Meta-repo* — the repository that holds the `household.json`. Always one of the entries in `repos[]`; the manifest's top-level `meta_repo` field points at its name.
- *Sibling* — a non-meta repo with its own `.git/` directory inside the meta-repo root. Cloned separately at Card-spawn time. Has independent commit history from the meta-repo.
- *Inline* — a non-meta repo *without* its own `.git/`, just a tracked subdirectory of the meta-repo. Arrives with the meta-repo clone. Used for things that conceptually deserve their own name but don't need independent history (e.g., the `lore/` KB while it's small).
- *Knowledge base / KB / lore* — the inline (usually) entry that holds Lorekeeper-shaped knowledge nodes. Discovered by Lorekeeper via sibling-fallback when CC launches from the household root.
- *Witan-household* — the canonical template repo at `Mindful-Stack/witan-household`. The shape this context expects. (Reeve itself is now a witan-household: this very lore exists because `reeve household new my-thing` from the template makes one of these.)
- *`household add` vs `household new`* — `add` registers an existing meta-repo (path or URL); `new` scaffolds a fresh meta-repo from the witan-household template, then registers it.

## Invariants

These are enforced in `reeve-household/src/manifest.rs::validate` (manifest level) and at handler entry (household-level), pinned by tests.

- **Name is PK.** `households.name` is a TEXT PRIMARY KEY in SQLite. Duplicate registrations fail with a UNIQUE-constraint error mapped to `RpcError::CONFLICT`.
- **Manifest must validate before insert.** `household add` and `household new` both call `load_manifest(&meta_repo_path)` *before* writing to the households table. A bad manifest fails the registration entirely; no half-state.
- **Manifest field rename.** The Manifest struct field is named `meta_repo` (Rust + JSON). Renamed from `workspace` in 2026-05. The old field name is no longer accepted as of 2026-05-25 — manifests that still use it must be migrated (one-line `sed -i 's/"workspace":/"meta_repo":/' household.json`). New scaffolds always write `meta_repo`.
- **`meta_repo` must match a `repos[]` entry.** Manifest validation rejects a manifest whose top-level `meta_repo: "foo"` has no `repos[]` entry named `foo`. Same rule for `knowledge_base`.
- **`repos[]` is non-empty + names unique + no reserved names.** The meta-repo entry must exist (so `repos[]` is at least one entry). Names must be unique across the array. Reserved names: `.git`, `.devcontainer` (these are directory names that would collide with meta-repo internals).
- **Path-source must exist + have `.git/`.** `household add <path>` validates that the path exists and contains `.git/`; otherwise NOT_FOUND or INVALID_ARGUMENT respectively.
- **URL-source must not collide.** `household add <url>` (with optional `--clone-into`) refuses to clone into an existing directory; CONFLICT.
- **`household new` target must be empty.** The destination directory must not exist, or must be empty. Refuses CONFLICT otherwise.

## RPC surface (5 methods)

| Method | Handler | Description |
|---|---|---|
| `household.add` | `handle_household_add` | Register an existing meta-repo by path (validates `.git/` + manifest) or clone from a URL then register. Returns `HouseholdSummary`. |
| `household.new` | `handle_household_new` | Scaffold a fresh meta-repo from a template (default: `Mindful-Stack/witan-household`), substitute the name into the manifest, commit, then register. Returns `HouseholdSummary`. |
| `household.ls` | `handle_household_ls` | List registered households with their summary (name, meta-repo path, knowledge_base, repo_count). |
| `household.rm` | `handle_household_rm` | Unregister a household by name. Does NOT delete the meta-repo on disk. NOT_FOUND if not registered. |
| `household.show` | `handle_household_show` | Full detail: summary + workspace URL + per-repo classification (Workspace/Sibling/Inline/Missing) + cards referencing this household. The cards list is pre-resolved by the dispatcher via `CardRepo::for_household` — the household handler doesn't know about cards. |

## Components inside `reeve-household`

- `domain.rs` — `Household` entity (struct with `name`, `meta_repo_path` — both `pub` for now, no invariants beyond construction-time validation) + the `iter_card_repos(household, manifest, workspace_path)` helper used by [[domain/workspace]]'s push and fetch.
- `manifest.rs` — `Manifest`, `RepoEntry`, `load_manifest`, `validate`. Tests pin every validation arm.
- `repo.rs` — `HouseholdRepo` trait + `pub(crate) SqliteHouseholdRepo` adapter + `make_household_components` factory.
- `scaffold.rs` — `HouseholdScaffold` trait + `pub(crate) RealHouseholdScaffold` adapter + `make_scaffold` factory.
- `handler.rs` — the 5 RPC handlers + helpers + the `NameParam` deserialize struct.

## The "two trait views, one struct" pattern

`SqliteHouseholdRepo` implements BOTH:
- `HouseholdRepo` (the persistence port owned by this crate)
- `reeve_card::HouseholdLookup` (the cross-aggregate port owned by [[domain/card]])

The factory returns both Arcs over the same concrete instance:

```rust
pub fn make_household_components(
    conn: Arc<Mutex<Connection>>,
) -> (Arc<dyn HouseholdRepo>, Arc<dyn reeve_card::HouseholdLookup>);
```

The daemon stores both in `DaemonState`; they always point at the same backing data. This is the cleanest way to satisfy both contracts without exposing the concrete type.

## Integration points

- **Inbound from [[domain/card]]** via the `HouseholdLookup::exists(name)` port. Card uses it to validate `mv → Implementing` before mutating.
- **Inbound from [[domain/workspace]]** via `HouseholdRepo::get(name)` (look up `meta_repo_path` for provisioning) and via `iter_card_repos` (manifest walk for fetch and push).
- **Inbound RPCs:** the 5 `household.*` methods via `reeved/src/dispatch.rs`.
- **Outbound to `reeve-card` (port only):** implements `reeve_card::HouseholdLookup`. The dep is intentional — Card defines what it needs; Household supplies it.
- **Outbound to filesystem:** `HouseholdScaffold` operations via `tokio::process::Command::new("git")`.
- **Outbound to SQLite:** `HouseholdRepo` operations via `rusqlite` (via `spawn_blocking`).

## Key workflows

1. **Add an existing path:** `household.add name=X source.path=/path/to/repo` → handler validates path exists + has `.git/` → loads manifest → `HouseholdRepo::insert` → return summary.
2. **Add via URL:** `household.add name=X source.url=git@... clone_into=/dest` → handler `HouseholdScaffold::clone_url(url, dest)` → loads manifest → `HouseholdRepo::insert` → return summary.
3. **Scaffold a new household:** `household.new name=X path=/dest template=...` → handler validates dest is empty → `clone_template(template, dest)` → `init_repo(dest)` (rm `.git`, `git init -b main`) → read template's `household.json`, substitute the workspace name → `commit_all(dest, "Initial workspace")` → `HouseholdRepo::insert` → return summary.
4. **List households:** trivial pass-through to `HouseholdRepo::list`, augmented by loading each manifest for the summary fields.
5. **Show details:** `HouseholdRepo::get` → load manifest → classify each `repos[]` entry's on-disk state → construct `HouseholdDetail { summary, workspace_url, repos, cards }`. The `cards` field is supplied by the dispatcher (which calls `CardRepo::for_household(name)`); the household handler is card-blind.

## See also

- [[domain/card]] — the consumer of `HouseholdLookup` (Card needs to verify a household exists at `mv → Implementing`).
- [[domain/workspace]] — uses `HouseholdRepo::get` to resolve `meta_repo_path` at provisioning time and uses `iter_card_repos` for fetch/push iteration.
- [[frameworks/rust/inverted-dependency-ports]] — why `HouseholdLookup` lives in `reeve-card` and is *implemented* here.
- [[frameworks/rust/repo-port-adapter]] — the `HouseholdRepo` + `SqliteHouseholdRepo` + factory shape.
- [[general/architecture]] — manifest deprecation policy, schema independence, `pub(crate)` adapter discipline.
