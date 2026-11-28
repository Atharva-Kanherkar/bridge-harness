//! The Bridge account: who is signed in to Bridge itself, through WorkOS
//! AuthKit.
//!
//! This is not a provider account. `provider_accounts` models a vendor's own
//! login (Claude, Codex and the rest); nothing here reads, stores or forwards
//! one of those.
//!
//! Bridge is a public OAuth client. Sign-in opens AuthKit in the person's own
//! browser, receives the code on a one-shot `127.0.0.1` listener and exchanges
//! it with PKCE, so the build carries a client id and nothing secret. The
//! refresh token lives in the OS keychain, the access token in this process's
//! memory, and the store keeps only who is signed in. Neither token is ever
//! returned by a function a protocol method reaches.
//!
//! Signed out is a normal state: every failure here ends there, and none of
//! it runs on the boot path.

use crate::events::CoreEvent;
use crate::runtime::BridgeCore;
use crate::BridgeError;
use base64::Engine;
use bridge_protocol::messages::{AccountState, AccountStatus, AccountUser, BeginAccountSignIn};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

/// The keychain service the refresh token lives under. Deliberately absent
/// from the services `bridged`'s keychain read helper serves: that helper can
/// be run by anything that can run `bridged`, and this item is read in process
/// only.
pub const KEYCHAIN_SERVICE: &str = "dev.bridge.deck.account";
const KEYCHAIN_ITEM: &str = "refresh-token";

/// Overrides the client id baked in at build time, to develop against a
/// WorkOS staging environment.
pub const CLIENT_ID_ENV: &str = "BRIDGE_WORKOS_CLIENT_ID";
const BAKED_CLIENT_ID: Option<&str> = option_env!("BRIDGE_WORKOS_CLIENT_ID");
const WORKOS_API: &str = "https://api.workos.com";

const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_TOKEN_BYTES: usize = 16 * 1024;

const NOT_SET_UP: &str = "Sign-in is not set up in this build of Bridge.";
const NO_KEYCHAIN: &str = "Signing in to Bridge needs the macOS Keychain, which this build does not have.";
const EXPIRED: &str = "Your Bridge sign-in expired. Sign in again.";
const TIMED_OUT: &str = "Sign-in timed out. Try again.";
const MISMATCH: &str = "The sign-in response did not match this request. Try again.";
const REFUSED: &str = "WorkOS did not accept the sign-in. Try again.";
const UNREACHABLE: &str = "Bridge could not reach the sign-in service. Check your connection and try again.";

/// Where sign-in talks to, and as which client.
#[derive(Clone)]
pub struct Config {
    pub client_id: String,
    pub api_base: String,
}

impl Config {
    /// The configured client, or `None` when this build has no client id.
    pub fn resolve() -> Option<Self> {
        Self::from_values(std::env::var(CLIENT_ID_ENV).ok().as_deref(), BAKED_CLIENT_ID)
    }

    fn from_values(env: Option<&str>, baked: Option<&str>) -> Option<Self> {
        let client_id = [env, baked]
            .into_iter()
            .flatten()
            .map(str::trim)
            .find(|value| !value.is_empty())?;
        // A client id is public and looks like `client_01H…`. A secret key
        // (`sk_…`) set here by mistake must never be sent as one.
        let plausible = client_id.len() <= 128
            && !client_id.starts_with("sk_")
            && client_id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
        plausible.then(|| Self { client_id: client_id.to_owned(), api_base: WORKOS_API.to_owned() })
    }
}

/// Whether this build can keep a session past a restart.
fn keychain_supported() -> bool {
    cfg!(any(target_os = "macos", test))
}

/// The refresh token's home. Saves and deletes share the store saved API keys
/// use; the read is in process, with keychain prompts off, so a rebuilt binary
/// that the item's access list does not name gets "missing" instead of a
/// dialog.
mod keychain {
    use super::{BridgeError, Zeroizing, KEYCHAIN_ITEM, KEYCHAIN_SERVICE};
    use crate::account_secrets::keychain as store;

    pub fn save(token: &str) -> Result<(), BridgeError> {
        // Replacing in place needs the old item's access list to name this
        // binary; a delete does not.
        store::delete(KEYCHAIN_SERVICE, KEYCHAIN_ITEM).ok();
        store::save(KEYCHAIN_SERVICE, KEYCHAIN_ITEM, token)
    }

    pub fn delete() -> Result<(), BridgeError> {
        store::delete(KEYCHAIN_SERVICE, KEYCHAIN_ITEM)
    }

    #[cfg(all(target_os = "macos", not(test)))]
    pub fn read() -> Option<Zeroizing<String>> {
        crate::provider_usage::credentials::keychain_in_process(KEYCHAIN_SERVICE, KEYCHAIN_ITEM)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .map(Zeroizing::new)
            .filter(|token| !token.is_empty())
    }

    #[cfg(not(all(target_os = "macos", not(test))))]
    pub fn read() -> Option<Zeroizing<String>> {
        store::read(KEYCHAIN_SERVICE, KEYCHAIN_ITEM).ok().flatten()
    }
}

// ---------------------------------------------------------------------------
// Store: who is signed in. No token columns, by construction and by test.

pub fn install_store(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS bridge_account (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            user_id TEXT NOT NULL,
            email TEXT NOT NULL,
            name TEXT NOT NULL,
            avatar_url TEXT,
            organization_id TEXT,
            signed_in_at TEXT NOT NULL
        );",
    )?;
    Ok(())
}

pub fn stored_user(db: &Connection) -> Result<Option<AccountUser>, BridgeError> {
    Ok(db
        .query_row(
            "SELECT user_id, email, name, avatar_url, organization_id FROM bridge_account WHERE id = 1",
            [],
            |row| {
                Ok(AccountUser {
                    id: row.get(0)?,
                    email: row.get(1)?,
                    name: row.get(2)?,
                    avatar_url: row.get(3)?,
                    organization_id: row.get(4)?,
                })
            },
        )
        .optional()?)
}

/// Record who is signed in. A refresh keeps the original sign-in time.
fn store_user(db: &Connection, user: &AccountUser) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO bridge_account(id, user_id, email, name, avatar_url, organization_id, signed_in_at)
         VALUES(1, ?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET
            signed_in_at = CASE WHEN bridge_account.user_id = excluded.user_id THEN bridge_account.signed_in_at ELSE excluded.signed_in_at END,
            user_id = excluded.user_id, email = excluded.email, name = excluded.name,
            avatar_url = excluded.avatar_url, organization_id = excluded.organization_id",
        params![user.id, user.email, user.name, user.avatar_url, user.organization_id, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

fn clear_user(db: &Connection) -> Result<(), BridgeError> {
    db.execute("DELETE FROM bridge_account", [])?;
    Ok(())
}

// ---------------------------------------------------------------------------
// PKCE and the authorization URL.

/// 244 bits from the OS generator, in PKCE's unreserved alphabet.
fn random_token() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

fn code_challenge(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn redirect_uri(port: u16) -> String {
    // The literal address, not `localhost`: a name can be resolved elsewhere.
    format!("http://127.0.0.1:{port}/callback")
}

fn authorization_url(config: &Config, redirect_uri: &str, state: &str, verifier: &str) -> Result<String, BridgeError> {
    reqwest::Url::parse_with_params(
        &format!("{}/user_management/authorize", config.api_base),
        [
            ("response_type", "code"),
            ("client_id", config.client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("provider", "authkit"),
            ("state", state),
            ("code_challenge", code_challenge(verifier).as_str()),
            ("code_challenge_method", "S256"),
        ],
    )
    .map(String::from)
    .map_err(|_| BridgeError::Invalid(NOT_SET_UP.into()))
}

// ---------------------------------------------------------------------------
// The loopback callback.

#[derive(Debug, PartialEq, Eq)]
enum Callback {
    /// Not the callback at all (a favicon request, a port scan). Keep waiting.
    Ignore,
    Code(String),
    /// The callback arrived and cannot be used. The flow ends.
    Rejected(&'static str),
}

/// Read one request target against the `state` this flow issued.
fn read_callback(target: &str, expected_state: &str) -> Callback {
    let Ok(url) = reqwest::Url::parse(&format!("http://127.0.0.1{target}")) else {
        return Callback::Ignore;
    };
    if url.path() != "/callback" {
        return Callback::Ignore;
    }
    let param = |name: &str| url.query_pairs().find(|(key, _)| key == name).map(|(_, value)| value.into_owned());
    if param("state").as_deref() != Some(expected_state) {
        return Callback::Rejected(MISMATCH);
    }
    if param("error").is_some() {
        return Callback::Rejected("Sign-in was cancelled in the browser.");
    }
    match param("code") {
        Some(code) if !code.is_empty() && code.len() <= 2048 => Callback::Code(code),
        _ => Callback::Rejected(MISMATCH),
    }
}

fn page(heading: &str, detail: &str) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Bridge</title>\
         <body style=\"font:16px -apple-system,system-ui,sans-serif;display:grid;place-items:center;height:100vh;margin:0;background:#111;color:#eee\">\
         <main style=\"text-align:center\"><h1 style=\"font-size:20px\">{heading}</h1><p style=\"opacity:.7\">{detail}</p></main>"
    );
    let mut response = tiny_http::Response::from_string(body);
    for (name, value) in [("Content-Type", "text/html; charset=utf-8"), ("Cache-Control", "no-store"), ("Referrer-Policy", "no-referrer")] {
        if let Ok(header) = tiny_http::Header::from_bytes(name, value) {
            response.add_header(header);
        }
    }
    response
}

/// Serve the listener until the callback arrives, the person cancels, or the
/// deadline passes. Consumes the server, so the port is closed on every path
/// out and a second callback finds nothing listening.
fn wait_for_code(server: tiny_http::Server, expected_state: &str, cancelled: &AtomicBool, timeout: Duration) -> Result<String, &'static str> {
    let deadline = Instant::now() + timeout;
    loop {
        if cancelled.load(Ordering::SeqCst) {
            return Err("");
        }
        if Instant::now() >= deadline {
            return Err(TIMED_OUT);
        }
        let Ok(Some(request)) = server.recv_timeout(Duration::from_millis(100)) else {
            continue;
        };
        if *request.method() != tiny_http::Method::Get {
            request.respond(tiny_http::Response::empty(405)).ok();
            continue;
        }
        match read_callback(request.url(), expected_state) {
            Callback::Ignore => {
                request.respond(tiny_http::Response::empty(404)).ok();
            }
            Callback::Code(code) => {
                request.respond(page("You're signed in", "You can close this tab and return to Bridge.")).ok();
                return Ok(code);
            }
            Callback::Rejected(reason) => {
                request.respond(page("Sign-in did not finish", "Return to Bridge and try again.").with_status_code(400)).ok();
                return Err(reason);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The token endpoint.

/// What a successful exchange or refresh hands back. Not `Debug`, not
/// `Serialize`: it can only be taken apart on purpose.
struct Session {
    user: AccountUser,
    access_token: Zeroizing<String>,
    refresh_token: Zeroizing<String>,
}

enum AuthError {
    /// WorkOS answered and said no: a spent or wrong code, or a refresh token
    /// that expired or was revoked.
    Rejected,
    /// No answer worth acting on: offline, a timeout, a 5xx. Says nothing
    /// about whether the session is still good.
    Unreachable,
}

#[derive(Deserialize)]
struct WireUser {
    id: String,
    email: String,
    #[serde(default)]
    first_name: Option<String>,
    #[serde(default)]
    last_name: Option<String>,
    #[serde(default)]
    profile_picture_url: Option<String>,
}

#[derive(Deserialize)]
struct WireSession {
    user: WireUser,
    #[serde(default)]
    organization_id: Option<String>,
    access_token: String,
    refresh_token: String,
}

fn display_name(user: &WireUser) -> String {
    let name = [user.first_name.as_deref(), user.last_name.as_deref()]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if name.is_empty() { user.email.clone() } else { name }
}

fn http_client() -> Result<reqwest::blocking::Client, AuthError> {
    reqwest::blocking::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| AuthError::Unreachable)
}

/// POST one grant to `/user_management/authenticate`. The response body is
/// parsed and dropped; no part of it reaches an error string.
fn authenticate(config: &Config, grant: serde_json::Value) -> Result<Session, AuthError> {
    let response = http_client()?
        .post(format!("{}/user_management/authenticate", config.api_base))
        .json(&grant)
        .send()
        .map_err(|_| AuthError::Unreachable)?;
    let status = response.status();
    if status.is_client_error() && status != reqwest::StatusCode::TOO_MANY_REQUESTS && status != reqwest::StatusCode::REQUEST_TIMEOUT {
        return Err(AuthError::Rejected);
    }
    if !status.is_success() {
        return Err(AuthError::Unreachable);
    }
    let wire: WireSession = response.json().map_err(|_| AuthError::Unreachable)?;
    let sane = |token: &str| !token.is_empty() && token.len() <= MAX_TOKEN_BYTES && !token.chars().any(char::is_control);
    if !sane(&wire.access_token) || !sane(&wire.refresh_token) || wire.user.id.is_empty() {
        return Err(AuthError::Unreachable);
    }
    Ok(Session {
        user: AccountUser {
            name: display_name(&wire.user),
            id: wire.user.id,
            email: wire.user.email,
            avatar_url: wire.user.profile_picture_url.filter(|url| url.starts_with("https://")),
            organization_id: wire.organization_id,
        },
        access_token: Zeroizing::new(wire.access_token),
        refresh_token: Zeroizing::new(wire.refresh_token),
    })
}

fn exchange_code(config: &Config, code: &str, verifier: &str) -> Result<Session, AuthError> {
    authenticate(config, serde_json::json!({
        "client_id": config.client_id,
        "grant_type": "authorization_code",
        "code": code,
        "code_verifier": verifier,
    }))
}

fn exchange_refresh(config: &Config, refresh_token: &str) -> Result<Session, AuthError> {
    authenticate(config, serde_json::json!({
        "client_id": config.client_id,
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
    }))
}

/// The WorkOS session id (`sid`) inside an access token, for sign-out. Read
/// without verifying the signature: it only names which session to end.
fn session_id(access_token: &str) -> Option<String> {
    let payload = access_token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let sid = claims.get("sid")?.as_str()?;
    (sid.len() <= 128 && sid.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')).then(|| sid.to_owned())
}

/// Ask WorkOS to end the session. Best effort: signing out locally never waits
/// on it or fails because of it.
fn revoke(config: &Config, session_id: &str) {
    let Ok(client) = http_client() else { return };
    let Ok(url) = reqwest::Url::parse_with_params(
        &format!("{}/user_management/sessions/logout", config.api_base),
        [("session_id", session_id)],
    ) else {
        return;
    };
    client.get(url).send().ok();
}

// ---------------------------------------------------------------------------
// Runtime state.

#[derive(Default)]
pub struct AccountRuntime {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// Bumped by everything that changes who is signed in, so a slow exchange
    /// or refresh that finishes after a sign-out or a newer sign-in is dropped.
    epoch: u64,
    pending: Option<Pending>,
    access_token: Option<Zeroizing<String>>,
    error: Option<String>,
}

struct Pending {
    epoch: u64,
    cancelled: Arc<AtomicBool>,
}

impl AccountRuntime {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn available(config: Option<&Config>) -> bool {
    config.is_some() && keychain_supported()
}

pub fn status(core: &BridgeCore) -> Result<AccountStatus, BridgeError> {
    status_with(core, Config::resolve().as_ref())
}

fn status_with(core: &BridgeCore, config: Option<&Config>) -> Result<AccountStatus, BridgeError> {
    let (signing_in, error) = {
        let inner = core.account.lock();
        (inner.pending.is_some(), inner.error.clone())
    };
    let user = stored_user(&core.db.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))?;
    let state = match (signing_in, &user) {
        (true, _) => AccountState::SigningIn,
        (false, Some(_)) => AccountState::SignedIn,
        (false, None) => AccountState::SignedOut,
    };
    Ok(AccountStatus {
        state,
        user: (state == AccountState::SignedIn).then_some(user).flatten(),
        error: (state == AccountState::SignedOut).then_some(error).flatten(),
        available: available(config),
    })
}

/// The access token for a Bridge Cloud call, when signed in and refreshed this
/// run. For host code only: no protocol method returns it.
pub fn access_token(core: &BridgeCore) -> Option<Zeroizing<String>> {
    core.account.lock().access_token.clone()
}

/// Start a sign-in: bind the loopback listener and hand back the page to open.
/// A sign-in already under way is abandoned for the new one.
pub fn begin_sign_in(core: &Arc<BridgeCore>) -> Result<BeginAccountSignIn, BridgeError> {
    let config = Config::resolve().ok_or_else(|| BridgeError::Invalid(NOT_SET_UP.into()))?;
    begin_sign_in_with(core, config, SIGN_IN_TIMEOUT)
}

fn begin_sign_in_with(core: &Arc<BridgeCore>, config: Config, timeout: Duration) -> Result<BeginAccountSignIn, BridgeError> {
    if !keychain_supported() {
        return Err(BridgeError::Invalid(NO_KEYCHAIN.into()));
    }
    let server = tiny_http::Server::http("127.0.0.1:0")
        .map_err(|_| BridgeError::Invalid("Bridge could not open a local port for sign-in.".into()))?;
    let port = server
        .server_addr()
        .to_ip()
        .map(|address| address.port())
        .ok_or_else(|| BridgeError::Invalid("Bridge could not open a local port for sign-in.".into()))?;
    let state = random_token();
    let verifier = Zeroizing::new(random_token());
    let redirect = redirect_uri(port);
    let authorization_url = authorization_url(&config, &redirect, &state, &verifier)?;

    let cancelled = Arc::new(AtomicBool::new(false));
    let epoch = {
        let mut inner = core.account.lock();
        if let Some(previous) = inner.pending.take() {
            previous.cancelled.store(true, Ordering::SeqCst);
        }
        inner.epoch += 1;
        inner.error = None;
        inner.pending = Some(Pending { epoch: inner.epoch, cancelled: cancelled.clone() });
        inner.epoch
    };
    core.events.publish(CoreEvent::AccountChanged);

    let worker = core.clone();
    let spawned = std::thread::Builder::new().name("bridge-account-sign-in".into()).spawn(move || {
        let outcome = wait_for_code(server, &state, &cancelled, timeout).and_then(|code| {
            exchange_code(&config, &code, &verifier).map_err(|error| match error {
                AuthError::Rejected => REFUSED,
                AuthError::Unreachable => UNREACHABLE,
            })
        });
        finish_sign_in(&worker, epoch, outcome);
    });
    if spawned.is_err() {
        finish_sign_in(core, epoch, Err("Bridge could not start sign-in. Try again."));
        return Err(BridgeError::Invalid("Bridge could not start sign-in. Try again.".into()));
    }
    Ok(BeginAccountSignIn { authorization_url })
}

/// Land a finished flow, unless something newer replaced it meanwhile.
fn finish_sign_in(core: &BridgeCore, epoch: u64, outcome: Result<Session, &'static str>) {
    {
        let mut inner = core.account.lock();
        if inner.pending.as_ref().map(|pending| pending.epoch) != Some(epoch) {
            return;
        }
        inner.pending = None;
        match outcome {
            Ok(session) => match adopt(core, &mut inner, session) {
                Ok(()) => inner.error = None,
                Err(error) => inner.error = Some(error.to_string()),
            },
            Err(reason) => inner.error = (!reason.is_empty()).then(|| reason.to_owned()),
        }
    }
    core.events.publish(CoreEvent::AccountChanged);
}

/// Make `session` the signed-in one: token to the keychain first, so a profile
/// is never shown for a session that could not be kept.
fn adopt(core: &BridgeCore, inner: &mut Inner, session: Session) -> Result<(), BridgeError> {
    keychain::save(&session.refresh_token)
        .map_err(|_| BridgeError::Invalid("Bridge could not save your sign-in in the macOS Keychain.".into()))?;
    if let Err(error) = store_user(&core.db.lock().unwrap_or_else(|poisoned| poisoned.into_inner()), &session.user) {
        keychain::delete().ok();
        return Err(error);
    }
    inner.epoch += 1;
    inner.access_token = Some(session.access_token);
    Ok(())
}

pub fn cancel_sign_in(core: &BridgeCore) -> Result<AccountStatus, BridgeError> {
    let cancelled = {
        let mut inner = core.account.lock();
        let pending = inner.pending.take();
        if let Some(pending) = &pending {
            pending.cancelled.store(true, Ordering::SeqCst);
            inner.epoch += 1;
        }
        pending.is_some()
    };
    if cancelled {
        core.events.publish(CoreEvent::AccountChanged);
    }
    status(core)
}

/// Forget the session here, then ask WorkOS to end it without waiting.
pub fn sign_out(core: &BridgeCore) -> Result<AccountStatus, BridgeError> {
    sign_out_with(core, Config::resolve())
}

fn sign_out_with(core: &BridgeCore, config: Option<Config>) -> Result<AccountStatus, BridgeError> {
    let access_token = {
        let mut inner = core.account.lock();
        if let Some(pending) = inner.pending.take() {
            pending.cancelled.store(true, Ordering::SeqCst);
        }
        inner.epoch += 1;
        inner.error = None;
        inner.access_token.take()
    };
    let removed = keychain::delete();
    clear_user(&core.db.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))?;
    core.events.publish(CoreEvent::AccountChanged);
    if let (Some(config), Some(session_id)) = (config.clone(), access_token.as_deref().and_then(|token| session_id(token))) {
        std::thread::Builder::new()
            .name("bridge-account-sign-out".into())
            .spawn(move || revoke(&config, &session_id))
            .ok();
    }
    removed?;
    status_with(core, config.as_ref())
}

/// Trade the stored refresh token for a fresh access token. Quiet by design:
/// no network leaves a signed-in profile on screen, and only WorkOS saying no
/// signs the person out.
pub fn refresh(core: &BridgeCore) {
    if let Some(config) = Config::resolve() {
        refresh_with(core, &config);
    }
}

fn refresh_with(core: &BridgeCore, config: &Config) {
    let Ok(Some(_)) = stored_user(&core.db.lock().unwrap_or_else(|poisoned| poisoned.into_inner())) else {
        return;
    };
    let epoch = core.account.lock().epoch;
    // The keychain read and the request both run without the runtime lock.
    let outcome = match keychain::read() {
        Some(token) => exchange_refresh(config, &token),
        // The profile outlived its token: another build's keychain item, or
        // one the person removed. Nothing to refresh with.
        None => Err(AuthError::Rejected),
    };
    {
        let mut inner = core.account.lock();
        if inner.epoch != epoch || inner.pending.is_some() {
            return;
        }
        match outcome {
            Ok(session) => {
                if let Err(error) = adopt(core, &mut inner, session) {
                    eprintln!("bridge account: could not keep the refreshed session: {error}");
                    return;
                }
            }
            Err(AuthError::Unreachable) => return,
            Err(AuthError::Rejected) => {
                inner.epoch += 1;
                inner.access_token = None;
                inner.error = Some(EXPIRED.into());
                keychain::delete().ok();
                clear_user(&core.db.lock().unwrap_or_else(|poisoned| poisoned.into_inner())).ok();
            }
        }
    }
    core.events.publish(CoreEvent::AccountChanged);
}

/// Refresh once, off the boot path. Startup never waits on WorkOS.
pub fn start_account_refresh(core: Arc<BridgeCore>) {
    std::thread::Builder::new()
        .name("bridge-account-refresh".into())
        .spawn(move || refresh(&core))
        .ok();
}

#[cfg(test)]
mod tests;
