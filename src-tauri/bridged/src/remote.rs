//! The optional remote transport: JSON-RPC over WebSocket text messages.
//!
//! Off unless the operator passes `--listen`. It speaks exactly the contract
//! the Unix socket speaks — the same handshake gate, token, limits, dispatch
//! and event forwarding — through the [`FrameSource`]/[`FrameSink`] seam in
//! [`crate::server`]. What this module adds is only what a network listener
//! needs that a mode-0600 socket does not:
//!
//! - **Origin rules.** A WebSocket upgrade that carries an `Origin` header is
//!   refused (403) unless that exact origin is on the allowlist; there is no
//!   wildcard. A request with no `Origin` is not a browser (browsers always
//!   send one), so it is left to the token.
//! - **Loopback by default.** The listener is plaintext, so binding anything
//!   but a loopback address needs an explicit opt-in, meant for a TLS
//!   terminator or tunnel in front of it.
//! - **Frame mapping.** One JSON-RPC frame is one text message; there is no
//!   newline framing. Binary messages and oversized messages end the
//!   connection.

use crate::server::{
    admit, handle_connection, overloaded_response, spawn_connection, Frame, FrameSink, FrameSource,
};
use crate::{Daemon, StartupError, MAX_FRAME_BYTES};
use std::io::ErrorKind;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::http::{header, StatusCode};
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{Error as WsError, Message, WebSocket};

/// How often a blocked read wakes so shutdown and writers are observed.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// A bounded write: a peer that stops reading cannot hold the writer lock
/// (and with it the event forwarder) forever.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// What the operator asked for with `--listen`, validated.
#[derive(Debug, Clone)]
pub struct RemoteConfig {
    addr: SocketAddr,
    allowed_origins: Vec<String>,
}

impl RemoteConfig {
    /// Validate a listen request. Rejects a wildcard or malformed origin, and a
    /// non-loopback address unless `allow_non_loopback` is set.
    pub fn new(
        addr: SocketAddr,
        allowed_origins: Vec<String>,
        allow_non_loopback: bool,
    ) -> Result<RemoteConfig, String> {
        if !addr.ip().is_loopback() && !allow_non_loopback {
            return Err(format!(
                "--listen {addr} is not a loopback address; the listener is plaintext, so \
                 binding it beyond loopback needs --allow-remote-bind (put TLS or a tunnel in \
                 front of it)"
            ));
        }
        let allowed_origins = allowed_origins
            .iter()
            .map(|origin| normalize_origin(origin))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(RemoteConfig { addr, allowed_origins })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

/// `scheme://host[:port]`, lowercased, no path and no wildcard.
fn normalize_origin(origin: &str) -> Result<String, String> {
    let trimmed = origin.trim().trim_end_matches('/').to_ascii_lowercase();
    let valid = trimmed
        .split_once("://")
        .is_some_and(|(scheme, host)| {
            !scheme.is_empty()
                && !host.is_empty()
                && !host.contains(['/', '*', ' '])
                && !scheme.contains('*')
        });
    if valid {
        Ok(trimmed)
    } else {
        Err(format!(
            "--allowed-origin {origin:?} must be an exact origin like https://app.example.com \
             (no wildcard, no path)"
        ))
    }
}

/// A bound remote listener, ready for [`crate::serve_with_remote`].
pub struct RemoteListener {
    listener: TcpListener,
    allowed_origins: Arc<Vec<String>>,
}

impl RemoteListener {
    /// The address actually bound (useful when the config asked for port 0).
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }
}

impl Daemon {
    /// Bind the remote listener. Called only when the operator opted in, so a
    /// daemon started without `--listen` never opens a network port beyond the
    /// health listener.
    pub fn bind_remote(&self, config: &RemoteConfig) -> Result<RemoteListener, StartupError> {
        let listener = TcpListener::bind(config.addr)
            .map_err(|error| StartupError::RemoteBind { addr: config.addr, error })?;
        Ok(RemoteListener {
            listener,
            allowed_origins: Arc::new(config.allowed_origins.clone()),
        })
    }
}

/// Accept loop for the remote listener; stops with the daemon.
pub(crate) fn serve_remote(daemon: &Daemon, remote: RemoteListener) -> std::io::Result<()> {
    remote.listener.set_nonblocking(true)?;
    while !daemon.state.shutting_down.load(Ordering::SeqCst) {
        match remote.listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nonblocking(false);
                if !admit(daemon) {
                    refuse_overloaded(stream);
                    continue;
                }
                let origins = remote.allowed_origins.clone();
                spawn_connection(daemon, move |core, state, events| {
                    // The upgrade runs here, not in the accept loop, so a slow
                    // peer cannot stall other connections; each read is bounded
                    // by the handshake deadline.
                    stream.set_read_timeout(Some(state.handshake_timeout))?;
                    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
                    let Some(socket) = upgrade(stream, &origins) else {
                        return Ok(());
                    };
                    socket.get_ref().set_read_timeout(Some(POLL_INTERVAL))?;
                    let link = Arc::new(WsLink { socket: Mutex::new(socket) });
                    handle_connection(core, state, events, WsSource(link.clone()), link)
                });
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Beyond the connection cap there is no upgrade: a plain HTTP 503 carrying
/// the same `overloaded` error body.
fn refuse_overloaded(mut stream: TcpStream) {
    use std::io::Write;
    let body = serde_json::to_string(&overloaded_response()).unwrap_or_default();
    let _ = write!(
        stream,
        "HTTP/1.1 503 Service Unavailable\r\ncontent-type: application/json\r\n\
         content-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn upgrade(stream: TcpStream, allowed: &[String]) -> Option<WebSocket<TcpStream>> {
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME_BYTES))
        .max_frame_size(Some(MAX_FRAME_BYTES));
    let check = |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
        match origin_verdict(request.headers().get(header::ORIGIN), allowed) {
            Ok(()) => Ok(response),
            Err(reason) => {
                let mut refusal = ErrorResponse::new(Some(reason));
                *refusal.status_mut() = StatusCode::FORBIDDEN;
                Err(refusal)
            }
        }
    };
    tungstenite::accept_hdr_with_config(stream, check, Some(config)).ok()
}

/// No `Origin` means a non-browser client (the token still gates it). A
/// present `Origin` must match the allowlist exactly.
fn origin_verdict(
    origin: Option<&header::HeaderValue>,
    allowed: &[String],
) -> Result<(), String> {
    let Some(origin) = origin else {
        return Ok(());
    };
    let presented = origin.to_str().unwrap_or_default().trim().trim_end_matches('/').to_ascii_lowercase();
    if allowed.iter().any(|candidate| *candidate == presented) {
        Ok(())
    } else {
        Err(format!("origin {presented:?} is not allowed to reach this daemon"))
    }
}

/// One `WebSocket` shared by the reader and the event forwarder. The reader
/// holds the lock only for one short, timed read at a time, so writes interleave
/// between polls.
struct WsLink {
    socket: Mutex<WebSocket<TcpStream>>,
}

struct WsSource(Arc<WsLink>);

impl FrameSource for WsSource {
    fn next(&mut self) -> std::io::Result<Frame> {
        loop {
            let message = self.0.socket.lock().unwrap().read();
            match message {
                Ok(Message::Text(text)) => return Ok(Frame::Line(text.as_bytes().to_vec())),
                Ok(Message::Binary(_)) => {
                    return Err(std::io::Error::new(
                        ErrorKind::InvalidData,
                        "binary WebSocket messages are not part of the protocol",
                    ))
                }
                // Control frames: tungstenite queues the pong itself.
                Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => continue,
                Ok(Message::Close(_)) => return Ok(Frame::Eof),
                Err(WsError::ConnectionClosed | WsError::AlreadyClosed) => return Ok(Frame::Eof),
                Err(WsError::Capacity(_)) => return Ok(Frame::TooLong),
                Err(WsError::Io(error))
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                {
                    // Lock released: give a waiting writer the chance to take
                    // it before this reader polls again.
                    std::thread::sleep(Duration::from_millis(2));
                    return Ok(Frame::Idle);
                }
                Err(WsError::Io(error)) => return Err(error),
                Err(other) => return Err(std::io::Error::new(ErrorKind::InvalidData, other)),
            }
        }
    }
}

impl FrameSink for WsLink {
    fn send(&self, frame: &[u8]) -> std::io::Result<()> {
        let text = std::str::from_utf8(frame)
            .map_err(|error| std::io::Error::new(ErrorKind::InvalidData, error))?;
        match self.socket.lock().unwrap().send(Message::text(text)) {
            Ok(()) => Ok(()),
            // A full socket buffer: tungstenite keeps the frame queued and
            // flushes it on the next read or write.
            Err(WsError::Io(error)) if error.kind() == ErrorKind::WouldBlock => Ok(()),
            Err(WsError::ConnectionClosed | WsError::AlreadyClosed) => {
                Err(std::io::Error::new(ErrorKind::BrokenPipe, "connection closed"))
            }
            Err(WsError::Io(error)) => Err(error),
            Err(other) => Err(std::io::Error::new(ErrorKind::Other, other)),
        }
    }

    fn close(&self) {
        let mut socket = self.socket.lock().unwrap();
        let _ = socket.close(None);
        let _ = socket.flush();
        let _ = socket.get_ref().shutdown(std::net::Shutdown::Both);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(text: &str) -> header::HeaderValue {
        header::HeaderValue::from_str(text).unwrap()
    }

    fn allowed() -> Vec<String> {
        vec!["https://app.example.com".into()]
    }

    #[test]
    fn a_listed_origin_is_admitted_case_and_slash_insensitively() {
        assert!(origin_verdict(Some(&value("https://app.example.com")), &allowed()).is_ok());
        assert!(origin_verdict(Some(&value("HTTPS://App.Example.com/")), &allowed()).is_ok());
    }

    #[test]
    fn an_unlisted_origin_is_refused_including_lookalikes() {
        for origin in [
            "https://evil.example",
            "http://app.example.com",
            "https://app.example.com.evil.example",
            "https://app.example.com:8443",
            "null",
        ] {
            assert!(
                origin_verdict(Some(&value(origin)), &allowed()).is_err(),
                "{origin} must be refused"
            );
        }
    }

    #[test]
    fn no_origin_header_is_a_non_browser_client_left_to_the_token() {
        assert!(origin_verdict(None, &allowed()).is_ok());
        assert!(origin_verdict(None, &[]).is_ok());
    }

    #[test]
    fn an_empty_allowlist_refuses_every_browser() {
        assert!(origin_verdict(Some(&value("https://app.example.com")), &[]).is_err());
    }

    #[test]
    fn origins_must_be_exact_never_wildcards_or_paths() {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        for bad in ["*", "https://*.example.com", "https://app.example.com/path", "example.com", "://x"] {
            assert!(
                RemoteConfig::new(addr, vec![bad.into()], false).is_err(),
                "{bad} must be rejected"
            );
        }
        assert!(RemoteConfig::new(addr, vec!["https://app.example.com/".into()], false).is_ok());
    }

    #[test]
    fn a_non_loopback_bind_needs_the_explicit_opt_in() {
        let public: SocketAddr = "0.0.0.0:4319".parse().unwrap();
        assert!(RemoteConfig::new(public, vec![], false).is_err());
        assert!(RemoteConfig::new(public, vec![], true).is_ok());
        let v6: SocketAddr = "[::1]:4319".parse().unwrap();
        assert!(RemoteConfig::new(v6, vec![], false).is_ok());
    }
}
