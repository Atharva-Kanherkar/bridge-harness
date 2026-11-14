//! Does a chat started on a trail answer from the trail?
//!
//! Criterion 4 of the Trails issue: a chat started on a trail, on a harness no
//! earlier chat used, answers "what did we decide about undo?" without the
//! person restating it. The fixture spreads arbitrary facts (an 8-second undo,
//! a 14-day reclaim, a shortcut dropped because of Arc) across two earlier
//! chats on other harnesses, a plan note, a PR snapshot, a draft and an issue,
//! so a guess cannot pass. Each question goes to a fresh chat:
//!
//! - **treatment**: the default brief carried through `carry_trail_brief`, the
//!   instructions compiled the way a cold direct start compiles them;
//! - **control**: the same question, nothing carried.
//!
//! Scored on the final reply, never on what the agent says it would do.
//!
//! ```text
//! BRIDGE_TRAILS_EVAL=1 cargo test -p bridge-core --lib trails::eval::live -- --ignored --nocapture
//! ```
//!
//! `BRIDGE_TRAILS_EVAL_HARNESS` picks the harness under test (default
//! `claude`), `BRIDGE_TRAILS_EVAL_MODEL` its model, `BRIDGE_TRAILS_EVAL_ONLY`
//! one case by index. Run it in a dedicated `CARGO_TARGET_DIR`.

use std::sync::Arc;

use bridge_protocol::messages::{CarryTrailBriefParams, OutgoingDestination, TrailAddedBy, TrailMemberKind, TrailMemberRef};

use crate::github_surface::{CheckConclusion, PullRequestState};
use crate::runtime::BridgeCore;
use crate::session_forest::{EntryKind, SessionForest};

/// One question and what its answer must contain. Every group must match;
/// a group matches when any of its phrases appears as whole words.
pub(crate) struct Case {
    pub ask: &'static str,
    pub all: &'static [&'static [&'static str]],
}

pub(crate) fn cases() -> Vec<Case> {
    vec![
        Case { ask: "what did we decide about undo?", all: &[&["8 seconds", "8 second", "8-second", "eight seconds", "8s", "8 s"], &["toast"]] },
        Case {
            ask: "how long do we keep an archived chat's worktree before it gets reclaimed?",
            all: &[&["14 days", "14-day", "fourteen days", "two weeks", "2 weeks"]],
        },
        Case { ask: "which check is failing on the PR right now?", all: &[&["vitest"]] },
        Case {
            ask: "did I already send the message to Yamada?",
            // A control that says "I can't say whether it went out" must not
            // pass: the reply has to know the draft exists and is unsent.
            all: &[
                &["draft", "drafted", "drafts"],
                &[
                    "unsent", "not sent", "not been sent", "hasn't been sent", "has not been sent", "haven't sent", "have not sent", "not yet sent",
                    "hasn't gone out", "has not gone out", "not sent yet", "never sent",
                ],
            ],
        },
        Case { ask: "why did we drop the keyboard shortcut for archiving?", all: &[&["arc"]] },
        Case { ask: "where does the archive button go in a sidebar row?", all: &[&["before"], &["harness"]] },
    ]
}

/// The facts the fixture plants, each in exactly one record.
pub(crate) const FACTS: &[&str] = &["8 seconds", "14 days", "collides with Arc", "before the harness mark", "vitest (macos)", "@yamada"];

pub(crate) const TRAIL_TITLE: &str = "Archive a chat";

/// `phrase` in `text` as whole words, ignoring case and curly apostrophes.
pub(crate) fn mentions(text: &str, phrase: &str) -> bool {
    let text = text.to_lowercase().replace(['\u{2019}', '\u{2018}'], "'");
    let phrase = phrase.to_lowercase();
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric());
    text.match_indices(&phrase).any(|(start, _)| !word(text[..start].chars().next_back()) && !word(text[start + phrase.len()..].chars().next()))
}

pub(crate) fn passes(case: &Case, reply: &str) -> bool {
    case.all.iter().all(|group| group.iter().any(|phrase| mentions(reply, phrase)))
}

/// The two harnesses the earlier chats ran on: never the one under test.
pub(crate) fn earlier_harnesses(under_test: &str) -> (String, String) {
    let others: Vec<&str> = ["codex", "opencode", "claude"].into_iter().filter(|harness| *harness != under_test).collect();
    (others[0].to_owned(), others[1].to_owned())
}

fn chat(db: &rusqlite::Connection, id: &str, harness: &str, title: &str, started: &str, turns: &[(&str, &str)]) {
    crate::trails::timeline::tests::add_session(db, id, harness, title, Some(started));
    let forest = SessionForest::new(db);
    for (user, assistant) in turns {
        forest.append(id, EntryKind::UserMessage, serde_json::json!({ "text": user })).unwrap();
        forest.append(id, EntryKind::AssistantMessage, serde_json::json!({ "text": assistant })).unwrap();
    }
}

/// A trail with two earlier chats on other harnesses, a plan note, a PR with a
/// failing check and an unsent draft. Returns the trail id.
pub(crate) fn seed(core: &Arc<BridgeCore>, under_test: &str) -> String {
    let (first, second) = earlier_harnesses(under_test);
    let note = core
        .notes
        .create(
            crate::notes::service::CreatePage {
                title: Some("Plan: archive a chat".into()),
                content: Some(
                    "## Decisions\n\n- Archiving a chat keeps its worktree for 14 days, then Bridge reclaims it.\n- Archived chats leave the sidebar but keep their history.\n\n## Open\n\n- Copy for the confirmation toast.\n"
                        .into(),
                ),
                ..Default::default()
            },
            "person",
        )
        .unwrap();
    let db = core.db.lock().unwrap();
    chat(
        &db,
        "chat-a",
        &first,
        "Build the archive action",
        "2026-11-10T09:00:00Z",
        &[
            (
                "Should undo for archive be a confirm dialog or something lighter?",
                "No confirm dialog. Archive happens at once and a toast offers Undo. We settled on the toast staying up for 8 seconds, long enough to catch a misclick without lingering.",
            ),
            ("ok go with that", "Done: the toast with Undo now stays for 8 seconds, and pressing Undo restores the chat to its old place in the sidebar."),
        ],
    );
    chat(
        &db,
        "chat-b",
        &second,
        "Archive button placement",
        "2026-11-11T10:00:00Z",
        &[
            (
                "Can archive have a keyboard shortcut, cmd shift A?",
                "I would drop it: cmd shift A collides with Arc, which many people run beside Bridge, and Arc swallows it first. Archive stays a button only.",
            ),
            (
                "fine. where should the button sit in the row?",
                "Put the archive button before the harness mark, so the mark stays the last thing in the row and lines up across chats.",
            ),
        ],
    );
    let pull = crate::trails::timeline::tests::view(
        PullRequestState::Open,
        vec![("vitest (macos)", Some(CheckConclusion::Failure)), ("lint", Some(CheckConclusion::Success))],
    );
    crate::trails::timeline::tests::attach_pr(&db, "chat-a", "https://github.com/o/r/pull/12", "2026-11-12T08:00:00Z", Some(&pull));
    let draft = crate::outgoing::create_draft(
        &db,
        "chat-a",
        OutgoingDestination::SlackDm { user: "@yamada".into() },
        "Could you review the archive PR today? Undo is a toast now.",
        "ask for review",
        &[],
        "2026-11-12T09:00:00Z",
    )
    .unwrap();
    let trail = crate::trails::store::create(&db, TRAIL_TITLE, None, "2026-11-10T08:00:00Z").unwrap();
    for (kind, reference) in [
        (TrailMemberKind::Session, "chat-a".to_owned()),
        (TrailMemberKind::Session, "chat-b".to_owned()),
        (TrailMemberKind::Note, note.id.clone()),
        (TrailMemberKind::PullRequest, "https://github.com/o/r/pull/12".to_owned()),
        (TrailMemberKind::Outgoing, draft.id.clone()),
    ] {
        crate::trails::store::add_member(&db, &trail.id, &TrailMemberRef { kind, reference }, TrailAddedBy::Person, "2026-11-12T10:00:00Z").unwrap();
    }
    trail.id
}

/// A new, empty chat on `harness`; with `trail`, the default brief carried in.
pub(crate) fn new_chat(core: &Arc<BridgeCore>, id: &str, harness: &str, trail: Option<&str>) -> String {
    {
        let db = core.db.lock().unwrap();
        crate::trails::timeline::tests::add_session(&db, id, harness, "New chat", None);
    }
    if let Some(trail_id) = trail {
        let preview = crate::api::preview_trail_brief(core, trail_id).unwrap();
        let items = preview.items.iter().filter(|item| item.default_on).map(|item| item.id.clone()).collect();
        crate::api::carry_trail_brief(core, &CarryTrailBriefParams { trail_id: trail_id.into(), session_id: id.into(), items }).unwrap();
    }
    crate::live_turn::direct_cold_start_instructions(core, id, harness).unwrap()
}

#[test]
fn fixture_brief_carries_every_fact() {
    let dir = tempfile::tempdir().unwrap();
    let core = Arc::new(BridgeCore::for_tests(dir.path()));
    let trail = seed(&core, "claude");
    let treated = new_chat(&core, "treated", "claude", Some(&trail));
    let control = new_chat(&core, "control", "claude", None);
    for fact in FACTS {
        assert!(treated.contains(fact), "the carried brief lost {fact:?}");
        assert!(!control.contains(fact), "the control already knows {fact:?}");
    }
    // The earlier chats ran elsewhere: the chat under test inherits them.
    let db = core.db.lock().unwrap();
    let harnesses: Vec<String> = db
        .prepare("SELECT harness FROM sessions WHERE id IN ('chat-a','chat-b')")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(!harnesses.iter().any(|harness| harness == "claude"), "{harnesses:?}");
}

#[test]
fn scoring_matches_whole_words_only() {
    assert!(mentions("It collides with Arc.", "arc"));
    assert!(!mentions("Archive stays a button", "arc"));
    assert!(mentions("The toast stays 8 seconds.", "8 seconds"));
    assert!(!mentions("18 seconds", "8 seconds"));
    let undo = &cases()[0];
    assert!(passes(undo, "A toast with Undo, shown for 8 seconds."));
    assert!(!passes(undo, "A toast with Undo."));
    // Codex writes typographic apostrophes.
    assert!(passes(&cases()[3], "No\u{2014}the message to @yamada was drafted but hasn\u{2019}t been sent"));
    assert!(!passes(&cases()[3], "I can\u{2019}t say whether it went out."));
}

/// What a real chat said. A chat that stalls (most often on a tool approval
/// nobody answers) is scored on what it said before the deadline.
struct Reply {
    text: String,
    stalled: Option<String>,
}

/// One reply from a real chat, or why the chat never ran.
fn ask(registry: &crate::adapters::AdapterRegistry, harness: &str, model: Option<&str>, cwd: &str, session: &str, instructions: &str, question: &str) -> Result<Reply, String> {
    use std::io::BufRead;
    let started = registry
        .start(
            harness,
            crate::adapters::StartRequest {
                session_id: Some(session),
                cwd,
                model,
                effort: None,
                instructions: Some(instructions),
                write_mode: Some(crate::delegation::WriteMode::Shared),
                read_only_sandbox: None,
                briefing: None,
                interactive: true,
                account: None,
                on_progress: None,
            },
        )
        .map_err(|error| format!("could not start: {error}"))?;
    let mut runtime = started.runtime;
    runtime.send_turn(question).map_err(|error| format!("could not send: {error}"))?;
    let (sender, receiver) = std::sync::mpsc::channel::<String>();
    let reader = started.reader;
    std::thread::spawn(move || {
        for line in reader.lines().map_while(Result::ok) {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(240);
    let mut text = String::new();
    let mut waiting_on: Option<String> = None;
    let outcome = loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let line = match receiver.recv_timeout(left) {
            Ok(line) => line,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let why = waiting_on.clone().map(|what| format!("waiting on {what}")).unwrap_or_else(|| "timed out".into());
                break Ok(Reply { text: text.clone(), stalled: Some(why) });
            }
            // A provider that exits mid-turn is broken, not slow.
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break Err(format!("the provider exited before the turn ended; said so far: {text}")),
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        let mut done = false;
        for event in registry.normalize(harness, &value) {
            if event.kind == "message.completed" && event.role.as_deref() == Some("assistant") {
                text = event.text.clone().unwrap_or_default();
            }
            if event.kind.ends_with(".requested") {
                waiting_on = Some(format!("{} {}", event.kind, event.title.clone().unwrap_or_default()));
            }
            done |= event.kind == "turn.completed";
        }
        if done {
            break Ok(Reply { text: text.clone(), stalled: None });
        }
    };
    runtime.stop(crate::adapters::ShutdownReason::Completed);
    outcome
}

#[test]
#[ignore = "runs real chats"]
fn live() {
    assert!(std::env::var_os("BRIDGE_TRAILS_EVAL").is_some(), "set BRIDGE_TRAILS_EVAL=1");
    let harness = std::env::var("BRIDGE_TRAILS_EVAL_HARNESS").unwrap_or_else(|_| "claude".into());
    let model = std::env::var("BRIDGE_TRAILS_EVAL_MODEL").ok().filter(|model| !model.is_empty());
    let only: Option<usize> = std::env::var("BRIDGE_TRAILS_EVAL_ONLY").ok().and_then(|value| value.parse().ok());
    let dir = tempfile::tempdir_in("/tmp").unwrap();
    let registry = Arc::new(crate::adapters::AdapterRegistry::built_in().expect("adapters"));
    // Discovery runs in the background; give it a moment, as the other live evals do.
    std::thread::sleep(std::time::Duration::from_secs(8));
    let mut core = BridgeCore::for_tests(dir.path());
    core.adapter_registry = registry.clone();
    let core = Arc::new(core);
    let trail = seed(&core, &harness);
    let preview = crate::api::preview_trail_brief(&core, &trail).unwrap();
    let (first, second) = earlier_harnesses(&harness);
    println!("trail brief: {} items, {} of {} tokens; earlier chats on {first} and {second}", preview.items.len(), preview.total_tokens, preview.budget_tokens);
    let cwd = dir.path().join("cwd");
    std::fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.to_string_lossy().to_string();

    let cases = cases();
    let (mut treated_ok, mut control_ok, mut started, mut stalled, mut asked) = (0, 0, 0, 0, 0);
    for (index, case) in cases.iter().enumerate() {
        if only.is_some_and(|only| only != index) {
            continue;
        }
        asked += 1;
        for (arm, carried) in [("treatment", true), ("control", false)] {
            let session = format!("{arm}-{index}");
            let instructions = new_chat(&core, &session, &harness, carried.then_some(trail.as_str()));
            match ask(&registry, &harness, model.as_deref(), &cwd, &session, &instructions, case.ask) {
                Ok(Reply { text: reply, stalled: stall }) => {
                    started += 1;
                    stalled += usize::from(stall.is_some());
                    let ok = passes(case, &reply);
                    if carried {
                        treated_ok += usize::from(ok);
                    } else {
                        control_ok += usize::from(ok);
                    }
                    let mark = match (carried, ok) {
                        (true, true) | (false, false) => "ok  ",
                        _ => "MISS",
                    };
                    let stall = stall.map(|why| format!("  (stalled, {why})")).unwrap_or_default();
                    println!("{mark} {arm:<9} {}{}{stall}", case.ask, if ok { "" } else { "  (no match)" });
                    if carried != ok || std::env::var_os("BRIDGE_TRAILS_EVAL_VERBOSE").is_some() {
                        println!("       said: {}", crate::notes::guards::clip(&reply.replace('\n', " "), 400));
                    }
                }
                Err(error) => println!("FAIL {arm:<9} {}: {error}", case.ask),
            }
        }
    }
    println!(
        "\ntrail recall on {harness} {}: treatment {treated_ok}/{asked}, control {control_ok}/{asked}, chats started {started}/{}, stalled {stalled}",
        model.as_deref().unwrap_or("(default model)"),
        asked * 2
    );
    assert_eq!(started, asked * 2, "every chat must start; a broken harness is a failure, not a skip");
    assert!(treated_ok + 1 >= asked, "treatment {treated_ok}/{asked}: the gate is all but one");
    assert!(control_ok <= 1, "control {control_ok}/{asked}: the facts are guessable, so the eval measures nothing");
}
