---
title: Commit and PR conventions
description: How Reeve commits look, why they're shaped that way, and the conventions agents and humans follow. Imperative mood, conventional-commit prefixes with milestone numbers, heredoc body for non-trivial commits, Co-Authored-By trailers for agent work.
tags: [general, git, commits, conventions, agents]
---

# Commit and PR conventions

Reeve has a strong convention for commit messages, and it has paid off — `git log --oneline` is a readable history of the project's evolution. These conventions exist so the log answers two questions at a glance: *what kind of change is this* and *what milestone or task did it belong to*.

## Subject line

- **Imperative mood, lowercase, no trailing period.** `add foo bar`, not `Added foo bar.` or `Adds foo bar`.
- **Under 70 characters** if you can help it. The body is for detail.
- **Conventional-commit-style prefix** with the milestone or task in parentheses:

| Prefix | When |
|---|---|
| `feat(mX):` | Feature implementation for milestone X. |
| `feat(N.X):` | Feature within refactor task N.X. |
| `fix(mX):` | Bug fix landed during milestone X work. |
| `fix(<topic>):` | Bug fix outside a milestone (e.g. `fix(observability):`). |
| `refactor(N.X):` | Refactor step N.X (e.g. `refactor(2.5):` for Stage 2 Task 5). |
| `polish(mX-followup):` | Polish/cleanup as part of milestone follow-up. |
| `test(mX):` | Test added/improved during milestone X. |
| `docs(<topic>):` | Documentation. `docs(arch):`, `docs(m5):`, etc. |
| `build(N.X):` | Build/scaffolding (Cargo.toml, scripts, etc.). |
| `chore:` | Housekeeping that doesn't fit elsewhere (Cargo.lock refresh, dead-code removal, fmt). |
| `revert(mX):` | Revert of a previous milestone-X commit. |
| `tweak(mX):` | Small adjustments (e.g. `tweak(m6): bump mv debounce 200ms → 400ms`). |
| `simplify(N.X-followup):` | Simplification of earlier work. |

Real examples from `git log`:

- `feat(m6): debounce mv RPCs by 200ms to coalesce rapid H/L presses`
- `refactor(4.5): wire LiveWorkspaceLifecycle; delete stubs + transitions + effects::LiveEffects`
- `fix(m3-followup): handle_mv runs lifecycle before column commit`
- `polish(m3-followup): mv-self parses params before lock + test cleanup`
- `docs(8.3): document architecture in CLAUDE.md`

The prefix carries the navigation; the subject explains the change. A reader scanning `git log --oneline` for "what M6 changed" greps `feat(m6)|fix(m6)`.

## Body (for non-trivial commits)

A good body answers three questions:

1. **What changed?** Bullet list of the user-facing or architectural changes.
2. **Why?** The problem being solved or the design decision. Link to specs/plans when useful.
3. **How does it work?** A paragraph or two on the mechanism, if it's not obvious from the subject.

For one-line commits (`chore:`, `tweak:`), no body is fine — the subject is enough.

For substantial commits (anything that touches >3 files or changes architecture), write the body via a heredoc to preserve formatting:

```sh
git commit -m "$(cat <<'EOF'
refactor(4.5): wire LiveWorkspaceLifecycle; delete stubs + transitions + effects::LiveEffects

reeved/src/transitions.rs deleted (owned by reeve-card now).
reeved/src/stubs.rs deleted (real impls in reeve-workspace + reeve-household).
DaemonState gains workspace_lifecycle Arc.
ListenerRegistry impls reeve_workspace::ListenerOps for cross-crate use.
EOF
)"
```

The single-quoted heredoc preserves `$`, backticks, etc. without expansion.

## Co-Authored-By trailer for agent work

Any commit produced by a Claude agent ends with:

```
Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
```

(Replace the model name with whichever was used; the email stays constant.)

This is a real GitHub convention — the trailer surfaces in PR contribution counts and is honest about authorship. Hand-written commits (or commits the agent helped draft but you typed out) don't need the trailer.

The Reeve git log is roughly 80% agent-coauthored at this point. The convention has not been controversial; the alternative ("just don't acknowledge") would be a step backwards in honesty about what built this.

## Linking to plans and specs

When a commit implements a step of a written plan, reference it in the body:

```
Task 4.7 of docs/superpowers/plans/2026-05-18-domain-driven-refactor.md.
Spec at docs/superpowers/specs/2026-05-18-domain-driven-refactor-design.md.
```

These references make `git log --grep` find the implementation when you remember the spec name.

## Squash vs preserve

Reeve historically preserves the per-step commit history within a feature branch (commits like `refactor(2.1)`, `refactor(2.2)`, ...) and merges with `--no-ff`. Each commit is small and readable; the merge commit aggregates the branch with its own multi-paragraph body.

Don't squash a 38-commit refactor into one commit — you lose the per-step traceability. Don't squash a 3-commit hotfix into one commit either, but a single commit branch needs no special handling.

## When CI / hooks fail

The conventions around hooks: never bypass with `--no-verify`. If a hook (cargo fmt, clippy, the isolation lint) fails, fix the issue rather than skip the check. The hook exists because the check matters.

If the failing hook is wrong (false positive), fix the hook in a separate commit before the work commit. Don't carry a `--no-verify` in your shell history.

## See also

- [[general/architecture]] for the architecture rules; commit prefixes reflect refactor steps.
- [[general/testing]] for what tests must pass before commit.
