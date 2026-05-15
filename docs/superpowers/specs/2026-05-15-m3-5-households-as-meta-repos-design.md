# Reeve M3.5 — Households as meta-repos

**Status:** Design (2026-05-15)
**Predecessor:** M3 clones + `devcontainer up` (merged to `main` as `3f3aeaa`, 2026-05-13/14)
**Successor:** M4 — namespaced refs + `reeve push` (rewritten on top of M3.5)
**Parent design:** `docs/superpowers/specs/2026-04-29-reeve-design.md`

## Context

M3 introduced per-card containers via `devcontainer up`, per-card clones of the household's declared repos at `~/reeve/cards/<id>/<repo>/`, and the `[[repos]]` schema in `HouseholdProfile`. It treats a household as a TOML file authored by hand (`~/.config/reeve/households/<name>.toml`), holding a list of absolute filesystem paths and a `devcontainer` field pointing at an arbitrary `devcontainer.json` somewhere on disk.

The original M4 design (`2026-05-14-m4-refs-and-push-and-kb-design.md`) proposed bolting a `[knowledge_base]` table onto that schema to teach Reeve about the Lorekeeper KB. Brainstorming on 2026-05-15 surfaced that this bolt-on was symptomatic of a deeper modelling problem: a household is conceptually a *bundle* — repos + KB + devcontainer + workspace-level guidance — and Reeve was modelling that bundle as a private config file rather than as a real, versioned, shareable artefact.

The `gp-workspace` pattern in `~/Source/ramudden/` showed a strong alternative: the household *is* a meta-repo that owns the manifest, the bootstrap, the workspace-level `CLAUDE.md`, and `.devcontainer/`. Sibling repos clone alongside as gitignored entries. The KB is just another sibling. The whole bundle becomes a single `git clone`-able thing.

M3.5 adopts this pattern. M4 then drops `[knowledge_base]`, `KbSpec`, basename-derivation, and the host-side Lorekeeper-presence warning entirely — the KB participates in fetch + push as a peer of any other repo, no special-casing.

## Summary

A **household** is a user-cloned (or Reeve-scaffolded) meta-repo containing:

- `repos.json` — manifest declaring the workspace name, the KB pointer, and the sibling repos
- `.devcontainer/devcontainer.json` — devcontainer config at the conventional location
- `CLAUDE.md` — workspace-level guidance
- `.gitignore` — excludes sibling repo dirs
- Sibling repos cloned alongside (gitignored), one of which carries the KB at the conventional name `lore/`

Reeve's `households` table stores `(name, meta_repo_path)`. Cards clone the meta-repo *and* the siblings it declares into `~/reeve/cards/<id>/`, mounting that as the container workspace. The devcontainer.json found at the conventional location drives `devcontainer up`.

A separate template repo at `Mindful-Stack/witan-household` provides the canonical starter. Both Reeve (`reeve household new`) and Lorekeeper (standalone use via the template repo's "Use this template" button or `git clone`) consume it. The template is tool-agnostic — Lorekeeper-only users get a working setup without Reeve.

The Lorekeeper plugin gains `./lore` and `./docs/lore` in its sibling-fallback resolution list (parallel Lorekeeper PR, not gated on Reeve M3.5 shipping). With the meta-repo containing a `lore/` sibling at the workspace root, Lorekeeper's existing fallback resolves the KB automatically inside the container — Reeve injects no env vars, writes no `.lorekeeper/config.json`, and creates no symlinks.

## Architecture

### Manifest schema

`repos.json` at the meta-repo root:

```json
{
  "name": "myarchy-workspace",
  "knowledge_base": "lore",
  "repos": [
    { "name": "backend",  "url": "git@github.com:Mindful-Stack/myarchy-backend.git",  "tags": ["backend", "core"] },
    { "name": "frontend", "url": "git@github.com:Mindful-Stack/myarchy-frontend.git", "tags": ["frontend"] },
    { "name": "lore",     "url": "git@github.com:Mindful-Stack/myarchy-lore.git",     "tags": ["docs"] }
  ]
}
```

Fields:

- `name` (string, optional) — workspace friendly name. Defaults to meta-repo directory basename. Used in logs, `reeve household ls`, commit messages.
- `knowledge_base` (string, optional) — references one of `repos[].name`. Cardinality-one by type. If set, the named entry MUST exist in `repos` (validated at load). If absent, no KB is wired; Lorekeeper inside the container falls through to its other resolution paths or reports no KB configured.
- `repos` (array) — sibling repos. Each entry:
  - `name` (string, required) — directory name on disk relative to the meta-repo root; also the manifest identifier.
  - `url` (string, optional) — git URL. Surfaced in `reeve household show`; consumed by user-owned bootstrap scripts (a `make setup` analogue) and by the template repo's documentation. NOT resolved by Reeve at card-spawn time; sibling sources come from `<meta-repo>/<entry.name>/.git` on the host.
  - `tags` (array of strings, optional) — categorization (`backend`, `frontend`, `core`, `docs`, etc.). KB-marker is NOT a tag — that lives in the top-level `knowledge_base` field.
  - `description` (string, optional) — human-readable, surfaced in `reeve household show`.

Validation (at manifest load, fail-fast):

1. JSON parses.
2. `repos` is an array (may be empty).
3. Every `repos[].name` is non-empty and unique.
4. If `knowledge_base` is set, it matches exactly one `repos[].name`.
5. No `repos[].name` matches a reserved name (`.git`, `.devcontainer`). Other collisions with existing files (e.g., a repo entry named `CLAUDE.md`) fail at clone-time with the underlying git error; they're not caught at manifest-load.

### Host-side topology

```
~/Source/myarchy-workspace/         ← meta-repo (user-cloned or `reeve household new`)
├── .git/
├── repos.json
├── .devcontainer/
│   └── devcontainer.json
├── CLAUDE.md
├── .gitignore                       ← excludes sibling dirs
├── backend/                         ← sibling clone (user-managed; populated by user's bootstrap)
│   └── .git/
├── frontend/                        ← sibling clone
│   └── .git/
└── lore/                            ← KB sibling; `knowledge_base: "lore"` in manifest
    └── .git/
```

Sibling clones inside the meta-repo are the **canonical source** for Reeve's `git clone --reference --dissociate` operations. Reeve never resolves manifest `url` fields at card-spawn time — it expects the host meta-repo to already contain each declared sibling as a populated subdirectory. If a sibling subdir is missing, that's a hard error at `→ Implementing` with a message telling the user to populate it (typically by running their meta-repo's bootstrap, e.g. `make setup`).

### Card-side topology

```
~/reeve/cards/<card-id>/             ← card workspace (mount root for devcontainer)
├── .git/                            ← per-card clone of meta-repo, --reference --dissociate from host meta-repo
├── repos.json                       ← (copy travels with meta-repo clone)
├── .devcontainer/
│   └── devcontainer.json            ← devcontainer up --workspace-folder uses this
├── CLAUDE.md
├── .gitignore
├── backend/                         ← per-card clone of backend, --reference --dissociate from host backend
│   └── .git/
├── frontend/
│   └── .git/
└── lore/                            ← per-card clone of lore; Lorekeeper sibling-fallback finds it
    └── .git/
```

Inside the container, this directory is bind-mounted at `/workspaces/<card-id>/`. The devcontainer.json at `<workspace>/.devcontainer/devcontainer.json` is found by `devcontainer up` via its standard convention — no manifest field needs to point at it.

The meta-repo participates in fetch + push as **repo zero** alongside siblings. Agent commits to `repos.json`, `.devcontainer/devcontainer.json`, `CLAUDE.md` flow back to the user's host meta-repo via `refs/reeve/<card-id>/*` in M4.

### Lorekeeper interaction (no Reeve mechanism)

The Lorekeeper plugin, if the user's `devcontainer.json` installs it (via `postCreateCommands`, base image, or devcontainer features), discovers the KB via its existing resolution order. M3.5 lands together with a parallel Lorekeeper PR that adds `./lore` and `./docs/lore` to Lorekeeper's sibling-fallback list. With the container's CWD at `/workspaces/<card-id>/` and a `lore/` sibling present, Lorekeeper finds the KB automatically.

Reeve injects no `KNOWLEDGE_BASE_PATH`, writes no `.lorekeeper/config.json`, and creates no symlinks. The integration is purely structural: provide the meta-repo + `lore/` sibling, Lorekeeper handles the rest.

Users who want to override (e.g., a KB named other than `lore`) set `containerEnv.KNOWLEDGE_BASE_PATH` in their own `devcontainer.json`. That's Lorekeeper's existing capability, not a Reeve mechanism.

### Reeve modules

#### New modules

- `crates/reeved/src/manifest.rs` — `Manifest` struct (`name`, `knowledge_base`, `repos: Vec<RepoEntry>`), `RepoEntry { name, url, tags, description }`, `load_manifest(meta_repo_path: &Path) -> Result<Manifest>`. Performs JSON parse + validation rules above. Tilde-expansion is not needed here (manifest is inside the meta-repo, paths are relative).

#### Modified modules

- `crates/reeved/src/household.rs` — `HouseholdProfile` removed. Replaced with a much smaller `Household { name: String, meta_repo_path: PathBuf }`. The `devcontainer` field is gone (found by convention at `<meta_repo_path>/.devcontainer/devcontainer.json`). The `[[repos]]` array is gone (read from the meta-repo's `repos.json` on demand). The `households` map at startup loads from the new sqlite `households` table, not from `~/.config/reeve/households/*.toml`.

- `crates/reeved/src/db.rs` — new `households` table:
  ```sql
  CREATE TABLE households (
    name TEXT PRIMARY KEY,
    meta_repo_path TEXT NOT NULL,
    added_at INTEGER NOT NULL
  );
  ```
  Migration: on first daemon start under M3.5, if `~/.config/reeve/households/*.toml` exists, log a warn pointing the user at `reeve household add` and skip auto-import (the old TOML format is incompatible enough that auto-conversion is more dangerous than helpful).

- `crates/reeved/src/clones.rs` — `clone_all_repos` is rewritten:
  1. Load manifest from `<meta_repo_path>/repos.json`.
  2. Clone the meta-repo itself: `git clone --reference <meta_repo_path>/.git --dissociate <meta_repo_path> <card_workspace>/`. Uses the same `--reference --dissociate` mechanism as siblings.
  3. For each `repo` in `manifest.repos`: `git clone --reference <meta_repo_path>/<repo.name>/.git --dissociate <meta_repo_path>/<repo.name> <card_workspace>/<repo.name>/`. If `<meta_repo_path>/<repo.name>/` is missing on the host, return an error with a clear message ("sibling `<name>` declared in repos.json but not cloned at `<path>` — run your workspace's bootstrap").

- `crates/reeved/src/effects.rs::ensure_running` — `clone_all_repos` invocation point unchanged. The `devcontainer` path passed to `devcontainer up` becomes `<card_workspace>/.devcontainer/devcontainer.json` (which was just cloned in as part of the meta-repo).

- `crates/reeved/src/handlers.rs` — new household-management handlers:
  - `handle_household_add { name, path | url } -> AddResult` — `path` registers an existing local clone; `url` clones to a default location (configurable via flag, default `~/Source/<name>/`) and registers. Validates the manifest before inserting into sqlite.
  - `handle_household_new { name, path?, template? } -> NewResult` — scaffolds from the template repo. Default template: `https://github.com/Mindful-Stack/witan-household`. Mechanism: `git clone --depth=1 <template>`, then `rm -rf .git && git init && git add -A && git commit -m "initial workspace"`. Default `path`: `~/Source/<name>/`. Sets `repos.json`'s top-level `name` field to the user-provided name. Registers as a household.
  - `handle_household_ls -> Vec<HouseholdSummary>` — `name`, `meta_repo_path`, manifest summary (workspace name, KB name, repo count).
  - `handle_household_rm { name } -> ()` — removes the row from sqlite. Does NOT delete the meta-repo on disk (user-owned).
  - `handle_household_show { name } -> HouseholdDetail` — full manifest contents + sibling presence check (which siblings are populated on host, which are missing) + the list of cards currently referencing this household (cheap join on the existing `cards` table by `household_name`).

- `crates/reeve-protocol/src/lib.rs` — new RPC types:
  - `HouseholdAddParams { name: String, source: HouseholdSource }`, `HouseholdSource { Path(PathBuf), Url(String) }`
  - `HouseholdNewParams { name: String, path: Option<PathBuf>, template: Option<String> }`
  - `HouseholdSummary { name: String, meta_repo_path: PathBuf, workspace_name: String, knowledge_base: Option<String>, repo_count: usize }`
  - `HouseholdDetail { summary: HouseholdSummary, repos: Vec<RepoDetail>, cards: Vec<CardReference> }`, `RepoDetail { name, url, tags, description, populated_on_host: bool }`, `CardReference { id, brief, column }`

- `crates/reeve/src/cli.rs` — new `household` subcommand tree:
  ```
  reeve household add  <name> <source>          # <source> is a filesystem path OR a git URL
                              [--path <dir>]    # only meaningful when <source> is a URL: where to clone (default ~/Source/<name>/)
  reeve household new  <name> [--path <dir>] [--template <url>]
  reeve household ls
  reeve household rm   <name>
  reeve household show <name>
  ```
  `add` distinguishes by inspecting `<source>`: starts with `git@` / `https://` / `file://` → URL (clone-then-register); otherwise → path (validate-and-register). `--path` is rejected with an error when `<source>` is a path.
  `reeve mv <id> implementing` keeps its existing `--household <name>` reference. The household-name lookup goes through the sqlite table now instead of the TOML directory scan.

### `Mindful-Stack/witan-household` template

Created as part of M3.5 work, in the `Mindful-Stack` org, configured as a GitHub template repo. Contents:

```
witan-household/
├── repos.json                       ← starter: { "name": "<your-workspace>", "knowledge_base": "lore", "repos": [] }
├── .devcontainer/
│   └── devcontainer.json            ← starter: ubuntu base, claude install, lorekeeper plugin install (commented)
├── CLAUDE.md                        ← workspace pattern explainer + space for project-specific guidance
├── .gitignore                       ← `/*` allowlist excluding repos.json, .devcontainer/, .gitignore, CLAUDE.md, README.md, lore/
├── README.md                        ← how to use this workspace (with or without Reeve, with or without Lorekeeper)
└── lore/
    └── knowledge/
        ├── general/_starter.md
        ├── domain/_starter.md
        ├── frameworks/_starter.md
        ├── languages/_starter.md
        └── learnings/_starter.md
```

The `.devcontainer/devcontainer.json` starter:

```jsonc
{
  "name": "witan-household",
  "image": "mcr.microsoft.com/devcontainers/base:ubuntu",
  "features": {
    "ghcr.io/devcontainers/features/node:1": {}
  },
  "postCreateCommand": [
    "curl -fsSL https://claude.ai/install.sh | bash"
    // To install Lorekeeper inside the card, add:
    // "claude plugin marketplace add Mindful-Stack/witan && claude plugin install lorekeeper@witan"
  ]
}
```

Both Reeve (`reeve household new --template <url>`, defaulting to the GitHub URL) and Lorekeeper standalone users (GitHub "Use this template" button or `git clone`) consume this. The template is tool-agnostic; it doesn't depend on Reeve or require Lorekeeper.

### Lorekeeper parallel PR

Tracked separately, but should land in the same Reeve milestone window:

- `/home/daniel/Source/lorekeeper/hooks/load-standards-reminder.sh` — extend sibling-fallback list:
  ```bash
  for candidate in "./docs/lore" "./lore" "./docs/shared-knowledge" "./shared-knowledge" "./knowledge"; do
    ...
  done
  ```
  Order: `lore`-prefixed candidates first (canonical going forward), legacy `shared-knowledge` and `knowledge` retained for back-compat.
- `/home/daniel/Source/lorekeeper/README.md` — lead with the meta-repo / sibling-fallback pattern as the recommended setup; demote `KNOWLEDGE_BASE_PATH` to "use this if your KB isn't at the conventional location."
- `/home/daniel/Source/lorekeeper/templates/knowledge-base/` — re-shape to match the `witan-household` structure (or replace with a pointer to the `witan-household` template repo). To be decided in Lorekeeper-side writing-plans.

### Error semantics

- **Manifest parse error**: hard error at any `reeve household add/new/show/ls` or at `→ Implementing`. Returns the JSON parse error position. `add` rolls back the registration.
- **Manifest validation failure** (duplicate `repos[].name`, dangling `knowledge_base` reference, reserved name collision): hard error with the rule that failed and the offending field.
- **Missing sibling clone** at `→ Implementing` (host meta-repo declares `repos[].name = "backend"` but `<meta_repo>/backend/` doesn't exist or has no `.git`): hard error pointing the user at their bootstrap step. Card stays in `Spawning`/transitions back; container is not started.
- **Missing meta-repo path** (household registered, but path no longer exists or is no longer a git repo): error at lookup with a "household `<name>` points at `<path>` which is gone — run `reeve household rm` and re-add" message.
- **Template clone failure** in `household new`: error from `git clone`; the partial directory is removed; the household is NOT registered.
- **Target path already exists** in `household new` (non-empty directory at `<path>`): hard error refusing to overwrite; the household is NOT registered.
- **Sibling path collision** with meta-repo internals (`.git`, `.devcontainer`): caught at manifest validation, refused at `add`.

## Testing

### Unit (manifest.rs)

- `load_manifest_parses_minimal` — `{ "repos": [] }` succeeds.
- `load_manifest_requires_knowledge_base_to_exist_in_repos` — `knowledge_base: "lore"` without a matching `repos` entry → validation error.
- `load_manifest_rejects_duplicate_repo_names`.
- `load_manifest_rejects_reserved_repo_names` (`.git`, `.devcontainer`).
- `load_manifest_defaults_workspace_name_to_dir_basename` when top-level `name` is absent.
- `load_manifest_treats_repos_field_as_array` (object → type error).

### Unit (household / db)

- `add_household_inserts_row` — sqlite assertion.
- `add_household_rejects_duplicate_name`.
- `add_household_validates_manifest_before_insert` — meta-repo with invalid manifest → registration fails, no row.
- `rm_household_removes_row_does_not_touch_disk`.
- `ls_households_returns_summary_with_manifest_overview`.
- `show_household_reports_missing_siblings` — declared but not populated on host.
- `show_household_lists_referencing_cards` — insert two cards referencing the household, one referencing a different household; assert only the first two appear in `cards`.

### Unit (handlers.rs)

- `household_new_uses_default_template_when_unspecified`.
- `household_new_writes_workspace_name_to_repos_json`.
- `household_new_creates_fresh_git_history` — no remote, no template refs.
- `mv_to_implementing_clones_meta_repo_and_each_sibling` (NoopGit recording per-call args).
- `mv_to_implementing_errors_on_missing_sibling` — meta-repo lacks declared sibling subdir → no clone calls, no container start, error returned.

### Integration (clones.rs)

- `real_clone_meta_repo_and_siblings_into_workspace` (tempfile): set up a host meta-repo with one sibling, run `clone_all_repos`, assert both `<workspace>/.git` and `<workspace>/<sibling>/.git` exist and dissociate succeeded.

### E2E (Docker-gated, in tests/e2e.rs)

- `m3_5_household_new_then_implementing_lands_workspace`:
  1. `reeve household new test-household --path <tmp>` — scaffolds from local copy of the template (test injects `--template <file://...>` to avoid network).
  2. Manually populate one sibling under `<tmp>/test-household/<sibling>/` (init + commit one file).
  3. `reeve new --household test-household --brief "smoke"` → `<id>`.
  4. `reeve mv <id> implementing`.
  5. Assert `~/reeve/cards/<id>/` contains the meta-repo clone (`.devcontainer/`, `repos.json`, `lore/`) AND the sibling clone.
  6. Assert container is running with workspace mounted.

- `m3_5_missing_sibling_blocks_implementing`:
  1. Scaffold a household, declare a sibling in `repos.json`, do NOT populate it on host.
  2. `reeve mv <id> implementing` → error with the expected message; no container started.

### KB-presence smoke (separate from Lorekeeper PR)

- `m3_5_lore_subdir_lands_in_workspace` — household with `knowledge_base: "lore"`, populated `lore/` sibling on host, post-`→ Implementing` the card workspace has `<workspace>/lore/` populated.

Lorekeeper-side sibling-fallback behaviour is tested in the Lorekeeper repo, not here.

## Open items deferred to writing-plans

- Default clone location flag name: `--path` vs `--at` vs `--dir`. Pick during plan.
- Whether `reeve household add <name> <url>` should *also* populate siblings (`gh repo clone` each one) or stop after cloning the meta-repo and let the user run their own bootstrap. Lean: stop after meta-repo; the user's `make setup` (or equivalent) handles siblings. Confirm.
- Whether the sqlite `households` row should include a `template_used` column for forensics. Lean: no, manifest is sufficient.
- Whether to bundle the witan-household template's *content* inside the Reeve binary as a fallback for offline use. Lean: no, GitHub clone is reliable enough; offline users can pre-clone the template and pass `--template <local-path>`.
- The exact starter content for `lore/knowledge/*/_starter.md` files. Defer to template-repo PR.

## Out of scope (do not implement in M3.5)

- Namespaced refs, `refs/reeve/<id>/*` (M4).
- `FetchWorker` and `reeve push` (M4).
- Auto-population of siblings on `reeve household add <url>` (deferred for review; see open items).
- Auto-import of `~/.config/reeve/households/*.toml` from M3 (warn-and-skip; user re-adds with `reeve household add`).
- Lorekeeper template-repo content restructure (`/home/daniel/Source/lorekeeper/templates/knowledge-base/` reshape). Lives in a parallel Lorekeeper PR.
- Marketplace-distributed household templates (a "Witan household marketplace") — Witan stays plugin-only.
- Cross-household sharing (multiple cards across multiple households on the same daemon — already supported in M3; nothing new here).
- `reeve household update` (re-running the template against an existing household to pick up template changes) — possible follow-up if churn justifies it.
- Service categories, heavy-slot scheduler, fork operations, TUI mutations — all parent-design items unchanged.

## Verification

A merged M3.5 must:

1. Pass all M1+M2+M3 tests untouched (except `HouseholdProfile`-specific tests, which are removed alongside the schema).
2. Pass the new unit, integration, and Docker-gated e2e tests.
3. `cargo test --workspace` clean (with `REEVE_TESTS_STRICT=1`); `cargo fmt --check` clean; `cargo clippy` clean.
4. Manually:
   1. `reeve household new reeve-self --path ~/Source/reeve-household` — scaffolds from `Mindful-Stack/witan-household`.
   2. Hand-edit `repos.json` to add a `repos[]` entry for the local `reeve` repo (`{ "name": "reeve", "url": "git@github.com:Daniel-Thyselius/reeve.git" }`), and a `lore` entry pointing at any local KB.
   3. Populate `~/Source/reeve-household/reeve/` and `~/Source/reeve-household/lore/` (manual clone).
   4. `reeve household show reeve-self` — confirms manifest parses, both siblings show `populated_on_host: true`.
   5. `reeve new --household reeve-self --brief "m3.5 smoke"` → `<id>`.
   6. `reeve mv <id> implementing` → `~/reeve/cards/<id>/` contains the meta-repo clone, `reeve/` sibling clone, `lore/` sibling clone; container is up with workspace mounted at `/workspaces/<id>/`.
   7. `docker exec <container> ls /workspaces/<id>/` → shows `.devcontainer/`, `repos.json`, `lore/`, `reeve/`, `CLAUDE.md`.
   8. (If Lorekeeper installed in the container via uncommented `postCreateCommands`) `docker exec <container> claude /lore:help` reports "Knowledge base found" at the workspace's `lore/` directory.
   9. `reeve mv <id> done` → container teardown; card workspace remains on disk (consistent with M3 behaviour).
