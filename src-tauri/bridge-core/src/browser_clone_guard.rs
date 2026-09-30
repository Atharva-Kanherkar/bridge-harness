//! Containment for a browser clone: nothing the clone runs may carry an
//! approved session off to a site that does not own it.
//!
//! Two layers, because neither is enough alone:
//!
//! - **Layer 1, the request checker.** Rides the browser-level CDP `Fetch`
//!   domain. Every request the clone makes — page, iframe, code under test,
//!   worker — is paused and inspected. A request that carries an approved
//!   session value (in plain, percent, base64, or hex form) to a host that does
//!   not own that value is failed. Verified against real Chrome: this catches
//!   page scripts, content scripts, image beacons, and worker `fetch`.
//! - **Layer 2, the egress proxy.** A local forward proxy the clone is launched
//!   against. It only opens connections to allowed hosts and refuses the rest.
//!   It exists because CDP `Fetch` never sees WebSocket connections at all
//!   (verified: a worker `WebSocket` slips past layer 1), and because a browser
//!   pointed at a proxy resolves no DNS of its own, so a disallowed host is
//!   never even looked up.
//!
//! The allowed set and the approved values live in one [`GuardState`] the two
//! layers share. Part 3 fills it from real approvals; this part defines it,
//! wires both layers onto a clone, and proves them against a real browser.

use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
#[cfg(test)]
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// How the guard treats one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Let it through.
    Allow,
    /// Stop it, with a plain-language reason (no session value in it).
    Block(String),
}

/// One thing the guard refused, for the audit surface. Never carries a session
/// value — only the host and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedRequest {
    pub host: String,
    pub reason: String,
    pub layer: &'static str,
}

/// The approved set for one clone: which hosts it may reach, and which session
/// values must never leave to a host that does not own them.
///
/// A "secret" is paired with the host that legitimately holds it, so a request
/// carrying it *to that host* is allowed and only carrying it *elsewhere* is a
/// leak.
#[derive(Default)]
pub struct GuardState {
    /// Hosts the clone may connect to at all (registrable domains; subdomains
    /// count). Empty means nothing is allowed out.
    allowed_hosts: HashSet<String>,
    /// (owning host, secret value) pairs. The value may travel to a host that
    /// the owner covers, never anywhere else.
    secrets: Vec<(String, String)>,
    blocked: Vec<BlockedRequest>,
}

impl GuardState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allow the clone to reach `host` and everything under it.
    pub fn allow_host(&mut self, host: &str) {
        self.allowed_hosts.insert(normalize_host(host));
    }

    /// Register a secret owned by `host`. It may be sent to `host` (or a
    /// subdomain of it), never to any other host.
    pub fn add_secret(&mut self, host: &str, value: &str) {
        if !value.is_empty() {
            self.secrets.push((normalize_host(host), value.to_owned()));
        }
    }

    pub fn blocked(&self) -> &[BlockedRequest] {
        &self.blocked
    }

    /// Is `host` (or a parent domain of it) on the allow list?
    fn host_allowed(&self, host: &str) -> bool {
        let host = normalize_host(host);
        self.allowed_hosts
            .iter()
            .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")))
    }

    /// Does `host` legitimately own `secret_owner` (same or subdomain)?
    fn host_covers(host: &str, owner: &str) -> bool {
        host == owner || host.ends_with(&format!(".{owner}"))
    }
}

/// Lowercase a host and drop a leading dot and any port.
fn normalize_host(host: &str) -> String {
    host.trim()
        .trim_start_matches('.')
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

// ------------------------------------------------------------ layer 1: values

/// Every textual form a secret could be smuggled in that is cheap to detect:
/// as-is, percent-encoded, standard and URL-safe base64 (with and without
/// padding), and lowercase and uppercase hex. Encryption defeats this; the
/// short lease and the host allow list are what bound that residual risk.
fn encodings(secret: &str) -> Vec<String> {
    let bytes = secret.as_bytes();
    let mut forms = vec![
        secret.to_owned(),
        percent_encode(secret),
        base64_standard(bytes, true),
        base64_standard(bytes, false),
        base64_url(bytes, true),
        base64_url(bytes, false),
        hex(bytes, false),
        hex(bytes, true),
    ];
    forms.retain(|form| form.len() >= 8);
    forms.sort();
    forms.dedup();
    forms
}

/// True if `haystack` contains `secret` in any of the forms above.
fn carries_secret(haystack: &str, secret: &str) -> bool {
    encodings(secret).iter().any(|form| haystack.contains(form.as_str()))
}

/// Adjudicate one intercepted request. `url` is the destination; `blob` is the
/// concatenation of everything a leak could ride in (URL, headers, body).
pub fn request_verdict(state: &GuardState, url: &str, blob: &str) -> Verdict {
    let host = host_of(url);
    for (owner, secret) in &state.secrets {
        if carries_secret(blob, secret) && !GuardState::host_covers(&host, owner) {
            return Verdict::Block(format!("carries a {owner} session value to {host}"));
        }
    }
    Verdict::Allow
}

fn host_of(url: &str) -> String {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let authority = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    normalize_host(authority)
}

// -------------------------------------------------------- layer 1: CDP wiring

/// The CDP commands to arm layer 1 on a clone: intercept every request at the
/// browser level. Sent once, after the browser is ready.
pub fn fetch_enable_command() -> (&'static str, Value) {
    ("Fetch.enable", json!({ "patterns": [{ "urlPattern": "*" }] }))
}

/// Given a `Fetch.requestPaused` event, return the CDP command that continues
/// or fails it, plus the block record when it is a block.
pub fn fetch_verdict(
    state: &Mutex<GuardState>,
    event_params: &Value,
) -> (&'static str, Value, Option<BlockedRequest>) {
    let request_id = event_params
        .get("requestId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let request = event_params.get("request").cloned().unwrap_or(Value::Null);
    let url = request.get("url").and_then(Value::as_str).unwrap_or_default().to_owned();
    let mut blob = String::new();
    blob.push_str(&url);
    if let Some(headers) = request.get("headers") {
        blob.push_str(&headers.to_string());
    }
    if let Some(post) = request.get("postData").and_then(Value::as_str) {
        blob.push_str(post);
    }

    let mut guard = state.lock().unwrap_or_else(|p| p.into_inner());
    match request_verdict(&guard, &url, &blob) {
        Verdict::Allow => (
            "Fetch.continueRequest",
            json!({ "requestId": request_id }),
            None,
        ),
        Verdict::Block(reason) => {
            let record = BlockedRequest {
                host: host_of(&url),
                reason: reason.clone(),
                layer: "request-checker",
            };
            guard.blocked.push(record.clone());
            (
                "Fetch.failRequest",
                json!({ "requestId": request_id, "errorReason": "BlockedByClient" }),
                Some(record),
            )
        }
    }
}

// -------------------------------------------------------- layer 2: egress proxy

/// A local forward proxy a clone is launched against. It opens connections only
/// to hosts the shared [`GuardState`] allows and refuses everything else,
/// WebSocket upgrades and unknown hosts included. Dropped to shut it down.
pub struct EgressProxy {
    port: u16,
    stop: Arc<AtomicBool>,
}

impl EgressProxy {
    /// Bind to loopback on an ephemeral port and start serving. The clone is
    /// pointed here with `--proxy-server`; see [`Self::launch_args`].
    pub fn start(state: Arc<Mutex<GuardState>>) -> std::io::Result<Arc<Self>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let proxy = Arc::new(Self {
            port,
            stop: Arc::clone(&stop),
        });
        let state_for_thread = state;
        thread::spawn(move || {
            for incoming in listener.incoming() {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(client) = incoming else { continue };
                let state = Arc::clone(&state_for_thread);
                thread::spawn(move || {
                    let _ = serve_client(client, &state);
                });
            }
        });
        Ok(proxy)
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// The launch flags that point a Chromium clone at this proxy. The
    /// `<-loopback>` removes loopback from the default bypass list, so even a
    /// request to 127.0.0.1 goes through the proxy and is checked.
    pub fn launch_args(&self) -> Vec<String> {
        vec![
            format!("--proxy-server=http://127.0.0.1:{}", self.port),
            "--proxy-bypass-list=<-loopback>".to_owned(),
        ]
    }
}

impl Drop for EgressProxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so it observes the stop flag.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

const PROXY_IO_TIMEOUT: Duration = Duration::from_secs(30);

fn serve_client(client: TcpStream, state: &Mutex<GuardState>) -> std::io::Result<()> {
    client.set_read_timeout(Some(PROXY_IO_TIMEOUT))?;
    let mut reader = BufReader::new(client.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");

    // Drain the rest of the request headers.
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        headers.push(line);
    }

    let host = if method.eq_ignore_ascii_case("CONNECT") {
        // CONNECT host:port — https and wss tunnels.
        normalize_host(target)
    } else {
        // Absolute-form request-target for plain http/ws: http://host/path.
        host_of(target)
    };

    let allowed = { state.lock().unwrap_or_else(|p| p.into_inner()).host_allowed(&host) };
    let mut client = client;
    if host.is_empty() || !allowed {
        record_block(state, &host);
        let _ = client.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n");
        return Ok(());
    }

    if method.eq_ignore_ascii_case("CONNECT") {
        tunnel(client, target)
    } else {
        forward_plain(client, target, method, &headers, reader)
    }
}

fn record_block(state: &Mutex<GuardState>, host: &str) {
    let mut guard = state.lock().unwrap_or_else(|p| p.into_inner());
    guard.blocked.push(BlockedRequest {
        host: if host.is_empty() { "<unknown>".into() } else { host.to_owned() },
        reason: "destination is not on the allow list".into(),
        layer: "egress-proxy",
    });
}

/// Open a raw tunnel to `authority` (host:port) and splice bytes both ways.
fn tunnel(mut client: TcpStream, authority: &str) -> std::io::Result<()> {
    let upstream = match TcpStream::connect(authority) {
        Ok(stream) => stream,
        Err(_) => {
            let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n");
            return Ok(());
        }
    };
    client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
    splice(client, upstream)
}

/// Forward one absolute-form plain HTTP request and stream the response back.
fn forward_plain(
    mut client: TcpStream,
    target: &str,
    method: &str,
    headers: &[String],
    body_reader: BufReader<TcpStream>,
) -> std::io::Result<()> {
    let host = host_of(target);
    let port = authority_port(target).unwrap_or(80);
    let path = origin_form(target);
    let upstream = match TcpStream::connect((host.as_str(), port)) {
        Ok(stream) => stream,
        Err(_) => {
            let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n");
            return Ok(());
        }
    };
    let mut upstream_write = upstream.try_clone()?;
    let mut head = format!("{method} {path} HTTP/1.1\r\n");
    for header in headers {
        head.push_str(header);
    }
    head.push_str("\r\n");
    upstream_write.write_all(head.as_bytes())?;
    // Pipe any request body the client already buffered, then splice both ways.
    let leftover = body_reader.buffer().to_vec();
    if !leftover.is_empty() {
        upstream_write.write_all(&leftover)?;
    }
    splice(client, upstream)
}

/// Copy bytes in both directions until either side closes.
fn splice(a: TcpStream, b: TcpStream) -> std::io::Result<()> {
    let (mut a_read, mut a_write) = (a.try_clone()?, a);
    let (mut b_read, mut b_write) = (b.try_clone()?, b);
    let up = thread::spawn(move || {
        let _ = std::io::copy(&mut a_read, &mut b_write);
        let _ = b_write.shutdown(std::net::Shutdown::Write);
    });
    let _ = std::io::copy(&mut b_read, &mut a_write);
    let _ = a_write.shutdown(std::net::Shutdown::Write);
    let _ = up.join();
    Ok(())
}

fn authority_port(url: &str) -> Option<u16> {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let authority = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    authority.rsplit(':').next().and_then(|value| value.parse().ok())
}

fn origin_form(url: &str) -> String {
    match url.split_once("://") {
        Some((_, rest)) => match rest.find('/') {
            Some(index) => rest[index..].to_owned(),
            None => "/".to_owned(),
        },
        None => url.to_owned(),
    }
}

// --------------------------------------------------------------- encoders

fn percent_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}

fn hex(bytes: &[u8], upper: bool) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        if upper {
            out.push_str(&format!("{byte:02X}"));
        } else {
            out.push_str(&format!("{byte:02x}"));
        }
    }
    out
}

const B64_STD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const B64_URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn base64_standard(bytes: &[u8], pad: bool) -> String {
    base64_with(bytes, B64_STD, pad)
}

fn base64_url(bytes: &[u8], pad: bool) -> String {
    base64_with(bytes, B64_URL, pad)
}

fn base64_with(bytes: &[u8], alphabet: &[u8; 64], pad: bool) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
        out.push(alphabet[((n >> 18) & 63) as usize] as char);
        out.push(alphabet[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(alphabet[((n >> 6) & 63) as usize] as char);
        } else if pad {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(alphabet[(n & 63) as usize] as char);
        } else if pad {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with(host: &str, secret: &str) -> Mutex<GuardState> {
        let mut state = GuardState::new();
        state.allow_host(host);
        state.add_secret(host, secret);
        Mutex::new(state)
    }

    #[test]
    fn a_secret_to_its_owner_is_allowed() {
        let state = state_with("granted.test", "S3cret");
        let v = request_verdict(
            &state.lock().unwrap(),
            "https://www.granted.test/api",
            "cookie: sid=S3cret",
        );
        assert_eq!(v, Verdict::Allow);
    }

    #[test]
    fn a_secret_to_another_host_is_blocked() {
        let state = state_with("granted.test", "S3cretValue123");
        let v = request_verdict(
            &state.lock().unwrap(),
            "https://evil.test/collect",
            "?c=S3cretValue123",
        );
        assert!(matches!(v, Verdict::Block(_)));
    }

    #[test]
    fn base64_and_hex_and_percent_forms_are_caught() {
        let secret = "S3cretValue123";
        let state = state_with("granted.test", secret);
        for blob in [
            base64_standard(secret.as_bytes(), true),
            base64_standard(secret.as_bytes(), false),
            base64_url(secret.as_bytes(), false),
            hex(secret.as_bytes(), false),
            hex(secret.as_bytes(), true),
            percent_encode(secret),
        ] {
            let v = request_verdict(
                &state.lock().unwrap(),
                "https://evil.test/x",
                &format!("payload={blob}"),
            );
            assert!(matches!(v, Verdict::Block(_)), "missed encoding: {blob}");
        }
    }

    #[test]
    fn an_unrelated_value_is_not_a_false_positive() {
        let state = state_with("granted.test", "S3cretValue123");
        let v = request_verdict(
            &state.lock().unwrap(),
            "https://evil.test/x",
            "just some ordinary telemetry payload",
        );
        assert_eq!(v, Verdict::Allow);
    }

    #[test]
    fn short_secrets_do_not_match_by_accident() {
        // An 8-char minimum keeps a short token from matching random text.
        assert!(encodings("abc").is_empty());
    }

    #[test]
    fn fetch_verdict_blocks_and_records() {
        let state = state_with("granted.test", "S3cretValue123");
        let event = json!({
            "requestId": "req-1",
            "request": { "url": "https://evil.test/c", "headers": { "x": "S3cretValue123" } }
        });
        let (command, params, record) = fetch_verdict(&state, &event);
        assert_eq!(command, "Fetch.failRequest");
        assert_eq!(params["errorReason"], "BlockedByClient");
        assert_eq!(record.unwrap().host, "evil.test");
        assert_eq!(state.lock().unwrap().blocked().len(), 1);
    }

    #[test]
    fn fetch_verdict_allows_the_owner() {
        let state = state_with("granted.test", "S3cretValue123");
        let event = json!({
            "requestId": "req-2",
            "request": { "url": "https://granted.test/api", "headers": { "cookie": "sid=S3cretValue123" } }
        });
        let (command, _params, record) = fetch_verdict(&state, &event);
        assert_eq!(command, "Fetch.continueRequest");
        assert!(record.is_none());
    }

    #[test]
    fn host_matching_covers_subdomains_only() {
        let mut state = GuardState::new();
        state.allow_host("granted.test");
        assert!(state.host_allowed("granted.test"));
        assert!(state.host_allowed("www.granted.test"));
        assert!(!state.host_allowed("granted.test.evil.com"));
        assert!(!state.host_allowed("notgranted.test"));
    }

    #[test]
    fn host_of_parses_authority() {
        assert_eq!(host_of("https://www.granted.test:443/a?b#c"), "www.granted.test");
        assert_eq!(host_of("http://user@granted.test/x"), "granted.test");
    }

    #[test]
    fn proxy_denies_a_disallowed_connect() {
        let state = Arc::new(Mutex::new(GuardState::new()));
        state.lock().unwrap().allow_host("granted.test");
        let proxy = EgressProxy::start(Arc::clone(&state)).unwrap();
        let mut client = TcpStream::connect(("127.0.0.1", proxy.port())).unwrap();
        client
            .write_all(b"CONNECT evil.test:443 HTTP/1.1\r\nHost: evil.test:443\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 403"), "{response:?}");
        assert_eq!(state.lock().unwrap().blocked()[0].host, "evil.test");
        assert_eq!(state.lock().unwrap().blocked()[0].layer, "egress-proxy");
    }

    #[test]
    fn proxy_tunnels_an_allowed_connect() {
        // A stand-in origin the proxy is allowed to reach.
        let origin = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin_port = origin.local_addr().unwrap().port();
        thread::spawn(move || {
            if let Ok((mut socket, _)) = origin.accept() {
                let mut buf = [0u8; 16];
                let n = socket.read(&mut buf).unwrap_or(0);
                let _ = socket.write_all(&buf[..n]);
            }
        });
        let state = Arc::new(Mutex::new(GuardState::new()));
        state.lock().unwrap().allow_host("127.0.0.1");
        let proxy = EgressProxy::start(Arc::clone(&state)).unwrap();
        let mut client = TcpStream::connect(("127.0.0.1", proxy.port())).unwrap();
        client
            .write_all(format!("CONNECT 127.0.0.1:{origin_port} HTTP/1.1\r\n\r\n").as_bytes())
            .unwrap();
        let mut reader = BufReader::new(client.try_clone().unwrap());
        let mut status = String::new();
        reader.read_line(&mut status).unwrap();
        assert!(status.starts_with("HTTP/1.1 200"), "{status:?}");
        // consume the blank line after the status
        let mut blank = String::new();
        reader.read_line(&mut blank).unwrap();
        client.write_all(b"ping").unwrap();
        let mut echoed = [0u8; 4];
        reader.read_exact(&mut echoed).unwrap();
        assert_eq!(&echoed, b"ping");
    }

    #[test]
    fn proxy_launch_args_keep_loopback_in_scope() {
        let state = Arc::new(Mutex::new(GuardState::new()));
        let proxy = EgressProxy::start(state).unwrap();
        let args = proxy.launch_args();
        assert!(args.iter().any(|a| a.contains(&format!("127.0.0.1:{}", proxy.port()))));
        assert!(args.iter().any(|a| a == "--proxy-bypass-list=<-loopback>"));
    }
}
