# Drop the `workspace` Field Alias — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate the `Manifest.workspace` field name from all three Witan tools (lorekeeper, witan-household, reeve) in one coordinated rollout. After this lands, `meta_repo` is the canonical and only-accepted name; the serde alias and deprecation warning come out of Reeve.

**Architecture:** Three coordinated commits, one per repo, executed in sequence: Lorekeeper → witan-household → Reeve. Lorekeeper and witan-household update their tooling to read `manifest.meta_repo` (no alias). Reeve drops `#[serde(alias = "workspace")]`, the `tracing::warn!` deprecation block, the alias-back-compat test, and flips its own `household.json`. The sequencing means Reeve's alias (the safety net) is the last thing dropped — graceful failure if paused mid-rollout.

**Tech Stack:** Rust (Reeve's `reeve-household` crate uses serde + rusqlite), Node.js (Lorekeeper's `scripts/` + witan-household's `lore/_tools/`), Bash (witan-household's `scripts/`), JSON manifests.

**Spec:** `docs/superpowers/specs/2026-05-25-drop-workspace-alias-design.md` (in Reeve)

**Important context — three working directories:**

| Stage | Working directory |
|---|---|
| Stage A | `/home/daniel/Source/lorekeeper` |
| Stage B | `/home/daniel/Source/witan-household` |
| Stage C | `/home/daniel/Source/reeve` (specifically on branch `feat/drop-workspace-alias`) |
| Stage D | All three (verification grep) |

Each stage's task assumes you've `cd`'d to the right directory. **Do NOT mix repos in one task.**

**Migration note:** Reeve's own `household.json` is the only household-in-the-wild that uses the deprecated `workspace` field name (it was reverted from `meta_repo` during the lore-scaffold work). Step C.2 flips it back. No other migration needed.

---

## File-structure overview

### Lorekeeper (`/home/daniel/Source/lorekeeper`)

| File | What it does | Change in this plan |
|---|---|---|
| `scripts/cultivate-detect.js` | Detects candidate bounded contexts for `/lore:cultivate`. Reads `manifest.workspace` to exclude the meta-repo entry from the sibling list. | Rewrite 2 sites: `manifest.workspace` → `manifest.meta_repo`. |
| `scripts/__tests__/cultivate-detect.test.js` | Tests for the above. Uses a sample manifest with `workspace: 'test-workspace'`. | Update the fixture's field name to `meta_repo`. |

### witan-household (`/home/daniel/Source/witan-household`)

| File | What it does | Change in this plan |
|---|---|---|
| `household.json` | The template's own manifest. Cloned by `git clone witan-household` and by `reeve household new`. | Field name `workspace` → `meta_repo`. |
| `lore/_tools/doctor.js` | The `/lore:doctor` workspace+KB diagnostic. Validates the manifest. | Rewrite 5 sites that read `manifest.workspace` to read `manifest.meta_repo`. Update the error message that says `missing required "workspace" field`. |
| `scripts/rename.sh` | Substitutes the workspace name into household.json + CLAUDE.md (called by `make rename NAME=...` and by `reeve household new`). | Rewrite 3 sites that read/write `m.workspace` and the `.workspace` property. Update the `node -e ...` snippet that reads the old name. |
| `scripts/setup.sh` | Bootstrap: clones every sibling declared in `repos[]`. Filters out the meta-repo entry via `r.name !== m.workspace`. | Rewrite 1 site. |
| `scripts/pull-all.sh` | `git pull` across every sibling. Same filter shape. | Rewrite 1 site. |
| `scripts/status-all.sh` | `git status` across every sibling. Same filter shape. | Rewrite 1 site. |
| `README.md` | Documentation including a sample household.json snippet and a `make setup` Makefile snippet. | Update the JSON example (line ~91) and the jq filter in the Makefile snippet (line ~122). |
| `lore/_tools/__tests__/doctor.test.js` | Tests for doctor.js. Has 3 fixture manifests using `workspace:` field. | Update all 3 fixtures to `meta_repo:`. |

### Reeve (`/home/daniel/Source/reeve`)

| File | What it does | Change in this plan |
|---|---|---|
| `crates/reeve-household/src/manifest.rs` | The `Manifest` struct with `#[serde(rename = "meta_repo", alias = "workspace")]` and the `load_manifest` deprecation warning. | Drop the `#[serde(...)]` attribute (the field name already matches the JSON name without it). Delete the `tracing::warn!` block in `load_manifest`. Delete the `old_workspace_field_name_still_parses_as_alias` test. |
| `household.json` | Reeve's own household manifest. Currently uses `workspace` (reverted during lore-scaffold work). | Flip to `meta_repo`. |
| `lore/knowledge/domain/household.md` | Ubiquitous-language entry mentioning the rename. | Replace the "alias for one release" language with "the old `workspace` field name is no longer accepted." |
| `lore/knowledge/general/architecture.md` | "Manifest schema migration" section. | Same update. |
| `CLAUDE.md` | The longer-form CLAUDE.md the user keeps; has its own "Manifest schema migration" section. | Same update. |
| `lore/knowledge/_index.json` | Generated by `make build-index`. | Rebuilt after the .md edits. |

---

## Stage A: Lorekeeper

### Task A.1: Update `cultivate-detect.js` + test fixture; commit

**Working directory:** `/home/daniel/Source/lorekeeper`

**Files:**
- Modify: `scripts/cultivate-detect.js` (lines 46, 115)
- Modify: `scripts/__tests__/cultivate-detect.test.js` (lines 20, 23)

- [ ] **Step 1: Create a new branch**

```bash
cd /home/daniel/Source/lorekeeper
git checkout main
git pull --ff-only 2>/dev/null || true   # no-op if no remote
git checkout -b feat/drop-workspace-alias
```

- [ ] **Step 2: Edit `scripts/cultivate-detect.js`**

The file has two identical filter expressions on lines 46 and 115. Both look like:

```javascript
.filter((r) => r.name !== manifest.workspace && r.name !== manifest.knowledge_base)
```

Change both to:

```javascript
.filter((r) => r.name !== manifest.meta_repo && r.name !== manifest.knowledge_base)
```

Use `sed` (matches both occurrences cleanly):

```bash
sed -i 's/manifest\.workspace/manifest.meta_repo/g' scripts/cultivate-detect.js
```

Verify with grep:

```bash
git grep -n 'manifest\.workspace' scripts/cultivate-detect.js
```

Expected: no output (both replaced).

- [ ] **Step 3: Edit the test fixture**

`scripts/__tests__/cultivate-detect.test.js` line 20 currently reads:

```javascript
workspace: 'test-workspace',
```

(Inside a sample manifest object the test feeds to `detect()`.)

Change to:

```javascript
meta_repo: 'test-workspace',
```

Use `sed`:

```bash
sed -i "s/workspace: 'test-workspace'/meta_repo: 'test-workspace'/" scripts/__tests__/cultivate-detect.test.js
```

Verify with grep:

```bash
git grep -n "workspace: 'test-workspace'" scripts/__tests__/cultivate-detect.test.js
```

Expected: no output.

Then verify the meta_repo replacement is present:

```bash
git grep -n "meta_repo: 'test-workspace'" scripts/__tests__/cultivate-detect.test.js
```

Expected: line ~20 (and possibly the workspace's repos[] entry on line 23 if it references it by reading meta_repo; that should be untouched — it's a `{ name: 'test-workspace' }` entry, not a `workspace:` field).

- [ ] **Step 4: Run all lorekeeper tests**

```bash
cd /home/daniel/Source/lorekeeper
node --test scripts/__tests__/*.test.js 2>&1 | tail -10
```

Expected: all tests pass. (The cultivate-detect tests now use `meta_repo` in their fixtures; the production code reads `manifest.meta_repo`. Symmetric.)

- [ ] **Step 5: Verify nothing live references the old name**

```bash
git grep -nE 'manifest\.workspace|m\.workspace|"workspace":' scripts/ 2>&1
```

Expected: no output. (Doc references inside `docs/superpowers/plans/` and `docs/superpowers/specs/` are historical and untouched — they live outside `scripts/`.)

- [ ] **Step 6: Commit**

```bash
git add scripts/cultivate-detect.js scripts/__tests__/cultivate-detect.test.js
git commit -m "$(cat <<'EOF'
refactor: read manifest.meta_repo instead of manifest.workspace

Aligns Lorekeeper with the renamed Manifest field (Reeve's
reeve-household crate has used meta_repo as the canonical name since
the 2026-05-18 domain refactor; the serde alias accepting "workspace"
is being retired across all three Witan tools).

- scripts/cultivate-detect.js: rewrite the 2 filter sites that read
  manifest.workspace.
- scripts/__tests__/cultivate-detect.test.js: update the fixture's
  field name.

Spec: /home/daniel/Source/reeve/docs/superpowers/specs/2026-05-25-drop-workspace-alias-design.md

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

Confirm the commit landed:

```bash
git log --oneline -1
```

Expected output starts with the new SHA + the commit subject.

---

## Stage B: witan-household

### Task B.1: Update template `household.json`, `lore/_tools/doctor.js`, all four shell scripts, `README.md`, and the doctor test fixtures; commit

**Working directory:** `/home/daniel/Source/witan-household`

**Files:**
- Modify: `household.json` (line 2 — the field name in the template's own manifest)
- Modify: `lore/_tools/doctor.js` (lines 82, 83, 86, 88, 98 — manifest reads + error message)
- Modify: `scripts/rename.sh` (lines 26, 39, 42 — reads + writes the field)
- Modify: `scripts/setup.sh` (line 65 — filter)
- Modify: `scripts/pull-all.sh` (line 16 — filter)
- Modify: `scripts/status-all.sh` (line 15 — filter)
- Modify: `README.md` (lines ~91, ~122 — sample snippets)
- Modify: `lore/_tools/__tests__/doctor.test.js` (lines 18, 33, 46 — fixture manifests)

- [ ] **Step 1: Create a new branch**

```bash
cd /home/daniel/Source/witan-household
git checkout main
git pull --ff-only 2>/dev/null || true
git checkout -b feat/drop-workspace-alias
```

- [ ] **Step 2: Edit `household.json` (the template's own manifest)**

The file currently has:

```json
{
  "workspace": "my-workspace",
  "knowledge_base": "lore",
  ...
}
```

Change line 2 to use `meta_repo` instead. Use `sed` (only ONE occurrence in this file, so the substitution is safe):

```bash
sed -i 's/"workspace": "my-workspace"/"meta_repo": "my-workspace"/' household.json
```

Verify:

```bash
head -5 household.json
```

Expected: the `"meta_repo": "my-workspace"` line on line 2.

- [ ] **Step 3: Edit `lore/_tools/doctor.js`**

Five sites need editing. Look at the file:

```bash
grep -n 'workspace\|manifest\.\(workspace\)' lore/_tools/doctor.js
```

Expected matches at lines 82, 83, 86, 88, 98.

The replacements:

| Line | From | To |
|---|---|---|
| 82 | `if (!manifest.workspace) {` | `if (!manifest.meta_repo) {` |
| 83 | `issues.error.push('household.json: missing required "workspace" field');` | `issues.error.push('household.json: missing required "meta_repo" field');` |
| 86 | `const workspaceEntry = (manifest.repos \|\| []).find(r => r.name === manifest.workspace);` | `const workspaceEntry = (manifest.repos \|\| []).find(r => r.name === manifest.meta_repo);` |
| 88 | `issues.error.push(\`household.json: workspace = "${manifest.workspace}" has no matching repos[] entry\`);` | `issues.error.push(\`household.json: meta_repo = "${manifest.meta_repo}" has no matching repos[] entry\`);` |
| 98 | `if (entry.name === manifest.workspace) continue;` | `if (entry.name === manifest.meta_repo) continue;` |

The variable name `workspaceEntry` on line 86 can stay — it's a local variable name, not a field reference, and renaming it would be churn (the user's other tools might reference it; let's confine the change to field-name reads). Same logic for the user-facing error string on line 83: it must change because it names the field that's missing, not a concept.

Apply with sed (multiple substitutions; all match the same patterns):

```bash
sed -i 's/manifest\.workspace/manifest.meta_repo/g' lore/_tools/doctor.js
sed -i 's/"workspace" field/"meta_repo" field/' lore/_tools/doctor.js
sed -i 's/workspace = "/meta_repo = "/' lore/_tools/doctor.js
```

Verify:

```bash
git grep -nE 'manifest\.workspace|"workspace" field|workspace = "' lore/_tools/doctor.js
```

Expected: no output.

- [ ] **Step 4: Edit `scripts/rename.sh`**

Three sites at lines 26, 39, 42. The script reads `household.json`'s `workspace` field, then rewrites it. It needs to read/write `meta_repo` after this change.

Current line 26:
```bash
OLD_NAME=$(node -e "console.log(require('./household.json').workspace)")
```

Change to:
```bash
OLD_NAME=$(node -e "console.log(require('./household.json').meta_repo)")
```

Current lines 39 + 42 (inside a `node -e` block):
```javascript
const old = m.workspace;
...
m.workspace = next;
```

Change to:
```javascript
const old = m.meta_repo;
...
m.meta_repo = next;
```

Apply with sed:

```bash
sed -i "s/\\.workspace)\"/.meta_repo)\"/" scripts/rename.sh
sed -i 's/m\.workspace = next/m.meta_repo = next/' scripts/rename.sh
sed -i 's/const old = m\.workspace/const old = m.meta_repo/' scripts/rename.sh
```

Verify:

```bash
git grep -nE 'm\.workspace|\.workspace\)' scripts/rename.sh
```

Expected: no output.

- [ ] **Step 5: Edit `scripts/setup.sh`, `scripts/pull-all.sh`, `scripts/status-all.sh`**

Each has one filter site referring to `m.workspace`. Apply the same `sed` to all three:

```bash
sed -i 's/m\.workspace/m.meta_repo/g' scripts/setup.sh scripts/pull-all.sh scripts/status-all.sh
```

Verify:

```bash
git grep -nE 'm\.workspace' scripts/
```

Expected: no output.

- [ ] **Step 6: Edit `README.md`**

Two sites in `README.md`:

Line ~91 is a JSON snippet showing `"workspace": "my-workspace"`. Change to `"meta_repo": "my-workspace"`.

Line ~122 is a Makefile snippet using `jq -r '.workspace' household.json`. Change to `jq -r '.meta_repo' household.json`.

Apply with sed (be careful — README.md may have multiple `workspace` occurrences in prose; we want to be specific):

```bash
sed -i 's/"workspace": "my-workspace"/"meta_repo": "my-workspace"/' README.md
sed -i "s/jq -r '\\.workspace' household.json/jq -r '.meta_repo' household.json/" README.md
```

Verify the field-name references are gone (other prose mentions of "workspace" as a word are fine — we only care about manifest-field references):

```bash
git grep -nE '"workspace":|\.workspace[^_-]' README.md
```

Expected: no output. (If you see something, inspect — it might be a prose mention that's not a JSON field reference.)

- [ ] **Step 7: Edit `lore/_tools/__tests__/doctor.test.js`**

Three fixture sites at lines 18, 33, 46. Each has a JS object like:

```javascript
const manifest = {
    workspace: 'ws',
    ...
};
```

Change all three to use `meta_repo` instead. Use `sed` (matches the property name in a JS object literal):

```bash
sed -i "s/workspace: 'ws'/meta_repo: 'ws'/g" lore/_tools/__tests__/doctor.test.js
sed -i "s/workspace: 'ghost'/meta_repo: 'ghost'/" lore/_tools/__tests__/doctor.test.js
```

Verify:

```bash
git grep -nE "workspace: '" lore/_tools/__tests__/doctor.test.js
```

Expected: no output.

Important: the test that asserts on the error message `'household.json: missing required "meta_repo" field'` (it was `"workspace"` before) may need updating if it's still asserting on the old text. Check:

```bash
grep -n 'missing required' lore/_tools/__tests__/doctor.test.js
```

If it shows an assertion against `"workspace"`, update to `"meta_repo"`:

```bash
sed -i 's/missing required "workspace"/missing required "meta_repo"/' lore/_tools/__tests__/doctor.test.js
```

- [ ] **Step 8: Run all witan-household tests**

```bash
cd /home/daniel/Source/witan-household
node --test lore/_tools/__tests__/*.test.js 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 9: Smoke-test the template end-to-end**

Run a `doctor` against the template itself (which now uses `meta_repo` in its own `household.json`):

```bash
node lore/_tools/cli.js doctor --dir lore/knowledge 2>&1 | tail -5
```

Expected: `Summary: 0 error(s)` (or possibly the same `[OK]/[FAIL]` lines we've seen before, but no `missing required "meta_repo" field` error).

If doctor flags `_index.json` as stale, rebuild it:

```bash
node lore/_tools/cli.js build-index --dir lore/knowledge 2>&1 | tail -1
node lore/_tools/cli.js doctor --dir lore/knowledge 2>&1 | tail -5
```

- [ ] **Step 10: Verify no residual field-name reads**

```bash
git grep -nE 'manifest\.workspace|m\.workspace|"workspace":|\.workspace[^_-]' household.json lore/_tools/ scripts/ README.md 2>&1
```

Expected: no output. (Doc references in `docs/superpowers/` are out of scope — they're historical plans.)

- [ ] **Step 11: Commit**

```bash
git add household.json lore/_tools/doctor.js lore/_tools/__tests__/doctor.test.js \
        scripts/rename.sh scripts/setup.sh scripts/pull-all.sh scripts/status-all.sh \
        README.md
git commit -m "$(cat <<'EOF'
refactor: rename manifest "workspace" field to "meta_repo"

Aligns the witan-household template with Reeve's renamed Manifest field
(canonical since the 2026-05-18 domain refactor; the serde alias is
being retired across all three Witan tools).

Eight files touched (live code + sample snippets):
- household.json: the template's own manifest now writes "meta_repo".
- lore/_tools/doctor.js: 5 sites reading manifest.workspace, plus the
  user-facing "missing required" error message.
- scripts/rename.sh: 3 sites reading and writing the field.
- scripts/setup.sh, pull-all.sh, status-all.sh: 1 filter site each.
- README.md: JSON snippet + jq snippet.
- lore/_tools/__tests__/doctor.test.js: 3 fixture manifests + 1
  error-message assertion.

Spec: /home/daniel/Source/reeve/docs/superpowers/specs/2026-05-25-drop-workspace-alias-design.md

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

Confirm:

```bash
git log --oneline -1
```

Expected: the new commit on top.

---

## Stage C: Reeve

### Task C.1: Drop the serde alias + deprecation warning + alias test

**Working directory:** `/home/daniel/Source/reeve` (on branch `feat/drop-workspace-alias`, which already has the spec commit + the .gitignore cleanup commit).

**Files:**
- Modify: `crates/reeve-household/src/manifest.rs` (the `Manifest` struct attribute, the `load_manifest` warning block, the alias test)

- [ ] **Step 1: Confirm you're on the right branch**

```bash
cd /home/daniel/Source/reeve
git branch --show-current
```

Expected: `feat/drop-workspace-alias`

- [ ] **Step 2: Inspect the current Manifest struct attribute**

```bash
grep -n -B 2 -A 5 'rename = "meta_repo"' crates/reeve-household/src/manifest.rs
```

You should see something like:

```rust
    /// Name of the repos[] entry that IS the meta-repo.
    /// Renamed from `workspace` (2026-05); old name accepted via alias for
    /// one release, will be removed.
    #[serde(rename = "meta_repo", alias = "workspace")]
    pub meta_repo: String,
```

- [ ] **Step 3: Drop the serde attribute (and update the doc comment)**

The field is named `meta_repo` in Rust; serde defaults to using the same name in JSON, so the entire `#[serde(rename = "meta_repo", alias = "workspace")]` attribute is unnecessary once the alias is removed. Drop the attribute and update the doc comment:

Open `crates/reeve-household/src/manifest.rs` and find the block above. Replace with:

```rust
    /// Name of the repos[] entry that IS the meta-repo.
    /// Renamed from `workspace` in 2026-05; the old field name is no
    /// longer accepted.
    pub meta_repo: String,
```

Use `sed` (multi-line edit via `perl` is more reliable here — `sed` struggles with multi-line patterns):

```bash
perl -i -pe 'BEGIN{undef $/;} s|/// Renamed from `workspace` \(2026-05\); old name accepted via alias for\n    /// one release, will be removed\.\n    #\[serde\(rename = "meta_repo", alias = "workspace"\)\]|/// Renamed from `workspace` in 2026-05; the old field name is no\n    /// longer accepted.|sg' crates/reeve-household/src/manifest.rs
```

Verify the attribute is gone:

```bash
grep -n 'serde(rename\|serde(alias\|alias = "workspace"' crates/reeve-household/src/manifest.rs
```

Expected: no output.

Verify the field is still declared:

```bash
grep -n 'pub meta_repo: String' crates/reeve-household/src/manifest.rs
```

Expected: one line showing the field declaration with the updated doc comment above it.

- [ ] **Step 4: Drop the deprecation warning in `load_manifest`**

Find the `load_manifest` function and the warning block:

```bash
grep -n -B 1 -A 7 'deprecated field `workspace`' crates/reeve-household/src/manifest.rs
```

You'll see something like:

```rust
    // Detect the deprecated `workspace` field name and warn.
    if body.contains("\"workspace\"") && !body.contains("\"meta_repo\"") {
        tracing::warn!(
            "manifest at {} uses deprecated field `workspace`; rename to `meta_repo`. \
             Alias support will be removed in a future release.",
            path.display()
        );
    }
```

Delete this entire block. Use `perl` for the multi-line removal:

```bash
perl -i -pe 'BEGIN{undef $/;} s|\n    // Detect the deprecated `workspace` field name and warn\.\n    if body\.contains\("\\\\"workspace\\\\""\) && !body\.contains\("\\\\"meta_repo\\\\""\) \{\n        tracing::warn!\(\n            "manifest at \{\} uses deprecated field `workspace`; rename to `meta_repo`\. \\\\\n             Alias support will be removed in a future release\.",\n            path\.display\(\)\n        \);\n    \}\n||sg' crates/reeve-household/src/manifest.rs
```

If the `perl` command above is too fragile, do it by hand: open `manifest.rs` in your editor, find the block (it's right after `let body = std::fs::read_to_string(&path)...` and before the `let manifest: Manifest = serde_json::from_str(...)` line), and delete it.

Verify the warning is gone:

```bash
grep -n 'deprecated field' crates/reeve-household/src/manifest.rs
```

Expected: no output.

Verify `load_manifest` still compiles around the removal:

```bash
cargo build -p reeve-household 2>&1 | tail -5
```

Expected: `Finished` with no errors. (Warnings about unused `tracing` import may surface — handle in next step.)

If `tracing` is now unused, remove the import:

```bash
git grep -n 'use tracing\|tracing::' crates/reeve-household/src/manifest.rs
```

If the only `tracing::` reference was the deleted warning, remove `use tracing;` (or the specific imported items).

- [ ] **Step 5: Delete the alias-back-compat test**

Find it:

```bash
grep -n -B 1 -A 10 'old_workspace_field_name_still_parses_as_alias' crates/reeve-household/src/manifest.rs
```

You'll see the entire test function. Delete it. Use `perl`:

```bash
perl -i -pe 'BEGIN{undef $/;} s|\n    #\[test\]\n    fn old_workspace_field_name_still_parses_as_alias\(\) \{\n[^\}]*?\n    \}\n||sg' crates/reeve-household/src/manifest.rs
```

Verify:

```bash
grep -n 'old_workspace_field_name_still_parses_as_alias' crates/reeve-household/src/manifest.rs
```

Expected: no output.

- [ ] **Step 6: Run the manifest crate tests**

```bash
cargo test -p reeve-household 2>&1 | grep 'test result'
```

Expected: all tests pass (one less than before — the alias test was deleted).

- [ ] **Step 7: Run the full workspace tests to check for regressions**

```bash
cargo test --workspace --lib 2>&1 | tail -5
cargo test -p reeved 2>&1 | grep 'test result' | tail -3
```

Expected: all pass. (No regressions — the alias removal doesn't affect Reeve's daemon-level behaviour beyond rejecting the old field name in manifests, which we cover in C.2.)

### Task C.2: Flip `household.json` from `workspace` to `meta_repo`

**Files:**
- Modify: `household.json` (line 2)

- [ ] **Step 1: Inspect the current household.json**

```bash
head -5 household.json
```

Expected: line 2 is `"workspace": "reeve",`.

- [ ] **Step 2: Flip the field name**

```bash
sed -i 's/"workspace": "reeve"/"meta_repo": "reeve"/' household.json
```

Verify:

```bash
head -5 household.json
```

Expected: line 2 is now `"meta_repo": "reeve",`.

- [ ] **Step 3: Verify Reeve's daemon parses the new manifest correctly**

The simplest check is running the manifest tests with the updated daemon code (alias removed). The tests use synthetic manifests, but we can also smoke-test `make doctor` against the real household.json:

```bash
make doctor 2>&1 | tail -5
```

Expected: `Summary: 0 error(s), 0 warning(s)` (or possibly `1 warning(s)` if the index needs rebuilding — we handle that in Task C.3).

### Task C.3: Update lore + CLAUDE.md text; rebuild index

**Files:**
- Modify: `lore/knowledge/domain/household.md` (the rename mention)
- Modify: `lore/knowledge/general/architecture.md` (the "Manifest schema migration" section)
- Modify: `CLAUDE.md` (the "Manifest schema migration" section)
- Modify: `lore/knowledge/_index.json` (rebuilt automatically)

- [ ] **Step 1: Update `lore/knowledge/domain/household.md`**

Find the line that mentions the deprecation:

```bash
grep -n -B 1 -A 2 "deprecated name `workspace`\|alias for one release\|Alias support will be removed" lore/knowledge/domain/household.md
```

The text currently says something like:

```
The deprecated name `workspace` is accepted via `#[serde(alias)]` for one release; loading a manifest that uses the old name emits a `tracing::warn`. New scaffolds always write `meta_repo`.
```

Replace with:

```
Renamed from `workspace` in 2026-05. The old field name is no longer accepted as of 2026-05 — manifests that still use it must be migrated (one-line `sed -i 's/"workspace":/"meta_repo":/' household.json`).
```

Use `perl` for the multi-line replacement (since `sed -i` is awkward with rich content):

Open `lore/knowledge/domain/household.md` in your editor and find the bullet point under "Ubiquitous language" or "Invariants" that begins with `Manifest field rename.` or `The deprecated name`. Replace the wording as shown. Verify by re-grepping:

```bash
grep -n 'no longer accepted\|workspace alias\|alias for one release\|will be removed' lore/knowledge/domain/household.md
```

Expected: at least one match showing the new "no longer accepted" wording; no matches of "alias for one release" or "will be removed".

- [ ] **Step 2: Update `lore/knowledge/general/architecture.md`**

Find the "Manifest field deprecation policy" section:

```bash
grep -n -B 1 -A 5 'Manifest field deprecation policy\|alias for one release\|Alias support will be removed' lore/knowledge/general/architecture.md
```

The current text says something like:

```
The Manifest field renamed from `workspace` to `meta_repo` in 2026-05. The old name is accepted via `#[serde(rename = "meta_repo", alias = "workspace")]`; loading a manifest with the old name emits a `tracing::warn`. The witan-household template writes the new name unconditionally; existing `household.json` files in users' workspaces continue to parse. Alias support will be removed in a future release...
```

Replace with:

```
The Manifest field renamed from `workspace` to `meta_repo` in 2026-05. The original alias-and-deprecation-warning phase ran from 2026-05 through 2026-05-25; the alias was then dropped across all three Witan tools (Reeve, Lorekeeper, witan-household). Manifests using the old `workspace` field name no longer parse — they must be migrated with `sed -i 's/"workspace":/"meta_repo":/' household.json`. The pattern generalises: rename-with-alias for one release, then drop. See the 2026-05-25 spec for the cross-tool rollout details.
```

Edit by hand in your editor — `perl` for multi-line replacement is fragile here.

Verify:

```bash
grep -n 'no longer parse\|original alias-and-deprecation\|will be removed in a future release' lore/knowledge/general/architecture.md
```

Expected: matches showing "no longer parse" and "original alias-and-deprecation"; no match for "will be removed in a future release".

- [ ] **Step 3: Update `CLAUDE.md`**

Find the "Manifest schema migration" section:

```bash
grep -n -B 1 -A 5 'Manifest schema migration\|Alias support will be removed' CLAUDE.md
```

The current text says something like:

```
## Manifest schema migration

`Manifest.workspace` was renamed to `Manifest.meta_repo` in 2026-05. The
serde field has `#[serde(rename = "meta_repo", alias = "workspace")]` so
existing `household.json` files still parse; a `tracing::warn!` fires when
the old name is encountered. The witan-household template repo writes the
new name unconditionally. Alias support will be removed in a future release.
```

Replace with:

```
## Manifest schema migration

`Manifest.workspace` was renamed to `Manifest.meta_repo` in 2026-05. The
serde alias and deprecation-warning phase ran through 2026-05-25; the
alias was then removed across all three Witan tools (Reeve, Lorekeeper,
witan-household). Manifests using the old `workspace` field name no
longer parse — they must be migrated with
`sed -i 's/"workspace":/"meta_repo":/' household.json`. See
`lore/knowledge/general/architecture.md` and the 2026-05-25 spec for
the cross-tool rollout history.
```

Edit by hand. Verify:

```bash
grep -n 'no longer parse\|alias and deprecation-warning phase\|will be removed in a future release' CLAUDE.md
```

Expected: matches for the new wording; none for the old "will be removed in a future release".

- [ ] **Step 4: Rebuild the lore index**

```bash
make build-index 2>&1 | tail -2
```

Expected: `[OK] Index built: ...lore/knowledge/_index.json`.

- [ ] **Step 5: Run `make doctor` against Reeve's now-meta_repo household**

```bash
make doctor 2>&1 | tail -5
```

Expected: `Summary: 0 error(s), 0 warning(s)`.

If doctor still reports `missing required "workspace"`, it means the witan-household-template tooling was not yet updated — go back and verify Task B.1 landed before continuing.

If doctor reports `missing required "meta_repo"` against a real household, something went wrong in Task C.2.

- [ ] **Step 6: Run the full test suite**

```bash
cargo test --workspace --lib 2>&1 | grep 'test result' | tail -5
cargo test -p reeved 2>&1 | grep 'test result' | tail -3
```

Expected: all pass.

### Task C.4: Commit Reeve's changes

**Files staged:** everything from C.1 through C.3.

- [ ] **Step 1: Review the diff**

```bash
git diff --stat
```

Expected: roughly 6 files changed.

```bash
git diff -- crates/reeve-household/src/manifest.rs household.json
```

Verify the manifest.rs change drops the attribute + warning + test, and the household.json flip is correct.

- [ ] **Step 2: Commit**

```bash
git add crates/reeve-household/src/manifest.rs household.json \
        lore/knowledge/domain/household.md \
        lore/knowledge/general/architecture.md \
        CLAUDE.md \
        lore/knowledge/_index.json
git commit -m "$(cat <<'EOF'
refactor(household): drop the workspace field alias and deprecation warning

The Manifest.workspace → Manifest.meta_repo rename landed in Stage 6 of
the 2026-05-18 domain refactor with #[serde(rename, alias)] for one
release. Lorekeeper and witan-household have now been updated to read
manifest.meta_repo (see the earlier two commits in this rollout). Reeve
can therefore drop:

- The #[serde(rename = "meta_repo", alias = "workspace")] attribute on
  Manifest.meta_repo (serde defaults to using meta_repo without it).
- The tracing::warn! deprecation block in load_manifest.
- The old_workspace_field_name_still_parses_as_alias test in
  manifest.rs::tests.

Reeve's own household.json flips from "workspace": "reeve" back to
"meta_repo": "reeve" (the rename had been reverted during lore-scaffold
work because witan-household tooling still expected the old name; that
constraint is now gone).

Lore + CLAUDE.md migration-policy text updated to reflect that the
alias phase ran from 2026-05-18 to 2026-05-25 and is now complete.

Spec: docs/superpowers/specs/2026-05-25-drop-workspace-alias-design.md

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 3: Confirm the commit**

```bash
git log --oneline -1
git log --oneline feat/drop-workspace-alias ^main
```

Expected: 3 commits on the branch (the spec, the .gitignore cleanup, and now this refactor commit).

---

## Stage D: Cross-repo verification

### Task D.1: Grep all three repos for any residual `workspace`-field reads

**Working directories:** each of the three repos.

- [ ] **Step 1: Grep Lorekeeper**

```bash
cd /home/daniel/Source/lorekeeper
git grep -nE 'manifest\.workspace|m\.workspace|"workspace":' scripts/ commands/ skills/ hooks/ agents/ 2>&1
```

Expected: no output. (Hits inside `docs/superpowers/plans/` or `docs/superpowers/specs/` are historical and allowed.)

- [ ] **Step 2: Grep witan-household**

```bash
cd /home/daniel/Source/witan-household
git grep -nE 'manifest\.workspace|m\.workspace|"workspace":|\.workspace[^_-]' household.json lore/_tools/ scripts/ README.md 2>&1
```

Expected: no output.

- [ ] **Step 3: Grep Reeve**

```bash
cd /home/daniel/Source/reeve
git grep -nE 'serde\(.*alias = "workspace"|"workspace":|m\.workspace' crates/ household.json 2>&1
```

Expected: no output in `crates/` or `household.json`. (Hits inside `lore/knowledge/` — describing the historical rename — are allowed and expected.)

```bash
git grep -nE 'manifest\.workspace|m\.workspace' lore/knowledge/ 2>&1
```

Expected: no output. (Only the description of the rename, not field-access expressions, should remain in lore.)

### Task D.2: Run all tests + doctors across all three repos

- [ ] **Step 1: Lorekeeper tests**

```bash
cd /home/daniel/Source/lorekeeper
node --test scripts/__tests__/*.test.js 2>&1 | tail -3
```

Expected: pass.

- [ ] **Step 2: witan-household tests + doctor**

```bash
cd /home/daniel/Source/witan-household
node --test lore/_tools/__tests__/*.test.js 2>&1 | tail -3
node lore/_tools/cli.js doctor --dir lore/knowledge 2>&1 | tail -5
```

Expected: tests pass; doctor reports `Summary: 0 error(s)` (possibly with a stale-index warning fixable via `node lore/_tools/cli.js build-index --dir lore/knowledge`).

- [ ] **Step 3: Reeve tests + doctor + isolation lint**

```bash
cd /home/daniel/Source/reeve
cargo test --workspace --lib 2>&1 | grep 'test result' | tail -5
cargo test -p reeved 2>&1 | grep 'test result' | tail -3
make doctor 2>&1 | tail -5
./scripts/check-context-isolation.sh
```

Expected: all green.

- [ ] **Step 4: Confirm no Reeve build/clippy regressions**

```bash
cd /home/daniel/Source/reeve
cargo build --workspace 2>&1 | tail -3
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3
```

Expected: clean build (or only pre-existing warnings unrelated to this change).

---

## Final wrap-up (not a task; reminder for the human)

After all three commits land and verification passes:

1. **Merge each branch to its `main`** (each repo independently — no PR ceremony needed since these are local-only):
   - `cd /home/daniel/Source/lorekeeper && git checkout main && git merge --no-ff feat/drop-workspace-alias && git branch -d feat/drop-workspace-alias`
   - `cd /home/daniel/Source/witan-household && git checkout main && git merge --no-ff feat/drop-workspace-alias && git branch -d feat/drop-workspace-alias`
   - `cd /home/daniel/Source/reeve && git checkout main && git merge --no-ff feat/drop-workspace-alias && git branch -d feat/drop-workspace-alias`

2. **Update project memory** to reflect that the manifest field rename rollout is complete (the `MEMORY.md` entry that today says "next is step 3 (lore docs) then step 4 (witan/Lorekeeper)" is stale once both are done).

3. **The stashed `--dangerously-skip-permissions` change** in Reeve's `crates/reeve-workspace/src/lifecycle.rs` is still in your stash list (`stash@{0}` and the duplicate `stash@{1}`). When you're back on `feat/m7-progress-events`, `git stash pop stash@{0}` it and `git stash drop stash@{1}` the duplicate.
