---
title: Pull-request guidelines
description: Conventions for PR titles, descriptions, and reviewer expectations across every repo in this household.
tags: [general, code-review, pr]
---

# Pull-request guidelines

Replace this content with your team's actual standards. This file ships as a starter so newcomers see the shape of a node.

## Title

- Imperative mood, lowercase, no trailing period: `add user-deletion endpoint` not `Added user-deletion endpoint.`
- Keep under 70 characters. The body is for detail.
- Prefix with a conventional-commit type when useful: `feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `test:`.

## Description

A good PR description answers three questions:

1. **What changed?** A bullet list of the user-facing or architectural changes.
2. **Why?** The problem being solved. Link to issues/specs.
3. **How to verify?** A test plan reviewer can execute.

## Review expectations

- At least one approving review before merge.
- All resolved threads.
- CI green.
- See [[domain/_starter]] for domain-specific review concerns.
