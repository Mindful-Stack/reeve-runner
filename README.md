# Reeve

A TUI orchestrator for parallel Claude Code sessions in a poly-repo setup.

See `docs/superpowers/specs/2026-04-29-reeve-design.md` for the design.

## Building

```sh
cargo build --release
```

## M1 status

Walking skeleton: daemon + CLI + read-only TUI for card metadata.
No containers, agents, or git plumbing yet.
