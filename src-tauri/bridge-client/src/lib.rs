//! Protocol client for the `bridged` daemon.
//!
//! [`DaemonClient`] speaks the newline-delimited JSON-RPC contract over the
//! daemon's Unix socket: it performs the token handshake, sends requests
//! sequentially (matching the daemon's per-connection model), and hands
//! interleaved notifications to the consumer over a channel.
//!
//! Recovery follows the event contract: the live channel is notify-only, so
//! after a disconnect or a `stream-lagged` marker the client replays durable
//! history from its last cursor via `sessions/replay_session_events` —
//! [`DaemonClient::replay_session_events`] and [`resilient` stream helpers
//! below wrap that. Every client of the daemon (the exec one-shot today, the
//! Tauri proxy and TUI next) shares this crate rather than restating the
//! reconnect rules.

use bridge_protocol::{
    ClientInfo, HandshakeRequest, HandshakeResponse, MethodName, Params, RequestId, RpcError,
    RpcNotification, RpcRequest, RpcResponse, HANDSHAKE_METHOD, PROTOCOL_VERSION,
};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const SOCKET_FILE_NAME: &str = "bridged.sock";
pub const TOKEN_FILE_NAME: &str = "daemon.token";

/// How long [`DaemonClient::call`] waits for its response before declaring
/// the connection dead. Generous: requests are handled sequentially behind
/// possibly-slow runtime work (Git scans, adapter teardown).
pub const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug)]
pub enum ClientError {
    /// The socket could not be reached (daemon not running, wrong path).
    Connect(std::io::Error),
    /// The token file was missing or unreadable.
    Token { path: PathBuf, error: std::io::Error },
    /// The server refused the handshake (wrong token, incompatible version).
    Handshake(RpcError),
    /// The connection died mid-conversation; reconnect and replay.
    Disconnected,
    /// The server answered a request with an error.
    Rpc(RpcError),
    /// The response arrived but did not match what was asked.
    Protocol(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Connect(error) => write!(formatter, "could not connect: {error}"),
            ClientError::Token { path, error } => write!(
                formatter,
                "could not read the daemon token at {}: {error}",
                path.display()
            ),
            ClientError::Handshake(error) => {
                write!(formatter, "handshake rejected ({}): {}", error.code, error.message)
            }
            ClientError::Disconnected => write!(formatter, "the daemon connection closed"),
            ClientError::Rpc(error) => write!(formatter, "{} ({})", error.message, error.code),
            ClientError::Protocol(message) => write!(formatter, "protocol violation: {message}"),
        }
    }
}

impl std::error::Error for ClientError {}

/// Where a daemon lives: its socket and the token that authenticates to it.
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub socket_path: PathBuf,
    pub auth_token: String,
}

impl Endpoint {
    /// The endpoint for a data directory, reading the daemon's token file.
    pub fn for_data_dir(data_dir: &Path) -> Result<Endpoint, ClientError> {
        let token_path = data_dir.join(TOKEN_FILE_NAME);
        let auth_token = std::fs::read_to_string(&token_path)
            .map_err(|error| ClientError::Token { path: token_path, error })?
            .trim()
            .to_owned();
        Ok(Endpoint { socket_path: data_dir.join(SOCKET_FILE_NAME), auth_token })
    }
}

/// A connected, handshaken client. Requests are sequential (the daemon's
/// model); notifications arrive on [`DaemonClient::notifications`] as the
/// reader thread receives them, including between a request and its response.
pub struct DaemonClient {
    writer: Mutex<UnixStream>,
    responses: Mutex<Receiver<RpcResponse>>,
    notifications: Receiver<RpcNotification>,
    next_id: AtomicI64,
    handshake: HandshakeResponse,
    call_timeout: Duration,
}

impl DaemonClient {
    pub fn connect(endpoint: &Endpoint) -> Result<DaemonClient, ClientError> {
        let stream = UnixStream::connect(&endpoint.socket_path).map_err(ClientError::Connect)?;
        let mut writer = stream.try_clone().map_err(ClientError::Connect)?;
        let mut reader = BufReader::new(stream);

        // Handshake first — the server answers nothing else.
        let handshake_params = serde_json::to_value(HandshakeRequest {
            protocol_version: PROTOCOL_VERSION,
            client: ClientInfo {
                name: "bridge-client".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
            auth_token: Some(endpoint.auth_token.clone()),
        })
        .expect("handshake serializes");
        write_frame(
            &mut writer,
            &RpcRequest::new(
                RequestId::Number(0),
                HANDSHAKE_METHOD,
                Some(Params::new(handshake_params).expect("handshake params are an object")),
            ),
        )
        .map_err(ClientError::Connect)?;
        let mut line = String::new();
        reader.read_line(&mut line).map_err(ClientError::Connect)?;
        let response: RpcResponse = serde_json::from_str(&line)
            .map_err(|error| ClientError::Protocol(format!("handshake reply: {error}")))?;
        let handshake: HandshakeResponse = match response {
            RpcResponse::Success(success) => serde_json::from_value(success.result)
                .map_err(|error| ClientError::Protocol(format!("handshake result: {error}")))?,
            RpcResponse::Failure(failure) => return Err(ClientError::Handshake(failure.error)),
        };

        // The reader thread splits the stream: responses (frames with an id)
        // answer `call`; notifications flow to the consumer. It exits when
        // the socket closes, closing both channels behind it.
        let (response_sender, responses) = std::sync::mpsc::channel::<RpcResponse>();
        let (notification_sender, notifications) = std::sync::mpsc::channel::<RpcNotification>();
        std::thread::Builder::new()
            .name("bridge-client-reader".into())
            .spawn(move || read_frames(reader, response_sender, notification_sender))
            .expect("client reader thread spawns");

        Ok(DaemonClient {
            writer: Mutex::new(writer),
            responses: Mutex::new(responses),
            notifications,
            next_id: AtomicI64::new(1),
            handshake,
            call_timeout: DEFAULT_CALL_TIMEOUT,
        })
    }

    pub fn handshake(&self) -> &HandshakeResponse {
        &self.handshake
    }

    pub fn set_call_timeout(&mut self, timeout: Duration) {
        self.call_timeout = timeout;
    }

    /// The live notification stream. Bounded only by process memory; consume
    /// it or drop the client.
    pub fn notifications(&self) -> &Receiver<RpcNotification> {
        &self.notifications
    }

    /// Call a registry method and wait for its result.
    pub fn call(&self, method: MethodName, params: Option<Value>) -> Result<Value, ClientError> {
        self.call_raw(method.as_str(), params)
    }

    /// Call by wire name — for reserved methods and forward-compat tooling.
    pub fn call_raw(&self, method: &str, params: Option<Value>) -> Result<Value, ClientError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let params = match params {
            Some(value) => Some(
                Params::new(value)
                    .map_err(|error| ClientError::Protocol(error.to_string()))?,
            ),
            None => None,
        };
        let request = RpcRequest::new(RequestId::Number(id), method, params);
        write_frame(&mut *self.writer.lock().unwrap(), &request)
            .map_err(|_| ClientError::Disconnected)?;
        let responses = self.responses.lock().unwrap();
        let response = responses
            .recv_timeout(self.call_timeout)
            .map_err(|_| ClientError::Disconnected)?;
        let (response_id, outcome) = match response {
            RpcResponse::Success(success) => (success.id, Ok(success.result)),
            RpcResponse::Failure(failure) => (failure.id, Err(ClientError::Rpc(failure.error))),
        };
        let expected = serde_json::to_value(RequestId::Number(id)).unwrap();
        if serde_json::to_value(&response_id).unwrap() != expected {
            return Err(ClientError::Protocol(format!(
                "response for id {response_id:?} arrived while awaiting {id}"
            )));
        }
        outcome
    }

    /// Replay durable events strictly after `after_sequence`, page by page,
    /// until exhausted — the recovery half of the event contract.
    pub fn replay_session_events(
        &self,
        session_id: &str,
        after_sequence: i64,
    ) -> Result<Vec<Value>, ClientError> {
        let mut cursor = after_sequence;
        let mut events = Vec::new();
        loop {
            let page = self.call(
                MethodName::ReplaySessionEvents,
                Some(serde_json::json!({"sessionId": session_id, "afterSequence": cursor})),
            )?;
            let page = page
                .as_array()
                .cloned()
                .ok_or_else(|| ClientError::Protocol("replay result is not an array".into()))?;
            let Some(last) = page.last() else { break };
            cursor = last["sequence"]
                .as_i64()
                .ok_or_else(|| ClientError::Protocol("replayed event has no sequence".into()))?;
            let full_page = page.len() as u32
                >= bridge_protocol::messages::DEFAULT_REPLAY_EVENT_LIMIT;
            events.extend(page);
            if !full_page {
                break;
            }
        }
        Ok(events)
    }
}

fn write_frame<T: serde::Serialize>(writer: &mut impl Write, frame: &T) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(frame)?;
    line.push(b'\n');
    writer.write_all(&line)?;
    writer.flush()
}

fn read_frames(
    mut reader: BufReader<UnixStream>,
    responses: Sender<RpcResponse>,
    notifications: Sender<RpcNotification>,
) {
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(response) = serde_json::from_str::<RpcResponse>(&line) {
            if responses.send(response).is_err() {
                return;
            }
            continue;
        }
        if let Ok(notification) = serde_json::from_str::<RpcNotification>(&line) {
            if notifications.send(notification).is_err() {
                return;
            }
        }
        // Unknown frames are skipped: additive servers may send shapes a
        // minor-older client does not know.
    }
}

/// One durable agent event observed on a session, live or replayed.
#[derive(Debug, Clone)]
pub struct SessionEvent {
    pub sequence: i64,
    pub payload: Value,
}

/// A gap-free stream of one session's durable events across disconnects and
/// live-channel lag. It consumes the client's notification stream, tracks the
/// durable cursor, and replays whenever the live channel proves unreliable —
/// exactly the notify-then-replay contract. Transient frames (sequence zero)
/// pass through live and are never replayed.
pub struct SessionEventStream<'client> {
    client: &'client DaemonClient,
    session_id: String,
    cursor: i64,
    /// Replayed events not yet handed to the consumer.
    pending: std::collections::VecDeque<SessionEvent>,
}

impl<'client> SessionEventStream<'client> {
    /// Start a stream from a durable cursor (0 for the beginning). Replays
    /// immediately, closing the subscribe-before-replay race: live durable
    /// events at or below the replayed cursor are discarded as duplicates.
    pub fn new(
        client: &'client DaemonClient,
        session_id: impl Into<String>,
        after_sequence: i64,
    ) -> Result<SessionEventStream<'client>, ClientError> {
        let session_id = session_id.into();
        let mut stream = SessionEventStream {
            client,
            session_id,
            cursor: after_sequence,
            pending: Default::default(),
        };
        stream.replay()?;
        Ok(stream)
    }

    pub fn cursor(&self) -> i64 {
        self.cursor
    }

    fn replay(&mut self) -> Result<(), ClientError> {
        for event in self.client.replay_session_events(&self.session_id, self.cursor)? {
            let sequence = event["sequence"].as_i64().unwrap_or(0);
            if sequence > self.cursor {
                self.cursor = sequence;
                self.pending.push_back(SessionEvent { sequence, payload: event });
            }
        }
        Ok(())
    }

    /// The next event for this session: replayed backlog first, then live.
    /// `Ok(None)` when `deadline` passes with nothing relevant. A
    /// `stream-lagged` marker triggers a replay instead of surfacing.
    pub fn next(&mut self, deadline: Instant) -> Result<Option<SessionEvent>, ClientError> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Ok(Some(event));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            let notification = match self
                .client
                .notifications()
                .recv_timeout(remaining.min(Duration::from_millis(200)))
            {
                Ok(notification) => notification,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return Err(ClientError::Disconnected),
            };
            match notification.method.as_str() {
                "agent-event" => {
                    let payload = notification
                        .params
                        .map(Params::into_value)
                        .unwrap_or(Value::Null);
                    if payload["sessionId"].as_str() != Some(self.session_id.as_str()) {
                        continue;
                    }
                    let sequence = payload["sequence"].as_i64().unwrap_or(0);
                    if sequence == 0 {
                        // Transient streaming frame: deliver, never track.
                        return Ok(Some(SessionEvent { sequence: 0, payload }));
                    }
                    if sequence <= self.cursor {
                        continue; // Duplicate of a replayed event.
                    }
                    if sequence > self.cursor + 1 {
                        // A gap in durable sequences: the live channel missed
                        // events. Recover from the store, not the channel.
                        self.replay()?;
                        continue;
                    }
                    self.cursor = sequence;
                    return Ok(Some(SessionEvent { sequence, payload }));
                }
                // The host says this connection lagged: durable history is
                // intact in the store — replay from the cursor.
                "stream-lagged" => self.replay()?,
                _ => continue,
            }
        }
    }
}
