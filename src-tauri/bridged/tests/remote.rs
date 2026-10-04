//! The remote WebSocket transport driven by a real WebSocket client against a
//! real daemon: the same contract as the Unix socket, plus the origin rules.

use bridged::{Daemon, DaemonConfig, RemoteConfig};
use serde_json::{json, Value};
use std::net::TcpStream;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tungstenite::client::IntoClientRequest;
use tungstenite::http::HeaderValue;
use tungstenite::{Message, WebSocket};

const ALLOWED_ORIGIN: &str = "https://app.example.com";

fn seed_data_dir(data_dir: &Path) {
    let db = bridge_core::store::open(&data_dir.join("bridge.db")).unwrap();
    let mut opencode = bridge_core::agent_config::state(&db)
        .unwrap()
        .harnesses
        .into_iter()
        .find(|harness| harness.id == "opencode")
        .unwrap();
    opencode.advanced = json!({"executablePath": data_dir.join("missing-opencode").to_string_lossy()});
    bridge_core::agent_config::save_harness(&db, opencode).unwrap();
}

struct Running {
    daemon: Arc<Daemon>,
    serving: Option<std::thread::JoinHandle<()>>,
    addr: std::net::SocketAddr,
    token: String,
    socket_path: std::path::PathBuf,
}

impl Running {
    fn start(data_dir: &Path) -> Running {
        seed_data_dir(data_dir);
        let (daemon, listener) = Daemon::start(DaemonConfig {
            data_dir: data_dir.to_path_buf(),
            socket_path: None,
            health_addr: None,
            browser_extension_path: data_dir.join("no-extension"),
            handshake_timeout: Duration::from_millis(600),
        })
        .expect("daemon starts");
        let config = RemoteConfig::new(
            "127.0.0.1:0".parse().unwrap(),
            vec![ALLOWED_ORIGIN.into()],
            false,
        )
        .unwrap();
        let remote = daemon.bind_remote(&config).expect("remote listener binds");
        let addr = remote.local_addr().unwrap();
        let daemon = Arc::new(daemon);
        let token = daemon.state.auth_token.clone();
        let socket_path = daemon.socket_path.clone();
        let serve_daemon = daemon.clone();
        let serving = std::thread::spawn(move || {
            bridged::serve_with_remote(&serve_daemon, listener, Some(remote)).expect("serves");
        });
        Running { daemon, serving: Some(serving), addr, token, socket_path }
    }

    fn stop(mut self) {
        self.daemon.state.shutting_down.store(true, Ordering::SeqCst);
        if let Some(handle) = self.serving.take() {
            handle.join().unwrap();
        }
        self.daemon.shutdown(Duration::from_secs(5));
    }
}

/// Upgrade result: either a socket, or the HTTP status the upgrade was refused
/// with.
fn connect(addr: std::net::SocketAddr, origin: Option<&str>) -> Result<WebSocket<TcpStream>, u16> {
    let stream = TcpStream::connect(addr).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut request = format!("ws://{addr}").into_client_request().unwrap();
    if let Some(origin) = origin {
        request.headers_mut().insert("origin", HeaderValue::from_str(origin).unwrap());
    }
    match tungstenite::client(request, stream) {
        Ok((socket, _)) => Ok(socket),
        Err(tungstenite::HandshakeError::Failure(tungstenite::Error::Http(response))) => {
            Err(response.status().as_u16())
        }
        Err(other) => panic!("unexpected handshake failure: {other}"),
    }
}

fn send(socket: &mut WebSocket<TcpStream>, frame: Value) {
    socket.send(Message::text(frame.to_string())).unwrap();
}

fn recv(socket: &mut WebSocket<TcpStream>) -> Value {
    loop {
        match socket.read().expect("a frame arrives") {
            Message::Text(text) => return serde_json::from_str(text.as_str()).unwrap(),
            Message::Ping(_) | Message::Pong(_) => continue,
            other => panic!("expected a text frame, got {other:?}"),
        }
    }
}

/// The frame that answers request `id`, skipping interleaved notifications
/// (a mutation publishes its own `state-changed`).
fn recv_response(socket: &mut WebSocket<TcpStream>, id: i64) -> Value {
    loop {
        let frame = recv(socket);
        if frame.get("id") == Some(&json!(id)) {
            return frame;
        }
    }
}

fn handshake_frame(token: &str) -> Value {
    json!({
        "jsonrpc": "2.0", "id": 0, "method": "protocol/handshake",
        "params": {
            "protocolVersion": {
                "major": bridge_protocol::PROTOCOL_VERSION.major,
                "minor": bridge_protocol::PROTOCOL_VERSION.minor,
            },
            "client": {"name": "remote-test", "version": "0"},
            "authToken": token,
        },
    })
}

#[test]
fn a_websocket_client_with_the_token_and_an_allowed_origin_drives_the_daemon() {
    let fixture = tempfile::tempdir().unwrap();
    let running = Running::start(fixture.path());

    let mut socket = connect(running.addr, Some(ALLOWED_ORIGIN)).expect("upgrade succeeds");
    send(&mut socket, handshake_frame(&running.token));
    let accepted = recv(&mut socket);
    assert_eq!(accepted["result"]["server"]["name"], json!("bridge"));

    send(&mut socket, json!({"jsonrpc": "2.0", "id": 1, "method": "health/health"}));
    let health = recv_response(&mut socket, 1);
    assert_eq!(health["id"], json!(1));
    assert_eq!(health["result"]["ok"], json!(true));

    // A mutation then an event: notifications interleave on the same socket.
    send(&mut socket, json!({"jsonrpc": "2.0", "id": 2, "method": "sessions/create_chat", "params": {"harness": "shell"}}));
    let created = recv_response(&mut socket, 2);
    assert_eq!(created["result"]["sessions"].as_array().unwrap().len(), 1);
    // The event stream reaches this socket: a published hint arrives as a
    // notification (the mutation above may have published its own too).
    running.daemon.core.events.publish(bridge_core::events::CoreEvent::StateChanged);
    let hint = recv(&mut socket);
    assert_eq!(hint["method"], json!("state-changed"));

    // Params are validated against the same contract as the Unix socket.
    send(&mut socket, json!({"jsonrpc": "2.0", "id": 3, "method": "browser/browser_frame", "params": {"afterRevision": -1}}));
    assert_eq!(recv_response(&mut socket, 3)["error"]["code"], json!(-32602));

    drop(socket);
    running.stop();
}

#[test]
fn the_handshake_gate_holds_over_websocket() {
    let fixture = tempfile::tempdir().unwrap();
    let running = Running::start(fixture.path());

    let mut socket = connect(running.addr, Some(ALLOWED_ORIGIN)).unwrap();
    send(&mut socket, handshake_frame("not-the-token"));
    assert_eq!(recv(&mut socket)["error"]["code"], json!(2001));

    let mut socket = connect(running.addr, Some(ALLOWED_ORIGIN)).unwrap();
    send(&mut socket, json!({"jsonrpc": "2.0", "id": 9, "method": "health/health"}));
    assert_eq!(recv(&mut socket)["error"]["code"], json!(-32600));

    // A peer that upgrades and then says nothing is reclaimed at the deadline.
    let mut silent = connect(running.addr, Some(ALLOWED_ORIGIN)).unwrap();
    let rejected = recv(&mut silent);
    assert_eq!(rejected["error"]["code"], json!(-32600));

    running.stop();
}

#[test]
fn an_origin_off_the_allowlist_is_refused_at_the_upgrade() {
    let fixture = tempfile::tempdir().unwrap();
    let running = Running::start(fixture.path());

    for origin in ["https://evil.example", "http://app.example.com", "null"] {
        assert_eq!(connect(running.addr, Some(origin)).err(), Some(403), "{origin}");
    }
    // A non-browser client sends no Origin; the token still gates it.
    let mut socket = connect(running.addr, None).expect("no Origin is not a browser");
    send(&mut socket, handshake_frame("wrong"));
    assert_eq!(recv(&mut socket)["error"]["code"], json!(2001));

    running.stop();
}

#[test]
fn oversized_and_binary_messages_end_the_connection() {
    let fixture = tempfile::tempdir().unwrap();
    let running = Running::start(fixture.path());

    let mut socket = connect(running.addr, Some(ALLOWED_ORIGIN)).unwrap();
    send(&mut socket, handshake_frame(&running.token));
    recv(&mut socket);
    let huge = "x".repeat(bridged::MAX_FRAME_BYTES + 1024);
    let _ = socket.send(Message::text(huge));
    // Skip any event notification that raced ahead of the refusal.
    let refused = loop {
        let frame = recv(&mut socket);
        if frame.get("error").is_some() {
            break frame;
        }
    };
    assert_eq!(refused["error"]["code"], json!(-32600));

    let mut socket = connect(running.addr, Some(ALLOWED_ORIGIN)).unwrap();
    send(&mut socket, handshake_frame(&running.token));
    recv(&mut socket);
    socket.send(Message::binary(b"{}".to_vec())).unwrap();
    assert!(socket.read().is_err() || matches!(socket.read(), Ok(Message::Close(_)) | Err(_)));

    running.stop();
}

#[test]
fn both_transports_serve_the_same_daemon_and_share_the_connection_cap() {
    let fixture = tempfile::tempdir().unwrap();
    let running = Running::start(fixture.path());

    let mut socket = connect(running.addr, Some(ALLOWED_ORIGIN)).unwrap();
    send(&mut socket, handshake_frame(&running.token));
    recv(&mut socket);
    send(&mut socket, json!({"jsonrpc": "2.0", "id": 1, "method": "sessions/create_chat", "params": {"harness": "shell"}}));
    let created = recv_response(&mut socket, 1);
    let session_id = created["result"]["sessions"][0]["id"].as_str().unwrap().to_owned();

    // The Unix socket sees the chat the WebSocket client created.
    use std::io::{BufRead, BufReader, Write};
    let unix = std::os::unix::net::UnixStream::connect(&running.socket_path).unwrap();
    let mut reader = BufReader::new(unix.try_clone().unwrap());
    let mut writer = unix;
    writeln!(writer, "{}", handshake_frame(&running.token)).unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(serde_json::from_str::<Value>(&line).unwrap().get("result").is_some());
    writeln!(
        writer,
        "{}",
        json!({"jsonrpc": "2.0", "id": 2, "method": "sessions/replay_session_events", "params": {"sessionId": session_id, "afterSequence": 0}})
    )
    .unwrap();
    line.clear();
    reader.read_line(&mut line).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["result"], json!([]));

    // Slots are counted across transports and released on disconnect.
    assert_eq!(running.daemon.state.connections.load(Ordering::SeqCst), 2);
    drop(socket);
    drop(writer);
    drop(reader);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while running.daemon.state.connections.load(Ordering::SeqCst) > 0 {
        assert!(std::time::Instant::now() < deadline, "slots were not released");
        std::thread::sleep(Duration::from_millis(50));
    }

    running.stop();
}

fn wait_for_released_slots(running: &Running) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while running.daemon.state.connections.load(Ordering::SeqCst) > 0 {
        assert!(std::time::Instant::now() < deadline, "the connection slot was never reclaimed");
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn a_peer_dripping_http_header_bytes_cannot_outlive_the_handshake_deadline() {
    let fixture = tempfile::tempdir().unwrap();
    let running = Running::start(fixture.path()); // 600 ms handshake deadline
    let mut stream = TcpStream::connect(running.addr).unwrap();
    stream.set_nonblocking(true).unwrap();
    let started = std::time::Instant::now();
    let drip = b"GET / HTTP/1.1\r\nHost: x\r\nX-Pad: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let mut closed = false;
    for byte in drip {
        use std::io::{Read, Write};
        if stream.write_all(&[*byte]).is_err() {
            closed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
        let mut probe = [0u8; 64];
        match stream.read(&mut probe) {
            Ok(0) => {
                closed = true;
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            _ => {
                closed = true;
                break;
            }
        }
        if started.elapsed() > Duration::from_secs(3) {
            break;
        }
    }
    assert!(closed, "a slow-drip peer held its connection past the deadline");
    assert!(
        started.elapsed() < Duration::from_millis(1800),
        "reclaimed too late: {:?}",
        started.elapsed()
    );
    wait_for_released_slots(&running);
    running.stop();
}

#[test]
fn a_peer_flooding_pings_before_authenticating_is_reclaimed_at_the_deadline() {
    let fixture = tempfile::tempdir().unwrap();
    let running = Running::start(fixture.path());
    let mut socket = connect(running.addr, Some(ALLOWED_ORIGIN)).unwrap();
    socket.get_ref().set_read_timeout(Some(Duration::from_millis(20))).unwrap();
    let started = std::time::Instant::now();
    let mut closed = false;
    while started.elapsed() < Duration::from_secs(3) {
        if socket.send(Message::Ping(vec![1].into())).is_err() {
            closed = true;
            break;
        }
        match socket.read() {
            Ok(Message::Close(_)) => {
                closed = true;
                break;
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => {
                closed = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    assert!(closed, "a ping-flooding peer held its connection past the deadline");
    assert!(started.elapsed() < Duration::from_millis(1800), "{:?}", started.elapsed());
    wait_for_released_slots(&running);
    running.stop();
}
