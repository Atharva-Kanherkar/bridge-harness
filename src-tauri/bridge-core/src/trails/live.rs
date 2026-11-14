//! The trail maintenance loop: follow new hard links shortly after the records
//! that carry them move, and tell clients once.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use bridge_protocol::messages::{TrailMemberAdded, TrailMemberKind, TrailsChanged};

use super::auto;
use super::timeline::NoteSnapshot;
use crate::events::{CoreEvent, ReceiveError};
use crate::BridgeCore;

/// Events arriving within this window share one sweep.
pub const DEBOUNCE: Duration = Duration::from_millis(1500);

/// What an event means for trails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Relevance {
    None,
    /// A new link may exist (a chat was created or forked); sweep, and say
    /// something only if the sweep found one.
    Links,
    /// A member's own state may have moved (checks, a draft, attention):
    /// sweep and always send a refetch hint.
    State,
}

pub fn relevance(event: &CoreEvent) -> Relevance {
    match event {
        CoreEvent::StateChanged => Relevance::Links,
        // A person changed a trail (added an issue, created one from a chat):
        // its new members may need reading from GitHub and watching. The
        // loop's own announcements carry additions or no trail, so it never
        // wakes itself.
        CoreEvent::TrailsChanged(changed) if changed.trail_id.is_some() && changed.added.is_empty() => Relevance::Links,
        CoreEvent::GithubSessionPrsChanged { .. }
        | CoreEvent::GithubChecksChanged { .. }
        | CoreEvent::GithubCiFinished { .. }
        | CoreEvent::OutgoingChanged { .. }
        | CoreEvent::NotesPageChanged(_)
        | CoreEvent::AttentionChanged => Relevance::State,
        _ => Relevance::None,
    }
}

/// The most sweeps one call runs while chats keep joining.
const MAX_ROUNDS: usize = 8;

/// Sweep every open trail and publish what joined, one notification per
/// trail. With `hint`, also publish a plain refetch hint when there are
/// trails, whether or not anything was added.
///
/// The sweep follows "a note linked to a member chat" through a snapshot of
/// only the notes linked to member chats, so when a chat joins, the sweep runs
/// again with its notes in view.
pub fn sweep_and_publish(core: &Arc<BridgeCore>, hint: bool) -> Vec<TrailMemberAdded> {
    let sessions = {
        let db = core.db.lock().unwrap();
        if !super::store::list(&db, false).map(|trails| !trails.is_empty()).unwrap_or(false) {
            return Vec::new();
        }
        super::timeline::open_trail_sessions(&db).unwrap_or_default()
    };
    let mut sessions = sessions;
    let mut all = Vec::new();
    for _ in 0..MAX_ROUNDS {
        let notes = NoteSnapshot::linked(core.notes.as_ref(), &sessions);
        let now = chrono::Utc::now().to_rfc3339();
        let db = core.db.lock().unwrap();
        let added = match auto::sweep(&db, &notes, &now) {
            Ok(added) => added,
            Err(error) => {
                eprintln!("trails: sweep failed: {error}");
                Vec::new()
            }
        };
        let chat_joined = added.iter().any(|item| item.member.kind == TrailMemberKind::Session);
        all.extend(added);
        if !chat_joined {
            break;
        }
        sessions = super::timeline::open_trail_sessions(&db).unwrap_or_default();
    }
    publish_added(core, &all);
    // Additions name only the trails that gained a member; a state event in
    // the same batch may have moved any other trail, so it still refetches.
    if hint {
        core.events.publish(CoreEvent::TrailsChanged(TrailsChanged { trail_id: None, added: Vec::new() }));
    }
    all
}

/// Watch every open pull request on an open trail, so the poller carries its
/// checks and state into the stored snapshot (and so the Trails list) with no
/// chat or trail page open. Merged and closed ones are left alone; arming is
/// idempotent and keeps each PR's cadence. Returns how many are watched.
pub fn arm_trail_pulls(core: &Arc<BridgeCore>) -> usize {
    // Every chat row of each trail PR, newest snapshot first: the same PR can
    // be attached from several workspaces, and an archived one leaves its row.
    let rows: Vec<(String, String, i64, String, String, Option<String>)> = {
        let db = core.db.lock().unwrap();
        let statement = db.prepare(
            "SELECT p.url, p.workspace_id, p.number, p.head_branch, p.title, json_extract(p.snapshot,'$.state') FROM trail_members m
             JOIN trails t ON t.id=m.trail_id AND t.archived_at IS NULL
             JOIN session_pull_requests p ON p.url=m.ref
             WHERE m.kind='pullRequest'
             ORDER BY p.url, p.snapshot IS NULL, p.fetched_at DESC, p.created_at DESC",
        );
        match statement.and_then(|mut statement| {
            statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)))?.collect()
        }) {
            Ok(rows) => rows,
            Err(error) => {
                eprintln!("trails: could not list trail pull requests: {error}");
                return 0;
            }
        }
    };
    let mut watched = 0;
    let mut current: Option<(String, bool)> = None;
    for (url, workspace_id, number, head_branch, title, state) in rows {
        // The newest snapshot decides whether the PR is still open; then the
        // first attachment whose workspace still resolves is armed.
        let open = match &current {
            Some((seen, open)) if *seen == url => *open,
            _ => {
                let open = state.as_deref().unwrap_or("open") == "open";
                current = Some((url.clone(), open));
                open
            }
        };
        if !open {
            continue;
        }
        let Ok(path) = core.workspace_path(&workspace_id) else { continue };
        core.github_poller.watch_attached(&workspace_id, std::path::PathBuf::from(path), number as u64, &head_branch, &title);
        watched += 1;
        // Armed once; this PR's other rows are skipped.
        current = Some((url, false));
    }
    watched
}

/// The most read-then-sweep rounds one pass runs.
const MAX_READ_ROUNDS: usize = 8;

/// Read from GitHub what the trails still lack (issue snapshots, PRs'
/// closing references), outside the database lock; if anything landed, the
/// links may bring new members, so sweep again and tell clients. Members a
/// sweep brings in are read in the same pass: the events it publishes never
/// wake this loop, so nothing else would read them.
pub fn read_github(core: &Arc<BridgeCore>) {
    for _ in 0..MAX_READ_ROUNDS {
        if !super::issues::read_missing(core).stored {
            break;
        }
        if sweep_and_publish(core, true).is_empty() {
            break;
        }
    }
}

pub fn publish_added(core: &Arc<BridgeCore>, added: &[TrailMemberAdded]) {
    let mut by_trail: BTreeMap<&str, Vec<TrailMemberAdded>> = BTreeMap::new();
    for item in added {
        by_trail.entry(item.trail_id.as_str()).or_default().push(item.clone());
    }
    for (trail_id, added) in by_trail {
        core.events.publish(CoreEvent::TrailsChanged(TrailsChanged { trail_id: Some(trail_id.to_owned()), added }));
    }
}

/// How often the loop looks for an event while a retry is pending.
const RETRY_TICK: Duration = Duration::from_millis(250);

enum Wake {
    Event(CoreEvent),
    Retry,
    Lagged,
    Closed,
}

/// The next event, or the moment the earliest failed GitHub read is due
/// again, whichever comes first. With no retry pending it blocks on events
/// alone; with one it looks for events every [`RETRY_TICK`] and re-reads the
/// deadline each time, so a read that succeeds meanwhile clears it.
fn next_wake(core: &BridgeCore, events: &mut crate::events::EventReceiver) -> Wake {
    loop {
        let Some(due) = core.trail_reads.next_retry() else {
            return match events.blocking_recv() {
                Ok(event) => Wake::Event(event),
                Err(ReceiveError::Lagged(_)) => Wake::Lagged,
                Err(ReceiveError::Closed) => Wake::Closed,
            };
        };
        match events.poll() {
            Ok(Some(event)) => return Wake::Event(event),
            Ok(None) => {}
            Err(ReceiveError::Lagged(_)) => return Wake::Lagged,
            Err(ReceiveError::Closed) => return Wake::Closed,
        }
        let now = std::time::Instant::now();
        if due <= now {
            return Wake::Retry;
        }
        std::thread::sleep((due - now).min(RETRY_TICK));
    }
}

/// Started by both hosts with the other maintenance loops.
pub fn start_trail_maintenance(core: Arc<BridgeCore>) {
    let mut events = core.events.subscribe();
    std::thread::Builder::new()
        .name("bridge-trails".into())
        .spawn(move || {
            sweep_and_publish(&core, false);
            read_github(&core);
            arm_trail_pulls(&core);
            loop {
                let first = match next_wake(&core, &mut events) {
                    Wake::Event(event) => relevance(&event),
                    // A failed GitHub read is due again; nothing else would
                    // ask for it, so read now.
                    Wake::Retry => {
                        read_github(&core);
                        arm_trail_pulls(&core);
                        continue;
                    }
                    // Missed events may have carried links: sweep anyway.
                    Wake::Lagged => Relevance::State,
                    Wake::Closed => break,
                };
                if first == Relevance::None {
                    continue;
                }
                std::thread::sleep(DEBOUNCE);
                let mut strongest = first;
                loop {
                    match events.try_next() {
                        Ok(event) => strongest = strongest.max(relevance(&event)),
                        Err(true) => strongest = Relevance::State,
                        Err(false) => break,
                    }
                }
                sweep_and_publish(&core, strongest == Relevance::State);
                read_github(&core);
                arm_trail_pulls(&core);
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;
    use bridge_protocol::messages::{TrailAddedBy, TrailMemberKind, TrailMemberRef};

    #[test]
    fn trails_changed_is_not_relevant_to_itself() {
        assert_eq!(relevance(&CoreEvent::TrailsChanged(TrailsChanged { trail_id: None, added: Vec::new() })), Relevance::None);
        let announced = TrailMemberAdded {
            trail_id: "t".into(),
            trail_title: "T".into(),
            member: TrailMemberRef { kind: TrailMemberKind::Issue, reference: "https://github.com/o/r/issues/1".into() },
            label: "Issue #1".into(),
        };
        assert_eq!(relevance(&CoreEvent::TrailsChanged(TrailsChanged { trail_id: Some("t".into()), added: vec![announced] })), Relevance::None);
        // A person's edit wakes the loop, so a new issue or PR is read and watched.
        assert_eq!(relevance(&CoreEvent::TrailsChanged(TrailsChanged { trail_id: Some("t".into()), added: Vec::new() })), Relevance::Links);
        assert_eq!(relevance(&CoreEvent::StateChanged), Relevance::Links);
        assert_eq!(relevance(&CoreEvent::OutgoingChanged { session_id: "s".into(), id: "o".into() }), Relevance::State);
    }

    #[test]
    fn a_state_batch_still_sends_the_plain_hint_when_another_trail_gains_a_member() {
        let dir = tempfile::tempdir().unwrap();
        let core = Arc::new(BridgeCore::for_tests(dir.path()));
        {
            let db = core.db.lock().unwrap();
            crate::trails::timeline::tests::add_session(&db, "a", "codex", "A", None);
            crate::trails::timeline::tests::add_session(&db, "b", "codex", "B", None);
            crate::trails::timeline::tests::attach_pr(&db, "a", "https://github.com/o/r/pull/12", "t", None);
            for (title, session) in [("A", "a"), ("B", "b")] {
                let trail = crate::trails::store::create(&db, title, None, "t0").unwrap();
                crate::trails::store::add_member(&db, &trail.id, &TrailMemberRef { kind: TrailMemberKind::Session, reference: session.into() }, TrailAddedBy::Person, "t0").unwrap();
            }
        }
        let mut events = core.events.subscribe();
        assert_eq!(sweep_and_publish(&core, true).len(), 1, "trail A gains its PR");
        let (mut targeted, mut plain) = (0, 0);
        while let Ok(event) = events.try_recv() {
            if let CoreEvent::TrailsChanged(changed) = event {
                if changed.trail_id.is_some() { targeted += 1 } else { plain += 1 }
            }
        }
        assert_eq!(targeted, 1);
        assert_eq!(plain, 1, "trail B's state may have moved in the same batch, so every trail refetches");
    }

    #[test]
    fn a_sweep_publishes_each_trails_additions_once() {
        let dir = tempfile::tempdir().unwrap();
        let core = Arc::new(BridgeCore::for_tests(dir.path()));
        assert!(sweep_and_publish(&core, true).is_empty(), "no trails, nothing to do");
        {
            let db = core.db.lock().unwrap();
            crate::trails::timeline::tests::add_session(&db, "s1", "codex", "Build", None);
            crate::trails::timeline::tests::attach_pr(&db, "s1", "https://github.com/o/r/pull/12", "t", None);
            let trail = crate::trails::store::create(&db, "T", None, "t0").unwrap();
            crate::trails::store::add_member(&db, &trail.id, &TrailMemberRef { kind: TrailMemberKind::Session, reference: "s1".into() }, TrailAddedBy::Person, "t0").unwrap();
        }
        let mut events = core.events.subscribe();
        assert_eq!(sweep_and_publish(&core, true).len(), 1);
        let mut seen = Vec::new();
        while let Ok(event) = events.try_recv() {
            if let CoreEvent::TrailsChanged(changed) = event {
                if changed.trail_id.is_some() {
                    seen.push(changed);
                }
            }
        }
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].added[0].label, "PR #12 Archive a chat");
        sweep_and_publish(&core, true);
        let mut hints = 0;
        while let Ok(event) = events.try_recv() {
            if let CoreEvent::TrailsChanged(changed) = event {
                assert!(changed.added.is_empty() && changed.trail_id.is_none());
                hints += 1;
            }
        }
        assert_eq!(hints, 1, "a second sweep sends a plain hint, no additions");
    }
    fn trail_with_pr(core: &Arc<BridgeCore>, view: &crate::session_prs::SessionPrView) -> String {
        let db = core.db.lock().unwrap();
        crate::trails::timeline::tests::attach_pr(&db, "s", "https://github.com/o/r/pull/12", "2026-11-12T00:00:00Z", Some(view));
        let trail = crate::trails::store::create(&db, "Archive", None, "t0").unwrap();
        for member in [
            TrailMemberRef { kind: TrailMemberKind::Session, reference: "s".into() },
            TrailMemberRef { kind: TrailMemberKind::PullRequest, reference: "https://github.com/o/r/pull/12".into() },
        ] {
            crate::trails::store::add_member(&db, &trail.id, &member, TrailAddedBy::Person, "t0").unwrap();
        }
        trail.id
    }

    fn state(core: &Arc<BridgeCore>) -> bridge_protocol::messages::TrailState {
        crate::api::list_trails(core, false).unwrap().trails[0].state
    }

    #[test]
    fn open_trail_prs_are_watched_and_merged_ones_are_not() {
        use crate::github_surface::{CheckConclusion, PullRequestState};
        let (_scratch, core, _repo) = crate::session_prs::tests::fixture();
        let trail = trail_with_pr(&core, &crate::trails::timeline::tests::view(PullRequestState::Open, vec![("test", Some(CheckConclusion::Success))]));
        assert!(!core.github_poller.is_watching("w", 12), "nothing has read the PR since launch");
        assert_eq!(arm_trail_pulls(&core), 1);
        assert!(core.github_poller.is_watching("w", 12));
        assert_eq!(arm_trail_pulls(&core), 1, "arming twice is harmless");

        let (_scratch, merged_core, _repo) = crate::session_prs::tests::fixture();
        trail_with_pr(&merged_core, &crate::trails::timeline::tests::view(PullRequestState::Merged, vec![]));
        assert_eq!(arm_trail_pulls(&merged_core), 0);
        assert!(!merged_core.github_poller.is_watching("w", 12));

        // An archived trail's PRs are not watched either.
        crate::trails::store::set_archived(&core.db.lock().unwrap(), &trail, true, "t1").unwrap();
        let (_scratch, archived_core, _repo) = crate::session_prs::tests::fixture();
        let archived = trail_with_pr(&archived_core, &crate::trails::timeline::tests::view(PullRequestState::Open, vec![]));
        crate::trails::store::set_archived(&archived_core.db.lock().unwrap(), &archived, true, "t1").unwrap();
        assert_eq!(arm_trail_pulls(&archived_core), 0);
    }

    #[test]
    fn a_failing_check_and_a_merge_move_a_trail_without_a_read() {
        use crate::github_surface::{CheckConclusion, PullRequestState};
        let (scratch, core, _repo) = crate::session_prs::tests::fixture();
        trail_with_pr(&core, &crate::trails::timeline::tests::view(PullRequestState::Open, vec![("test", Some(CheckConclusion::Success))]));
        assert_eq!(state(&core), bridge_protocol::messages::TrailState::InProgress);
        // CI goes red on GitHub. No chat, no trail page, no refresh.
        std::fs::write(
            scratch.path().join("checks.json"),
            r#"[{"name":"test","state":"FAILURE","bucket":"fail","link":"https://ci/test","workflow":"CI"}]"#,
        )
        .unwrap();
        arm_trail_pulls(&core);
        core.github_poller.poll_once(&core);
        let listed = crate::api::list_trails(&core, false).unwrap().trails.remove(0);
        assert_eq!(listed.state, bridge_protocol::messages::TrailState::NeedsYou);
        assert_eq!(listed.needs_you_reason.as_deref(), Some("test is failing on #12"));
        // Fixed and merged.
        std::fs::write(scratch.path().join("checks.json"), r#"[{"name":"test","state":"SUCCESS","bucket":"pass","link":"","workflow":"CI"}]"#).unwrap();
        let path = scratch.path().join("brief.json");
        let mut brief: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        brief["state"] = serde_json::json!("MERGED");
        std::fs::write(&path, brief.to_string()).unwrap();
        core.github_poller.due_now();
        core.github_poller.poll_once(&core);
        assert_eq!(state(&core), bridge_protocol::messages::TrailState::Shipped);
        assert_eq!(arm_trail_pulls(&core), 0, "a merged PR is no longer armed");
    }
    /// The session PR fixture, with a `gh` that also answers closing
    /// references and issue reads.
    fn fixture_with_issues() -> (tempfile::TempDir, Arc<BridgeCore>) {
        use std::os::unix::fs::PermissionsExt;
        let (scratch, core, _repo) = crate::session_prs::tests::fixture();
        std::fs::write(scratch.path().join("closing.json"), r#"{"closingIssuesReferences":[{"url":"https://github.com/o/r/issues/41"}]}"#).unwrap();
        std::fs::write(
            scratch.path().join("issue.json"),
            r#"{"number":41,"title":"Archived chats stay searchable","body":"For 90 days.","state":"OPEN","stateReason":"","author":{"login":"mika"},
                "labels":[],"comments":[],"createdAt":"2026-11-09T09:00:00Z","updatedAt":"2026-11-09T09:00:00Z","closedAt":null,
                "url":"https://github.com/o/r/issues/41","closedByPullRequestsReferences":[{"url":"https://github.com/o/r/pull/12"}]}"#,
        )
        .unwrap();
        let gh = scratch.path().join("gh");
        std::fs::write(
            &gh,
            r#"#!/bin/sh
root=$(dirname "$0")
case "$1 $2" in
  "auth status") exit 0 ;;
  "pr view") case "$*" in *closingIssuesReferences*) cat "$root/closing.json" ;; *) cat "$root/brief.json" ;; esac ;;
  "pr checks") cat "$root/checks.json" ;;
  "issue view") cat "$root/issue.json" ;;
  *) exit 1 ;;
esac
"#,
        )
        .unwrap();
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
        (scratch, core)
    }

    #[test]
    fn an_issue_a_read_brings_in_is_read_in_the_same_pass() {
        use crate::github_surface::PullRequestState;
        let (_scratch, core) = fixture_with_issues();
        let trail = trail_with_pr(&core, &crate::trails::timeline::tests::view(PullRequestState::Merged, vec![]));
        // One pass: the PR's closing reference is read, the sweep adds the
        // issue, and the issue itself is read too, with no other event.
        read_github(&core);
        let db = core.db.lock().unwrap();
        let members = crate::trails::store::members(&db, &trail).unwrap();
        assert!(members.iter().any(|member| member.kind == TrailMemberKind::Issue && member.reference == "https://github.com/o/r/issues/41"), "{members:?}");
        assert!(crate::trails::issues::get(&db, "https://github.com/o/r/issues/41").unwrap().is_some(), "the issue that joined must have been read");
        let (issues, pulls) = crate::trails::issues::missing(&db).unwrap();
        assert!(issues.is_empty() && pulls.is_empty(), "nothing left unread: {issues:?} {pulls:?}");
    }
    #[test]
    fn a_failed_read_wakes_the_loop_when_it_is_due() {
        // `for_tests` has no `gh`: every GitHub read fails, as offline.
        let dir = tempfile::tempdir().unwrap();
        let core = Arc::new(BridgeCore::for_tests(dir.path()));
        let url = "https://github.com/o/r/issues/77";
        {
            let db = core.db.lock().unwrap();
            let trail = crate::trails::store::create(&db, "T", None, "t0").unwrap();
            crate::trails::store::add_member(&db, &trail.id, &TrailMemberRef { kind: TrailMemberKind::Issue, reference: url.into() }, TrailAddedBy::Person, "t0").unwrap();
        }
        start_trail_maintenance(core.clone());
        let until = |done: &dyn Fn() -> bool| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while std::time::Instant::now() < deadline {
                if done() {
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            false
        };
        assert!(until(&|| core.trail_reads.failed_at(url).is_some()), "the first read fails offline");
        // The backoff runs out with nothing published at all.
        core.trail_reads.backdate(url, crate::trails::issues::RETRY_AFTER + std::time::Duration::from_secs(1));
        let backdated = core.trail_reads.failed_at(url).unwrap();
        assert!(until(&|| core.trail_reads.failed_at(url).is_some_and(|at| at > backdated)), "the due read must be tried again without an event");
    }
    #[test]
    fn arming_tries_every_attachment_of_a_trail_pr() {
        // The same PR attached from two workspaces; archiving one leaves its
        // link row behind. Whichever is archived, the live one is armed.
        for archived in ["w1", "w2"] {
            let dir = tempfile::tempdir().unwrap();
            let core = Arc::new(BridgeCore::for_tests(dir.path()));
            {
                let db = core.db.lock().unwrap();
                db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','P',?1,'now')", rusqlite::params![dir.path().to_str().unwrap()]).unwrap();
                for (workspace, session) in [("w1", "s1"), ("w2", "s2")] {
                    let path = dir.path().join(workspace);
                    std::fs::create_dir_all(&path).unwrap();
                    db.execute(
                        "INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES(?1,'p','c','t','main',?2,'idle','now')",
                        rusqlite::params![workspace, path.to_str().unwrap()],
                    )
                    .unwrap();
                    db.execute(
                        "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,depth,title) VALUES(?1,?2,'codex','Chat','idle','estimated','direct',0,'Chat')",
                        rusqlite::params![session, workspace],
                    )
                    .unwrap();
                    db.execute(
                        "INSERT INTO session_pull_requests(session_id,workspace_id,repo_host,repo_owner,repo_name,number,url,title,head_branch,head_sha,attribution,created_at)
                         VALUES(?1,?2,'github.com','o','r',12,'https://github.com/o/r/pull/12','PR','feat/pr','abc','manual','now')",
                        rusqlite::params![session, workspace],
                    )
                    .unwrap();
                }
                let trail = crate::trails::store::create(&db, "T", None, "t0").unwrap();
                crate::trails::store::add_member(&db, &trail.id, &TrailMemberRef { kind: TrailMemberKind::PullRequest, reference: "https://github.com/o/r/pull/12".into() }, TrailAddedBy::Person, "t0").unwrap();
                crate::workspaces::archive_workspace_records(&db, archived, 0, || Ok(())).unwrap();
            }
            let live = if archived == "w1" { "w2" } else { "w1" };
            assert_eq!(arm_trail_pulls(&core), 1, "archived {archived}");
            assert!(core.github_poller.is_watching(live, 12), "archived {archived}: the PR is armed under {live}");
            assert!(!core.github_poller.is_watching(archived, 12));
        }
    }

    #[test]
    fn a_chat_that_joins_mid_sweep_brings_its_linked_note() {
        let dir = tempfile::tempdir().unwrap();
        let core = Arc::new(BridgeCore::for_tests(dir.path()));
        let note = core
            .notes
            .create(crate::notes::service::CreatePage { title: Some("Fork plan".into()), ..Default::default() }, "person")
            .unwrap();
        core.notes
            .update(crate::notes::service::UpdateNote { id: note.id.clone(), add_session_id: Some("fork".into()), ..Default::default() }, "person")
            .unwrap();
        {
            let db = core.db.lock().unwrap();
            crate::trails::timeline::tests::add_session(&db, "s1", "codex", "Build", None);
            crate::trails::timeline::tests::add_session(&db, "fork", "claude", "Try again", None);
            db.execute("UPDATE sessions SET fork_parent_session_id='s1' WHERE id='fork'", []).unwrap();
            let trail = crate::trails::store::create(&db, "T", None, "t0").unwrap();
            crate::trails::store::add_member(&db, &trail.id, &TrailMemberRef { kind: TrailMemberKind::Session, reference: "s1".into() }, TrailAddedBy::Person, "t0").unwrap();
        }
        // Before the fork joined, its note was not in the snapshot; one call
        // still brings both.
        let added = sweep_and_publish(&core, false);
        let kinds: Vec<(TrailMemberKind, String)> = added.iter().map(|item| (item.member.kind, item.member.reference.clone())).collect();
        assert_eq!(kinds, [(TrailMemberKind::Session, "fork".to_owned()), (TrailMemberKind::Note, note.id.clone())]);
        assert_eq!(added[1].label, "Note “Fork plan”");
    }
}
