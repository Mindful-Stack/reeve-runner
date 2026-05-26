# Reeve

Reeve is a Rust-based, daemon-driven kanban orchestrator for parallel Claude Code sessions. This directory is a **witan-household meta-repo**: it bundles the daemon (`crates/reeved`), CLI/TUI client (`crates/reeve`), three bounded-context crates (`reeve-card`, `reeve-workspace`, `reeve-household`), the wire protocol (`reeve-protocol`), and an inline knowledge base at `lore/`.

## Reading order for new contributors

1. **Domain and standards** — start with `lore/knowledge/general/architecture.md` (the rules every change follows), then read the three domain nodes:
   - `lore/knowledge/domain/card.md` — the Card aggregate (kanban metadata + column transitions)
   - `lore/knowledge/domain/workspace.md` — per-card materialization (clones + container + fetch)
   - `lore/knowledge/domain/household.md` — registered project bundles (manifest + scaffold)
2. **Rust patterns** — `lore/knowledge/frameworks/rust/` has 4 nodes covering the recurring patterns (port-adapter, inverted-dep ports, async-over-sync rusqlite, γ type pattern).
3. **Specs and plans** — `docs/superpowers/specs/` (point-in-time design docs per milestone) and `docs/superpowers/plans/` (implementation plans). Read these for history; don't treat them as living documentation.

Lorekeeper (`/plugin install lorekeeper@witan`) finds `lore/` automatically when CC launches from this directory. Use `/lore:help`, `/lore:prime <node>`, `/lore:explore`, `/lore:review`, `/lore:doctor`.

## Layout

```
reeve/                              ← this directory (the meta-repo)
├── Cargo.toml                      ← workspace root: 6 crates
├── household.json                  ← witan-household manifest
├── crates/                         ← the source
│   ├── reeve-protocol/             ← wire DTOs, paths::, lifecycle helpers
│   ├── reeve-card/                 ← Card aggregate + ports
│   ├── reeve-workspace/            ← Workspace aggregate (impls WorkspaceLifecycle)
│   ├── reeve-household/            ← Household aggregate (impls HouseholdLookup)
│   ├── reeved/                     ← daemon binary (composition root)
│   └── reeve/                      ← CLI + TUI
├── lore/                           ← knowledge base (inline; tracked here)
│   ├── knowledge/                  ← domain/, general/, frameworks/, languages/, learnings/
│   └── _tools/                     ← build-index + validators (run via make)
├── docs/superpowers/               ← specs and plans (per-milestone design history)
├── scripts/                        ← check-context-isolation.sh + witan-household helpers
├── tests/                          ← cross-crate e2e
└── .devcontainer/                  ← for Reeve cards spawned from this repo
```

## Adding things — quick map

- **A new RPC method:** pick the owning aggregate; add the handler in that crate's `handler.rs`; re-export from `lib.rs`; add a match arm in `crates/reeved/src/dispatch.rs`. If it changes a card's column state, the dispatch arm must acquire `state.lock_card(&id).await` first. See `lore/knowledge/general/architecture.md` for the full rules.
- **A new cross-aggregate port:** define the trait in the *consumer* crate; implement in the *supplier* crate (which depends on the consumer). See `lore/knowledge/frameworks/rust/inverted-dependency-ports.md`.
- **A new repository:** trait + `pub(crate)` adapter + factory returning `Arc<dyn Trait>`. All three live in `<crate>/src/repo.rs`. See `lore/knowledge/frameworks/rust/repo-port-adapter.md`.
- **A new TUI binding:** edit `crates/reeve/src/tui/events.rs::BINDINGS`, add an `Action` variant, handle it in `crates/reeve/src/tui/actions.rs::handle`. Render layer in `render.rs` consumes `&App`.

## Shell hygiene for the household layout

- Use `git -C <subdir>` instead of `cd <subdir> && git ...` when working with sibling repos (none today, but the convention applies if you add some).
- Prefer relative paths over absolute paths when the target is inside the workspace.

## Build and verify

| Goal | Command |
|---|---|
| All unit + lib tests | `cargo test --workspace --lib && cargo test -p reeved` |
| Format check | `cargo fmt --check` |
| Lints (deny warnings) | `cargo clippy --workspace --all-targets -- -D warnings` |
| Context isolation lint | `scripts/check-context-isolation.sh` |
| Lore validators (frontmatter, links, orphans) | `make validate` |
| Full diagnostic (workspace + KB) | `make doctor` |

Run all of the above before opening a PR. See `lore/knowledge/general/testing.md` for the four test layers and what each pins.

## Commit conventions

Imperative + lowercase + conventional-commit prefix with milestone or task number: `feat(m6): ...`, `refactor(N.X): ...`, `fix(m3-followup): ...`, `chore: ...`. Heredoc body for non-trivial commits. `Co-Authored-By: Claude` trailer on agent-coauthored commits. Full spec at `lore/knowledge/general/commit-conventions.md`.

## Manifest schema migration

`Manifest.workspace` was renamed to `Manifest.meta_repo` in 2026-05. The
serde alias and deprecation-warning phase ran through 2026-05-25; the
alias was then removed across all three Witan tools (Reeve, Lorekeeper,
witan-household). Manifests using the old `workspace` field name no
longer parse — they must be migrated with
`sed -i 's/"workspace":/"meta_repo":/' household.json`. See
`lore/knowledge/general/architecture.md` and the 2026-05-25 spec for
the cross-tool rollout history.

## Refactor history

- `docs/superpowers/specs/2026-04-29-reeve-design.md` — parent design
- `docs/superpowers/specs/2026-05-18-domain-driven-refactor-design.md` — the spec that produced the bounded-context structure
- `docs/superpowers/plans/2026-05-18-domain-driven-refactor.md` — the implementation plan executed across the 2026-05-18/19 branch
- Each milestone (M1–M6) has its own spec + plan in `docs/superpowers/`
