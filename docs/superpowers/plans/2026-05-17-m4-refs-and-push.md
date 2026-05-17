# M4 — Namespaced refs + `reeve push` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a continuous `git fetch` worker that surfaces card commits as `refs/reeve/<id>/*` on the host, plus a `reeve push <id>` command that fast-forwards `refs/heads/*` from the namespace and pushes to `origin`.

**Architecture:** Reeved gains four new `GitOps` methods (`fetch`, `list_namespace_refs`, `fast_forward`, `push`), a `FetchWorker` background tokio task driven by a 5-second timer, a `handle_push` RPC that iterates each card's meta-repo + manifest repos through the new methods, and a `Push { id }` CLI subcommand. The meta-repo participates as repo zero — no KB special case, no plugin awareness.

**Tech Stack:** Rust 2024, tokio (async runtime + sync::Mutex + select! shutdown), rusqlite (Connection wrapped in tokio Mutex), `git` subprocess via `tokio::process::Command`.

**Spec:** `docs/superpowers/specs/2026-05-14-m4-refs-and-push-and-kb-design.md`

**Deferred items (writing-plans considered, will NOT implement in M4):**

- **Peer-pid bail** in `FetchWorker` startup (to defend against M3.5 known-issue #4: zombie daemons). Decided against because the root fix should live in `reeve daemon stop` (kill *all* reeved PIDs, not only the pidfile's). A FetchWorker band-aid masks the underlying bug. File as a M3.5 follow-up; revisit if zombie-double-fetch becomes a real symptom.
- **Cleanup of `refs/reeve/<id>/*` on `→ Done`.** Per spec, refs persist for inspection. Open question for M5.

---

## File map

| Path | Responsibility | Change kind |
|---|---|---|
| `crates/reeve-protocol/src/lib.rs` | `PushParams`, `PushResult`, `BranchPushResult`, `BranchPushStatus` types | additive |
| `crates/reeved/src/clones.rs` | `GitOps` trait + `RealGitOps` get 4 new methods; mocks updated | additive |
| `crates/reeved/src/household.rs` | `iter_card_repos(household, manifest)` helper | additive |
| `crates/reeved/src/handlers.rs` | `handle_push` + dispatcher case `"push"`; `NoopGit` test mock updated | additive |
| `crates/reeved/src/fetch.rs` | NEW module: `FetchWorker::tick` and `FetchWorker::run` | new |
| `crates/reeved/src/main.rs` | spawn FetchWorker, hold shutdown sender, await its exit | additive |
| `crates/reeve/src/cli.rs` | `Command::Push { id }` + handler that prints per-branch table | additive |
| `tests/e2e.rs` | `m4_fetch_and_push_round_trip` end-to-end test | additive |

---

## Task 1: Protocol types

**Files:**
- Modify: `crates/reeve-protocol/src/lib.rs`

- [ ] **Step 1: Append the M4 protocol types**

Open `crates/reeve-protocol/src/lib.rs`. Append after the existing `HouseholdDetail` type and before `pub mod paths`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushParams {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushResult {
    pub branches: Vec<BranchPushResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchPushResult {
    /// `manifest.workspace` for the meta-repo; `repos[].name` for siblings.
    pub repo: String,
    pub branch: String,
    pub status: BranchPushStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BranchPushStatus {
    Pushed,
    NotFastForward,
    PushFailed { message: String },
}
```

- [ ] **Step 2: Verify the crate still builds**

Run: `cargo check -p reeve-protocol`
Expected: PASS, zero warnings about the new types.

- [ ] **Step 3: Commit**

```bash
git add crates/reeve-protocol/src/lib.rs
git commit -m "feat(m4): protocol types for push (PushParams/Result, BranchPushResult/Status)"
```

---

## Task 2: `GitOps::fetch`

**Files:**
- Modify: `crates/reeved/src/clones.rs`
- Modify: `crates/reeved/src/handlers.rs` (NoopGit impl in tests)

- [ ] **Step 1: Write the failing unit test for MockGitOps::fetch**

Append inside `crates/reeved/src/clones.rs`'s existing `#[cfg(test)] mod tests` block, before the existing tests' closing brace:

```rust
    #[tokio::test]
    async fn mock_fetch_records_call() {
        let dir = tempdir().unwrap();
        let workdir = dir.path().join("host");
        let source = dir.path().join("card-clone");
        let ops = MockGitOps::new();
        ops.fetch(&workdir, &source, "deadbeef").await.unwrap();
        let fetches = ops.fetches.lock().unwrap();
        assert_eq!(fetches.len(), 1);
        assert_eq!(fetches[0], (workdir.clone(), source.clone(), "deadbeef".to_string()));
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p reeved clones::tests::mock_fetch_records_call`
Expected: FAIL with `no method named 'fetch'` or `no field 'fetches'`.

- [ ] **Step 3: Add `fetch` to the `GitOps` trait**

Edit `crates/reeved/src/clones.rs`. The trait at line 20 becomes:

```rust
#[async_trait]
pub trait GitOps: Send + Sync {
    async fn clone(&self, source: &Path, dest: &Path) -> Result<()>;
    /// Plain `git clone <url> <dest>` — no --reference, no --dissociate.
    /// For adding a remote household whose meta-repo lives off-host.
    async fn clone_url(&self, url: &str, dest: &Path) -> Result<()>;
    /// Run `git fetch <source> '+refs/heads/*:refs/reeve/<namespace>/*'` in `workdir`.
    /// Used by the M4 fetch worker to surface in-container commits on the host.
    async fn fetch(&self, workdir: &Path, source: &Path, namespace: &str) -> Result<()>;
}
```

- [ ] **Step 4: Implement `fetch` on `RealGitOps`**

In `crates/reeved/src/clones.rs`, add inside `impl GitOps for RealGitOps`, after the existing `clone_url` impl:

```rust
    async fn fetch(&self, workdir: &Path, source: &Path, namespace: &str) -> Result<()> {
        let refspec = format!("+refs/heads/*:refs/reeve/{namespace}/*");
        let output = Command::new("git")
            .args([
                "fetch",
                source.to_str().context("source path not utf-8")?,
                &refspec,
            ])
            .current_dir(workdir)
            .output()
            .await
            .context("exec git fetch")?;
        if !output.status.success() {
            anyhow::bail!(
                "git fetch failed (exit {}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }
```

- [ ] **Step 5: Add the `fetches` recording field to `MockGitOps` and implement `fetch`**

In the same file, change `MockGitOps` (currently around line 169) to add a fetches vector. The struct becomes:

```rust
    struct MockGitOps {
        calls: Mutex<Vec<(PathBuf, PathBuf)>>,
        fetches: Mutex<Vec<(PathBuf, PathBuf, String)>>,
    }

    impl MockGitOps {
        fn new() -> Self {
            Self {
                calls: Mutex::new(vec![]),
                fetches: Mutex::new(vec![]),
            }
        }
    }
```

Add `fetch` impl in the `impl GitOps for MockGitOps` block:

```rust
        async fn fetch(&self, workdir: &Path, source: &Path, namespace: &str) -> Result<()> {
            self.fetches.lock().unwrap().push((
                workdir.to_path_buf(),
                source.to_path_buf(),
                namespace.to_string(),
            ));
            Ok(())
        }
```

- [ ] **Step 6: Add `fetch` impl to `InlineSimulatingOps`**

In the same `tests` module (around line 270), add to `impl GitOps for InlineSimulatingOps`:

```rust
        async fn fetch(&self, workdir: &Path, source: &Path, namespace: &str) -> Result<()> {
            self.inner.fetch(workdir, source, namespace).await
        }
```

- [ ] **Step 7: Add `fetch` impl to `NoopGit` in handlers tests**

Edit `crates/reeved/src/handlers.rs`. Find `impl GitOps for NoopGit` (around line 674) and add:

```rust
        async fn fetch(
            &self,
            _workdir: &std::path::Path,
            _source: &std::path::Path,
            _namespace: &str,
        ) -> Result<()> {
            Ok(())
        }
```

- [ ] **Step 8: Run the unit test to verify it passes**

Run: `cargo test -p reeved clones::tests::mock_fetch_records_call`
Expected: PASS.

- [ ] **Step 9: Write the failing integration test for real git fetch**

Append inside `crates/reeved/src/clones.rs`'s `#[cfg(test)] mod integration_tests` block, after the existing tests:

```rust
    #[tokio::test]
    async fn real_fetch_populates_namespaced_refs() {
        let dir = tempdir().unwrap();
        let host = host_repo(dir.path(), "host");
        let card_clone = dir.path().join("card-clone");

        // Clone the host repo as the card workspace would.
        let ops = RealGitOps;
        ops.clone(&host, &card_clone).await.unwrap();

        // Make a new commit in the card clone on a feature branch.
        git_status(&card_clone, &["checkout", "-b", "feat-x"]);
        std::fs::write(card_clone.join("new.txt"), "hi\n").unwrap();
        git_status(&card_clone, &["add", "new.txt"]);
        git_status(&card_clone, &["-c", "user.email=t@t.com", "-c", "user.name=t", "commit", "-m", "x"]);

        // Fetch from host's perspective, populating refs/reeve/<id>/.
        ops.fetch(&host, &card_clone, "card-deadbeef").await.unwrap();

        // Assert host now has refs/reeve/card-deadbeef/feat-x pointing at the new commit.
        let output = std::process::Command::new("git")
            .args(["rev-parse", "refs/reeve/card-deadbeef/feat-x"])
            .current_dir(&host)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "rev-parse should find namespaced ref: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Run git and assert success; helper for tests.
    fn git_status(repo: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(repo)
            .status()
            .expect("git invoke");
        assert!(status.success(), "git {args:?} failed in {}", repo.display());
    }
```

- [ ] **Step 10: Run the integration test**

Run: `cargo test -p reeved clones::integration_tests::real_fetch_populates_namespaced_refs`
Expected: PASS. Requires `git` in `$PATH`.

- [ ] **Step 11: Commit**

```bash
git add crates/reeved/src/clones.rs crates/reeved/src/handlers.rs
git commit -m "feat(m4): GitOps::fetch — namespaced refspec into refs/reeve/<id>/"
```

---

## Task 3: `GitOps::list_namespace_refs`

**Files:**
- Modify: `crates/reeved/src/clones.rs`
- Modify: `crates/reeved/src/handlers.rs` (NoopGit)

- [ ] **Step 1: Write the failing unit test**

Append inside `crates/reeved/src/clones.rs`'s `#[cfg(test)] mod tests`:

```rust
    #[tokio::test]
    async fn mock_list_namespace_refs_returns_recorded() {
        let dir = tempdir().unwrap();
        let workdir = dir.path().join("host");
        let ops = MockGitOps::new();
        ops.set_list_namespace_refs_response(vec!["feat-a".into(), "feat-b".into()]);
        let result = ops.list_namespace_refs(&workdir, "card-x").await.unwrap();
        assert_eq!(result, vec!["feat-a".to_string(), "feat-b".to_string()]);
        let calls = ops.list_namespace_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], (workdir.clone(), "card-x".to_string()));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p reeved clones::tests::mock_list_namespace_refs_returns_recorded`
Expected: FAIL with `no method named 'list_namespace_refs'`.

- [ ] **Step 3: Add `list_namespace_refs` to the trait**

Append to the `GitOps` trait:

```rust
    /// Return branch names found under `refs/reeve/<namespace>/`. The
    /// namespace prefix is stripped; consumers see plain branch names.
    /// Empty vec when there are no refs in that namespace.
    async fn list_namespace_refs(&self, workdir: &Path, namespace: &str) -> Result<Vec<String>>;
```

- [ ] **Step 4: Implement on `RealGitOps`**

```rust
    async fn list_namespace_refs(&self, workdir: &Path, namespace: &str) -> Result<Vec<String>> {
        let prefix = format!("refs/reeve/{namespace}/");
        let output = Command::new("git")
            .args(["for-each-ref", "--format=%(refname)", &prefix])
            .current_dir(workdir)
            .output()
            .await
            .context("exec git for-each-ref")?;
        if !output.status.success() {
            anyhow::bail!(
                "git for-each-ref failed (exit {}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let branches: Vec<String> = stdout
            .lines()
            .filter_map(|line| line.strip_prefix(&prefix).map(|s| s.to_string()))
            .collect();
        Ok(branches)
    }
```

- [ ] **Step 5: Add recording fields + impl to `MockGitOps`**

Update `MockGitOps` struct in the tests module:

```rust
    struct MockGitOps {
        calls: Mutex<Vec<(PathBuf, PathBuf)>>,
        fetches: Mutex<Vec<(PathBuf, PathBuf, String)>>,
        list_namespace_calls: Mutex<Vec<(PathBuf, String)>>,
        list_namespace_response: Mutex<Vec<String>>,
    }

    impl MockGitOps {
        fn new() -> Self {
            Self {
                calls: Mutex::new(vec![]),
                fetches: Mutex::new(vec![]),
                list_namespace_calls: Mutex::new(vec![]),
                list_namespace_response: Mutex::new(vec![]),
            }
        }

        fn set_list_namespace_refs_response(&self, branches: Vec<String>) {
            *self.list_namespace_response.lock().unwrap() = branches;
        }
    }
```

Add to the `impl GitOps for MockGitOps`:

```rust
        async fn list_namespace_refs(&self, workdir: &Path, namespace: &str) -> Result<Vec<String>> {
            self.list_namespace_calls
                .lock()
                .unwrap()
                .push((workdir.to_path_buf(), namespace.to_string()));
            Ok(self.list_namespace_response.lock().unwrap().clone())
        }
```

- [ ] **Step 6: Add impl to `InlineSimulatingOps`**

```rust
        async fn list_namespace_refs(&self, workdir: &Path, namespace: &str) -> Result<Vec<String>> {
            self.inner.list_namespace_refs(workdir, namespace).await
        }
```

- [ ] **Step 7: Add impl to `NoopGit` in handlers tests**

```rust
        async fn list_namespace_refs(
            &self,
            _workdir: &std::path::Path,
            _namespace: &str,
        ) -> Result<Vec<String>> {
            Ok(vec![])
        }
```

- [ ] **Step 8: Run the unit test**

Run: `cargo test -p reeved clones::tests::mock_list_namespace_refs_returns_recorded`
Expected: PASS.

- [ ] **Step 9: Write the failing integration test**

Append in `integration_tests`:

```rust
    #[tokio::test]
    async fn real_list_namespace_refs_returns_branches() {
        let dir = tempdir().unwrap();
        let host = host_repo(dir.path(), "host");

        // Manually create two refs in the namespace.
        let head = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&host)
            .output()
            .unwrap();
        assert!(head.status.success());
        let head_sha = String::from_utf8_lossy(&head.stdout).trim().to_string();

        for branch in ["feat-a", "feat-b"] {
            let refname = format!("refs/reeve/card-x/{branch}");
            let status = std::process::Command::new("git")
                .args(["update-ref", &refname, &head_sha])
                .current_dir(&host)
                .status()
                .unwrap();
            assert!(status.success());
        }

        let ops = RealGitOps;
        let mut branches = ops.list_namespace_refs(&host, "card-x").await.unwrap();
        branches.sort();
        assert_eq!(branches, vec!["feat-a".to_string(), "feat-b".to_string()]);
    }

    #[tokio::test]
    async fn real_list_namespace_refs_returns_empty_when_no_match() {
        let dir = tempdir().unwrap();
        let host = host_repo(dir.path(), "host");
        let ops = RealGitOps;
        let result = ops.list_namespace_refs(&host, "missing-card").await.unwrap();
        assert!(result.is_empty());
    }
```

- [ ] **Step 10: Run integration tests**

Run: `cargo test -p reeved clones::integration_tests::real_list_namespace_refs`
Expected: both tests PASS.

- [ ] **Step 11: Commit**

```bash
git add crates/reeved/src/clones.rs crates/reeved/src/handlers.rs
git commit -m "feat(m4): GitOps::list_namespace_refs — enumerate refs/reeve/<id>/<branch>"
```

---

## Task 4: `GitOps::fast_forward` + `FfOutcome`

**Files:**
- Modify: `crates/reeved/src/clones.rs`
- Modify: `crates/reeved/src/handlers.rs` (NoopGit)

- [ ] **Step 1: Add the `FfOutcome` enum to clones.rs**

Edit `crates/reeved/src/clones.rs`. Add near the top of the file, after the existing imports and before `pub trait GitOps`:

```rust
/// Result of attempting a fast-forward update via `GitOps::fast_forward`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfOutcome {
    /// `target_ref` was updated to point at `source_ref`.
    Updated,
    /// `target_ref` is not an ancestor of `source_ref`; left unchanged.
    NotFastForward,
}
```

- [ ] **Step 2: Write the failing unit test**

Append inside `mod tests`:

```rust
    #[tokio::test]
    async fn mock_fast_forward_returns_configured_outcome() {
        let dir = tempdir().unwrap();
        let workdir = dir.path().join("host");
        let ops = MockGitOps::new();
        ops.set_fast_forward_response(FfOutcome::Updated);
        let result = ops
            .fast_forward(&workdir, "refs/heads/main", "refs/reeve/card-x/main")
            .await
            .unwrap();
        assert_eq!(result, FfOutcome::Updated);
        let calls = ops.fast_forward_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0],
            (
                workdir.clone(),
                "refs/heads/main".to_string(),
                "refs/reeve/card-x/main".to_string()
            )
        );
    }
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -p reeved clones::tests::mock_fast_forward`
Expected: FAIL with missing method.

- [ ] **Step 4: Add to the `GitOps` trait**

```rust
    /// Update `target_ref` to point at `source_ref` only if the move is a
    /// fast-forward (target is an ancestor of source). Returns `Updated` on
    /// success or `NotFastForward` if the move would lose history. Other
    /// failures (git crash, ref doesn't exist, etc.) return `Err`.
    async fn fast_forward(
        &self,
        workdir: &Path,
        target_ref: &str,
        source_ref: &str,
    ) -> Result<FfOutcome>;
```

- [ ] **Step 5: Implement on `RealGitOps`**

```rust
    async fn fast_forward(
        &self,
        workdir: &Path,
        target_ref: &str,
        source_ref: &str,
    ) -> Result<FfOutcome> {
        // `git merge-base --is-ancestor` returns exit 0 if ancestor, exit 1 if not,
        // exit >1 on a real error (e.g. one ref doesn't exist).
        // If target_ref doesn't exist yet (newly-discovered branch), is-ancestor
        // returns >1 too. We treat that case as "FF allowed" — there's no
        // history to lose. Distinguish that via a separate `git rev-parse`
        // existence probe before running is-ancestor.
        let target_exists = Command::new("git")
            .args(["rev-parse", "--verify", "--quiet", target_ref])
            .current_dir(workdir)
            .output()
            .await
            .context("exec git rev-parse for target_ref")?;

        if target_exists.status.success() {
            // Target exists; require strict FF.
            let ancestor = Command::new("git")
                .args(["merge-base", "--is-ancestor", target_ref, source_ref])
                .current_dir(workdir)
                .output()
                .await
                .context("exec git merge-base --is-ancestor")?;
            // Exit 0 = ancestor (FF OK); exit 1 = not ancestor (NotFF);
            // any other = real error.
            match ancestor.status.code() {
                Some(0) => {}
                Some(1) => return Ok(FfOutcome::NotFastForward),
                other => anyhow::bail!(
                    "git merge-base --is-ancestor exited {:?}: {}",
                    other,
                    String::from_utf8_lossy(&ancestor.stderr)
                ),
            }
        }
        // Either target didn't exist (new branch) or FF was OK — update.
        let update = Command::new("git")
            .args(["update-ref", target_ref, source_ref])
            .current_dir(workdir)
            .output()
            .await
            .context("exec git update-ref")?;
        if !update.status.success() {
            anyhow::bail!(
                "git update-ref failed (exit {}): {}",
                update.status,
                String::from_utf8_lossy(&update.stderr)
            );
        }
        Ok(FfOutcome::Updated)
    }
```

- [ ] **Step 6: Add recording fields + impl to `MockGitOps`**

Update `MockGitOps` struct:

```rust
    struct MockGitOps {
        calls: Mutex<Vec<(PathBuf, PathBuf)>>,
        fetches: Mutex<Vec<(PathBuf, PathBuf, String)>>,
        list_namespace_calls: Mutex<Vec<(PathBuf, String)>>,
        list_namespace_response: Mutex<Vec<String>>,
        fast_forward_calls: Mutex<Vec<(PathBuf, String, String)>>,
        fast_forward_response: Mutex<FfOutcome>,
    }

    impl MockGitOps {
        fn new() -> Self {
            Self {
                calls: Mutex::new(vec![]),
                fetches: Mutex::new(vec![]),
                list_namespace_calls: Mutex::new(vec![]),
                list_namespace_response: Mutex::new(vec![]),
                fast_forward_calls: Mutex::new(vec![]),
                fast_forward_response: Mutex::new(FfOutcome::Updated),
            }
        }

        fn set_list_namespace_refs_response(&self, branches: Vec<String>) {
            *self.list_namespace_response.lock().unwrap() = branches;
        }

        fn set_fast_forward_response(&self, outcome: FfOutcome) {
            *self.fast_forward_response.lock().unwrap() = outcome;
        }
    }
```

Add to `impl GitOps for MockGitOps`:

```rust
        async fn fast_forward(
            &self,
            workdir: &Path,
            target_ref: &str,
            source_ref: &str,
        ) -> Result<FfOutcome> {
            self.fast_forward_calls.lock().unwrap().push((
                workdir.to_path_buf(),
                target_ref.to_string(),
                source_ref.to_string(),
            ));
            Ok(*self.fast_forward_response.lock().unwrap())
        }
```

- [ ] **Step 7: Add impl to `InlineSimulatingOps`**

```rust
        async fn fast_forward(
            &self,
            workdir: &Path,
            target_ref: &str,
            source_ref: &str,
        ) -> Result<FfOutcome> {
            self.inner.fast_forward(workdir, target_ref, source_ref).await
        }
```

- [ ] **Step 8: Add impl to `NoopGit` in handlers tests**

```rust
        async fn fast_forward(
            &self,
            _workdir: &std::path::Path,
            _target_ref: &str,
            _source_ref: &str,
        ) -> Result<crate::clones::FfOutcome> {
            Ok(crate::clones::FfOutcome::Updated)
        }
```

- [ ] **Step 9: Run unit test**

Run: `cargo test -p reeved clones::tests::mock_fast_forward`
Expected: PASS.

- [ ] **Step 10: Write integration tests**

Append in `integration_tests`:

```rust
    #[tokio::test]
    async fn real_fast_forward_advances_target_when_ancestor() {
        let dir = tempdir().unwrap();
        let host = host_repo(dir.path(), "host");

        // Set up a `refs/reeve/card-x/main` one commit ahead of `refs/heads/main`.
        std::fs::write(host.join("a.txt"), "a\n").unwrap();
        git_status(&host, &["add", "a.txt"]);
        git_status(&host, &["-c", "user.email=t@t.com", "-c", "user.name=t", "commit", "-m", "a"]);
        let advance_sha = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&host)
            .output()
            .unwrap();
        let advance_sha = String::from_utf8_lossy(&advance_sha.stdout).trim().to_string();
        git_status(&host, &["update-ref", "refs/reeve/card-x/main", &advance_sha]);
        // Rewind heads/main to the previous commit (HEAD~1).
        git_status(&host, &["update-ref", "refs/heads/main", "HEAD~1"]);

        let ops = RealGitOps;
        let outcome = ops
            .fast_forward(&host, "refs/heads/main", "refs/reeve/card-x/main")
            .await
            .unwrap();
        assert_eq!(outcome, FfOutcome::Updated);

        let after = std::process::Command::new("git")
            .args(["rev-parse", "refs/heads/main"])
            .current_dir(&host)
            .output()
            .unwrap();
        let after_sha = String::from_utf8_lossy(&after.stdout).trim().to_string();
        assert_eq!(after_sha, advance_sha, "main advanced to the namespaced ref");
    }

    #[tokio::test]
    async fn real_fast_forward_rejects_non_ff() {
        let dir = tempdir().unwrap();
        let host = host_repo(dir.path(), "host");

        // Create divergent histories: heads/main has one commit, refs/reeve/card-x/main
        // points at a DIFFERENT commit on a parallel branch with no shared base beyond
        // the initial commit.
        git_status(&host, &["checkout", "-b", "parallel"]);
        std::fs::write(host.join("p.txt"), "p\n").unwrap();
        git_status(&host, &["add", "p.txt"]);
        git_status(&host, &["-c", "user.email=t@t.com", "-c", "user.name=t", "commit", "-m", "parallel"]);
        let parallel_sha = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&host)
            .output()
            .unwrap();
        let parallel_sha = String::from_utf8_lossy(&parallel_sha.stdout).trim().to_string();
        git_status(&host, &["checkout", "main"]);
        std::fs::write(host.join("m.txt"), "m\n").unwrap();
        git_status(&host, &["add", "m.txt"]);
        git_status(&host, &["-c", "user.email=t@t.com", "-c", "user.name=t", "commit", "-m", "main2"]);
        git_status(&host, &["update-ref", "refs/reeve/card-x/main", &parallel_sha]);

        let ops = RealGitOps;
        let outcome = ops
            .fast_forward(&host, "refs/heads/main", "refs/reeve/card-x/main")
            .await
            .unwrap();
        assert_eq!(outcome, FfOutcome::NotFastForward);
    }

    #[tokio::test]
    async fn real_fast_forward_creates_new_branch_target() {
        let dir = tempdir().unwrap();
        let host = host_repo(dir.path(), "host");

        // Add a namespaced ref for a branch that doesn't yet exist under refs/heads.
        let head_sha = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&host)
            .output()
            .unwrap();
        let head_sha = String::from_utf8_lossy(&head_sha.stdout).trim().to_string();
        git_status(&host, &["update-ref", "refs/reeve/card-x/feat-new", &head_sha]);

        let ops = RealGitOps;
        let outcome = ops
            .fast_forward(&host, "refs/heads/feat-new", "refs/reeve/card-x/feat-new")
            .await
            .unwrap();
        assert_eq!(outcome, FfOutcome::Updated);

        let result = std::process::Command::new("git")
            .args(["rev-parse", "refs/heads/feat-new"])
            .current_dir(&host)
            .output()
            .unwrap();
        assert!(result.status.success(), "new branch ref was created");
    }
```

- [ ] **Step 11: Run integration tests**

Run: `cargo test -p reeved clones::integration_tests::real_fast_forward`
Expected: all 3 tests PASS.

- [ ] **Step 12: Commit**

```bash
git add crates/reeved/src/clones.rs crates/reeved/src/handlers.rs
git commit -m "feat(m4): GitOps::fast_forward + FfOutcome for safe ref advancement"
```

---

## Task 5: `GitOps::push`

**Files:**
- Modify: `crates/reeved/src/clones.rs`
- Modify: `crates/reeved/src/handlers.rs` (NoopGit)

- [ ] **Step 1: Write the failing unit test**

```rust
    #[tokio::test]
    async fn mock_push_records_call() {
        let dir = tempdir().unwrap();
        let workdir = dir.path().join("host");
        let ops = MockGitOps::new();
        ops.push(&workdir, "feat-x").await.unwrap();
        let calls = ops.push_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], (workdir.clone(), "feat-x".to_string()));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p reeved clones::tests::mock_push_records_call`
Expected: FAIL.

- [ ] **Step 3: Add `push` to the `GitOps` trait**

```rust
    /// Run `git push origin <branch>` in `workdir`. The remote `origin`
    /// must be configured on the host repo; absence is reported as an
    /// `Err` returned from this method.
    async fn push(&self, workdir: &Path, branch: &str) -> Result<()>;
```

- [ ] **Step 4: Implement on `RealGitOps`**

```rust
    async fn push(&self, workdir: &Path, branch: &str) -> Result<()> {
        let output = Command::new("git")
            .args(["push", "origin", branch])
            .current_dir(workdir)
            .output()
            .await
            .context("exec git push")?;
        if !output.status.success() {
            anyhow::bail!(
                "git push failed (exit {}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }
```

- [ ] **Step 5: Add recording field + impl to `MockGitOps`**

Update `MockGitOps`:

```rust
    struct MockGitOps {
        calls: Mutex<Vec<(PathBuf, PathBuf)>>,
        fetches: Mutex<Vec<(PathBuf, PathBuf, String)>>,
        list_namespace_calls: Mutex<Vec<(PathBuf, String)>>,
        list_namespace_response: Mutex<Vec<String>>,
        fast_forward_calls: Mutex<Vec<(PathBuf, String, String)>>,
        fast_forward_response: Mutex<FfOutcome>,
        push_calls: Mutex<Vec<(PathBuf, String)>>,
        push_response: Mutex<std::result::Result<(), String>>,
    }
```

Update `new()`:

```rust
        fn new() -> Self {
            Self {
                calls: Mutex::new(vec![]),
                fetches: Mutex::new(vec![]),
                list_namespace_calls: Mutex::new(vec![]),
                list_namespace_response: Mutex::new(vec![]),
                fast_forward_calls: Mutex::new(vec![]),
                fast_forward_response: Mutex::new(FfOutcome::Updated),
                push_calls: Mutex::new(vec![]),
                push_response: Mutex::new(Ok(())),
            }
        }
```

Add helper for configuring push failure:

```rust
        fn set_push_failure(&self, message: &str) {
            *self.push_response.lock().unwrap() = Err(message.to_string());
        }
```

Add to `impl GitOps for MockGitOps`:

```rust
        async fn push(&self, workdir: &Path, branch: &str) -> Result<()> {
            self.push_calls
                .lock()
                .unwrap()
                .push((workdir.to_path_buf(), branch.to_string()));
            match self.push_response.lock().unwrap().clone() {
                Ok(()) => Ok(()),
                Err(msg) => anyhow::bail!("{msg}"),
            }
        }
```

- [ ] **Step 6: Add to `InlineSimulatingOps`**

```rust
        async fn push(&self, workdir: &Path, branch: &str) -> Result<()> {
            self.inner.push(workdir, branch).await
        }
```

- [ ] **Step 7: Add to `NoopGit` in handlers tests**

```rust
        async fn push(&self, _workdir: &std::path::Path, _branch: &str) -> Result<()> {
            Ok(())
        }
```

- [ ] **Step 8: Run unit test**

Run: `cargo test -p reeved clones::tests::mock_push_records_call`
Expected: PASS.

- [ ] **Step 9: Write integration test against a bare remote**

```rust
    #[tokio::test]
    async fn real_push_to_bare_remote_succeeds() {
        let dir = tempdir().unwrap();
        let host = host_repo(dir.path(), "host");

        // Create a bare remote and wire it as `origin`.
        let bare = dir.path().join("origin.git");
        let status = std::process::Command::new("git")
            .args(["init", "--bare", "--quiet"])
            .arg(&bare)
            .status()
            .unwrap();
        assert!(status.success(), "git init --bare failed");

        let bare_url = format!("file://{}", bare.display());
        git_status(&host, &["remote", "add", "origin", &bare_url]);

        let ops = RealGitOps;
        ops.push(&host, "main").await.unwrap();

        // Verify the bare repo received the branch.
        let result = std::process::Command::new("git")
            .args(["rev-parse", "main"])
            .current_dir(&bare)
            .output()
            .unwrap();
        assert!(result.status.success(), "bare repo has main: {}", String::from_utf8_lossy(&result.stderr));
    }
```

- [ ] **Step 10: Run integration test**

Run: `cargo test -p reeved clones::integration_tests::real_push_to_bare_remote_succeeds`
Expected: PASS.

- [ ] **Step 11: Commit**

```bash
git add crates/reeved/src/clones.rs crates/reeved/src/handlers.rs
git commit -m "feat(m4): GitOps::push — git push origin <branch>"
```

---

## Task 6: `iter_card_repos` helper

**Files:**
- Modify: `crates/reeved/src/household.rs`

- [ ] **Step 1: Write the failing unit test**

Append in `crates/reeved/src/household.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Manifest, RepoEntry};
    use std::path::PathBuf;

    fn manifest(workspace: &str, repos: &[&str]) -> Manifest {
        Manifest {
            workspace: workspace.to_string(),
            knowledge_base: None,
            repos: repos
                .iter()
                .map(|name| RepoEntry {
                    name: (*name).to_string(),
                    url: None,
                    tags: vec![],
                    description: None,
                })
                .collect(),
        }
    }

    #[test]
    fn iter_card_repos_yields_workspace_first_then_siblings_in_order() {
        let h = Household {
            name: "test".into(),
            meta_repo_path: PathBuf::from("/tmp/meta"),
        };
        let m = manifest("ws", &["ws", "backend", "lore"]);
        let yielded: Vec<(String, PathBuf)> = iter_card_repos(&h, &m)
            .map(|(n, p)| (n.to_string(), p))
            .collect();
        assert_eq!(
            yielded,
            vec![
                ("ws".to_string(), PathBuf::from("/tmp/meta")),
                ("backend".to_string(), PathBuf::from("/tmp/meta/backend")),
                ("lore".to_string(), PathBuf::from("/tmp/meta/lore")),
            ]
        );
    }

    #[test]
    fn iter_card_repos_skips_workspace_entry_in_siblings_section() {
        // Manifest where the workspace entry isn't first in repos[]; ensure
        // the helper still emits it first and doesn't double-emit.
        let h = Household {
            name: "test".into(),
            meta_repo_path: PathBuf::from("/tmp/meta"),
        };
        let m = manifest("ws", &["backend", "ws", "lore"]);
        let yielded: Vec<String> = iter_card_repos(&h, &m).map(|(n, _)| n.to_string()).collect();
        assert_eq!(yielded, vec!["ws", "backend", "lore"]);
    }

    #[test]
    fn iter_card_repos_with_only_workspace_yields_one_entry() {
        let h = Household {
            name: "test".into(),
            meta_repo_path: PathBuf::from("/tmp/meta"),
        };
        let m = manifest("ws", &["ws"]);
        let yielded: Vec<String> = iter_card_repos(&h, &m).map(|(n, _)| n.to_string()).collect();
        assert_eq!(yielded, vec!["ws"]);
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p reeved household::tests::iter_card_repos`
Expected: FAIL with `cannot find value 'iter_card_repos'`.

- [ ] **Step 3: Implement `iter_card_repos`**

Append in `crates/reeved/src/household.rs`, before the `#[cfg(test)] mod tests` block:

```rust
/// Yields (repo_name, host_path) for every repo participating in a card.
/// Used by both the fetch worker and the push handler.
///
/// Iteration order is part of the contract (pinned for testability):
/// 1. The workspace entry — `manifest.workspace` — first, with host_path
///    = `household.meta_repo_path` itself.
/// 2. Then every entry in `manifest.repos` in declaration order, skipping
///    the one whose `name == manifest.workspace` (already yielded as #1).
///    For each, host_path = `household.meta_repo_path.join(entry.name)`.
pub fn iter_card_repos<'a>(
    household: &'a Household,
    manifest: &'a Manifest,
) -> impl Iterator<Item = (&'a str, PathBuf)> + 'a {
    let workspace_name = manifest.workspace.as_str();
    let meta = household.meta_repo_path.clone();
    let workspace_entry = std::iter::once((workspace_name, meta.clone()));
    let siblings = manifest
        .repos
        .iter()
        .filter(move |r| r.name != workspace_name)
        .map(move |r| (r.name.as_str(), meta.join(&r.name)));
    workspace_entry.chain(siblings)
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p reeved household::tests::iter_card_repos`
Expected: all 3 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/reeved/src/household.rs
git commit -m "feat(m4): iter_card_repos — workspace-first, manifest-declaration-order iteration"
```

---

## Task 7: `handle_push` + dispatcher case

**Files:**
- Modify: `crates/reeved/src/handlers.rs`

- [ ] **Step 1: Write the failing unit test — empty branch list**

Append to `handlers.rs`'s test module (after the existing `household_*` tests):

```rust
    use reeve_protocol::{BranchPushStatus, PushParams, PushResult};

    fn insert_test_household(state: &DaemonState, name: &str, meta_repo_path: &std::path::Path) {
        let conn = state.db.blocking_lock();
        crate::db::insert_household(
            &conn,
            &crate::db::HouseholdRow {
                name: name.to_string(),
                meta_repo_path: meta_repo_path.to_path_buf(),
                added_at: 0,
            },
        )
        .unwrap();
    }

    /// Write a minimal household.json at `meta_repo_path`.
    fn write_test_manifest(meta_repo_path: &std::path::Path, workspace: &str, repos: &[&str]) {
        let repos_json: Vec<String> = repos
            .iter()
            .map(|n| format!(r#"{{"name":"{n}"}}"#))
            .collect();
        let body = format!(
            r#"{{"workspace":"{workspace}","repos":[{}]}}"#,
            repos_json.join(",")
        );
        std::fs::write(meta_repo_path.join("household.json"), body).unwrap();
    }

    #[tokio::test]
    async fn push_with_no_branches_returns_empty_result() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join("ws");
        std::fs::create_dir_all(&meta).unwrap();
        write_test_manifest(&meta, "ws", &["ws"]);

        let state = fresh_state();
        insert_test_household(&state, "h", &meta);

        // Insert a card pointing at the household.
        let conn = state.db.blocking_lock();
        crate::db::insert_card(
            &conn,
            &reeve_protocol::Card {
                id: "card-x".into(),
                household: "h".into(),
                brief: "x".into(),
                column: reeve_protocol::Column::Implementing,
                created_at: 0,
                updated_at: 0,
                needs_heavy: false,
                parent_id: None,
            },
        )
        .unwrap();
        drop(conn);

        let req = RpcRequest {
            id: 1,
            method: "push".into(),
            params: serde_json::json!({ "id": "card-x" }),
        };
        let value = dispatch(req, state, CallerContext::host()).await.unwrap();
        let result: PushResult = serde_json::from_value(value).unwrap();
        assert!(result.branches.is_empty());
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p reeved handlers::tests::push_with_no_branches_returns_empty_result`
Expected: FAIL — dispatcher doesn't yet handle `"push"`.

- [ ] **Step 3: Add `handle_push`**

Edit `crates/reeved/src/handlers.rs`. Add to the imports at the top:

```rust
use reeve_protocol::{
    BranchPushResult, BranchPushStatus, Card, Column, ContainerStatus, HouseholdAddParams,
    HouseholdDetail, HouseholdNewParams, HouseholdSource, HouseholdSummary, LsParams, LsResult,
    MvParams, MvResult, MvSelfParams, MvSelfResult, NewParams, NewResult, PushParams, PushResult,
    RpcError, RpcRequest, StatusSelfParams, StatusSelfResult,
};
```

(Adapt to whatever the existing import shape is; add `BranchPushResult`, `BranchPushStatus`, `PushParams`, `PushResult` alphabetically.)

Then append (alongside the other handlers, e.g. after `handle_household_show`):

```rust
async fn handle_push(state: &DaemonState, params: PushParams) -> Result<Value, RpcError> {
    // 1. Fetch the card from the DB (column doesn't matter; we'll push whatever
    //    refs are in the namespace).
    let card_id = params.id.clone();
    let db = state.db.clone();
    let card = tokio::task::spawn_blocking(move || -> Result<Option<Card>> {
        let conn = db.blocking_lock();
        crate::db::get_card(&conn, &card_id)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?
    .ok_or_else(|| RpcError {
        code: RpcError::NOT_FOUND,
        message: format!("card '{}' not found", params.id),
    })?;

    // 2. Resolve the household from sqlite (NOT from a cached in-memory map).
    let household_name = card.household.clone();
    let db = state.db.clone();
    let row = tokio::task::spawn_blocking(move || -> Result<Option<crate::db::HouseholdRow>> {
        let conn = db.blocking_lock();
        crate::db::get_household(&conn, &household_name)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?
    .ok_or_else(|| RpcError {
        code: RpcError::INTERNAL,
        message: format!("household '{}' not configured", card.household),
    })?;
    let household = crate::household::Household {
        name: row.name,
        meta_repo_path: row.meta_repo_path,
    };

    // 3. Load + validate the manifest.
    let manifest = crate::household::load_household_manifest(&household).map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("load manifest: {e:#}"),
    })?;

    // 4. For each repo, for each branch in the namespace, FF + push.
    let mut branches = Vec::new();
    for (repo_name, host_path) in crate::household::iter_card_repos(&household, &manifest) {
        let listed = state
            .git
            .list_namespace_refs(&host_path, &card.id)
            .await
            .map_err(|e| RpcError {
                code: RpcError::INTERNAL,
                message: format!("list refs/reeve/{}/* in {}: {e:#}", card.id, host_path.display()),
            })?;
        for branch in listed {
            let target = format!("refs/heads/{branch}");
            let source = format!("refs/reeve/{}/{}", card.id, branch);
            let status = match state.git.fast_forward(&host_path, &target, &source).await {
                Ok(crate::clones::FfOutcome::Updated) => match state.git.push(&host_path, &branch).await {
                    Ok(()) => BranchPushStatus::Pushed,
                    Err(e) => BranchPushStatus::PushFailed {
                        message: format!("{e:#}"),
                    },
                },
                Ok(crate::clones::FfOutcome::NotFastForward) => BranchPushStatus::NotFastForward,
                Err(e) => BranchPushStatus::PushFailed {
                    message: format!("fast-forward check failed: {e:#}"),
                },
            };
            branches.push(BranchPushResult {
                repo: repo_name.to_string(),
                branch,
                status,
            });
        }
    }
    serde_json::to_value(PushResult { branches }).map_err(to_internal)
}
```

- [ ] **Step 4: Add dispatcher case for `"push"`**

In the same file, the `dispatch` function's `match req.method.as_str()` block. Add (alphabetically, before the `"household.*"` cases, OR after `"status-self"` — match existing convention):

```rust
        "push" => {
            let params = parse_params::<PushParams>(req.params)?;
            handle_push(&state, params).await
        }
```

- [ ] **Step 5: Run the empty-branches test**

Run: `cargo test -p reeved handlers::tests::push_with_no_branches_returns_empty_result`
Expected: PASS.

- [ ] **Step 6: Add the per-branch-status test**

Append:

```rust
    /// Test mock that returns configured branches from list_namespace_refs and
    /// configured FfOutcome per call. Used to exercise handle_push's logic
    /// without a real git.
    struct ConfigurableGit {
        branches_per_repo: std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, Vec<String>>>,
        ff_outcomes: std::sync::Mutex<Vec<crate::clones::FfOutcome>>,
        push_failures: std::sync::Mutex<std::collections::HashMap<String, String>>,
        push_call_order: std::sync::Mutex<Vec<(std::path::PathBuf, String)>>,
    }

    impl ConfigurableGit {
        fn new() -> Self {
            Self {
                branches_per_repo: std::sync::Mutex::new(std::collections::HashMap::new()),
                ff_outcomes: std::sync::Mutex::new(vec![]),
                push_failures: std::sync::Mutex::new(std::collections::HashMap::new()),
                push_call_order: std::sync::Mutex::new(vec![]),
            }
        }
        fn set_branches(&self, repo: std::path::PathBuf, branches: Vec<String>) {
            self.branches_per_repo.lock().unwrap().insert(repo, branches);
        }
        fn queue_ff_outcome(&self, outcome: crate::clones::FfOutcome) {
            self.ff_outcomes.lock().unwrap().push(outcome);
        }
        fn fail_push(&self, branch: &str, message: &str) {
            self.push_failures
                .lock()
                .unwrap()
                .insert(branch.to_string(), message.to_string());
        }
    }

    #[async_trait::async_trait]
    impl crate::clones::GitOps for ConfigurableGit {
        async fn clone(&self, _s: &std::path::Path, _d: &std::path::Path) -> Result<()> { Ok(()) }
        async fn clone_url(&self, _u: &str, _d: &std::path::Path) -> Result<()> { Ok(()) }
        async fn fetch(&self, _w: &std::path::Path, _s: &std::path::Path, _n: &str) -> Result<()> { Ok(()) }
        async fn list_namespace_refs(
            &self,
            workdir: &std::path::Path,
            _namespace: &str,
        ) -> Result<Vec<String>> {
            Ok(self
                .branches_per_repo
                .lock()
                .unwrap()
                .get(workdir)
                .cloned()
                .unwrap_or_default())
        }
        async fn fast_forward(
            &self,
            _workdir: &std::path::Path,
            _target: &str,
            _source: &str,
        ) -> Result<crate::clones::FfOutcome> {
            // Pop in order; default to Updated if queue empty.
            let mut q = self.ff_outcomes.lock().unwrap();
            Ok(if q.is_empty() { crate::clones::FfOutcome::Updated } else { q.remove(0) })
        }
        async fn push(&self, workdir: &std::path::Path, branch: &str) -> Result<()> {
            self.push_call_order
                .lock()
                .unwrap()
                .push((workdir.to_path_buf(), branch.to_string()));
            if let Some(msg) = self.push_failures.lock().unwrap().get(branch) {
                anyhow::bail!("{msg}");
            }
            Ok(())
        }
    }

    fn fresh_state_with_configurable_git(git: std::sync::Arc<ConfigurableGit>) -> DaemonState {
        let conn = crate::db::open(":memory:").unwrap();
        let db: crate::server::DbHandle = std::sync::Arc::new(tokio::sync::Mutex::new(conn));
        DaemonState {
            db: db.clone(),
            containers: std::sync::Arc::new(NoopOps) as crate::container::ContainerOpsHandle,
            git: git as crate::clones::GitOpsHandle,
            listeners: crate::listeners::ListenerRegistry::new(db, std::sync::Arc::new(NoopDispatcher)),
            claude_dir: std::path::PathBuf::from("/tmp/nonexistent-test-claude-dir"),
            api_key: None,
        }
    }

    #[tokio::test]
    async fn push_records_per_branch_status_pushed_and_not_ff() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join("ws");
        std::fs::create_dir_all(&meta).unwrap();
        write_test_manifest(&meta, "ws", &["ws"]);

        let git = std::sync::Arc::new(ConfigurableGit::new());
        git.set_branches(meta.clone(), vec!["feat-a".into(), "feat-b".into()]);
        // First branch: Updated then push OK; Second: NotFastForward.
        git.queue_ff_outcome(crate::clones::FfOutcome::Updated);
        git.queue_ff_outcome(crate::clones::FfOutcome::NotFastForward);

        let state = fresh_state_with_configurable_git(git);
        insert_test_household(&state, "h", &meta);
        {
            let conn = state.db.blocking_lock();
            crate::db::insert_card(
                &conn,
                &Card {
                    id: "card-y".into(),
                    household: "h".into(),
                    brief: "y".into(),
                    column: Column::Implementing,
                    created_at: 0,
                    updated_at: 0,
                    needs_heavy: false,
                    parent_id: None,
                },
            )
            .unwrap();
        }

        let req = RpcRequest {
            id: 1,
            method: "push".into(),
            params: serde_json::json!({ "id": "card-y" }),
        };
        let value = dispatch(req, state, CallerContext::host()).await.unwrap();
        let result: PushResult = serde_json::from_value(value).unwrap();
        assert_eq!(result.branches.len(), 2);
        assert!(matches!(result.branches[0].status, BranchPushStatus::Pushed));
        assert!(matches!(result.branches[1].status, BranchPushStatus::NotFastForward));
        assert_eq!(result.branches[0].branch, "feat-a");
        assert_eq!(result.branches[1].branch, "feat-b");
    }
```

- [ ] **Step 7: Run the per-branch-status test**

Run: `cargo test -p reeved handlers::tests::push_records_per_branch_status`
Expected: PASS.

- [ ] **Step 8: Add the iteration-order test**

```rust
    #[tokio::test]
    async fn push_iterates_meta_repo_then_manifest_repos_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join("ws");
        std::fs::create_dir_all(&meta).unwrap();
        write_test_manifest(&meta, "ws", &["ws", "backend", "lore"]);

        let git = std::sync::Arc::new(ConfigurableGit::new());
        // Each repo path has one branch; assert push_call_order matches.
        git.set_branches(meta.clone(), vec!["feat-meta".into()]);
        git.set_branches(meta.join("backend"), vec!["feat-back".into()]);
        git.set_branches(meta.join("lore"), vec!["feat-lore".into()]);

        let state = fresh_state_with_configurable_git(git.clone());
        insert_test_household(&state, "h", &meta);
        {
            let conn = state.db.blocking_lock();
            crate::db::insert_card(
                &conn,
                &Card {
                    id: "card-z".into(),
                    household: "h".into(),
                    brief: "z".into(),
                    column: Column::Implementing,
                    created_at: 0,
                    updated_at: 0,
                    needs_heavy: false,
                    parent_id: None,
                },
            )
            .unwrap();
        }

        let req = RpcRequest {
            id: 1,
            method: "push".into(),
            params: serde_json::json!({ "id": "card-z" }),
        };
        let value = dispatch(req, state, CallerContext::host()).await.unwrap();
        let result: PushResult = serde_json::from_value(value).unwrap();
        let repos: Vec<&str> = result.branches.iter().map(|b| b.repo.as_str()).collect();
        assert_eq!(repos, vec!["ws", "backend", "lore"]);
        let call_order = git.push_call_order.lock().unwrap();
        let paths: Vec<&std::path::PathBuf> = call_order.iter().map(|(p, _)| p).collect();
        assert_eq!(
            paths,
            vec![&meta, &meta.join("backend"), &meta.join("lore")]
        );
    }
```

- [ ] **Step 9: Run the order test**

Run: `cargo test -p reeved handlers::tests::push_iterates_meta_repo_then_manifest_repos_in_order`
Expected: PASS.

- [ ] **Step 10: Run the whole reeved test suite to confirm no regressions**

Run: `cargo test -p reeved`
Expected: PASS. Test count: was 110 pre-M4 (Tasks 1-7 incremental count); new ≈ +3 unit tests for handle_push + the 3 iter_card_repos tests + the 4 new clones unit tests (mock_fetch, mock_list_namespace_refs, mock_fast_forward, mock_push) + 5 integration tests (1 fetch, 2 list_namespace, 3 fast_forward — wait, 3 ff tests, 1 list_namespace pair = 2 list_namespace, 1 push integration). Final reeved count should be around 110 + 3 + 3 + 4 + 1 + 2 + 3 + 1 = 127 tests. Exact number depends on existing pre-M4 baseline.

- [ ] **Step 11: Commit**

```bash
git add crates/reeved/src/handlers.rs
git commit -m "feat(m4): handle_push — iterate meta-repo + manifest repos, FF + push per branch"
```

---

## Task 8: `fetch::tick` — single-iteration fetch function

**Files:**
- Create: `crates/reeved/src/fetch.rs`
- Modify: `crates/reeved/src/main.rs` (add `mod fetch;`)

- [ ] **Step 1: Create the fetch module with a stub**

Create `crates/reeved/src/fetch.rs`:

```rust
//! M4 fetch worker. Periodically fetches each running card's clones into
//! `refs/reeve/<card-id>/*` on the host repos, surfacing in-container
//! commits in the user's git tooling without modifying `refs/heads/*`.

use crate::effects::DaemonState;
use anyhow::Result;
use reeve_protocol::ContainerStatus;
use std::time::Duration;
use tracing::warn;

/// Hardcoded for v1; configurable later.
pub const FETCH_INTERVAL: Duration = Duration::from_secs(5);

/// Run one fetch iteration: for each `Running` or `Spawning` card, walk
/// its meta-repo + manifest repos and fetch from the card workspace into
/// `refs/reeve/<card-id>/*` on the host repo. Per-fetch failures log at
/// `warn` and continue; this function never returns `Err` from a fetch
/// failure (the DB iteration may still error).
pub async fn tick(state: &DaemonState) -> Result<()> {
    let containers = {
        let db = state.db.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<crate::db::ContainerRow>> {
            let conn = db.blocking_lock();
            crate::db::list_containers(&conn)
        })
        .await??
    };

    for container in containers {
        if !matches!(container.status, ContainerStatus::Running | ContainerStatus::Spawning) {
            continue;
        }
        if let Err(e) = tick_one_card(state, &container.card_id).await {
            warn!("fetch tick for card {}: {e:#}", container.card_id);
        }
    }
    Ok(())
}

/// Fetch for a single card. Logs per-fetch errors at warn; returns Err only
/// for unrecoverable per-card problems (e.g., card row missing entirely).
async fn tick_one_card(state: &DaemonState, card_id: &str) -> Result<()> {
    // Look up the card to find its household.
    let card_id_for_query = card_id.to_string();
    let db = state.db.clone();
    let card = tokio::task::spawn_blocking(move || -> Result<Option<reeve_protocol::Card>> {
        let conn = db.blocking_lock();
        crate::db::get_card(&conn, &card_id_for_query)
    })
    .await??;
    let Some(card) = card else {
        warn!("fetch tick: card {} not found in DB", card_id);
        return Ok(());
    };

    // Look up the household.
    let household_name = card.household.clone();
    let db = state.db.clone();
    let row = tokio::task::spawn_blocking(move || -> Result<Option<crate::db::HouseholdRow>> {
        let conn = db.blocking_lock();
        crate::db::get_household(&conn, &household_name)
    })
    .await??;
    let Some(row) = row else {
        warn!("fetch tick: household '{}' for card {} not configured", card.household, card_id);
        return Ok(());
    };
    let household = crate::household::Household {
        name: row.name,
        meta_repo_path: row.meta_repo_path,
    };

    // Load the manifest. Failure to parse skips this card until the user fixes
    // household.json; next tick re-tries.
    let manifest = match crate::household::load_household_manifest(&household) {
        Ok(m) => m,
        Err(e) => {
            warn!("fetch tick: manifest for card {}: {e:#}", card_id);
            return Ok(());
        }
    };

    let workspace = reeve_protocol::paths::card_workspace_path(card_id);
    for (repo_name, host_path) in crate::household::iter_card_repos(&household, &manifest) {
        // The card-side source path mirrors the host topology:
        //   - workspace repo: <workspace>/
        //   - sibling repo:   <workspace>/<name>/
        let card_clone = if repo_name == manifest.workspace {
            workspace.clone()
        } else {
            workspace.join(repo_name)
        };
        if let Err(e) = state.git.fetch(&host_path, &card_clone, card_id).await {
            warn!("fetch {}/{}: {e:#}", card_id, repo_name);
        }
    }
    Ok(())
}
```

- [ ] **Step 2: Add `mod fetch;` to main.rs**

Edit `crates/reeved/src/main.rs`. Find the `mod` declarations near the top (alphabetical with the others — `effects`, `manifest`, `server`, etc.). Add `mod fetch;` in alphabetical order:

```rust
mod effects;
mod fetch;
mod handlers;
```

(Adapt to the actual existing order; insert alphabetically.)

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p reeved`
Expected: PASS. The `tick` function compiles; not yet wired into any caller.

- [ ] **Step 4: Write the unit test for `tick`**

Append at the bottom of `crates/reeved/src/fetch.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::clones::GitOps;
    use std::sync::Arc;

    /// Mock that records every fetch call.
    struct RecordingGit {
        calls: std::sync::Mutex<Vec<(std::path::PathBuf, std::path::PathBuf, String)>>,
    }

    impl RecordingGit {
        fn new() -> Self {
            Self { calls: std::sync::Mutex::new(vec![]) }
        }
    }

    #[async_trait::async_trait]
    impl GitOps for RecordingGit {
        async fn clone(&self, _s: &std::path::Path, _d: &std::path::Path) -> Result<()> { Ok(()) }
        async fn clone_url(&self, _u: &str, _d: &std::path::Path) -> Result<()> { Ok(()) }
        async fn fetch(
            &self,
            workdir: &std::path::Path,
            source: &std::path::Path,
            namespace: &str,
        ) -> Result<()> {
            self.calls.lock().unwrap().push((
                workdir.to_path_buf(),
                source.to_path_buf(),
                namespace.to_string(),
            ));
            Ok(())
        }
        async fn list_namespace_refs(
            &self,
            _workdir: &std::path::Path,
            _namespace: &str,
        ) -> Result<Vec<String>> {
            Ok(vec![])
        }
        async fn fast_forward(
            &self,
            _workdir: &std::path::Path,
            _target: &str,
            _source: &str,
        ) -> Result<crate::clones::FfOutcome> {
            Ok(crate::clones::FfOutcome::Updated)
        }
        async fn push(&self, _workdir: &std::path::Path, _branch: &str) -> Result<()> { Ok(()) }
    }

    struct NoopOps;
    #[async_trait::async_trait]
    impl crate::container::ContainerOps for NoopOps {
        async fn spawn(&self, _argv: Vec<String>) -> Result<()> { Ok(()) }
        async fn teardown(&self, _name: &str) -> Result<()> { Ok(()) }
        async fn inspect_status(&self, _name: &str) -> Result<Option<String>> { Ok(None) }
        async fn discover_container_name(&self, _card_id: &str) -> Result<Option<String>> { Ok(None) }
    }

    struct NoopDispatcher;
    impl crate::listeners::Dispatcher for NoopDispatcher {
        fn dispatch(
            &self,
            _req: reeve_protocol::RpcRequest,
            _db: crate::server::DbHandle,
            _caller: crate::server::CallerContext,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = std::result::Result<serde_json::Value, reeve_protocol::RpcError>> + Send>> {
            Box::pin(async { Ok(serde_json::Value::Null) })
        }
    }

    fn write_manifest(meta: &std::path::Path, workspace: &str, repos: &[&str]) {
        std::fs::create_dir_all(meta).unwrap();
        let repos_json: Vec<String> = repos.iter().map(|n| format!(r#"{{"name":"{n}"}}"#)).collect();
        let body = format!(r#"{{"workspace":"{workspace}","repos":[{}]}}"#, repos_json.join(","));
        std::fs::write(meta.join("household.json"), body).unwrap();
    }

    fn state_with(git: Arc<RecordingGit>) -> DaemonState {
        let conn = crate::db::open(":memory:").unwrap();
        let db: crate::server::DbHandle = Arc::new(tokio::sync::Mutex::new(conn));
        DaemonState {
            db: db.clone(),
            containers: Arc::new(NoopOps) as crate::container::ContainerOpsHandle,
            git: git as crate::clones::GitOpsHandle,
            listeners: crate::listeners::ListenerRegistry::new(db, Arc::new(NoopDispatcher)),
            claude_dir: std::path::PathBuf::from("/tmp/nonexistent-test-claude-dir"),
            api_key: None,
        }
    }

    #[tokio::test]
    async fn tick_fetches_once_per_running_card_and_per_repo() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join("ws");
        write_manifest(&meta, "ws", &["ws", "backend"]);

        let git = Arc::new(RecordingGit::new());
        let state = state_with(git.clone());

        {
            let conn = state.db.blocking_lock();
            crate::db::insert_household(
                &conn,
                &crate::db::HouseholdRow {
                    name: "h".into(),
                    meta_repo_path: meta.clone(),
                    added_at: 0,
                },
            )
            .unwrap();
            crate::db::insert_card(
                &conn,
                &reeve_protocol::Card {
                    id: "card-a".into(),
                    household: "h".into(),
                    brief: "a".into(),
                    column: reeve_protocol::Column::Implementing,
                    created_at: 0,
                    updated_at: 0,
                    needs_heavy: false,
                    parent_id: None,
                },
            )
            .unwrap();
            crate::db::upsert_container(
                &conn,
                "card-a",
                "container-a",
                ContainerStatus::Running,
                0,
            )
            .unwrap();
        }

        tick(&state).await.unwrap();

        let calls = git.calls.lock().unwrap();
        assert_eq!(calls.len(), 2, "one fetch per (running card × repo): {calls:?}");
        // Workspace first, then sibling.
        assert_eq!(calls[0].2, "card-a");
        assert_eq!(calls[0].0, meta);
        assert_eq!(calls[1].0, meta.join("backend"));
    }

    #[tokio::test]
    async fn tick_skips_non_running_containers() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join("ws");
        write_manifest(&meta, "ws", &["ws"]);

        let git = Arc::new(RecordingGit::new());
        let state = state_with(git.clone());

        {
            let conn = state.db.blocking_lock();
            crate::db::insert_household(
                &conn,
                &crate::db::HouseholdRow {
                    name: "h".into(),
                    meta_repo_path: meta.clone(),
                    added_at: 0,
                },
            )
            .unwrap();
            crate::db::insert_card(
                &conn,
                &reeve_protocol::Card {
                    id: "card-b".into(),
                    household: "h".into(),
                    brief: "b".into(),
                    column: reeve_protocol::Column::Implementing,
                    created_at: 0,
                    updated_at: 0,
                    needs_heavy: false,
                    parent_id: None,
                },
            )
            .unwrap();
            crate::db::upsert_container(
                &conn,
                "card-b",
                "container-b",
                ContainerStatus::Failed,
                0,
            )
            .unwrap();
        }

        tick(&state).await.unwrap();

        let calls = git.calls.lock().unwrap();
        assert!(calls.is_empty(), "Failed containers should be skipped");
    }

    #[tokio::test]
    async fn tick_continues_after_missing_household() {
        let dir = tempfile::tempdir().unwrap();
        let meta_b = dir.path().join("ws-b");
        write_manifest(&meta_b, "ws-b", &["ws-b"]);

        let git = Arc::new(RecordingGit::new());
        let state = state_with(git.clone());

        {
            let conn = state.db.blocking_lock();
            // Household "b" exists; "a" does not.
            crate::db::insert_household(
                &conn,
                &crate::db::HouseholdRow {
                    name: "b".into(),
                    meta_repo_path: meta_b.clone(),
                    added_at: 0,
                },
            )
            .unwrap();
            for (id, hh) in [("card-a", "a-missing"), ("card-b", "b")] {
                crate::db::insert_card(
                    &conn,
                    &reeve_protocol::Card {
                        id: id.into(),
                        household: hh.into(),
                        brief: id.into(),
                        column: reeve_protocol::Column::Implementing,
                        created_at: 0,
                        updated_at: 0,
                        needs_heavy: false,
                        parent_id: None,
                    },
                )
                .unwrap();
                crate::db::upsert_container(&conn, id, &format!("c-{id}"), ContainerStatus::Running, 0)
                    .unwrap();
            }
        }

        tick(&state).await.unwrap();

        let calls = git.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "only card-b should produce a fetch");
        assert_eq!(calls[0].2, "card-b");
    }
}
```

- [ ] **Step 5: Run the unit tests**

Run: `cargo test -p reeved fetch::tests`
Expected: all 3 tests PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/reeved/src/fetch.rs crates/reeved/src/main.rs
git commit -m "feat(m4): fetch::tick — single-iteration fetch over running cards"
```

---

## Task 9: `fetch::run` loop with shutdown

**Files:**
- Modify: `crates/reeved/src/fetch.rs`

- [ ] **Step 1: Append `run` to `fetch.rs`**

Add above the `#[cfg(test)] mod tests` block:

```rust
/// Start the fetch worker loop. Returns when the shutdown receiver fires.
/// Each iteration calls `tick(state)`; sleeps `FETCH_INTERVAL` between
/// iterations. Either the timer or shutdown can win the `select!`.
pub async fn run(state: DaemonState, mut shutdown: tokio::sync::oneshot::Receiver<()>) {
    loop {
        tokio::select! {
            _ = &mut shutdown => {
                tracing::info!("fetch worker: shutdown received");
                return;
            }
            _ = tokio::time::sleep(FETCH_INTERVAL) => {
                if let Err(e) = tick(&state).await {
                    tracing::warn!("fetch worker tick: {e:#}");
                }
            }
        }
    }
}
```

- [ ] **Step 2: Write a shutdown-respecting test**

Append in `#[cfg(test)] mod tests`:

```rust
    #[tokio::test(start_paused = true)]
    async fn run_exits_on_shutdown_signal() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join("ws");
        write_manifest(&meta, "ws", &["ws"]);

        let git = Arc::new(RecordingGit::new());
        let state = state_with(git);
        let (tx, rx) = tokio::sync::oneshot::channel();

        let handle = tokio::spawn(super::run(state, rx));

        // Advance virtual time by less than one interval, then shut down.
        tokio::time::sleep(Duration::from_millis(100)).await;
        tx.send(()).unwrap();

        // The worker should exit cleanly.
        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("worker did not exit within 1 second")
            .expect("worker panicked");
    }
```

- [ ] **Step 3: Run the test**

Run: `cargo test -p reeved fetch::tests::run_exits_on_shutdown_signal`
Expected: PASS.

- [ ] **Step 4: Run the full fetch test suite**

Run: `cargo test -p reeved fetch`
Expected: 4 tests PASS (3 from Task 8 + the new shutdown one).

- [ ] **Step 5: Commit**

```bash
git add crates/reeved/src/fetch.rs
git commit -m "feat(m4): fetch::run — loop with FETCH_INTERVAL + shutdown channel"
```

---

## Task 10: Wire FetchWorker into main.rs

**Files:**
- Modify: `crates/reeved/src/main.rs`

- [ ] **Step 1: Add the spawn + shutdown wiring in main()**

Edit `crates/reeved/src/main.rs`. After `container::reconcile_on_startup(&state).await?;` (around line 93) and before the `let state_for_dispatch = state.clone();` line, add:

```rust
    // Spawn the fetch worker. Hold its shutdown sender; we'll fire it after
    // server::run returns.
    let (fetch_shutdown_tx, fetch_shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let fetch_worker = tokio::spawn(fetch::run(state.clone(), fetch_shutdown_rx));
```

Then after `let result = server::run(...)` returns (around line 108) and before `listeners.stop_all().await;`, add:

```rust
    // Shut the fetch worker down and wait for it to exit. Ignore send error
    // (channel may already be closed if the worker died early — e.g. panicked).
    let _ = fetch_shutdown_tx.send(());
    if let Err(e) = fetch_worker.await {
        warn!("fetch worker join error on shutdown: {e:#}");
    }
```

- [ ] **Step 2: Build the daemon and run the existing test suite**

Run: `cargo test -p reeved`
Expected: PASS. Nothing new should break; the worker spawns and shuts down cleanly in the unit-level path.

- [ ] **Step 3: Commit**

```bash
git add crates/reeved/src/main.rs
git commit -m "feat(m4): wire FetchWorker into main() — spawn at startup, shutdown after server::run"
```

---

## Task 11: CLI — `reeve push <id>` subcommand

**Files:**
- Modify: `crates/reeve/src/cli.rs`

- [ ] **Step 1: Add the `Push` subcommand variant**

Edit `crates/reeve/src/cli.rs`. In the existing `pub enum Command` (around line 29), add (alphabetical position; before or near `StatusSelf`):

```rust
    /// Promote namespaced card branches to refs/heads/ and push to origin.
    Push { id: String },
```

- [ ] **Step 2: Wire the dispatch in `run`**

In the `pub async fn run(cmd: Command)` match block, add:

```rust
        Command::Push { id } => push(id).await,
```

- [ ] **Step 3: Add the `push` handler function**

Append in `cli.rs` (alongside the other handlers like `ls`, `new`, `mv`):

```rust
async fn push(id: String) -> Result<()> {
    let socket = paths::detect_socket();
    let result = client::call(&socket, "push", serde_json::json!({ "id": id }))
        .await
        .context("calling push")?;
    match result {
        Ok(value) => {
            let result: reeve_protocol::PushResult = serde_json::from_value(value)?;
            if result.branches.is_empty() {
                println!(
                    "(no branches in refs/reeve/{id}/* — has the agent committed anything yet?)"
                );
                return Ok(());
            }
            // Print a per-branch table.
            println!("{:<24}  {:<32}  STATUS", "REPO", "BRANCH");
            let mut any_failed = false;
            for b in &result.branches {
                let status_str = match &b.status {
                    reeve_protocol::BranchPushStatus::Pushed => "pushed".to_string(),
                    reeve_protocol::BranchPushStatus::NotFastForward => {
                        any_failed = true;
                        "not-fast-forward".to_string()
                    }
                    reeve_protocol::BranchPushStatus::PushFailed { message } => {
                        any_failed = true;
                        format!("push-failed: {message}")
                    }
                };
                println!("{:<24}  {:<32}  {}", b.repo, b.branch, status_str);
            }
            if any_failed {
                anyhow::bail!("one or more branches did not push successfully");
            }
            Ok(())
        }
        Err(e) => anyhow::bail!("daemon error: {e}"),
    }
}
```

- [ ] **Step 4: Verify the CLI builds**

Run: `cargo build -p reeve`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/reeve/src/cli.rs
git commit -m "feat(m4): reeve push <id> CLI — prints per-branch table, exits non-zero on partial failure"
```

---

## Task 12: E2E test + workspace verification + manual smoke

**Files:**
- Modify: `tests/e2e.rs`

- [ ] **Step 1: Add helper for bare-remote setup**

Append in `tests/e2e.rs` (alongside the existing helpers like `build_test_template` and `init_sibling`):

```rust
/// Create a bare git repo at `path` and return its `file://` URL.
fn init_bare_remote(path: &std::path::Path) -> String {
    let status = std::process::Command::new("git")
        .args(["init", "--bare", "--quiet"])
        .arg(path)
        .status()
        .expect("git invoke");
    assert!(status.success(), "git init --bare at {} failed", path.display());
    format!("file://{}", path.display())
}

/// Add an `origin` remote to `repo` pointing at `url`.
fn add_origin(repo: &std::path::Path, url: &str) {
    git_in(repo, &["remote", "add", "origin", url]);
}
```

- [ ] **Step 2: Add the e2e test**

Append the full test in `tests/e2e.rs` (after the existing M3.5 e2e tests):

```rust
#[test]
fn m4_fetch_and_push_round_trip() {
    if !is_docker_available() { eprintln!("skip: docker"); return; }
    if !is_devcontainer_cli_available() { eprintln!("skip: devcontainer"); return; }

    let temp = tempfile::tempdir().unwrap();
    build_workspace();
    let env = isolated_env_with_config(temp.path());

    // 1. Build the host template (meta-repo + lore inline), then add a "code"
    //    sibling we'll commit through.
    let template_src = temp.path().join("template-source");
    build_test_template(&template_src);
    let dest = temp.path().join("m4-household");

    // 2. Daemon.
    let _daemon = DaemonGuard(
        cmd("reeved", &env)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn reeved"),
    );
    let socket = temp.path().join("reeved.sock");
    let mut waited = Duration::ZERO;
    while !socket.exists() && waited < Duration::from_secs(2) {
        sleep(Duration::from_millis(50));
        waited += Duration::from_millis(50);
    }
    assert!(socket.exists(), "socket not created");

    // 3. household new from the local template.
    let out = cmd("reeve", &env)
        .args(["household", "new", "m4-household",
               "--path", &dest.to_string_lossy(),
               "--template", &format!("file://{}", template_src.display())])
        .output()
        .expect("run reeve household new");
    assert!(out.status.success(), "household new: {}", String::from_utf8_lossy(&out.stderr));

    // 4. Populate a "code" sibling repo and wire bare-remote origins for
    //    meta-repo + code. (lore stays inline; no remote needed.)
    let code_sibling = dest.join("code");
    init_sibling(&code_sibling);
    let bare_meta = temp.path().join("origin-meta.git");
    let bare_code = temp.path().join("origin-code.git");
    let bare_meta_url = init_bare_remote(&bare_meta);
    let bare_code_url = init_bare_remote(&bare_code);
    add_origin(&dest, &bare_meta_url);
    add_origin(&code_sibling, &bare_code_url);

    // 5. Update manifest to declare code as a sibling; commit.
    write_manifest(&dest.join("household.json"), &format!(r#"{{
      "workspace": "m4-household",
      "knowledge_base": "lore",
      "repos": [
        {{ "name": "m4-household" }},
        {{ "name": "code" }},
        {{ "name": "lore" }}
      ]
    }}"#));
    commit_meta_changes(&dest);

    // 6. Create card + mv to implementing.
    let out_new = cmd("reeve", &env)
        .args(["new", "--household", "m4-household", "--brief", "m4 round-trip"])
        .output().expect("run reeve new");
    assert!(out_new.status.success(), "reeve new: {}", String::from_utf8_lossy(&out_new.stderr));
    let card_id = parse_card_id_from_new_output(&String::from_utf8_lossy(&out_new.stdout));

    let out_mv = cmd("reeve", &env)
        .args(["mv", &card_id, "implementing"])
        .output().expect("run reeve mv");
    assert!(out_mv.status.success(), "reeve mv: {}", String::from_utf8_lossy(&out_mv.stderr));

    let container_name = discover_container_by_label(&card_id)
        .expect("container should be discoverable by label");
    let _cleanup = DockerCleanup(Some(container_name.clone()));

    // 7. Commit inside the container as the `vscode` remoteUser, on both the
    //    meta-repo (repo zero) and the code sibling. Inline git config because
    //    the test container has no ambient identity.
    let workspace_in_container = format!("/workspaces/{card_id}");
    let exec = |args: &[&str]| -> std::process::Output {
        std::process::Command::new("docker")
            .args(["exec", "-u", "vscode", &container_name])
            .args(args)
            .output()
            .expect("docker exec")
    };

    // Meta-repo commit.
    let meta_commit = exec(&[
        "bash", "-lc",
        &format!("set -e; cd {workspace_in_container} && \
            git -c user.email=t@t.com -c user.name=t checkout -b feat-m4-meta && \
            echo 'meta-edit' >> CLAUDE.md && \
            git -c user.email=t@t.com -c user.name=t commit -am meta-edit"),
    ]);
    assert!(meta_commit.status.success(),
        "meta commit failed: {}", String::from_utf8_lossy(&meta_commit.stderr));

    // Code-sibling commit.
    let code_commit = exec(&[
        "bash", "-lc",
        &format!("set -e; cd {workspace_in_container}/code && \
            git -c user.email=t@t.com -c user.name=t checkout -b feat-m4-code && \
            echo 'hi' > smoke.txt && \
            git -c user.email=t@t.com -c user.name=t add smoke.txt && \
            git -c user.email=t@t.com -c user.name=t commit -m smoke"),
    ]);
    assert!(code_commit.status.success(),
        "code commit failed: {}", String::from_utf8_lossy(&code_commit.stderr));

    // 8. Wait at least one fetch tick.
    sleep(Duration::from_secs(7));

    // 9. Assert refs/reeve/<id>/* on the host.
    let meta_ref = std::process::Command::new("git")
        .args(["rev-parse", &format!("refs/reeve/{card_id}/feat-m4-meta")])
        .current_dir(&dest)
        .output()
        .unwrap();
    assert!(meta_ref.status.success(),
        "meta-repo namespaced ref: {}", String::from_utf8_lossy(&meta_ref.stderr));

    let code_ref = std::process::Command::new("git")
        .args(["rev-parse", &format!("refs/reeve/{card_id}/feat-m4-code")])
        .current_dir(&code_sibling)
        .output()
        .unwrap();
    assert!(code_ref.status.success(),
        "code-sibling namespaced ref: {}", String::from_utf8_lossy(&code_ref.stderr));

    // 10. reeve push.
    let out_push = cmd("reeve", &env).args(["push", &card_id]).output()
        .expect("run reeve push");
    assert!(out_push.status.success(),
        "reeve push: stdout={} stderr={}",
        String::from_utf8_lossy(&out_push.stdout),
        String::from_utf8_lossy(&out_push.stderr));

    // 11. Assert refs/heads/feat-m4-meta on host meta-repo AND in bare remote.
    let head_meta_host = std::process::Command::new("git")
        .args(["rev-parse", "refs/heads/feat-m4-meta"])
        .current_dir(&dest)
        .output().unwrap();
    assert!(head_meta_host.status.success(), "host meta has refs/heads/feat-m4-meta");
    let head_meta_remote = std::process::Command::new("git")
        .args(["rev-parse", "refs/heads/feat-m4-meta"])
        .current_dir(&bare_meta)
        .output().unwrap();
    assert!(head_meta_remote.status.success(), "bare-meta has refs/heads/feat-m4-meta");
    // And code sibling.
    let head_code_host = std::process::Command::new("git")
        .args(["rev-parse", "refs/heads/feat-m4-code"])
        .current_dir(&code_sibling)
        .output().unwrap();
    assert!(head_code_host.status.success(), "host code has refs/heads/feat-m4-code");
    let head_code_remote = std::process::Command::new("git")
        .args(["rev-parse", "refs/heads/feat-m4-code"])
        .current_dir(&bare_code)
        .output().unwrap();
    assert!(head_code_remote.status.success(), "bare-code has refs/heads/feat-m4-code");

    // 12. mv to done. Container is torn down. Namespaced refs persist.
    let out_done = cmd("reeve", &env).args(["mv", &card_id, "done"]).output()
        .expect("run reeve mv done");
    assert!(out_done.status.success(),
        "reeve mv done: {}", String::from_utf8_lossy(&out_done.stderr));

    let meta_ref_after = std::process::Command::new("git")
        .args(["rev-parse", &format!("refs/reeve/{card_id}/feat-m4-meta")])
        .current_dir(&dest)
        .output().unwrap();
    assert!(meta_ref_after.status.success(),
        "namespaced ref should still exist after Done");
}
```

You may need a small `write_manifest` helper if it doesn't already exist in `tests/e2e.rs` (Task 12 of M3.5 added `write_manifest(path, body)` — verify it's there). If absent, add:

```rust
fn write_manifest(path: &std::path::Path, body: &str) {
    std::fs::write(path, body).unwrap();
}
```

- [ ] **Step 3: Run the e2e test under strict mode**

Run: `REEVE_TESTS_STRICT=1 cargo test --test e2e m4_fetch_and_push_round_trip -- --nocapture --test-threads 1`
Expected: PASS. Test runtime ~30-60s (devcontainer pull on first run, image cached afterward).

- [ ] **Step 4: Workspace-wide verification**

Run sequentially:

```bash
REEVE_TESTS_STRICT=1 cargo test --workspace
cargo fmt --check
cargo clippy --workspace -- -D warnings
```

Expected: all three clean.

- [ ] **Step 5: Commit the e2e test**

```bash
git add tests/e2e.rs
git commit -m "test(m4): e2e fetch + push round-trip with bare-repo origins"
```

- [ ] **Step 6: Manual smoke per spec verification section**

Follow `docs/superpowers/specs/2026-05-14-m4-refs-and-push-and-kb-design.md` Verification section 4.1-4.9. Specifically:

1. Use the household scaffolded during M3.5 smoke (`reeve-self` at `~/Source/reeve-household`), or scaffold a fresh one.
2. `reeve daemon start`.
3. `reeve new --household reeve-self --brief "m4 smoke"` → capture `<id>`.
4. `reeve mv <id> implementing` → wait for container.
5. Make commits inside the container as `vscode` user across the meta-repo + each sibling.
6. Wait 6 seconds.
7. `git -C ~/Source/reeve-household log refs/reeve/<id>/<branch> -1 --oneline` confirms each commit landed.
8. **Negative smoke (push-failure rendering)**: `reeve push <id>` — the M3.5 smoke setup has no `origin` remotes, so this should print a per-branch table where every status is `push-failed: <git stderr>` and exit non-zero. Confirms the daemon iterates correctly and the CLI renders the failure status.
9. `reeve mv <id> done` → container down; namespaced refs persist.

If any step diverges, file a follow-up rather than block — but flag it in the PR description.

- [ ] **Step 7: Final commit if Cargo.lock changed**

If `cargo build --workspace` changed `Cargo.lock`:

```bash
git add Cargo.lock
git commit -m "chore(m4): bump lockfile"
```

---

## Self-review checklist

- [ ] **Spec coverage:** every section of `docs/superpowers/specs/2026-05-14-m4-refs-and-push-and-kb-design.md` maps to at least one task.
  - Continuous fetch worker → Tasks 8+9+10
  - `reeve push` semantics → Task 7 (handler), Task 11 (CLI)
  - Meta-repo as repo zero → Task 6 (`iter_card_repos` yields workspace first), exercised in Task 7's iteration-order test and Task 12 e2e (meta-repo commit + assertion)
  - Refspec topology → Task 2 (fetch refspec is `+refs/heads/*:refs/reeve/<id>/*`)
  - Error semantics → handled by `warn!` patterns in Task 8/9 and `BranchPushStatus` mapping in Task 7
  - Testing categories — unit (clones, fetch, handlers), integration (clones), e2e — all covered
  - DB-direct household lookup in worker AND push → Task 7 and Task 8 (both use `spawn_blocking + get_household`)
  - Manifest re-read failure semantics → Task 8 tick warns + continues
  - Deferred peer-pid bail → documented at plan top

- [ ] **Placeholder scan:** no `TBD/TODO/implement later/similar to Task N` patterns. Every code step contains complete code.

- [ ] **Type consistency:**
  - `FfOutcome { Updated, NotFastForward }` defined in Task 4, used in Tasks 5, 7, 8, 11.
  - `BranchPushStatus { Pushed, NotFastForward, PushFailed { message } }` defined in Task 1, used in Tasks 7, 11.
  - `iter_card_repos(&Household, &Manifest) -> impl Iterator<Item=(&str, PathBuf)>` defined in Task 6, used in Tasks 7, 8.
  - `MockGitOps` grows incrementally through Tasks 2-5; final struct shape with all 8 fields is consistent across the four tasks (each task's snippet shows the full struct shape).
  - The `tokio::sync::Mutex<Connection>` and the `spawn_blocking + blocking_lock` pattern in fetch/handlers matches the M3.5 convention introduced in Task 12 of that plan.

---

## Execution handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-17-m4-refs-and-push.md`. Two execution options:

**1. Subagent-Driven (recommended)** — dispatch a fresh subagent per task, two-stage review (spec compliance then code quality) after each. Fast iteration; keeps main-thread context for coordination only.

**2. Inline Execution** — execute tasks in this session via executing-plans, batch with checkpoints.

Which approach?
