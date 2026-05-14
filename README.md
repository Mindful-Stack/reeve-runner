# Reeve

A TUI orchestrator for parallel Claude Code sessions in a poly-repo setup.

See `docs/superpowers/specs/2026-04-29-reeve-design.md` for the design.

## Building

```sh
cargo build --release
```

## Status

| Milestone | Description |
|---|---|
| M1 | Walking skeleton: daemon + CLI + read-only TUI for card metadata. |
| M2 | Container lifecycle: `→ Implementing` spawns a Docker container per card; per-card Unix socket identity; daemon-restart reconcile. |
| M3 | Per-card clones via `git clone --reference --dissociate` + `devcontainer up` swap; new household schema (`devcontainer` + `[[repos]]`). |
| M4 | (next) Namespaced refspecs (`refs/reeve/<id>/*`) + `reeve push`. |

## Development

### Required tools

- Rust toolchain ≥ 1.85 (Rust 2024 edition)
- `git`
- `docker` (the `docker` CLI; daemon must be running locally)
- `@devcontainers/cli` — `npm install -g @devcontainers/cli`. Required as of M3.
- `sqlite3` (optional — handy for poking at `~/.local/share/reeve/reeved.sqlite`)

### Running tests

```sh
cargo test --workspace            # ~98 tests; tool-gated tests skip silently
REEVE_TESTS_STRICT=1 cargo test --workspace   # fail loud if docker or devcontainer is missing
```

The strict mode is what implementers and any CI should use. Without it,
the docker- and devcontainer-gated end-to-end tests skip with a printed
message, which looks indistinguishable from "passed" in normal output —
a real silent-skip bug we hit during M3. Always set it before declaring
a milestone done.

### Manual smoke (per milestone)

End-to-end automated tests catch most regressions. A short manual smoke
at the end of each milestone catches the rest (environment differences,
external-tool version drift). See each milestone's spec under
`docs/superpowers/specs/` for the smoke checklist.
