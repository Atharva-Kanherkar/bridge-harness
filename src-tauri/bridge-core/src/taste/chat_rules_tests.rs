use super::*;
use crate::memory_packet::{self, Retrieval};
use crate::taste::replay::fixture;
use rusqlite::params;

fn open() -> (tempfile::TempDir, Connection) {
    let scratch = tempfile::tempdir().unwrap();
    let db = crate::store::open(&scratch.path().join("bridge.db")).unwrap();
    db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','P','/tmp/taste-p','now')", []).unwrap();
    (scratch, db)
}

/// A workspace at `path` and a chat in it; `reviewed` names its repository
/// through a review of `octo/repo`.
fn chat(db: &Connection, id: &str, workspace: &str, path: &str, reviewed: bool) {
    db.execute(
        "INSERT INTO workspaces(id,project_id,title,path,status,created_at) VALUES(?1,'p','T',?2,'idle','now')",
        params![workspace, path],
    )
    .unwrap();
    db.execute(
        "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind) VALUES(?1,?2,'codex','Chat','idle','estimated','direct')",
        params![id, workspace],
    )
    .unwrap();
    if reviewed {
        let (job, _) = fixture::review(db, "repo", 1, &format!("sha-{workspace}"), fixture::day(1.0), vec![], vec![]);
        db.execute("UPDATE review_jobs SET workspace_id=?2 WHERE id=?1", params![job, workspace]).unwrap();
    }
}

fn rule(db: &Connection, scope: &str, kind: &str, body: &str) -> String {
    crate::memory_ledger::save_taste_rule(db, scope, kind, body, None).unwrap().id
}

fn packet(db: &Connection, session: &str) -> Option<String> {
    memory_packet::for_session(db, "account:local", session, Retrieval::PacketOnly).unwrap().map(|packet| packet.text)
}

#[test]
fn a_chat_gets_its_repositorys_code_rules_and_every_repositorys_and_nothing_else() {
    let (_scratch, db) = open();
    chat(&db, "reviewed", "w1", "/tmp/taste-w1", true);
    crate::memory_ledger::save(&db, "Uses bun for scripts", None, None).unwrap();
    rule(&db, "account:local", CODE_KIND, "Early returns over nested ifs");
    rule(&db, "project:octo/repo", CODE_KIND, "No `unwrap()` outside tests");
    rule(&db, "project:octo/repo", "review_avoid", "Skip doc comment nits");
    rule(&db, "project:octo/other", CODE_KIND, "Tabs, not spaces");
    let text = packet(&db, "reviewed").unwrap();
    assert!(text.starts_with("Bridge account memory"), "memories first: {text}");
    let at = text.find(HEADING).expect("the code rules section");
    let rules: Vec<&str> = text[at + HEADING.len()..].lines().collect();
    assert_eq!(rules.len(), 2, "{text}");
    assert!(rules[0].ends_with("code rule: No `unwrap()` outside tests"), "the repository's own first: {rules:?}");
    assert!(rules[1].ends_with("code rule: Early returns over nested ifs"));
    assert!(!text.contains("doc comment") && !text.contains("Tabs"), "no review rules, no other repository's");

    let audit: String = db.query_row("SELECT selected_ids FROM memory_retrieval_audits ORDER BY rowid DESC LIMIT 1", [], |row| row.get(0)).unwrap();
    assert!(audit.contains("No `unwrap()` outside tests") && audit.contains("your code taste"), "the audit records what was sent");
}

#[test]
fn a_chat_with_no_code_rules_gets_the_packet_it_always_did() {
    let (_scratch, db) = open();
    chat(&db, "reviewed", "w1", "/tmp/taste-w1", true);
    crate::memory_ledger::save(&db, "Uses bun for scripts", None, None).unwrap();
    let before = packet(&db, "reviewed");
    rule(&db, "project:octo/repo", "review_avoid", "Skip doc comment nits");
    rule(&db, "project:octo/other", CODE_KIND, "Tabs, not spaces");
    assert_eq!(packet(&db, "reviewed"), before, "byte-identical");
}

#[test]
fn code_rules_alone_still_make_a_packet_and_the_memory_switch_turns_them_off() {
    let (_scratch, db) = open();
    chat(&db, "reviewed", "w1", "/tmp/taste-w1", true);
    let id = rule(&db, "project:octo/repo", CODE_KIND, "No `unwrap()` outside tests");
    let mut globbed = crate::memory_ledger::load_record(&db, &id).unwrap().unwrap();
    db.execute("UPDATE memory_records SET path_globs='[\"src/**\"]' WHERE id=?1", params![id]).unwrap();
    globbed.path_globs = Some(vec!["src/**".into()]);
    let text = packet(&db, "reviewed").unwrap();
    assert!(text.starts_with(HEADING), "no memory heading without memories: {text}");
    assert!(text.contains(&line(&globbed)) && text.contains("(paths: src/**)"));
    memory_packet::set_injection(&db, "account:local", false).unwrap();
    assert_eq!(packet(&db, "reviewed"), None, "off with memory");
}

#[test]
fn at_most_eight_code_rules_and_never_past_the_packet_budget() {
    let (_scratch, db) = open();
    chat(&db, "reviewed", "w1", "/tmp/taste-w1", true);
    for index in 0..12 {
        rule(&db, "project:octo/repo", CODE_KIND, &format!("Rule {index}: keep functions short"));
    }
    assert_eq!(for_chat(&db, "reviewed").unwrap().len(), MAX_TASTE_RULES_IN_PACKET);
    let (_scratch, db) = open();
    chat(&db, "reviewed", "w1", "/tmp/taste-w1", true);
    for index in 0..8 {
        rule(&db, "project:octo/repo", CODE_KIND, &format!("{index} {}", "very long rule text ".repeat(150)));
    }
    let text = packet(&db, "reviewed").unwrap();
    let lines = text.lines().filter(|line| line.contains("code rule")).count();
    assert!(lines > 0 && lines < 8, "the budget held: {lines} lines, {} chars", text.len());
}

#[test]
fn a_chat_in_an_unreviewed_checkout_is_placed_by_its_origin_remote() {
    let (_scratch, db) = open();
    let checkout = tempfile::tempdir().unwrap();
    std::fs::create_dir(checkout.path().join(".git")).unwrap();
    std::fs::write(checkout.path().join(".git/config"), "[remote \"origin\"]\n\turl = git@github.com:Octo/Repo.git\n").unwrap();
    chat(&db, "fresh", "w2", &checkout.path().to_string_lossy(), false);
    chat(&db, "elsewhere", "w3", "/tmp/taste-not-a-checkout", false);
    rule(&db, "project:octo/repo", CODE_KIND, "No `unwrap()` outside tests");
    assert_eq!(for_chat(&db, "fresh").unwrap().len(), 1);
    assert!(for_chat(&db, "elsewhere").unwrap().is_empty());
    assert_eq!(crate::taste::repository::scope_of_session(&db, "fresh").unwrap().as_deref(), Some("project:octo/repo"));
}

/// A real chat on a real harness, with and without an approved code rule for
/// its repository (#1204). The rule reaches the chat the way every launch
/// sends memory: the session context frame beside the first message. Run by
/// hand:
///
/// ```text
/// BRIDGE_TASTE_LIVE_HARNESS=claude \
///   cargo test -p bridge-core --lib taste::chat_rules::tests::live_chat_follows_code_rules -- --ignored --nocapture
/// ```
///
/// `BRIDGE_TASTE_LIVE_MODEL` picks a model (OpenCode Go: `opencode-go/glm-5.3`).
#[test]
#[ignore = "live: runs a real harness; set BRIDGE_TASTE_LIVE_HARNESS"]
fn live_chat_follows_code_rules() {
    use crate::adapters::{AdapterRegistry, ShutdownReason, StartRequest, TurnContext};
    use std::io::BufRead;
    let harness = std::env::var("BRIDGE_TASTE_LIVE_HARNESS").expect("set BRIDGE_TASTE_LIVE_HARNESS to claude, codex or opencode");
    let model = std::env::var("BRIDGE_TASTE_LIVE_MODEL").ok();
    let registry = std::sync::Arc::new(AdapterRegistry::built_in().unwrap());
    std::thread::sleep(std::time::Duration::from_secs(8));
    let ask = "Reply with one Rust code block and nothing else. Do not create files or run tools. \
               Write `fn load_port(path: &str)` that reads a config file and parses its first line as a TCP port.";
    let rule = "Fallible functions return `anyhow::Result`, and every `?` carries `.context(\"...\")` saying what failed.";
    let mut outcomes = Vec::new();
    for with_rule in [false, true] {
        let (_scratch, db) = open();
        chat(&db, "live", "w1", "/tmp/taste-live", true);
        if with_rule {
            crate::memory_ledger::save_taste_rule(&db, "project:octo/repo", CODE_KIND, rule, None).unwrap();
        }
        let frame = crate::session_context::build("", memory_packet::for_compile(&db, "live").unwrap().as_deref());
        assert_eq!(frame.as_ref().is_some_and(|frame| frame.text().contains("anyhow::Result")), with_rule, "the rule rides in the frame only when set");
        let cwd = tempfile::tempdir().unwrap();
        let cwd_text = cwd.path().to_string_lossy().to_string();
        let started = registry
            .start(
                &harness,
                StartRequest {
                    session_id: None,
                    cwd: &cwd_text,
                    model: model.as_deref(),
                    effort: None,
                    speed: None,
                    instructions: None,
                    write_mode: None,
                    read_only_sandbox: None,
                    briefing: None,
                    interactive: true,
                    account: None,
                    on_progress: None,
                },
            )
            .unwrap_or_else(|error| panic!("{harness} did not start: {error}"));
        let mut runtime = started.runtime;
        runtime
            .send_turn_with_context(ask, TurnContext { session: frame.as_ref().map(crate::session_context::SessionContext::text), credentials: None })
            .unwrap();
        let mut reply = String::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(240);
        let mut lines = started.reader.lines();
        while std::time::Instant::now() < deadline {
            let Some(Ok(line)) = lines.next() else { break };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
            let mut done = false;
            for event in registry.normalize(&harness, &value) {
                if event.kind == "message.completed" && event.role.as_deref() == Some("assistant") {
                    reply.push_str(event.text.as_deref().unwrap_or_default());
                }
                done |= event.kind == "turn.completed" || event.kind == "turn.failed";
            }
            if done {
                break;
            }
        }
        runtime.stop(ShutdownReason::Completed);
        let follows = reply.contains("anyhow") && reply.contains(".context(");
        eprintln!("{harness} {} follows={follows}\n{reply}\n", if with_rule { "with rule" } else { "plain    " });
        outcomes.push((with_rule, follows));
    }
    eprintln!("{harness}: (with rule, follows) {outcomes:?}");
    assert!(outcomes[1].1, "with the rule, the code follows it");
}
