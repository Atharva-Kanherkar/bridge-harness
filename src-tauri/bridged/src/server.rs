//! The socket server: accept loops, per-connection framing, the handshake
//! gate, request handling, and event forwarding.
//!
//! Two transports share one connection layer. The Unix socket frames
//! newline-delimited JSON — one complete JSON-RPC frame per line in both
//! directions. The optional remote listener (see [`crate::remote`]) carries the
//! same frames as WebSocket text messages. Both reduce to a [`FrameSource`]
//! and a [`FrameSink`], so the handshake gate, limits, dispatch and event
//! forwarding below are written once. Requests on a connection are handled sequentially in
//! arrival order; notifications from the event hub interleave between frames
//! (each write holds the connection's writer lock for exactly one line).
//!
//! Every read carries a short timeout so the loops can observe shutdown and
//! connection-close flags: an idle connection drains within one poll interval
//! of a shutdown request, and a connection that never handshakes is reclaimed
//! at the handshake deadline.

use crate::dispatch::dispatch;
use crate::{Daemon, DaemonState, MAX_CONNECTIONS, MAX_FRAME_BYTES};
use bridge_core::BridgeCore;
use bridge_protocol::{
    negotiate, CancelParams, ErrorCode, HandshakeRequest, MethodName, Params, RequestId,
    ResponseId, RpcError, RpcRequest, RpcResponse, CANCEL_METHOD, HANDSHAKE_METHOD,
};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How often blocked loops wake to check shutdown/close flags.
const POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Run the accept loop until shutdown is requested. Polling (rather than a
/// blocking accept) lets the signal handler stop the loop without tricks.
pub fn serve(daemon: &Daemon, listener: UnixListener) -> std::io::Result<()> {
    serve_with_remote(daemon, listener, None)
}

/// [`serve`], plus the optional remote WebSocket listener on its own accept
/// thread. Both transports draw from the same connection cap and stop on the
/// same shutdown flag; a failure of either loop stops the other.
pub fn serve_with_remote(
    daemon: &Daemon,
    listener: UnixListener,
    remote: Option<crate::remote::RemoteListener>,
) -> std::io::Result<()> {
    std::thread::scope(|scope| {
        let remote_loop = remote.map(|remote| {
            scope.spawn(move || {
                let result = crate::remote::serve_remote(daemon, remote);
                if result.is_err() {
                    daemon.state.shutting_down.store(true, Ordering::SeqCst);
                }
                result
            })
        });
        let local = serve_unix(daemon, listener);
        if local.is_err() {
            daemon.state.shutting_down.store(true, Ordering::SeqCst);
        }
        let remote = match remote_loop {
            Some(handle) => handle.join().unwrap_or_else(|_| {
                Err(std::io::Error::new(std::io::ErrorKind::Other, "remote accept loop panicked"))
            }),
            None => Ok(()),
        };
        local.and(remote)
    })
}

fn serve_unix(daemon: &Daemon, listener: UnixListener) -> std::io::Result<()> {
    listener.set_nonblocking(true)?;
    while !daemon.state.shutting_down.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nonblocking(false);
                if !admit(daemon) {
                    refuse_overloaded(stream);
                    continue;
                }
                spawn_connection(daemon, move |core, state, events| {
                    handle_unix(core, state, events, stream)
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Reserve a connection slot, or report that the cap is reached. The slot is
/// released by [`spawn_connection`] when the connection ends. Both accept
/// loops call this concurrently, so the check and the increment are one atomic
/// step: two loops at `cap - 1` cannot both succeed.
pub(crate) fn admit(daemon: &Daemon) -> bool {
    reserve_slot(&daemon.state.connections, MAX_CONNECTIONS)
}

fn reserve_slot(connections: &std::sync::atomic::AtomicUsize, cap: usize) -> bool {
    connections
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| (count < cap).then_some(count + 1))
        .is_ok()
}

/// Run one admitted connection on its own thread and release its slot after.
pub(crate) fn spawn_connection<F>(daemon: &Daemon, run: F)
where
    F: FnOnce(&Arc<BridgeCore>, &Arc<DaemonState>, &crate::EventHub) -> std::io::Result<()>
        + Send
        + 'static,
{
    let core = daemon.core.clone();
    let state = daemon.state.clone();
    let events = daemon.events.clone();
    std::thread::Builder::new()
        .name("bridged-connection".into())
        .spawn(move || {
            let _ = run(&core, &state, &events);
            state.connections.fetch_sub(1, Ordering::SeqCst);
        })
        .expect("connection thread spawns");
}

/// One `overloaded` frame, then close — the defined refusal beyond the
/// connection cap.
fn refuse_overloaded(mut stream: UnixStream) {
    let _ = write_line(&mut stream, &overloaded_response());
}

pub(crate) fn overloaded_response() -> RpcResponse {
    RpcResponse::error(
        ResponseId::Null,
        RpcError::new(ErrorCode::Overloaded, "the daemon is at its connection limit"),
    )
}

fn write_line<T: serde::Serialize>(writer: &mut impl Write, frame: &T) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(frame)?;
    line.push(b'\n');
    writer.write_all(&line)?;
    writer.flush()
}

/// Where a connection's frames are written. One implementation per transport;
/// every write is one whole frame, serialised against concurrent writers.
pub(crate) trait FrameSink: Send + Sync {
    fn send(&self, frame: &[u8]) -> std::io::Result<()>;
    /// Tear the connection down; the paired source then reads EOF.
    fn close(&self);
}

/// Where a connection's frames are read from.
pub(crate) trait FrameSource {
    fn next(&mut self) -> std::io::Result<Frame>;

    /// An absolute handshake deadline this transport already started (the
    /// WebSocket one starts at TCP accept, so the HTTP upgrade and the protocol
    /// handshake share a single budget). `None` starts it at the first read.
    fn handshake_deadline(&self) -> Option<Instant> {
        None
    }
}

fn write_frame<T: serde::Serialize>(sink: &dyn FrameSink, frame: &T) -> std::io::Result<()> {
    sink.send(&serde_json::to_vec(frame)?)
}

struct UnixSink(Mutex<UnixStream>);

impl FrameSink for UnixSink {
    fn send(&self, frame: &[u8]) -> std::io::Result<()> {
        let mut line = Vec::with_capacity(frame.len() + 1);
        line.extend_from_slice(frame);
        line.push(b'\n');
        let mut stream = self.0.lock().unwrap();
        stream.write_all(&line)?;
        stream.flush()
    }

    fn close(&self) {
        let _ = self.0.lock().unwrap().shutdown(std::net::Shutdown::Both);
    }
}

/// One read attempt's outcome.
pub(crate) enum Frame {
    /// A complete newline-terminated line.
    Line(Vec<u8>),
    /// The frame exceeded [`MAX_FRAME_BYTES`]; the connection must close.
    TooLong,
    /// The peer closed the connection.
    Eof,
    /// The read timed out with no complete frame; poll flags and retry.
    /// Partial bytes stay buffered — a frame may arrive across many polls.
    Idle,
}

/// A frame reader that survives read timeouts: bytes consumed before a
/// timeout are kept in `buffer`, so a slowly-arriving frame is assembled
/// across polls instead of being dropped.
struct FrameReader {
    reader: BufReader<UnixStream>,
    buffer: Vec<u8>,
}

impl FrameReader {
    fn new(stream: UnixStream) -> FrameReader {
        FrameReader { reader: BufReader::new(stream), buffer: Vec::new() }
    }
}

impl FrameSource for FrameReader {
    fn next(&mut self) -> std::io::Result<Frame> {
        loop {
            if self.buffer.len() > MAX_FRAME_BYTES {
                self.buffer.clear();
                return Ok(Frame::TooLong);
            }
            let budget = (MAX_FRAME_BYTES + 1 - self.buffer.len()) as u64;
            let mut bounded = (&mut self.reader).take(budget);
            match bounded.read_until(b'\n', &mut self.buffer) {
                // EOF — a partial buffered line is not a frame either way.
                Ok(0) => return Ok(Frame::Eof),
                Ok(_) => {
                    if self.buffer.last() == Some(&b'\n') {
                        return Ok(Frame::Line(std::mem::take(&mut self.buffer)));
                    }
                    // No newline yet: either the cap was hit (checked at the
                    // top of the loop) or more bytes are coming.
                    continue;
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(Frame::Idle);
                }
                Err(error) => return Err(error),
            }
        }
    }
}

/// Serve one Unix-socket connection.
fn handle_unix(
    core: &Arc<BridgeCore>,
    state: &Arc<DaemonState>,
    events: &crate::EventHub,
    stream: UnixStream,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(POLL_INTERVAL))?;
    let reader = FrameReader::new(stream.try_clone()?);
    let sink: Arc<dyn FrameSink> = Arc::new(UnixSink(Mutex::new(stream)));
    handle_connection(core, state, events, reader, sink)
}

/// The transport-independent connection: handshake gate, event forwarding,
/// then the sequential request loop.
pub(crate) fn handle_connection(
    core: &Arc<BridgeCore>,
    state: &Arc<DaemonState>,
    events: &crate::EventHub,
    mut reader: impl FrameSource,
    writer: Arc<dyn FrameSink>,
) -> std::io::Result<()> {
    // --- handshake gate --------------------------------------------------
    let response = match expect_handshake(state, &mut reader) {
        Ok(response) => response,
        Err(response) => {
            let _ = write_frame(&*writer, &response);
            writer.close();
            return Ok(());
        }
    };
    // Register with the hub BEFORE acknowledging the handshake: an event
    // published the instant after the response is sent must reach this
    // connection.
    let subscription = events.register();
    write_frame(&*writer, &response)?;

    // --- event forwarding ------------------------------------------------
    // The `closed` flag bounds the forwarder's lifetime: it polls between
    // queue reads and exits within one interval of the request loop ending,
    // so joining it cannot stall the connection slot.
    let closed = Arc::new(AtomicBool::new(false));
    let forward_writer = writer.clone();
    let forward_closed = closed.clone();
    let forwarder = std::thread::Builder::new()
        .name("bridged-events".into())
        .spawn(move || {
            loop {
                match subscription.recv_timeout(POLL_INTERVAL) {
                    Ok(notification) => {
                        if write_frame(&*forward_writer, &notification).is_err() {
                            break;
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        if forward_closed.load(Ordering::SeqCst) {
                            break;
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .expect("event forwarder spawns");

    // --- request loop ------------------------------------------------------
    let result = request_loop(core, state, &mut reader, &*writer);
    closed.store(true, Ordering::SeqCst);
    writer.close();
    let _ = forwarder.join();
    result
}

/// Enforce handshake-first: the first frame must be a `protocol/handshake`
/// request with the correct per-install token and a compatible version, and
/// it must arrive before the handshake deadline — a silent connection cannot
/// hold a slot. `Ok` carries the success response to send once the caller has
/// subscribed; `Err` carries the rejection to send before closing.
fn expect_handshake(
    state: &DaemonState,
    reader: &mut impl FrameSource,
) -> Result<RpcResponse, RpcResponse> {
    let deadline = reader
        .handshake_deadline()
        .unwrap_or_else(|| Instant::now() + state.handshake_timeout);
    let line = loop {
        match reader.next() {
            Ok(Frame::Line(line)) => break line,
            Ok(Frame::Idle) => {
                if state.shutting_down.load(Ordering::SeqCst) {
                    return Err(RpcResponse::error(
                        ResponseId::Null,
                        RpcError::new(ErrorCode::ShuttingDown, "the daemon is shutting down"),
                    ));
                }
                if Instant::now() >= deadline {
                    return Err(RpcResponse::error(
                        ResponseId::Null,
                        RpcError::new(
                            ErrorCode::InvalidRequest,
                            "the connection did not handshake within the deadline",
                        ),
                    ));
                }
            }
            Ok(Frame::TooLong) => {
                return Err(RpcResponse::error(
                    ResponseId::Null,
                    RpcError::new(
                        ErrorCode::InvalidRequest,
                        format!("request frame exceeds {MAX_FRAME_BYTES} bytes"),
                    ),
                ))
            }
            Ok(Frame::Eof) | Err(_) => {
                return Err(RpcResponse::parse_error("the connection closed before a handshake"))
            }
        }
    };
    let request: RpcRequest = serde_json::from_slice(&line).map_err(|_| {
        invalid_request_for_line(&line, "the first frame was not a JSON-RPC 2.0 request")
    })?;
    let id = request.id.clone();
    if request.method != HANDSHAKE_METHOD {
        return Err(RpcResponse::error(
            id,
            RpcError::new(
                ErrorCode::InvalidRequest,
                format!("the first request on a connection must be {HANDSHAKE_METHOD}"),
            ),
        ));
    }
    let handshake: HandshakeRequest = request
        .params
        .map(Params::into_value)
        .ok_or(())
        .and_then(|value| serde_json::from_value(value).map_err(|_| ()))
        .map_err(|()| {
            RpcResponse::error(
                id.clone(),
                RpcError::new(ErrorCode::InvalidParams, "handshake params are invalid"),
            )
        })?;
    // Token first: an unauthenticated peer learns nothing about the server —
    // not even its protocol version.
    let presented = handshake.auth_token.as_deref().unwrap_or_default();
    if !crate::token_matches(&state.auth_token, presented) {
        return Err(RpcResponse::error(
            id,
            RpcError::new(
                ErrorCode::Unauthorized,
                format!(
                    "the handshake must carry the token from the daemon's {} file",
                    crate::TOKEN_FILE_NAME
                ),
            ),
        ));
    }
    match negotiate(&handshake) {
        Ok(mut accepted) => {
            // Negotiation is about the protocol; only the serving process
            // knows which binary it is, so the identity is stamped here.
            accepted.build_id = state.build_id.clone();
            Ok(RpcResponse::result(
                id,
                serde_json::to_value(accepted).expect("handshake response serializes"),
            ))
        }
        Err(error) => Err(RpcResponse::error(id, error)),
    }
}

/// Handle requests sequentially until EOF or shutdown. Idle polls let an open
/// but quiet connection observe shutdown within one interval.
fn request_loop(
    core: &Arc<BridgeCore>,
    state: &Arc<DaemonState>,
    reader: &mut impl FrameSource,
    writer: &dyn FrameSink,
) -> std::io::Result<()> {
    loop {
        let line = match reader.next()? {
            Frame::Eof => return Ok(()),
            Frame::Idle => {
                if state.shutting_down.load(Ordering::SeqCst) {
                    return Ok(());
                }
                continue;
            }
            Frame::TooLong => {
                let response = RpcResponse::error(
                    ResponseId::Null,
                    RpcError::new(
                        ErrorCode::InvalidRequest,
                        format!("request frame exceeds {MAX_FRAME_BYTES} bytes"),
                    ),
                );
                write_frame(writer, &response)?;
                return Ok(());
            }
            Frame::Line(line) => line,
        };
        if line.iter().all(|byte| byte.is_ascii_whitespace()) {
            continue;
        }
        if let Some(response) = handle_frame(core, state, &line) {
            write_frame(writer, &response)?;
        }
    }
}

/// Decode and answer one frame. Notifications get no response per JSON-RPC;
/// everything else gets exactly one, carrying the request's own id whenever
/// it can be recovered.
fn handle_frame(
    core: &Arc<BridgeCore>,
    state: &Arc<DaemonState>,
    line: &[u8],
) -> Option<RpcResponse> {
    let request: RpcRequest = match serde_json::from_slice(line) {
        Ok(request) => request,
        Err(_) => return classify_undecodable(line),
    };
    let id = request.id.clone();
    // An in-flight request finishes normally during shutdown; only frames
    // arriving after the flag are refused — with their own id.
    if state.shutting_down.load(Ordering::SeqCst) {
        return Some(RpcResponse::error(
            id,
            RpcError::new(ErrorCode::ShuttingDown, "the daemon is shutting down"),
        ));
    }
    if request.method == HANDSHAKE_METHOD {
        return Some(RpcResponse::error(
            id,
            RpcError::new(ErrorCode::InvalidRequest, "the connection already handshook"),
        ));
    }
    if request.method == CANCEL_METHOD {
        // The contract defines $/cancel as a notification; the request form
        // is a malformed frame and saying "ok" to it would claim a
        // cancellation that never happened.
        return Some(RpcResponse::error(
            id,
            RpcError::new(
                ErrorCode::InvalidRequest,
                "$/cancel is a notification, not a request; requests on this \
                 connection are handled sequentially, so there is never an \
                 in-flight request to cancel when one is read — use \
                 sessions/interrupt_turn to interrupt a running turn",
            ),
        ));
    }
    let Some(method) = MethodName::parse(&request.method) else {
        return Some(RpcResponse::error(
            id,
            RpcError::new(
                ErrorCode::MethodNotFound,
                format!("unknown method {}", request.method),
            ),
        ));
    };
    let params = request.params.map(Params::into_value);
    match dispatch(core, method, params) {
        Ok(result) => Some(RpcResponse::result(id, result)),
        Err(error) => Some(RpcResponse::error(id, error)),
    }
}

/// A frame that did not decode as a request: distinguish invalid JSON
/// (`parse_error`), a notification (consumed without a response — `$/cancel`
/// is validated, everything else ignored), and a structurally invalid request
/// (`invalid_request`, echoing the id when one is recoverable).
fn classify_undecodable(line: &[u8]) -> Option<RpcResponse> {
    let Ok(value) = serde_json::from_slice::<Value>(line) else {
        return Some(RpcResponse::parse_error("the frame was not valid JSON"));
    };
    if let Ok(notification) = serde_json::from_slice::<bridge_protocol::RpcNotification>(line) {
        if notification.method == CANCEL_METHOD {
            // Validated but necessarily a no-op: requests on this connection
            // are sequential, so nothing is in flight while frames are read.
            let _ = notification
                .params
                .map(Params::into_value)
                .map(serde_json::from_value::<CancelParams>);
        }
        return None;
    }
    Some(invalid_request_response(&value, "the frame was not a JSON-RPC 2.0 request"))
}

fn invalid_request_for_line(line: &[u8], message: &str) -> RpcResponse {
    match serde_json::from_slice::<Value>(line) {
        Ok(value) => invalid_request_response(&value, message),
        Err(_) => RpcResponse::parse_error("the frame was not valid JSON"),
    }
}

/// An `invalid_request` failure that echoes the offending frame's id when it
/// carries a usable one, as JSON-RPC asks.
fn invalid_request_response(value: &Value, message: &str) -> RpcResponse {
    let id = value
        .get("id")
        .cloned()
        .and_then(|id| serde_json::from_value::<RequestId>(id).ok())
        .map(ResponseId::from)
        .unwrap_or(ResponseId::Null);
    RpcResponse::error(id, RpcError::new(ErrorCode::InvalidRequest, message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn a_slot_is_reserved_only_below_the_cap() {
        let count = AtomicUsize::new(0);
        assert!(reserve_slot(&count, 2));
        assert!(reserve_slot(&count, 2));
        assert!(!reserve_slot(&count, 2));
        assert_eq!(count.load(Ordering::SeqCst), 2, "a refusal must not leak a slot");
    }

    #[test]
    fn racing_reservations_at_the_edge_admit_exactly_the_remaining_slots() {
        for _ in 0..200 {
            let cap = 32;
            let count = Arc::new(AtomicUsize::new(cap - 1));
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let admitted: usize = (0..2)
                .map(|_| {
                    let (count, barrier) = (count.clone(), barrier.clone());
                    std::thread::spawn(move || {
                        barrier.wait();
                        reserve_slot(&count, cap) as usize
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .sum();
            assert_eq!(admitted, 1);
            assert_eq!(count.load(Ordering::SeqCst), cap);
        }
    }
}
