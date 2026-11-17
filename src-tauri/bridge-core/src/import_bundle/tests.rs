use super::*;
use crate::external_import::{
    commit_import, ConflictPolicy, ImportPlan, NormalizedImportCandidate,
    SetupActivationPolicy,
};
use proptest::prelude::*;
use rusqlite::{params, Connection};

pub(crate) fn source(tool: &str) -> BundleSource {
    BundleSource {
        tool: tool.into(),
        tool_version: Some("1.0.0".into()),
        schema_version: Some("fixture-v1".into()),
        adapter_version: "1".into(),
        roots: vec![format!("/fixture/{tool}")],
        detected_at: "2026-11-17T00:00:00Z".into(),
    }
}

pub(crate) fn entry(id: &str, kind: BundleEntryKind, text: &str) -> BundleEntry {
    BundleEntry {
        source_id: id.into(),
        at: format!("2026-11-01T10:00:{:02}Z", id.len() % 60),
        kind,
        text: Some(text.into()),
        tool: None,
        attachments: Vec::new(),
    }
}

pub(crate) fn tool_entry(id: &str, name: &str, output: &str, status: ToolStatus) -> BundleEntry {
    BundleEntry {
        source_id: id.into(),
        at: "2026-11-01T10:00:30Z".into(),
        kind: BundleEntryKind::ToolCall,
        text: None,
        tool: Some(BundleTool {
            call_id: format!("call-{id}"),
            name: name.into(),
            input: json!({ "command": "ls -la" }),
            output: Some(output.into()),
            status,
        }),
        attachments: Vec::new(),
    }
}

pub(crate) fn chat(key: &str, harness: &str, native: Option<&str>, entries: Vec<BundleEntry>) -> BundleChat {
    BundleChat {
        key: key.into(),
        workspace_key: None,
        cwd: Some("/tmp/bridge-import-fixture/project".into()),
        title: None,
        title_source: TitleSource::Derived,
        created_at: Some("2026-11-01T10:00:00Z".into()),
        updated_at: Some("2026-11-01T11:00:00Z".into()),
        archived: false,
        pinned: false,
        harness: harness.into(),
        model: Some("fixture-model".into()),
        effort: None,
        native_ref: native.map(|id| NativeRef {
            harness: harness.into(),
            session_id: id.into(),
            config_root: None,
            transcript_path: None,
        }),
        parent_chat_key: None,
        agent_role: None,
        entries_source: EntriesSource::Native,
        entries,
        pull_requests: Vec::new(),
        source_ref: format!("/fixture/{harness}/{key}.jsonl"),
        source_label: format!("{key}.jsonl"),
    }
}

pub(crate) fn rich_entries() -> Vec<BundleEntry> {
    let mut image = entry("u2", BundleEntryKind::UserMessage, "");
    image.attachments.push(BundleAttachment {
        media_type: "image/png".into(),
        name: Some("shot.png".into()),
        data_base64: Some("iVBORw0KGgo=".into()),
        bytes: 8,
    });
    vec![
        entry("u1", BundleEntryKind::UserMessage, "Fix the failing login test"),
        entry("r1", BundleEntryKind::Reasoning, "The test mocks the clock; check the token expiry."),
        tool_entry("t1", "Bash", "ok\n3 passed", ToolStatus::Completed),
        tool_entry("t2", "Read", "no such file", ToolStatus::Failed),
        entry("a1", BundleEntryKind::AssistantMessage, "Fixed: the expiry used local time."),
        image,
        entry("c1", BundleEntryKind::CompactionBoundary, "Summary of the first hour"),
        entry("a2", BundleEntryKind::AssistantMessage, "That screenshot shows the old banner."),
    ]
}

pub(crate) fn database() -> Connection {
    crate::store::open(std::path::Path::new(":memory:")).unwrap()
}

pub(crate) fn normalize(candidates: &[ImportCandidate]) -> Vec<NormalizedImportCandidate> {
    candidates
        .iter()
        .map(|candidate| NormalizedImportCandidate {
            candidate: candidate.clone(),
            deterministic_bridge_id: candidate.candidate_id.clone(),
        })
        .collect()
}

pub(crate) fn plan_for(candidates: &[ImportCandidate], policy: ConflictPolicy) -> ImportPlan {
    ImportPlan {
        selected_candidate_ids: candidates.iter().map(|c| c.candidate_id.clone()).collect(),
        conflict_policy: policy,
        setup_activation_policy: SetupActivationPolicy::Disabled,
        memory_scope: Some("account:local".into()),
        dry_run: false,
        created_at: "2026-11-17T00:00:00Z".into(),
    }
}

pub(crate) fn count(db: &Connection, sql: &str) -> i64 {
    db.query_row(sql, [], |row| row.get(0)).unwrap()
}

fn import(db: &Connection, bundles: Vec<TransferBundle>) -> crate::external_import::ImportCommit {
    let lowered = lower(&resolve(bundles).unwrap());
    let normalized = normalize(&lowered.candidates);
    commit_import(db, &normalized, &plan_for(&lowered.candidates, ConflictPolicy::Skip)).unwrap()
}

#[test]
fn one_native_session_from_three_tools_is_one_chat_with_three_sources() {
    let mut claude = TransferBundle::new(source("claude_code"));
    claude.chats.push(chat("s1", "claude", Some("abc"), rich_entries()));
    let mut conductor = TransferBundle::new(source("conductor"));
    let mut copy = chat("ws-1", "claude", Some("abc"), vec![entry("x", BundleEntryKind::UserMessage, "partial copy")]);
    copy.entries_source = EntriesSource::ToolCopy;
    copy.title = Some("Login bug".into());
    copy.title_source = TitleSource::Tool;
    conductor.chats.push(copy);
    let mut t3 = TransferBundle::new(source("t3code"));
    let mut thread = chat("thread-9", "claude", Some("abc"), Vec::new());
    thread.entries_source = EntriesSource::None;
    thread.title = Some("Renamed by me".into());
    thread.title_source = TitleSource::User;
    thread.pinned = true;
    t3.chats.push(thread);

    let resolved = resolve(vec![conductor, claude, t3]).unwrap();
    assert_eq!(resolved.chats.len(), 1);
    let merged = &resolved.chats[0];
    assert_eq!(merged.identity, "claude:abc");
    assert_eq!(merged.contributors.len(), 3);
    assert_eq!(merged.chat.entries_source, EntriesSource::Native, "native transcript wins over a tool copy");
    assert_eq!(merged.chat.entries.len(), rich_entries().len(), "entries are never interleaved");
    assert_eq!(merged.chat.title.as_deref(), Some("Renamed by me"), "a title the person typed wins");
    assert!(merged.chat.pinned);

    let db = database();
    let lowered = lower(&resolved);
    let normalized = normalize(&lowered.candidates);
    let commit = commit_import(&db, &normalized, &plan_for(&lowered.candidates, ConflictPolicy::Skip)).unwrap();
    assert_eq!(commit.imported, 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM sessions WHERE kind='imported'"), 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM import_session_sources"), 3);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM import_native_sessions WHERE native_key='claude:abc'"), 1);
}

#[test]
fn a_tool_copy_fills_in_when_the_native_transcript_is_gone() {
    let mut claude = TransferBundle::new(source("claude_code"));
    let mut gone = chat("s1", "claude", Some("old"), Vec::new());
    gone.entries_source = EntriesSource::None;
    claude.chats.push(gone);
    let mut conductor = TransferBundle::new(source("conductor"));
    let mut copy = chat("ws", "claude", Some("old"), vec![entry("m", BundleEntryKind::UserMessage, "kept by conductor")]);
    copy.entries_source = EntriesSource::ToolCopy;
    conductor.chats.push(copy);
    let resolved = resolve(vec![claude, conductor]).unwrap();
    assert_eq!(resolved.chats[0].chat.entries_source, EntriesSource::ToolCopy);
    assert_eq!(resolved.chats[0].chat.entries.len(), 1);
}

#[test]
fn a_newer_bundle_version_fails_closed() {
    let mut bundle = TransferBundle::new(source("claude_code"));
    bundle.bundle_version = 2;
    let error = resolve(vec![bundle]).unwrap_err().to_string();
    assert!(error.contains("version 2"), "{error}");
}

#[test]
fn every_entry_kind_lands_in_the_forest_and_renders_as_its_live_twin() {
    let mut bundle = TransferBundle::new(source("codex"));
    let mut thread = chat("t", "codex", Some("thread-1"), rich_entries());
    thread.archived = true;
    bundle.chats.push(thread);
    let db = database();
    let commit = import(&db, vec![bundle]);
    assert_eq!(commit.imported, 1, "{commit:?}");
    let (harness, cwd, label, archived): (String, Option<String>, String, Option<String>) = db
        .query_row("SELECT harness,cwd,label,archived_at FROM sessions WHERE kind='imported'", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap();
    assert_eq!(harness, "codex");
    assert_eq!(cwd.as_deref(), Some("/tmp/bridge-import-fixture/project"));
    assert_eq!(label, "Imported from Codex");
    assert!(archived.is_some(), "an archived source chat stays archived");
    let kinds: Vec<String> = db
        .prepare("SELECT kind FROM session_entries ORDER BY sequence")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        kinds,
        vec!["user.message", "reasoning", "tool.completed", "tool.completed", "assistant.message", "user.message", "compaction", "assistant.message"]
    );
    let failed: String = db
        .query_row("SELECT payload FROM session_entries WHERE kind='tool.completed' AND payload LIKE '%no such file%'", [], |row| row.get(0))
        .unwrap();
    let failed: Value = serde_json::from_str(&failed).unwrap();
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["data"]["name"], "Read");
    assert_eq!(failed["importSource"], "codex");
    let image: String = db
        .query_row("SELECT payload FROM session_entries WHERE payload LIKE '%dataUri%'", [], |row| row.get(0))
        .unwrap();
    let image: Value = serde_json::from_str(&image).unwrap();
    assert_eq!(image["data"]["attachments"][0]["dataUri"], "data:image/png;base64,iVBORw0KGgo=");
    assert_eq!(image["text"], "Attached image");
}

#[test]
fn a_chat_is_filed_under_the_workspace_whose_path_matches_its_folder() {
    let db = database();
    db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/bridge-import-fixture','now')", []).unwrap();
    db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Kyoto','Task','main','/tmp/bridge-import-fixture/project','idle','now')", []).unwrap();
    let mut bundle = TransferBundle::new(source("claude_code"));
    bundle.chats.push(chat("s", "claude", Some("s"), rich_entries()));
    import(&db, vec![bundle]);
    let workspace: Option<String> = db.query_row("SELECT workspace_id FROM sessions WHERE kind='imported'", [], |row| row.get(0)).unwrap();
    assert_eq!(workspace.as_deref(), Some("w"));
    assert_eq!(count(&db, "SELECT COUNT(*) FROM import_project_hints WHERE match_state='confirmed' AND matched_workspace_id='w'"), 1);
}

#[test]
fn subagent_chats_import_as_children_of_their_parent() {
    let mut bundle = TransferBundle::new(source("claude_code"));
    let mut child = chat("agent-1", "claude", Some("agent-1"), vec![entry("c", BundleEntryKind::UserMessage, "search the repo")]);
    child.parent_chat_key = Some("main".into());
    child.agent_role = Some(AgentRole::Subagent);
    // Child listed first: resolution must still write the parent first.
    bundle.chats.push(child);
    bundle.chats.push(chat("main", "claude", Some("main-session"), rich_entries()));
    let db = database();
    let commit = import(&db, vec![bundle]);
    assert_eq!(commit.imported, 2, "{commit:?}");
    let (parent, depth): (Option<String>, i64) = db
        .query_row("SELECT parent_session_id, depth FROM sessions WHERE title='search the repo'", [], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap();
    let parent_title: String = db.query_row("SELECT title FROM sessions WHERE id=?1", params![parent.unwrap()], |row| row.get(0)).unwrap();
    assert_eq!(parent_title, "Fix the failing login test");
    assert_eq!(depth, 1);
}

#[test]
fn a_child_listed_before_its_parent_is_still_linked() {
    let mut bundle = TransferBundle::new(source("claude_code"));
    let mut child = chat("kid", "claude", Some("kid"), vec![entry("c", BundleEntryKind::UserMessage, "child task")]);
    child.parent_chat_key = Some("main".into());
    bundle.chats.push(chat("main", "claude", Some("main"), rich_entries()));
    bundle.chats.push(child);
    let lowered = lower(&resolve(vec![bundle]).unwrap());
    let mut normalized = normalize(&lowered.candidates);
    normalized.reverse();
    let db = database();
    let commit = commit_import(&db, &normalized, &plan_for(&lowered.candidates, ConflictPolicy::Skip)).unwrap();
    assert_eq!(commit.imported, 2);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM sessions WHERE title='child task' AND parent_session_id IS NOT NULL"), 1);
}

#[test]
fn a_missing_or_cyclic_parent_keeps_the_chat_at_the_top_level() {
    let mut bundle = TransferBundle::new(source("claude_code"));
    let mut orphan = chat("o", "claude", Some("o"), rich_entries());
    orphan.parent_chat_key = Some("nowhere".into());
    let mut a = chat("a", "claude", Some("a"), rich_entries());
    a.parent_chat_key = Some("b".into());
    let mut b = chat("b", "claude", Some("b"), rich_entries());
    b.parent_chat_key = Some("a".into());
    bundle.chats.extend([orphan, a, b]);
    let resolved = resolve(vec![bundle]).unwrap();
    assert!(resolved.diagnostics.iter().any(|d| d.code == "parent_chat_missing"));
    assert!(resolved.diagnostics.iter().any(|d| d.code == "parent_chat_cycle"));
    assert!(resolved.chats.iter().any(|chat| chat.parent_identity.is_none() && chat.identity == "claude:o"));
    let lowered = lower(&resolved);
    assert_eq!(lowered.candidates.len(), 3);
}

#[test]
fn reimporting_through_another_tool_is_a_no_op_and_a_changed_chat_asks_first() {
    let db = database();
    let mut claude = TransferBundle::new(source("claude_code"));
    claude.chats.push(chat("s1", "claude", Some("same"), rich_entries()));
    assert_eq!(import(&db, vec![claude.clone()]).imported, 1);
    // The same file again: the ledger catches it.
    let again = import(&db, vec![claude]);
    assert_eq!((again.imported, again.skipped), (0, 1));
    // The same native session through Conductor, same content.
    let mut conductor = TransferBundle::new(source("conductor"));
    conductor.chats.push(chat("ws", "claude", Some("same"), rich_entries()));
    let via_conductor = import(&db, vec![conductor]);
    assert_eq!((via_conductor.imported, via_conductor.skipped), (0, 1));
    assert_eq!(
        via_conductor.candidate_results[0].diagnostics[0].code,
        "native_session_already_imported"
    );
    // The session grew: changed source, nothing written without a choice.
    let mut grown = TransferBundle::new(source("conductor"));
    let mut entries = rich_entries();
    entries.push(entry("later", BundleEntryKind::UserMessage, "one more thing"));
    grown.chats.push(chat("ws", "claude", Some("same"), entries));
    let lowered = lower(&resolve(vec![grown.clone()]).unwrap());
    let normalized = normalize(&lowered.candidates);
    let changed = commit_import(&db, &normalized, &plan_for(&lowered.candidates, ConflictPolicy::Skip)).unwrap();
    assert_eq!(changed.changed, 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM sessions WHERE kind='imported'"), 1);
    let revision = commit_import(&db, &normalized, &plan_for(&lowered.candidates, ConflictPolicy::ImportAsNewHistoricalRevision)).unwrap();
    assert_eq!(revision.imported, 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM sessions WHERE kind='imported'"), 2);
}

#[test]
fn a_session_bridge_ran_itself_is_never_imported() {
    let db = database();
    db.execute("INSERT INTO sessions(id,harness,label,status,metric_source,provider_session_id) VALUES('live','claude','Claude','stopped','reported','bridge-ran-this')", []).unwrap();
    let mut claude = TransferBundle::new(source("claude_code"));
    claude.chats.push(chat("s", "claude", Some("bridge-ran-this"), rich_entries()));
    let commit = import(&db, vec![claude]);
    assert_eq!((commit.imported, commit.skipped), (0, 1));
    assert_eq!(commit.candidate_results[0].diagnostics[0].code, "bridge_own_session");
}

#[test]
fn secrets_in_messages_and_tool_io_never_reach_the_database() {
    let db = database();
    let sentinel = "xoxb-not-a-real-token-0123456789";
    let mut bundle = TransferBundle::new(source("claude_code"));
    let mut entries = rich_entries();
    entries.push(entry("leak", BundleEntryKind::UserMessage, &format!("my token is {sentinel}")));
    let mut tool = tool_entry("leak-tool", "Bash", &format!("export SLACK={sentinel}"), ToolStatus::Completed);
    tool.tool.as_mut().unwrap().input = json!({ "command": "env", "env": { "SLACK": sentinel } });
    entries.push(tool);
    bundle.chats.push(chat("s", "claude", Some("s"), entries));
    let lowered = lower(&resolve(vec![{
        let mut probe = TransferBundle::new(source("claude_code"));
        let mut leaky = chat("p", "claude", Some("p"), rich_entries());
        leaky.cwd = Some(format!("/work/{sentinel}"));
        probe.chats.push(leaky);
        probe
    }]).unwrap());
    assert!(!lowered.candidates[0].project_hint.as_deref().unwrap_or_default().contains(sentinel), "the folder hint is redacted too");
    import(&db, vec![bundle]);
    let leaked = count(&db, &format!(
        "SELECT (SELECT COUNT(*) FROM session_entries WHERE payload LIKE '%{sentinel}%')
              + (SELECT COUNT(*) FROM sessions WHERE title LIKE '%{sentinel}%')"
    ));
    assert_eq!(leaked, 0);
    assert!(count(&db, "SELECT COUNT(*) FROM session_entries WHERE payload LIKE '%[redacted:%'") >= 2);
}

#[test]
fn oversized_images_and_empty_messages_are_reported_not_dropped_silently() {
    let mut bundle = TransferBundle::new(source("claude_code"));
    let mut big = entry("big", BundleEntryKind::UserMessage, "see attached");
    big.attachments.push(BundleAttachment {
        media_type: "image/png".into(),
        name: None,
        data_base64: Some("A".repeat(MAX_INLINE_ATTACHMENT_BYTES + 1)),
        bytes: (MAX_INLINE_ATTACHMENT_BYTES + 1) as u64,
    });
    let entries = vec![big, entry("empty", BundleEntryKind::AssistantMessage, "   ")];
    bundle.chats.push(chat("s", "claude", Some("s"), entries));
    bundle.chats.push(chat("nothing", "claude", Some("n"), vec![entry("e", BundleEntryKind::AssistantMessage, "")]));
    let lowered = lower(&resolve(vec![bundle]).unwrap());
    assert_eq!(lowered.candidates.len(), 1, "a chat with nothing to show is reported, not imported");
    let by_category: BTreeMap<&str, u32> = lowered.excluded.iter().fold(BTreeMap::new(), |mut map, item| {
        *map.entry(item.category.as_str()).or_insert(0) += item.count;
        map
    });
    assert_eq!(by_category.get("attachment"), Some(&1));
    assert_eq!(by_category.get("empty_message"), Some(&2));
    assert_eq!(by_category.get("chat"), Some(&1));
}

#[test]
fn long_tool_output_is_cut_with_a_visible_marker() {
    let mut bundle = TransferBundle::new(source("claude_code"));
    let long = "x".repeat(MAX_TOOL_OUTPUT_CHARS + 50);
    bundle.chats.push(chat("s", "claude", Some("s"), vec![
        entry("u", BundleEntryKind::UserMessage, "cat the log"),
        tool_entry("t", "Bash", &long, ToolStatus::Completed),
    ]));
    let lowered = lower(&resolve(vec![bundle]).unwrap());
    let output = lowered.candidates[0].normalized_payload["messages"][1]["data"]["aggregatedOutput"].as_str().unwrap();
    assert!(output.ends_with("[output cut at 20000 characters on import]"));
    assert!(output.chars().count() < MAX_TOOL_OUTPUT_CHARS + 100);
}

#[test]
fn an_unknown_harness_is_refused_instead_of_stored() {
    let mut bundle = TransferBundle::new(source("claude_code"));
    bundle.chats.push(chat("s", "gemini", None, rich_entries()));
    let lowered = lower(&resolve(vec![bundle]).unwrap());
    let db = database();
    let normalized = normalize(&lowered.candidates);
    let error = commit_import(&db, &normalized, &plan_for(&lowered.candidates, ConflictPolicy::Skip)).unwrap_err();
    assert!(error.to_string().contains("unsupported harness"), "{error}");
    assert_eq!(count(&db, "SELECT COUNT(*) FROM sessions"), 0, "nothing is half-written");
}

#[test]
fn workspaces_and_projects_merge_by_real_path() {
    let mut a = TransferBundle::new(source("conductor"));
    a.projects.push(BundleProject { key: "r".into(), name: "repo".into(), repo_root: "/tmp/repo/".into(), remote_url: None });
    a.workspaces.push(BundleWorkspace { key: "w".into(), project_key: Some("r".into()), path: "/tmp/repo/ws/../ws".into(), branch: Some("feat".into()), base_branch: None, title: None, archived: false, created_at: None });
    let mut b = TransferBundle::new(source("t3code"));
    b.projects.push(BundleProject { key: "p1".into(), name: "Repo".into(), repo_root: "/tmp/repo".into(), remote_url: None });
    b.workspaces.push(BundleWorkspace { key: "x".into(), project_key: Some("p1".into()), path: "/tmp/repo/ws".into(), branch: None, base_branch: None, title: Some("Login".into()), archived: false, created_at: None });
    let resolved = resolve(vec![a, b]).unwrap();
    assert_eq!(resolved.projects.len(), 1);
    assert_eq!(resolved.workspaces.len(), 1);
    assert_eq!(resolved.workspaces[0].branch.as_deref(), Some("feat"));
    assert_eq!(resolved.workspaces[0].title.as_deref(), Some("Login"));
}

#[test]
fn setup_and_preferences_import_inert() {
    let mut bundle = TransferBundle::new(source("t3code"));
    bundle.setup.push(BundleSetup { key: "s".into(), kind: CandidateKind::Script, scope: "project".into(), project_key: None, name: "setup".into(), body: Some("bun install".into()), config: None, source_ref: "/r/t3.json".into(), source_label: "t3.json".into() });
    bundle.setup.push(BundleSetup { key: "s".into(), kind: CandidateKind::Script, scope: "project".into(), project_key: None, name: "setup".into(), body: Some("bun install".into()), config: None, source_ref: "/r/t3.json".into(), source_label: "t3.json".into() });
    bundle.preferences.push(BundlePreference { key: "m".into(), name: "defaultModel".into(), value: json!("claude-opus-5-5"), bridge_setting: Some("defaultModel".into()), source_ref: "/r/settings.json".into() });
    let db = database();
    let commit = import(&db, vec![bundle]);
    assert_eq!(commit.imported, 2, "duplicate setup collapses to one: {commit:?}");
    assert_eq!(count(&db, "SELECT COUNT(*) FROM import_setup_candidates WHERE activation_state='disabled'"), 2);
}

#[test]
fn memories_from_another_tool_carry_that_tool_in_their_provenance() {
    let mut bundle = TransferBundle::new(source("codex"));
    bundle.memories.push(BundleMemory { key: "m".into(), scope: "global".into(), project_key: None, kind: "fact".into(), body: "Prefers bun over npm".into(), source_ref: "/c/memories/a.md".into(), source_label: "memories/a.md".into() });
    let db = database();
    assert_eq!(import(&db, vec![bundle]).imported, 1);
    let provenance: String = db.query_row("SELECT provenance FROM memory_records", [], |row| row.get(0)).unwrap();
    assert_eq!(provenance, "codex:memory");
}

#[test]
fn a_failure_after_any_write_leaves_nothing_behind() {
    let mut bundle = TransferBundle::new(source("claude_code"));
    let mut child = chat("kid", "claude", Some("kid"), rich_entries());
    child.parent_chat_key = Some("main".into());
    bundle.chats.push(chat("main", "claude", Some("main"), rich_entries()));
    bundle.chats.push(child);
    bundle.memories.push(BundleMemory { key: "m".into(), scope: "global".into(), project_key: None, kind: "fact".into(), body: "uses jj".into(), source_ref: "/m".into(), source_label: "m".into() });
    let lowered = lower(&resolve(vec![bundle]).unwrap());
    let normalized = normalize(&lowered.candidates);
    let plan = plan_for(&lowered.candidates, ConflictPolicy::Skip);
    for fail_after in 1..=normalized.len() {
        let db = database();
        let error = crate::external_import::commit_import_failing_after(&db, &normalized, &plan, fail_after).unwrap_err();
        assert!(error.to_string().contains("injected"));
        for table in ["sessions", "session_entries", "memory_records", "import_native_sessions", "import_session_sources", "import_ledger", "imported_record_provenance", "import_commits"] {
            assert_eq!(count(&db, &format!("SELECT COUNT(*) FROM {table}")), 0, "{table} after failing at {fail_after}");
        }
    }
}

fn arbitrary_entry() -> impl Strategy<Value = BundleEntry> {
    (
        "[a-z0-9]{1,6}",
        0u8..5,
        proptest::option::of("\\PC{0,40}"),
        any::<bool>(),
        0u8..3,
    )
        .prop_map(|(id, kind, text, with_image, status)| {
            let kind = match kind {
                0 => BundleEntryKind::UserMessage,
                1 => BundleEntryKind::AssistantMessage,
                2 => BundleEntryKind::Reasoning,
                3 => BundleEntryKind::ToolCall,
                _ => BundleEntryKind::CompactionBoundary,
            };
            BundleEntry {
                source_id: id.clone(),
                at: "2026-11-01T10:00:00Z".into(),
                kind,
                text: text.clone(),
                tool: (kind == BundleEntryKind::ToolCall && status > 0).then(|| BundleTool {
                    call_id: id,
                    name: text.clone().unwrap_or_default(),
                    input: json!({ "q": text }),
                    output: text,
                    status: if status == 1 { ToolStatus::Completed } else { ToolStatus::Failed },
                }),
                attachments: if with_image && kind == BundleEntryKind::UserMessage {
                    vec![BundleAttachment { media_type: "image/png".into(), name: None, data_base64: Some("AAAA".into()), bytes: 3 }]
                } else {
                    Vec::new()
                },
            }
        })
}

fn arbitrary_chat() -> impl Strategy<Value = BundleChat> {
    (
        "[a-c]{1,2}",
        prop_oneof![Just("claude"), Just("codex"), Just("opencode"), Just("other")],
        proptest::option::of("[a-c]{1,2}"),
        proptest::option::of("[a-c]{1,2}"),
        proptest::collection::vec(arbitrary_entry(), 0..6),
        0u8..3,
        proptest::option::of("\\PC{0,20}"),
    )
        .prop_map(|(key, harness, native, parent, entries, source_kind, title)| {
            let mut chat = chat(&key, harness, native.as_deref(), entries);
            chat.parent_chat_key = parent;
            chat.entries_source = match source_kind {
                0 => EntriesSource::Native,
                1 => EntriesSource::ToolCopy,
                _ => EntriesSource::None,
            };
            chat.title = title;
            chat.title_source = TitleSource::Tool;
            chat
        })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// Fuzz: any mix of bundles resolves and lowers without panicking, keeps
    /// one chat per identity with parents first, and a commit is all or nothing.
    #[test]
    fn arbitrary_bundles_resolve_lower_and_commit_atomically(
        groups in proptest::collection::vec(
            (prop_oneof![Just("claude_code"), Just("conductor"), Just("t3code")], proptest::collection::vec(arbitrary_chat(), 0..5)),
            1..4,
        )
    ) {
        let bundles: Vec<TransferBundle> = groups.into_iter().map(|(tool, chats)| {
            let mut bundle = TransferBundle::new(source(tool));
            bundle.chats = chats;
            bundle
        }).collect();
        let resolved = resolve(bundles).unwrap();
        let identities: HashSet<&str> = resolved.chats.iter().map(|chat| chat.identity.as_str()).collect();
        prop_assert_eq!(identities.len(), resolved.chats.len());
        let mut seen = HashSet::new();
        for chat in &resolved.chats {
            if let Some(parent) = &chat.parent_identity {
                prop_assert!(seen.contains(parent.as_str()), "parent {} after child {}", parent, chat.identity);
            }
            seen.insert(chat.identity.as_str());
        }
        let lowered = lower(&resolved);
        for candidate in &lowered.candidates {
            let messages = candidate.normalized_payload["messages"].as_array().unwrap();
            prop_assert!(!messages.is_empty());
            for (index, message) in messages.iter().enumerate() {
                prop_assert_eq!(message["sequence"].as_u64(), Some(index as u64 + 1));
            }
        }
        let db = database();
        let normalized = normalize(&lowered.candidates);
        if normalized.is_empty() {
            return Ok(());
        }
        match commit_import(&db, &normalized, &plan_for(&lowered.candidates, ConflictPolicy::ImportAlongside)) {
            Ok(commit) => {
                prop_assert_eq!(count(&db, "SELECT COUNT(*) FROM sessions") as u32, commit.imported);
                let orphans = count(&db, "SELECT COUNT(*) FROM sessions s WHERE s.parent_session_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sessions p WHERE p.id = s.parent_session_id)");
                prop_assert_eq!(orphans, 0);
            }
            Err(_) => {
                prop_assert_eq!(count(&db, "SELECT COUNT(*) FROM sessions"), 0);
                prop_assert_eq!(count(&db, "SELECT COUNT(*) FROM session_entries"), 0);
            }
        }
    }
}

#[test]
fn exclusions_merge_by_category_label_and_reason_and_ignore_zero() {
    let mut bundle = TransferBundle::new(source("codex"));
    bundle.exclude("attachment", "a.jsonl", 2, "too big");
    bundle.exclude("attachment", "a.jsonl", 3, "too big");
    bundle.exclude("attachment", "b.jsonl", 1, "too big");
    bundle.exclude("attachment", "a.jsonl", 1, "not an image");
    bundle.exclude("chat", "a.jsonl", 1, "too big");
    bundle.exclude("chat", "c.jsonl", 0, "nothing");
    let summary: Vec<(String, String, u32, String)> = bundle
        .excluded
        .iter()
        .map(|item| (item.category.clone(), item.source_label.clone(), item.count, item.reason.clone()))
        .collect();
    assert_eq!(summary, vec![
        ("attachment".into(), "a.jsonl".into(), 5, "too big".into()),
        ("attachment".into(), "b.jsonl".into(), 1, "too big".into()),
        ("attachment".into(), "a.jsonl".into(), 1, "not an image".into()),
        ("chat".into(), "a.jsonl".into(), 1, "too big".into()),
    ]);
    // Exclusions from several bundles add up in the resolved import.
    let mut other = TransferBundle::new(source("conductor"));
    other.exclude("attachment", "a.jsonl", 4, "too big");
    let resolved = resolve(vec![bundle, other]).unwrap();
    assert_eq!(resolved.excluded[0].count, 9);
}

#[test]
fn import_limits_are_the_documented_values() {
    assert_eq!(MAX_INLINE_ATTACHMENT_BYTES, 5_242_880);
    assert_eq!(MAX_TOOL_OUTPUT_CHARS, 20_000);
    assert_eq!(BUNDLE_VERSION, 1);
}

#[test]
fn a_native_chat_with_no_entries_does_not_beat_a_tool_copy_with_some() {
    let mut claude = TransferBundle::new(source("claude_code"));
    claude.chats.push(chat("s", "claude", Some("x"), Vec::new()));
    let mut t3 = TransferBundle::new(source("t3code"));
    let mut copy = chat("t", "claude", Some("x"), vec![entry("m", BundleEntryKind::UserMessage, "kept")]);
    copy.entries_source = EntriesSource::ToolCopy;
    t3.chats.push(copy);
    let resolved = resolve(vec![claude, t3]).unwrap();
    assert_eq!(resolved.chats[0].chat.entries_source, EntriesSource::ToolCopy);
    // And a longer native transcript beats a shorter native one.
    let mut a = TransferBundle::new(source("claude_code"));
    a.chats.push(chat("s", "claude", Some("y"), vec![entry("1", BundleEntryKind::UserMessage, "one")]));
    let mut b = TransferBundle::new(source("conductor"));
    b.chats.push(chat("w", "claude", Some("y"), rich_entries()));
    assert_eq!(resolve(vec![a, b]).unwrap().chats[0].chat.entries.len(), rich_entries().len());
}
