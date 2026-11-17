use super::*;
use crate::import_bundle::{self, BundleSource, TransferBundle};
use serde_json::json;
use std::io::Write;
use proptest::prelude::*;

fn write_lines(path: &Path, lines: &[Value]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = File::create(path).unwrap();
    for line in lines {
        writeln!(file, "{line}").unwrap();
    }
}

fn user(uuid: &str, at: &str, content: Value) -> Value {
    json!({"type":"user","uuid":uuid,"sessionId":"sess-1","timestamp":at,"cwd":"/work/app","message":{"role":"user","content":content}})
}

fn assistant(uuid: &str, at: &str, blocks: Value) -> Value {
    json!({"type":"assistant","uuid":uuid,"sessionId":"sess-1","timestamp":at,"cwd":"/work/app","message":{"role":"assistant","model":"claude-opus-5-5","content":blocks}})
}

fn kinds(parsed: &ParsedTranscript) -> Vec<BundleEntryKind> {
    parsed.chat.entries.iter().map(|entry| entry.kind).collect()
}

#[test]
fn every_block_kind_maps_to_its_entry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("projects/-work-app/sess-1.jsonl");
    write_lines(&path, &[
        json!({"type":"queue-operation","operation":"enqueue","timestamp":"2026-11-01T09:59:59Z","sessionId":"sess-1"}),
        user("u1", "2026-11-01T10:00:00Z", json!("Fix the login test")),
        assistant("a1", "2026-11-01T10:00:01Z", json!([{"type":"thinking","thinking":"Check the clock mock.","signature":"x"}])),
        assistant("a2", "2026-11-01T10:00:02Z", json!([{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"npm test"}}])),
        user("u2", "2026-11-01T10:00:03Z", json!([{"type":"tool_result","tool_use_id":"toolu_1","content":"1 failing","is_error":true}])),
        assistant("a3", "2026-11-01T10:00:04Z", json!([{"type":"tool_use","id":"toolu_2","name":"Read","input":{"file_path":"logo.png"}}])),
        user("u3", "2026-11-01T10:00:05Z", json!([{"type":"tool_result","tool_use_id":"toolu_2","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"iVBORw0KGgo="}},{"type":"text","text":"read ok"}]}])),
        assistant("a4", "2026-11-01T10:00:06Z", json!([{"type":"text","text":"Fixed."}])),
        user("u4", "2026-11-01T10:00:07Z", json!([{"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":"/9j/4AAQ"}},{"type":"text","text":"what about this screen?"}])),
        json!({"type":"custom-title","customTitle":"Login test fix","sessionId":"sess-1"}),
        json!({"type":"ai-title","aiTitle":"Something else","sessionId":"sess-1"}),
    ]);
    let parsed = read_transcript(&path, "k", "label").unwrap();
    assert_eq!(kinds(&parsed), vec![
        BundleEntryKind::UserMessage,
        BundleEntryKind::Reasoning,
        BundleEntryKind::ToolCall,
        BundleEntryKind::ToolCall,
        BundleEntryKind::AssistantMessage,
        BundleEntryKind::UserMessage,
    ]);
    let bash = parsed.chat.entries[2].tool.as_ref().unwrap();
    assert_eq!((bash.name.as_str(), bash.status, bash.output.as_deref()), ("Bash", ToolStatus::Failed, Some("1 failing")));
    let read = parsed.chat.entries[3].tool.as_ref().unwrap();
    assert_eq!(read.output.as_deref(), Some("[image]\nread ok"));
    assert_eq!(parsed.chat.entries[5].attachments[0].media_type, "image/jpeg");
    assert_eq!(parsed.chat.title.as_deref(), Some("Login test fix"), "a /rename wins");
    assert_eq!(parsed.chat.title_source, TitleSource::User);
    assert_eq!(parsed.chat.cwd.as_deref(), Some("/work/app"));
    assert_eq!(parsed.chat.model.as_deref(), Some("claude-opus-5-5"));
    assert_eq!(parsed.chat.native_ref.as_ref().unwrap().session_id, "sess-1");
    assert_eq!(parsed.chat.created_at.as_deref(), Some("2026-11-01T10:00:00Z"));
    assert_eq!(parsed.chat.updated_at.as_deref(), Some("2026-11-01T10:00:07Z"));
    assert_eq!(parsed.source_items as usize, parsed.chat.entries.len(), "every source item is an entry");
    assert!(parsed.skipped.0.is_empty(), "{:?}", parsed.skipped);
}

#[test]
fn compaction_becomes_one_boundary_carrying_the_summary() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("projects/p/sess-1.jsonl");
    write_lines(&path, &[
        user("u1", "2026-11-01T10:00:00Z", json!("long task")),
        json!({"type":"system","subtype":"compact_boundary","uuid":"c1","timestamp":"2026-11-01T11:00:00Z","content":"Conversation compacted","compactMetadata":{"trigger":"auto","preTokens":180000}}),
        json!({"type":"user","uuid":"s1","timestamp":"2026-11-01T11:00:01Z","isCompactSummary":true,"isVisibleInTranscriptOnly":true,"message":{"role":"user","content":"This session is being continued. Summary: fixed auth."}}),
        user("u2", "2026-11-01T11:01:00Z", json!("carry on")),
        // A summary with no boundary before it still becomes a boundary.
        json!({"type":"user","uuid":"s2","timestamp":"2026-11-01T12:00:01Z","isCompactSummary":true,"message":{"role":"user","content":[{"type":"text","text":"Second summary"}]}}),
    ]);
    let parsed = read_transcript(&path, "k", "label").unwrap();
    assert_eq!(kinds(&parsed), vec![
        BundleEntryKind::UserMessage,
        BundleEntryKind::CompactionBoundary,
        BundleEntryKind::UserMessage,
        BundleEntryKind::CompactionBoundary,
    ]);
    assert_eq!(parsed.chat.entries[1].text.as_deref(), Some("This session is being continued. Summary: fixed auth."));
    assert_eq!(parsed.chat.entries[3].text.as_deref(), Some("Second summary"));
}

#[test]
fn meta_records_bookkeeping_and_slash_commands_are_handled() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("projects/p/sess-1.jsonl");
    write_lines(&path, &[
        json!({"type":"user","uuid":"m1","timestamp":"2026-11-01T10:00:00Z","isMeta":true,"message":{"role":"user","content":"Caveat: local command output"}}),
        user("u1", "2026-11-01T10:00:01Z", json!("<command-name>/model</command-name>\n<command-message>model</command-message>\n<command-args>opus</command-args>")),
        json!({"type":"system","subtype":"local_command","uuid":"l1","timestamp":"2026-11-01T10:00:02Z","content":"<local-command-stdout>Set model</local-command-stdout>"}),
        json!({"type":"cost-state","timestamp":"2026-11-01T10:00:03Z"}),
        json!({"type":"brand-new-record","timestamp":"2026-11-01T10:00:04Z"}),
        json!({"type":"system","subtype":"brand_new_subtype","uuid":"x","timestamp":"2026-11-01T10:00:05Z"}),
        json!("a bare string line"),
        json!({"no":"type"}),
    ]);
    std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"{not json\n\n").unwrap();
    let parsed = read_transcript(&path, "k", "label").unwrap();
    assert_eq!(parsed.chat.entries.len(), 1);
    assert_eq!(parsed.chat.entries[0].text.as_deref(), Some("/model opus"));
    let skipped = &parsed.skipped.0;
    for reason in ["record:brand-new-record", "system:brand_new_subtype", "non_object_line", "untyped_record", "malformed_line"] {
        assert_eq!(skipped.get(reason), Some(&1), "{reason} in {skipped:?}");
    }
    assert!(!skipped.contains_key("record:cost-state"), "bookkeeping is skipped silently");
}

#[test]
fn bad_timestamps_missing_ids_and_orphan_results_are_counted_not_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("projects/p/sess-1.jsonl");
    write_lines(&path, &[
        json!({"type":"user","timestamp":"2026-11-01T10:00:00Z","message":{"role":"user","content":"no uuid"}}),
        user("u1", "yesterday", json!("bad time")),
        user("u2", "2026-11-01T10:00:00Z", json!([{"type":"tool_result","tool_use_id":"never-called","content":"x"}])),
        assistant("a1", "2026-11-01T10:00:01Z", json!([{"type":"tool_use","id":"t","name":"Bash","input":{}}])),
        assistant("a1", "2026-11-01T10:00:01Z", json!([{"type":"text","text":"repeat of a1"}])),
        user("u3", "2026-11-01T10:00:02Z", json!("kept")),
    ]);
    let parsed = read_transcript(&path, "k", "label").unwrap();
    assert_eq!(kinds(&parsed), vec![BundleEntryKind::ToolCall, BundleEntryKind::UserMessage]);
    assert_eq!(parsed.chat.entries[0].tool.as_ref().unwrap().status, ToolStatus::Unknown, "a call with no result is interrupted");
    let skipped = &parsed.skipped.0;
    assert_eq!(skipped.get("missing_uuid"), Some(&1));
    assert_eq!(skipped.get("invalid_timestamp"), Some(&1));
    assert_eq!(skipped.get("tool_result_without_call"), Some(&1));
    assert_eq!(skipped.get("duplicate_source_id"), Some(&1));
}

#[test]
fn subagent_transcripts_become_child_chats() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("projects/p/sess-1.jsonl");
    write_lines(&main, &[
        user("u1", "2026-11-01T10:00:00Z", json!("map the repo")),
        assistant("a1", "2026-11-01T10:00:01Z", json!([{"type":"tool_use","id":"task","name":"Task","input":{"subagent_type":"Explore","prompt":"Map it"}}])),
    ]);
    let sub = dir.path().join("projects/p/sess-1/subagents/agent-abc.jsonl");
    write_lines(&sub, &[
        json!({"type":"user","uuid":"s1","sessionId":"sess-1","isSidechain":true,"agentId":"abc","timestamp":"2026-11-01T10:00:02Z","message":{"role":"user","content":"Map the repo and report key files"}}),
        json!({"type":"assistant","uuid":"s2","sessionId":"sess-1","isSidechain":true,"agentId":"abc","timestamp":"2026-11-01T10:00:03Z","message":{"role":"assistant","content":[{"type":"text","text":"src/ and test/ matter."}]}}),
    ]);
    std::fs::write(sub.with_extension("meta.json"), r#"{"agentType":"Explore","spawnDepth":1,"model":"claude-haiku-5-5"}"#).unwrap();
    std::fs::write(dir.path().join("projects/p/sess-1/subagents/notes.txt"), "not a transcript").unwrap();
    let parsed = read_session(&main, "projects/p/sess-1.jsonl", &[dir.path().canonicalize().unwrap()]).unwrap().chats;
    assert_eq!(parsed.len(), 2);
    let child = &parsed[1].chat;
    assert_eq!(child.parent_chat_key.as_deref(), Some(parsed[0].chat.key.as_str()));
    assert_eq!(child.agent_role, Some(AgentRole::Subagent));
    assert_eq!(child.title.as_deref(), Some("Explore: Map the repo and report key files"));
    assert_eq!(child.model.as_deref(), Some("claude-haiku-5-5"));
    assert_eq!(child.native_ref.as_ref().unwrap().session_id, "sess-1/agent-abc");
    assert_eq!(child.source_label, "projects/p/sess-1.jsonl > agent-abc");
}

#[test]
fn an_ungrounded_ai_title_is_not_used() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("projects/p/sess-1.jsonl");
    write_lines(&path, &[
        json!({"type":"ai-title","aiTitle":"Bridge session capabilities","sessionId":"sess-1"}),
        user("u1", "2026-11-01T10:00:00Z", json!("Why does the dynamo table throttle?")),
    ]);
    let parsed = read_transcript(&path, "k", "label").unwrap();
    assert_eq!(parsed.chat.title, None, "Bridge derives one from the first message instead");
    let grounded = dir.path().join("projects/p/sess-2.jsonl");
    write_lines(&grounded, &[
        json!({"type":"ai-title","aiTitle":"Dynamo table throttling","sessionId":"sess-2"}),
        user("u1", "2026-11-01T10:00:00Z", json!("Why does the dynamo table throttle?")),
    ]);
    assert_eq!(read_transcript(&grounded, "k", "label").unwrap().chat.title.as_deref(), Some("Dynamo table throttling"));
}

#[test]
fn peek_reads_title_folder_and_start_without_the_whole_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("projects/p/sess-1.jsonl");
    let mut lines = vec![user("u1", "2026-11-01T10:00:00Z", json!("Port the billing page"))];
    for index in 0..2_000 {
        lines.push(assistant(&format!("a{index}"), "2026-11-01T10:00:01Z", json!([{"type":"text","text":"x".repeat(100)}])));
    }
    lines.push(json!({"type":"custom-title","customTitle":"Billing port","sessionId":"sess-1"}));
    write_lines(&path, &lines);
    let peek = peek_session(&path);
    assert_eq!(peek.title.as_deref(), Some("Billing port"), "the rename near the end is found");
    assert_eq!(peek.cwd.as_deref(), Some("/work/app"));
    assert_eq!(peek.started_at.as_deref(), Some("2026-11-01T10:00:00Z"));
    assert_eq!(peek_session(&dir.path().join("missing.jsonl")), SessionPeek::default());
}

#[test]
fn a_huge_single_line_is_skipped_without_reading_it_into_one_string() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("projects/p/sess-1.jsonl");
    write_lines(&path, &[user("u1", "2026-11-01T10:00:00Z", json!("before"))]);
    {
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        let blob = vec![b'a'; MAX_LINE_BYTES + 10];
        file.write_all(&blob).unwrap();
        file.write_all(b"\n").unwrap();
        writeln!(file, "{}", user("u2", "2026-11-01T10:00:01Z", json!("after"))).unwrap();
    }
    let parsed = read_transcript(&path, "k", "label").unwrap();
    assert_eq!(parsed.chat.entries.len(), 2);
    assert_eq!(parsed.skipped.0.get("oversized_line"), Some(&1));
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig { cases: 200, ..Default::default() })]
    /// Fuzz: arbitrary bytes, arbitrary JSON values and mutated real-shaped
    /// records never panic the reader, and every entry it emits is well formed.
    #[test]
    fn arbitrary_lines_never_panic(
        lines in proptest::collection::vec(
            proptest::prop_oneof![
                proptest::collection::vec(proptest::prelude::any::<u8>(), 0..200).prop_map(|bytes| String::from_utf8_lossy(&bytes).replace('\n', " ")),
                "\\PC{0,120}",
                (0u8..6, "[a-z0-9]{0,6}", proptest::option::of("\\PC{0,30}"), proptest::prelude::any::<bool>()).prop_map(|(shape, id, text, flag)| {
                    let text = text.unwrap_or_default();
                    match shape {
                        0 => user(&id, "2026-11-01T10:00:00Z", json!(text)).to_string(),
                        1 => assistant(&id, "2026-11-01T10:00:01Z", json!([{"type":"tool_use","id":id,"name":text,"input":{"x":flag}}])).to_string(),
                        2 => user(&id, "2026-11-01T10:00:02Z", json!([{"type":"tool_result","tool_use_id":id,"content":text,"is_error":flag}])).to_string(),
                        3 => assistant(&id, "2026-11-01T10:00:03Z", json!([{"type":"thinking","thinking":text},{"type":"text","text":text}])).to_string(),
                        4 => json!({"type":"system","subtype":"compact_boundary","uuid":id,"timestamp":"2026-11-01T10:00:04Z"}).to_string(),
                        _ => json!({"type":"user","uuid":id,"timestamp":"2026-11-01T10:00:05Z","isCompactSummary":flag,"message":{"content":[{"type":"image","source":{"data":text}}]}}).to_string(),
                    }
                }),
            ],
            0..40,
        )
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("projects/p/s.jsonl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, lines.join("\n")).unwrap();
        let parsed = read_transcript(&path, "k", "l").unwrap();
        for entry in &parsed.chat.entries {
            proptest::prop_assert!(DateTime::parse_from_rfc3339(&entry.at).is_ok());
            proptest::prop_assert_eq!(entry.kind == BundleEntryKind::ToolCall, entry.tool.is_some());
        }
        let non_boundary = parsed.chat.entries.iter().filter(|entry| entry.kind != BundleEntryKind::CompactionBoundary).count();
        proptest::prop_assert_eq!(non_boundary, parsed.source_items as usize);
        let _ = peek_session(&path);
        let mut bundle = TransferBundle::new(BundleSource { tool: "claude_code".into(), tool_version: None, schema_version: None, adapter_version: "2".into(), roots: vec![], detected_at: "now".into() });
        bundle.chats.push(parsed.chat);
        let _ = import_bundle::lower(&import_bundle::resolve(vec![bundle]).unwrap());
    }
}

/// Reads every transcript on this machine and prints aggregate numbers only.
/// `BRIDGE_IMPORT_SWEEP=~/.claude/projects cargo test -p bridge-core --lib real_store_sweep -- --ignored --nocapture`
#[test]
#[ignore]
fn real_store_sweep() {
    let root = std::env::var("BRIDGE_IMPORT_SWEEP").expect("set BRIDGE_IMPORT_SWEEP to a projects dir");
    let started = std::time::Instant::now();
    let mut sessions = Vec::new();
    for project in std::fs::read_dir(&root).unwrap().flatten() {
        let Ok(files) = std::fs::read_dir(project.path()) else { continue };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                sessions.push(path);
            }
        }
    }
    sessions.sort();
    let mut kinds: BTreeMap<String, u64> = BTreeMap::new();
    let mut skipped: BTreeMap<String, u64> = BTreeMap::new();
    let (mut chats, mut children, mut errors, mut parity_breaks, mut entries_total) = (0u64, 0u64, 0u64, 0u64, 0u64);
    let (mut slowest, mut slowest_bytes) = (std::time::Duration::ZERO, 0u64);
    let (mut lowered_candidates, mut excluded_total) = (0usize, 0u64);
    let (mut committed, mut commit_errors) = (0u32, 0u32);
    let db = crate::store::open(Path::new(":memory:")).unwrap();
    for (index, path) in sessions.iter().enumerate() {
        let bytes = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
        let clock = std::time::Instant::now();
        let roots = [Path::new(&root).canonicalize().unwrap()];
        let parsed = match read_session(path, &format!("session-{index}"), &roots) {
            Ok(parsed) => parsed.chats,
            Err(_) => { errors += 1; continue; }
        };
        let took = clock.elapsed();
        if took > slowest { slowest = took; slowest_bytes = bytes; }
        let mut bundle = TransferBundle::new(BundleSource { tool: "claude_code".into(), tool_version: None, schema_version: Some("claude-jsonl-v2".into()), adapter_version: "2".into(), roots: vec![], detected_at: "2026-11-17T00:00:00Z".into() });
        for (position, item) in parsed.into_iter().enumerate() {
            if position == 0 { chats += 1 } else { children += 1 }
            let non_boundary = item.chat.entries.iter().filter(|entry| entry.kind != BundleEntryKind::CompactionBoundary).count();
            if non_boundary != item.source_items as usize { parity_breaks += 1 }
            for entry in &item.chat.entries {
                *kinds.entry(format!("{:?}", entry.kind)).or_insert(0) += 1;
                entries_total += 1;
            }
            for (reason, count) in &item.skipped.0 {
                *skipped.entry(reason.clone()).or_insert(0) += u64::from(*count);
            }
            bundle.chats.push(item.chat);
        }
        let lowered = import_bundle::lower(&import_bundle::resolve(vec![bundle]).unwrap());
        lowered_candidates += lowered.candidates.len();
        excluded_total += lowered.excluded.iter().map(|item| u64::from(item.count)).sum::<u64>();
        // Write every 40th session for real, to prove the forest accepts real payloads.
        if index % 40 == 0 && !lowered.candidates.is_empty() {
            let normalized: Vec<_> = lowered.candidates.iter().map(|candidate| crate::external_import::NormalizedImportCandidate { candidate: candidate.clone(), deterministic_bridge_id: candidate.candidate_id.clone() }).collect();
            let plan = crate::external_import::ImportPlan { selected_candidate_ids: lowered.candidates.iter().map(|c| c.candidate_id.clone()).collect(), conflict_policy: crate::external_import::ConflictPolicy::ImportAlongside, setup_activation_policy: crate::external_import::SetupActivationPolicy::Disabled, memory_scope: None, dry_run: false, created_at: "2026-11-17T00:00:00Z".into() };
            match crate::external_import::commit_import(&db, &normalized, &plan) {
                Ok(commit) => committed += commit.imported,
                Err(error) => { commit_errors += 1; eprintln!("commit error: {}", error.to_string().chars().take(160).collect::<String>()); }
            }
        }
    }
    let mut top: Vec<_> = skipped.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1));
    println!("SWEEP sessions={} chats={chats} subagent_chats={children} read_errors={errors} parity_breaks={parity_breaks}", sessions.len());
    println!("SWEEP entries={entries_total} by_kind={kinds:?}");
    println!("SWEEP candidates={lowered_candidates} excluded_items={excluded_total} committed_sample_chats={committed} commit_errors={commit_errors}");
    println!("SWEEP slowest_file_ms={} slowest_file_mb={:.1} total_s={:.1}", slowest.as_millis(), slowest_bytes as f64 / 1e6, started.elapsed().as_secs_f64());
    println!("SWEEP top_skipped={:?}", top.into_iter().take(15).collect::<Vec<_>>());
    assert_eq!(errors, 0);
    assert_eq!(parity_breaks, 0);
    assert_eq!(commit_errors, 0);
}

/// Peak resident memory of this process, in bytes.
fn peak_rss_bytes() -> u64 {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    let raw = usage.ru_maxrss as u64;
    if cfg!(target_os = "macos") { raw } else { raw * 1024 }
}

/// Run in a child process by `discarding_a_line_never_holds_more_than_one_buffer`,
/// so no other test's allocations share the measurement.
#[test]
#[ignore]
fn oversized_line_child() {
    let Ok(path) = std::env::var("BRIDGE_OVERSIZED_LINE_FILE") else { return };
    let before = peak_rss_bytes();
    let parsed = read_transcript(Path::new(&path), "k", "l").unwrap();
    assert_eq!(parsed.skipped.0.get("oversized_line"), Some(&1));
    assert_eq!(parsed.chat.entries.len(), 1, "the line after the oversized one is still read");
    println!("PEAK_GROWTH={}", peak_rss_bytes().saturating_sub(before));
}

#[test]
fn discarding_a_line_never_holds_more_than_one_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("projects/p/huge-line.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    {
        // One 256 MiB line with no newline in it, then one ordinary record.
        let mut file = File::create(&path).unwrap();
        let chunk = vec![b'a'; 4 * 1024 * 1024];
        for _ in 0..64 {
            file.write_all(&chunk).unwrap();
        }
        file.write_all(b"\n").unwrap();
        writeln!(file, "{}", user("u1", "2026-11-01T10:00:00Z", json!("after"))).unwrap();
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "claude_import::transcript::tests::oversized_line_child", "--ignored", "--nocapture", "--test-threads=1"])
        .env("BRIDGE_OVERSIZED_LINE_FILE", &path)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}\n{}", String::from_utf8_lossy(&output.stderr));
    // The harness prints "test … " before the test runs, so the figure can
    // share that line.
    let growth: u64 = stdout
        .split("PEAK_GROWTH=")
        .nth(1)
        .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|digits| digits.parse().ok())
        .expect("the child reports its peak");
    // The 64 MiB line buffer is the only allowance; keeping the rest of the
    // line would add another 192 MiB.
    assert!(growth < 128 * 1024 * 1024, "peak grew by {} MiB", growth / 1024 / 1024);
}

fn sidechain(uuid: &str, at: &str, kind: &str, content: Value) -> Value {
    json!({"type":kind,"uuid":uuid,"sessionId":"sess-1","isSidechain":true,"agentId":"abc","timestamp":at,"message":{"role":kind,"content":content}})
}

fn main_with_inline_sidechain(dir: &Path) -> PathBuf {
    let main = dir.join("projects/p/sess-1.jsonl");
    write_lines(&main, &[
        user("u1", "2026-11-01T10:00:00Z", json!("map the repo please")),
        sidechain("s1", "2026-11-01T10:00:01Z", "user", json!("Map the repo and report key files")),
        sidechain("s2", "2026-11-01T10:00:02Z", "assistant", json!([{"type":"text","text":"src/ and test/ matter."}])),
        assistant("a1", "2026-11-01T10:00:03Z", json!([{"type":"text","text":"Here is the map."}])),
    ]);
    main
}

#[test]
fn inline_sidechains_become_child_chats() {
    let dir = tempfile::tempdir().unwrap();
    let main = main_with_inline_sidechain(dir.path());
    let read = read_session(&main, "projects/p/sess-1.jsonl", &[dir.path().canonicalize().unwrap()]).unwrap();
    assert_eq!(read.chats.len(), 2, "the parent and one inline child");
    let parent = &read.chats[0].chat;
    let texts: Vec<_> = parent.entries.iter().filter_map(|e| e.text.clone()).collect();
    assert_eq!(texts, vec!["map the repo please", "Here is the map."], "the subagent's turns are not the person's");
    let child = &read.chats[1].chat;
    assert_eq!(child.entries.len(), 2);
    assert_eq!(child.parent_chat_key.as_deref(), Some(parent.key.as_str()));
    assert_eq!(child.agent_role, Some(AgentRole::Subagent));
    assert_eq!(child.native_ref.as_ref().unwrap().session_id, "sess-1/agent-abc");
    assert_eq!(child.title.as_deref(), Some("Map the repo and report key files"));
}

#[test]
fn an_inline_sidechain_and_its_file_are_one_child() {
    let dir = tempfile::tempdir().unwrap();
    let main = main_with_inline_sidechain(dir.path());
    write_lines(&dir.path().join("projects/p/sess-1/subagents/agent-abc.jsonl"), &[
        sidechain("s1", "2026-11-01T10:00:01Z", "user", json!("Map the repo and report key files")),
        sidechain("s2", "2026-11-01T10:00:02Z", "assistant", json!([{"type":"text","text":"src/ and test/ matter."}])),
    ]);
    let read = read_session(&main, "projects/p/sess-1.jsonl", &[dir.path().canonicalize().unwrap()]).unwrap();
    assert_eq!(read.chats.len(), 2, "the file and the inline copy are the same child");
    assert!(read.chats[1].chat.source_ref.ends_with("agent-abc.jsonl"), "the separate file is preferred");
}
