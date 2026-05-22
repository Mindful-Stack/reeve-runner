---
title: Database connection pool exhaustion under load test
tags: [database, performance, load-testing]
confidence: verified
source: developer-input
date: 2026-05-16
---

# Database connection pool exhaustion under load test

> Replace this with real learnings as your team encounters them. This file shows the shape of a verified learning.

## What happened

During the v2.3 load test, the API tier exhausted its database connection pool at ~120 concurrent requests, leading to 30-second timeouts and cascading retries. The issue did not reproduce on staging because staging's max_connections was lower than production but the API tier's pool size was identical.

## Root cause

A long-running report query (introduced in PR #1234) held a connection for ~8 seconds. Under load, even moderate report-generation traffic could lock out the rest of the API.

## Fix

- Moved the report query to a separate read-replica connection pool.
- Added a per-endpoint connection-acquisition timeout of 2 seconds; over-budget requests fast-fail to 503 with a Retry-After header.

## When this matters

Any future endpoint that holds a DB connection longer than 1 second should follow the same pattern. See [[general/_starter]] for PR-level guardrails.
