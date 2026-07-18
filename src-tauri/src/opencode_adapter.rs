use crate::{
    adapters::{AdapterRuntime, ResumeRequest, ShutdownReason, StartRequest},
    binary,
    delegation::WriteMode,
    BridgeError,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use reqwest::blocking::{Client, Response};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde_json::{json, Value};
use std::{
    io::{BufRead, Read},
    net::TcpListener,
    process::{Child, Command, Stdio},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

const MINIMUM_VERSION: (u64, u64, u64) = (1, 18, 3);

pub struct OpenCodeRuntime {
    child: Child,
    client: Client,
    base_url: String,
    directory: String,
    session_id: String,
    model: Option<ModelRef>,
    variant: Option<String>,
    instructions: Option<String>,
    current_turn: Arc<Mutex<Option<String>>>,
    stopped: bool,
}

pub struct StartedOpenCode {
    pub runtime: OpenCodeRuntime,
    pub reader: ChannelReader,
    pub startup_messages: Vec<Value>,
}

#[derive(Clone)]
struct ModelRef {
    provider_id: String,
    model_id: String,
}

pub fn start(request: StartRequest<'_>) -> Result<StartedOpenCode, BridgeError> {
    launch(request, None)
}

pub fn resume(request: ResumeRequest<'_>) -> Result<StartedOpenCode, BridgeError> {
    launch(
        StartRequest {
            cwd: request.cwd,
            model: request.model,
            effort: request.effort,
            instructions: request.instructions,
            write_mode: request.write_mode,
        },
        Some(request.provider_session_id),
    )
}

fn launch(
    request: StartRequest<'_>,
    resume_session_id: Option<&str>,
) -> Result<StartedOpenCode, BridgeError> {
    ensure_supported_version()?;
    let binary = binary::resolve("opencode")
        .ok_or_else(|| BridgeError::Invalid("OpenCode binary is not installed".into()))?;
    let port = reserve_port()?;
    let base_url = format!("http://127.0.0.1:{port}");
    let server_password = uuid::Uuid::new_v4().to_string();
    let mut command = Command::new(binary);
    command
        .args([
            "serve",
            "--hostname",
            "127.0.0.1",
            "--port",
            &port.to_string(),
        ])
        .current_dir(request.cwd)
        .env("OPENCODE_SERVER_USERNAME", "bridge")
        .env("OPENCODE_SERVER_PASSWORD", &server_password)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::adapters::configure_process_group(&mut command);
    let mut child = command.spawn()?;
    let mut headers = HeaderMap::new();
    let authorization = format!("Basic {}", BASE64.encode(format!("bridge:{server_password}")));
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&authorization)
            .map_err(|error| BridgeError::Adapter(format!("Cannot secure OpenCode server: {error}")))?,
    );
    let client = Client::builder()
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(2))
        .timeout(None)
        .build()
        .map_err(|error| BridgeError::Adapter(format!("Cannot create OpenCode client: {error}")))?;
    if let Err(error) = wait_until_ready(&client, &base_url, &mut child) {
        let _ = crate::adapters::terminate_process_group(child.id());
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }

    let model = request.model.and_then(parse_model);
    let variant = request
        .effort
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let directory = request.cwd.to_owned();
    let session = match resume_session_id {
        Some(session_id) => {
            let response = client
                .get(endpoint(
                    &base_url,
                    &format!("/session/{session_id}"),
                    &directory,
                ))
                .timeout(Duration::from_secs(10))
                .send()
                .map_err(http_error("resume OpenCode session"))?;
            checked_json(response, "resume OpenCode session")?
        }
        None => {
            let mut body = json!({
                "title": "Bridge session",
                "permission": permission_rules(request.write_mode),
            });
            if let Some(model) = &model {
                body["model"] = json!({
                    "providerID": model.provider_id,
                    "id": model.model_id,
                    "variant": variant,
                });
            }
            let response = client
                .post(endpoint(&base_url, "/session", &directory))
                .timeout(Duration::from_secs(10))
                .json(&body)
                .send()
                .map_err(http_error("create OpenCode session"))?;
            checked_json(response, "create OpenCode session")?
        }
    };
    let session_id = session
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| BridgeError::Adapter(format!("OpenCode returned no session id: {session}")))?
        .to_owned();

    let (sender, receiver) = mpsc::channel();
    spawn_event_stream(
        client.clone(),
        base_url.clone(),
        directory.clone(),
        session_id.clone(),
        sender,
    );
    let startup_messages = vec![json!({
        "type": "session.created",
        "properties": { "sessionID": session_id, "info": session }
    })];
    Ok(StartedOpenCode {
        runtime: OpenCodeRuntime {
            child,
            client,
            base_url,
            directory,
            session_id,
            model,
            variant,
            instructions: request.instructions.map(str::to_owned),
            current_turn: Arc::new(Mutex::new(None)),
            stopped: false,
        },
        reader: ChannelReader::new(receiver),
        startup_messages,
    })
}

fn reserve_port() -> Result<u16, BridgeError> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

fn wait_until_ready(client: &Client, base_url: &str, child: &mut Child) -> Result<(), BridgeError> {
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait()? {
            return Err(BridgeError::Adapter(format!(
                "OpenCode server exited during startup with {status}"
            )));
        }
        if client
            .get(format!("{base_url}/global/health"))
            .send()
            .is_ok_and(|response| response.status().is_success())
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(BridgeError::Adapter(
        "OpenCode server did not become ready".into(),
    ))
}

fn permission_rules(write_mode: Option<WriteMode>) -> Value {
    let mut rules = vec![json!({"permission":"*", "pattern":"*", "action":"allow"})];
    match write_mode {
        Some(WriteMode::ReadOnly) => {
            rules.push(json!({"permission":"edit", "pattern":"*", "action":"deny"}));
            rules.push(json!({"permission":"bash", "pattern":"*", "action":"ask"}));
            rules.push(json!({"permission":"external_directory", "pattern":"*", "action":"deny"}));
        }
        Some(WriteMode::Shared | WriteMode::Isolated) => {
            rules.push(json!({"permission":"edit", "pattern":"*", "action":"ask"}));
            rules.push(json!({"permission":"bash", "pattern":"*", "action":"ask"}));
            rules.push(json!({"permission":"external_directory", "pattern":"*", "action":"ask"}));
        }
        None | Some(WriteMode::Full) => {}
    }
    Value::Array(rules)
}

fn parse_model(value: &str) -> Option<ModelRef> {
    let value = value.trim();
    let (provider_id, model_id) = value.split_once('/')?;
    (!provider_id.is_empty() && !model_id.is_empty()).then(|| ModelRef {
        provider_id: provider_id.to_owned(),
        model_id: model_id.to_owned(),
    })
}

fn endpoint(base_url: &str, path: &str, directory: &str) -> String {
    let mut url = reqwest::Url::parse(&format!("{base_url}{path}"))
        .expect("locally constructed OpenCode URL is valid");
    url.query_pairs_mut().append_pair("directory", directory);
    url.into()
}

fn checked_json(response: Response, action: &str) -> Result<Value, BridgeError> {
    let status = response.status();
    let body = response.text().map_err(|error| {
        BridgeError::Adapter(format!(
            "Cannot read OpenCode response while trying to {action}: {error}"
        ))
    })?;
    if !status.is_success() {
        return Err(BridgeError::Adapter(format!(
            "Failed to {action} ({status}): {body}"
        )));
    }
    serde_json::from_str(&body).map_err(|error| {
        BridgeError::Adapter(format!(
            "Invalid OpenCode response while trying to {action}: {error}"
        ))
    })
}

fn http_error(action: &'static str) -> impl FnOnce(reqwest::Error) -> BridgeError {
    move |error| BridgeError::Adapter(format!("Failed to {action}: {error}"))
}

fn spawn_event_stream(
    client: Client,
    base_url: String,
    directory: String,
    session_id: String,
    sender: mpsc::Sender<String>,
) {
    thread::spawn(move || {
        let Ok(response) = client.get(endpoint(&base_url, "/event", &directory)).send() else {
            return;
        };
        let mut reader = std::io::BufReader::new(response);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let Some(data) = line.trim_end().strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            let Ok(value) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            let belongs_to_session = value
                .pointer("/properties/sessionID")
                .and_then(Value::as_str)
                == Some(session_id.as_str());
            if belongs_to_session && sender.send(format!("{value}\n")).is_err() {
                break;
            }
        }
    });
}

impl OpenCodeRuntime {
    fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        action: &'static str,
    ) -> Result<(), BridgeError> {
        let mut request = self
            .client
            .request(method, endpoint(&self.base_url, path, &self.directory))
            .timeout(Duration::from_secs(10));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().map_err(http_error(action))?;
        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            Err(BridgeError::Adapter(format!(
                "Failed to {action} ({status}): {body}"
            )))
        }
    }

    fn terminate(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        let _ = self.request(
            reqwest::Method::POST,
            "/instance/dispose",
            None,
            "dispose OpenCode server",
        );
        let _ = crate::adapters::terminate_process_group(self.child.id());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl AdapterRuntime for OpenCodeRuntime {
    fn process_id(&self) -> u32 {
        self.child.id()
    }
    fn provider_session_id(&self) -> &str {
        &self.session_id
    }
    fn current_turn(&self) -> Arc<Mutex<Option<String>>> {
        self.current_turn.clone()
    }
    fn send_turn(&self, text: &str) -> Result<(), BridgeError> {
        self.send_turn_with_context(text, "")
    }
    fn send_turn_with_context(
        &self,
        text: &str,
        application_context: &str,
    ) -> Result<(), BridgeError> {
        let mut body = json!({
            "parts": [{"type":"text", "text": text}],
        });
        if let Some(model) = &self.model {
            body["model"] = json!({"providerID": model.provider_id, "modelID": model.model_id});
        }
        if let Some(variant) = &self.variant {
            body["variant"] = json!(variant);
        }
        let system = [
            self.instructions.as_deref().unwrap_or_default(),
            application_context,
        ]
        .into_iter()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
        if !system.is_empty() {
            body["system"] = json!(system);
        }
        self.request(
            reqwest::Method::POST,
            &format!("/session/{}/prompt_async", self.session_id),
            Some(body),
            "send OpenCode turn",
        )
    }
    fn interrupt(&self) -> Result<(), BridgeError> {
        self.request(
            reqwest::Method::POST,
            &format!("/session/{}/abort", self.session_id),
            None,
            "interrupt OpenCode turn",
        )
    }
    fn respond(&self, request_id: Value, decision: &str) -> Result<(), BridgeError> {
        let request_id = request_id
            .as_str()
            .ok_or_else(|| BridgeError::Invalid("OpenCode approval id is invalid".into()))?;
        let reply = match decision {
            "accept" => "once",
            "acceptForSession" => "always",
            "decline" | "cancel" => "reject",
            _ => {
                return Err(BridgeError::Invalid(
                    "Unsupported OpenCode approval decision".into(),
                ))
            }
        };
        self.request(
            reqwest::Method::POST,
            &format!("/permission/{request_id}/reply"),
            Some(json!({"reply": reply})),
            "resolve OpenCode permission",
        )
    }
    fn stop(&mut self, _reason: ShutdownReason) {
        self.terminate();
    }
}

impl Drop for OpenCodeRuntime {
    fn drop(&mut self) {
        self.terminate();
    }
}

pub fn supports_native_resume() -> bool {
    binary_version().as_deref().is_some_and(is_supported_version)
}

pub fn binary_version() -> Option<String> {
    binary::version("opencode")
}

pub fn go_credentials_configured() -> bool {
    let Some(binary) = binary::resolve("opencode") else {
        return false;
    };
    Command::new(binary)
        .args(["auth", "list"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .is_some_and(|output| {
            String::from_utf8_lossy(&output.stdout)
                .to_ascii_lowercase()
                .contains("opencode go")
        })
}

pub fn is_supported_version(version: &str) -> bool {
    let mut parts = version
        .trim()
        .trim_start_matches('v')
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .take(3)
        .filter_map(|part| part.parse::<u64>().ok());
    let parsed = (parts.next(), parts.next(), parts.next());
    matches!(parsed, (Some(major), Some(minor), Some(patch)) if (major, minor, patch) >= MINIMUM_VERSION)
}

fn ensure_supported_version() -> Result<(), BridgeError> {
    let version = binary_version()
        .ok_or_else(|| BridgeError::Invalid("OpenCode binary is not installed".into()))?;
    if is_supported_version(&version) {
        return Ok(());
    }
    Err(BridgeError::Invalid(format!(
        "OpenCode {version} is incompatible with Bridge. Upgrade to OpenCode 1.18.3 or newer."
    )))
}

pub struct ChannelReader {
    receiver: mpsc::Receiver<String>,
    buffer: Vec<u8>,
    position: usize,
}

impl ChannelReader {
    fn new(receiver: mpsc::Receiver<String>) -> Self {
        Self {
            receiver,
            buffer: Vec::new(),
            position: 0,
        }
    }
}

impl Read for ChannelReader {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let available = self.fill_buf()?;
        let count = available.len().min(output.len());
        output[..count].copy_from_slice(&available[..count]);
        self.consume(count);
        Ok(count)
    }
}

impl BufRead for ChannelReader {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        if self.position >= self.buffer.len() {
            match self.receiver.recv() {
                Ok(next) => {
                    self.buffer = next.into_bytes();
                    self.position = 0;
                }
                Err(_) => {
                    self.buffer.clear();
                    self.position = 0;
                }
            }
        }
        Ok(&self.buffer[self.position..])
    }

    fn consume(&mut self, amount: usize) {
        self.position = (self.position + amount).min(self.buffer.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_provider_qualified_models() {
        let model = parse_model("anthropic/claude-sonnet-4").unwrap();
        assert_eq!(model.provider_id, "anthropic");
        assert_eq!(model.model_id, "claude-sonnet-4");
        assert!(parse_model("unqualified").is_none());
    }

    #[test]
    fn rejects_opencode_versions_with_the_incompatible_context_schema() {
        assert!(!is_supported_version("1.17.4"));
        assert!(is_supported_version("1.18.3"));
        assert!(is_supported_version("v1.18.4"));
        assert!(!is_supported_version("unknown"));
    }

    #[test]
    fn worker_permissions_preserve_bridge_write_modes() {
        let has_rule = |rules: &Value, permission: &str, action: &str| {
            rules.as_array().is_some_and(|items| {
                items.iter().any(|item| {
                    item.get("permission").and_then(Value::as_str) == Some(permission)
                        && item.get("action").and_then(Value::as_str) == Some(action)
                })
            })
        };
        let read_only = permission_rules(Some(WriteMode::ReadOnly));
        assert!(has_rule(&read_only, "edit", "deny"));
        let isolated = permission_rules(Some(WriteMode::Isolated));
        assert!(has_rule(&isolated, "edit", "ask"));
    }
}
