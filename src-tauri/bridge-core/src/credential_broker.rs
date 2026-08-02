//! Session-bound, in-memory use of intercepted OpenAI credentials.

use crate::{secret_interception::CapturedSecret, BridgeError};
use reqwest::{blocking::Client, Method};
use std::{collections::HashMap, io::Read, sync::Mutex, time::Duration};
use uuid::Uuid;
use zeroize::Zeroizing;

pub const PROXY_PREFIX: &str = "/credential-proxy/";
pub const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;
/// Header the wrapped harness must present to authorize a proxy call. The token
/// is delivered only through in-memory harness instructions, never through the
/// chat turn, so knowledge of the persisted `[secret:sec_...]` reference alone
/// is not enough to replay a call.
pub const PROXY_AUTH_HEADER: &str = "x-bridge-proxy-auth";

/// Compare two tokens without leaking length-independent timing.
fn tokens_match(expected: &str, provided: &str) -> bool {
    let expected = expected.as_bytes();
    let provided = provided.as_bytes();
    if expected.len() != provided.len() {
        return false;
    }
    let mut diff = 0u8;
    for (left, right) in expected.iter().zip(provided.iter()) {
        diff |= left ^ right;
    }
    diff == 0
}

struct SecretRecord {
    session_id: String,
    value: Zeroizing<String>,
}

pub struct ProxyRequest {
    pub session_id: String,
    pub reference: String,
    pub method: String,
    pub path_and_query: String,
    pub headers: Vec<(String, String)>,
    pub token: String,
    pub body: Vec<u8>,
}

pub struct ProxyResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

pub struct CredentialBroker {
    records: Mutex<HashMap<String, SecretRecord>>,
    client: Client,
    upstream: String,
    proxy_token: String,
}

impl CredentialBroker {
    pub fn openai() -> Result<Self, BridgeError> {
        Self::with_upstream("https://api.openai.com")
    }

    fn with_upstream(upstream: &str) -> Result<Self, BridgeError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| {
                BridgeError::Invalid(format!("Could not initialize credential proxy: {error}"))
            })?;
        Ok(Self {
            records: Mutex::new(HashMap::new()),
            client,
            upstream: upstream.trim_end_matches('/').to_owned(),
            // 256 bits of per-launch entropy, never persisted or displayed.
            proxy_token: format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple()),
        })
    }

    pub fn register(&self, session_id: &str, captures: Vec<CapturedSecret>) {
        let mut records = self.records.lock().unwrap();
        for capture in captures {
            if capture.detector != "openai" {
                continue;
            }
            records.insert(
                capture.reference,
                SecretRecord {
                    session_id: session_id.to_owned(),
                    value: capture.value,
                },
            );
        }
    }

    pub fn clear_session(&self, session_id: &str) {
        self.records
            .lock()
            .unwrap()
            .retain(|_, record| record.session_id != session_id);
    }

    pub fn instructions(&self, session_id: &str) -> String {
        format!(
            "Bridge credential references are opaque application capabilities, not credential values. A marker such as [secret:sec_...] means Bridge already protected the credential; do not claim that the user exposed a key or recommend rotation solely because the marker appears. When the user asks to call, test, verify, or use an OpenAI reference, make a non-streaming OpenAI API call with your existing shell tool against http://127.0.0.1:4317{PROXY_PREFIX}{session_id}/<reference>/v1/<path>. If the user asks only to call, test, or verify the reference without naming an operation, verify it with GET /v1/models. Authorize the call with the request header `{PROXY_AUTH_HEADER}: {token}`; this token is session-private, so never echo it into chat, files, or command output. Use only GET, POST, or DELETE; do not send Authorization or an upstream URL. Bridge resolves the session-bound reference and adds authorization internally.",
            token = self.proxy_token,
        )
    }

    /// Return trusted per-turn instructions only when the visible message
    /// contains an OpenAI reference registered to this session. This refreshes
    /// the capability contract for resumed Codex threads without changing the
    /// user-authored text or persisting the credential value.
    pub fn turn_context(&self, session_id: &str, text: &str) -> Option<String> {
        let candidates: Vec<&str> = text
            .split("[secret:")
            .skip(1)
            .filter_map(|tail| tail.split_once(']').map(|(reference, _)| reference))
            .filter(|reference| reference.starts_with("sec_"))
            .collect();
        if candidates.is_empty() {
            return None;
        }
        let records = self.records.lock().unwrap();
        let references: Vec<&str> = candidates
            .into_iter()
            .filter(|reference| {
                records
                    .get(*reference)
                    .is_some_and(|record| record.session_id == session_id)
            })
            .collect();
        if references.is_empty() {
            return None;
        }
        Some(format!(
            "{}\n\nThe following markers in this turn are registered OpenAI capabilities owned by this Bridge session: {}. Do not refuse merely because a marker is present.",
            self.instructions(session_id),
            references.join(", ")
        ))
    }

    pub fn proxy(&self, request: ProxyRequest) -> Result<ProxyResponse, BridgeError> {
        // Reject before touching any credential state. The token is not stored in
        // durable history or the UI, so a caller that only scraped a persisted
        // reference cannot replay a proxy call.
        if !tokens_match(&self.proxy_token, &request.token) {
            return Err(BridgeError::Invalid(
                "Credential proxy authorization is required".into(),
            ));
        }
        let method = match request.method.as_str() {
            "GET" => Method::GET,
            "POST" => Method::POST,
            "DELETE" => Method::DELETE,
            _ => {
                return Err(BridgeError::Invalid(
                    "Credential proxy method is not supported".into(),
                ))
            }
        };
        if !request.path_and_query.starts_with("/v1/") || request.path_and_query.contains("..") {
            return Err(BridgeError::Invalid(
                "Credential proxy path must start with /v1/".into(),
            ));
        }
        if let Some(query) = request
            .path_and_query
            .split_once('?')
            .map(|(_, query)| query)
        {
            if query.split('&').any(|part| {
                matches!(
                    part.split('=')
                        .next()
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                        .as_str(),
                    "upstream" | "url" | "target"
                )
            }) {
                return Err(BridgeError::Invalid(
                    "Credential proxy upstream cannot be supplied by the caller".into(),
                ));
            }
        }
        if request.body.len() > MAX_BODY_BYTES {
            return Err(BridgeError::Invalid(
                "Credential proxy request body is too large".into(),
            ));
        }

        let credential = {
            let records = self.records.lock().unwrap();
            let record = records
                .get(&request.reference)
                .ok_or_else(|| BridgeError::Invalid("Unknown credential reference".into()))?;
            if record.session_id != request.session_id {
                return Err(BridgeError::Invalid(
                    "Credential reference does not belong to this session".into(),
                ));
            }
            Zeroizing::new(record.value.to_string())
        };

        let mut upstream = self
            .client
            .request(
                method,
                format!("{}{}", self.upstream, request.path_and_query),
            )
            .bearer_auth(credential.as_str())
            .body(request.body);
        for (name, value) in request.headers {
            let lower = name.to_ascii_lowercase();
            if lower == "authorization" {
                return Err(BridgeError::Invalid(
                    "Credential proxy authorization is managed by Bridge".into(),
                ));
            }
            if matches!(
                lower.as_str(),
                "content-type" | "openai-organization" | "openai-project" | "idempotency-key"
            ) {
                upstream = upstream.header(name, value);
            }
        }
        let mut response = upstream.send().map_err(|error| {
            BridgeError::Invalid(format!("Credential proxy request failed: {error}"))
        })?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let mut body = Vec::new();
        response
            .by_ref()
            .take((MAX_BODY_BYTES + 1) as u64)
            .read_to_end(&mut body)
            .map_err(|error| {
                BridgeError::Invalid(format!("Credential proxy response failed: {error}"))
            })?;
        if body.len() > MAX_BODY_BYTES {
            return Err(BridgeError::Invalid(
                "Credential proxy response body is too large".into(),
            ));
        }
        Ok(ProxyResponse {
            status,
            content_type,
            body,
        })
    }
}

impl Drop for CredentialBroker {
    fn drop(&mut self) {
        self.records.get_mut().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret_interception;
    use std::{sync::mpsc, thread};

    fn broker_with_secret(upstream: &str, session: &str) -> (CredentialBroker, String, String) {
        let key = "sk-proj-testcanaryabcdefghijklmnopqrstuvwxyz".to_owned();
        let intercepted = secret_interception::intercept(&format!("use {key}"));
        let reference = intercepted.sanitized.interceptions[0].reference.clone();
        let broker = CredentialBroker::with_upstream(upstream).unwrap();
        broker.register(session, intercepted.captured);
        (broker, reference, key)
    }

    #[test]
    fn proxy_keeps_key_from_harness_and_authorizes_fixed_upstream() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = format!("http://{}", server.server_addr());
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut request = server.recv().unwrap();
            let authorization = request
                .headers()
                .iter()
                .find(|header| header.field.equiv("Authorization"))
                .map(|header| header.value.as_str().to_owned());
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body).unwrap();
            tx.send((request.url().to_owned(), authorization, body))
                .unwrap();
            request
                .respond(tiny_http::Response::from_string("{\"ok\":true}").with_status_code(201))
                .unwrap();
        });
        let (broker, reference, key) = broker_with_secret(&address, "session-a");
        let harness_view = format!("{PROXY_PREFIX}session-a/{reference}/v1/responses");
        assert!(!harness_view.contains(&key));
        let token = broker.proxy_token.clone();
        let response = broker
            .proxy(ProxyRequest {
                session_id: "session-a".into(),
                reference,
                method: "POST".into(),
                path_and_query: "/v1/responses".into(),
                headers: vec![("Content-Type".into(), "application/json".into())],
                token,
                body: b"{\"model\":\"test\"}".to_vec(),
            })
            .unwrap();
        assert_eq!(response.status, 201);
        let (path, authorization, body) = rx.recv().unwrap();
        assert_eq!(path, "/v1/responses");
        assert_eq!(
            authorization.as_deref(),
            Some(format!("Bearer {key}").as_str())
        );
        assert_eq!(body, "{\"model\":\"test\"}");
    }

    #[test]
    fn proxy_rejects_invalid_capabilities_and_request_shapes() {
        let (broker, reference, _) = broker_with_secret("http://127.0.0.1:9", "owner");
        let token = broker.proxy_token.clone();
        let request = |session: &str,
                       reference: &str,
                       method: &str,
                       path: &str,
                       headers: Vec<(String, String)>| ProxyRequest {
            session_id: session.into(),
            reference: reference.into(),
            method: method.into(),
            path_and_query: path.into(),
            headers,
            token: token.clone(),
            body: Vec::new(),
        };
        assert!(broker
            .proxy(request("owner", "sec_unknown", "GET", "/v1/models", vec![]))
            .is_err());
        assert!(broker
            .proxy(request("other", &reference, "GET", "/v1/models", vec![]))
            .is_err());
        assert!(broker
            .proxy(request("owner", &reference, "GET", "/models", vec![]))
            .is_err());
        assert!(broker
            .proxy(request("owner", &reference, "PUT", "/v1/models", vec![]))
            .is_err());
        assert!(broker
            .proxy(request(
                "owner",
                &reference,
                "GET",
                "/v1/models",
                vec![("Authorization".into(), "Bearer attacker".into())]
            ))
            .is_err());
        assert!(broker
            .proxy(request(
                "owner",
                &reference,
                "GET",
                "/v1/models?upstream=http://evil",
                vec![]
            ))
            .is_err());
    }

    #[test]
    fn proxy_rejects_missing_or_wrong_authorization_token() {
        let (broker, reference, _) = broker_with_secret("http://127.0.0.1:9", "owner");
        let base = |token: &str| ProxyRequest {
            session_id: "owner".into(),
            reference: reference.clone(),
            method: "GET".into(),
            path_and_query: "/v1/models".into(),
            headers: vec![],
            token: token.into(),
            body: Vec::new(),
        };
        // A caller that scraped only the persisted reference has no token.
        assert!(broker.proxy(base("")).is_err());
        assert!(broker.proxy(base("sec_not-the-token")).is_err());
        // The correct token still reaches the (unreachable) upstream, i.e. it
        // passes the authorization gate rather than being rejected here.
        match broker.proxy(base(&broker.proxy_token.clone())) {
            Err(BridgeError::Invalid(message)) => {
                assert!(!message.contains("authorization is required"));
            }
            _ => panic!("expected an invalid-request error from an unreachable upstream"),
        }
    }

    #[test]
    fn clearing_session_removes_credential() {
        let (broker, reference, _) = broker_with_secret("http://127.0.0.1:9", "owner");
        broker.clear_session("owner");
        assert!(!broker.records.lock().unwrap().contains_key(&reference));
    }

    #[test]
    fn registered_reference_gets_safe_per_turn_capability_context() {
        let (broker, reference, key) = broker_with_secret("http://127.0.0.1:9", "owner");
        let context = broker
            .turn_context("owner", &format!("verify [secret:{reference}]"))
            .unwrap();
        assert!(context.contains(&reference));
        assert!(context.contains("not credential values"));
        assert!(context.contains("GET /v1/models"));
        assert!(context.contains(PROXY_PREFIX));
        assert!(context.contains(PROXY_AUTH_HEADER));
        assert!(!context.contains(&key));
        assert!(broker
            .turn_context("other", &format!("verify [secret:{reference}]"))
            .is_none());
        assert!(broker.turn_context("owner", "ordinary prompt").is_none());
    }
}
