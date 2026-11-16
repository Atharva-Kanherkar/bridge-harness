//! Extra-account sign-in and limits end to end, through the daemon.
//!
//! Only the vendors are fake: `testing/fixtures/account-login-e2e/bin` holds
//! stand-ins for `claude`, `codex` and `opencode` whose sign-in records a login
//! in whatever folder Bridge pointed them at, and whose usage probes answer
//! from that folder. The daemon does everything else for real: it resolves the
//! account from Settings, creates its folder, launches the CLI in a PTY with
//! the account's environment, and reads each account's limits separately.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bridged::{Daemon, DaemonConfig};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testing/fixtures/account-login-e2e")
}

/// Point every vendor lookup at the fakes and every inherited home at the
/// test's own folder, so nothing touches the real `~/.codex` or data home.
fn fake_vendors(root: &Path) {
    let mut paths = vec![fixtures().join("bin")];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
    std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
    std::env::set_var("BRIDGE_CLAUDE_SIDECAR", fixtures().join("sidecar.mjs"));
    std::env::set_var("CODEX_HOME", root.join("codex-main"));
    std::env::set_var("XDG_DATA_HOME", root.join("xdg"));
    std::env::remove_var("CLAUDE_CONFIG_DIR");
    // An inherited key would outrank an account's own login; the sign-in
    // must not see it.
    std::env::set_var("ANTHROPIC_API_KEY", "inherited-e2e-key");
    std::fs::create_dir_all(root.join("codex-main")).unwrap();
    std::fs::create_dir_all(root.join("xdg")).unwrap();
}

fn set_accounts(data_dir: &Path, harness: &str, accounts: Value) {
    let db = bridge_core::store::open(&data_dir.join("bridge.db")).unwrap();
    let mut config = bridge_core::agent_config::state(&db).unwrap().harnesses.into_iter().find(|item| item.id == harness).unwrap();
    config.enabled = true;
    config.advanced = json!({ "accounts": accounts });
    bridge_core::agent_config::save_harness(&db, config).unwrap();
}

fn wait_for(path: &Path) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Ok(value) = serde_json::from_str(&text) {
                return value;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("{} never appeared", path.display());
}

#[test]
fn every_cli_account_signs_in_to_its_own_folder_and_reports_its_own_limits() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    fake_vendors(&root);
    let data_dir = root.join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let folder = |name: &str| root.join(name).to_string_lossy().into_owned();
    {
        let db = bridge_core::store::open(&data_dir.join("bridge.db")).unwrap();
        let mut menu = bridge_core::menu_bar::load(&db).unwrap();
        menu.codex_enabled = true;
        menu.claude_enabled = true;
        menu.cursor_enabled = false;
        menu.opencode_enabled = false;
        bridge_core::menu_bar::save(&db, &menu).unwrap();
    }
    set_accounts(&data_dir, "claude", json!([
        { "id": "work", "label": "Work", "homePath": folder("claude_work") },
        { "id": "personal", "label": "Personal", "homePath": folder("claude_personal") },
    ]));
    set_accounts(&data_dir, "codex", json!([
        { "id": "personal", "label": "Personal", "shadowHomePath": folder("codex_personal") },
        { "id": "solo", "label": "Solo", "homePath": folder("codex_solo") },
    ]));
    set_accounts(&data_dir, "opencode", json!([{ "id": "go", "label": "Go", "homePath": folder("opencode_go") }]));
    set_accounts(&data_dir, "cursor", json!([{ "id": "team", "label": "Team" }]));

    let daemon = RunningDaemon::start(&data_dir);
    let mut rpc = daemon.rpc();

    // Sign every CLI account in. None of the folders exist beforehand.
    let logins = [
        ("claude", "work", root.join("claude_work/.e2e-login.json")),
        ("claude", "personal", root.join("claude_personal/.e2e-login.json")),
        ("codex", "personal", root.join("codex_personal/auth.json")),
        ("codex", "solo", root.join("codex_solo/auth.json")),
        ("opencode", "go", root.join("opencode_go/opencode/auth.json")),
    ];
    for (provider, account, _) in &logins {
        assert!(!root.join(format!("{provider}_{account}")).exists());
        let started = rpc.call("auth/start_provider_login", json!({ "provider": provider, "account": account }));
        assert_eq!(started["terminalId"], format!("{provider}:account:{account}"));
    }
    for (provider, account, marker) in &logins {
        let login = wait_for(marker);
        if *provider == "claude" {
            assert_eq!(login["email"], format!("claude_{account}@claude.e2e"));
            assert_eq!(login["inheritedKey"], false, "{account} saw the inherited ANTHROPIC_API_KEY");
        }
    }
    // The shared Codex home never received a login of its own.
    assert!(!root.join("codex-main/auth.json").exists());

    // An API-key account has no CLI sign-in; an unknown one is refused.
    let refused = rpc.request("auth/start_provider_login", json!({ "provider": "cursor", "account": "team" })).unwrap_err();
    assert!(refused.to_string().contains("API key"), "{refused}");
    let missing = rpc.request("auth/start_provider_login", json!({ "provider": "claude", "account": "ghost" })).unwrap_err();
    assert!(missing.to_string().contains("no longer in Settings"), "{missing}");
    for hostile in ["../etc", "work/../../x", "", "claude"] {
        assert!(rpc.request("auth/start_provider_login", json!({ "provider": "claude", "account": hostile })).is_err(), "{hostile:?} was accepted");
    }

    // Limits: each account row reports the login in its own folder.
    let overview = rpc.call("usage/refresh_provider_usage_overviews_interactive", Value::Null);
    let rows = overview["providers"].as_array().unwrap();
    let row = |provider: &str, account: Option<&str>| rows.iter()
        .find(|row| row["provider"] == provider && row["accountId"].as_str() == account)
        .unwrap_or_else(|| panic!("no {provider}/{account:?} row in {overview:#}"))
        .clone();
    for (provider, account, email) in [
        ("claude", "work", "claude_work@claude.e2e"),
        ("claude", "personal", "claude_personal@claude.e2e"),
        ("codex", "personal", "codex_personal@codex.e2e"),
        ("codex", "solo", "codex_solo@codex.e2e"),
    ] {
        let snapshot = row(provider, Some(account));
        assert_eq!(snapshot["account"], email, "{provider}/{account}: {snapshot:#}");
        assert!(snapshot["error"].is_null(), "{provider}/{account}: {snapshot:#}");
        assert!(!snapshot["windows"].as_array().unwrap().is_empty(), "{provider}/{account} has no windows");
        assert!(snapshot.get("resetCredits").is_none());
    }
    // The user's own Codex login was never signed in, and stays that way.
    assert!(row("codex", None)["account"].is_null());
    // Rows follow their provider: default first, then its accounts.
    let order: Vec<_> = rows.iter().map(|row| format!("{}/{}", row["provider"].as_str().unwrap(), row["accountId"].as_str().unwrap_or("-"))).collect();
    assert_eq!(&order[..3], ["codex/-", "codex/personal", "codex/solo"]);
    assert_eq!(&order[3..6], ["claude/-", "claude/work", "claude/personal"]);
    // Cursor's menu toggle is off, but its account is configured: it lists,
    // honestly unread.
    assert!(row("cursor", Some("team"))["error"].as_str().is_some());

    // Removing an account removes its limits.
    set_accounts(&data_dir, "claude", json!([{ "id": "work", "label": "Work", "homePath": folder("claude_work") }]));
    let overview = rpc.call("usage/refresh_provider_usage_overviews_interactive", Value::Null);
    let ids: Vec<_> = overview["providers"].as_array().unwrap().iter()
        .filter(|row| row["provider"] == "claude")
        .map(|row| row["accountId"].as_str().unwrap_or("-").to_owned())
        .collect();
    assert_eq!(ids, ["-", "work"]);

    drop(rpc);
    daemon.stop();
}

struct RunningDaemon {
    daemon: Arc<Daemon>,
    accept_loop: Option<std::thread::JoinHandle<()>>,
}

impl RunningDaemon {
    fn start(data_dir: &Path) -> RunningDaemon {
        let (daemon, listener) = Daemon::start(DaemonConfig {
            data_dir: data_dir.to_path_buf(),
            socket_path: None,
            health_addr: None,
            browser_extension_path: data_dir.join("no-extension"),
            handshake_timeout: Duration::from_millis(400),
        })
        .expect("daemon starts");
        let daemon = Arc::new(daemon);
        let serve_daemon = daemon.clone();
        let accept_loop = std::thread::spawn(move || {
            bridged::serve(&serve_daemon, listener).expect("accept loop serves");
        });
        RunningDaemon { daemon, accept_loop: Some(accept_loop) }
    }

    fn rpc(&self) -> Rpc {
        Rpc::connect(&self.daemon.socket_path, &self.daemon.state.auth_token)
    }

    fn stop(mut self) {
        self.daemon.state.shutting_down.store(true, Ordering::SeqCst);
        if let Some(handle) = self.accept_loop.take() {
            handle.join().unwrap();
        }
        self.daemon.shutdown(Duration::from_secs(5));
    }
}

struct Rpc {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next: i64,
}

impl Rpc {
    fn connect(socket_path: &Path, token: &str) -> Rpc {
        let stream = UnixStream::connect(socket_path).expect("client connects");
        stream.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
        let mut rpc = Rpc { reader: BufReader::new(stream.try_clone().unwrap()), writer: stream, next: 0 };
        let handshake = rpc.request("protocol/handshake", json!({
            "protocolVersion": { "major": bridge_protocol::PROTOCOL_VERSION.major, "minor": bridge_protocol::PROTOCOL_VERSION.minor },
            "client": { "name": "limit-recovery-e2e", "version": "0" },
            "authToken": token,
        }));
        assert!(handshake.is_ok(), "{handshake:?}");
        rpc
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        self.next += 1;
        let mut frame = json!({ "jsonrpc": "2.0", "id": self.next, "method": method });
        if !params.is_null() {
            frame["params"] = params;
        }
        let mut line = serde_json::to_vec(&frame).unwrap();
        line.push(b'\n');
        self.writer.write_all(&line).unwrap();
        loop {
            let mut text = String::new();
            self.reader.read_line(&mut text).expect("a frame arrives");
            assert!(!text.is_empty(), "the daemon closed the connection");
            let frame: Value = serde_json::from_str(&text).unwrap();
            if frame.get("id") != Some(&json!(self.next)) {
                continue;
            }
            return match frame.get("error") {
                Some(error) => Err(error.clone()),
                None => Ok(frame["result"].clone()),
            };
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        self.request(method, params).unwrap_or_else(|error| panic!("{method} failed: {error}"))
    }

}
