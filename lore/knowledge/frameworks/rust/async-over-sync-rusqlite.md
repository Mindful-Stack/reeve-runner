---
title: Async-over-sync — spawn_blocking + blocking_lock around rusqlite
description: How Reeve's Sqlite*Repo adapters expose async traits while using synchronous rusqlite under the hood. The spawn_blocking + blocking_lock pattern lives inside the adapter; consumers never see it. Avoids holding locks across .await points.
tags: [frameworks, rust, async, tokio, rusqlite, sqlite, concurrency]
---

# Async-over-sync (rusqlite + tokio)

`rusqlite` is a synchronous library: `Connection`, `Statement`, and friends block the calling thread. Reeve's traits are async (`#[async_trait] pub trait CardRepo: Send + Sync { async fn list(&self) -> ...; }`). The pattern that bridges them is consistent across every `Sqlite*Repo`: clone an `Arc<Mutex<Connection>>` into a `spawn_blocking` closure, lock with `blocking_lock()` *inside* the closure, do the SQL, return.

This document is the canonical reference for that pattern.

## The pattern

```rust
use anyhow::Result;
use rusqlite::Connection;
use std::sync::Arc;
use tokio::sync::Mutex;

pub(crate) struct SqliteCardRepo {
    conn: Arc<Mutex<Connection>>,
}

#[async_trait]
impl CardRepo for SqliteCardRepo {
    async fn list(&self) -> Result<Vec<Card>> {
        let conn = self.conn.clone();                          // 1. Arc::clone, cheap
        tokio::task::spawn_blocking(move || -> Result<Vec<Card>> {   // 2. move onto blocking pool
            let conn = conn.blocking_lock();                   // 3. acquire the tokio Mutex synchronously
            let mut stmt = conn.prepare(
                "SELECT id, household, brief, column_, created_at, updated_at, needs_heavy, parent_id
                 FROM cards ORDER BY created_at ASC",
            )?;
            let rows = stmt.query_map([], row_to_card)?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .await?                                                 // 4. .await the JoinHandle; flatten the Result
    }
}
```

Four moves, each non-obvious if you haven't seen the pattern before:

1. **`self.conn.clone()`** — Arc clone (refcount bump, never blocks). The closure needs `'static`-ish data, so we can't borrow `self`.
2. **`tokio::task::spawn_blocking(move || { ... })`** — moves the work onto tokio's *blocking* thread pool, which is separate from the async runtime threads. Blocking SQLite calls don't starve the async runtime.
3. **`conn.blocking_lock()`** — `tokio::sync::Mutex` has both `lock().await` (async) and `blocking_lock()` (sync). We use the sync form *inside the blocking closure*. Crucially: we never hold the Mutex across an `.await` point in async code.
4. **`.await?` on the JoinHandle** — `spawn_blocking` returns `Result<T, JoinError>`. The closure returns `Result<T>`. `?` flattens both: a `JoinError` becomes the outer error; a closure error stays the closure error.

## Why every piece is non-negotiable

### Why `spawn_blocking`?

`rusqlite` calls (especially `prepare`, `query_row`, `execute`) can take milliseconds to seconds depending on data size and disk speed. Calling them directly from an async function would block the tokio runtime thread — starving other tasks. `spawn_blocking` moves the work to a thread pool sized for blocking work (default: 512 threads).

### Why `tokio::sync::Mutex` instead of `std::sync::Mutex`?

Because the *daemon* needs to hold the `Connection` across many handlers, and some of those handlers may interact in async ways (e.g., a future "per-card lock" might `lock().await` to coordinate concurrent mvs). `tokio::sync::Mutex` supports both async lock acquisition (`lock().await`, never blocks the runtime) and sync lock acquisition (`blocking_lock()`, used inside `spawn_blocking`). `std::sync::Mutex` only supports sync acquisition, and holding it across an `.await` would risk a runtime deadlock.

In practice, all current Reeve code paths use `blocking_lock()` because the SQL work happens inside `spawn_blocking`. We could theoretically use `std::sync::Mutex`, but the option to add `lock().await` paths in the future is worth the negligible difference.

### Why clone the Arc into the closure?

The closure runs on a blocking pool thread, outliving the calling stack frame. It needs ownership of its data, not a borrow. `Arc::clone` is a cheap refcount bump; no actual data is duplicated.

### Why `blocking_lock` and not `lock().await`?

Because we're *inside* a `spawn_blocking` closure — we have a synchronous context, not an async one. `lock().await` would require an async runtime, but `spawn_blocking` runs the closure as a plain sync function. `blocking_lock()` is the sync API on `tokio::sync::Mutex` that works in a sync context. (Internally, it'll block the thread waiting for the lock, but that's fine — we're on a blocking-pool thread, not a runtime thread.)

## What this avoids

The pattern exists specifically to avoid these pitfalls:

- **Holding a lock across `.await`.** If an async function locks `tokio::sync::Mutex` via `.lock().await` and then `.await`s something else while holding the guard, other tasks waiting on the same mutex are blocked indefinitely until the first task's `.await` resolves. By locking *inside* `spawn_blocking`, the guard lives only for the duration of the sync closure — no async await happens while holding it.
- **Blocking the runtime.** Direct `rusqlite::Connection::execute(...)` in an async function blocks a runtime thread. `spawn_blocking` moves it off the runtime.
- **Cross-thread `Connection` access.** SQLite connections are `Send` but using one from multiple threads concurrently without synchronisation is undefined behaviour. The `Arc<Mutex<Connection>>` shape lets multiple async tasks share *one* connection by serialising access. (We don't use a connection pool — we use a single connection serialised by the mutex, which is fine for our request rate.)

## Common mistakes

```rust
// MISTAKE 1: Locking outside spawn_blocking
async fn list(&self) -> Result<Vec<Card>> {
    let conn = self.conn.lock().await;  // ← async lock OK on its own
    let mut stmt = conn.prepare("SELECT ...")?;  // ← blocks the runtime thread!
    // ...
}
```

The `prepare`/`query` calls block. They must be inside `spawn_blocking`.

```rust
// MISTAKE 2: Holding lock across .await
async fn list(&self) -> Result<Vec<Card>> {
    let conn = self.conn.lock().await;  // ← lock held...
    some_other_async_call().await;       // ← ...across an .await — DEADLOCK RISK
    let mut stmt = conn.prepare("...")?;
    // ...
}
```

Other tasks waiting on `self.conn.lock()` are blocked until `some_other_async_call().await` resolves. If that call itself tries to lock the same mutex, deadlock.

```rust
// MISTAKE 3: Forgetting the Arc clone
async fn list(&self) -> Result<Vec<Card>> {
    tokio::task::spawn_blocking(move || {
        let conn = self.conn.blocking_lock();  // ← can't move self into 'static closure
        // ...
    }).await?
}
```

`self` is a `&Self` (or `&mut Self`); `move` requires the data to be owned. Hence the explicit `let conn = self.conn.clone();` before the closure.

```rust
// MISTAKE 4: spawn_blocking::<Result<T>>(...).await without ?
let result: Result<Result<Vec<Card>>, JoinError> = spawn_blocking(...).await;
// You now have a nested Result; forgetting `?` leaves the JoinError unhandled.
```

The `.await?` flattens both layers. `JoinError` (the closure panicked) becomes the outer error; the closure's `Result` is unwrapped.

## When to deviate

Three legitimate exceptions to this pattern in Reeve:

1. **Truly synchronous helpers that take a `&Connection`.** `init_schema(conn: &Connection) -> Result<()>` is sync; it runs at startup, once, before the runtime is hot. No spawn_blocking needed.
2. **Tests using `Connection::open_in_memory()`** — same as above; tests own their connection in a way that doesn't need cross-task sharing.
3. **`std::process::Command` for one-shot reads** — `git_remote_url(meta_repo)` in `reeve-household/src/handler.rs` uses sync `std::process::Command` because (a) it's called once during `household.show` and (b) the entire surrounding context is already executing inside an async handler that doesn't block on this particular call.

## See also

- [[frameworks/rust/repo-port-adapter]] for the broader pattern that this fits inside (every `Sqlite*Repo` uses this).
- [[general/architecture]] §"Adapters are pub(crate)" — this pattern is implementation detail; consumers never see it.
- [[general/testing]] — tests construct `Connection::open_in_memory()` synchronously, then drive the trait through the factory.
