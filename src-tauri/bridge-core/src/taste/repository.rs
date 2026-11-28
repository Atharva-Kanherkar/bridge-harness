//! Which repository a workspace works in, as a taste scope
//! (`project:<owner>/<repo>`): from its latest review when it has one, else
//! from its checkout's `origin` remote, read from the git config without
//! running git.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use crate::BridgeError;

/// The repository scope of a workspace, if it is a GitHub repository Bridge
/// can name.
pub fn scope_of_workspace(db: &Connection, workspace_id: &str) -> Result<Option<String>, BridgeError> {
    let reviewed: Option<(String, String)> = db
        .query_row(
            "SELECT repo_owner, repo_name FROM review_jobs WHERE workspace_id=?1 ORDER BY created_at DESC LIMIT 1",
            params![workspace_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((owner, repo)) = reviewed {
        return Ok(crate::memory_ledger::project_scope(&owner, &repo).ok());
    }
    let path: Option<String> =
        db.query_row("SELECT path FROM workspaces WHERE id=?1", params![workspace_id], |row| row.get(0)).optional()?;
    Ok(path
        .and_then(|path| origin_of(Path::new(&path)))
        .and_then(|(owner, repo)| crate::memory_ledger::project_scope(&owner, &repo).ok()))
}

/// The repository scope of a chat's workspace; none for a chat with no
/// workspace or one Bridge cannot name.
pub fn scope_of_session(db: &Connection, session_id: &str) -> Result<Option<String>, BridgeError> {
    let workspace: Option<String> = db
        .query_row("SELECT workspace_id FROM sessions WHERE id=?1", params![session_id], |row| row.get(0))
        .optional()?
        .flatten();
    match workspace {
        Some(workspace) => scope_of_workspace(db, &workspace),
        None => Ok(None),
    }
}

/// The git config a checkout reads: `.git/config`, or for a linked worktree
/// the main repository's, through `gitdir:` and `commondir`.
fn config_of(checkout: &Path) -> Option<PathBuf> {
    let dot_git = checkout.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git.join("config"));
    }
    let pointer = std::fs::read_to_string(&dot_git).ok()?;
    let gitdir = pointer.lines().find_map(|line| line.strip_prefix("gitdir:")).map(str::trim)?;
    let gitdir = if Path::new(gitdir).is_absolute() { PathBuf::from(gitdir) } else { checkout.join(gitdir) };
    let common = std::fs::read_to_string(gitdir.join("commondir"))
        .ok()
        .map(|common| {
            let common = common.trim();
            if Path::new(common).is_absolute() { PathBuf::from(common) } else { gitdir.join(common) }
        })
        .unwrap_or(gitdir);
    Some(common.join("config"))
}

/// The owner and name of the checkout's `origin` remote, when it is GitHub.
pub fn origin_of(checkout: &Path) -> Option<(String, String)> {
    let config = std::fs::read_to_string(config_of(checkout)?).ok()?;
    let mut in_origin = false;
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            in_origin = line.replace(' ', "") == "[remote\"origin\"]";
            continue;
        }
        if in_origin {
            if let Some((key, value)) = line.split_once('=') {
                if key.trim() == "url" {
                    let repository = crate::github_surface::parse_remote_url(value.trim())?;
                    return (repository.host.eq_ignore_ascii_case("github.com")).then_some((repository.owner, repository.name));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkout(config: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/config"), config).unwrap();
        dir
    }

    #[test]
    fn the_origin_remote_names_the_repository() {
        let https = checkout("[core]\n\tbare = false\n[remote \"upstream\"]\n\turl = https://github.com/other/thing.git\n[remote \"origin\"]\n\turl = https://github.com/Octo/Repo.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n");
        assert_eq!(origin_of(https.path()), Some(("Octo".into(), "Repo".into())));
        let ssh = checkout("[remote \"origin\"]\n\turl = git@github.com:octo/repo.git\n");
        assert_eq!(origin_of(ssh.path()), Some(("octo".into(), "repo".into())));
        assert_eq!(origin_of(checkout("[remote \"origin\"]\n\turl = https://gitlab.com/octo/repo.git\n").path()), None, "GitHub only");
        assert_eq!(origin_of(checkout("[core]\n").path()), None);
        assert_eq!(origin_of(tempfile::tempdir().unwrap().path()), None, "not a checkout");
    }

    #[test]
    fn a_linked_worktree_reads_its_main_repositorys_config() {
        let main = checkout("[remote \"origin\"]\n\turl = https://github.com/octo/repo\n");
        let gitdir = main.path().join(".git/worktrees/feature");
        std::fs::create_dir_all(&gitdir).unwrap();
        std::fs::write(gitdir.join("commondir"), "../..\n").unwrap();
        let worktree = tempfile::tempdir().unwrap();
        std::fs::write(worktree.path().join(".git"), format!("gitdir: {}\n", gitdir.display())).unwrap();
        assert_eq!(origin_of(worktree.path()), Some(("octo".into(), "repo".into())));
    }
}
