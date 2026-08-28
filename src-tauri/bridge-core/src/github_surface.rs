//! Deterministic, read-only access to GitHub data through the user's `gh` CLI.
//!
//! The surface deliberately owns no credentials. `gh auth status` is the
//! availability boundary, repository identity comes from the workspace's Git
//! configuration, and every later GitHub operation is launched as an argv
//! array rather than through a shell.

use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Mutex,
};
use thiserror::Error;

pub const AUTH_REMEDIATION: &str = "gh auth login";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum GithubAvailability {
    Available,
    NotInstalled,
    NotAuthenticated { remediation: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubRepository {
    pub host: String,
    pub owner: String,
    pub name: String,
}

impl GithubRepository {
    /// Repository selector accepted by `gh --repo`.
    pub fn selector(&self) -> String {
        if self.host.eq_ignore_ascii_case("github.com") {
            format!("{}/{}", self.owner, self.name)
        } else {
            format!("{}/{}/{}", self.host, self.owner, self.name)
        }
    }
}

#[derive(Debug, Error)]
pub enum GithubSurfaceError {
    #[error("GitHub CLI is unavailable: {status:?}")]
    Unavailable { status: GithubAvailability },
    #[error("GitHub command {operation} failed: {stderr}")]
    CommandFailed {
        operation: &'static str,
        stderr: String,
    },
    #[error("GitHub returned a malformed {resource} response: {detail}")]
    MalformedResponse {
        resource: &'static str,
        detail: String,
    },
    #[error("could not resolve a GitHub repository for {workspace}: {detail}")]
    RepositoryResolution { workspace: String, detail: String },
    #[error("I/O while invoking GitHub tooling: {0}")]
    Io(#[from] std::io::Error),
}

/// A discovered `gh` installation and its current authentication state.
///
/// The status is retained so normal reads do not rerun `gh auth status` on
/// every panel refresh. Call [`GithubSurface::refresh_availability`] when the
/// user completes `gh auth login` or asks to probe again.
pub struct GithubSurface {
    binary: Option<PathBuf>,
    availability: Mutex<GithubAvailability>,
}

impl std::fmt::Debug for GithubSurface {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GithubSurface")
            .field("binary", &self.binary)
            .field("availability", &self.availability())
            .finish()
    }
}

impl Default for GithubSurface {
    fn default() -> Self {
        Self::discover()
    }
}

impl GithubSurface {
    pub fn discover() -> Self {
        Self::from_binary(crate::binary::resolve("gh"))
    }

    fn from_binary(binary: Option<PathBuf>) -> Self {
        let availability = probe_availability(binary.as_deref());
        Self {
            binary,
            availability: Mutex::new(availability),
        }
    }

    pub fn availability(&self) -> GithubAvailability {
        self.availability
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn refresh_availability(&self) -> GithubAvailability {
        let refreshed = probe_availability(self.binary.as_deref());
        *self
            .availability
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = refreshed.clone();
        refreshed
    }

    fn require_binary(&self) -> Result<&Path, GithubSurfaceError> {
        let status = self.availability();
        if status != GithubAvailability::Available {
            return Err(GithubSurfaceError::Unavailable { status });
        }
        self.binary
            .as_deref()
            .ok_or(GithubSurfaceError::Unavailable {
                status: GithubAvailability::NotInstalled,
            })
    }

    /// Resolve the repository from the workspace itself; callers cannot supply
    /// an arbitrary `owner/repo` string.
    pub fn resolve_repository(
        &self,
        workspace: &Path,
    ) -> Result<GithubRepository, GithubSurfaceError> {
        ensure_git_repository(workspace)?;

        for remote in branch_remote_candidates(workspace) {
            if let Some(repository) = repository_for_remote(workspace, &remote) {
                return Ok(repository);
            }
        }

        if let Some(repository) = self.default_repository(workspace) {
            return Ok(repository);
        }

        if let Some(repository) = repository_for_remote(workspace, "origin") {
            return Ok(repository);
        }

        Err(GithubSurfaceError::RepositoryResolution {
            workspace: workspace.display().to_string(),
            detail: "no GitHub push/upstream remote, gh default, or origin remote was available"
                .into(),
        })
    }

    fn default_repository(&self, workspace: &Path) -> Option<GithubRepository> {
        let binary = self.require_binary().ok()?;
        let output = Command::new(binary)
            .current_dir(workspace)
            .args(["repo", "set-default", "--view"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        parse_repository_selector(String::from_utf8_lossy(&output.stdout).trim())
    }

    #[cfg(test)]
    fn discover_on_path(path: &Path) -> Self {
        let binary = which::which_in("gh", Some(std::ffi::OsString::from(path)), ".").ok();
        Self::from_binary(binary)
    }
}

fn probe_availability(binary: Option<&Path>) -> GithubAvailability {
    let Some(binary) = binary else {
        return GithubAvailability::NotInstalled;
    };
    match Command::new(binary).args(["auth", "status"]).output() {
        Ok(output) if output.status.success() => GithubAvailability::Available,
        Ok(_) => GithubAvailability::NotAuthenticated {
            remediation: AUTH_REMEDIATION.into(),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            GithubAvailability::NotInstalled
        }
        Err(_) => GithubAvailability::NotAuthenticated {
            remediation: AUTH_REMEDIATION.into(),
        },
    }
}

fn ensure_git_repository(workspace: &Path) -> Result<(), GithubSurfaceError> {
    let output = run_git(workspace, ["rev-parse", "--git-dir"])?;
    if output.status.success() {
        Ok(())
    } else {
        Err(GithubSurfaceError::RepositoryResolution {
            workspace: workspace.display().to_string(),
            detail: stderr_or_status(&output),
        })
    }
}

/// Git's push destination precedence is branch.pushRemote, then
/// remote.pushDefault, then the branch's upstream remote. Keep that ordering
/// before consulting `gh`'s per-repository default.
fn branch_remote_candidates(workspace: &Path) -> Vec<String> {
    let Some(branch) = git_stdout(workspace, ["symbolic-ref", "--quiet", "--short", "HEAD"]) else {
        return Vec::new();
    };
    let keys = [
        format!("branch.{branch}.pushRemote"),
        "remote.pushDefault".to_owned(),
        format!("branch.{branch}.remote"),
    ];
    let mut remotes = Vec::new();
    for key in keys {
        if let Some(remote) = git_stdout(workspace, ["config", "--get", key.as_str()]) {
            if remote != "." && !remotes.contains(&remote) {
                remotes.push(remote);
            }
        }
    }
    remotes
}

fn repository_for_remote(workspace: &Path, remote: &str) -> Option<GithubRepository> {
    let url = git_stdout(workspace, ["remote", "get-url", "--push", remote])
        .or_else(|| git_stdout(workspace, ["remote", "get-url", remote]))?;
    parse_remote_url(&url)
}

fn run_git<I, S>(workspace: &Path, args: I) -> Result<Output, GithubSurfaceError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    Ok(Command::new("git")
        .current_dir(workspace)
        .args(args)
        .output()?)
}

fn git_stdout<I, S>(workspace: &Path, args: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let output = run_git(workspace, args).ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn stderr_or_status(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if stderr.is_empty() {
        format!("process exited with {}", output.status)
    } else {
        stderr
    }
}

fn parse_repository_selector(value: &str) -> Option<GithubRepository> {
    let parts = value
        .trim()
        .trim_end_matches(".git")
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let (host, owner, name) = match parts.as_slice() {
        [owner, name] => ("github.com", *owner, *name),
        [host, owner, name] => (*host, *owner, *name),
        _ => return None,
    };
    valid_repository_parts(host, owner, name).then(|| GithubRepository {
        host: host.to_owned(),
        owner: owner.to_owned(),
        name: name.to_owned(),
    })
}

fn parse_remote_url(value: &str) -> Option<GithubRepository> {
    let value = value.trim().trim_end_matches('/').trim_end_matches(".git");
    if let Some((_, rest)) = value.split_once("://") {
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit('@').next()?;
        return parse_repository_selector(&format!("{host}/{path}"));
    }

    // scp-style Git URL: git@github.com:owner/repository.git
    if let Some((authority, path)) = value.split_once(':') {
        if authority.contains('@') && !path.starts_with('/') {
            let host = authority.rsplit('@').next()?;
            return parse_repository_selector(&format!("{host}/{path}"));
        }
    }
    None
}

fn valid_repository_parts(host: &str, owner: &str, name: &str) -> bool {
    !host.is_empty()
        && !owner.is_empty()
        && !name.is_empty()
        && ![host, owner, name]
            .iter()
            .any(|part| part.contains(char::is_whitespace) || *part == "." || *part == "..")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt, process::Command};
    use tempfile::TempDir;

    fn fake_gh(authenticated: bool, default_repository: Option<&str>) -> TempDir {
        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("gh");
        let auth_exit = if authenticated { 0 } else { 1 };
        let default = default_repository.unwrap_or("");
        let script = format!(
            "#!/bin/sh\nif [ \"$1 $2\" = \"auth status\" ]; then exit {auth_exit}; fi\nif [ \"$1 $2 $3\" = \"repo set-default --view\" ]; then\n  if [ -n \"{default}\" ]; then printf '%s\\n' '{default}'; exit 0; fi\n  exit 1\nfi\nexit 2\n"
        );
        fs::write(&binary, script).unwrap();
        let mut permissions = fs::metadata(&binary).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&binary, permissions).unwrap();
        directory
    }

    fn git(repository: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(repository)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Bridge Test")
            .env("GIT_AUTHOR_EMAIL", "bridge@example.com")
            .env("GIT_COMMITTER_NAME", "Bridge Test")
            .env("GIT_COMMITTER_EMAIL", "bridge@example.com")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn repository() -> TempDir {
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "-b", "main"]);
        fs::write(directory.path().join("README.md"), "fixture\n").unwrap();
        git(directory.path(), &["add", "README.md"]);
        git(directory.path(), &["commit", "-m", "fixture"]);
        directory
    }

    fn expected(owner: &str, name: &str) -> GithubRepository {
        GithubRepository {
            host: "github.com".into(),
            owner: owner.into(),
            name: name.into(),
        }
    }

    #[test]
    fn discovery_reports_available_for_an_authenticated_gh() {
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());
        assert_eq!(surface.availability(), GithubAvailability::Available);
    }

    #[test]
    fn discovery_reports_not_installed_without_gh() {
        let empty_path = tempfile::tempdir().unwrap();
        let surface = GithubSurface::discover_on_path(empty_path.path());
        assert_eq!(surface.availability(), GithubAvailability::NotInstalled);
        assert!(matches!(
            surface.require_binary(),
            Err(GithubSurfaceError::Unavailable {
                status: GithubAvailability::NotInstalled
            })
        ));
    }

    #[test]
    fn discovery_reports_signed_out_with_exact_remediation() {
        let fake = fake_gh(false, None);
        let surface = GithubSurface::discover_on_path(fake.path());
        assert_eq!(
            surface.availability(),
            GithubAvailability::NotAuthenticated {
                remediation: "gh auth login".into()
            }
        );
    }

    #[test]
    fn repository_resolution_follows_push_default_origin_order() {
        let repository = repository();
        git(
            repository.path(),
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/origin/project.git",
            ],
        );
        git(
            repository.path(),
            &[
                "remote",
                "add",
                "upstream",
                "git@github.com:push/project.git",
            ],
        );
        git(
            repository.path(),
            &["config", "branch.main.pushRemote", "upstream"],
        );
        let fake = fake_gh(true, Some("default/project"));
        let surface = GithubSurface::discover_on_path(fake.path());
        assert_eq!(
            surface.resolve_repository(repository.path()).unwrap(),
            expected("push", "project")
        );

        git(
            repository.path(),
            &["config", "--unset", "branch.main.pushRemote"],
        );
        assert_eq!(
            surface.resolve_repository(repository.path()).unwrap(),
            expected("default", "project")
        );

        let no_default = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(no_default.path());
        assert_eq!(
            surface.resolve_repository(repository.path()).unwrap(),
            expected("origin", "project")
        );
    }

    #[test]
    fn upstream_remote_is_used_when_no_push_remote_is_configured() {
        let repository = repository();
        git(
            repository.path(),
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/upstream/project.git",
            ],
        );
        git(
            repository.path(),
            &["config", "branch.main.remote", "origin"],
        );
        let fake = fake_gh(true, Some("default/project"));
        let surface = GithubSurface::discover_on_path(fake.path());
        assert_eq!(
            surface.resolve_repository(repository.path()).unwrap(),
            expected("upstream", "project")
        );
    }

    #[test]
    fn linked_worktree_resolves_like_its_parent() {
        let repository = repository();
        git(
            repository.path(),
            &[
                "remote",
                "add",
                "origin",
                "ssh://git@github.com/shared/project.git",
            ],
        );
        let worktree_root = tempfile::tempdir().unwrap();
        let worktree = worktree_root.path().join("linked");
        git(
            repository.path(),
            &[
                "worktree",
                "add",
                "-b",
                "linked",
                worktree.to_str().unwrap(),
            ],
        );
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());
        assert_eq!(
            surface.resolve_repository(repository.path()).unwrap(),
            surface.resolve_repository(&worktree).unwrap()
        );
    }

    #[test]
    fn remote_url_formats_are_normalized_without_accepting_local_paths() {
        assert_eq!(
            parse_remote_url("https://github.com/owner/repo.git"),
            Some(expected("owner", "repo"))
        );
        assert_eq!(
            parse_remote_url("git@github.com:owner/repo.git"),
            Some(expected("owner", "repo"))
        );
        assert_eq!(
            parse_remote_url("ssh://git@github.example.com/owner/repo.git")
                .unwrap()
                .selector(),
            "github.example.com/owner/repo"
        );
        assert_eq!(parse_remote_url("../local/repo"), None);
    }
}
