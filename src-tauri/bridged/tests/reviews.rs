//! The reviews domain over the daemon socket (testing/feat-review-job-runner.md):
//! start is idempotent, list, get, cancel and retry answer in their documented
//! shapes, and `reviews/job_updated` reaches the connection as the daemon's own
//! runner drives a job.
//!
//! Fully offline. GitHub is a local bare repository holding `main` and
//! `refs/pull/103/head`, and the shared `fake-gh.sh` (first on `PATH`) answers
//! `gh`, resolving the repository through its default. The daemon's runner
//! prepares a real review checkout; the reviewer itself is not in this build,
//! so the job then fails with a plain reason, and its checkout is gone. A
//! drafted job (which the runner never claims) gives the idempotency checks a
//! job that holds still.

// A test harness: a failed unwrap or expect in a helper is the test failing.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use bridge_protocol::messages::{ReviewSeverity, ReviewSide};
use bridged::{Daemon, DaemonConfig};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn git(cwd: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git").args(args).current_dir(cwd).output().unwrap();
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// `gh` that answers from `testing/fixtures/github`, first on `PATH`.
fn fake_gh() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testing/fixtures/github").canonicalize().unwrap();
    std::fs::write(directory.path().join("fixture-root"), fixtures.to_str().unwrap()).unwrap();
    std::fs::write(directory.path().join("auth-exit"), "0").unwrap();
    std::fs::write(directory.path().join("default-repository"), "fixture/project").unwrap();
    std::os::unix::fs::symlink(fixtures.join("fake-gh.sh"), directory.path().join("gh")).unwrap();
    let mut paths = vec![directory.path().to_path_buf()];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
    std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
    directory
}

/// A project and workspace on a repository whose origin is the fixture repo,
/// and a drafted review of PR 103 at the fixture's head.
fn seed(data_dir: &Path, repo: &Path) -> String {
    let db = bridge_core::store::open(&data_dir.join("bridge.db")).unwrap();
    db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo',?1,'now')", [repo.to_string_lossy()])
        .unwrap();
    db.execute(
        "INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','c','t','main',?1,'ready','now')",
        [repo.to_string_lossy()],
    )
    .unwrap();
    use bridge_core::reviews::{steps::DraftFinding, store};
    let (job, _) = store::insert(
        &db,
        &store::NewJob {
            workspace_id: "w".into(),
            repo_owner: "fixture".into(),
            repo_name: "project".into(),
            number: 103,
            head_sha: "current-head-sha".into(),
            base_sha: None,
            incremental_from_sha: None,
            trigger: Default::default(),
            post_mode: Default::default(),
            harnesses: vec!["claude".into()],
            retry_of: None,
            publication_key: None,
        },
        chrono::Utc::now(),
    )
    .unwrap();
    let pass = store::insert_pass(&db, &job.id, 0, 1, "claude", chrono::Utc::now()).unwrap();
    store::insert_findings(
        &db,
        &job.id,
        &pass,
        &[DraftFinding {
            path: "src/lib.rs".into(),
            line: 7,
            start_line: None,
            side: ReviewSide::Right,
            severity: ReviewSeverity::Critical,
            title: "Off by one".into(),
            body: "The loop skips the last element.".into(),
            confidence: 0.9,
            fingerprint: "fp".into(),
            outside_diff: false,
        }],
        chrono::Utc::now(),
    )
    .unwrap();
    db.execute("UPDATE review_jobs SET status='drafted' WHERE id=?1", [&job.id]).unwrap();
    job.id
}

struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next: i64,
    notifications: Vec<Value>,
}

impl Client {
    fn connect(socket_path: &Path, token: &str) -> Client {
        let stream = UnixStream::connect(socket_path).expect("client connects");
        stream.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        let mut client =
            Client { reader: BufReader::new(stream.try_clone().unwrap()), writer: stream, next: 0, notifications: Vec::new() };
        let handshake = client.call(
            "protocol/handshake",
            json!({
                "protocolVersion": {
                    "major": bridge_protocol::PROTOCOL_VERSION.major,
                    "minor": bridge_protocol::PROTOCOL_VERSION.minor,
                },
                "client": {"name": "reviews-test", "version": "0"},
                "authToken": token,
            }),
        );
        assert!(handshake.get("result").is_some(), "handshake failed: {handshake}");
        client
    }

    fn frame(&mut self) -> Value {
        let mut frame = String::new();
        self.reader.read_line(&mut frame).expect("a frame arrives");
        assert!(!frame.is_empty(), "the daemon closed the connection");
        serde_json::from_str(&frame).unwrap()
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        let mut line =
            serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).unwrap();
        line.push(b'\n');
        self.writer.write_all(&line).unwrap();
        self.writer.flush().unwrap();
        loop {
            let frame = self.frame();
            if frame.get("id") == Some(&json!(id)) {
                return frame;
            }
            if frame.get("method").is_some() {
                self.notifications.push(frame);
            }
        }
    }

    /// The first `reviews/job_updated` for `job_id` that `matches`, from what
    /// already arrived or what arrives next.
    fn job_update(&mut self, job_id: &str, matches: impl Fn(&Value) -> bool) -> Value {
        let wanted = |frame: &Value| {
            frame["method"] == json!("reviews/job_updated") && frame["params"]["job"]["id"] == json!(job_id) && matches(&frame["params"]["job"])
        };
        if let Some(found) = self.notifications.iter().find(|frame| wanted(frame)) {
            return found["params"]["job"].clone();
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            assert!(Instant::now() < deadline, "no matching reviews/job_updated for {job_id}");
            let frame = self.frame();
            let found = wanted(&frame).then(|| frame["params"]["job"].clone());
            self.notifications.push(frame);
            if let Some(job) = found {
                return job;
            }
        }
    }
}

/// Retry queues a linked job; the daemon's runner claims it, prepares a real
/// checkout of the PR head, and, with no reviewer in this build, fails it with
/// a plain reason, leaving no checkout behind.
fn retry_prepares_then_fails_cleanly(client: &mut Client, data_dir: &Path, repo: &Path, seeded: &str) -> String {
    let retried = client.call("reviews/retry_review", json!({"jobId": seeded}));
    let retry = &retried["result"]["job"];
    assert_eq!(retry["retryOf"], json!(seeded), "{retried}");
    let retry_id = retry["id"].as_str().unwrap().to_owned();
    assert_ne!(retry_id, seeded);
    let failed = client.job_update(&retry_id, |job| job["status"] == json!("failed"));
    assert_eq!(failed["error"], json!("This build cannot run PR reviews yet"));
    client.job_update(&retry_id, |job| job["status"] == json!("preparing"));
    let reviewing = client.job_update(&retry_id, |job| job["status"] == json!("reviewing"));
    assert_ne!(reviewing["headSha"], json!("current-head-sha"), "the job moved to the head the fetch found");
    let detail = client.call("reviews/get_review", json!({"jobId": retry_id}));
    assert_eq!(detail["result"]["passes"][0]["error"], json!("This build cannot run PR reviews yet"), "{detail}");
    let reviews = data_dir.join("worktrees").join("reviews");
    let left: Vec<_> = std::fs::read_dir(&reviews).map(|entries| entries.filter_map(Result::ok).map(|entry| entry.path()).collect()).unwrap_or_default();
    assert!(left.is_empty(), "review checkouts left: {left:?}");
    let listed_worktrees = git(repo, &["worktree", "list", "--porcelain"]);
    assert_eq!(listed_worktrees.matches("worktree ").count(), 1, "{listed_worktrees}");
    retry_id
}

/// A repository whose `origin` is a local bare repository standing in for
/// GitHub, with `main` and pull request #103.
fn repository_with_a_pull_request(repo: &Path, github: &Path) {
    git(repo, &["init", "-q", "-b", "main"]);
    git(repo, &["config", "user.email", "t@example.invalid"]);
    git(repo, &["config", "user.name", "T"]);
    git(repo, &["config", "commit.gpgsign", "false"]);
    std::fs::write(repo.join("README.md"), "base\n").unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-q", "-m", "base"]);
    git(github, &["init", "-q", "--bare"]);
    git(repo, &["remote", "add", "origin", github.to_str().unwrap()]);
    git(repo, &["push", "-q", "origin", "main"]);
    git(repo, &["checkout", "-q", "-b", "pr"]);
    std::fs::write(repo.join("README.md"), "base\nchanged by the PR\n").unwrap();
    git(repo, &["commit", "-q", "-am", "the PR"]);
    git(repo, &["push", "-q", "origin", "HEAD:refs/pull/103/head"]);
    git(repo, &["checkout", "-q", "main"]);
}

#[test]
fn reviews_methods_round_trip_through_the_daemon() {
    let _gh = fake_gh();
    let fixture = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let github = tempfile::tempdir().unwrap();
    repository_with_a_pull_request(repo.path(), github.path());
    let seeded = seed(fixture.path(), repo.path());
    let (daemon, listener) = Daemon::start(DaemonConfig {
        data_dir: fixture.path().to_path_buf(),
        socket_path: None,
        health_addr: None,
        browser_extension_path: fixture.path().join("no-extension"),
        handshake_timeout: Duration::from_millis(400),
    })
    .expect("daemon starts");
    let daemon = std::sync::Arc::new(daemon);
    let serve_daemon = daemon.clone();
    std::thread::spawn(move || {
        bridged::serve(&serve_daemon, listener).expect("accept loop serves");
    });
    let mut client = Client::connect(&daemon.socket_path, &daemon.state.auth_token);
    let sessions_before: i64 =
        daemon.core.db.lock().unwrap().query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0)).unwrap();

    // Start is idempotent: the PR head (read from gh) and harness set match the
    // drafted job, so it comes back instead of a new one.
    let started = client.call("reviews/start_review", json!({"workspaceId": "w", "number": 103, "harnesses": ["claude-code"]}));
    let job = &started["result"]["job"];
    assert_eq!(job["id"], json!(seeded), "{started}");
    assert_eq!(job["status"], json!("drafted"));
    assert_eq!(job["headSha"], json!("current-head-sha"));
    assert_eq!(job["harnesses"], json!(["claude"]));
    assert_eq!(job["findingCounts"], json!({"total": 1, "critical": 1, "major": 0, "minor": 0, "nit": 0}));
    assert_eq!(job["postMode"], json!("draft"));
    assert_eq!(job["trigger"], json!("manual"));

    let listed = client.call("reviews/list_reviews", json!({"workspaceId": "w", "number": 103}));
    assert_eq!(listed["result"]["jobs"].as_array().unwrap().len(), 1, "{listed}");
    assert_eq!(listed["result"]["jobs"][0]["id"], json!(seeded));

    let detail = client.call("reviews/get_review", json!({"jobId": seeded}));
    assert_eq!(detail["result"]["passes"][0]["harness"], json!("claude"), "{detail}");
    let finding = &detail["result"]["findings"][0];
    assert_eq!((finding["path"].clone(), finding["side"].clone()), (json!("src/lib.rs"), json!("RIGHT")));
    assert_eq!(finding["decision"], json!("pending"));

    // Cancel settles the draft now and tells the connection.
    let cancelled = client.call("reviews/cancel_review", json!({"jobId": seeded}));
    assert_eq!(cancelled["result"]["job"]["status"], json!("cancelled"), "{cancelled}");
    client.job_update(&seeded, |job| job["status"] == json!("cancelled"));
    let again = client.call("reviews/cancel_review", json!({"jobId": seeded}));
    assert!(again["error"]["message"].as_str().unwrap().contains("already cancelled"), "{again}");

    let retry_id = retry_prepares_then_fails_cleanly(&mut client, fixture.path(), repo.path(), &seeded);
    let listed = client.call("reviews/list_reviews", json!({"workspaceId": "w"}));
    let ids: Vec<&str> = listed["result"]["jobs"].as_array().unwrap().iter().map(|job| job["id"].as_str().unwrap()).collect();
    assert_eq!(ids, [retry_id.as_str(), seeded.as_str()], "newest first");

    // Bad requests are refused without a job.
    let unknown = client.call("reviews/start_review", json!({"workspaceId": "w", "number": 103, "sessionId": "s"}));
    assert_eq!(unknown["error"]["code"], json!(-32602), "{unknown}");
    let bugbot = client.call("reviews/start_review", json!({"workspaceId": "w", "number": 103, "harnesses": ["bugbot"]}));
    assert!(bugbot["error"]["message"].as_str().unwrap().contains("Cursor Bugbot"), "{bugbot}");
    let missing = client.call("reviews/get_review", json!({"jobId": "missing"}));
    assert!(missing["error"].is_object(), "{missing}");

    let sessions_after: i64 =
        daemon.core.db.lock().unwrap().query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0)).unwrap();
    assert_eq!(sessions_after, sessions_before, "no review touched a chat or created a session");
}
