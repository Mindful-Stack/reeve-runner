---
title: React component conventions
description: Naming, file structure, and composition rules for React components in this household.
tags: [frameworks, react, conventions]
---

# React component conventions

> Replace with your team's actual conventions. This is a starter showing structure and how to cross-reference other nodes.

## File structure

- One component per file.
- Filename matches the default export: `UserCard.tsx` exports `UserCard`.
- Co-locate styles: `UserCard.tsx` + `UserCard.module.css` in the same dir.

## Composition

- Functional components only; no class components in new code.
- Hooks at the top of the function; no conditional hook calls.
- Extract custom hooks when logic exceeds ~20 lines.

## Naming

- Components: `PascalCase`.
- Hooks: `useCamelCase`.
- Boolean props: `isX`, `hasY`, `shouldZ`. Avoid bare `flag` or `enabled`.

## Cross-references

- For PR-review expectations, see [[general/_starter]].
- For TypeScript-specific style, see [[languages/_starter]].
