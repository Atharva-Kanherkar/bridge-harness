//! The socket server: accept loop, per-connection framing, the handshake
//! gate, request handling, and event forwarding.
//!
//! Framing is newline-delimited JSON — one complete JSON-RPC frame per line
//! in both directions. Requests on a connection are handled sequentially in
//! arrival order; notifications from the event bus interleave between frames
//! (each write holds the connection's writer lock for exactly one line).

use crate::dispatch::dispatch;
use crate::{Daemon, MAX_CONNECTIONS, MAX_FRAME_BYTES};
use bridge_core::events::ReceiveError;
use bridge_core::BridgeCore;
use bridge_protocol::{
    negotiate, ErrorCode, HandshakeRequest, MethodName, Params, RpcError, RpcNotification,
    RpcRequest, RpcResponse, CANCEL_METHOD, HANDSHAKE_METHOD,
};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Run the accept loop until shutdown is requested. Polling (rather than a
/// blocking accept) lets the signal handler stop the loop without tricks.
pub fn serve(daemon: &Daemon, listener: UnixListener) -> std::io::Result<()> {
    listener.set_nonblocking(true)?;
    while !daemon.state.shutting_down.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nonblocking(false);
                if daemon.state.connections.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
                    refuse_overloaded(stream);
                    continue;
                }
                daemon.state.connections.fetch_add(1, Ordering::SeqCst);
                let core = daemon.core.clone();
                let state = daemon.state.clone();
                std::thread::Builder::new()
                    .name("bridged-connection".into())
                    .spawn(move || {
                        let _ = handle_connection(&core, &state, stream);
                        state.connections.fetch_sub(1, Ordering::SeqCst);
                    })
                    .expect("connection thread spawns");
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// One `overloaded` frame, then close — the defined refusal beyond the
/// connection cap.
fn refuse_overloaded(mut stream: UnixStream) {
    let response = RpcResponse::error(
        bridge_protocol::ResponseId::Null,
        RpcError::new(ErrorCode::Overloaded, "the daemon is at its connection limit"),
    );
    let _ = write_frame(&mut stream, &response);
}

fn write_frame<T: serde::Serialize>(writer: &mut impl Write, frame: &T) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(frame)?;
    line.push(b'\n');
    writer.write_all(&line)?;
    writer.flush()
}

/// Read one newline-terminated frame, enforcing the size cap. `Ok(None)` is
/// a clean EOF; an oversized frame is an error the caller reports and then
/// closes on.
fn read_frame(reader: &mut impl BufRead) -> std::io::Result<Option<Result<Vec<u8>, ()>>> {
    let mut line = Vec::new();
    let mut take = reader.take((MAX_FRAME_BYTES + 1) as u64);
    let read = take.read_until(b'\n', &mut line)?;
    if read == 0 {
        return Ok(None);
    }
    if line.len() > MAX_FRAME_BYTES {
        // Drain nothing further — the connection closes after the error.
        return Ok(Some(Err(())));
    }
    Ok(Some(Ok(line)))
}

fn handle_connection(
    core: &Arc<BridgeCore>,
    state: &Arc<crate::DaemonState>,
    stream: UnixStream,
) -> std::io::Result<()> {
    let reader_stream = stream.try_clone()?;
    let mut reader = BufReader::new(reader_stream);
    let writer = Arc::new(Mutex::new(stream));

    // --- handshake gate --------------------------------------------------
    let response = match expect_handshake(state, &mut reader) {
        Ok(response) => response,
        Err(response) => {
            let _ = write_frame(&mut *writer.lock().unwrap(), &response);
            return Ok(());
        }
    };
    // Subscribe BEFORE acknowledging the handshake: an event published the
    // instant after the response is sent must reach this connection.
    let receiver = core.events.subscribe();
    write_frame(&mut *writer.lock().unwrap(), &response)?;

    // --- event forwarding ------------------------------------------------
    let forward_writer = writer.clone();
    let forwarder = std::thread::Builder::new()
        .name("bridged-events".into())
        .spawn(move || forward_events(receiver, forward_writer))
        .expect("event forwarder spawns");

    // --- request loop ------------------------------------------------------
    let result = request_loop(core, state, &mut reader, &writer);
    // Closing the read side ends the connection; shut the write side down so
    // the forwarder's next send fails and the thread exits.
    let _ = writer.lock().unwrap().shutdown(std::net::Shutdown::Both);
    let _ = forwarder.join();
    result
}

/// Enforce handshake-first: the first frame must be a `protocol/handshake`
/// request with the correct per-install token and a compatible version.
/// `Ok` carries the success response to send once the caller has subscribed;
/// `Err` carries the rejection to send before closing the connection.
fn expect_handshake(
    state: &crate::DaemonState,
    reader: &mut impl BufRead,
) -> Result<RpcResponse, RpcResponse> {
    let line = match read_frame(reader) {
        Ok(Some(Ok(line))) => line,
        Ok(Some(Err(()))) => {
            return Err(RpcResponse::error(
                bridge_protocol::ResponseId::Null,
                RpcError::new(
                    ErrorCode::InvalidRequest,
                    format!("request frame exceeds {MAX_FRAME_BYTES} bytes"),
                ),
            ))
        }
        Ok(None) | Err(_) => {
            return Err(RpcResponse::parse_error("the connection closed before a handshake"))
        }
    };
    let request: RpcRequest = serde_json::from_slice(&line).map_err(|_| {
        RpcResponse::parse_error("the first frame was not a JSON-RPC 2.0 request")
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
                format!("the handshake must carry the token from the daemon's {} file", crate::TOKEN_FILE_NAME),
            ),
        ));
    }
    match negotiate(&handshake) {
        Ok(accepted) => Ok(RpcResponse::result(
            id,
            serde_json::to_value(accepted).expect("handshake response serializes"),
        )),
        Err(error) => Err(RpcResponse::error(id, error)),
    }
}

/// Forward every core event as a JSON-RPC notification until the connection
/// dies. On lag, resend the idempotent refetch hints — durable agent events
/// recover client-side via cursor replay, per the event contract.
fn forward_events(
    mut receiver: bridge_core::events::EventReceiver,
    writer: Arc<Mutex<UnixStream>>,
) {
    loop {
        match receiver.blocking_recv() {
            Ok(event) => {
                let notification = RpcNotification::new(
                    event.kind().as_str(),
                    Params::new(event.payload()).ok(),
                );
                if write_frame(&mut *writer.lock().unwrap(), &notification).is_err() {
                    break;
                }
            }
            Err(ReceiveError::Lagged(_)) => {
                for event in receiver.reconciliation_events() {
                    let notification = RpcNotification::new(
                        event.kind().as_str(),
                        Params::new(event.payload()).ok(),
                    );
                    if write_frame(&mut *writer.lock().unwrap(), &notification).is_err() {
                        return;
                    }
                }
            }
            Err(ReceiveError::Closed) => break,
        }
    }
}

/// Handle requests sequentially until EOF or shutdown.
fn request_loop(
    core: &Arc<BridgeCore>,
    state: &Arc<crate::DaemonState>,
    reader: &mut impl BufRead,
    writer: &Arc<Mutex<UnixStream>>,
) -> std::io::Result<()> {
    loop {
        let line = match read_frame(reader)? {
            None => return Ok(()),
            Some(Err(())) => {
                let response = RpcResponse::error(
                    bridge_protocol::ResponseId::Null,
                    RpcError::new(
                        ErrorCode::InvalidRequest,
                        format!("request frame exceeds {MAX_FRAME_BYTES} bytes"),
                    ),
                );
                write_frame(&mut *writer.lock().unwrap(), &response)?;
                return Ok(());
            }
            Some(Ok(line)) => line,
        };
        if line.iter().all(|byte| byte.is_ascii_whitespace()) {
            continue;
        }
        if state.shutting_down.load(Ordering::SeqCst) {
            let response = RpcResponse::error(
                bridge_protocol::ResponseId::Null,
                RpcError::new(ErrorCode::ShuttingDown, "the daemon is shutting down"),
            );
            write_frame(&mut *writer.lock().unwrap(), &response)?;
            return Ok(());
        }
        let response = handle_frame(core, &line);
        if let Some(response) = response {
            write_frame(&mut *writer.lock().unwrap(), &response)?;
        }
    }
}

/// Decode and answer one frame. Notifications ($/cancel and any other) get no
/// response per JSON-RPC; everything else gets exactly one.
fn handle_frame(core: &Arc<BridgeCore>, line: &[u8]) -> Option<RpcResponse> {
    let request: RpcRequest = match serde_json::from_slice(line) {
        Ok(request) => request,
        Err(_) => {
            // A notification is a valid frame with no id; it gets no reply.
            if serde_json::from_slice::<bridge_protocol::RpcNotification>(line).is_ok() {
                return None;
            }
            return Some(RpcResponse::parse_error("the frame was not a JSON-RPC 2.0 request"));
        }
    };
    let id = request.id.clone();
    if request.method == HANDSHAKE_METHOD {
        return Some(RpcResponse::error(
            id,
            RpcError::new(ErrorCode::InvalidRequest, "the connection already handshook"),
        ));
    }
    if request.method == CANCEL_METHOD {
        // $/cancel is specified as a notification; sent as a request it is
        // still best-effort and there is nothing concurrent to cancel on a
        // sequential connection.
        return Some(RpcResponse::result(id, Value::Null));
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
