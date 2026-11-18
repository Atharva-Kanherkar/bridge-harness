//! Which tools on this machine Bridge can import from, and how much each holds.
//!
//! Read-only and cheap: it lists folders and counts files. It never opens a
//! database, never reads a transcript, and never reads a credential file, so
//! it is safe to run on every visit to the import screen.

use bridge_protocol::messages as wire;
use std::fs;
use std::path::{Path, PathBuf};

/// Where every known tool keeps its data, relative to the home folder.
struct KnownTool {
    tool: &'static str,
    label: &'static str,
    supported: bool,
    /// Folders whose existence means the tool was used here.
    markers: &'static [&'static str],
    /// Folders of worktrees the tool created (`<root>/<repo>/<workspace>`).
    worktree_roots: &'static [&'static str],
    note: &'static str,
}

const OTHER_TOOLS: &[KnownTool] = &[
    KnownTool {
        tool: "codex",
        label: "Codex",
        supported: false,
        markers: &[".codex/sessions"],
        worktree_roots: &[],
        note: "Codex chats are next. Nothing is read until then.",
    },
    KnownTool {
        tool: "opencode",
        label: "OpenCode",
        supported: false,
        markers: &[".local/share/opencode/opencode.db"],
        worktree_roots: &[],
        note: "OpenCode chats are coming. Nothing is read until then.",
    },
    KnownTool {
        tool: "conductor",
        label: "Conductor",
        supported: false,
        markers: &["Library/Application Support/com.conductor.app", "conductor/workspaces"],
        worktree_roots: &["conductor/workspaces"],
        note: "Conductor runs Claude Code, so its chats come over with the Claude Code import. Its workspace names and scripts are coming.",
    },
    KnownTool {
        tool: "t3code",
        label: "T3 Code",
        supported: false,
        markers: &[".t3/userdata"],
        worktree_roots: &[".t3/worktrees"],
        note: "T3 Code runs Claude Code, so its Claude chats come over with the Claude Code import. Thread titles and projects are coming.",
    },
    KnownTool {
        tool: "orca",
        label: "Orca",
        supported: false,
        markers: &["Library/Application Support/Orca", "orca/workspaces"],
        worktree_roots: &["orca/workspaces"],
        note: "Orca's Claude Code chats come over with the Claude Code import. Its projects and multi-agent runs are coming.",
    },
    KnownTool {
        tool: "superset",
        label: "Superset",
        supported: false,
        markers: &[".paradev", ".superset"],
        worktree_roots: &[".paradev/worktrees", ".superset/worktrees"],
        note: "Superset's Claude Code chats come over with the Claude Code import. Its workspaces and agent settings are coming.",
    },
];

fn subdirs(path: &Path) -> Vec<PathBuf> {
    fs::read_dir(path)
        .map(|read| {
            read.flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

fn files_with(path: &Path, extension: &str) -> u64 {
    fs::read_dir(path)
        .map(|read| {
            read.flatten()
                .filter(|entry| {
                    entry.path().extension().and_then(|ext| ext.to_str()) == Some(extension)
                        && entry.path().is_file()
                })
                .count() as u64
        })
        .unwrap_or(0)
}

/// Count rollout files under a date tree (`YYYY/MM/DD/*.jsonl`) without reading them.
fn count_jsonl_tree(path: &Path, depth: usize) -> u64 {
    if depth == 0 {
        return files_with(path, "jsonl");
    }
    subdirs(path)
        .iter()
        .map(|child| count_jsonl_tree(child, depth - 1))
        .sum::<u64>()
        + files_with(path, "jsonl")
}

fn claude(
    home: &Path,
    config_dir: Option<&Path>,
    accounts: &[PathBuf],
) -> wire::DetectedImportSource {
    let roots = crate::claude_import::config_roots(home, config_dir, accounts);
    let mut counts = wire::ImportSourceCounts::default();
    for root in &roots {
        for project in subdirs(&root.join("projects")) {
            let chats = files_with(&project, "jsonl");
            if is_temp_project(&project) {
                counts.background += chats;
                continue;
            }
            counts.chats += chats;
            if chats > 0 {
                counts.projects += 1;
            }
            counts.memories += files_with(&project.join("memory"), "md");
        }
        counts.setup += ["CLAUDE.md", "settings.json"]
            .iter()
            .filter(|name| root.join(name).is_file())
            .count() as u64;
        for folder in ["agents", "commands", "skills", "rules"] {
            counts.setup += fs::read_dir(root.join(folder))
                .map(|read| read.flatten().count() as u64)
                .unwrap_or(0);
        }
    }
    wire::DetectedImportSource {
        tool: crate::claude_import::PROVIDER.into(),
        label: "Claude Code".into(),
        installed: !roots.is_empty(),
        supported: true,
        roots: roots.iter().map(|root| root.to_string_lossy().into_owned()).collect(),
        counts,
        note: Some(
            "Chats with their tool calls, thinking, images and subagents, plus memory and setup. Chats Claude Code ran for Conductor, T3 Code, Orca and Superset come over too."
                .into(),
        ),
    }
}

fn other(home: &Path, known: &KnownTool) -> wire::DetectedImportSource {
    let roots: Vec<PathBuf> = known
        .markers
        .iter()
        .map(|marker| home.join(marker))
        .filter(|path| path.exists())
        .collect();
    let mut counts = wire::ImportSourceCounts::default();
    if known.tool == "codex" {
        counts.chats = count_jsonl_tree(&home.join(".codex/sessions"), 3)
            + count_jsonl_tree(&home.join(".codex/archived_sessions"), 0);
    }
    for root in known.worktree_roots {
        for repo in subdirs(&home.join(root)) {
            counts.projects += 1;
            counts.workspaces += subdirs(&repo).len() as u64;
        }
    }
    wire::DetectedImportSource {
        tool: known.tool.into(),
        label: known.label.into(),
        installed: !roots.is_empty(),
        supported: known.supported,
        roots: roots
            .iter()
            .map(|root| root.to_string_lossy().into_owned())
            .collect(),
        counts,
        note: Some(known.note.into()),
    }
}

/// Claude names a project folder after its path with `/` as `-`. Sessions
/// that ran under the OS temp folder are SDK probes and background runs.
pub(crate) fn is_temp_project(project: &Path) -> bool {
    project
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            ["-private-var-folders-", "-var-folders-", "-private-tmp-", "-tmp-"]
                .iter()
                .any(|prefix| name.starts_with(prefix))
        })
}

/// Every known tool, Claude Code first, then the rest in a fixed order.
pub fn detect(
    home: &Path,
    claude_config_dir: Option<&Path>,
    accounts: &[PathBuf],
) -> Vec<wire::DetectedImportSource> {
    std::iter::once(claude(home, claude_config_dir, accounts))
        .chain(OTHER_TOOLS.iter().map(|known| other(home, known)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "{}").unwrap();
    }

    #[test]
    fn a_fresh_machine_lists_every_tool_as_not_found() {
        let home = tempfile::tempdir().unwrap();
        let sources = detect(home.path(), None, &[]);
        assert_eq!(
            sources.iter().map(|s| s.tool.as_str()).collect::<Vec<_>>(),
            vec![
                "claude_code",
                "codex",
                "opencode",
                "conductor",
                "t3code",
                "orca",
                "superset"
            ]
        );
        assert!(sources.iter().all(|source| !source.installed));
        assert!(sources[0].supported && sources.iter().skip(1).all(|source| !source.supported));
    }

    #[test]
    fn counts_come_from_file_listings_across_every_claude_folder() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        touch(&h.join(".claude/projects/-a/1.jsonl"));
        touch(&h.join(".claude/projects/-a/2.jsonl"));
        touch(&h.join(".claude/projects/-a/1/subagents/agent-x.jsonl"));
        touch(&h.join(".claude/projects/-a/memory/MEMORY.md"));
        touch(&h.join(".claude/projects/-empty/notes.txt"));
        touch(&h.join(".claude/projects/-private-var-folders-h7-T/probe.jsonl"));
        touch(&h.join(".claude/projects/-tmp-scratch/probe.jsonl"));
        touch(&h.join(".claude/CLAUDE.md"));
        touch(&h.join(".claude/agents/reviewer.md"));
        let account = h.join("accounts/work");
        touch(&account.join("projects/-b/3.jsonl"));
        let claude = &detect(h, None, &[account])[0];
        assert!(claude.installed);
        assert_eq!(claude.roots.len(), 2);
        assert_eq!(
            (
                claude.counts.chats,
                claude.counts.projects,
                claude.counts.memories,
                claude.counts.setup
            ),
            (3, 2, 1, 2)
        );
    }

    #[test]
    fn ades_are_found_by_their_folders_and_count_workspaces() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        fs::create_dir_all(h.join("conductor/workspaces/repo-a/tokyo")).unwrap();
        fs::create_dir_all(h.join("conductor/workspaces/repo-a/paris")).unwrap();
        fs::create_dir_all(h.join("conductor/workspaces/repo-b/lima")).unwrap();
        fs::create_dir_all(h.join(".t3/userdata")).unwrap();
        touch(&h.join(".codex/sessions/2026/11/17/rollout-1.jsonl"));
        touch(&h.join(".codex/archived_sessions/rollout-0.jsonl"));
        let sources = detect(h, None, &[]);
        let by = |tool: &str| sources.iter().find(|s| s.tool == tool).unwrap().clone();
        let conductor = by("conductor");
        assert!(conductor.installed);
        assert_eq!(
            (conductor.counts.projects, conductor.counts.workspaces),
            (2, 3)
        );
        assert!(by("t3code").installed);
        assert_eq!(by("codex").counts.chats, 2);
        assert!(!by("orca").installed);
    }
}

#[cfg(test)]
mod batch_tests {
    use crate::external_import::NormalizedImportCandidate;
    use crate::import_bundle::tests::{chat, normalize, rich_entries, source};
    use crate::import_bundle::{lower, resolve, TransferBundle};
    use std::collections::HashSet;

    /// 60 top-level chats, the first with two subagent children: batches stay
    /// near 25, a parent and its children are never split, and nothing is lost.
    #[test]
    fn batches_keep_families_together_and_cover_every_selected_chat() {
        let mut bundle = TransferBundle::new(source("claude_code"));
        for index in 0..60 {
            bundle.chats.push(chat(&format!("c{index}"), "claude", Some(&format!("s{index}")), rich_entries()));
        }
        for kid in ["k1", "k2"] {
            let mut child = chat(kid, "claude", Some(kid), rich_entries());
            child.parent_chat_key = Some("c0".into());
            bundle.chats.push(child);
        }
        let lowered = lower(&resolve(vec![bundle]).unwrap());
        let normalized: Vec<NormalizedImportCandidate> = normalize(&lowered.candidates);
        let selected: HashSet<&str> = normalized.iter().map(|c| c.candidate.candidate_id.as_str()).collect();
        let batches = crate::api::import_batches(&normalized, &selected);
        let flat: Vec<&String> = batches.iter().flatten().collect();
        assert_eq!(flat.len(), 62);
        assert!(batches.iter().all(|batch| batch.len() <= 25));
        let parent = &normalized[0].candidate.candidate_id;
        let family = batches.iter().find(|batch| batch.contains(parent)).unwrap();
        let children: Vec<_> = normalized
            .iter()
            .filter(|c| c.candidate.normalized_payload["parentCandidateId"].as_str() == Some(parent))
            .collect();
        assert_eq!(children.len(), 2);
        assert!(children.iter().all(|c| family.contains(&c.candidate.candidate_id)));
    }
}
