//! Safe project discovery primitives used by the onboarding dialog.

use crate::{git, BridgeError};
use bridge_protocol::messages::GithubRepoCandidate;
use serde::Deserialize;
use std::{collections::VecDeque, path::{Path, PathBuf}, process::Command};

const MAX_LOCATE_DEPTH: usize = 4;
const MAX_LOCATE_RESULTS: usize = 20;

pub fn clone_repo(url: &str, destination: &Path) -> Result<PathBuf, BridgeError> {
    let url = validate_github_url(url)?;
    if destination.exists() {
        return Err(BridgeError::Invalid(format!("Clone destination already exists: {}", destination.display())));
    }
    let parent = destination.parent().ok_or_else(|| BridgeError::Invalid("Clone destination needs a parent folder".into()))?;
    std::fs::create_dir_all(parent)?;
    let output = git::git_command(parent).args(["clone", "--", url, &destination.to_string_lossy()]).output()?;
    if !output.status.success() {
        return Err(BridgeError::Git(String::from_utf8_lossy(&output.stderr).trim().into()));
    }
    Ok(std::fs::canonicalize(destination)?)
}

pub fn validate_github_url(value: &str) -> Result<&str, BridgeError> {
    let url = value.trim();
    let github_https = url.starts_with("https://github.com/") || url.starts_with("http://github.com/");
    let github_ssh = url.starts_with("git@github.com:") || url.starts_with("ssh://git@github.com/");
    let path = if let Some(path) = url.strip_prefix("git@github.com:") {
        path
    } else if let Some(path) = url.strip_prefix("ssh://git@github.com/") {
        path
    } else {
        url.trim_start_matches("https://github.com/").trim_start_matches("http://github.com/")
    };
    if !(github_https || github_ssh) || url.starts_with('-') || path.trim_matches('/').split('/').count() != 2 {
        return Err(BridgeError::Invalid("Enter a GitHub repository HTTPS or SSH URL".into()));
    }
    Ok(url)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhRepo { name_with_owner: String, url: String, ssh_url: String, private: bool, pushed_at: Option<String>, description: Option<String> }

pub fn search_github_repos(query: &str) -> Result<Vec<GithubRepoCandidate>, BridgeError> {
    let query = query.trim();
    if query.is_empty() { return Err(BridgeError::Invalid("Enter a repository name to search".into())); }
    // The same resolution the GitHub surface uses. A minimal macOS GUI `PATH`
    // has no `/opt/homebrew/bin`, so `Command::new("gh")` fails on exactly the
    // installs the surface reports as available.
    let binary = crate::binary::resolve("gh").ok_or_else(|| BridgeError::Adapter("GitHub CLI unavailable: gh is not installed".into()))?;
    let output = Command::new(binary).args(["api", "--paginate", "/user/repos?per_page=100&affiliation=owner,collaborator,organization_member"])
        .output().map_err(|error| BridgeError::Adapter(format!("GitHub CLI unavailable: {error}")))?;
    if !output.status.success() { return Err(BridgeError::Adapter(String::from_utf8_lossy(&output.stderr).trim().into())); }
    let repos = parse_repo_pages(&output.stdout)?;
    let needle = query.to_lowercase();
    Ok(repos.into_iter().filter(|repo| [repo.name_with_owner.as_str(), repo.description.as_deref().unwrap_or("")].iter().any(|value| value.to_lowercase().contains(&needle)))
        .take(MAX_LOCATE_RESULTS).map(|repo| GithubRepoCandidate { name_with_owner: repo.name_with_owner, url: repo.url, ssh_url: repo.ssh_url, is_private: repo.private, pushed_at: repo.pushed_at }).collect())
}

/// `gh api --paginate` writes one JSON array per page, concatenated — past
/// 100 repositories a single `from_slice` sees trailing JSON and fails, which
/// the caller could only report as "no matches". Read every page instead.
fn parse_repo_pages(stdout: &[u8]) -> Result<Vec<GhRepo>, BridgeError> {
    let mut repos = Vec::new();
    for page in serde_json::Deserializer::from_slice(stdout).into_iter::<Vec<GhRepo>>() {
        repos.extend(page.map_err(|error| BridgeError::Adapter(format!("GitHub returned invalid repository data: {error}")))?);
    }
    Ok(repos)
}

pub fn locate_folders(query: &str, roots: &[String]) -> Result<Vec<String>, BridgeError> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() { return Err(BridgeError::Invalid("Describe the project to locate".into())); }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut queue: VecDeque<(PathBuf, usize)> = roots.iter().map(|root| root.strip_prefix("~/").and_then(|tail| home.as_ref().map(|home| home.join(tail))).unwrap_or_else(|| PathBuf::from(root))).filter(|root| root.is_dir()).map(|root| (root, 0)).collect();
    let mut matches = Vec::new();
    while let Some((path, depth)) = queue.pop_front() {
        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("").to_lowercase();
        if name.contains(&needle) && path.join(".git").exists() { matches.push(path.to_string_lossy().to_string()); if matches.len() == MAX_LOCATE_RESULTS { break; } }
        if depth < MAX_LOCATE_DEPTH { if let Ok(entries) = std::fs::read_dir(&path) { for entry in entries.flatten() { let child = entry.path(); if child.is_dir() && !child.file_name().is_some_and(|name| name == ".git" || name == "node_modules") { queue.push_back((child, depth + 1)); } } } }
    }
    Ok(matches)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_github_clone_urls() {
        assert!(validate_github_url("https://github.com/bridge/harness.git").is_ok());
        assert!(validate_github_url("git@github.com:bridge/harness.git").is_ok());
        assert!(validate_github_url("ssh://git@github.com/bridge/harness.git").is_ok());
        assert!(validate_github_url("https://example.com/bridge/harness").is_err());
        assert!(validate_github_url("--upload-pack=evil").is_err());
    }

    #[test]
    fn every_paginated_page_is_parsed() {
        let page = |name: &str| format!(
            r#"[{{"nameWithOwner":"owner/{name}","url":"https://github.com/owner/{name}","sshUrl":"git@github.com:owner/{name}.git","private":false,"pushedAt":null,"description":null}}]"#
        );
        let single = parse_repo_pages(page("one").as_bytes()).unwrap();
        assert_eq!(single.len(), 1);

        // Two pages, concatenated exactly as `gh api --paginate` emits them.
        let concatenated = format!("{}\n{}", page("one"), page("two"));
        let both = parse_repo_pages(concatenated.as_bytes()).unwrap();
        assert_eq!(
            both.iter().map(|repo| repo.name_with_owner.as_str()).collect::<Vec<_>>(),
            ["owner/one", "owner/two"]
        );

        assert!(parse_repo_pages(b"not json").is_err());
    }

    #[test]
    fn locate_returns_only_matching_git_roots() {
        let scratch = tempfile::tempdir().unwrap();
        let repo = scratch.path().join("payments-api");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(scratch.path().join("payments-not-a-repo")).unwrap();
        assert_eq!(locate_folders("payments", &[scratch.path().to_string_lossy().into_owned()]).unwrap(), vec![repo.to_string_lossy().into_owned()]);
    }
}
