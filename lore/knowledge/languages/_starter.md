---
title: TypeScript code style
description: Language-level conventions for TypeScript code in this household.
tags: [languages, typescript, style]
---

# TypeScript code style

> Replace with your team's actual style guide.

## Types

- `interface` for public shapes, `type` for unions and computed types.
- No `any` unless interfacing with an untyped library; prefer `unknown` + narrowing.
- Function return types explicit at API boundaries; inferred elsewhere.

## Naming

- `PascalCase` for types and classes, `camelCase` for variables and functions.
- Boolean variables: `isX`, `hasY`. Avoid bare `flag`.
- Constants: `SCREAMING_SNAKE_CASE` only for module-level immutable primitives.

## Imports

- Absolute imports (`@/foo/bar`) for cross-module references.
- Relative imports (`./baz`) for same-module siblings.
- Never `import * as X` for first-party code.

See [[frameworks/_starter]] for React-specific overlay on this.
