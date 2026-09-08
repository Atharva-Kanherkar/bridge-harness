//! Per-repository build caches, shared across every checkout of a repository.
//!
//! Bridge's worktrees are cheap to cut — about 30 MB — and expensive to keep,
//! because agents build *inside* them. The single largest cost measured on a
//! real machine was one stopped worker holding 3.9 GB: 3.4 GB of
//! `src-tauri/target` and 508 MB of `node_modules`, none of it Bridge's own
//! doing and none of it worth a byte of backup. Capping and reclaiming
//! checkouts (see [`crate::worktree_registry`]) manages that cost. This module
//! removes most of it, by pointing the expensive directories at one cache per
//! repository instead of one per checkout.
//!
//! ## Why per repository, and what it costs
//!
//! Keying on the repository — not the checkout, not the branch — is what makes
//! a second worker start warm: dependencies dominate a build directory and they
//! are identical across branches of one project. Two consequences are worth
//! stating plainly rather than discovering:
//!
//! - **Concurrent builds serialize.** Cargo takes a lock on its target
//!   directory, so two workers building the same repository at once will wait
//!   on each other instead of building in parallel. That is a real cost, paid
//!   knowingly: the alternative is a multi-gigabyte target directory per
//!   worker, and `max_concurrent_workers` is 2 by default.
//! - **Switching branches churns.** Artifacts for changed crates are rebuilt.
//!   Artifacts for unchanged dependencies — the bulk — are not.
//!
//! ## What is deliberately not here
//!
//! Only the variables this project's own toolchain reads: Cargo, Bun, npm.
//! Redirecting caches for toolchains a repository may not use would be guessing,
//! and every variable added here is one more piece of an agent's environment
//! Bridge has taken over. `CARGO_HOME` is excluded specifically: it holds
//! registry credentials, and relocating it is a security decision, not a disk
//! one.

use crate::git;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::RwLock;

static BUILD_CACHE_ROOT: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Environment variables Bridge redirects, and the subdirectory each gets.
///
/// A variable already set in Bridge's own environment is left alone — an
/// operator who has pointed `CARGO_TARGET_DIR` somewhere deliberately outranks
/// this module.
const REDIRECTED: &[(&str, &str)] = &[
    ("CARGO_TARGET_DIR", "cargo-target"),
    ("BUN_INSTALL_CACHE_DIR", "bun"),
    ("npm_config_cache", "npm"),
];

/// Point the cache root at a directory, once, at startup. Mirrors the
/// registration [`crate::claude_adapter::register_node_compile_cache_root`] and
/// [`crate::process_ledger::register_ledger_root`] use: the spawn sites are deep
/// in adapter code that has no route to the data directory.
pub fn register_root(root: impl Into<PathBuf>) {
    *BUILD_CACHE_ROOT
        .write()
        .expect("the build cache root lock is never poisoned") = Some(root.into());
}

fn root() -> Option<PathBuf> {
    BUILD_CACHE_ROOT
        .read()
        .expect("the build cache root lock is never poisoned")
        .clone()
}

/// The cache directory for whichever repository `cwd` belongs to, or `None` when
/// no repository can be resolved — a scratch chat with no project gets no cache
/// rather than sharing a wrong one.
///
/// Named from the repository's own directory plus a short digest of its full
/// path, so two projects that happen to share a basename do not share a cache.
pub fn repository_cache(cwd: &Path) -> Option<PathBuf> {
    repository_cache_in(&root()?, cwd)
}

/// [`repository_cache`] against an explicit root. The registered root is process
/// global, which makes it awkward to reason about and impossible to test in
/// parallel; every decision this module makes is expressed here, against
/// arguments, and the global is read only at the edges.
pub fn repository_cache_in(root: &Path, cwd: &Path) -> Option<PathBuf> {
    let repo = git::main_worktree_root(cwd)?;
    let name = repo
        .file_name()
        .and_then(|value| value.to_str())
        .map(git::slug)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "repository".to_owned());
    Some(root.join(format!("{name}-{}", path_digest(&repo))))
}

/// Short, stable digest of a path. Only needs to separate distinct repositories,
/// so a truncated hash reads better in a directory listing than a full one.
fn path_digest(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    format!("{:x}", hasher.finalize())[..12].to_owned()
}

/// The variables to set for a process working in `cwd`, in order.
pub fn env_for(cwd: &Path) -> Vec<(&'static str, PathBuf)> {
    let Some(root) = root() else {
        return Vec::new();
    };
    variables_in(&root, cwd, |key| std::env::var_os(key).is_some())
}

/// The decision, as a pure function.
///
/// `already_set` is injected rather than read from the process environment so
/// this can be tested without mutating global state — which, besides being
/// unsound in a threaded test binary, made concurrently running tests silently
/// change each other's answers.
pub fn variables_in(
    root: &Path,
    cwd: &Path,
    already_set: impl Fn(&str) -> bool,
) -> Vec<(&'static str, PathBuf)> {
    let Some(cache) = repository_cache_in(root, cwd) else {
        return Vec::new();
    };
    REDIRECTED
        .iter()
        .filter(|(key, _)| !already_set(key))
        .map(|(key, directory)| (*key, cache.join(directory)))
        .collect()
}

/// Redirect a process's build and package caches out of its worktree.
///
/// Returns the cache directory when one was applied, for logging. Directory
/// creation is best effort: a cache that cannot be created is not worth failing
/// a session over, and every tool here creates its own directory anyway.
pub fn apply(command: &mut Command, cwd: &Path) -> Option<PathBuf> {
    let variables = env_for(cwd);
    if variables.is_empty() {
        return None;
    }
    for (key, value) in &variables {
        let _ = std::fs::create_dir_all(value);
        command.env(key, value);
    }
    repository_cache(cwd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as Process;

    fn git_cmd(cwd: &Path, args: &[&str]) {
        let output = Process::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// A repository plus one linked worktree of it.
    fn repo_with_worktree(base: &Path, name: &str) -> (PathBuf, PathBuf) {
        let repo = base.join(name);
        std::fs::create_dir_all(&repo).unwrap();
        let repo = std::fs::canonicalize(&repo).unwrap();
        git_cmd(&repo, &["init", "-q", "-b", "main"]);
        git_cmd(&repo, &["config", "user.email", "t@example.invalid"]);
        git_cmd(&repo, &["config", "user.name", "Bridge Test"]);
        git_cmd(&repo, &["config", "commit.gpgsign", "false"]);
        std::fs::write(repo.join("base.txt"), "base\n").unwrap();
        git_cmd(&repo, &["add", "."]);
        git_cmd(&repo, &["commit", "-q", "-m", "base"]);
        let linked = base.join(format!("{name}-worker"));
        git_cmd(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feat/x",
                linked.to_str().unwrap(),
                "HEAD",
            ],
        );
        (repo, linked)
    }

    /// Every test works against an explicit root. The registered root is
    /// process global, so tests that set it would change each other's answers
    /// as they run in parallel.
    fn root_for(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("build-caches")
    }

    /// Nothing is set: the honest default for the pure tests below.
    fn nothing_set(_: &str) -> bool {
        false
    }

    /// The whole point: a second checkout of the same repository starts warm.
    #[test]
    fn the_cache_is_keyed_by_repository_not_by_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let root = root_for(&dir);
        let (repo, linked) = repo_with_worktree(dir.path(), "alpha");

        let from_repo = repository_cache_in(&root, &repo).unwrap();
        let from_worktree = repository_cache_in(&root, &linked).unwrap();
        assert_eq!(
            from_repo, from_worktree,
            "every checkout of one repository shares one cache",
        );

        let (other, _) = repo_with_worktree(dir.path(), "beta");
        assert_ne!(
            repository_cache_in(&root, &other).unwrap(),
            from_repo,
            "and two repositories never share one",
        );
    }

    /// Two projects can share a basename; they must not share a cache.
    #[test]
    fn repositories_with_the_same_name_get_separate_caches() {
        let dir = tempfile::tempdir().unwrap();
        let root = root_for(&dir);
        let left = dir.path().join("left");
        let right = dir.path().join("right");
        std::fs::create_dir_all(&left).unwrap();
        std::fs::create_dir_all(&right).unwrap();
        let (first, _) = repo_with_worktree(&left, "harness");
        let (second, _) = repo_with_worktree(&right, "harness");
        assert_ne!(
            repository_cache_in(&root, &first).unwrap(),
            repository_cache_in(&root, &second).unwrap(),
        );
    }

    #[test]
    fn a_directory_that_is_not_a_repository_gets_no_cache() {
        let dir = tempfile::tempdir().unwrap();
        let root = root_for(&dir);
        let scratch = dir.path().join("scratch");
        std::fs::create_dir_all(&scratch).unwrap();
        assert!(repository_cache_in(&root, &scratch).is_none());
        assert!(
            variables_in(&root, &scratch, nothing_set).is_empty(),
            "and no variables are set",
        );
    }

    #[test]
    fn every_redirected_variable_points_inside_the_repository_cache() {
        let dir = tempfile::tempdir().unwrap();
        let root = root_for(&dir);
        let (repo, linked) = repo_with_worktree(dir.path(), "alpha");
        let cache = repository_cache_in(&root, &repo).unwrap();
        let variables = variables_in(&root, &linked, nothing_set);
        assert_eq!(variables.len(), REDIRECTED.len());
        for (key, value) in variables {
            assert!(
                value.starts_with(&cache),
                "{key} points outside the cache: {}",
                value.display(),
            );
            assert!(
                !value.starts_with(&linked),
                "{key} still points into the checkout, which is the whole problem",
            );
        }
    }

    /// An operator who has pointed a cache somewhere deliberately outranks this.
    #[test]
    fn an_operator_set_variable_is_never_overridden() {
        let dir = tempfile::tempdir().unwrap();
        let root = root_for(&dir);
        let (repo, _) = repo_with_worktree(dir.path(), "alpha");
        let keys: Vec<&str> = variables_in(&root, &repo, |key| key == "CARGO_TARGET_DIR")
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        assert!(
            !keys.contains(&"CARGO_TARGET_DIR"),
            "Bridge must not overwrite a deliberate choice: {keys:?}",
        );
        assert!(keys.contains(&"BUN_INSTALL_CACHE_DIR"), "{keys:?}");
    }

    /// The one test that exercises the global registration and the real
    /// environment, so `apply` itself is covered end to end. Serialized against
    /// the other global user rather than run in parallel with it.
    #[test]
    fn apply_sets_the_variables_and_creates_the_directories() {
        static GLOBAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = GLOBAL.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        register_root(root_for(&dir));
        let (repo, _) = repo_with_worktree(dir.path(), "alpha");

        let mut command = Process::new("true");
        let applied = apply(&mut command, &repo).expect("a cache is applied");
        assert_eq!(applied, repository_cache_in(&root_for(&dir), &repo).unwrap());
        for (_, directory) in REDIRECTED {
            // Any variable the surrounding environment already sets is skipped
            // by design, so only assert on the ones that were applied.
            if std::env::var_os(
                REDIRECTED
                    .iter()
                    .find(|(_, sub)| sub == directory)
                    .map(|(key, _)| *key)
                    .unwrap(),
            )
            .is_none()
            {
                assert!(
                    applied.join(directory).is_dir(),
                    "{directory} was not created",
                );
            }
        }

        let scratch = dir.path().join("scratch");
        std::fs::create_dir_all(&scratch).unwrap();
        let mut outside = Process::new("true");
        assert!(
            apply(&mut outside, &scratch).is_none(),
            "and nothing is applied outside a repository",
        );
    }
}
