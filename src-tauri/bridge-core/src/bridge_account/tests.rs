use super::*;
use std::sync::mpsc;

const REFRESH_1: &str = "rt_first_yAjhKk123NLIjdrBdGZPf8pLIDvK";
const REFRESH_2: &str = "rt_second_Qw8xUu2mZk41TnPbVc7RyLs0HdEa";

/// The test keychain is one map for the whole process, and this module keeps
/// one item in it.
fn keychain_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _ = keychain::delete();
    guard
}

fn core() -> (tempfile::TempDir, Arc<BridgeCore>) {
    let scratch = tempfile::tempdir().unwrap();
    let core = Arc::new(BridgeCore::for_tests(scratch.path()));
    (scratch, core)
}

fn access_token_for(session: &str) -> String {
    let encode = |value: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value);
    format!("{}.{}.sig", encode(r#"{"alg":"RS256"}"#), encode(&format!(r#"{{"sid":"{session}","sub":"user_1"}}"#)))
}

fn session_body(refresh_token: &str) -> serde_json::Value {
    serde_json::json!({
        "user": { "id": "user_1", "email": "ada@example.com", "first_name": "Ada", "last_name": "Lovelace", "profile_picture_url": "https://cdn.example.com/ada.png" },
        "organization_id": "org_1",
        "access_token": access_token_for("session_1"),
        "refresh_token": refresh_token,
    })
}

struct Seen {
    path: String,
    body: serde_json::Value,
}

/// A stand-in for WorkOS: answers each request with what `respond` returns
/// and reports what it was sent.
fn fake_workos(respond: impl Fn(&Seen) -> (u16, serde_json::Value) + Send + 'static) -> (Config, mpsc::Receiver<Seen>) {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for mut request in server.incoming_requests() {
            let mut body = String::new();
            let _ = request.as_reader().read_to_string(&mut body);
            let seen = Seen { path: request.url().to_owned(), body: serde_json::from_str(&body).unwrap_or(serde_json::Value::Null) };
            let (status, answer) = respond(&seen);
            let _ = request.respond(
                tiny_http::Response::from_string(answer.to_string())
                    .with_status_code(status)
                    .with_header("Content-Type: application/json".parse::<tiny_http::Header>().unwrap()),
            );
            if sender.send(seen).is_err() {
                break;
            }
        }
    });
    (Config { client_id: "client_test".into(), api_base: format!("http://127.0.0.1:{port}") }, receiver)
}

/// Somewhere nothing is listening.
fn dead_endpoint() -> Config {
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    Config { client_id: "client_test".into(), api_base: format!("http://127.0.0.1:{port}") }
}

fn listener() -> (tiny_http::Server, u16) {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    (server, port)
}

fn get(url: &str) -> Result<u16, reqwest::Error> {
    reqwest::blocking::Client::builder().timeout(Duration::from_secs(5)).build().unwrap().get(url).send().map(|response| response.status().as_u16())
}

fn wait_until(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn query(url: &str, name: &str) -> String {
    reqwest::Url::parse(url).unwrap().query_pairs().find(|(key, _)| key == name).map(|(_, value)| value.into_owned()).unwrap_or_else(|| panic!("no {name} in {url}"))
}

fn signed_in(core: &BridgeCore) {
    keychain::save(REFRESH_1).unwrap();
    store_user(
        &core.db.lock().unwrap(),
        &AccountUser { id: "user_1".into(), email: "ada@example.com".into(), name: "Ada Lovelace".into(), avatar_url: None, organization_id: None },
    )
    .unwrap();
}

#[test]
fn the_challenge_is_s256_of_the_verifier() {
    // RFC 7636, appendix B.
    assert_eq!(code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    let verifier = random_token();
    assert!((43..=128).contains(&verifier.len()), "RFC 7636 verifier length");
    assert!(verifier.bytes().all(|byte| byte.is_ascii_alphanumeric()));
    assert_ne!(verifier, random_token());
}

#[test]
fn the_authorization_url_carries_client_redirect_state_and_challenge_and_no_secret() {
    let config = Config { client_id: "client_test".into(), api_base: WORKOS_API.into() };
    let url = authorization_url(&config, &redirect_uri(49152), "state-1", "verifier-1").unwrap();
    assert!(url.starts_with("https://api.workos.com/user_management/authorize?"));
    assert_eq!(query(&url, "response_type"), "code");
    assert_eq!(query(&url, "client_id"), "client_test");
    assert_eq!(query(&url, "redirect_uri"), "http://127.0.0.1:49152/callback");
    assert_eq!(query(&url, "provider"), "authkit");
    assert_eq!(query(&url, "state"), "state-1");
    assert_eq!(query(&url, "code_challenge"), code_challenge("verifier-1"));
    assert_eq!(query(&url, "code_challenge_method"), "S256");
    assert!(!url.contains("verifier-1"), "the verifier never leaves the host");
    assert!(!url.contains("secret"));
}

#[test]
fn the_client_id_comes_from_the_environment_before_the_build_and_is_never_a_secret_key() {
    assert_eq!(Config::from_values(Some("client_env"), Some("client_baked")).unwrap().client_id, "client_env");
    assert_eq!(Config::from_values(None, Some("client_baked")).unwrap().client_id, "client_baked");
    assert_eq!(Config::from_values(Some("  "), Some("client_baked")).unwrap().client_id, "client_baked");
    assert!(Config::from_values(None, None).is_none(), "no client id means sign-in is off");
    assert!(Config::from_values(Some("sk_test_a1b2c3"), None).is_none(), "an API key is refused as a client id");
    assert!(Config::from_values(Some("client id"), None).is_none());
}

#[test]
fn the_callback_needs_the_state_this_flow_issued() {
    assert_eq!(read_callback("/callback?code=abc&state=s1", "s1"), Callback::Code("abc".into()));
    assert_eq!(read_callback("/callback?code=abc&state=other", "s1"), Callback::Rejected(MISMATCH));
    assert_eq!(read_callback("/callback?code=abc", "s1"), Callback::Rejected(MISMATCH));
    assert_eq!(read_callback("/callback?state=s1", "s1"), Callback::Rejected(MISMATCH));
    assert!(matches!(read_callback("/callback?error=access_denied&state=s1", "s1"), Callback::Rejected(_)));
    assert_eq!(read_callback("/favicon.ico", "s1"), Callback::Ignore);
    assert_eq!(read_callback("/callback/extra?code=abc&state=s1", "s1"), Callback::Ignore);
}

#[test]
fn the_listener_takes_one_matching_callback_then_stops_listening() {
    let (server, port) = listener();
    let waiting = std::thread::spawn(move || wait_for_code(server, "s1", &AtomicBool::new(false), Duration::from_secs(10)));
    assert_eq!(get(&format!("http://127.0.0.1:{port}/favicon.ico")).unwrap(), 404, "a stray request does not end the flow");
    assert_eq!(get(&format!("http://127.0.0.1:{port}/callback?code=abc&state=s1")).unwrap(), 200);
    assert_eq!(waiting.join().unwrap(), Ok("abc".to_owned()));
    assert!(get(&format!("http://127.0.0.1:{port}/callback?code=abc&state=s1")).is_err(), "a second callback finds nothing listening");
}

#[test]
fn the_listener_rejects_a_mismatched_state() {
    let (server, port) = listener();
    let waiting = std::thread::spawn(move || wait_for_code(server, "s1", &AtomicBool::new(false), Duration::from_secs(10)));
    assert_eq!(get(&format!("http://127.0.0.1:{port}/callback?code=abc&state=forged")).unwrap(), 400);
    assert_eq!(waiting.join().unwrap(), Err(MISMATCH));
    assert!(get(&format!("http://127.0.0.1:{port}/callback?code=abc&state=s1")).is_err(), "the right state arrives too late");
}

#[test]
fn the_listener_times_out_and_closes() {
    let (server, port) = listener();
    assert_eq!(wait_for_code(server, "s1", &AtomicBool::new(false), Duration::from_millis(150)), Err(TIMED_OUT));
    assert!(get(&format!("http://127.0.0.1:{port}/callback?code=abc&state=s1")).is_err(), "a late callback finds nothing listening");
}

#[test]
fn signing_in_exchanges_the_code_with_the_verifier_and_keeps_tokens_out_of_everything_a_client_sees() {
    let _keychain = keychain_guard();
    let (_scratch, core) = core();
    let (config, seen) = fake_workos(|_| (200, session_body(REFRESH_1)));
    let mut events = core.events.subscribe();

    let begun = begin_sign_in_with(&core, config.clone(), Duration::from_secs(30)).unwrap();
    assert_eq!(status_with(&core, Some(&config)).unwrap().state, AccountState::SigningIn);
    let redirect = query(&begun.authorization_url, "redirect_uri");
    assert!(redirect.starts_with("http://127.0.0.1:"), "loopback by address, not by name");
    let state = query(&begun.authorization_url, "state");

    assert_eq!(get(&format!("{redirect}?code=code_1&state={state}")).unwrap(), 200);
    wait_until("the exchange lands", || status_with(&core, Some(&config)).unwrap().state == AccountState::SignedIn);

    let exchange = seen.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(exchange.path, "/user_management/authenticate");
    assert_eq!(exchange.body["grant_type"], "authorization_code");
    assert_eq!(exchange.body["client_id"], "client_test");
    assert_eq!(exchange.body["code"], "code_1");
    assert_eq!(code_challenge(exchange.body["code_verifier"].as_str().unwrap()), query(&begun.authorization_url, "code_challenge"));
    assert!(exchange.body.get("client_secret").is_none(), "a public client sends no secret");

    let status = status_with(&core, Some(&config)).unwrap();
    assert_eq!(
        status.user,
        Some(AccountUser {
            id: "user_1".into(),
            email: "ada@example.com".into(),
            name: "Ada Lovelace".into(),
            avatar_url: Some("https://cdn.example.com/ada.png".into()),
            organization_id: Some("org_1".into()),
        })
    );
    assert_eq!(keychain::read().as_deref().map(String::as_str), Some(REFRESH_1));
    assert_eq!(access_token(&core).as_deref().map(String::as_str), Some(access_token_for("session_1").as_str()));

    // Nothing a client or the database holds carries either token.
    let wire = serde_json::to_string(&status).unwrap();
    let row: String = core
        .db
        .lock()
        .unwrap()
        .query_row("SELECT user_id||email||name||coalesce(avatar_url,'')||coalesce(organization_id,'')||signed_in_at FROM bridge_account", [], |row| row.get(0))
        .unwrap();
    for held in [wire.as_str(), row.as_str(), begun.authorization_url.as_str()] {
        assert!(!held.contains(REFRESH_1));
        assert!(!held.contains(&access_token_for("session_1")));
    }
    let mut hints = 0;
    while let Ok(event) = events.try_recv() {
        assert!(matches!(event, CoreEvent::AccountChanged));
        assert_eq!(event.payload(), serde_json::Value::Null, "the hint carries nothing");
        hints += 1;
    }
    assert!(hints >= 2, "one hint when sign-in starts and one when it lands");
}

#[test]
fn a_code_workos_refuses_leaves_the_app_signed_out_with_a_readable_error() {
    let _keychain = keychain_guard();
    let (_scratch, core) = core();
    // What WorkOS answers for a spent or forged code.
    let (config, _seen) = fake_workos(|_| (400, serde_json::json!({ "error": "invalid_grant", "error_description": "The code 'code_1' has expired or is invalid." })));
    let begun = begin_sign_in_with(&core, config.clone(), Duration::from_secs(30)).unwrap();
    let callback = format!("{}?code=code_1&state={}", query(&begun.authorization_url, "redirect_uri"), query(&begun.authorization_url, "state"));
    get(&callback).unwrap();
    wait_until("the refusal lands", || status_with(&core, Some(&config)).unwrap().state == AccountState::SignedOut);
    let status = status_with(&core, Some(&config)).unwrap();
    assert_eq!(status.error.as_deref(), Some(REFUSED));
    assert!(!status.error.unwrap().contains("code_1"), "the provider's own words are not passed on");
    assert!(keychain::read().is_none());
    assert!(stored_user(&core.db.lock().unwrap()).unwrap().is_none());
}

#[test]
fn a_forged_callback_ends_the_flow_signed_out_without_calling_workos() {
    let _keychain = keychain_guard();
    let (_scratch, core) = core();
    let (config, seen) = fake_workos(|_| (200, session_body(REFRESH_1)));
    let begun = begin_sign_in_with(&core, config.clone(), Duration::from_secs(30)).unwrap();
    assert_eq!(get(&format!("{}?code=stolen&state=forged", query(&begun.authorization_url, "redirect_uri"))).unwrap(), 400);
    wait_until("the flow ends", || status_with(&core, Some(&config)).unwrap().state == AccountState::SignedOut);
    assert_eq!(status_with(&core, Some(&config)).unwrap().error.as_deref(), Some(MISMATCH));
    assert!(seen.try_recv().is_err(), "no exchange was attempted");
    assert!(keychain::read().is_none());
}

#[test]
fn a_sign_in_nobody_finishes_times_out_signed_out() {
    let _keychain = keychain_guard();
    let (_scratch, core) = core();
    let begun = begin_sign_in_with(&core, dead_endpoint(), Duration::from_millis(150)).unwrap();
    let callback = format!("{}?code=late&state={}", query(&begun.authorization_url, "redirect_uri"), query(&begun.authorization_url, "state"));
    wait_until("the flow times out", || status_with(&core, None).unwrap().state == AccountState::SignedOut);
    assert_eq!(status_with(&core, None).unwrap().error.as_deref(), Some(TIMED_OUT));
    wait_until("the port closes", || get(&callback).is_err());
}

#[test]
fn cancelling_closes_the_listener_and_leaves_nothing_behind() {
    let _keychain = keychain_guard();
    let (_scratch, core) = core();
    let begun = begin_sign_in_with(&core, dead_endpoint(), Duration::from_secs(30)).unwrap();
    let callback = format!("{}?code=late&state={}", query(&begun.authorization_url, "redirect_uri"), query(&begun.authorization_url, "state"));
    let after = cancel_sign_in(&core).unwrap();
    assert_eq!(after.state, AccountState::SignedOut);
    assert_eq!(after.error, None, "cancelling is not an error");
    wait_until("the port closes", || get(&callback).is_err());
    assert!(keychain::read().is_none());
    assert!(stored_user(&core.db.lock().unwrap()).unwrap().is_none());
    assert!(access_token(&core).is_none());
}

#[test]
fn starting_again_abandons_the_sign_in_before_it() {
    let _keychain = keychain_guard();
    let (_scratch, core) = core();
    let (config, _seen) = fake_workos(|_| (200, session_body(REFRESH_1)));
    let first = begin_sign_in_with(&core, config.clone(), Duration::from_secs(30)).unwrap();
    let second = begin_sign_in_with(&core, config.clone(), Duration::from_secs(30)).unwrap();
    let stale = format!("{}?code=code_1&state={}", query(&first.authorization_url, "redirect_uri"), query(&first.authorization_url, "state"));
    wait_until("the first port closes", || get(&stale).is_err());
    assert_eq!(status_with(&core, Some(&config)).unwrap().state, AccountState::SigningIn, "the second is still waiting");
    get(&format!("{}?code=code_2&state={}", query(&second.authorization_url, "redirect_uri"), query(&second.authorization_url, "state"))).unwrap();
    wait_until("the second lands", || status_with(&core, Some(&config)).unwrap().state == AccountState::SignedIn);
}

#[test]
fn a_refresh_keeps_the_rotated_token_and_the_original_sign_in_time() {
    let _keychain = keychain_guard();
    let (_scratch, core) = core();
    signed_in(&core);
    let signed_in_at = |core: &BridgeCore| core.db.lock().unwrap().query_row("SELECT signed_in_at FROM bridge_account", [], |row| row.get::<_, String>(0)).unwrap();
    let before = signed_in_at(&core);
    let (config, seen) = fake_workos(|_| (200, session_body(REFRESH_2)));

    refresh_with(&core, &config);

    let sent = seen.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(sent.body["grant_type"], "refresh_token");
    assert_eq!(sent.body["refresh_token"], REFRESH_1);
    assert_eq!(sent.body["client_id"], "client_test");
    assert!(sent.body.get("client_secret").is_none());
    assert_eq!(keychain::read().as_deref().map(String::as_str), Some(REFRESH_2), "the rotated token replaces the spent one");
    assert!(access_token(&core).is_some());
    assert_eq!(status_with(&core, Some(&config)).unwrap().state, AccountState::SignedIn);
    assert_eq!(signed_in_at(&core), before);
}

#[test]
fn an_expired_or_revoked_refresh_token_signs_out_once_and_asks_to_sign_in_again() {
    let _keychain = keychain_guard();
    let (_scratch, core) = core();
    signed_in(&core);
    let (config, seen) = fake_workos(|_| (400, serde_json::json!({ "error": "invalid_grant", "error_description": "Session has already ended." })));

    refresh_with(&core, &config);

    let status = status_with(&core, Some(&config)).unwrap();
    assert_eq!(status.state, AccountState::SignedOut);
    assert_eq!(status.error.as_deref(), Some(EXPIRED));
    assert!(keychain::read().is_none());
    assert!(access_token(&core).is_none());
    seen.recv_timeout(Duration::from_secs(5)).unwrap();
    refresh_with(&core, &config);
    assert!(seen.try_recv().is_err(), "signed out, there is nothing left to retry");
}

#[test]
fn a_profile_whose_keychain_item_is_gone_signs_out_without_calling_workos() {
    let _keychain = keychain_guard();
    let (_scratch, core) = core();
    signed_in(&core);
    keychain::delete().unwrap();
    let (config, seen) = fake_workos(|_| (200, session_body(REFRESH_2)));
    refresh_with(&core, &config);
    assert_eq!(status_with(&core, Some(&config)).unwrap().error.as_deref(), Some(EXPIRED));
    assert!(seen.try_recv().is_err());
}

#[test]
fn with_workos_unreachable_or_failing_the_signed_in_profile_still_shows() {
    let _keychain = keychain_guard();
    let (_scratch, core) = core();
    signed_in(&core);
    let (failing, _seen) = fake_workos(|_| (503, serde_json::json!({ "message": "unavailable" })));
    for config in [dead_endpoint(), failing] {
        refresh_with(&core, &config);
        let status = status_with(&core, Some(&config)).unwrap();
        assert_eq!(status.state, AccountState::SignedIn);
        assert_eq!(status.user.unwrap().email, "ada@example.com");
        assert_eq!(keychain::read().as_deref().map(String::as_str), Some(REFRESH_1), "the token is kept for the next try");
    }
}

#[test]
fn signing_out_removes_the_keychain_item_and_the_row_and_asks_workos_to_end_the_session() {
    let _keychain = keychain_guard();
    let (_scratch, core) = core();
    signed_in(&core);
    let (config, seen) = fake_workos(|seen| if seen.path.starts_with("/user_management/authenticate") { (200, session_body(REFRESH_2)) } else { (302, serde_json::Value::Null) });
    refresh_with(&core, &config);
    seen.recv_timeout(Duration::from_secs(5)).unwrap();

    let after = sign_out_with(&core, Some(config)).unwrap();

    assert_eq!(after.state, AccountState::SignedOut);
    assert_eq!(after.user, None);
    assert!(keychain::read().is_none());
    assert!(access_token(&core).is_none());
    assert!(stored_user(&core.db.lock().unwrap()).unwrap().is_none());
    let logout = seen.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(logout.path, "/user_management/sessions/logout?session_id=session_1");
}

#[test]
fn signing_out_works_with_workos_unreachable_and_when_already_signed_out() {
    let _keychain = keychain_guard();
    let (_scratch, core) = core();
    signed_in(&core);
    assert_eq!(sign_out_with(&core, Some(dead_endpoint())).unwrap().state, AccountState::SignedOut);
    assert_eq!(sign_out_with(&core, None).unwrap().state, AccountState::SignedOut);
    assert!(keychain::read().is_none());
}

#[test]
fn a_refresh_that_lands_after_a_sign_out_is_dropped() {
    let _keychain = keychain_guard();
    let (_scratch, core) = core();
    signed_in(&core);
    let (release, held) = mpsc::channel::<()>();
    let held = Mutex::new(held);
    let (config, seen) = fake_workos(move |_| {
        let _ = held.lock().unwrap().recv_timeout(Duration::from_secs(10));
        (200, session_body(REFRESH_2))
    });
    let refreshing = { let (core, config) = (core.clone(), config.clone()); std::thread::spawn(move || refresh_with(&core, &config)) };
    wait_until("the refresh is in flight", || refreshing.is_finished() || core.account.lock().epoch == 0 && keychain::read().is_some());
    std::thread::sleep(Duration::from_millis(100));
    sign_out_with(&core, None).unwrap();
    release.send(()).unwrap();
    refreshing.join().unwrap();
    seen.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(status_with(&core, Some(&config)).unwrap().state, AccountState::SignedOut);
    assert!(keychain::read().is_none(), "the late session is not written back");
    assert!(access_token(&core).is_none());
}

#[test]
fn without_a_client_id_bridge_is_simply_signed_out() {
    let _keychain = keychain_guard();
    let (_scratch, core) = core();
    let status = status_with(&core, None).unwrap();
    assert_eq!(status, AccountStatus { state: AccountState::SignedOut, user: None, error: None, available: false });
    refresh(&core);
    assert_eq!(status_with(&core, None).unwrap().state, AccountState::SignedOut);
}

#[test]
fn the_session_id_is_read_from_the_access_token_and_nothing_odd_is_trusted() {
    assert_eq!(session_id(&access_token_for("session_01HQ")), Some("session_01HQ".into()));
    assert_eq!(session_id(&access_token_for("a b&return_to=evil")), None);
    assert_eq!(session_id("not-a-jwt"), None);
}

#[test]
fn a_session_with_no_usable_tokens_is_not_adopted() {
    let (config, _seen) = fake_workos(|_| (200, serde_json::json!({ "user": { "id": "user_1", "email": "ada@example.com" }, "access_token": "", "refresh_token": "" })));
    assert!(matches!(exchange_refresh(&config, REFRESH_1), Err(AuthError::Unreachable)));
    let (config, _seen) = fake_workos(|_| (200, serde_json::json!({ "user": { "id": "user_1", "email": "ada@example.com" }, "access_token": "a.b.c", "refresh_token": "rt_ok_1234567890" })));
    let session = exchange_refresh(&config, REFRESH_1).ok().unwrap();
    assert_eq!(session.user.name, "ada@example.com", "no name falls back to the email");
}

#[test]
fn account_tokens_do_not_survive_redaction() {
    let access = "eyJhbGciOiJSUzI1NiJ9.eyJzaWQiOiJzZXNzaW9uXzEiLCJzdWIiOiJ1c2VyXzEifQ.c2lnbmF0dXJlLWJ5dGVzLWhlcmU";
    for line in [
        format!("refresh_token={REFRESH_1}"),
        format!(r#"{{"refresh_token": "{REFRESH_1}"}}"#),
        format!("authenticate failed, access_token: {access}"),
        format!("Authorization: Bearer {access}"),
    ] {
        let sanitized = crate::secret_interception::sanitize(&line).text;
        assert!(!sanitized.contains(REFRESH_1), "{sanitized}");
        assert!(!sanitized.contains(access), "{sanitized}");
    }
}
