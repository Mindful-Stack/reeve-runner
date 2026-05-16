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

- `household.json` — manifest declaring the workspace name, the KB pointer, and the sibling repos
- `.devcontainer/devcontainer.json` — devcontainer config at the conventional location
- `CLAUDE.md` — workspace-level guidance
- `.gitignore` — excludes sibling repo dirs
- Sibling repos cloned alongside (gitignored), one of which carries the KB at the conventional name `lore/`

Reeve's `households` table stores `(name, meta_repo_path)`. Cards clone the meta-repo *and* the siblings it declares into `~/reeve/cards/<id>/`, mounting that as the container workspace. The devcontainer.json found at the conventional location drives `devcontainer up`.

A separate template repo at `Mindful-Stack/witan-household` provides the canonical starter. Both Reeve (`reeve household new`) and Lorekeeper (standalone use via the template repo's "Use this template" button or `git clone`) consume it. The template is tool-agnostic — Lorekeeper-only users get a working setup without Reeve.

The Lorekeeper plugin gains `./lore` and `./docs/lore` in its sibling-fallback resolution list (parallel Lorekeeper PR, not gated on Reeve M3.5 shipping). With the meta-repo containing a `lore/` sibling at the workspace root, Lorekeeper's existing fallback resolves the KB automatically inside the container — Reeve injects no env vars, writes no `.lorekeeper/config.json`, and creates no symlinks.

## Architecture

### Manifest schema

`household.json` at the meta-repo root. The schema is unified: every repo participating in the workspace — including the workspace meta-repo itself — is a `repos[]` entry. Two singular top-level pointers select which entry plays the workspace role and which (if any) holds the knowledge base.

```json
{
  "workspace": "myarchy-workspace",
  "knowledge_base": "lore",
  "repos": [
    { "name": "myarchy-workspace", "url": "git@github.com:Mindful-Stack/myarchy-workspace.git", "description": "The workspace meta-repo" },
    { "name": "backend",  "url": "git@github.com:Mindful-Stack/myarchy-backend.git",  "tags": ["backend", "core"] },
    { "name": "frontend", "url": "git@github.com:Mindful-Stack/myarchy-frontend.git", "tags": ["frontend"] },
    { "name": "lore",     "url": "git@github.com:Mindful-Stack/myarchy-lore.git",     "tags": ["docs"] }
  ]
}
```

Fields:

- `workspace` (string, **required**) — references one of `repos[].name`. That entry is the meta-repo itself; its on-disk location is the meta-repo root.
- `knowledge_base` (string, optional) — references one of `repos[].name`. The entry that holds the KB. Cardinality-one by type. May equal `workspace` for single-repo setups where the KB is tracked inline in the meta-repo. If absent, no KB is wired; Lorekeeper inside the container falls through to its own resolution paths or reports no KB configured.
- `repos` (array, non-empty) — every repo in this workspace, the meta-repo included. Each entry:
  - `name` (string, required) — manifest identifier and, for siblings, the directory name on disk relative to the meta-repo root. For the workspace entry, the on-disk location is the meta-repo root itself (not a subdirectory of that name).
  - `url` (string, optional) — git URL. Surfaced in `reeve household show`; consumed by user-owned bootstrap scripts (a `make setup` analogue) and by `reeve household new`. For sibling entries, NOT resolved by Reeve at card-spawn time (sibling sources come from `<meta-repo>/<name>/.git` on the host). For the workspace entry, documentation only.
  - `tags` (array of strings, optional) — categorization (`backend`, `frontend`, `core`, `docs`, etc.). KB-marker and workspace-marker are NOT tags — they live in the top-level pointers.
  - `description` (string, optional) — human-readable, surfaced in `reeve household show`.

Validation (at manifest load, fail-fast):

1. JSON parses.
2. `repos` is a non-empty array.
3. Every `repos[].name` is non-empty and unique.
4. `workspace` matches exactly one `repos[].name`.
5. If `knowledge_base` is set, it matches exactly one `repos[].name` (which may be the same entry as `workspace`).
6. No `repos[].name` matches a reserved name (`.git`, `.devcontainer`). Other collisions with existing files (e.g., a repo entry named `CLAUDE.md`) fail at clone-time with the underlying git error; they're not caught at manifest-load.

#### Sibling vs inline (determined at clone time, not in the manifest)

For each `repos[]` entry other than the workspace, Reeve dispatches on disk state at `→ Implementing`:

| `<meta-repo>/<name>/` state | Entry's role | Card-clone behaviour |
|---|---|---|
| Exists with `.git/` | Separate sibling repo | `--reference --dissociate` clone into `<card-workspace>/<name>/` |
| Exists without `.git/` | Inline directory tracked as part of the meta-repo | No separate clone — it travels with the meta-repo |
| Doesn't exist | User forgot to bootstrap | Hard error: "sibling `<name>` declared in household.json but not cloned" |

This makes the single-repo inline-KB workspace (where `lore/` is a tracked directory in the meta-repo, not a separate git history) a first-class case. The manifest is the same shape; only disk state differs.

### Host-side topology

Multi-repo case (every non-workspace entry is a separate sibling git repo):

```
~/Source/myarchy-workspace/         ← meta-repo (user-cloned or `reeve household new`)
├── .git/
├── household.json
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

Single-repo inline-KB case (the witan-household starter shape):

```
~/Source/my-workspace/              ← meta-repo (workspace + inline KB)
├── .git/
├── household.json                       ← workspace: "my-workspace", knowledge_base: "lore", repos: [{my-workspace}, {lore}]
├── .devcontainer/devcontainer.json
├── CLAUDE.md
├── .gitignore                       ← does NOT exclude lore/ (it's tracked)
└── lore/                            ← inline KB directory, tracked by the workspace meta-repo (no .git/)
    └── knowledge/
        └── ...
```

Both shapes use the same manifest schema; Reeve dispatches at clone time on whether a `repos[]` entry's directory has its own `.git/`.

Sibling clones inside the meta-repo are the **canonical source** for Reeve's `git clone --reference --dissociate` operations. Reeve never resolves manifest `url` fields at card-spawn time — it expects the host meta-repo to already contain each declared non-workspace sibling either as a populated git subdirectory (with `.git/`) or as an inline tracked directory (no `.git/`, lives in the meta-repo's history). A `repos[]` entry whose directory is entirely missing from disk is a hard error at `→ Implementing` with a message telling the user to populate it (typically by running their meta-repo's bootstrap, e.g. `make setup`).

### Card-side topology

```
~/reeve/cards/<card-id>/             ← card workspace (mount root for devcontainer)
├── .git/                            ← per-card clone of meta-repo, --reference --dissociate from host meta-repo
├── household.json                       ← (copy travels with meta-repo clone)
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

The meta-repo participates in fetch + push as **repo zero** alongside siblings. Agent commits to `household.json`, `.devcontainer/devcontainer.json`, `CLAUDE.md` flow back to the user's host meta-repo via `refs/reeve/<card-id>/*` in M4.

### Lorekeeper interaction (no Reeve mechanism)

The Lorekeeper plugin, if the user's `devcontainer.json` installs it (via `postCreateCommands`, base image, or devcontainer features), discovers the KB via its existing resolution order. M3.5 lands together with a parallel Lorekeeper PR that adds `./lore` and `./docs/lore` to Lorekeeper's sibling-fallback list. With the container's CWD at `/workspaces/<card-id>/` and a `lore/` sibling present, Lorekeeper finds the KB automatically.

Reeve injects no `KNOWLEDGE_BASE_PATH`, writes no `.lorekeeper/config.json`, and creates no symlinks. The integration is purely structural: provide the meta-repo + `lore/` sibling, Lorekeeper handles the rest.

Users who want to override (e.g., a KB named other than `lore`) set `containerEnv.KNOWLEDGE_BASE_PATH` in their own `devcontainer.json`. That's Lorekeeper's existing capability, not a Reeve mechanism.

### Reeve modules

#### New modules

- `crates/reeved/src/manifest.rs` — `Manifest` struct (`workspace`, `knowledge_base`, `repos: Vec<RepoEntry>`), `RepoEntry { name, url, tags, description }`, `load_manifest(meta_repo_path: &Path) -> Result<Manifest>`. Helper `Manifest::workspace_entry(&self) -> &RepoEntry` returns the entry pointed to by `workspace`. Helper `Manifest::knowledge_base_entry(&self) -> Option<&RepoEntry>` returns the KB entry if set. Performs JSON parse + validation rules above. Tilde-expansion is not needed here (manifest is inside the meta-repo, paths are relative).

#### Modified modules

- `crates/reeved/src/household.rs` — `HouseholdProfile` removed. Replaced with a much smaller `Household { name: String, meta_repo_path: PathBuf }`. The `devcontainer` field is gone (found by convention at `<meta_repo_path>/.devcontainer/devcontainer.json`). The `[[repos]]` array is gone (read from the meta-repo's `household.json` on demand). The `households` map at startup loads from the new sqlite `households` table, not from `~/.config/reeve/households/*.toml`.

- `crates/reeved/src/db.rs` — new `households` table:
  ```sql
  CREATE TABLE households (
    name TEXT PRIMARY KEY,
    meta_repo_path TEXT NOT NULL,
    added_at INTEGER NOT NULL
  );
  ```
  Migration: on first daemon start under M3.5, if `~/.config/reeve/households/*.toml` exists, log a warn pointing the user at `reeve household add` and skip auto-import (the old TOML format is incompatible enough that auto-conversion is more dangerous than helpful).

- `crates/reeved/src/clones.rs` — `clone_all_for_card` walks the unified `repos[]`:
  1. Load manifest from `<meta_repo_path>/household.json`.
  2. Clone the **workspace entry** (the one named by `manifest.workspace`): `git clone --reference <meta_repo_path>/.git --dissociate <meta_repo_path> <card_workspace>/`. The card workspace dir is **created by `git clone`**, not pre-created by the daemon — `git clone` refuses to write into a non-empty directory, so the caller must NOT `create_dir_all(<card_workspace>)` before this step. The parent (`$REEVE_CARDS_DIR`) IS pre-created.
  3. For each remaining `repo` in `manifest.repos` (i.e. every entry except the one identified as the workspace), dispatch on host disk state:
     - `<meta_repo_path>/<repo.name>/.git/` exists → separate sibling repo. `git clone --reference <meta_repo_path>/<repo.name>/.git --dissociate <meta_repo_path>/<repo.name> <card_workspace>/<repo.name>/`.
     - `<meta_repo_path>/<repo.name>/` exists but has no `.git/` → inline directory (tracked as part of the workspace meta-repo). No separate clone — it travelled with the workspace clone in step 2. Reeve does nothing for this entry beyond noting it.
     - `<meta_repo_path>/<repo.name>/` does not exist → hard error ("sibling `<name>` declared in household.json but not cloned at `<path>` — run your workspace's bootstrap").

- `crates/reeved/src/effects.rs::ensure_running` — clone invocation switches to `clone_all_for_card(&*git, &household, &manifest, &workspace)`. **Remove the pre-existing `std::fs::create_dir_all(&workspace)` call** that M3 added before clones — step 2 above now handles workspace creation. The `$REEVE_CARDS_DIR` parent dir is still pre-created (M3 also does this implicitly when binding the cards socket dir). The `devcontainer` path passed to `devcontainer up` becomes `<card_workspace>/.devcontainer/devcontainer.json` (which was just cloned in as part of the workspace meta-repo).

- `crates/reeved/src/handlers.rs` — new household-management handlers:
  - `handle_household_add { name, path | url } -> AddResult` — `path` registers an existing local clone; `url` clones to a default location (configurable via flag, default `~/Source/<name>/`) and registers. Validates the manifest before inserting into sqlite.
  - `handle_household_new { name, path?, template? } -> NewResult` — scaffolds from the template repo. Default template: `https://github.com/Mindful-Stack/witan-household`. Mechanism: `git clone --depth=1 <template>`, then `rm -rf .git && git init && git add -A && git commit -m "initial workspace"`. Default `path`: `~/Source/<name>/`. Substitutes the user-provided name into the manifest in two places: the top-level `workspace` field AND the matching `repos[].name` entry (the workspace entry). Registers as a household.
  - `handle_household_ls -> Vec<HouseholdSummary>` — `name`, `meta_repo_path`, manifest summary (workspace name, KB name, repo count).
  - `handle_household_rm { name } -> ()` — removes the row from sqlite. Does NOT delete the meta-repo on disk (user-owned).
  - `handle_household_show { name } -> HouseholdDetail` — full manifest contents + sibling presence check (which siblings are populated on host, which are missing) + the list of cards currently referencing this household (cheap join on the existing `cards` table by `household_name`).

- `crates/reeve-protocol/src/lib.rs` — new RPC types:
  - `HouseholdAddParams { name: String, source: HouseholdSource }`, `HouseholdSource { Path(PathBuf), Url(String) }`
  - `HouseholdNewParams { name: String, path: Option<PathBuf>, template: Option<String> }`
  - `HouseholdSummary { name: String, meta_repo_path: PathBuf, workspace_name: String, knowledge_base: Option<String>, repo_count: usize }`
  - `HouseholdDetail { summary: HouseholdSummary, workspace_url: Option<String>, repos: Vec<RepoDetail>, cards: Vec<CardReference> }`, `RepoDetail { name, url, tags, description, role: RepoRole, populated_on_host: bool }`, `RepoRole { Workspace, KnowledgeBase, Sibling, Inline }`, `CardReference { id, brief, column }`. `workspace_url` comes from the workspace `repos[]` entry's `url`, falling back to `git remote get-url origin` in the meta-repo if absent. `RepoRole` is derived: `Workspace` if the entry name matches `manifest.workspace`, `KnowledgeBase` if it matches `manifest.knowledge_base`, `Inline` if its on-disk directory exists without `.git/`, `Sibling` otherwise. The workspace and KB roles compose — a single-repo inline-KB workspace's sole entry has `Workspace + KnowledgeBase`; carried on the wire as a `Vec<RepoRole>` or a flags struct, exact encoding deferred to writing-plans.

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
├── household.json                       ← starter (see below)
├── .devcontainer/
│   └── devcontainer.json            ← starter: ubuntu base, claude install, lorekeeper plugin install (commented)
├── CLAUDE.md                        ← workspace pattern explainer + space for project-specific guidance
├── .gitignore                       ← `/*` allowlist excluding household.json, .devcontainer/, .gitignore, CLAUDE.md, README.md, lore/
├── README.md                        ← how to use this workspace (with or without Reeve, with or without Lorekeeper)
└── lore/
    └── knowledge/
        ├── general/_starter.md
        ├── domain/_starter.md
        ├── frameworks/_starter.md
        ├── languages/_starter.md
        └── learnings/_starter.md
```

The starter `household.json` (single-repo, inline-KB shape — `reeve household new` substitutes the workspace name in both places):

```json
{
  "workspace": "my-workspace",
  "knowledge_base": "lore",
  "repos": [
    { "name": "my-workspace", "url": "git@github.com:your-org/my-workspace.git", "description": "The workspace meta-repo (contains the inline lore/ KB)" },
    { "name": "lore", "description": "Inline knowledge-base directory tracked in this meta-repo. Move to a separate sibling repo if it outgrows the inline model." }
  ]
}
```

The `lore` entry has no `url` — by convention that signals "inline directory tracked in the workspace meta-repo." Reeve's clone-time disk-state dispatch confirms this (no `.git/` under `lore/`) and skips the separate sibling clone. Users who want a separate KB repo: remove the inline `lore/` directory, add a `url` to the `lore` entry, run their bootstrap to `git clone` the KB as a real sibling. The Lorekeeper sibling-fallback finds either shape automatically.

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

- `load_manifest_parses_minimal_single_repo_workspace` — `{ "workspace": "ws", "repos": [{ "name": "ws" }] }` succeeds; `Manifest::workspace_entry()` returns the right entry.
- `load_manifest_requires_workspace_to_match_a_repos_entry` — `workspace: "missing"` with no matching `repos` entry → validation error.
- `load_manifest_requires_knowledge_base_to_match_a_repos_entry` — `knowledge_base: "lore"` without a matching `repos` entry → validation error.
- `load_manifest_allows_knowledge_base_to_equal_workspace` — single-repo inline-KB shape parses.
- `load_manifest_rejects_empty_repos` — `repos: []` → validation error (workspace required, so `repos[]` can't be empty).
- `load_manifest_rejects_duplicate_repo_names`.
- `load_manifest_rejects_reserved_repo_names` (`.git`, `.devcontainer`).
- `load_manifest_rejects_missing_workspace_field` — manifest without `workspace` key → validation error.
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
  5. Assert `~/reeve/cards/<id>/` contains the meta-repo clone (`.devcontainer/`, `household.json`, `lore/`) AND the sibling clone.
  6. Assert container is running with workspace mounted.

- `m3_5_missing_sibling_blocks_implementing`:
  1. Scaffold a household, declare a sibling in `household.json`, do NOT populate it on host.
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
   1. `reeve household new reeve-self --path ~/Source/reeve-household` — scaffolds from `Mindful-Stack/witan-household`. The scaffolded `household.json` has `workspace: "reeve-self"` and a single `repos[]` entry of the same name, plus the inline `lore` entry.
   2. Hand-edit `household.json` to add a `repos[]` entry for the local `reeve` repo (`{ "name": "reeve", "url": "git@github.com:Daniel-Thyselius/reeve.git" }`). Leave `lore` inline OR add a `url` and clone it as a separate sibling — both shapes are valid.
   3. Populate `~/Source/reeve-household/reeve/` (manual clone). If you made `lore` a sibling, populate that too.
   4. `reeve household show reeve-self` — confirms manifest parses, every entry's `role` and `populated_on_host` are correct (workspace + inline-or-sibling for `lore`, sibling for `reeve`).
   5. `reeve new --household reeve-self --brief "m3.5 smoke"` → `<id>`.
   6. `reeve mv <id> implementing` → `~/reeve/cards/<id>/` contains the workspace clone (with inline `lore/` if applicable), the `reeve/` sibling clone (and the `lore/` sibling clone if applicable); container is up with workspace mounted at `/workspaces/<id>/`.
   7. `docker exec <container> ls /workspaces/<id>/` → shows `.devcontainer/`, `household.json`, `lore/`, `reeve/`, `CLAUDE.md`.
   8. (If Lorekeeper installed in the container via uncommented `postCreateCommand`) `docker exec <container> claude /lore:help` reports "Knowledge base found" at the workspace's `lore/` directory — works the same whether `lore/` is inline or a sibling.
   9. `reeve mv <id> done` → container teardown; card workspace remains on disk (consistent with M3 behaviour).
