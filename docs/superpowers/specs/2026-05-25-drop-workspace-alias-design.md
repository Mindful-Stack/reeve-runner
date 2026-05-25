# Drop the `workspace` field alias across the Witan tools

**Status:** Design (2026-05-25)
**Scope:** Cross-tool change spanning three repos. Spec lives in Reeve since the rename originated here; implementation will be three coordinated commits, one per repo.
**Predecessor:** Stage 6 of the 2026-05-18 domain-driven refactor introduced `Manifest.meta_repo` with `#[serde(rename = "meta_repo", alias = "workspace")]` for back-compat. The alias step is what this spec retires.

## Context

In Stage 6 of the domain-driven refactor we renamed `Manifest.workspace` to `Manifest.meta_repo`. The motivation was overload — "workspace" already meant three things in the Witan ecosystem (the manifest's `workspace` field, Reeve's per-card on-disk directory `~/reeve/cards/<id>/`, and the devcontainer CLI's `--workspace-folder` flag). Renaming the manifest field freed up "workspace" so Reeve's per-card aggregate could be named `Workspace` unambiguously.

The migration plan was: `#[serde(rename = "meta_repo", alias = "workspace")]` for one release with a `tracing::warn!` deprecation log, then drop the alias. The first half landed in Reeve; the second half never did because Lorekeeper and the witan-household template were never updated to accept the new name. Today we have a vocabulary fork:

| Path | Field name in resulting `household.json` |
|---|---|
| `reeve household new` (Reeve scaffolds via template, then rewrites the manifest via serde) | `meta_repo` (canonical) |
| `git clone Mindful-Stack/witan-household` then `cd` into it | `workspace` (template ships the old name) |
| `/lore:init` on an existing project | `workspace` (Lorekeeper's init clones the template and runs `rename.sh` value-only) |

And Lorekeeper's tooling currently only accepts `workspace`:

- `lorekeeper/scripts/cultivate-detect.js` reads `manifest.workspace` at two sites (lines 46 and 115).
- `witan-household/lore/_tools/doctor.js` requires the `workspace` field; emits `missing required "workspace" field` otherwise.

A household created via `reeve household new` therefore *breaks Lorekeeper's doctor*. This was discovered during the lore-scaffold work, which is why Reeve's own `household.json` was reverted to use `workspace` (so `make doctor` would pass).

## Goal

Eliminate the `Manifest.workspace` field name from all three Witan tools. After this change, `meta_repo` is the canonical and only-accepted name. The serde alias and deprecation warning come out of Reeve; Lorekeeper and the witan-household template stop reading the old field name.

## Non-goals

- Not renaming `knowledge_base` or any other manifest field.
- Not introducing a general `manifest_version` field. If we ever rename another field, we'll do the same alias-and-drop dance — three repos is small enough that ad-hoc coordination beats a versioning framework.
- Not building a migration helper script. Only one household-in-the-wild needs migrating (Reeve's own), and the migration is a one-line `sed`.
- Not changing any code outside the manifest field name (no architectural refactor, no policy change).

## Per-repo changes

### `reeve` (`/home/daniel/Source/reeve`)

| File | Change |
|---|---|
| `crates/reeve-household/src/manifest.rs` | Drop the `#[serde(rename = "meta_repo", alias = "workspace")]` attribute from the `meta_repo` field. The Rust field name already matches the JSON name, so serde defaults to `meta_repo` without the attribute. |
| Same file: `load_manifest` | Delete the `tracing::warn!` block that detects the deprecated `workspace` field name in raw JSON. |
| Same file: `tests` | Delete the `old_workspace_field_name_still_parses_as_alias` test. |
| `household.json` | Flip `"workspace": "reeve"` back to `"meta_repo": "reeve"`. |
| `lore/knowledge/domain/household.md` | Update the ubiquitous-language entry that describes the rename: remove the "alias for one release" language; replace with "renamed in 2026-05; the old `workspace` field name is no longer accepted." |
| `lore/knowledge/general/architecture.md` | Same update to the "Manifest schema migration" section. |
| `CLAUDE.md` (the long form the user keeps) | Same update to the "Manifest schema migration" section. |

After: rebuild `lore/knowledge/_index.json` (`make build-index`); `make doctor` passes; `cargo test --workspace --lib` passes; `git grep -E '"workspace"|manifest\.workspace|m\.workspace' crates/ scripts/ lore/_tools/` returns only doc/history references, no live code.

### `lorekeeper` (`/home/daniel/Source/lorekeeper`)

| File | Change |
|---|---|
| `scripts/cultivate-detect.js` line 46 | `r.name !== manifest.workspace` → `r.name !== manifest.meta_repo` |
| Same file, line 115 | Same edit (the filter is the same shape in a second function). |
| `scripts/init-detect.js` | Re-grep for any `manifest.workspace` / `"workspace"` reads. My current grep returned nothing in this file, but it's worth verifying after editing `cultivate-detect.js` in case the touch surfaces something. |
| `__tests__/*` | Update any test-fixture manifests that hard-code `"workspace": "..."` to use `"meta_repo": "..."`. Identify by `git grep -E '"workspace":' __tests__/`. |

After: `node --test __tests__/*.test.js` passes; `git grep -E '"workspace"|manifest\.workspace|m\.workspace' .` returns nothing in `scripts/`, `__tests__/`, `commands/`, `skills/`, `hooks/`, `agents/`.

### `witan-household` (`/home/daniel/Source/witan-household`)

| File | Change |
|---|---|
| `household.json` (the template's own manifest) | `"workspace": "my-workspace"` → `"meta_repo": "my-workspace"`. |
| `lore/_tools/doctor.js` | The check that emits `missing required "workspace" field` becomes `missing required "meta_repo" field`. Any other read of `manifest.workspace` rewrites to `manifest.meta_repo`. |
| `lore/_tools/build-index.js` | Re-grep + rewrite if any `manifest.workspace` reads exist. |
| `lore/_tools/validate-frontmatter.js` | Re-grep + rewrite if it touches the field. |
| `lore/_tools/check-orphans.js` | Re-grep + rewrite if it touches the field. |
| `lore/_tools/__tests__/*` | Update any test-fixture manifests from `workspace` to `meta_repo`. |
| `scripts/rename.sh` | If the script substitutes the field NAME, update. If it only substitutes the VALUE (the workspace name), no change. |
| `README.md` (and any other docs with sample `household.json` snippets) | Update the snippets. |

After: `node --test lore/_tools/__tests__/*.test.js` passes; the template can be cloned and its own `make doctor` should pass clean.

## Sequencing

Recommended order, all in one session:

1. **Lorekeeper** first — its tooling stops reading `manifest.workspace` and starts reading `manifest.meta_repo`. Lorekeeper's tests pass with the new field name; the old name would no longer be filtered correctly.
2. **witan-household** second — the template's `household.json` and `lore/_tools/*` align with Lorekeeper. New households scaffolded from the template now write `meta_repo`.
3. **Reeve** last — drop the serde alias and deprecation warning; flip `household.json` to `meta_repo`; update lore + CLAUDE.md. By this point, both upstream consumers expect `meta_repo`, so Reeve can require it too without breaking anyone.

Rationale: doing Lorekeeper + witan-household before Reeve means Reeve's daemon never has to talk to a tool that still requires the deprecated name. If something goes wrong mid-sequence, Reeve still accepts `workspace` via the alias until step 3 lands.

The reverse order would also work (Reeve drops alias first, consumers updated later), but it briefly leaves Reeve refusing households that Lorekeeper still tells users to create with `workspace`. Confusing.

If you'd rather do all three commits truly atomically (no intermediate state), that's also fine — there's no published version of any tool to break. The recommended sequence is purely about graceful failure if you pause mid-rollout.

## Verification

After all three commits land:

| Check | Where |
|---|---|
| `cargo test --workspace -p reeve-household` | Manifest tests pass (the deleted alias test is the only one removed). |
| `cargo test --workspace --lib && cargo test -p reeved` | Nothing else regresses in Reeve. |
| `make doctor` in Reeve's repo | Passes — no longer warns about the missing workspace field; accepts the flipped `meta_repo` manifest. |
| `node --test scripts/__tests__/*.test.js` in lorekeeper | Passes. |
| `node --test lore/_tools/__tests__/*.test.js` in witan-household | Passes against the meta_repo-shaped fixtures. |
| `git grep -E '"workspace"\|manifest\.workspace\|m\.workspace' crates/ scripts/ lore/_tools/` in Reeve | Returns only doc/history references (the spec history, the `domain/household.md` mention of the historical rename). No live code. |
| Same grep in lorekeeper and witan-household | Returns nothing in live code (`scripts/`, `__tests__/`, `lore/_tools/`, etc.). |

The grep is the "no drift" check. Once it's clean across all three repos and the doctor/tests pass, the alias is gone.

## Risks

- **Forgot a file.** Mitigation: the post-merge `git grep` across all three repos. If anything lights up, fix it.
- **A household in the wild we forgot.** Mitigation: only Reeve's own household exists (confirmed during brainstorm). If we later discover one, the user runs `sed -i 's/"workspace":/"meta_repo":/' household.json` on it.
- **Sequencing pause leaves a broken household.** Mitigation: the recommended order (Lorekeeper → witan-household → Reeve) is graceful — Reeve's alias is the last thing dropped, so any intermediate-state household keeps working.
- **A consumer we haven't catalogued.** All three repos (`reeve`, `lorekeeper`, `witan-household`) are owned by the user and not published anywhere with external readers. No other consumer is plausible.

## Out of scope (deferred)

- A `manifest_version` field for future rename machinery — YAGNI until a third rename surfaces.
- A migration helper script — only one household needs migrating; one sed line is faster than a script.
- Updating Reeve to emit `meta_repo` deprecation warnings if a future field gets renamed — that's a recurring-pattern question, not this rename's problem.
- Lorekeeper's `/lore:init` flow updates (e.g., changing how `init-detect.js` classifies projects) — out of scope. We only touch the field-name reads.

## Decision log

| Decision | Choice | Why |
|---|---|---|
| Complete the rename, not revert it | Yes | The reasoning that motivated the rename (the three-way overload of "workspace") still stands; reverting solves nothing. |
| End state | `meta_repo`-only; alias dropped | User confirmed "drop the alias ASAP." Solo-user/local tools mean no external compatibility burden. |
| Sequencing | Lorekeeper → witan-household → Reeve | Graceful failure if paused mid-sequence; Reeve's alias is the safety net until last. |
| Migration helper | None | Only one household in the wild; one `sed` is enough. |
| Manifest versioning | Not introduced | YAGNI for two renames over two years. |
