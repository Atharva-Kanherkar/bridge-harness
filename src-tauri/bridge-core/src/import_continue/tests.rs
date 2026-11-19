use super::*;
use crate::claude_import::{ClaudeCodeImporter, PROVIDER};
use crate::external_import::{
    commit_import, CandidateKind, ConflictPolicy, DiscoveryRequest, DiscoverySelection, ExternalHarnessImporter,
    ImportPlan, SetupActivationPolicy,
};
use serde_json::json;
use std::fs;

const SESSION: &str = "9b6f0a52-1c2d-4e3f-8a9b-0c1d2e3f4a5b";

/// A Claude home with one real-shaped session whose folder exists, imported
/// into a fresh store. Returns the store, the home, the folder and the
/// imported chat's id.
struct Fixture {
    _root: tempfile::TempDir,
    _scratch: tempfile::TempDir,
    core: Arc<BridgeCore>,
    home: PathBuf,
    cwd: PathBuf,
    imported: String,
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("work/app");
    fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.canonicalize().unwrap();
    let home = root.path().join(".claude").canonicalize().unwrap_or_else(|_| {
        fs::create_dir_all(root.path().join(".claude")).unwrap();
        root.path().join(".claude").canonicalize().unwrap()
    });
    let cwd_text = cwd.to_string_lossy().into_owned();
    let transcript = transcript_path(&home, &cwd_text, SESSION);
    fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    let lines = [
        json!({"type":"user","uuid":"u1","sessionId":SESSION,"timestamp":"2026-11-01T10:00:00Z","cwd":cwd_text,"message":{"role":"user","content":"fix the login"}}),
        json!({"type":"assistant","uuid":"a1","sessionId":SESSION,"timestamp":"2026-11-01T10:00:05Z","cwd":cwd_text,"message":{"role":"assistant","content":[{"type":"text","text":"Fixed: the token was read before it was set."}]}}),
    ];
    fs::write(&transcript, lines.iter().map(|line| line.to_string()).collect::<Vec<_>>().join("\n")).unwrap();

    let scratch = tempfile::tempdir().unwrap();
    let core = Arc::new(BridgeCore::for_tests(scratch.path()));
    let request = DiscoveryRequest {
        provider: PROVIDER.into(),
        approved_roots: vec![home.to_string_lossy().into_owned()],
        selected_export: None,
        source_version: None,
        schema_version: None,
        format_versions: std::collections::BTreeMap::new(),
    };
    let discovery = ClaudeCodeImporter.discover_with_homes(&request, std::slice::from_ref(&home)).unwrap();
    let selection = DiscoverySelection {
        artifact_ids: discovery
            .artifacts
            .iter()
            .filter(|artifact| artifact.kind == CandidateKind::Conversation)
            .map(|artifact| artifact.artifact_id.clone())
            .collect(),
    };
    let normalized: Vec<_> = ClaudeCodeImporter
        .preview(&discovery, &selection)
        .unwrap()
        .into_iter()
        .filter(|candidate| candidate.kind == CandidateKind::Conversation)
        .map(|candidate| ClaudeCodeImporter.normalize(candidate).unwrap())
        .collect();
    assert_eq!(normalized.len(), 1);
    let imported = normalized[0].deterministic_bridge_id.clone();
    {
        let db = core.db.lock().unwrap();
        let plan = ImportPlan {
            selected_candidate_ids: vec![normalized[0].candidate.candidate_id.clone()],
            conflict_policy: ConflictPolicy::Skip,
            setup_activation_policy: SetupActivationPolicy::Disabled,
            memory_scope: None,
            dry_run: false,
            created_at: "2026-11-19T10:00:00Z".into(),
        };
        let commit = commit_import(&db, &normalized, &plan).unwrap();
        assert_eq!(commit.imported, 1);
    }
    Fixture { _root: root, _scratch: scratch, core, home, cwd, imported }
}

impl Fixture {
    fn plan(&self, homes: &[ClaudeHome]) -> Plan {
        let db = self.core.db.lock().unwrap();
        plan_in(&db, &self.imported, homes).unwrap()
    }
    fn own_login(&self) -> Vec<ClaudeHome> {
        vec![(None, self.home.clone())]
    }
    fn transcript(&self) -> PathBuf {
        transcript_path(&self.home, &self.cwd.to_string_lossy(), SESSION)
    }
    fn entries(&self, session: &str) -> Vec<(String, String, String)> {
        let db = self.core.db.lock().unwrap();
        let mut statement = db
            .prepare("SELECT kind, payload, created_at FROM session_entries WHERE session_id=?1 ORDER BY sequence")
            .unwrap();
        statement
            .query_map(params![session], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }
    fn raw_entries(&self, session: &str) -> Vec<(String, String, String, Option<String>)> {
        let db = self.core.db.lock().unwrap();
        let mut statement = db
            .prepare("SELECT id, payload, created_at, parent_entry_id FROM session_entries WHERE session_id=?1 ORDER BY sequence")
            .unwrap();
        statement
            .query_map(params![session], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }
    fn session(&self, id: &str) -> (String, Option<String>, Option<String>, Option<String>, Option<String>) {
        let db = self.core.db.lock().unwrap();
        db.query_row(
            "SELECT s.kind, s.provider_session_id, s.account, s.fork_parent_session_id, h.restoration_mode
             FROM sessions s LEFT JOIN session_heads h ON h.session_id=s.id WHERE s.id=?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .unwrap()
    }
}

#[test]
fn continues_natively_when_claude_code_still_has_the_session() {
    let f = fixture();
    let plan = f.plan(&f.own_login());
    assert_eq!(plan, Plan { mode: NATIVE, reason: None, native: Some((SESSION.into(), None)) });
}

#[test]
fn a_session_under_a_claude_account_signs_in_with_that_account() {
    let f = fixture();
    let elsewhere = tempfile::tempdir().unwrap();
    let homes = vec![(None, elsewhere.path().to_path_buf()), (Some("work".to_owned()), f.home.clone())];
    assert_eq!(f.plan(&homes).native, Some((SESSION.into(), Some("work".into()))));
}

#[test]
fn every_reason_it_cannot_continue_natively() {
    let f = fixture();
    let reason = |plan: Plan| {
        assert_eq!(plan.mode, HISTORY);
        assert!(plan.native.is_none());
        plan.reason.unwrap()
    };

    let elsewhere = tempfile::tempdir().unwrap();
    assert_eq!(
        reason(f.plan(&[(None, elsewhere.path().to_path_buf())])),
        "The session is saved under a Claude Code folder Bridge does not sign in with."
    );

    {
        let db = f.core.db.lock().unwrap();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,provider_session_id,kind)
             VALUES('other',NULL,'claude','Other','stopped','reported',?1,'direct')",
            params![SESSION],
        )
        .unwrap();
    }
    assert_eq!(reason(f.plan(&f.own_login())), "Another Bridge chat already continues this Claude Code session.");
    f.core.db.lock().unwrap().execute("DELETE FROM sessions WHERE id='other'", []).unwrap();

    fs::rename(&f.cwd, f.cwd.with_file_name("moved")).unwrap();
    assert_eq!(reason(f.plan(&f.own_login())), "This chat's folder no longer exists.");
    fs::rename(f.cwd.with_file_name("moved"), &f.cwd).unwrap();

    fs::remove_file(f.transcript()).unwrap();
    assert_eq!(reason(f.plan(&f.own_login())), "Claude Code no longer has this session.");

    f.core.db.lock().unwrap().execute("DELETE FROM import_native_sessions", []).unwrap();
    assert_eq!(reason(f.plan(&f.own_login())), "Bridge did not record which Claude Code session this was.");
}

#[test]
fn only_an_imported_chat_continues_this_way() {
    let f = fixture();
    let plan = f.plan(&f.own_login());
    let id = continue_in(&f.core.db.lock().unwrap(), &f.imported, &plan).unwrap();
    let db = f.core.db.lock().unwrap();
    assert!(plan_in(&db, &id, &f.own_login()).unwrap_err().to_string().contains("Only an imported chat"));
}

#[test]
fn a_native_continuation_resumes_the_claude_session_and_leaves_the_import_untouched() {
    let f = fixture();
    let before = f.raw_entries(&f.imported);
    let plan = f.plan(&f.own_login());

    let id = continue_in(&f.core.db.lock().unwrap(), &f.imported, &plan).unwrap();

    assert_eq!(f.raw_entries(&f.imported), before, "the imported chat is byte-identical");
    let (kind, provider, account, fork_parent, restoration) = f.session(&id);
    assert_eq!(kind, "direct", "an ordinary chat that can start");
    assert_eq!(provider.as_deref(), Some(SESSION), "its first start resumes the Claude Code session");
    assert_eq!(account, None);
    assert_eq!(fork_parent.as_deref(), Some(f.imported.as_str()), "it branches from the imported chat");
    assert_eq!(restoration.as_deref(), Some("native"));
    assert_eq!(f.entries(&id), f.entries(&f.imported), "the history is copied");
    let continuation_ids: Vec<String> = f.raw_entries(&id).into_iter().map(|row| row.0).collect();
    assert!(before.iter().all(|row| !continuation_ids.contains(&row.0)), "under new entry ids");
    assert_eq!(
        crate::restoration::select_plan(false, provider.as_deref(), true, false, true, false),
        crate::restoration::RestorationPlan::Native
    );

    let again = f.plan(&f.own_login());
    assert_eq!(again.mode, HISTORY, "a second continuation cannot take the same session");
    assert_eq!(again.reason.as_deref(), Some("Another Bridge chat already continues this Claude Code session."));
}

#[test]
fn a_history_continuation_starts_fresh_from_the_copied_history() {
    let f = fixture();
    fs::remove_file(f.transcript()).unwrap();
    let before = f.raw_entries(&f.imported);
    let plan = f.plan(&f.own_login());

    let id = continue_in(&f.core.db.lock().unwrap(), &f.imported, &plan).unwrap();

    assert_eq!(f.raw_entries(&f.imported), before);
    let (kind, provider, _, fork_parent, restoration) = f.session(&id);
    assert_eq!(kind, "direct");
    assert_eq!(provider, None, "nothing to resume");
    assert_eq!(fork_parent.as_deref(), Some(f.imported.as_str()));
    assert_eq!(restoration.as_deref(), Some("checkpoint_restored"), "seeded with the history");
    assert_eq!(f.entries(&id), f.entries(&f.imported));
}

/// The copied history reaches the model: a history continuation's first
/// start injects it as restoration context, and so does a native one that
/// has to fall back. Found by the real Claude Code run, where the new chat
/// answered "this is the first message in our conversation".
#[test]
fn the_continuation_gives_the_imported_conversation_to_the_model() {
    let f = fixture();
    fs::remove_file(f.transcript()).unwrap();
    let plan = f.plan(&f.own_login());
    let id = continue_in(&f.core.db.lock().unwrap(), &f.imported, &plan).unwrap();

    let context = crate::restoration::checkpoint_context(&f.core.db.lock().unwrap(), &id).unwrap();

    let context = context.expect("the continuation has restoration context");
    assert!(context.contains("fix the login"), "{context}");
    assert!(context.contains("the token was read before it was set"), "{context}");
    let imported = crate::restoration::checkpoint_context(&f.core.db.lock().unwrap(), &f.imported).unwrap();
    assert!(imported.is_none_or(|text| !text.contains("fix the login")), "the imported chat itself stays out of any context");
}

#[test]
fn a_continuation_keeps_its_session_and_mode_across_a_restart() {
    let f = fixture();
    let plan = f.plan(&f.own_login());
    let id = continue_in(&f.core.db.lock().unwrap(), &f.imported, &plan).unwrap();
    // What a restart reads: the store written to disk and opened again.
    let disk = tempfile::tempdir().unwrap();
    let path = disk.path().join("bridge.db");
    f.core.db.lock().unwrap().execute("VACUUM INTO ?1", params![path.to_string_lossy()]).unwrap();
    let reopened = crate::store::open(&path).unwrap();
    let (provider, restoration): (Option<String>, String) = reopened
        .query_row(
            "SELECT s.provider_session_id, h.restoration_mode FROM sessions s JOIN session_heads h ON h.session_id=s.id WHERE s.id=?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((provider.as_deref(), restoration.as_str()), (Some(SESSION), "native"));
}

#[test]
fn forking_an_imported_chat_makes_a_chat_that_can_start() {
    let f = fixture();
    let db = f.core.db.lock().unwrap();
    let head: String = db
        .query_row("SELECT active_entry_id FROM session_heads WHERE session_id=?1", params![f.imported], |row| row.get(0))
        .unwrap();
    let fork = crate::sessions::fork_session_records(&db, &f.imported, &head, None, None, None, "shared", None).unwrap();
    let kind: String = db.query_row("SELECT kind FROM sessions WHERE id=?1", params![fork], |row| row.get(0)).unwrap();
    assert_eq!(kind, "direct");
}

#[test]
fn claude_code_files_a_session_under_its_folder_name() {
    assert_eq!(project_folder("/Users/you/work/my_app.v2"), "-Users-you-work-my-app-v2");
}
