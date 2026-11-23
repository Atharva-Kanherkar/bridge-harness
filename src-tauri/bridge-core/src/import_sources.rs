//! Which tools on this machine Bridge can import from, and how much each holds.
//!
//! Read-only and cheap: it lists folders and counts files, and reads only
//! OpenCode's session table (opened read-only). It never reads a transcript
//! or a message, and never reads a credential file or table, so it is safe to
//! run on every visit to the import screen.

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
    bridge_data: &[PathBuf],
) -> wire::DetectedImportSource {
    let roots = crate::claude_import::config_roots(home, config_dir, accounts);
    let mut counts = wire::ImportSourceCounts::default();
    // A project used from two accounts is one project.
    let mut projects = std::collections::HashSet::new();
    for root in &roots {
        for project in subdirs(&root.join("projects")) {
            let chats = files_with(&project, "jsonl");
            let bridge_run = project
                .file_name()
                .is_some_and(|slug| crate::import_placement::is_bridge_claude_project(&slug.to_string_lossy(), bridge_data));
            if is_temp_project(&project) || bridge_run {
                counts.background += chats;
                continue;
            }
            counts.chats += chats;
            if chats > 0 {
                projects.insert(project.file_name().map(std::ffi::OsStr::to_os_string));
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
    counts.projects = projects.len() as u64;
    wire::DetectedImportSource {
        tool: crate::claude_import::PROVIDER.into(),
        label: "Claude Code".into(),
        installed: !roots.is_empty(),
        supported: true,
        roots: roots.iter().map(|root| root.to_string_lossy().into_owned()).collect(),
        counts,
        note: Some(
            "Chats with their tool calls, thinking, images and subagents, plus setup. Claude Code keeps using its own memory. Chats Claude Code ran for Conductor, T3 Code, Orca and Superset come over too."
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

/// The account folders added in Settings, per tool: Claude config folders,
/// Codex homes, and OpenCode data folders (`<data home>/opencode`).
#[derive(Debug, Clone, Default)]
pub struct AccountFolders {
    pub claude: Vec<PathBuf>,
    pub codex: Vec<PathBuf>,
    pub opencode: Vec<PathBuf>,
    /// Bridge's own data folders: chats run there are Bridge's own runs,
    /// counted as background runs, not chats.
    pub bridge_data: Vec<PathBuf>,
}

/// Every known tool, Claude Code first, then the rest in a fixed order.
pub fn detect(
    home: &Path,
    claude_config_dir: Option<&Path>,
    accounts: &[PathBuf],
) -> Vec<wire::DetectedImportSource> {
    let accounts = AccountFolders { claude: accounts.to_vec(), ..AccountFolders::default() };
    detect_with(home, claude_config_dir, &accounts, None, None)
}

/// [`detect`], with the Codex home named (`CODEX_HOME`) instead of
/// `~/.codex`, the data home (`XDG_DATA_HOME`) instead of `~/.local/share`,
/// and each tool's account folders.
pub fn detect_with(
    home: &Path,
    claude_config_dir: Option<&Path>,
    accounts: &AccountFolders,
    codex_home: Option<&Path>,
    xdg_data_home: Option<&Path>,
) -> Vec<wire::DetectedImportSource> {
    [
        claude(home, claude_config_dir, &accounts.claude, &accounts.bridge_data),
        codex(home, codex_home, &accounts.codex),
        opencode(home, xdg_data_home, &accounts.opencode),
        conductor(home),
        t3code(&crate::t3code_import::Paths::from_env(home)),
    ]
    .into_iter()
        .chain(OTHER_TOOLS.iter().map(|known| other(home, known)))
        .collect()
}

/// Codex: each rollout is a chat, counted from its first line only. One that
/// ran in the OS temp folder is a background run; a subagent's thread comes
/// over under its parent and is not counted on its own.
fn codex(home: &Path, codex_home: Option<&Path>, accounts: &[PathBuf]) -> wire::DetectedImportSource {
    let own = codex_home.map(Path::to_path_buf).unwrap_or_else(|| home.join(".codex"));
    let roots = crate::codex_import::existing_folders(std::iter::once(own).chain(accounts.iter().cloned()), |root| {
        root.join("sessions").is_dir() || root.join("archived_sessions").is_dir()
    });
    let installed = !roots.is_empty();
    let mut counts = wire::ImportSourceCounts::default();
    let mut folders = std::collections::HashSet::new();
    for root in &roots {
        for path in crate::codex_import::list_rollouts(root) {
            let meta = crate::codex_import::rollout::read_meta(&path).unwrap_or_default();
            let cwd = meta.cwd.unwrap_or_default();
            if crate::codex_import::is_temp_folder(&cwd) {
                counts.background += 1;
            } else if meta.parent_thread_id.is_none() {
                counts.chats += 1;
                folders.insert(cwd);
            }
        }
    }
    counts.projects = folders.len() as u64;
    wire::DetectedImportSource {
        tool: crate::codex_import::PROVIDER.into(),
        label: "Codex".into(),
        installed,
        supported: true,
        roots: roots.iter().map(|root| root.to_string_lossy().into_owned()).collect(),
        counts,
        note: Some("Chats with their commands, edits and MCP calls. Codex keeps using its own memory.".into()),
    }
}

/// OpenCode: each session in its store is a chat, counted from the session
/// table only. One that ran in the OS temp folder or in Bridge's title
/// scratch folder is a background run; a subagent's session comes over under
/// its parent and is not counted on its own.
fn opencode(home: &Path, xdg_data_home: Option<&Path>, accounts: &[PathBuf]) -> wire::DetectedImportSource {
    let data_home = xdg_data_home.map_or_else(|| home.join(".local/share"), Path::to_path_buf);
    let own = crate::opencode_import::data_folder(&data_home);
    let roots = crate::codex_import::existing_folders(std::iter::once(own).chain(accounts.iter().cloned()), |folder| {
        folder.join(crate::opencode_import::STORE_FILE).is_file()
    });
    let installed = !roots.is_empty();
    let mut counts = wire::ImportSourceCounts::default();
    let mut projects = std::collections::HashSet::new();
    for root in &roots {
        if let Some(found) = crate::opencode_import::count_sessions(&root.join(crate::opencode_import::STORE_FILE)) {
            counts.chats += found.chats;
            counts.background += found.background;
            projects.extend(found.folders);
        }
    }
    counts.projects = projects.len() as u64;
    wire::DetectedImportSource {
        tool: crate::opencode_import::PROVIDER.into(),
        label: "OpenCode".into(),
        installed,
        supported: true,
        roots: roots.iter().map(|root| root.to_string_lossy().into_owned()).collect(),
        counts,
        note: Some("Chats with their commands, edits and subagents. OpenCode keeps its own memory and setup.".into()),
    }
}

/// Conductor: its workspaces, from its database when Bridge knows the
/// version, else from the folders under `~/conductor/workspaces`. Archived
/// workspaces are not counted. Reads the workspace tables only, never a
/// message or a secret.
fn conductor(home: &Path) -> wire::DetectedImportSource {
    let paths = crate::conductor_import::Paths::for_home(home);
    let installed = paths.installed();
    let mut counts = wire::ImportSourceCounts::default();
    if installed {
        let discovery = crate::conductor_import::discover(&paths);
        let active: Vec<_> = discovery.candidates.iter().filter(|candidate| !candidate.archived).collect();
        counts.workspaces = active.len() as u64;
        counts.projects = discovery.repos as u64;
        counts.chats = discovery.chats.values().sum();
    }
    wire::DetectedImportSource {
        tool: crate::conductor_import::PROVIDER.into(),
        label: "Conductor".into(),
        installed,
        supported: true,
        roots: if installed { vec![home.join("conductor").to_string_lossy().into_owned()] } else { Vec::new() },
        counts,
        note: Some("Workspaces come over as Bridge workspaces, left where they are, so Conductor keeps working with them. Their chats come over with the Claude Code and Codex imports.".into()),
    }
}

/// T3 Code: its threads and the worktrees they ran in, from its database
/// (opened read-only; projects and threads only, never a message or a
/// secret). Deleted threads and subagents' threads are not counted, nor
/// worktrees whose threads are all archived.
fn t3code(paths: &crate::t3code_import::Paths) -> wire::DetectedImportSource {
    let installed = paths.installed();
    let mut counts = wire::ImportSourceCounts::default();
    let mut note = "Threads come over as chats, with their full Claude Code, Codex or OpenCode history, or T3 Code's own copy when that is gone. Worktrees come over as Bridge workspaces, left where they are, so T3 Code keeps working with them.".to_owned();
    if installed {
        match crate::t3code_import::read(paths) {
            Ok((store, _)) => {
                let found = crate::t3code_import::counts(&store);
                counts.chats = found.chats;
                counts.projects = found.projects;
                counts.workspaces = found.workspaces;
            }
            Err(reason) => note = reason,
        }
    }
    wire::DetectedImportSource {
        tool: crate::t3code_import::PROVIDER.into(),
        label: "T3 Code".into(),
        installed,
        supported: true,
        roots: if installed { vec![paths.base.to_string_lossy().into_owned()] } else { Vec::new() },
        counts,
        note: Some(note),
    }
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
        assert!(sources.iter().take(5).all(|source| source.supported), "Claude Code, Codex, OpenCode, Conductor and T3 Code import today");
        assert!(sources.iter().skip(5).all(|source| !source.supported));
    }

    #[test]
    fn chats_bridge_ran_itself_count_as_background_runs() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        let data = PathBuf::from("/Users/me/Library/Application Support/dev.bridge.deck");
        let slug = crate::import_placement::claude_slug(&data.join("chats/0a1b"));
        touch(&h.join(format!(".claude/projects/{slug}/slack.jsonl")));
        touch(&h.join(format!(".claude/projects/{slug}/slack2.jsonl")));
        touch(&h.join(".claude/projects/-Users-me-code-app/real.jsonl"));
        let accounts = AccountFolders { bridge_data: vec![data], ..AccountFolders::default() };
        let claude = detect_with(h, None, &accounts, None, None).into_iter().find(|source| source.tool == "claude_code").unwrap();
        assert_eq!((claude.counts.chats, claude.counts.projects, claude.counts.background), (1, 1, 2));
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
    fn codex_counts_chats_from_first_lines_leaving_background_runs_and_subagents_out() {
        let home = tempfile::tempdir().unwrap();
        let codex = home.path().join(".codex");
        let rollout = |rel: &str, cwd: &str, parent: Option<&str>| {
            let source = parent.map_or(serde_json::json!("exec"), |parent| serde_json::json!({ "subagent": { "thread_spawn": { "parent_thread_id": parent } } }));
            let line = serde_json::json!({ "timestamp": "t", "type": "session_meta", "payload": { "id": rel, "cwd": cwd, "source": source } });
            let path = codex.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, format!("{line}\n")).unwrap();
        };
        rollout("sessions/2026/11/01/rollout-a.jsonl", "/work/app", None);
        rollout("sessions/2026/11/01/rollout-b.jsonl", "/work/api", None);
        rollout("sessions/2026/11/01/rollout-c.jsonl", "/work/app", Some("a"));
        rollout("sessions/2026/11/02/rollout-d.jsonl", "/private/var/folders/xy/T/probe", None);
        rollout("archived_sessions/rollout-e.jsonl", "/work/app", None);

        let codex = detect(home.path(), None, &[]).into_iter().find(|source| source.tool == "codex").unwrap();

        assert!(codex.installed && codex.supported);
        assert_eq!((codex.counts.chats, codex.counts.projects, codex.counts.background), (3, 2, 1));
    }

    #[test]
    fn opencode_counts_chats_from_its_session_table_leaving_background_runs_and_subagents_out() {
        let home = tempfile::tempdir().unwrap();
        let fixture = crate::opencode_import::fixture::Fixture::new(&home.path().join(".local/share"));
        fixture.session("ses_a", None, "/work/app", Some("a"), 1);
        fixture.session("ses_b", None, "/work/api", Some("b"), 2);
        fixture.session("ses_c", Some("ses_a"), "/work/app", Some("c"), 3);
        fixture.session("ses_d", None, "/private/var/folders/xy/T/probe", Some("d"), 4);
        fixture.v1_session("ses_e", None, "/work/app", "e", 5);

        let opencode = detect(home.path(), None, &[]).into_iter().find(|source| source.tool == "opencode").unwrap();

        assert!(opencode.installed && opencode.supported);
        assert_eq!((opencode.counts.chats, opencode.counts.projects, opencode.counts.background), (3, 2, 1));
        assert_eq!(opencode.roots, vec![fixture.home.to_string_lossy().into_owned()]);
    }

    #[test]
    fn opencode_follows_the_data_home() {
        let home = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let fixture = crate::opencode_import::fixture::Fixture::new(data.path());
        fixture.session("ses_a", None, "/work/app", Some("a"), 1);

        let sources = detect_with(home.path(), None, &AccountFolders::default(), None, Some(data.path()));
        let opencode = sources.iter().find(|source| source.tool == "opencode").unwrap();

        assert_eq!((opencode.installed, opencode.counts.chats), (true, 1));
    }

    #[test]
    fn account_folders_are_counted_with_your_own() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        let rollout = |codex: &Path, id: &str, cwd: &str| {
            let line = serde_json::json!({ "timestamp": "t", "type": "session_meta", "payload": { "id": id, "cwd": cwd, "source": "exec" } });
            let path = codex.join(format!("sessions/2026/11/01/rollout-{id}.jsonl"));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, format!("{line}\n")).unwrap();
        };
        rollout(&h.join(".codex"), "a", "/work/app");
        rollout(&h.join("codex-work"), "b", "/work/api");
        let own = crate::opencode_import::fixture::Fixture::new(&h.join(".local/share"));
        own.session("ses_a", None, "/work/app", Some("a"), 1);
        let work = crate::opencode_import::fixture::Fixture::new(&h.join("opencode-work"));
        work.session("ses_b", None, "/work/app", Some("b"), 2);
        work.session("ses_c", None, "/work/web", Some("c"), 3);
        touch(&h.join(".claude/projects/-a/1.jsonl"));
        touch(&h.join("claude-work/projects/-b/2.jsonl"));
        touch(&h.join("claude-work/projects/-a/3.jsonl"));
        let accounts = AccountFolders {
            claude: vec![h.join("claude-work"), h.join(".claude")],
            codex: vec![h.join("codex-work"), h.join("codex-missing")],
            opencode: vec![work.home.clone()],
            ..AccountFolders::default()
        };

        let sources = detect_with(h, None, &accounts, None, None);
        let by = |tool: &str| sources.iter().find(|source| source.tool == tool).unwrap().clone();

        let claude = by("claude_code");
        assert_eq!((claude.counts.chats, claude.counts.projects, claude.roots.len()), (3, 2, 2), "your own folder listed once; a project in both folders is one project");
        let codex = by("codex");
        assert_eq!((codex.counts.chats, codex.counts.projects, codex.roots.len()), (2, 2, 2), "a missing account home is skipped");
        let opencode = by("opencode");
        assert_eq!((opencode.counts.chats, opencode.counts.projects, opencode.roots.len()), (3, 2, 2), "a folder in two stores is one project");
    }

    #[test]
    fn ades_are_found_by_their_folders_and_count_workspaces() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        fs::create_dir_all(h.join("conductor/workspaces/repo-a/tokyo")).unwrap();
        fs::create_dir_all(h.join("conductor/workspaces/repo-a/paris")).unwrap();
        fs::create_dir_all(h.join("conductor/workspaces/repo-b/lima")).unwrap();
        touch(&h.join(".t3/userdata/state.sqlite"));
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

    #[test]
    fn conductor_counts_come_from_its_database_without_archived_workspaces() {
        use crate::conductor_import::fixture::{Fixture, Row};
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        let fixture = Fixture::at(&crate::conductor_import::Paths::for_home(h).store, 181);
        fixture.repo("r", "app", &h.join("code/app"), 3);
        fixture.workspace(&Row { key: "a", repo: "r", directory: "tokyo", branch: Some("a"), state: "active", ..Row::default() });
        fixture.workspace(&Row { key: "b", repo: "r", directory: "oslo", branch: Some("b"), state: "archived", ..Row::default() });
        fixture.session("s1", "a", "claude");

        let conductor = detect(h, None, &[]).into_iter().find(|source| source.tool == "conductor").unwrap();

        assert!(conductor.installed && conductor.supported);
        assert_eq!((conductor.counts.workspaces, conductor.counts.projects, conductor.counts.chats), (1, 1, 1));
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
