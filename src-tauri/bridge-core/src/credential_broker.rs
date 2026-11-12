//! API keys an agent can use but never read.
//!
//! A key reaches Bridge two ways: pasted into a chat, where
//! `secret_interception` swaps it for a `[secret:sec_…]` placeholder and the
//! value stays in memory for that chat, or saved in Settings, where the value
//! goes to the Keychain (`api_keys`) and the placeholder is the key's name,
//! `[secret:OPENAI_API_KEY]`. The agent only ever holds the placeholder.
//!
//! To use a key the agent calls `/credential-proxy/<session>/<host>/<path>` on
//! Bridge's loopback port. Bridge forwards the call to `https://<host>/<path>`
//! and swaps each placeholder in a request header for the real key, but only
//! when that key is allowed on that host, and scrubs the key from the response
//! before the agent sees it. Headers only: a request body is assembled from
//! whatever the agent is working with, so it is the easiest way to get a key
//! echoed back.
//!
//! The call also has to carry the proxy pass, a per-launch value Bridge puts
//! in every harness's environment as `$BRIDGE_PROXY_PASS`. The agent writes
//! the variable, never the value, so a command saved to history holds nothing
//! a reader could replay.

use crate::{agent::NormalizedEvent, api_keys, secret_interception::CapturedSecret, BridgeError};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use reqwest::{blocking::Client, Method};
use serde_json::Value;
use std::{
    collections::HashMap,
    io::Read,
    net::SocketAddr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, LazyLock, Mutex,
    },
    time::Duration,
};
use uuid::Uuid;
use zeroize::Zeroizing;

pub const PROXY_PREFIX: &str = "/credential-proxy/";
pub const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;
/// Header that carries the proxy pass.
pub const PROXY_AUTH_HEADER: &str = "x-bridge-proxy-auth";
/// The environment variable every harness process gets the pass in. Its name
/// avoids KEY, SECRET and TOKEN so Codex's default shell filter keeps it.
pub const PROXY_PASS_ENV: &str = "BRIDGE_PROXY_PASS";
/// Where the proxy listens when the port is free. A taken port falls back to
/// an ephemeral one, and the instructions name whichever was bound.
pub const DEFAULT_PROXY_ADDR: &str = "127.0.0.1:4317";
const PLACEHOLDER_OPEN: &str = "[secret:";
/// Response header naming the placeholders whose keys Bridge removed from it.
pub const REDACTED_HEADER: &str = "x-bridge-redacted";
/// The route pasted OpenAI keys used before keys had sites:
/// `/credential-proxy/<session>/sec_…/v1/…`. Still served, for OpenAI only.
const LEGACY_OPENAI_HOST: &str = "api.openai.com";

/// 256 bits of per-launch entropy. Never persisted, never in a prompt.
static PROXY_PASS: LazyLock<String> =
    LazyLock::new(|| format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple()));

pub fn proxy_pass() -> &'static str {
    PROXY_PASS.as_str()
}

/// Give a harness process the pass. Its shell commands inherit it.
pub fn apply_proxy_pass(command: &mut std::process::Command) {
    command.env(PROXY_PASS_ENV, proxy_pass());
}

fn pass_matches(provided: &str) -> bool {
    let expected = proxy_pass().as_bytes();
    let provided = provided.as_bytes();
    if expected.len() != provided.len() {
        return false;
    }
    expected
        .iter()
        .zip(provided)
        .fold(0u8, |diff, (left, right)| diff | (left ^ right))
        == 0
}

/// The same event with the pass replaced by its variable name, or `None` when
/// the pass is not in it. History and exports go through this, so an agent
/// that printed its environment still leaves nothing replayable behind.
pub fn scrub_event(event: &NormalizedEvent) -> Option<NormalizedEvent> {
    let pass = proxy_pass();
    let in_text = |text: &Option<String>| text.as_deref().is_some_and(|text| text.contains(pass));
    if !in_text(&event.title) && !in_text(&event.text) && !value_contains(&event.data, pass) {
        return None;
    }
    let replacement = format!("${PROXY_PASS_ENV}");
    let mut clean = event.clone();
    for text in [&mut clean.title, &mut clean.text].into_iter().flatten() {
        *text = text.replace(pass, &replacement);
    }
    replace_in_value(&mut clean.data, pass, &replacement);
    Some(clean)
}

fn value_contains(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(text) => text.contains(needle),
        Value::Array(items) => items.iter().any(|item| value_contains(item, needle)),
        Value::Object(map) => map.values().any(|item| value_contains(item, needle)),
        _ => false,
    }
}

fn replace_in_value(value: &mut Value, needle: &str, replacement: &str) {
    match value {
        Value::String(text) if text.contains(needle) => *text = text.replace(needle, replacement),
        Value::Array(items) => items.iter_mut().for_each(|item| replace_in_value(item, needle, replacement)),
        Value::Object(map) => map.values_mut().for_each(|item| replace_in_value(item, needle, replacement)),
        _ => {}
    }
}

/// The sites a pasted key may be sent to before the person changes anything.
/// A kind with no well-known API starts with none.
pub fn default_hosts(kind: &str) -> &'static [&'static str] {
    match kind {
        "openai" => &["api.openai.com"],
        "anthropic" => &["api.anthropic.com"],
        "github" => &["api.github.com", "uploads.github.com"],
        "slack" => &["slack.com"],
        "notion" => &["api.notion.com"],
        _ => &[],
    }
}

/// How the agent's instructions name a kind of key.
pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "openai" => "OpenAI key",
        "anthropic" => "Anthropic key",
        "github" => "GitHub token",
        "slack" => "Slack token",
        "notion" => "Notion key",
        "bearer" => "bearer token",
        "jwt" => "JWT",
        "credential_assignment" => "credential",
        _ => "API key",
    }
}

/// Kinds a header can carry. A private key or a lone AWS access key id is
/// redacted from the chat but cannot be used this way.
fn usable_kind(kind: &str) -> bool {
    !matches!(kind, "private_key" | "aws_access_key")
}

fn host_allowed(allowed: &[String], host: &str) -> bool {
    allowed.iter().any(|pattern| match pattern.strip_prefix("*.") {
        Some(parent) => host.len() > parent.len() + 1
            && host.ends_with(parent)
            && host.as_bytes()[host.len() - parent.len() - 1] == b'.',
        None => pattern == host,
    })
}

/// Every `[secret:…]` reference in a text, in order.
fn placeholder_references(text: &str) -> Vec<&str> {
    text.split(PLACEHOLDER_OPEN)
        .skip(1)
        .filter_map(|tail| tail.split_once(']').map(|(reference, _)| reference))
        .filter(|reference| {
            !reference.is_empty()
                && reference.len() <= 80
                && reference.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
        .collect()
}

struct ChatKey {
    session_id: String,
    kind: String,
    hosts: Vec<String>,
    value: Zeroizing<String>,
    /// Set once the person saved this key for every chat; from then on the
    /// saved key's sites apply.
    saved_as: Option<String>,
}

struct SavedKey {
    metadata: api_keys::Metadata,
    /// Read from the Keychain on first use, then kept for this launch.
    value: Option<Zeroizing<String>>,
}

/// A key as the settings page sees it. Never carries the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySummary {
    pub reference: String,
    pub kind: String,
    pub hosts: Vec<String>,
    pub session_id: Option<String>,
    pub saved_as: Option<String>,
}

pub struct ProxyRequest {
    /// The route after [`PROXY_PREFIX`]: `<session>/<host>/<path and query>`.
    pub route: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug)]
pub struct ProxyResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Why Bridge would not forward a call, worded for the agent that made it.
#[derive(Debug, PartialEq, Eq)]
pub struct ProxyRefusal {
    pub status: u16,
    pub message: String,
}

impl ProxyRefusal {
    fn new(status: u16, message: impl Into<String>) -> Self {
        Self { status, message: message.into() }
    }
}

/// A call that passed every check, ready to send.
struct Prepared {
    url: reqwest::Url,
    host: String,
    method: Method,
    headers: Vec<(String, Zeroizing<String>)>,
    body: Vec<u8>,
    /// What to turn back into a placeholder in the response.
    used: Vec<Scrub>,
}

/// A value the response must not carry, and what replaces it.
struct Scrub {
    /// What the agent wrote: the placeholder, or for Basic credentials the
    /// base64 token with placeholders inside.
    shown: String,
    value: Zeroizing<String>,
    /// The placeholders the `x-bridge-redacted` header names.
    label: String,
}

/// The base64 token and its decoded `user:password` text of a Basic
/// `Authorization` header that carries a placeholder.
fn basic_credentials<'a>(lower_name: &str, value: &'a str) -> Option<(&'a str, Zeroizing<String>)> {
    if lower_name != "authorization" {
        return None;
    }
    let (scheme, token) = value.trim().split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("basic") {
        return None;
    }
    let token = token.trim();
    let decoded = Zeroizing::new(String::from_utf8(BASE64.decode(token).ok()?).ok()?);
    decoded.contains(PLACEHOLDER_OPEN).then_some((token, decoded))
}

/// A placeholder resolved for one call.
struct ResolvedKey {
    label: String,
    hosts: Vec<String>,
    value: Zeroizing<String>,
}

/// Headers that describe one connection, not the request. Bridge sets its own.
const DROPPED_REQUEST_HEADERS: &[&str] = &[
    "host", "connection", "keep-alive", "proxy-connection", "proxy-authorization", "te",
    "trailer", "transfer-encoding", "upgrade", "content-length", "accept-encoding",
    PROXY_AUTH_HEADER,
];
const DROPPED_RESPONSE_HEADERS: &[&str] = &[
    "connection", "keep-alive", "transfer-encoding", "content-length", "content-encoding",
    "set-cookie", "upgrade", "trailer",
];

pub struct CredentialBroker {
    chat_keys: Mutex<HashMap<String, ChatKey>>,
    saved: Mutex<HashMap<String, SavedKey>>,
    /// Bumped whenever the saved keys change, so a running chat is told about
    /// a key added after it started.
    generation: AtomicU64,
    briefed: Mutex<HashMap<String, u64>>,
    endpoint: Mutex<Option<SocketAddr>>,
    client: Client,
    scheme: &'static str,
}

impl CredentialBroker {
    pub fn new() -> Result<Self, BridgeError> {
        Self::with_scheme("https")
    }

    fn with_scheme(scheme: &'static str) -> Result<Self, BridgeError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            // A redirect would carry the key's header to a host nobody allowed.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| {
                BridgeError::Invalid(format!("Could not initialize credential proxy: {error}"))
            })?;
        Ok(Self {
            chat_keys: Mutex::new(HashMap::new()),
            saved: Mutex::new(HashMap::new()),
            generation: AtomicU64::new(0),
            briefed: Mutex::new(HashMap::new()),
            endpoint: Mutex::new(None),
            client,
            scheme,
        })
    }

    /// Load the saved keys' names and sites. Values stay in the Keychain until
    /// a call needs one.
    pub fn load_saved(&self, db: &rusqlite::Connection) -> Result<(), BridgeError> {
        let mut saved = self.saved.lock().unwrap();
        saved.clear();
        for (name, metadata) in api_keys::list(db)? {
            saved.insert(name, SavedKey { metadata, value: None });
        }
        self.generation.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    /// Record a saved key's sites, and its value when the caller just wrote it.
    pub fn put_saved(&self, name: &str, metadata: api_keys::Metadata, value: Option<Zeroizing<String>>) {
        let mut saved = self.saved.lock().unwrap();
        let value = value.or_else(|| saved.remove(name).and_then(|existing| existing.value));
        saved.insert(name.to_owned(), SavedKey { metadata, value });
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    pub fn remove_saved(&self, name: &str) {
        self.saved.lock().unwrap().remove(name);
        for key in self.chat_keys.lock().unwrap().values_mut() {
            if key.saved_as.as_deref() == Some(name) {
                key.saved_as = None;
            }
        }
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Whether a saved key's value can be read. The Keychain ties an item to
    /// the build that wrote it unless the build is signed with a team id, so a
    /// local rebuild of Bridge cannot read keys an earlier build saved. A key
    /// read here stays in memory for the rest of the launch.
    pub fn saved_readable(&self, name: &str) -> bool {
        if self.saved.lock().unwrap().get(name).is_some_and(|key| key.value.is_some()) {
            return true;
        }
        let Some(value) = api_keys::read_value(name).ok().flatten() else { return false };
        if let Some(key) = self.saved.lock().unwrap().get_mut(name) {
            key.value = Some(value);
        }
        true
    }

    pub fn saved_metadata(&self, name: &str) -> Option<api_keys::Metadata> {
        self.saved.lock().unwrap().get(name).map(|key| key.metadata.clone())
    }

    /// Keep a chat's pasted keys for that chat. Each starts on its kind's
    /// well-known API, if it has one.
    pub fn register(&self, session_id: &str, captures: Vec<CapturedSecret>) {
        let mut keys = self.chat_keys.lock().unwrap();
        for capture in captures {
            if !usable_kind(capture.detector) {
                continue;
            }
            keys.insert(
                capture.reference,
                ChatKey {
                    session_id: session_id.to_owned(),
                    kind: capture.detector.to_owned(),
                    hosts: default_hosts(capture.detector).iter().map(|host| (*host).to_owned()).collect(),
                    value: capture.value,
                    saved_as: None,
                },
            );
        }
    }

    pub fn clear_session(&self, session_id: &str) {
        self.chat_keys
            .lock()
            .unwrap()
            .retain(|_, key| key.session_id != session_id);
        self.briefed.lock().unwrap().remove(session_id);
    }

    pub fn forget_chat_key(&self, reference: &str) -> bool {
        self.chat_keys.lock().unwrap().remove(reference).is_some()
    }

    /// A pasted key's kind and value, for saving it under a name.
    pub fn chat_key_for_saving(&self, reference: &str) -> Option<(String, Zeroizing<String>)> {
        self.chat_keys
            .lock()
            .unwrap()
            .get(reference)
            .map(|key| (key.kind.clone(), Zeroizing::new(key.value.to_string())))
    }

    pub fn mark_saved_as(&self, reference: &str, name: &str) {
        if let Some(key) = self.chat_keys.lock().unwrap().get_mut(reference) {
            key.saved_as = Some(name.to_owned());
        }
    }

    pub fn summaries(&self) -> (Vec<KeySummary>, Vec<KeySummary>) {
        let mut saved: Vec<KeySummary> = self
            .saved
            .lock()
            .unwrap()
            .iter()
            .map(|(name, key)| KeySummary {
                reference: name.clone(),
                kind: key.metadata.kind.clone(),
                hosts: key.metadata.hosts.clone(),
                session_id: None,
                saved_as: None,
            })
            .collect();
        saved.sort_by(|left, right| left.reference.cmp(&right.reference));
        let mut chat: Vec<KeySummary> = self
            .chat_keys
            .lock()
            .unwrap()
            .iter()
            .map(|(reference, key)| KeySummary {
                reference: reference.clone(),
                kind: key.kind.clone(),
                hosts: key.hosts.clone(),
                session_id: Some(key.session_id.clone()),
                saved_as: key.saved_as.clone(),
            })
            .collect();
        chat.sort_by(|left, right| left.session_id.cmp(&right.session_id).then(left.reference.cmp(&right.reference)));
        (saved, chat)
    }

    pub fn set_endpoint(&self, address: SocketAddr) {
        *self.endpoint.lock().unwrap() = Some(address);
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    pub fn endpoint(&self) -> Option<SocketAddr> {
        *self.endpoint.lock().unwrap()
    }

    /// The calling contract every session is launched with. Marks the session
    /// as told about the current saved keys.
    pub fn instructions(&self, session_id: &str) -> String {
        let generation = self.generation.load(Ordering::SeqCst);
        self.briefed.lock().unwrap().insert(session_id.to_owned(), generation);
        self.contract(session_id)
    }

    fn contract(&self, session_id: &str) -> String {
        let protection = "Bridge keeps API keys for the user. You never see a key's value, only a placeholder such as [secret:OPENAI_API_KEY] or [secret:sec_...]. A placeholder means Bridge already protected the key, so do not tell the user they exposed it or must rotate it because of the placeholder, and never ask them to paste a key's value.";
        let Some(endpoint) = self.endpoint() else {
            return format!("{protection} Bridge's key proxy is not running in this session, so keys cannot be used to call APIs.");
        };
        let saved = self.saved.lock().unwrap();
        let mut names: Vec<(&String, &SavedKey)> = saved.iter().collect();
        names.sort_by(|left, right| left.0.cmp(right.0));
        let listing = if names.is_empty() {
            "The user has no saved keys yet. Keys they paste into this chat become placeholders you can use the same way.".to_owned()
        } else {
            let entries: Vec<String> = names
                .iter()
                .map(|(name, key)| {
                    let sites = if key.metadata.hosts.is_empty() { "no sites allowed yet".to_owned() } else { key.metadata.hosts.join(", ") };
                    format!("[secret:{name}] ({sites})")
                })
                .collect();
            format!("Saved keys: {}.", entries.join("; "))
        };
        format!(
            "{protection} To call an API with a key, send the request through Bridge with your shell and put the placeholder in the request header where the API expects the key, for example:\ncurl -sS \"http://{endpoint}{PROXY_PREFIX}{session_id}/<host>/<path>\" -H \"{PROXY_AUTH_HEADER}: ${PROXY_PASS_ENV}\" -H \"Authorization: Bearer [secret:NAME]\"\nBridge sends it to https://<host>/<path> and replaces the placeholder with the real key, in headers only and only on sites the user allowed for that key. For Basic auth, curl -u \"user:[secret:NAME]\" works the same way. Any method works; responses are not streamed. Bridge also turns the key back into its placeholder in every response, so a site that echoes your request shows the placeholder even though it received the real key; such a response carries an x-bridge-redacted header. Keep the quotes so the shell leaves the brackets alone, and always write ${PROXY_PASS_ENV} as a variable: never print or copy its value. If Bridge says a key is not allowed on a site, ask the user to allow that site for the key in Bridge under Settings, Permissions, API keys. {listing}"
        )
    }

    /// Trusted per-turn context: the calling contract again, when the turn
    /// names a key this chat can use or the saved keys changed since the chat
    /// was last told. Never carries a value.
    pub fn turn_context(&self, session_id: &str, text: &str) -> Option<String> {
        let mentioned: Vec<String> = {
            let chat_keys = self.chat_keys.lock().unwrap();
            let saved = self.saved.lock().unwrap();
            let mut seen = Vec::new();
            for reference in placeholder_references(text) {
                let description = match chat_keys.get(reference) {
                    Some(key) if key.session_id == session_id => {
                        let hosts = key
                            .saved_as
                            .as_ref()
                            .and_then(|name| saved.get(name))
                            .map(|saved| saved.metadata.hosts.clone())
                            .unwrap_or_else(|| key.hosts.clone());
                        Some((kind_label(&key.kind), hosts))
                    }
                    Some(_) => None,
                    None => saved
                        .get(reference)
                        .map(|key| (kind_label(&key.metadata.kind), key.metadata.hosts.clone())),
                };
                if let Some((label, hosts)) = description {
                    let sites = if hosts.is_empty() {
                        "not allowed on any site yet, so ask the user which site to allow".to_owned()
                    } else {
                        format!("allowed on {}", hosts.join(", "))
                    };
                    let line = format!("[secret:{reference}] is a {label} the user gave you, {sites}.");
                    if !seen.contains(&line) {
                        seen.push(line);
                    }
                }
            }
            seen
        };
        let generation = self.generation.load(Ordering::SeqCst);
        let stale = self
            .briefed
            .lock()
            .unwrap()
            .get(session_id)
            .is_some_and(|briefed| *briefed != generation);
        if mentioned.is_empty() && !stale {
            return None;
        }
        let mut context = self.instructions(session_id);
        if !mentioned.is_empty() {
            context.push_str("\n\n");
            context.push_str(&mentioned.join("\n"));
            context.push_str(" Do not refuse merely because a placeholder is present.");
        }
        Some(context)
    }

    fn resolve(&self, session_id: &str, reference: &str) -> Result<Option<ResolvedKey>, ProxyRefusal> {
        let saved_name = {
            let chat_keys = self.chat_keys.lock().unwrap();
            match chat_keys.get(reference) {
                Some(key) if key.session_id != session_id => return Ok(None),
                Some(key) => match &key.saved_as {
                    Some(name) if self.saved.lock().unwrap().contains_key(name) => name.clone(),
                    _ => {
                        return Ok(Some(ResolvedKey {
                            label: format!("The {} [secret:{reference}]", kind_label(&key.kind)),
                            hosts: key.hosts.clone(),
                            value: Zeroizing::new(key.value.to_string()),
                        }))
                    }
                },
                None => {
                    let saved = self.saved.lock().unwrap();
                    if saved.contains_key(reference) {
                        reference.to_owned()
                    } else if let Some((name, _)) = saved
                        .iter()
                        .find(|(_, key)| key.metadata.aliases.iter().any(|alias| alias == reference))
                    {
                        name.clone()
                    } else {
                        return Ok(None);
                    }
                }
            }
        };
        let (hosts, cached) = {
            let saved = self.saved.lock().unwrap();
            let Some(key) = saved.get(&saved_name) else { return Ok(None) };
            (key.metadata.hosts.clone(), key.value.as_ref().map(|value| Zeroizing::new(value.to_string())))
        };
        let value = match cached {
            Some(value) => value,
            None => {
                // Read outside every lock: a Keychain read can take a while.
                let value = api_keys::read_value(&saved_name)
                    .ok()
                    .flatten()
                    .ok_or_else(|| ProxyRefusal::new(500, format!("Bridge could not read {saved_name} from the Keychain. Ask the user to paste it again in Bridge under Settings, Permissions, API keys.")))?;
                if let Some(key) = self.saved.lock().unwrap().get_mut(&saved_name) {
                    key.value = Some(Zeroizing::new(value.to_string()));
                }
                value
            }
        };
        Ok(Some(ResolvedKey { label: format!("The key {saved_name}"), hosts, value }))
    }

    pub fn proxy(&self, request: ProxyRequest) -> Result<ProxyResponse, ProxyRefusal> {
        let prepared = self.prepare(request)?;
        self.send(prepared)
    }

    /// Everything short of the network: the pass, the route, the method and
    /// every placeholder resolved against the host.
    fn prepare(&self, request: ProxyRequest) -> Result<Prepared, ProxyRefusal> {
        let provided = request
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(PROXY_AUTH_HEADER))
            .map(|(_, value)| value.as_str())
            .unwrap_or_default();
        if !pass_matches(provided) {
            return Err(ProxyRefusal::new(
                403,
                format!("Send the header {PROXY_AUTH_HEADER}: ${PROXY_PASS_ENV} with every call through Bridge"),
            ));
        }
        let (session_id, rest) = request
            .route
            .split_once('/')
            .filter(|(session, _)| !session.is_empty())
            .ok_or_else(|| ProxyRefusal::new(400, "The route is /credential-proxy/<session>/<host>/<path>"))?;
        let (first, remainder) = rest.split_once('/').unwrap_or((rest, ""));
        let path_and_query = format!("/{remainder}");
        if path_and_query.chars().any(|c| c.is_control() || c == ' ') {
            return Err(ProxyRefusal::new(400, "Percent-encode spaces and control characters in the path"));
        }
        let mut headers = request.headers;
        let host = if first.starts_with("sec_") {
            // The route before keys had sites: the reference names the key and
            // the upstream is OpenAI's.
            if !path_and_query.starts_with("/v1/") {
                return Err(ProxyRefusal::new(400, "This route only reaches OpenAI's /v1/ API"));
            }
            if headers.iter().any(|(name, _)| name.eq_ignore_ascii_case("authorization")) {
                return Err(ProxyRefusal::new(400, "Bridge adds the authorization for this route"));
            }
            headers.push(("Authorization".into(), format!("Bearer {PLACEHOLDER_OPEN}{first}]")));
            LEGACY_OPENAI_HOST.to_owned()
        } else {
            let host = api_keys::normalize_host(first)
                .map_err(|_| ProxyRefusal::new(400, "Name the site right after the session, like /credential-proxy/<session>/api.github.com/user"))?;
            if host.starts_with("*.") {
                return Err(ProxyRefusal::new(400, "Name one site, not a wildcard"));
            }
            host
        };
        let method = match request.method.as_str() {
            "GET" => Method::GET,
            "POST" => Method::POST,
            "PUT" => Method::PUT,
            "PATCH" => Method::PATCH,
            "DELETE" => Method::DELETE,
            _ => return Err(ProxyRefusal::new(405, "Use GET, POST, PUT, PATCH or DELETE")),
        };
        if request.body.len() > MAX_BODY_BYTES {
            return Err(ProxyRefusal::new(413, "The request body is too large"));
        }
        let url = reqwest::Url::parse(&format!("{}://{host}{path_and_query}", self.scheme))
            .map_err(|_| ProxyRefusal::new(400, "That is not a URL Bridge can call"))?;
        let parsed_host = match url.port() {
            Some(port) => format!("{}:{port}", url.host_str().unwrap_or_default()),
            None => url.host_str().unwrap_or_default().to_owned(),
        };
        if parsed_host != host {
            return Err(ProxyRefusal::new(400, "That is not a URL Bridge can call"));
        }

        // Swap placeholders for values, header by header. Every key named must
        // be allowed on this host, or nothing is sent.
        let mut used: Vec<Scrub> = Vec::new();
        let mut outbound: Vec<(String, Zeroizing<String>)> = Vec::new();
        for (name, value) in headers {
            let lower = name.to_ascii_lowercase();
            if DROPPED_REQUEST_HEADERS.contains(&lower.as_str()) {
                continue;
            }
            let filled = match basic_credentials(&lower, &value) {
                // `curl -u "user:[secret:KEY]"` sends the placeholder base64
                // encoded, so it is decoded, filled and encoded again.
                Some((token, decoded)) => {
                    let first = used.len();
                    let filled = self.fill(session_id, &host, &decoded, &mut used)?;
                    let encoded = Zeroizing::new(BASE64.encode(filled.as_bytes()));
                    let label = used[first..].iter().map(|scrub| scrub.label.as_str()).collect::<Vec<_>>().join(", ");
                    if !label.is_empty() {
                        used.push(Scrub { shown: token.to_owned(), value: Zeroizing::new(encoded.to_string()), label });
                    }
                    Zeroizing::new(format!("Basic {}", encoded.as_str()))
                }
                None => self.fill(session_id, &host, &value, &mut used)?,
            };
            outbound.push((name, filled));
        }
        if used.is_empty() {
            return Err(ProxyRefusal::new(400, "Put a key placeholder such as [secret:NAME] in a request header. Bridge only forwards calls that use a key."));
        }
        Ok(Prepared { url, host, method, headers: outbound, body: request.body, used })
    }

    /// Replace every placeholder in one header value with its key, refusing
    /// a key this chat cannot use or that is not allowed on `host`.
    fn fill(&self, session_id: &str, host: &str, text: &str, used: &mut Vec<Scrub>) -> Result<Zeroizing<String>, ProxyRefusal> {
        let mut filled = Zeroizing::new(text.to_owned());
        for reference in placeholder_references(text) {
            let key = self
                .resolve(session_id, reference)?
                .ok_or_else(|| ProxyRefusal::new(403, format!("Bridge has no key [secret:{reference}] for this chat. Use a placeholder from the conversation or a saved key.")))?;
            if !host_allowed(&key.hosts, host) {
                return Err(ProxyRefusal::new(403, format!("{} is not allowed on {host}. Ask the user to allow {host} for it in Bridge under Settings, Permissions, API keys.", key.label)));
            }
            let placeholder = format!("{PLACEHOLDER_OPEN}{reference}]");
            *filled = filled.replace(&placeholder, key.value.as_str());
            if !used.iter().any(|scrub| scrub.shown == placeholder) {
                used.push(Scrub { shown: placeholder.clone(), value: key.value, label: placeholder });
            }
        }
        Ok(filled)
    }

    fn send(&self, prepared: Prepared) -> Result<ProxyResponse, ProxyRefusal> {
        let Prepared { url, host, method, headers: outbound, body, used } = prepared;
        let mut upstream = self.client.request(method, url).body(body);
        for (name, value) in &outbound {
            upstream = upstream.header(name.as_str(), value.as_str());
        }
        drop(outbound);
        let mut response = upstream.send().map_err(|error| {
            ProxyRefusal::new(502, scrub_text(&format!("The call to {host} failed: {error}"), &used))
        })?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter(|(name, _)| !DROPPED_RESPONSE_HEADERS.contains(&name.as_str()))
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_owned(), scrub_text(value, &used)))
            })
            .collect();
        let mut body = Vec::new();
        response
            .by_ref()
            .take((MAX_BODY_BYTES + 1) as u64)
            .read_to_end(&mut body)
            .map_err(|error| ProxyRefusal::new(502, format!("Reading the response from {host} failed: {error}")))?;
        if body.len() > MAX_BODY_BYTES {
            return Err(ProxyRefusal::new(502, "The response is too large"));
        }
        let mut headers: Vec<(String, String)> = headers;
        let scrubbed = scrub_bytes(body.clone(), &used);
        // Say so when a key came back: an echo that shows the placeholder
        // otherwise reads as a call that never used the key.
        let mut redacted: Vec<&str> = Vec::new();
        for scrub in &used {
            let came_back = contains_bytes(&body, scrub.value.as_bytes())
                || headers.iter().any(|(_, header)| header.contains(scrub.shown.as_str()));
            if came_back && !redacted.contains(&scrub.label.as_str()) {
                redacted.push(scrub.label.as_str());
            }
        }
        if !redacted.is_empty() {
            headers.push((REDACTED_HEADER.into(), redacted.join(", ")));
        }
        Ok(ProxyResponse { status, headers, body: scrubbed })
    }
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|window| window == needle)
}

/// Replace every key value with its placeholder. A site that echoes the
/// request back, or puts the key in an error, must not hand it to the agent.
fn scrub_text(text: &str, used: &[Scrub]) -> String {
    used.iter().fold(text.to_owned(), |text, scrub| {
        if scrub.value.is_empty() { text } else { text.replace(scrub.value.as_str(), &scrub.shown) }
    })
}

fn scrub_bytes(body: Vec<u8>, used: &[Scrub]) -> Vec<u8> {
    used.iter().fold(body, |body, scrub| {
        let (placeholder, needle) = (&scrub.shown, scrub.value.as_bytes());
        if needle.is_empty() || body.len() < needle.len() {
            return body;
        }
        let mut out = Vec::with_capacity(body.len());
        let mut index = 0;
        while index < body.len() {
            if body[index..].starts_with(needle) {
                out.extend_from_slice(placeholder.as_bytes());
                index += needle.len();
            } else {
                out.push(body[index]);
                index += 1;
            }
        }
        out
    })
}

/// Bind the proxy's port: the default when it is free, an ephemeral one when
/// it is not. The broker records what was bound, so the instructions name a
/// port that answers.
pub fn bind(broker: &CredentialBroker) -> Option<tiny_http::Server> {
    let server = tiny_http::Server::http(DEFAULT_PROXY_ADDR)
        .or_else(|_| tiny_http::Server::http("127.0.0.1:0"))
        .ok()?;
    let address = server.server_addr().to_ip()?;
    broker.set_endpoint(address);
    Some(server)
}

/// Serve the proxy alone. The daemon's host, which has no `/health` here.
pub fn start(broker: Arc<CredentialBroker>) -> Option<SocketAddr> {
    let server = bind(&broker)?;
    let address = broker.endpoint();
    std::thread::Builder::new()
        .name("bridge-credential-proxy".into())
        .spawn(move || {
            for request in server.incoming_requests() {
                handle(request, &broker);
            }
        })
        .ok()?;
    address
}

/// Answer one request: a proxy call, or 404. A host with other routes answers
/// those first.
pub fn handle(request: tiny_http::Request, broker: &Arc<CredentialBroker>) {
    let Some(route) = request.url().strip_prefix(PROXY_PREFIX).map(str::to_owned) else {
        respond_json(request, 404, serde_json::json!({"ok": false, "error": "not found"}));
        return;
    };
    // Check the pass before reading a body or starting a thread.
    let provided = request
        .headers()
        .iter()
        .find(|header| header.field.equiv(PROXY_AUTH_HEADER))
        .map(|header| header.value.as_str().to_owned())
        .unwrap_or_default();
    if !pass_matches(&provided) {
        respond_json(
            request,
            403,
            serde_json::json!({"ok": false, "error": format!("Send the header {PROXY_AUTH_HEADER}: ${PROXY_PASS_ENV} with every call through Bridge")}),
        );
        return;
    }
    let method = request.method().as_str().to_owned();
    let headers: Vec<(String, String)> = request
        .headers()
        .iter()
        .map(|header| (header.field.to_string(), header.value.as_str().to_owned()))
        .collect();
    let broker = broker.clone();
    // Each call on its own thread: a slow upstream must not hold up the
    // accept loop, `/health`, or another agent's call.
    std::thread::spawn(move || {
        let mut request = request;
        let mut body = Vec::new();
        if request
            .as_reader()
            .take((MAX_BODY_BYTES + 1) as u64)
            .read_to_end(&mut body)
            .is_err()
        {
            respond_json(request, 400, serde_json::json!({"ok": false, "error": "Could not read the request body"}));
            return;
        }
        match broker.proxy(ProxyRequest { route, method, headers, body }) {
            Ok(proxied) => {
                let mut response = tiny_http::Response::from_data(proxied.body).with_status_code(proxied.status);
                for (name, value) in proxied.headers {
                    if let Ok(header) = tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()) {
                        response.add_header(header);
                    }
                }
                let _ = request.respond(response);
            }
            Err(refusal) => respond_json(request, refusal.status, serde_json::json!({"ok": false, "error": refusal.message})),
        }
    });
}

fn respond_json(request: tiny_http::Request, status: u16, body: Value) {
    let mut response = tiny_http::Response::from_string(body.to_string()).with_status_code(status);
    if let Ok(header) = tiny_http::Header::from_bytes("Content-Type", "application/json") {
        response.add_header(header);
    }
    let _ = request.respond(response);
}

impl Drop for CredentialBroker {
    fn drop(&mut self) {
        self.chat_keys.get_mut().unwrap().clear();
        self.saved.get_mut().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret_interception;
    use std::{sync::mpsc, thread};

    const GITHUB: &str = "ghp_canaryabcdefghijklmnopqrstuvwxyz0123";

    /// A broker that calls plain HTTP, with one pasted GitHub token in
    /// `session`. Returns the token's placeholder reference.
    fn broker_with_github(session: &str) -> (CredentialBroker, String) {
        let intercepted = secret_interception::intercept(&format!("use {GITHUB}"));
        let reference = intercepted.sanitized.interceptions[0].reference.clone();
        let broker = CredentialBroker::with_scheme("http").unwrap();
        broker.register(session, intercepted.captured);
        (broker, reference)
    }

    fn allow(broker: &CredentialBroker, reference: &str, hosts: &[&str]) {
        broker.chat_keys.lock().unwrap().get_mut(reference).unwrap().hosts =
            hosts.iter().map(|host| (*host).to_owned()).collect();
    }

    type Seen = (String, String, Vec<(String, String)>, String);

    /// One-shot upstream. Replies with `reply`, reports what it received.
    fn upstream(reply: &'static str) -> (String, mpsc::Receiver<Seen>) {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let host = format!("127.0.0.1:{}", server.server_addr().to_ip().unwrap().port());
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut request = server.recv().unwrap();
            let headers = request
                .headers()
                .iter()
                .map(|header| (header.field.to_string().to_ascii_lowercase(), header.value.to_string()))
                .collect();
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body).unwrap();
            tx.send((request.method().to_string(), request.url().to_owned(), headers, body)).unwrap();
            let reply = reply.replace("{KEY}", GITHUB).replace(
                "{BASIC}",
                &BASE64.encode(format!("me@example.com:{GITHUB}")),
            );
            request
                .respond(tiny_http::Response::from_string(reply).with_status_code(201))
                .unwrap();
        });
        (host, rx)
    }

    fn call(route: String, headers: Vec<(&str, String)>) -> ProxyRequest {
        let mut headers: Vec<(String, String)> =
            headers.into_iter().map(|(name, value)| (name.to_owned(), value)).collect();
        headers.push((PROXY_AUTH_HEADER.into(), proxy_pass().to_owned()));
        ProxyRequest { route, method: "PATCH".into(), headers, body: b"{\"a\":1}".to_vec() }
    }

    fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
        headers.iter().find(|(field, _)| field == name).map(|(_, value)| value.as_str())
    }

    #[test]
    fn a_placeholder_in_a_header_becomes_the_key_only_on_an_allowed_host() {
        let (host, seen) = upstream("{\"echo\":\"{KEY}\"}");
        let (broker, reference) = broker_with_github("chat");
        allow(&broker, &reference, &[&host]);
        let response = broker
            .proxy(call(
                format!("chat/{host}/repos/x/issues/1?per_page=5"),
                vec![
                    ("Authorization", format!("Bearer [secret:{reference}]")),
                    ("X-GitHub-Api-Version", "2022-11-28".into()),
                    ("Accept-Encoding", "gzip".into()),
                ],
            ))
            .unwrap();
        let (method, path, headers, body) = seen.recv().unwrap();
        assert_eq!(method, "PATCH");
        assert_eq!(path, "/repos/x/issues/1?per_page=5");
        assert_eq!(header(&headers, "authorization"), Some(format!("Bearer {GITHUB}").as_str()));
        assert_eq!(header(&headers, "x-github-api-version"), Some("2022-11-28"));
        assert_eq!(header(&headers, PROXY_AUTH_HEADER), None, "the pass never leaves Bridge");
        assert_eq!(header(&headers, "accept-encoding"), None, "the response must arrive scrubbable");
        assert_eq!(body, "{\"a\":1}", "the body is forwarded untouched");
        assert_eq!(response.status, 201);
        let returned = String::from_utf8(response.body).unwrap();
        assert!(!returned.contains(GITHUB), "an echoed key is scrubbed: {returned}");
        assert!(returned.contains(&format!("[secret:{reference}]")));
        assert_eq!(header(&response.headers, REDACTED_HEADER), Some(format!("[secret:{reference}]").as_str()), "the agent is told a key came back");
    }

    #[test]
    fn a_key_is_refused_on_a_host_nobody_allowed() {
        let (broker, reference) = broker_with_github("chat");
        let refusal = broker
            .proxy(call(
                "chat/evil.example/collect".into(),
                vec![("Authorization", format!("Bearer [secret:{reference}]"))],
            ))
            .unwrap_err();
        assert_eq!(refusal.status, 403);
        assert!(refusal.message.contains("not allowed on evil.example"), "{}", refusal.message);
        assert!(refusal.message.contains("Settings"));
        assert!(!refusal.message.contains(GITHUB));
    }

    #[test]
    fn placeholders_outside_headers_are_never_filled() {
        let (host, seen) = upstream("ok");
        let (broker, reference) = broker_with_github("chat");
        allow(&broker, &reference, &[&host]);
        let mut request = call(
            format!("chat/{host}/echo?key=[secret:{reference}]"),
            vec![("Authorization", format!("token [secret:{reference}]"))],
        );
        request.body = format!("{{\"key\":\"[secret:{reference}]\"}}").into_bytes();
        let response = broker.proxy(request).unwrap();
        assert_eq!(header(&response.headers, REDACTED_HEADER), None, "nothing came back, so nothing is flagged");
        let (_, path, _, body) = seen.recv().unwrap();
        assert!(!path.contains(GITHUB) && !body.contains(GITHUB));
        assert!(body.contains(&format!("[secret:{reference}]")));
    }

    #[test]
    fn a_call_without_a_key_is_not_forwarded() {
        let (broker, _) = broker_with_github("chat");
        let refusal = broker
            .proxy(call("chat/api.github.com/user".into(), vec![("Accept", "application/json".into())]))
            .unwrap_err();
        assert_eq!(refusal.status, 400);
        assert!(refusal.message.contains("only forwards calls that use a key"));
    }

    #[test]
    fn a_chat_cannot_use_another_chats_pasted_key() {
        let (broker, reference) = broker_with_github("owner");
        let refusal = broker
            .proxy(call(
                "intruder/api.github.com/user".into(),
                vec![("Authorization", format!("Bearer [secret:{reference}]"))],
            ))
            .unwrap_err();
        assert_eq!(refusal.status, 403);
        assert!(refusal.message.contains("has no key"));
    }

    #[test]
    fn the_pass_is_required_and_never_comes_from_a_prompt() {
        let (broker, reference) = broker_with_github("chat");
        for pass in ["", "sec_not-the-pass"] {
            let mut request = call(
                "chat/api.github.com/user".into(),
                vec![("Authorization", format!("Bearer [secret:{reference}]"))],
            );
            request.headers.retain(|(name, _)| name != PROXY_AUTH_HEADER);
            request.headers.push((PROXY_AUTH_HEADER.into(), pass.into()));
            assert_eq!(broker.proxy(request).unwrap_err().status, 403);
        }
        broker.set_endpoint("127.0.0.1:4317".parse().unwrap());
        let contract = broker.instructions("chat");
        assert!(!contract.contains(proxy_pass()), "the pass is never in a prompt");
        assert!(contract.contains(&format!("${PROXY_PASS_ENV}")));
    }

    #[test]
    fn route_shapes_and_methods_are_checked() {
        let (broker, reference) = broker_with_github("chat");
        allow(&broker, &reference, &["api.github.com"]);
        let auth = || vec![("Authorization", format!("Bearer [secret:{reference}]"))];
        for route in ["", "chat", "/api.github.com/user", "chat/*.github.com/user", "chat/a b/x", "chat/api.github.com/a b"] {
            assert_eq!(broker.proxy(call(route.into(), auth())).unwrap_err().status, 400, "{route}");
        }
        let mut request = call("chat/api.github.com/user".into(), auth());
        request.method = "CONNECT".into();
        assert_eq!(broker.proxy(request).unwrap_err().status, 405);
        let mut request = call("chat/api.github.com/user".into(), auth());
        request.body = vec![0; MAX_BODY_BYTES + 1];
        assert_eq!(broker.proxy(request).unwrap_err().status, 413);
    }

    #[test]
    fn the_old_openai_route_still_works() {
        let key = "sk-proj-legacycanaryabcdefghijklmnopqrstuv";
        let intercepted = secret_interception::intercept(&format!("use {key}"));
        let reference = intercepted.sanitized.interceptions[0].reference.clone();
        let broker = CredentialBroker::new().unwrap();
        broker.register("chat", intercepted.captured);
        let Ok(prepared) = broker.prepare(call(format!("chat/{reference}/v1/models"), vec![])) else {
            panic!("the old route should resolve");
        };
        assert_eq!(prepared.url.as_str(), "https://api.openai.com/v1/models");
        let authorization = prepared.headers.iter().find(|(name, _)| name == "Authorization").unwrap();
        assert_eq!(authorization.1.as_str(), format!("Bearer {key}"));
        let refusal = broker
            .proxy(call(format!("chat/{reference}/models"), vec![]))
            .unwrap_err();
        assert_eq!(refusal.status, 400);
        let refusal = broker
            .proxy(call(format!("chat/{reference}/v1/models"), vec![("Authorization", "Bearer attacker".into())]))
            .unwrap_err();
        assert_eq!(refusal.status, 400);
    }

    #[test]
    fn a_saved_key_works_in_every_chat_and_follows_its_sites() {
        let (host, seen) = upstream("ok");
        let broker = CredentialBroker::with_scheme("http").unwrap();
        api_keys::save_value("BROKER_TEST_SAVED", GITHUB).unwrap();
        broker.put_saved(
            "BROKER_TEST_SAVED",
            api_keys::Metadata { kind: "github".into(), hosts: vec![host.clone()], aliases: vec!["sec_oldchat".into()] },
            None,
        );
        broker
            .proxy(call(format!("any-chat/{host}/user"), vec![("Authorization", "Bearer [secret:BROKER_TEST_SAVED]".into())]))
            .unwrap();
        let (_, _, headers, _) = seen.recv().unwrap();
        assert_eq!(header(&headers, "authorization"), Some(format!("Bearer {GITHUB}").as_str()));
        // An old chat's placeholder that was saved under this name still resolves.
        let refusal = broker
            .proxy(call("other/elsewhere.example/x".into(), vec![("Authorization", "Bearer [secret:sec_oldchat]".into())]))
            .unwrap_err();
        assert!(refusal.message.contains("BROKER_TEST_SAVED is not allowed on elsewhere.example"), "{}", refusal.message);
        broker.remove_saved("BROKER_TEST_SAVED");
        let refusal = broker
            .proxy(call(format!("any-chat/{host}/user"), vec![("Authorization", "Bearer [secret:BROKER_TEST_SAVED]".into())]))
            .unwrap_err();
        assert!(refusal.message.contains("has no key"));
        api_keys::delete_value("BROKER_TEST_SAVED").unwrap();
    }

    #[test]
    fn basic_credentials_are_filled_inside_their_encoding_and_scrubbed_back() {
        let (host, seen) = upstream("{\"authorization\":\"Basic {BASIC}\"}");
        let (broker, reference) = broker_with_github("chat");
        allow(&broker, &reference, &[&host]);
        // What `curl -u "me@example.com:[secret:…]"` sends.
        let token = BASE64.encode(format!("me@example.com:[secret:{reference}]"));
        let response = broker
            .proxy(call(format!("chat/{host}/rest/api/3/myself"), vec![("Authorization", format!("Basic {token}"))]))
            .unwrap();
        let (_, _, headers, _) = seen.recv().unwrap();
        let sent = header(&headers, "authorization").unwrap().strip_prefix("Basic ").unwrap();
        assert_eq!(BASE64.decode(sent).unwrap(), format!("me@example.com:{GITHUB}").into_bytes());
        let returned = String::from_utf8(response.body).unwrap();
        assert!(!returned.contains(sent), "the encoded key is scrubbed too: {returned}");
        assert!(returned.contains(&token));
        assert_eq!(header(&response.headers, REDACTED_HEADER), Some(format!("[secret:{reference}]").as_str()));
        // Basic credentials with no placeholder are not a key Bridge fills.
        let plain = BASE64.encode("user:password");
        assert_eq!(
            broker.proxy(call(format!("chat/{host}/x"), vec![("Authorization", format!("Basic {plain}"))])).unwrap_err().status,
            400
        );
    }

    #[test]
    fn wildcard_sites_cover_subdomains_only() {
        let allowed = vec!["*.example.com".to_owned()];
        assert!(host_allowed(&allowed, "api.example.com"));
        assert!(host_allowed(&allowed, "a.b.example.com"));
        assert!(!host_allowed(&allowed, "example.com"));
        assert!(!host_allowed(&allowed, "badexample.com"));
        assert!(!host_allowed(&allowed, "example.com.evil.net"));
    }

    #[test]
    fn pasted_keys_start_on_their_kinds_api() {
        let (broker, reference) = broker_with_github("chat");
        let (_, chat) = broker.summaries();
        let key = chat.iter().find(|key| key.reference == reference).unwrap();
        assert_eq!(key.hosts, vec!["api.github.com", "uploads.github.com"]);
        let private = secret_interception::intercept(
            "-----BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----",
        );
        broker.register("chat", private.captured);
        assert_eq!(broker.summaries().1.len(), 1, "a private key is redacted, not kept");
    }

    #[test]
    fn clearing_a_chat_forgets_its_keys() {
        let (broker, reference) = broker_with_github("owner");
        broker.clear_session("owner");
        assert!(!broker.chat_keys.lock().unwrap().contains_key(&reference));
    }

    #[test]
    fn turn_context_names_the_chats_keys_and_new_saved_keys() {
        let (broker, reference) = broker_with_github("owner");
        broker.set_endpoint("127.0.0.1:4317".parse().unwrap());
        broker.instructions("owner");
        let context = broker.turn_context("owner", &format!("list my repos [secret:{reference}]")).unwrap();
        assert!(context.contains(&format!("[secret:{reference}] is a GitHub token")));
        assert!(context.contains("allowed on api.github.com, uploads.github.com"));
        assert!(context.contains(PROXY_PREFIX) && context.contains(PROXY_AUTH_HEADER));
        assert!(!context.contains(GITHUB) && !context.contains(proxy_pass()));
        assert!(broker.turn_context("other", &format!("[secret:{reference}]")).is_none());
        assert!(broker.turn_context("owner", "ordinary prompt").is_none());
        broker.put_saved(
            "TURN_CONTEXT_KEY",
            api_keys::Metadata { kind: "openai".into(), hosts: vec!["api.openai.com".into()], aliases: vec![] },
            None,
        );
        let refreshed = broker.turn_context("owner", "ordinary prompt").unwrap();
        assert!(refreshed.contains("[secret:TURN_CONTEXT_KEY] (api.openai.com)"));
        assert!(broker.turn_context("owner", "ordinary prompt").is_none(), "told once per change");
    }

    #[test]
    fn instructions_say_so_when_the_proxy_is_not_running() {
        let broker = CredentialBroker::with_scheme("http").unwrap();
        let contract = broker.instructions("chat");
        assert!(contract.contains("not running"));
        assert!(!contract.contains(PROXY_PREFIX));
    }

    #[test]
    fn history_never_keeps_the_pass() {
        let mut event = NormalizedEvent::new("tool.started");
        let command = format!("curl -H '{PROXY_AUTH_HEADER}: {}' http://127.0.0.1:4317/x", proxy_pass());
        event.title = Some(command.clone());
        event.data = serde_json::json!({"command": command, "nested": [{"out": proxy_pass()}]});
        let clean = scrub_event(&event).unwrap();
        let serialized = format!("{:?}", clean);
        assert!(!serialized.contains(proxy_pass()));
        assert!(clean.title.unwrap().contains("$BRIDGE_PROXY_PASS"));
        let mut ordinary = NormalizedEvent::new("tool.started");
        ordinary.title = Some("ls".into());
        assert!(scrub_event(&ordinary).is_none());
    }

    #[test]
    fn scrubbing_replaces_every_occurrence() {
        let used = vec![Scrub { shown: "[secret:K]".into(), value: Zeroizing::new("abc".into()), label: "[secret:K]".into() }];
        assert_eq!(scrub_bytes(b"xabcabcy".to_vec(), &used), b"x[secret:K][secret:K]y".to_vec());
        assert_eq!(scrub_text("abc", &used), "[secret:K]");
    }
}
