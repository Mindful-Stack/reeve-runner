---
title: User management — bounded context
description: Domain context for user accounts, authentication, and authorization. One node per bounded context in this household.
tags: [domain, ddd, core]
---

# User management

> Replace this placeholder with your actual bounded context. The shape below follows DDD conventions; keep the headings even when the content changes.

## Purpose

This context owns the lifecycle of user accounts: registration, authentication, profile data, deletion, and the audit trail of each.

## Key entities

- **User** — root aggregate. Holds identity, email, account state.
- **Session** — represents an authenticated browser/client. Belongs to a User.
- **Role** — a named permission set; many-to-many with User.

## Ubiquitous language

- *Account* and *User* are synonyms; *account* is preferred in user-facing copy, *user* in code.
- *Registered* means email confirmed; *unregistered* means email pending.
- *Disabled* (admin-deactivated) is distinct from *deleted* (user-initiated, irreversible after 30 days).

## Integration points

- **Inbound:** registration flow from the marketing site; SSO callbacks from identity providers.
- **Outbound:** account-lifecycle events on the message bus (`user.registered`, `user.deleted`); audit log writes.

## Key workflows

1. **Registration:** marketing-site → backend → email-confirmation → User registered.
2. **Deletion:** user-initiated → 30-day soft-delete → hard-delete cron.

See also [[frameworks/_starter]] for framework conventions used here.
