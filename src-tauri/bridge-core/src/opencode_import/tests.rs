use super::*;
use crate::opencode_import::fixture::{replied, said, Fixture, CANARY};

/// A store with a chat and its subagent, an untitled chat in another
/// folder, a background run in the temp folder and a Bridge title run.
fn store(data_home: &Path) -> Fixture {
    let fixture = Fixture::new(data_home);
    fixture.session("ses_b_child", Some("ses_a_parent"), "/work/app", Some("Audit (@general subagent)"), 1_790_000_000_200);
    fixture.session("ses_a_parent", None, "/work/app", Some("Fix the login"), 1_790_000_000_100);
    fixture.session("ses_c", None, "/work/api", None, 1_790_000_000_300);
    fixture.session("ses_d_temp", None, "/private/var/folders/xy/T/probe", Some("probe"), 1_790_000_000_400);
    fixture.session("ses_e_title", None, "/Users/me/Library/Application Support/dev.bridge.deck/chats/title-runs", Some("titles"), 1_790_000_000_500);
    for (id, text) in [("ses_a_parent", "fix the login"), ("ses_b_child", "audit"), ("ses_c", "add a route"), ("ses_d_temp", "ping"), ("ses_e_title", "title these")] {
        fixture.row(id, 1, "user", &said(text));
        fixture.row(id, 2, "assistant", &replied("ok"));
    }
    fixture
}

#[test]
fn the_work_list_puts_parents_first_and_leaves_background_runs_out() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = store(dir.path());

    let work = job_work(&[fixture.home.clone()]).unwrap();

    let ids: Vec<String> = work
        .artifacts
        .iter()
        .map(|artifact| parse_source_ref(&artifact.canonical_source_ref).unwrap().1)
        .collect();
    assert_eq!(ids, vec!["ses_a_parent", "ses_b_child", "ses_c"], "oldest first, so a parent comes before its subagent");
    assert_eq!(work.left_out, 2, "the temp-folder run and the title run");
    let keys: Vec<String> = work.artifacts.iter().map(job_key).collect();
    assert!(keys.windows(2).all(|pair| pair[0] < pair[1]), "the cursor order is the work order: {keys:?}");
    assert_eq!(work.artifacts[0].project_path.as_deref(), Some("/work/app"));
    assert!(work.artifacts.iter().all(|artifact| artifact.estimated_bytes > 0));
}

#[test]
fn a_data_home_or_a_data_folder_both_find_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = store(dir.path());

    let from_folder = job_work(&[fixture.home.clone()]).unwrap().artifacts.len();
    let from_home = job_work(&[dir.path().canonicalize().unwrap()]).unwrap().artifacts.len();

    assert_eq!((from_folder, from_home), (3, 3));
}

#[test]
fn a_batch_previews_into_chats_with_native_keys() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = store(dir.path());
    let roots = vec![fixture.home.clone()];
    let work = job_work(&roots).unwrap();
    let selection = DiscoverySelection { artifact_ids: work.artifacts.iter().map(|artifact| artifact.artifact_id.clone()).collect() };

    let candidates = OpenCodeImporter.preview(&job_discovery(&roots, work.artifacts), &selection).unwrap();

    let conversations: Vec<&ImportCandidate> = candidates.iter().filter(|candidate| candidate.kind == CandidateKind::Conversation).collect();
    assert_eq!(conversations.len(), 3, "{candidates:?}");
    let keys: Vec<Option<&str>> = conversations.iter().map(|candidate| candidate.normalized_payload["nativeKey"].as_str()).collect();
    assert!(keys.contains(&Some("opencode:ses_a_parent")) && keys.contains(&Some("opencode:ses_b_child")), "{keys:?}");
    let child = conversations.iter().find(|candidate| candidate.normalized_payload["nativeKey"] == "opencode:ses_b_child").unwrap();
    assert_eq!(child.normalized_payload["parentNativeKey"].as_str(), Some("opencode:ses_a_parent"));
    assert!(!serde_json::to_string(&candidates).unwrap().contains(CANARY));
}

#[test]
fn a_session_gone_from_the_store_is_reported_not_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = store(dir.path());
    let roots = vec![fixture.home.clone()];
    let work = job_work(&roots).unwrap();
    let selection = DiscoverySelection { artifact_ids: work.artifacts.iter().map(|artifact| artifact.artifact_id.clone()).collect() };
    fixture.delete("ses_c");

    let candidates = OpenCodeImporter.preview(&job_discovery(&roots, work.artifacts), &selection).unwrap();

    assert_eq!(candidates.iter().filter(|candidate| candidate.kind == CandidateKind::Conversation).count(), 2);
    let gone = candidates.iter().find(|candidate| candidate.kind == CandidateKind::Unsupported).unwrap();
    assert!(gone.diagnostics[0].message.contains("no longer has this session"), "{:?}", gone.diagnostics);
}

#[test]
fn a_store_outside_the_approved_folders_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let fixture = store(dir.path());
    let work = job_work(&[fixture.home.clone()]).unwrap();
    let selection = DiscoverySelection { artifact_ids: work.artifacts.iter().map(|artifact| artifact.artifact_id.clone()).collect() };

    let candidates = OpenCodeImporter.preview(&job_discovery(&[other.path().canonicalize().unwrap()], work.artifacts), &selection).unwrap();

    assert!(candidates.iter().all(|candidate| candidate.kind == CandidateKind::Unsupported));
}

#[test]
fn counts_leave_background_runs_apart_and_subagents_out() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = store(dir.path());

    let counts = count_sessions(&fixture.store).unwrap();

    assert_eq!(counts, StoreCounts { chats: 2, projects: 2, background: 2 });
}

#[test]
fn a_store_still_has_a_session_until_opencode_deletes_it() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = store(dir.path());

    assert!(still_has(&fixture.home, "ses_a_parent"));
    assert!(!still_has(&fixture.home, "ses_missing"));
    fixture.delete("ses_a_parent");
    assert!(!still_has(&fixture.home, "ses_a_parent"));
    assert!(!still_has(&dir.path().join("nowhere"), "ses_c"));
}

#[test]
fn a_source_reference_is_the_store_and_the_session() {
    let reference = source_ref(Path::new("/data/opencode/opencode.db"), "ses_1");
    assert_eq!(reference, "/data/opencode/opencode.db#ses_1");
    assert_eq!(parse_source_ref(&reference), Some((PathBuf::from("/data/opencode/opencode.db"), "ses_1".into())));
    assert_eq!(parse_source_ref("/no/session"), None);
}

#[test]
fn background_runs_are_the_temp_folder_and_bridge_title_runs() {
    assert!(is_background("/private/var/folders/xy/T/probe"));
    assert!(is_background("/tmp/scratch"));
    assert!(is_background("/Users/me/Library/Application Support/dev.bridge.deck/chats/title-runs"));
    assert!(!is_background("/Users/me/work/app"));
    assert!(!is_background("/Users/me/Library/Application Support/dev.bridge.deck/chats/3f2a"));
}
