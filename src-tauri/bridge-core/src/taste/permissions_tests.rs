use super::*;
use serde_json::json;

fn open() -> (tempfile::TempDir, Connection) {
    let scratch = tempfile::tempdir().unwrap();
    let db = crate::store::open(&scratch.path().join("bridge.db")).unwrap();
    for chat in ["a", "b", "c"] {
        db.execute(
            "INSERT INTO sessions(id,harness,label,status,metric_source,kind) VALUES(?1,'claude','Chat','idle','estimated','direct')",
            params![chat],
        )
        .unwrap();
    }
    (scratch, db)
}

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-28T12:00:00Z").unwrap().with_timezone(&Utc)
}

/// Answer `count` requests for `command` in `chat`, `days_ago`.
fn answer(db: &Connection, chat: &str, first_event: i64, count: i64, command: &str, decision: &str, days_ago: i64) {
    let request = json!({"tool_name": "Bash", "tool_input": {"command": command}});
    for event in first_event..first_event + count {
        let at = (now() - Duration::days(days_ago)).to_rfc3339();
        signals::record_permission(db, chat, event, "claude", &request, decision, &at).unwrap();
    }
}

fn signal(tool: &str, body: Option<&str>, path: Option<&str>) -> Signal {
    Signal {
        id: "s".into(), at: "t".into(), scope_key: "account:local".into(), source: signals::SOURCE_PERMISSION.into(),
        verdict: "allowed".into(), subject_id: "x".into(), harness: None, path: path.map(str::to_owned), language: None,
        severity: None, category: None, title: Some(tool.into()), body: body.map(str::to_owned), edited_body: None,
        fingerprint: None, job_id: None, pr_number: None, session_id: None, detail: None, superseded_by: None,
    }
}

#[test]
fn a_request_is_its_tool_and_a_short_pattern() {
    let pattern = |tool: &str, body: Option<&str>, path: Option<&str>| pattern_of(&signal(tool, body, path)).map(|(_, pattern)| pattern);
    assert_eq!(pattern("Bash", Some("cargo test -p bridge-core --lib"), None).as_deref(), Some("cargo test"));
    assert_eq!(pattern("Bash", Some("RUST_LOG=debug cargo test"), None).as_deref(), Some("cargo test"), "env assignments skipped");
    assert_eq!(pattern("Bash", Some("ls -la"), None).as_deref(), Some("ls"), "a flag is not a subcommand");
    assert_eq!(pattern("Edit", None, Some("src/taste/learn.rs")).as_deref(), Some("src/**"));
    assert_eq!(pattern("Write", None, Some("./README.md")).as_deref(), Some("README.md"));
    assert_eq!(pattern("WebSearch", None, None).as_deref(), Some(""));
    assert_eq!(pattern("Bash", Some("   "), None), None);
}

#[test]
fn allows_are_never_suggested_for_network_destructive_or_credential_commands() {
    for pattern in ["curl", "git push", "git reset", "rm", "sudo", "npm publish", "cat .env", "ssh", "printenv", "bash"] {
        assert!(risky(pattern), "{pattern}");
    }
    for pattern in ["cargo test", "bun run", "git status", "git diff", "ls", "src/**", "rg"] {
        assert!(!risky(pattern), "{pattern}");
    }
}

#[test]
fn repeated_answers_across_chats_are_suggested_and_nothing_less() {
    let (_scratch, db) = open();
    answer(&db, "a", 1, 3, "cargo test --lib", "accept", 5);
    answer(&db, "b", 1, 3, "cargo test -p bridge-core", "acceptForSession", 2);
    answer(&db, "a", 10, 6, "bun run lint", "accept", 3); // one chat only
    answer(&db, "a", 20, 3, "git push origin main", "decline", 4);
    answer(&db, "b", 20, 2, "git push --force", "decline", 1);
    answer(&db, "a", 30, 3, "git push", "accept", 50); // allowed long ago, risky anyway
    let found = suggestions(&db, now()).unwrap();
    let summary: Vec<(String, String, usize, usize)> =
        found.iter().map(|item| (item.verdict.clone(), item.pattern.clone(), item.count, item.sessions)).collect();
    assert_eq!(summary, [("allowed".to_string(), "cargo test".to_string(), 6, 2), ("declined".to_string(), "git push".to_string(), 5, 2)]);
    assert_eq!(found[0].scope_key, "account:local");
}

#[test]
fn a_recent_opposite_answer_or_a_risky_allow_is_not_suggested() {
    let (_scratch, db) = open();
    answer(&db, "a", 1, 3, "cargo build", "accept", 10);
    answer(&db, "b", 1, 3, "cargo build", "accept", 9);
    answer(&db, "c", 1, 1, "cargo build", "decline", 2);
    answer(&db, "a", 10, 3, "curl https://example.com", "accept", 3);
    answer(&db, "b", 10, 3, "curl https://example.com", "accept", 3);
    assert!(suggestions(&db, now()).unwrap().is_empty());
    assert_eq!(suggestions(&db, now() + Duration::days(40)).unwrap().len(), 1, "the decline is old a month later");
}

#[test]
fn a_dismissed_suggestion_stays_dismissed_and_at_most_three_show() {
    let (_scratch, db) = open();
    for (index, command) in ["cargo test", "cargo check", "bun test", "bun run build"].iter().enumerate() {
        let base = (index as i64) * 100;
        answer(&db, "a", base, 3, command, "accept", 1);
        answer(&db, "b", base, 3, command, "accept", 1);
    }
    let found = suggestions(&db, now()).unwrap();
    assert_eq!(found.len(), MAX_SUGGESTIONS);
    dismiss(&db, &found[0].key, now()).unwrap();
    dismiss(&db, &found[0].key, now()).unwrap();
    let after = suggestions(&db, now()).unwrap();
    assert!(after.iter().all(|item| item.key != found[0].key));
    assert_eq!(after.len(), MAX_SUGGESTIONS, "the fourth moves up");
    assert!(dismiss(&db, " ", now()).is_err());
}
