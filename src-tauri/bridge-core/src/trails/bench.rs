//! How fast the trail reads are on a store shaped like a busy person's: many
//! trails with long chats, PR snapshots, notes, drafts and issues, plus the
//! unrelated records a real database holds. Not a gate: it prints timings so a
//! change to the reads can be compared before and after on the same machine.
//!
//! ```text
//! cargo test --release -p bridge-core --lib trails::bench -- --ignored --nocapture
//! ```
//!
//! The database is a file in WAL mode, as in the app, not `:memory:`.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bridge_protocol::messages::{OutgoingDestination, TrailAddedBy, TrailMemberKind, TrailMemberRef};

use crate::github_surface::{CheckConclusion, PullRequestState};
use crate::runtime::BridgeCore;
use crate::session_forest::{EntryKind, SessionForest};

pub(crate) const TRAILS: usize = 20;
pub(crate) const CHATS_PER_TRAIL: usize = 3;
/// User and assistant turns per chat.
pub(crate) const TURNS_PER_CHAT: usize = 300;
pub(crate) const OTHER_NOTES: usize = 1500;
pub(crate) const OTHER_CHATS: usize = 200;

fn paragraph(seed: usize) -> String {
    format!(
        "Turn {seed}: we looked at the archive flow again, compared the toast against a dialog, read the sidebar row code, \
         ran the vitest suite, and noted which checks were slow. The decision stays as it was; the follow-up is in the plan note. {}",
        "x".repeat(seed % 120)
    )
}

/// A core over a file database with `TRAILS` trails and the noise around them.
pub(crate) fn seeded(dir: &std::path::Path) -> (Arc<BridgeCore>, Vec<String>) {
    let mut core = BridgeCore::for_tests(dir);
    core.db = Mutex::new(crate::store::open(&dir.join("bridge.db")).unwrap());
    let core = Arc::new(core);
    let mut note_ids = Vec::new();
    for index in 0..OTHER_NOTES + TRAILS {
        let note = core
            .notes
            .create(
                crate::notes::service::CreatePage {
                    title: Some(format!("Note {index}")),
                    content: Some(format!("## Notes {index}\n\n{}\n", paragraph(index).repeat(4))),
                    ..Default::default()
                },
                "person",
            )
            .unwrap();
        note_ids.push(note.id);
    }
    let mut trails = Vec::new();
    let db = core.db.lock().unwrap();
    // The forest opens its own transactions; only seeding skips the fsyncs.
    let synchronous: i64 = db.query_row("PRAGMA synchronous", [], |row| row.get(0)).unwrap();
    db.execute_batch("PRAGMA synchronous=OFF").unwrap();
    let transaction = &*db;
    for other in 0..OTHER_CHATS {
        crate::trails::timeline::tests::add_session(transaction, &format!("other-{other}"), "codex", &format!("Other {other}"), Some("2026-11-01T00:00:00Z"));
        crate::trails::timeline::tests::attach_pr(transaction, &format!("other-{other}"), &format!("https://github.com/o/r/pull/{}", 5000 + other), "2026-11-01T00:00:00Z", None);
    }
    for trail_index in 0..TRAILS {
        let trail = crate::trails::store::create(transaction, &format!("Trail {trail_index}"), None, "2026-11-01T00:00:00Z").unwrap();
        let mut members = Vec::new();
        for chat in 0..CHATS_PER_TRAIL {
            let id = format!("t{trail_index}-c{chat}");
            let harness = ["claude", "codex", "opencode"][chat % 3];
            crate::trails::timeline::tests::add_session(transaction, &id, harness, &format!("Chat {chat} of trail {trail_index}"), Some("2026-11-02T00:00:00Z"));
            let forest = SessionForest::new(transaction);
            for turn in 0..TURNS_PER_CHAT {
                forest.append(&id, EntryKind::UserMessage, serde_json::json!({ "text": paragraph(turn) })).unwrap();
                forest.append(&id, EntryKind::AssistantMessage, serde_json::json!({ "text": paragraph(turn + 1).repeat(3) })).unwrap();
            }
            members.push(TrailMemberRef { kind: TrailMemberKind::Session, reference: id });
        }
        let url = format!("https://github.com/o/r/pull/{}", 100 + trail_index);
        let checks: Vec<(String, Option<CheckConclusion>)> = (0..12)
            .map(|check| (format!("check {check}"), Some(if check == 3 && trail_index % 2 == 0 { CheckConclusion::Failure } else { CheckConclusion::Success })))
            .collect();
        let view = crate::trails::timeline::tests::view(PullRequestState::Open, checks.iter().map(|(name, conclusion)| (name.as_str(), *conclusion)).collect());
        crate::trails::timeline::tests::attach_pr(transaction, &format!("t{trail_index}-c0"), &url, "2026-11-03T00:00:00Z", Some(&view));
        transaction.execute("INSERT INTO trail_pr_link_reads(pr_url,fetched_at) VALUES(?1,'2026-11-03T00:00:00Z')", rusqlite::params![url]).unwrap();
        members.push(TrailMemberRef { kind: TrailMemberKind::PullRequest, reference: url });
        let draft = crate::outgoing::create_draft(
            transaction,
            &format!("t{trail_index}-c0"),
            OutgoingDestination::SlackDm { user: "@mika".into() },
            "Could you review the archive PR today?",
            "review",
            &[],
            "2026-11-04T00:00:00Z",
        )
        .unwrap();
        members.push(TrailMemberRef { kind: TrailMemberKind::Outgoing, reference: draft.id });
        let issue = crate::trails::issues::tests::snapshot(2000 + trail_index as u64, true, None, &[]);
        crate::trails::issues::save(transaction, &issue).unwrap();
        members.push(TrailMemberRef { kind: TrailMemberKind::Issue, reference: issue.url });
        members.push(TrailMemberRef { kind: TrailMemberKind::Note, reference: note_ids[OTHER_NOTES + trail_index].clone() });
        for member in members {
            crate::trails::store::add_member(transaction, &trail.id, &member, TrailAddedBy::Person, "2026-11-05T00:00:00Z").unwrap();
        }
        trails.push(trail.id);
    }
    db.execute_batch(&format!("PRAGMA synchronous={synchronous}")).unwrap();
    drop(db);
    (core, trails)
}

/// Median and 90th percentile of `runs` warm calls, after three warm-ups.
fn time(runs: usize, mut call: impl FnMut()) -> (Duration, Duration) {
    for _ in 0..3 {
        call();
    }
    let mut samples: Vec<Duration> = (0..runs)
        .map(|_| {
            let start = Instant::now();
            call();
            start.elapsed()
        })
        .collect();
    samples.sort();
    (samples[runs / 2], samples[runs * 9 / 10])
}

#[test]
#[ignore = "timings, not a gate"]
fn reads() {
    let dir = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let (core, trails) = seeded(dir.path());
    println!(
        "seeded {TRAILS} trails x {CHATS_PER_TRAIL} chats x {} entries, {} notes, {OTHER_CHATS} other chats in {:.1?}",
        TURNS_PER_CHAT * 2,
        OTHER_NOTES + TRAILS,
        started.elapsed()
    );
    let middle = trails[TRAILS / 2].clone();
    let runs = 20;
    let rows: Vec<(&str, (Duration, Duration))> = vec![
        ("list_trails", time(runs, || drop(crate::api::list_trails(&core, false).unwrap()))),
        ("read_trail", time(runs, || drop(crate::api::read_trail(&core, &middle, false).unwrap()))),
        ("preview_trail_brief", time(runs, || drop(crate::api::preview_trail_brief(&core, &middle).unwrap()))),
        ("sweep", time(runs, || drop(crate::trails::live::sweep_and_publish(&core, false)))),
        (
            "loop batch (sweep, GitHub reads, arm)",
            time(runs, || {
                crate::trails::live::sweep_and_publish(&core, true);
                crate::trails::live::read_github(&core);
                crate::trails::live::arm_trail_pulls(&core);
            }),
        ),
    ];
    // Where the time goes, part by part.
    let notes = crate::trails::timeline::NoteSnapshot::all(core.notes.as_ref());
    let row = crate::trails::store::require(&core.db.lock().unwrap(), &middle).unwrap();
    let mut rows = rows;
    rows.extend([
        ("  part: NoteSnapshot::all", time(runs, || drop(crate::trails::timeline::NoteSnapshot::all(core.notes.as_ref())))),
        ("  part: auto::sweep (notes in hand)", time(runs, || drop(crate::trails::auto::sweep(&core.db.lock().unwrap(), &notes, "t").unwrap()))),
        ("  part: timeline::detail, one trail", time(runs, || drop(crate::trails::timeline::detail(&core.db.lock().unwrap(), &notes, &row).unwrap()))),
        (
            "  part: all summaries",
            time(runs, || {
                let db = core.db.lock().unwrap();
                for trail in crate::trails::store::list(&db, false).unwrap() {
                    drop(crate::trails::timeline::summary(&db, &notes, &trail).unwrap());
                }
            }),
        ),
        ("  part: brief::preview (notes in hand)", time(runs, || drop(crate::trails::brief::preview(&core.db.lock().unwrap(), &notes, &row).unwrap()))),
        ("  part: arm_trail_pulls", time(runs, || drop(crate::trails::live::arm_trail_pulls(&core)))),
    ]);
    println!("\n{:<40} {:>12} {:>12}", "call", "median", "p90");
    for (name, (median, p90)) in rows {
        println!("{name:<40} {:>12.2?} {:>12.2?}", median, p90);
    }
}
