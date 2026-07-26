//! Workspace file access for the composer's `@file` mentions.
//!
//! Two surfaces use this module:
//!   * the autocomplete list (`list_files`) shown while the user types `@`, and
//!   * per-turn context injection (`mention_context`) which reads the files the
//!     user referenced and hands their contents to the adapter as trusted,
//!     application-owned context — never folded into the visible user message.
//!
//! All reads are confined to the session's workspace root. Paths that escape the
//! root via `..` or symlinks are rejected by [`resolve_within`].

use crate::BridgeError;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Cap on entries returned to the autocomplete list.
const MAX_FILES: usize = 5000;
/// Per-file byte ceiling for injected context (larger files are truncated).
const MAX_FILE_BYTES: usize = 256 * 1024;
/// Aggregate byte ceiling across all referenced files in a single turn.
const MAX_TOTAL_CONTEXT_BYTES: usize = 512 * 1024;

/// Directories skipped by the non-git fallback walk.
const IGNORED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".venv",
    "venv",
    "__pycache__",
    ".turbo",
    ".cache",
];

/// List workspace files relative to `root`.
///
/// Prefers `git ls-files` (tracked + untracked-but-not-ignored) so the list
/// matches what a developer thinks of as "the project", and falls back to a
/// bounded directory walk for non-git folders.
pub fn list_files(root: &Path) -> Result<Vec<String>, BridgeError> {
    if let Some(files) = git_tracked(root) {
        return Ok(files);
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out.dedup();
    out.truncate(MAX_FILES);
    Ok(out)
}

fn git_tracked(root: &Path) -> Option<Vec<String>> {
    let output = Command::new("git")
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let mut files: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .split('\0')
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect();
    files.sort();
    files.dedup();
    files.truncate(MAX_FILES);
    Some(files)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    if out.len() >= MAX_FILES {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        if out.len() >= MAX_FILES {
            return;
        }
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if file_type.is_dir() {
            if IGNORED_DIRS.contains(&name.as_ref()) {
                continue;
            }
            walk(root, &entry.path(), out);
        } else if file_type.is_file() {
            if let Ok(rel) = entry.path().strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
}

/// Resolve a workspace-relative `candidate` to an absolute path proven to live
/// inside `root`. Returns `None` on traversal, a missing file, or a non-file.
pub fn resolve_within(root: &Path, candidate: &str) -> Option<PathBuf> {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return None;
    }
    let root = root.canonicalize().ok()?;
    let resolved = root.join(candidate).canonicalize().ok()?;
    if !resolved.starts_with(&root) || !resolved.is_file() {
        return None;
    }
    Some(resolved)
}

/// Read a single referenced file's contents (bounded), for a UI preview.
pub fn read_file(root: &Path, candidate: &str) -> Result<String, BridgeError> {
    let path = resolve_within(root, candidate).ok_or_else(|| {
        BridgeError::Invalid(format!("File is not inside the workspace: {candidate}"))
    })?;
    read_bounded(&path)
}

fn read_bounded(path: &Path) -> Result<String, BridgeError> {
    let bytes = std::fs::read(path)?;
    let truncated = bytes.len() > MAX_FILE_BYTES;
    let slice = &bytes[..bytes.len().min(MAX_FILE_BYTES)];
    let mut text = String::from_utf8_lossy(slice).into_owned();
    if truncated {
        text.push_str("\n… [truncated]");
    }
    Ok(text)
}

/// Extract unique `@file` mention candidates from user text, in first-seen order.
///
/// An `@` only starts a mention at the beginning of the text or after
/// whitespace, so email-style `name@host` fragments are ignored.
pub fn extract_mentions(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'@' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && is_path_char(bytes[j]) {
                j += 1;
            }
            if j > start {
                // Path chars are ASCII, so `start..j` is a valid char boundary.
                let token = text[start..j].trim_end_matches(['.', ',', ';', ':']);
                if !token.is_empty() && !out.iter().any(|existing| existing == token) {
                    out.push(token.to_string());
                }
            }
            i = j;
            continue;
        }
        i += 1;
    }
    out
}

fn is_path_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/')
}

/// Build a trusted application-context block for the `@file` mentions in `text`.
/// Returns `None` when no mention resolves to a real file under `root`.
pub fn mention_context(root: &Path, text: &str) -> Option<String> {
    let mentions = extract_mentions(text);
    if mentions.is_empty() {
        return None;
    }
    let mut sections: Vec<String> = Vec::new();
    let mut total = 0usize;
    for mention in mentions {
        let Some(path) = resolve_within(root, &mention) else {
            continue;
        };
        let Ok(contents) = read_bounded(&path) else {
            continue;
        };
        if total + contents.len() > MAX_TOTAL_CONTEXT_BYTES {
            break;
        }
        total += contents.len();
        sections.push(format!("===== {mention} =====\n{contents}"));
    }
    if sections.is_empty() {
        return None;
    }
    Some(format!(
        "The user referenced the following workspace files with @-mentions. \
         Their current contents are provided for context:\n\n{}",
        sections.join("\n\n")
    ))
}

/// Merge the credential-broker context and the file-mention context into a
/// single trusted application-context block.
pub fn merge_context(credential: Option<String>, files: Option<String>) -> Option<String> {
    match (credential, files) {
        (Some(credential), Some(files)) => Some(format!("{credential}\n\n{files}")),
        (Some(credential), None) => Some(credential),
        (None, files) => files,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_mentions_after_whitespace_only() {
        let found = extract_mentions("look at @src/App.tsx and email me@host.com then @Cargo.toml.");
        assert_eq!(found, vec!["src/App.tsx".to_string(), "Cargo.toml".to_string()]);
    }

    #[test]
    fn dedupes_mentions() {
        let found = extract_mentions("@a/b.rs and again @a/b.rs");
        assert_eq!(found, vec!["a/b.rs".to_string()]);
    }

    #[test]
    fn resolve_rejects_traversal() {
        let dir = std::env::temp_dir();
        assert!(resolve_within(&dir, "../etc/passwd").is_none());
    }

    #[test]
    fn merge_prefers_both() {
        assert_eq!(
            merge_context(Some("cred".into()), Some("files".into())),
            Some("cred\n\nfiles".into())
        );
        assert_eq!(merge_context(None, Some("files".into())), Some("files".into()));
        assert_eq!(merge_context(None, None), None);
    }
}
