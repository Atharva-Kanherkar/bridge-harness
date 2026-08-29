//! Protocol client for the `bridged` daemon.
//!
//! [`DaemonClient`] speaks the newline-delimited JSON-RPC contract over the
//! daemon's Unix socket: it performs the token handshake, sends requests
//! sequentially (matching the daemon's per-connection model), and fans
//! interleaved notifications out to any number of subscribers over bounded
//! queues — a subscriber that falls behind is told with a locally
//! synthesized `stream-lagged` marker, exactly the signal the daemon itself
//! uses, so recovery is one code path.
//!
//! Recovery follows the event contract: the live channel is notify-only, so
//! after a disconnect or a lag marker the client replays durable history from
//! its last cursor via `sessions/replay_session_events`.
//! [`SessionEventStream`] encodes those rules once; its cursor reflects what
//! the consumer has actually been handed, never what is merely queued. Every
//! client of the daemon (the exec one-shot today, the Tauri proxy and TUI
//! next) shares this crate rather than restating the reconnect rules.

use bridge_protocol::{
    ClientInfo, HandshakeRequest, HandshakeResponse, MethodName, Params, RequestId, RpcError,
    RpcNotification, RpcRequest, RpcResponse, HANDSHAKE_METHOD, PROTOCOL_VERSION,
};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub const SOCKET_FILE_NAME: &str = "bridged.sock";
pub const TOKEN_FILE_NAME: &str = "daemon.token";

/// Default budget for [`DaemonClient::call`]. Generous: requests are handled
/// sequentially behind possibly-slow runtime work (Git scans, adapter
/// teardown). Callers with an end-to-end deadline use
/// [`DaemonClient::call_with_timeout`].
pub const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(300);

/// Default budget for [`DaemonClient::connect`] to finish the handshake — a
/// wedged daemon must not park the client forever.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Bounded per-subscriber queue, mirroring the daemon's own sink capacity. A
/// subscriber that stops draining loses live frames and gets a lag marker —
/// never unbounded client memory.
const SUBSCRIBER_CAPACITY: usize = 1024;

/// Server frames include state snapshots and replay pages, so they need a
/// separate ceiling from the daemon's 1 MiB request limit. This remains a
/// hard memory bound without rejecting normal large workspace responses.
const MAX_SERVER_FRAME_BYTES: usize = 64 * 1024 * 1024;

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
    /// The call's deadline passed. The connection stays usable — a late
    /// response is discarded by id when the next call runs.
    Timeout,
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
            ClientError::Timeout => write!(formatter, "the call timed out"),
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

/// One notification subscriber's queue, plus the lag debt owed to it.
struct Subscriber {
    sender: SyncSender<RpcNotification>,
    lagged: Arc<AtomicBool>,
}

/// A bounded subscription whose lag debt is independent of its full queue.
pub struct NotificationSubscription {
    receiver: Receiver<RpcNotification>,
    lagged: Arc<AtomicBool>,
}

impl NotificationSubscription {
    fn take_lag_marker(&self) -> Option<RpcNotification> {
        if !self.lagged.swap(false, Ordering::SeqCst) {
            return None;
        }
        // Everything already queued predates the overflow. Drop that epoch
        // before reconciliation so stale transient frames cannot follow it.
        for _ in 0..SUBSCRIBER_CAPACITY {
            if self.receiver.try_recv().is_err() {
                break;
            }
        }
        Some(local_lag_marker())
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<RpcNotification, RecvTimeoutError> {
        if let Some(marker) = self.take_lag_marker() {
            return Ok(marker);
        }
        self.receiver.recv_timeout(timeout)
    }

    pub fn try_recv(&self) -> Result<RpcNotification, TryRecvError> {
        if let Some(marker) = self.take_lag_marker() {
            return Ok(marker);
        }
        self.receiver.try_recv()
    }
}

/// One complete request/response transaction at a time per connection.
#[derive(Default)]
struct CallGate {
    busy: Mutex<bool>,
    ready: Condvar,
}

impl CallGate {
    fn acquire(&self, deadline: Instant) -> Result<CallPermit<'_>, ClientError> {
        let mut busy = self.busy.lock().unwrap();
        while *busy {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(ClientError::Timeout);
            }
            let (next, result) = self.ready.wait_timeout(busy, remaining).unwrap();
            busy = next;
            if result.timed_out() && *busy {
                return Err(ClientError::Timeout);
            }
        }
        *busy = true;
        Ok(CallPermit(self))
    }
}

struct CallPermit<'a>(&'a CallGate);

impl Drop for CallPermit<'_> {
    fn drop(&mut self) {
        *self.0.busy.lock().unwrap() = false;
        self.0.ready.notify_one();
    }
}

/// A connected, handshaken client. Requests are sequential (the daemon's
/// model); notifications fan out to every [`DaemonClient::subscribe`]d
/// receiver as the reader thread receives them, including between a request
/// and its response.
pub struct DaemonClient {
    stream: Mutex<UnixStream>,
    responses: Mutex<Receiver<RpcResponse>>,
    subscribers: Arc<Mutex<Vec<Subscriber>>>,
    call_gate: CallGate,
    next_id: AtomicI64,
    handshake: HandshakeResponse,
    call_timeout: Duration,
}

impl DaemonClient {
    pub fn connect(endpoint: &Endpoint) -> Result<DaemonClient, ClientError> {
        DaemonClient::connect_with_timeout(endpoint, DEFAULT_CONNECT_TIMEOUT)
    }

    /// Connect and handshake within `timeout` — a daemon that accepts but
    /// never answers cannot park the caller.
    pub fn connect_with_timeout(
        endpoint: &Endpoint,
        timeout: Duration,
    ) -> Result<DaemonClient, ClientError> {
        let stream = UnixStream::connect(&endpoint.socket_path).map_err(ClientError::Connect)?;
        stream.set_read_timeout(Some(timeout)).map_err(ClientError::Connect)?;
        let mut writer = stream.try_clone().map_err(ClientError::Connect)?;
        let mut reader = BufReader::new(stream.try_clone().map_err(ClientError::Connect)?);

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
        let line = read_frame(&mut reader).map_err(|error| {
            if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)
            {
                ClientError::Timeout
            } else if error.kind() == std::io::ErrorKind::InvalidData {
                ClientError::Protocol(error.to_string())
            } else {
                ClientError::Connect(error)
            }
        })?.ok_or(ClientError::Disconnected)?;
        let response: RpcResponse = serde_json::from_slice(&line)
            .map_err(|error| ClientError::Protocol(format!("handshake reply: {error}")))?;
        let handshake: HandshakeResponse = match response {
            RpcResponse::Success(success) => serde_json::from_value(success.result)
                .map_err(|error| ClientError::Protocol(format!("handshake result: {error}")))?,
            RpcResponse::Failure(failure) => return Err(ClientError::Handshake(failure.error)),
        };
        // Post-handshake reads block indefinitely; deadlines live on `call`
        // and on the subscribers, not on the socket.
        stream.set_read_timeout(None).map_err(ClientError::Connect)?;

        // The reader thread splits the stream: responses (frames with an id)
        // answer `call`; notifications fan out to subscribers. It exits when
        // the socket closes, closing the channels behind it.
        let (response_sender, responses) = std::sync::mpsc::channel::<RpcResponse>();
        let subscribers: Arc<Mutex<Vec<Subscriber>>> = Arc::new(Mutex::new(Vec::new()));
        let reader_subscribers = subscribers.clone();
        std::thread::Builder::new()
            .name("bridge-client-reader".into())
            .spawn(move || read_frames(reader, response_sender, reader_subscribers))
            .expect("client reader thread spawns");

        Ok(DaemonClient {
            stream: Mutex::new(stream),
            responses: Mutex::new(responses),
            subscribers,
            call_gate: CallGate::default(),
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

    /// Subscribe to the notification stream. Every subscriber gets every
    /// notification from now on; one that stops draining loses live frames
    /// and receives a `stream-lagged` marker when it resumes — replay durable
    /// history, exactly as for daemon-side lag.
    pub fn subscribe(&self) -> NotificationSubscription {
        let (sender, receiver) = std::sync::mpsc::sync_channel(SUBSCRIBER_CAPACITY);
        let lagged = Arc::new(AtomicBool::new(false));
        self.subscribers.lock().unwrap().push(Subscriber { sender, lagged: lagged.clone() });
        NotificationSubscription { receiver, lagged }
    }

    /// Whether a call is in flight on this connection right now. The daemon
    /// serves each connection sequentially, so a caller holding several
    /// clients can use this to land new work on an idle one instead of
    /// queueing behind a slow request.
    pub fn busy(&self) -> bool {
        *self.call_gate.busy.lock().unwrap()
    }

    /// Call a registry method with the client's default timeout.
    pub fn call(&self, method: MethodName, params: Option<Value>) -> Result<Value, ClientError> {
        self.call_raw(method.as_str(), params, self.call_timeout)
    }

    /// Call a registry method within an explicit deadline budget.
    pub fn call_with_timeout(
        &self,
        method: MethodName,
        params: Option<Value>,
        timeout: Duration,
    ) -> Result<Value, ClientError> {
        self.call_raw(method.as_str(), params, timeout)
    }

    /// Call by wire name — for reserved methods and forward-compat tooling.
    pub fn call_raw(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
    ) -> Result<Value, ClientError> {
        let deadline = Instant::now().checked_add(timeout).ok_or(ClientError::Timeout)?;
        let _permit = self.call_gate.acquire(deadline)?;
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let params = match params {
            Some(value) => Some(
                Params::new(value)
                    .map_err(|error| ClientError::Protocol(error.to_string()))?,
            ),
            None => None,
        };
        let request = RpcRequest::new(RequestId::Number(id), method, params);
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ClientError::Timeout);
        }
        let mut stream = self.stream.lock().unwrap();
        stream.set_write_timeout(Some(remaining)).map_err(|_| ClientError::Disconnected)?;
        let written = write_frame(&mut *stream, &request);
        let _ = stream.set_write_timeout(None);
        drop(stream);
        if let Err(error) = written {
            return Err(match error.kind() {
                std::io::ErrorKind::InvalidData => ClientError::Protocol(error.to_string()),
                _ => {
                    // write_all may have emitted a prefix. Reusing this socket
                    // would append the next request to a corrupt JSON frame.
                    let _ = self.stream.lock().unwrap().shutdown(std::net::Shutdown::Both);
                    ClientError::Disconnected
                }
            });
        }
        let responses = self.responses.lock().unwrap();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(ClientError::Timeout);
            }
            let response = match responses.recv_timeout(remaining) {
                Ok(response) => response,
                Err(RecvTimeoutError::Timeout) => return Err(ClientError::Timeout),
                Err(RecvTimeoutError::Disconnected) => return Err(ClientError::Disconnected),
            };
            let (response_id, outcome) = match response {
                RpcResponse::Success(success) => (success.id, Ok(success.result)),
                RpcResponse::Failure(failure) => {
                    (failure.id, Err(ClientError::Rpc(failure.error)))
                }
            };
            match serde_json::to_value(&response_id).unwrap().as_i64() {
                // A response to an earlier call whose deadline passed: stale,
                // discard so this call pairs with its own response.
                Some(stale) if stale < id => continue,
                Some(matched) if matched == id => return outcome,
                other => {
                    return Err(ClientError::Protocol(format!(
                        "response for id {other:?} arrived while awaiting {id}"
                    )))
                }
            }
        }
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

impl Drop for DaemonClient {
    fn drop(&mut self) {
        // The reader thread owns a clone of this socket; shutting the socket
        // down (not merely closing this handle) unblocks its read with EOF so
        // the thread exits and the daemon's connection slot is released.
        let _ = self.stream.lock().unwrap().shutdown(std::net::Shutdown::Both);
    }
}

fn write_frame<T: serde::Serialize>(writer: &mut impl Write, frame: &T) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(frame)?;
    line.push(b'\n');
    if line.len() > bridged::MAX_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("request frame exceeds {} bytes", bridged::MAX_FRAME_BYTES),
        ));
    }
    writer.write_all(&line)?;
    writer.flush()
}

fn read_frame(reader: &mut impl BufRead) -> std::io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    let mut bounded = reader.take((MAX_SERVER_FRAME_BYTES + 1) as u64);
    let read = bounded.read_until(b'\n', &mut line)?;
    if read == 0 {
        return Ok(None);
    }
    if line.len() > MAX_SERVER_FRAME_BYTES || line.last() != Some(&b'\n') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("daemon frame exceeds {MAX_SERVER_FRAME_BYTES} bytes"),
        ));
    }
    Ok(Some(line))
}

fn read_frames(
    mut reader: BufReader<UnixStream>,
    responses: Sender<RpcResponse>,
    subscribers: Arc<Mutex<Vec<Subscriber>>>,
) {
    // On exit, drop every subscriber's sender so their receivers observe
    // `Disconnected` — a dead connection must be visible, not a silent stall.
    struct ClearOnExit(Arc<Mutex<Vec<Subscriber>>>);
    impl Drop for ClearOnExit {
        fn drop(&mut self) {
            self.0.lock().unwrap().clear();
        }
    }
    let _clear = ClearOnExit(subscribers.clone());
    loop {
        let line = match read_frame(&mut reader) {
            Ok(Some(line)) => line,
            Ok(None) | Err(_) => return,
        };
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        if let Ok(response) = serde_json::from_slice::<RpcResponse>(&line) {
            if responses.send(response).is_err() {
                return;
            }
            continue;
        }
        if let Ok(notification) = serde_json::from_slice::<RpcNotification>(&line) {
            fan_out(&subscribers, &notification);
        }
        // Unknown frames are skipped: additive servers may send shapes a
        // minor-older client does not know.
    }
}

/// Deliver to every subscriber; lag debt is stored outside the bounded queue
/// so it remains observable even if the producer goes quiet after overflow.
fn fan_out(subscribers: &Arc<Mutex<Vec<Subscriber>>>, notification: &RpcNotification) {
    let mut registered = subscribers.lock().unwrap();
    registered.retain_mut(|subscriber| {
        match subscriber.sender.try_send(notification.clone()) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                subscriber.lagged.store(true, Ordering::SeqCst);
                true
            }
            Err(TrySendError::Disconnected(_)) => false,
        }
    });
}

fn local_lag_marker() -> RpcNotification {
    RpcNotification::new(
        "stream-lagged",
        Params::new(serde_json::json!({})).ok(),
    )
}

/// One durable agent event observed on a session, live or replayed.
#[derive(Debug, Clone)]
pub struct SessionEvent {
    pub sequence: i64,
    pub payload: Value,
}

/// A gap-free stream of one session's durable events across disconnects and
/// live-channel lag. It owns its notification subscription, tracks the
/// durable cursor, and replays whenever the live channel proves unreliable —
/// exactly the notify-then-replay contract. Transient frames (sequence zero)
/// pass through live and are never replayed.
pub struct SessionEventStream<'client> {
    client: &'client DaemonClient,
    subscription: NotificationSubscription,
    session_id: String,
    /// The last sequence handed to the consumer — safe to resume from after
    /// a reconnect. Only delivery advances it.
    cursor: i64,
    /// The highest sequence fetched into `pending` (dedup/gap watermark).
    fetched: i64,
    /// Replayed events not yet handed to the consumer.
    pending: std::collections::VecDeque<SessionEvent>,
}

impl<'client> SessionEventStream<'client> {
    /// Start a stream from a durable cursor (0 for the beginning). Subscribes
    /// first, then replays, closing the subscribe-before-replay race: live
    /// durable events at or below the replayed watermark are discarded as
    /// duplicates.
    pub fn new(
        client: &'client DaemonClient,
        session_id: impl Into<String>,
        after_sequence: i64,
    ) -> Result<SessionEventStream<'client>, ClientError> {
        let subscription = client.subscribe();
        let mut stream = SessionEventStream {
            client,
            subscription,
            session_id: session_id.into(),
            cursor: after_sequence,
            fetched: after_sequence,
            pending: Default::default(),
        };
        stream.replay()?;
        Ok(stream)
    }

    /// The consumer's resume point: the last sequence actually delivered by
    /// [`SessionEventStream::next`], never one that is merely queued.
    pub fn cursor(&self) -> i64 {
        self.cursor
    }

    fn replay(&mut self) -> Result<(), ClientError> {
        for event in self.client.replay_session_events(&self.session_id, self.fetched)? {
            let sequence = event["sequence"].as_i64().unwrap_or(0);
            if sequence > self.fetched {
                self.fetched = sequence;
                self.pending.push_back(SessionEvent { sequence, payload: event });
            }
        }
        Ok(())
    }

    /// The next event for this session: replayed backlog first, then live.
    /// `Ok(None)` when `deadline` passes with nothing relevant. A
    /// `stream-lagged` marker (daemon- or client-side) triggers a replay
    /// instead of surfacing.
    pub fn next(&mut self, deadline: Instant) -> Result<Option<SessionEvent>, ClientError> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                if event.sequence > 0 {
                    self.cursor = event.sequence;
                }
                return Ok(Some(event));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            let notification = match self
                .subscription
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
                    if sequence <= self.fetched {
                        continue; // Duplicate of a replayed event.
                    }
                    if sequence > self.fetched + 1 {
                        // A gap in durable sequences: the live channel missed
                        // events. Recover from the store, not the channel.
                        self.replay()?;
                        continue;
                    }
                    self.fetched = sequence;
                    self.cursor = sequence;
                    return Ok(Some(SessionEvent { sequence, payload }));
                }
                // The live channel dropped frames (daemon-side or in this
                // client): durable history is intact — replay from the
                // watermark.
                "stream-lagged" => self.replay()?,
                _ => continue,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notification(method: &str) -> RpcNotification {
        RpcNotification::new(method, Params::new(serde_json::json!({})).ok())
    }

    #[test]
    fn every_subscriber_receives_every_notification() {
        let subscribers = Arc::new(Mutex::new(Vec::new()));
        let (sender_a, receiver_a) = std::sync::mpsc::sync_channel(SUBSCRIBER_CAPACITY);
        let (sender_b, receiver_b) = std::sync::mpsc::sync_channel(SUBSCRIBER_CAPACITY);
        let lagged_a = Arc::new(AtomicBool::new(false));
        let lagged_b = Arc::new(AtomicBool::new(false));
        subscribers.lock().unwrap().push(Subscriber { sender: sender_a, lagged: lagged_a.clone() });
        subscribers.lock().unwrap().push(Subscriber { sender: sender_b, lagged: lagged_b.clone() });
        let receiver_a = NotificationSubscription { receiver: receiver_a, lagged: lagged_a };
        let receiver_b = NotificationSubscription { receiver: receiver_b, lagged: lagged_b };
        fan_out(&subscribers, &notification("state-changed"));
        fan_out(&subscribers, &notification("adapters-changed"));
        for receiver in [&receiver_a, &receiver_b] {
            assert_eq!(receiver.try_recv().unwrap().method, "state-changed");
            assert_eq!(receiver.try_recv().unwrap().method, "adapters-changed");
        }
    }

    #[test]
    fn an_overflowed_subscriber_gets_a_lag_marker_when_it_drains() {
        let subscribers = Arc::new(Mutex::new(Vec::new()));
        let (sender, receiver) = std::sync::mpsc::sync_channel(SUBSCRIBER_CAPACITY);
        let lagged = Arc::new(AtomicBool::new(false));
        subscribers.lock().unwrap().push(Subscriber { sender, lagged: lagged.clone() });
        let receiver = NotificationSubscription { receiver, lagged };
        for _ in 0..(SUBSCRIBER_CAPACITY + 8) {
            fan_out(&subscribers, &notification("session-output"));
        }
        // No later delivery is needed to make the lag marker observable.
        assert_eq!(receiver.try_recv().unwrap().method, "stream-lagged");
        assert_eq!(receiver.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn dropped_subscribers_unregister() {
        let subscribers = Arc::new(Mutex::new(Vec::new()));
        let (sender, receiver) = std::sync::mpsc::sync_channel(SUBSCRIBER_CAPACITY);
        subscribers.lock().unwrap().push(Subscriber {
            sender,
            lagged: Arc::new(AtomicBool::new(false)),
        });
        drop(receiver);
        fan_out(&subscribers, &notification("state-changed"));
        assert!(subscribers.lock().unwrap().is_empty());
    }

    #[test]
    fn call_gate_is_single_flight_and_waiting_consumes_the_deadline() {
        let gate = CallGate::default();
        let first = gate.acquire(Instant::now() + Duration::from_secs(1)).unwrap();
        assert!(matches!(
            gate.acquire(Instant::now() + Duration::from_millis(10)),
            Err(ClientError::Timeout)
        ));
        drop(first);
        assert!(gate.acquire(Instant::now() + Duration::from_secs(1)).is_ok());
    }

    #[test]
    fn outbound_and_inbound_frames_are_bounded() {
        let oversized = "x".repeat(bridged::MAX_FRAME_BYTES + 1);
        let mut sink = Vec::new();
        assert_eq!(write_frame(&mut sink, &oversized).unwrap_err().kind(), std::io::ErrorKind::InvalidData);
        let mut large_valid = vec![b'x'; 2 * 1024 * 1024];
        large_valid.push(b'\n');
        assert_eq!(
            read_frame(&mut std::io::Cursor::new(large_valid)).unwrap().unwrap().len(),
            2 * 1024 * 1024 + 1
        );
        let mut input = std::io::Cursor::new(vec![b'x'; MAX_SERVER_FRAME_BYTES + 1]);
        assert_eq!(read_frame(&mut input).unwrap_err().kind(), std::io::ErrorKind::InvalidData);
    }
}
