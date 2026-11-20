//! Carry a provider thread to another account on the same app.
//!
//! Two accounts of one app read their threads from different homes, so a
//! switch between them used to hand over like a switch to another app: a
//! summary, a projection, a fresh thread. That is the switch a person makes
//! when one account runs out of quota, and it cost them the conversation even
//! though the thread was a file on the same disk the whole time.
//!
//! Claude Code keeps a thread at `<config dir>/projects/<folder>/<id>.jsonl`,
//! where the folder is named after the working directory. Codex keeps a
//! rollout under its home's `sessions/` (or `archived_sessions/`), filed by
//! date. Either way the thread is one file, and the new account resumes it if
//! the file is where its own home would have put it. OpenCode keeps threads in
//! a database of its own and is not carried.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::provider_accounts::{self, HostHomes, ProviderAccount};
use crate::BridgeError;

/// The longest thread id a carry accepts.
const MAX_THREAD_ID: usize = 128;

/// Where a thread lives under one account's home, and where it goes under
/// another's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Carry {
    pub thread: String,
    pub source: PathBuf,
    pub destination: PathBuf,
}

/// What [`copy`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Copied {
    /// The destination now holds the source's bytes.
    Written,
    /// The destination was already newer than the source: it has the thread,
    /// possibly further along, and is kept as it is.
    KeptNewer,
    /// Both paths are one file.
    Same,
}

/// The home an account of `harness` reads its threads from. `None` for a
/// harness whose threads are not files.
pub fn home(harness: &str, account: Option<&ProviderAccount>, host: &HostHomes) -> Option<PathBuf> {
    match harness {
        "claude" => Some(provider_accounts::claude_home(account, host)),
        "codex" => Some(provider_accounts::codex_home_layout(account, host).shared_home_path),
        _ => None,
    }
}

/// A provider thread id a carry will put in a path: letters, digits, `-` and
/// `_`, so it can never name a parent folder or another file.
pub fn valid_thread_id(thread: &str) -> bool {
    !thread.is_empty()
        && thread.len() <= MAX_THREAD_ID
        && thread.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Claude Code's folder for a working directory: every character that is not
/// a letter or a digit written as `-`.
fn claude_folder(cwd: &str) -> String {
    cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// Where Claude Code keeps `thread` under `home`: the working directory's
/// folder first, then any project folder, in name order.
fn find_claude(home: &Path, thread: &str, cwd: Option<&str>) -> Option<PathBuf> {
    let projects = home.join("projects");
    let file = format!("{thread}.jsonl");
    if let Some(cwd) = cwd.filter(|cwd| !cwd.is_empty()) {
        let candidate = projects.join(claude_folder(cwd)).join(&file);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let mut folders = fs::read_dir(&projects)
        .ok()?
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    folders.sort();
    folders.into_iter().map(|folder| folder.join(&file)).find(|candidate| candidate.is_file())
}

/// Where `thread` lives under `from` and where it would go under `to`, when
/// `harness` keeps threads as files and `from` still has this one.
pub fn locate(harness: &str, thread: &str, from: &Path, to: &Path, cwd: Option<&str>) -> Option<Carry> {
    if !valid_thread_id(thread) {
        return None;
    }
    let source = match harness {
        "claude" => find_claude(from, thread, cwd)?,
        "codex" => crate::codex_import::find_rollout(from, thread)?,
        _ => return None,
    };
    // The same place under the other home. Only plain names below the home:
    // nothing a path could use to climb out of it.
    let relative = source.strip_prefix(from).ok()?;
    if relative.as_os_str().is_empty() || !relative.components().all(|part| matches!(part, Component::Normal(_))) {
        return None;
    }
    Some(Carry { thread: thread.to_owned(), source: source.clone(), destination: to.join(relative) })
}

fn same_file(left: &Path, right: &Path) -> io::Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let (left, right) = (fs::metadata(left)?, fs::metadata(right)?);
        Ok(left.dev() == right.dev() && left.ino() == right.ino())
    }
    #[cfg(not(unix))]
    {
        Ok(fs::canonicalize(left)? == fs::canonicalize(right)?)
    }
}

/// Copy the thread's file into the other home: to a temp file beside the
/// destination, then renamed over it, so a reader never sees half a
/// transcript. A destination newer than the source is kept.
pub fn copy(carry: &Carry) -> Result<Copied, BridgeError> {
    let fail = |what: &str, path: &Path, error: io::Error| {
        BridgeError::Invalid(format!("could not {what} {}: {error}", path.display()))
    };
    let source = fs::metadata(&carry.source).map_err(|error| fail("read", &carry.source, error))?;
    if !source.is_file() {
        return Err(BridgeError::Invalid(format!("{} is not a file", carry.source.display())));
    }
    if let Ok(destination) = fs::metadata(&carry.destination) {
        if same_file(&carry.source, &carry.destination).unwrap_or(false) {
            return Ok(Copied::Same);
        }
        if let (Ok(kept), Ok(incoming)) = (destination.modified(), source.modified()) {
            if kept > incoming {
                return Ok(Copied::KeptNewer);
            }
        }
    }
    let folder = carry
        .destination
        .parent()
        .ok_or_else(|| BridgeError::Invalid(format!("{} has no folder", carry.destination.display())))?;
    fs::create_dir_all(folder).map_err(|error| fail("create", folder, error))?;
    let name = carry
        .destination
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temp = folder.join(format!(".{name}.bridge-carry-{}", uuid::Uuid::new_v4()));
    let written = fs::copy(&carry.source, &temp)
        .map_err(|error| fail("copy to", &temp, error))
        .and_then(|_| fs::rename(&temp, &carry.destination).map_err(|error| fail("move into", &carry.destination, error)));
    if let Err(error) = written {
        // Best effort: a temp file that will not go is named for this carry
        // and is never read as a transcript.
        if let Err(cleanup) = fs::remove_file(&temp) {
            if cleanup.kind() != io::ErrorKind::NotFound {
                return Err(BridgeError::Invalid(format!("{error}; and could not remove {}: {cleanup}", temp.display())));
            }
        }
        return Err(error);
    }
    Ok(Copied::Written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn a_claude_transcript_is_copied_to_the_same_project_folder() {
        let root = tempfile::tempdir().unwrap();
        let (own, personal) = (root.path().join(".claude"), root.path().join(".claude_personal"));
        let thread = "dfbeb894-c814-4f79-bc39-98b3ecc65774";
        write(&own.join("projects/-Users-ada-app/").join(format!("{thread}.jsonl")), "{\"turn\":1}\n");
        write(&own.join("projects/-Users-ada-other/unrelated.jsonl"), "x");
        let carry = locate("claude", thread, &own, &personal, Some("/Users/ada/app")).unwrap();
        assert_eq!(carry.destination, personal.join("projects/-Users-ada-app").join(format!("{thread}.jsonl")));
        assert_eq!(copy(&carry).unwrap(), Copied::Written);
        assert_eq!(fs::read_to_string(&carry.destination).unwrap(), "{\"turn\":1}\n");
        let leftovers = fs::read_dir(carry.destination.parent().unwrap()).unwrap().count();
        assert_eq!(leftovers, 1, "no temp file left behind");
        // Found without the working directory too, by scanning the projects.
        assert_eq!(locate("claude", thread, &own, &personal, None).unwrap(), carry);
        assert_eq!(locate("claude", thread, &own, &personal, Some("/elsewhere")).unwrap(), carry);
    }

    #[test]
    fn a_codex_rollout_keeps_its_relative_path() {
        let root = tempfile::tempdir().unwrap();
        let (work, home) = (root.path().join("codex-work"), root.path().join("codex-home"));
        let rollout = work.join("sessions/2026/11/19/rollout-2026-11-19T02-22-29-019a1b2c-3d4e.jsonl");
        write(&rollout, "{\"type\":\"session_meta\"}\n");
        let carry = locate("codex", "019a1b2c-3d4e", &work, &home, None).unwrap();
        assert_eq!(carry.destination, home.join("sessions/2026/11/19/rollout-2026-11-19T02-22-29-019a1b2c-3d4e.jsonl"));
        assert_eq!(copy(&carry).unwrap(), Copied::Written);
        assert!(carry.destination.is_file());
        assert!(locate("opencode", "019a1b2c-3d4e", &work, &home, None).is_none(), "OpenCode threads are not files");
    }

    #[test]
    fn a_newer_destination_is_kept_and_an_older_one_is_replaced() {
        let root = tempfile::tempdir().unwrap();
        let (from, to) = (root.path().join("a"), root.path().join("b"));
        write(&from.join("projects/p/t1.jsonl"), "source");
        write(&to.join("projects/p/t1.jsonl"), "further along");
        let carry = locate("claude", "t1", &from, &to, None).unwrap();
        let future = SystemTime::now() + Duration::from_secs(3600);
        fs::File::options().write(true).open(&carry.destination).unwrap().set_modified(future).unwrap();
        assert_eq!(copy(&carry).unwrap(), Copied::KeptNewer);
        assert_eq!(fs::read_to_string(&carry.destination).unwrap(), "further along");

        let past = SystemTime::now() - Duration::from_secs(3600);
        fs::File::options().write(true).open(&carry.destination).unwrap().set_modified(past).unwrap();
        assert_eq!(copy(&carry).unwrap(), Copied::Written);
        assert_eq!(fs::read_to_string(&carry.destination).unwrap(), "source");

        let same = Carry { thread: "t1".into(), source: carry.source.clone(), destination: carry.source.clone() };
        assert_eq!(copy(&same).unwrap(), Copied::Same);
    }

    #[test]
    fn a_thread_id_with_a_path_in_it_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let (from, to) = (root.path().join("a"), root.path().join("b"));
        write(&from.join("projects/p/x.jsonl"), "x");
        for bad in ["../x", "p/x", "", "x y", "x\0", ".", "..", &"a".repeat(MAX_THREAD_ID + 1)] {
            assert!(!valid_thread_id(bad), "{bad:?}");
            assert!(locate("claude", bad, &from, &to, None).is_none(), "{bad:?}");
        }
        assert!(valid_thread_id("ses_e225cb0acffeNNH7411PE2B689"));
    }

    #[test]
    fn a_missing_transcript_is_not_carried() {
        let root = tempfile::tempdir().unwrap();
        let (from, to) = (root.path().join("a"), root.path().join("b"));
        assert!(locate("claude", "gone", &from, &to, None).is_none(), "no projects folder at all");
        write(&from.join("projects/p/other.jsonl"), "x");
        assert!(locate("claude", "gone", &from, &to, None).is_none());
        assert!(locate("codex", "gone", &from, &to, None).is_none());
        let vanished = Carry { thread: "gone".into(), source: from.join("projects/p/gone.jsonl"), destination: to.join("projects/p/gone.jsonl") };
        assert!(copy(&vanished).is_err(), "a source deleted after planning fails the copy");
        assert!(!to.join("projects/p").exists() || fs::read_dir(to.join("projects/p")).unwrap().count() == 0);
    }

    #[test]
    fn homes_follow_the_account() {
        let host = HostHomes { home: PathBuf::from("/Users/ada"), claude_config_dir: None, codex_home: None, xdg_data_home: None };
        let personal = ProviderAccount { id: "personal".into(), label: "Personal".into(), home_path: "~/.claude_personal".into(), shadow_home_path: String::new() };
        assert_eq!(home("claude", None, &host), Some(PathBuf::from("/Users/ada/.claude")));
        assert_eq!(home("claude", Some(&personal), &host), Some(PathBuf::from("/Users/ada/.claude_personal")));
        assert_eq!(home("codex", None, &host), Some(PathBuf::from("/Users/ada/.codex")));
        assert_eq!(home("opencode", None, &host), None);
    }
}
