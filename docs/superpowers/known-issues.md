# Reeve — known issues

Smaller-than-card items discovered during smoke testing or normal use. Each entry has a one-line summary, the symptom, a guess at severity, and a sketch of a fix direction. Items get promoted to a card (or batched into one) when picked up.

> See "Collecting these in Reeve" at the bottom of this file for the open design question on whether to introduce a first-class `task` concept below `card`.

---

## #1 — `clone_all_for_card` refuses shallow sibling sources with a confusing error

**Symptom.** When a sibling repo on the host was cloned with `--depth=N` (a common case for users who shallow-cloned to save disk), `clone_all_for_card` errors out. The error is propagated from `git clone --reference --dissociate` and doesn't surface a clear "your source repo is shallow; use a full clone or change strategy" message.

**Severity.** Medium. Real users hit this — shallow clones are common. The user blames Reeve when it's a constraint of the `--reference` mechanism.

**Fix direction.** Two options:
1. Detect the shallow state up-front (`git -C <sibling> rev-parse --is-shallow-repository`) and either (a) fall back to a plain `git clone` without `--reference` (slower per card but works), or (b) refuse with a clear error and a recommended remediation.
2. Always fall back to plain clone when `--reference --dissociate` fails; loses the cheap object-share but works.

Lean: (1)(a). Detect-and-fallback preserves the cheap clone for normal cases and degrades gracefully for shallow ones. A log line records the fallback so we know it happened.

**Source location.** `crates/reeved/src/clones.rs::clone_all_for_card`.

---

## #2 — Orphan workspace + container on `ensure_running` mid-flow failure

**Symptom.** If `devcontainer up` exits non-zero (or any other later step fails) after `clone_all_for_card` succeeded, the workspace directory at `~/reeve/cards/<id>/` remains on disk and a partial Docker container may exist. The cleanup in `clone_all_for_card` only fires when clones themselves fail — failures downstream leak state. The next time the user runs `reeve mv <id> implementing`, item #3 below also kicks in.

**Severity.** Medium-high. Failures compound: every retry that fails between clone and devcontainer-up leaves more orphan dirs and partial containers.

**Fix direction.** `ensure_running` needs a defer-style cleanup that fires on any error path past `clone_all_for_card`. Pattern:

```rust
let workspace_guard = Cleanup::on_drop(|| {
    if let Err(e) = std::fs::remove_dir_all(&workspace) { warn!(...); }
});
// ...clone, devcontainer up, etc...
workspace_guard.cancel();   // success: don't fire cleanup
```

Plus a teardown of any partially-created container (`docker rm -f reeve-<card-id>` best-effort) on the error path.

**Source location.** `crates/reeved/src/effects.rs::ensure_running`.

---

## #3 — `mv → Implementing` with pre-existing workspace produces a low-level git error

**Symptom.** If `~/reeve/cards/<id>/` already exists (e.g. from a prior failed `→ Implementing` per #2, or manual user action), the next `mv → Implementing` tries to clone into the existing directory and gets a `fatal: destination path '...' already exists and is not an empty directory` from git. The error reaches the user without context.

**Severity.** Low-medium. The card is stuck until the user manually `rm -rf`s the directory. The fix message would unblock self-service.

**Fix direction.** Pre-flight check at the start of `ensure_running`: if the workspace dir exists, return an error of the shape "workspace at `<path>` already exists from a previous attempt. Run `rm -rf <path>` to clean up, then retry `reeve mv <id> implementing`." Once #2 is fixed, this case becomes rare; until then, a clear message is the workaround.

**Source location.** `crates/reeved/src/effects.rs::ensure_running` — first guard before any clone/spawn work.

---

## #4 — Pre-existing: zombie `reeved` daemons survive `reeve daemon stop`

**Symptom.** `reeve daemon stop` reads the pidfile, sends SIGTERM to that pid, exits. Older `reeved` processes from earlier sessions whose pidfile got overwritten (or was never written) are not touched. Result: multiple `reeved` instances running, contending for the socket and the sqlite file.

**Severity.** Medium for power users; low for typical daily use. Symptoms are confusing (random RPC failures, double-fetch-worker activity once M4 lands) when multiple daemons run.

**Fix direction.** `reeve daemon stop` could:
1. Read the pidfile and SIGTERM as today.
2. Also `pgrep -f 'reeved'` (or `ss --listening --processes ... :<socket>` if the socket path is known) to find any other reeved processes.
3. Report each: "found N additional `reeved` PIDs: ... ; kill them with `kill <pid>` or run `reeve daemon stop --all` to force-kill all."

Or, more aggressively: on `reeve daemon start`, refuse to start if another `reeved` is running. The current behavior of letting them coexist is what enables the zombie state.

Predates the unification work; latent for a while.

**Source location.** `crates/reeve/src/cli.rs` (`stop` subcommand handler) and `crates/reeved/src/main.rs` (startup-time uniqueness check).

---

## Collecting these in Reeve — open design question

These items are real but smaller than a card warrants. A "card" today spawns its own container, gets its own kanban movement, takes meaningful planning. Filing each of these as a card would be ceremony out of proportion to the work.

Sketch of a possible `task` concept:

- A `task` is a low-friction backlog item: title, description, status (open/done), optional `card_id`. Stored in sqlite alongside cards.
- CLI surface roughly:
  ```
  reeve task add "<brief>"
  reeve task ls [--card <id>] [--unassigned]
  reeve task done <id>
  reeve task attach <task-id> <card-id>
  reeve new --tasks t1,t2,...     # create a card seeded with these tasks
  ```
- TUI: a tasks pane (likely on the kanban backlog column) listing unattached tasks; multi-select + `c` to create a card from them.
- An "attached" task shows up in its card's drill-in view as a checkbox the agent works through.
- Done tasks track back to the card they shipped in for traceability.

This is the Linear-style "issues that group into projects" model, adapted to Reeve's card terminology.

Worth a real brainstorm before any implementation. Belongs in the parent design's deferred list (already has "cross-household unified view" — tasks would be peer-level).
