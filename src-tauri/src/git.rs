use crate::BridgeError;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub const CITIES: &[&str] = &[
    "Kyoto",
    "Lisbon",
    "Reykjavik",
    "Oslo",
    "Seoul",
    "Tallinn",
    "Nairobi",
    "Prague",
    "Jaipur",
    "Helsinki",
    "Medellin",
    "Valencia",
    "Taipei",
    "Dublin",
    "Zurich",
    "Austin",
    "Kigali",
    "Naples",
    "Vienna",
    "Busan",
];

pub fn validate_repo(path: &Path) -> Result<String, BridgeError> {
    let root = run(path, ["rev-parse", "--show-toplevel"])?;
    let canonical = std::fs::canonicalize(root.trim())?;
    if canonical != std::fs::canonicalize(path)? {
        return Err(BridgeError::Invalid(
            "Choose the repository root, not a subdirectory".into(),
        ));
    }
    Ok(canonical.to_string_lossy().to_string())
}
pub fn slug(value: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in value.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            dash = false
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true
        }
    }
    out.trim_matches('-').chars().take(42).collect()
}
pub fn create_worktree(repo: &Path, path: &Path, branch: &str) -> Result<(), BridgeError> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    run(
        repo,
        [
            "worktree",
            "add",
            "-b",
            branch,
            &path.to_string_lossy(),
            "HEAD",
        ],
    )?;
    Ok(())
}

pub fn remove_worktree(repo: &Path, path: &Path) -> Result<(), BridgeError> {
    run(repo, ["worktree", "remove", &path.to_string_lossy()])?;
    Ok(())
}
pub fn stats(path: &Path) -> Result<(i64, i64, i64), BridgeError> {
    let porcelain = run(path, ["status", "--porcelain"])?;
    let dirty = porcelain.lines().count() as i64;
    let diff = run(path, ["diff", "--numstat", "HEAD"])?;
    let mut adds = 0;
    let mut dels = 0;
    for l in diff.lines() {
        let p: Vec<_> = l.split('\t').collect();
        if p.len() > 1 {
            adds += p[0].parse::<i64>().unwrap_or(0);
            dels += p[1].parse::<i64>().unwrap_or(0)
        }
    }
    Ok((dirty, adds, dels))
}
fn run<'a, I>(cwd: &Path, args: I) -> Result<String, BridgeError>
where
    I: IntoIterator<Item = &'a str>,
{
    let output = Command::new("git").args(args).current_dir(cwd).output()?;
    if !output.status.success() {
        return Err(BridgeError::Git(
            String::from_utf8_lossy(&output.stderr).trim().into(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into())
}
pub fn workspace_path(base: &Path, project: &str, city: &str) -> PathBuf {
    base.join(slug(project)).join(city)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn creates_safe_slug() {
        assert_eq!(slug(" Add OAuth / callbacks! "), "add-oauth-callbacks");
    }
    #[test]
    fn city_pool_is_unique() {
        let mut v = CITIES.to_vec();
        v.sort();
        v.dedup();
        assert_eq!(v.len(), CITIES.len());
    }
    #[test]
    fn clean_worktree_can_be_created_and_archived() {
        let fixture = tempfile::tempdir().unwrap();
        let repo = fixture.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        let git = |args: &[&str]| {
            let status = Command::new("git")
                .args(args)
                .current_dir(&repo)
                .status()
                .unwrap();
            assert!(status.success(), "git command failed: {args:?}");
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "bridge-test@example.invalid"]);
        git(&["config", "user.name", "Bridge Test"]);
        git(&["commit", "--allow-empty", "-m", "fixture", "-q"]);
        let worktree = fixture.path().join("Kyoto");
        create_worktree(&repo, &worktree, "bridge/archive-test").unwrap();
        assert!(worktree.exists());
        remove_worktree(&repo, &worktree).unwrap();
        assert!(!worktree.exists());
    }
}
