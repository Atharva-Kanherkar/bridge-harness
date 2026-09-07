use crate::{
    adapters::{AdapterRuntime, ResumeRequest, ShutdownReason, StartRequest, TurnContext},
    binary,
    context_inventory::{
        AdapterContextInventory, ContextInventoryScope, ContextLifecyclePhase, ContextSegmentClass,
        ContextSegmentObservation,
    },
    delegation::WriteMode,
    model::{AuthState, CapabilityTier, ModelOption},
    BridgeError,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use reqwest::blocking::{Client, Response};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    env,
    io::{BufRead, Read},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const MINIMUM_VERSION: (u64, u64, u64) = (1, 18, 3);

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct OpenCodeSettings {
    pub executable_path: Option<String>,
    pub visible_models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeCatalog {
    pub executable_path: String,
    pub version: String,
    pub providers: Vec<OpenCodeProvider>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeProvider {
    pub id: String,
    pub name: String,
    pub connected: bool,
    pub source: Option<String>,
    pub environment_variables: Vec<String>,
    pub default_model: Option<String>,
    pub auth_methods: Vec<OpenCodeAuthMethod>,
    pub models: Vec<OpenCodeModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeAuthMethod {
    pub kind: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeModel {
    pub id: String,
    pub provider_id: String,
    pub model_id: String,
    pub label: String,
    pub reasoning: bool,
    /// Discovery metadata projected into ModelOption, not the provider settings UI.
    #[serde(skip)]
    pub variants: Vec<String>,
    pub tool_call: bool,
    pub attachment: bool,
    pub context_window: Option<u64>,
    pub output_limit: Option<u64>,
    pub input_cost: Option<f64>,
    pub output_cost: Option<f64>,
}

pub struct OpenCodeRuntime {
    child: Child,
    // Held so the launch record outlives the child and is removed with it.
    _ledger: crate::process_ledger::LaunchGuard,
    stderr_tail: crate::adapters::StderrTail,
    client: Option<Client>,
    base_url: String,
    directory: String,
    session_id: String,
    model: Option<ModelRef>,
    variant: Option<String>,
    instructions: Option<String>,
    context_inventory: Mutex<Vec<AdapterContextInventory>>,
    current_turn: Arc<Mutex<Option<String>>>,
    shutting_down: Arc<AtomicBool>,
    stopped: bool,
    queue_metrics: crate::frame_queue::QueueMetrics,
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

pub fn start_with_settings(
    request: StartRequest<'_>,
    settings: &OpenCodeSettings,
) -> Result<StartedOpenCode, BridgeError> {
    launch(request, None, settings)
}

pub fn resume_with_settings(
    request: ResumeRequest<'_>,
    settings: &OpenCodeSettings,
) -> Result<StartedOpenCode, BridgeError> {
    launch(
        StartRequest {
            cwd: request.cwd,
            model: request.model,
            effort: request.effort,
            instructions: request.instructions,
            write_mode: request.write_mode,
            read_only_sandbox: request.read_only_sandbox,
            briefing: request.briefing,
            on_progress: request.on_progress,
        },
        Some(request.provider_session_id),
        settings,
    )
}

fn launch(
    request: StartRequest<'_>,
    resume_session_id: Option<&str>,
    settings: &OpenCodeSettings,
) -> Result<StartedOpenCode, BridgeError> {
    ensure_read_only_transport_supported(request.read_only_sandbox.is_some())?;
    // OpenCode's permission rules are coarse families, so admitting one reviewed
    // connector read would admit its neighbours. Refused at the boundary with the
    // reason, never accepted-and-ignored.
    if request.briefing.is_some() {
        return Err(BridgeError::Invalid(
            crate::briefing_policy::adapter_may_brief("opencode")
                .err()
                .map(|error| error.reason())
                .unwrap_or_else(|| "OpenCode cannot enforce briefing authority".into()),
        ));
    }
    if let Some(session_id) = resume_session_id {
        validate_path_id("session id", session_id)?;
    }
    let binary = resolve_executable(settings)?;
    ensure_supported_version(&binary)?;
    let port = reserve_port()?;
    let base_url = format!("http://127.0.0.1:{port}");
    let server_password = uuid::Uuid::new_v4().to_string();
    let port_argument = port.to_string();
    let mut command = crate::adapters::supervised_command(
        &binary,
        &["serve", "--hostname", "127.0.0.1", "--port", &port_argument],
    );
    command
        .current_dir(request.cwd)
        .env("OPENCODE_SERVER_USERNAME", "bridge")
        .env("OPENCODE_SERVER_PASSWORD", &server_password)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        // Piped and tail-captured so a dead server reports its own error
        // instead of a generic exit.
        .stderr(Stdio::piped());
    crate::adapters::configure_process_group(&mut command);
    if let Some(on_progress) = request.on_progress {
        on_progress(crate::adapters::StartupPhase::Spawning);
    }
    let spawned_at = std::time::Instant::now();
    let mut child = command.spawn()?;
    let ledger = crate::process_ledger::record_launch("opencode.session", request.cwd, child.id());
    let stderr_tail = crate::adapters::StderrTail::capture(&mut child);
    let client = match build_authenticated_client(&server_password) {
        Ok(client) => client,
        Err(error) => {
            stop_child(&mut child);
            return Err(error);
        }
    };
    if let Some(on_progress) = request.on_progress {
        on_progress(crate::adapters::StartupPhase::Handshake);
    }
    if let Err(error) = wait_until_ready(&client, &base_url, &mut child) {
        stop_child(&mut child);
        drop_client_safely(client);
        return Err(error);
    }
    crate::process_ledger::log_spawn_to_ready("opencode", "server_ready", spawned_at);

    let model = request.model.and_then(parse_model);
    let variant = request
        .effort
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let directory = request.cwd.to_owned();
    let session_result = match resume_session_id {
        Some(session_id) => client
            .get(endpoint(
                &base_url,
                &format!("/session/{session_id}"),
                &directory,
            ))
            .timeout(Duration::from_secs(10))
            .send()
            .map_err(http_error("resume OpenCode session"))
            .and_then(|response| checked_json(response, "resume OpenCode session")),
        None => {
            let body = session_create_body(model.as_ref(), variant.as_deref(), request.write_mode);
            client
                .post(endpoint(&base_url, "/session", &directory))
                .timeout(Duration::from_secs(10))
                .json(&body)
                .send()
                .map_err(http_error("create OpenCode session"))
                .and_then(|response| checked_json(response, "create OpenCode session"))
        }
    };
    let session = match session_result {
        Ok(session) => session,
        Err(error) => {
            stop_child(&mut child);
            drop_client_safely(client);
            return Err(error);
        }
    };
    let session_id = session.get("id").and_then(Value::as_str).map(str::to_owned);
    let Some(session_id) = session_id else {
        stop_child(&mut child);
        drop_client_safely(client);
        return Err(BridgeError::Adapter(format!(
            "OpenCode returned no session id: {session}"
        )));
    };
    if let Err(error) = validate_path_id("session id", &session_id) {
        stop_child(&mut child);
        drop_client_safely(client);
        return Err(error);
    }

    let shutting_down = Arc::new(AtomicBool::new(false));
    let (sender, receiver, queue_metrics) =
        crate::frame_queue::bounded_frame_queue(crate::frame_queue::QueueBudget::default());
    if let Err(error) = spawn_event_stream(
        client.clone(),
        base_url.clone(),
        directory.clone(),
        session_id.clone(),
        sender,
        shutting_down.clone(),
    ) {
        stop_child(&mut child);
        drop_client_safely(client);
        return Err(error);
    }
    if let Some(on_progress) = request.on_progress {
        on_progress(crate::adapters::StartupPhase::SessionOpen);
    }
    let startup_messages = vec![json!({
        "type": "session.created",
        "properties": { "sessionID": session_id, "info": session }
    })];
    Ok(StartedOpenCode {
        runtime: OpenCodeRuntime {
            child,
            _ledger: ledger,
            stderr_tail,
            client: Some(client),
            base_url,
            directory,
            session_id,
            model,
            variant,
            instructions: request.instructions.map(str::to_owned),
            context_inventory: Mutex::new(opencode_context_inventory(
                if resume_session_id.is_some() {
                    ContextLifecyclePhase::Resume
                } else {
                    ContextLifecyclePhase::Start
                },
            )?),
            current_turn: Arc::new(Mutex::new(None)),
            shutting_down,
            stopped: false,
            queue_metrics,
        },
        reader: ChannelReader::new(receiver),
        startup_messages,
    })
}

fn ensure_read_only_transport_supported(enabled: bool) -> Result<(), BridgeError> {
    if enabled {
        Err(BridgeError::Invalid(
            "OpenCode read-only workers are unsupported because its local HTTP transport cannot run inside the offline sandbox; refusing to start without isolation"
                .into(),
        ))
    } else {
        Ok(())
    }
}

fn session_create_body(
    model: Option<&ModelRef>,
    variant: Option<&str>,
    write_mode: Option<WriteMode>,
) -> Value {
    let mut body = json!({
        "title": "Bridge session",
        "permission": permission_rules(write_mode),
    });
    if let Some(model) = model {
        body["model"] = json!({
            "providerID": model.provider_id,
            "id": model.model_id,
        });
        if let Some(variant) = variant {
            body["model"]["variant"] = json!(variant);
        }
    }
    body
}

fn stop_child(child: &mut Child) {
    let _ = crate::adapters::terminate_process_group(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

fn drop_client_safely(client: Client) {
    // reqwest's blocking client owns an internal Tokio runtime and panics when
    // its final handle is dropped from Tauri's async command context.
    let _ = thread::Builder::new()
        .name("opencode-client-drop".into())
        .spawn(move || drop(client));
}

fn reserve_port() -> Result<u16, BridgeError> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

fn build_authenticated_client(server_password: &str) -> Result<Client, BridgeError> {
    let authorization = format!(
        "Basic {}",
        BASE64.encode(format!("bridge:{server_password}"))
    );
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&authorization).map_err(|error| {
            BridgeError::Adapter(format!("Cannot secure OpenCode server: {error}"))
        })?,
    );
    Client::builder()
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(2))
        .timeout(None)
        .build()
        .map_err(|error| BridgeError::Adapter(format!("Cannot create OpenCode client: {error}")))
}

/// The body `/session/{id}/summarize` requires.
///
/// `providerID` and `modelID` are not optional on the wire: the endpoint uses
/// them to pick the model that writes the summary. `auto` stays false, because
/// this request is always something the user asked for by hand.
fn summarize_body(model: &ModelRef) -> Value {
    json!({
        "providerID": model.provider_id,
        "modelID": model.model_id,
        "auto": false,
    })
}

/// Backoff between readiness probes: short while the server is most likely
/// still booting, ramping up to the steady 100ms poll once it has had time to
/// come up. `attempt` is zero-based (the delay taken *after* probe `attempt`).
fn probe_backoff(attempt: usize) -> Duration {
    const RAMP_MS: [u64; 4] = [10, 15, 25, 40];
    Duration::from_millis(RAMP_MS.get(attempt).copied().unwrap_or(100))
}

fn wait_until_ready(client: &Client, base_url: &str, child: &mut Child) -> Result<(), BridgeError> {
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut attempt = 0usize;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait()? {
            return Err(BridgeError::Adapter(format!(
                "OpenCode server exited during startup with {status}"
            )));
        }
        if client
            .get(format!("{base_url}/global/health"))
            .timeout(Duration::from_secs(2))
            .send()
            .is_ok_and(|response| response.status().is_success())
        {
            // The reserved port is released before the child binds it, so another
            // local process can win the race and answer in its place. The real
            // OpenCode child must still be alive and must reject requests that
            // lack this instance's credentials.
            if let Some(status) = child.try_wait()? {
                return Err(BridgeError::Adapter(format!(
                    "OpenCode server exited during startup with {status}"
                )));
            }
            return verify_server_requires_credentials(base_url);
        }
        thread::sleep(probe_backoff(attempt));
        attempt += 1;
    }
    Err(BridgeError::Adapter(
        "OpenCode server did not become ready".into(),
    ))
}

fn verify_server_requires_credentials(base_url: &str) -> Result<(), BridgeError> {
    // Runs on a dedicated thread so the probe client (and its response) are
    // dropped there — see drop_client_safely.
    let health_url = format!("{base_url}/global/health");
    let handle = thread::Builder::new()
        .name("opencode-auth-probe".into())
        .spawn(move || -> Result<(), BridgeError> {
            let probe = Client::builder()
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(2))
                .build()
                .map_err(|error| {
                    BridgeError::Adapter(format!("Cannot create OpenCode probe client: {error}"))
                })?;
            let unauthorized = probe
                .get(&health_url)
                .send()
                .is_ok_and(|response| response.status() == reqwest::StatusCode::UNAUTHORIZED);
            unauthorized.then_some(()).ok_or_else(|| {
                BridgeError::Adapter(
                    "OpenCode server port appears to be claimed by another process: \
                     its health endpoint does not require Bridge credentials"
                        .into(),
                )
            })
        })
        .map_err(|error| BridgeError::Adapter(format!("Cannot probe OpenCode server: {error}")))?;
    handle
        .join()
        .map_err(|_| BridgeError::Adapter("OpenCode credential probe panicked".into()))?
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
    sender: crate::frame_queue::FrameSender,
    shutting_down: Arc<AtomicBool>,
) -> Result<(), BridgeError> {
    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let response = match client.get(endpoint(&base_url, "/event", &directory)).send() {
            Ok(response) if response.status().is_success() => response,
            Ok(response) => {
                let _ = ready_sender.send(Err(BridgeError::Adapter(format!(
                    "Failed to connect to OpenCode event stream ({})",
                    response.status()
                ))));
                return;
            }
            Err(error) => {
                let _ =
                    ready_sender.send(Err(http_error("connect to OpenCode event stream")(error)));
                return;
            }
        };
        if ready_sender.send(Ok(())).is_err() {
            return;
        }
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
            if !belongs_to_session {
                continue;
            }
            // Streaming deltas are the only sheddable frames: their terminal
            // `message.part.updated` carries the complete content. Everything
            // else is durable and back-pressures this socket when the
            // consumer stalls, instead of buffering without bound.
            let transient =
                value.get("type").and_then(Value::as_str) == Some("message.part.delta");
            let frame = format!("{value}\n");
            let delivered = if transient {
                sender.send_transient(frame).map(|_| ())
            } else {
                sender.send_durable(frame)
            };
            if delivered.is_err() {
                break;
            }
        }
        // The stream ended. During shutdown that is expected; otherwise the
        // turn would silently appear finished, so surface the disconnect.
        if !shutting_down.load(Ordering::SeqCst) {
            let error_event = json!({
                "type": "session.error",
                "properties": {
                    "sessionID": session_id,
                    "error": { "message": "OpenCode event stream disconnected unexpectedly" }
                }
            });
            let _ = sender.send_durable(format!("{error_event}\n"));
        }
    });
    ready_receiver
        .recv_timeout(Duration::from_secs(10))
        .map_err(|error| {
            BridgeError::Adapter(format!(
                "Timed out connecting to OpenCode event stream: {error}"
            ))
        })?
}

impl OpenCodeRuntime {
    fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        action: &'static str,
    ) -> Result<(), BridgeError> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| BridgeError::Adapter("OpenCode runtime is stopped".into()))?
            .clone();
        let url = endpoint(&self.base_url, path, &self.directory);
        // Callers include async Tauri commands. Run the blocking HTTP exchange
        // on a dedicated thread so every reqwest temporary (request builder,
        // response, client clone) is created and dropped off the async runtime
        // — dropping the last blocking-client handle there panics.
        let handle = thread::Builder::new()
            .name("opencode-request".into())
            .spawn(move || -> Result<(), BridgeError> {
                let mut request = client.request(method, url).timeout(Duration::from_secs(10));
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
            })
            .map_err(|error| {
                BridgeError::Adapter(format!("Cannot dispatch OpenCode request: {error}"))
            })?;
        handle
            .join()
            .map_err(|_| BridgeError::Adapter(format!("Failed to {action}: request panicked")))?
    }

    fn terminate(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        self.shutting_down.store(true, Ordering::SeqCst);
        // Do not issue a blocking reqwest request here. Tauri may call stop from
        // an async command worker, and reqwest's blocking client owns a Tokio
        // runtime that must never be torn down from an async runtime context.
        // Terminating the dedicated process group disposes this private server.
        let _ = crate::adapters::terminate_process_group(self.child.id());
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(client) = self.client.take() {
            drop_client_safely(client);
        }
    }
}

impl AdapterRuntime for OpenCodeRuntime {
    fn process_id(&self) -> u32 {
        self.child.id()
    }
    fn provider_session_id(&self) -> &str {
        &self.session_id
    }
    fn event_queue_metrics(&self) -> Option<crate::frame_queue::QueueMetricsSnapshot> {
        Some(self.queue_metrics.snapshot())
    }
    fn current_turn(&self) -> Arc<Mutex<Option<String>>> {
        self.current_turn.clone()
    }
    fn context_inventory(&self) -> Vec<AdapterContextInventory> {
        self.context_inventory.lock().unwrap().clone()
    }
    fn send_turn(&self, text: &str) -> Result<(), BridgeError> {
        self.send_turn_with_context(text, TurnContext::default())
    }
    fn send_turn_with_context(
        &self,
        text: &str,
        context: TurnContext<'_>,
    ) -> Result<(), BridgeError> {
        let body = prompt_body(
            self.model.as_ref(),
            self.variant.as_deref(),
            self.instructions.as_deref(),
            text,
            context,
        );
        self.request(
            reqwest::Method::POST,
            &format!("/session/{}/prompt_async", self.session_id),
            Some(body),
            "send OpenCode turn",
        )?;
        crate::context_inventory::record_runtime_inventory(
            &self.context_inventory,
            opencode_context_inventory(ContextLifecyclePhase::PerTurn)?,
        );
        Ok(())
    }
    fn interrupt(&self) -> Result<(), BridgeError> {
        self.request(
            reqwest::Method::POST,
            &format!("/session/{}/abort", self.session_id),
            None,
            "interrupt OpenCode turn",
        )
    }
    /// `/session/{id}/summarize` names the model that writes the summary and
    /// takes no focus, so OpenCode compacts the whole session. Without a known
    /// model there is nothing to name, and the request would be rejected.
    fn native_compaction(&self) -> crate::adapters::NativeCompaction {
        if self.model.is_some() {
            crate::adapters::NativeCompaction::WholeConversation
        } else {
            crate::adapters::NativeCompaction::Unsupported
        }
    }
    fn compact_native(&self, _focus: Option<&str>) -> Result<(), BridgeError> {
        let model = self.model.as_ref().ok_or_else(|| {
            BridgeError::Invalid("OpenCode needs a selected model to summarize".into())
        })?;
        self.request(
            reqwest::Method::POST,
            &format!("/session/{}/summarize", self.session_id),
            Some(summarize_body(model)),
            "compact the OpenCode session",
        )
    }
    fn respond(&self, request_id: Value, decision: &str) -> Result<(), BridgeError> {
        let request_id = request_id
            .as_str()
            .ok_or_else(|| BridgeError::Invalid("OpenCode approval id is invalid".into()))?;
        validate_path_id("approval id", request_id)?;
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
    fn answer_question(&self, request_id: Value, answers: Value) -> Result<(), BridgeError> {
        let request_id = request_id
            .as_str()
            .ok_or_else(|| BridgeError::Invalid("OpenCode question id is invalid".into()))?;
        validate_path_id("question id", request_id)?;
        self.request(
            reqwest::Method::POST,
            &format!("/question/{request_id}/reply"),
            Some(json!({"answers": answers})),
            "answer OpenCode question",
        )
    }
    fn reject_question(&self, request_id: Value) -> Result<(), BridgeError> {
        let request_id = request_id
            .as_str()
            .ok_or_else(|| BridgeError::Invalid("OpenCode question id is invalid".into()))?;
        validate_path_id("question id", request_id)?;
        self.request(
            reqwest::Method::POST,
            &format!("/question/{request_id}/reject"),
            None,
            "reject OpenCode question",
        )
    }
    fn failure_context(&mut self) -> Option<String> {
        crate::adapters::process_failure_context(&mut self.child, &self.stderr_tail)
    }
    fn stop(&mut self, _reason: ShutdownReason) {
        self.terminate();
    }
}

fn prompt_body(
    model: Option<&ModelRef>,
    variant: Option<&str>,
    instructions: Option<&str>,
    text: &str,
    context: TurnContext<'_>,
) -> Value {
    // `system` is rebuilt every turn, so it has to be a function of the launch
    // alone: folding per-turn context in here made a turn carrying a
    // `[secret:]` marker bust its own prefix. Bridge's per-turn words are
    // parts, ahead of the user's text, exactly like every other provider.
    let mut parts = context
        .entries()
        .map(|entry| json!({"type":"text", "text": entry.value}))
        .collect::<Vec<_>>();
    parts.push(json!({"type":"text", "text": text}));
    let mut body = json!({ "parts": parts });
    if let Some(model) = model {
        body["model"] = json!({"providerID": model.provider_id, "modelID": model.model_id});
    }
    if let Some(variant) = variant {
        body["variant"] = json!(variant);
    }
    if let Some(system) = instructions.map(str::trim).filter(|value| !value.is_empty()) {
        body["system"] = json!(system);
    }
    body
}

pub(crate) fn opencode_context_inventory(
    lifecycle_phase: ContextLifecyclePhase,
) -> Result<Vec<AdapterContextInventory>, BridgeError> {
    let observations = || {
        vec![
            ContextSegmentObservation::unavailable(
                ContextSegmentClass::ProviderBaseInstructions,
                match lifecycle_phase {
                    ContextLifecyclePhase::Start => "OpenCode does not report provider base instructions when Bridge creates the session",
                    ContextLifecyclePhase::Resume => "OpenCode does not report provider base instructions retained or recomputed for an adopted session",
                    ContextLifecyclePhase::PerTurn => "The per-turn system value is Bridge-authored; OpenCode does not report additional provider base instructions presented to the turn",
                },
            ),
            ContextSegmentObservation::unavailable(
                ContextSegmentClass::ToolSchemas,
                "OpenCode does not report provider-owned tool schemas presented to the selected model",
            ),
            ContextSegmentObservation::unavailable(
                ContextSegmentClass::McpDynamicTools,
                "OpenCode does not report which MCP or dynamic tools are presented to this turn",
            ),
            ContextSegmentObservation::unavailable(
                ContextSegmentClass::SkillsPlugins,
                "OpenCode does not report which skills or plugins contribute model context",
            ),
            ContextSegmentObservation::unavailable(
                ContextSegmentClass::AgentDefinitions,
                "OpenCode does not report provider-owned agent definitions presented to the model",
            ),
        ]
    };
    let mut inventories = Vec::new();
    if lifecycle_phase != ContextLifecyclePhase::PerTurn {
        inventories.push(AdapterContextInventory::new(
            "opencode",
            ContextInventoryScope::Catalog,
            lifecycle_phase,
            observations(),
        )?);
    }
    inventories.push(AdapterContextInventory::new(
        "opencode",
        ContextInventoryScope::TurnPresented,
        lifecycle_phase,
        observations(),
    )?);
    Ok(inventories)
}

impl Drop for OpenCodeRuntime {
    fn drop(&mut self) {
        self.terminate();
    }
}

pub fn resolve_executable(settings: &OpenCodeSettings) -> Result<PathBuf, BridgeError> {
    // A Bridge-managed payload outranks a copy bundled with the app and anything
    // on PATH, but never the executable the user configured explicitly.
    let mut candidates = Vec::new();
    if let Some(managed) = crate::managed_runtime::managed_entrypoint("opencode") {
        candidates.push(managed);
    }
    candidates.extend(managed_executable_candidates());
    choose_executable(
        settings.executable_path.as_deref(),
        &candidates,
        binary::resolve("opencode"),
    )
}

fn choose_executable(
    explicit: Option<&str>,
    managed: &[PathBuf],
    system: Option<PathBuf>,
) -> Result<PathBuf, BridgeError> {
    if let Some(explicit) = explicit.map(str::trim).filter(|value| !value.is_empty()) {
        let path = PathBuf::from(explicit);
        if is_executable(&path) {
            return Ok(path);
        }
        return Err(BridgeError::Invalid(format!(
            "Configured OpenCode executable is missing or not executable: {}",
            path.display()
        )));
    }

    if let Some(path) = managed.iter().find(|path| is_executable(path)) {
        return Ok(path.clone());
    }
    system
        .filter(|path| is_executable(path))
        .ok_or_else(|| BridgeError::Invalid("OpenCode binary is not installed".into()))
}

fn managed_executable_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = env::var_os("BRIDGE_OPENCODE_SIDECAR") {
        candidates.push(PathBuf::from(path));
    }
    if let Ok(executable) = env::current_exe() {
        if let Some(directory) = executable.parent() {
            candidates.push(directory.join("opencode"));
            candidates.push(directory.join("../Resources/bin/opencode"));
            candidates.push(directory.join("../Resources/opencode"));
        }
    }
    candidates
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return std::fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn binary_version_at(executable: &Path) -> Option<String> {
    let output = Command::new(executable).arg("--version").output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub fn is_supported_version(version: &str) -> bool {
    let mut parts = version
        .trim()
        .trim_start_matches('v')
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .take(3)
        .filter_map(|part| part.parse::<u64>().ok());
    let major = parts.next();
    let minor = parts.next().unwrap_or(0);
    let patch = parts.next().unwrap_or(0);
    matches!(major, Some(major) if (major, minor, patch) >= MINIMUM_VERSION)
}

fn ensure_supported_version(executable: &Path) -> Result<String, BridgeError> {
    let version = binary_version_at(executable).ok_or_else(|| {
        BridgeError::Invalid(format!(
            "Cannot read OpenCode version from {}",
            executable.display()
        ))
    })?;
    if is_supported_version(&version) {
        return Ok(version);
    }
    Err(BridgeError::Invalid(format!(
        "OpenCode {version} is incompatible with Bridge. Upgrade to OpenCode 1.18.3 or newer."
    )))
}

pub fn discover(
    settings: &OpenCodeSettings,
    directory: &str,
) -> Result<OpenCodeCatalog, BridgeError> {
    let executable = resolve_executable(settings)?;
    let version = ensure_supported_version(&executable)?;
    with_control_server(&executable, directory, |client, base_url| {
        let providers = checked_json(
            client
                .get(endpoint(base_url, "/provider", directory))
                .timeout(Duration::from_secs(20))
                .send()
                .map_err(http_error("discover OpenCode providers"))?,
            "discover OpenCode providers",
        )?;
        let auth_methods = checked_json(
            client
                .get(endpoint(base_url, "/provider/auth", directory))
                .timeout(Duration::from_secs(20))
                .send()
                .map_err(http_error("discover OpenCode authentication methods"))?,
            "discover OpenCode authentication methods",
        )?;
        parse_catalog(
            &providers,
            &auth_methods,
            executable.to_string_lossy().as_ref(),
            &version,
        )
    })
}

pub fn set_provider_api_key(
    settings: &OpenCodeSettings,
    directory: &str,
    provider_id: &str,
    api_key: &str,
) -> Result<OpenCodeCatalog, BridgeError> {
    validate_provider_id(provider_id)?;
    if api_key.trim().is_empty() {
        return Err(BridgeError::Invalid(
            "Provider API key cannot be empty".into(),
        ));
    }
    let executable = resolve_executable(settings)?;
    ensure_supported_version(&executable)?;
    with_control_server(&executable, directory, |client, base_url| {
        checked_json(
            client
                .put(format!("{base_url}/auth/{provider_id}"))
                .timeout(Duration::from_secs(20))
                .json(&json!({"type":"api", "key":api_key.trim()}))
                .send()
                .map_err(http_error("save OpenCode provider authentication"))?,
            "save OpenCode provider authentication",
        )?;
        Ok(())
    })?;
    discover(settings, directory)
}

pub fn remove_provider_auth(
    settings: &OpenCodeSettings,
    directory: &str,
    provider_id: &str,
) -> Result<OpenCodeCatalog, BridgeError> {
    validate_provider_id(provider_id)?;
    let executable = resolve_executable(settings)?;
    ensure_supported_version(&executable)?;
    with_control_server(&executable, directory, |client, base_url| {
        checked_json(
            client
                .delete(format!("{base_url}/auth/{provider_id}"))
                .timeout(Duration::from_secs(20))
                .send()
                .map_err(http_error("remove OpenCode provider authentication"))?,
            "remove OpenCode provider authentication",
        )?;
        Ok(())
    })?;
    discover(settings, directory)
}

fn validate_provider_id(provider_id: &str) -> Result<(), BridgeError> {
    let valid = !provider_id.is_empty()
        && provider_id.len() <= 128
        && provider_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    valid.then_some(()).ok_or_else(|| {
        BridgeError::Invalid("OpenCode provider id contains unsupported characters".into())
    })
}

/// Identifiers interpolated into OpenCode URL paths (session ids, permission
/// request ids) must stay within a safe charset so they can neither break URL
/// parsing nor redirect the request to a different endpoint.
fn validate_path_id(kind: &str, value: &str) -> Result<(), BridgeError> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    valid.then_some(()).ok_or_else(|| {
        BridgeError::Invalid(format!("OpenCode {kind} contains unsupported characters"))
    })
}

/// Owns a short-lived control server: the child dies and its launch record
/// clears on every exit from the scope, unwinding included, instead of only on
/// the straight-line return path.
struct ControlServer {
    child: Child,
    _ledger: crate::process_ledger::LaunchGuard,
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        stop_child(&mut self.child);
    }
}

fn with_control_server<T>(
    executable: &Path,
    directory: &str,
    action: impl FnOnce(&Client, &str) -> Result<T, BridgeError>,
) -> Result<T, BridgeError> {
    let port = reserve_port()?;
    let base_url = format!("http://127.0.0.1:{port}");
    let password = uuid::Uuid::new_v4().to_string();
    let port_argument = port.to_string();
    let mut command = crate::adapters::supervised_command(
        executable,
        &["serve", "--hostname", "127.0.0.1", "--port", &port_argument],
    );
    command
        .current_dir(directory)
        .env("OPENCODE_SERVER_USERNAME", "bridge")
        .env("OPENCODE_SERVER_PASSWORD", &password)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::adapters::configure_process_group(&mut command);
    let child = command.spawn()?;
    let ledger = crate::process_ledger::record_launch("opencode.control", directory, child.id());
    let mut server = ControlServer {
        child,
        _ledger: ledger,
    };
    match build_authenticated_client(&password) {
        Ok(client) => {
            let result = wait_until_ready(&client, &base_url, &mut server.child)
                .and_then(|_| action(&client, &base_url));
            drop_client_safely(client);
            result
        }
        Err(error) => Err(error),
    }
}

fn parse_catalog(
    providers: &Value,
    auth_methods: &Value,
    executable_path: &str,
    version: &str,
) -> Result<OpenCodeCatalog, BridgeError> {
    let connected = providers
        .get("connected")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            BridgeError::Adapter("OpenCode provider catalog has no connected list".into())
        })?
        .iter()
        .filter_map(Value::as_str)
        .collect::<HashSet<_>>();
    let defaults = providers
        .get("default")
        .and_then(Value::as_object)
        .ok_or_else(|| BridgeError::Adapter("OpenCode provider catalog has no defaults".into()))?;
    let all = providers
        .get("all")
        .and_then(Value::as_array)
        .ok_or_else(|| BridgeError::Adapter("OpenCode provider catalog has no providers".into()))?;
    let auth = auth_methods.as_object();
    let mut normalized = all
        .iter()
        .filter_map(|provider| {
            let id = provider.get("id")?.as_str()?.to_owned();
            let is_connected = connected.contains(id.as_str());
            let mut models = if is_connected {
                provider
                    .get("models")
                    .and_then(Value::as_object)
                    .into_iter()
                    .flat_map(|models| models.values())
                    .filter_map(|model| normalize_model(&id, model))
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            models.sort_by(|left, right| left.id.cmp(&right.id));
            let methods = auth
                .and_then(|items| items.get(&id))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|method| {
                    Some(OpenCodeAuthMethod {
                        kind: method.get("type")?.as_str()?.to_owned(),
                        label: method.get("label")?.as_str()?.to_owned(),
                    })
                })
                .collect();
            Some(OpenCodeProvider {
                name: provider
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(&id)
                    .to_owned(),
                connected: is_connected,
                source: provider
                    .get("source")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                environment_variables: provider
                    .get("env")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect(),
                default_model: defaults
                    .get(&id)
                    .and_then(Value::as_str)
                    .map(|model| format!("{id}/{model}")),
                auth_methods: methods,
                models,
                id,
            })
        })
        .collect::<Vec<_>>();
    normalized.sort_by(|left, right| {
        right
            .connected
            .cmp(&left.connected)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(OpenCodeCatalog {
        executable_path: executable_path.to_owned(),
        version: version.to_owned(),
        providers: normalized,
    })
}

fn normalize_model(provider_id: &str, model: &Value) -> Option<OpenCodeModel> {
    let model_id = model.get("id")?.as_str()?.to_owned();
    let id = format!("{provider_id}/{model_id}");
    Some(OpenCodeModel {
        label: model
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(&model_id)
            .to_owned(),
        reasoning: model
            .pointer("/capabilities/reasoning")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        variants: model.get("variants").and_then(Value::as_object)
            .map(|variants| variants.iter()
                .filter(|(name, options)| !name.trim().is_empty() && options.get("disabled").and_then(Value::as_bool) != Some(true))
                .map(|(name, _)| name.clone()).collect())
            .unwrap_or_default(),
        tool_call: model
            .pointer("/capabilities/toolcall")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        attachment: model
            .pointer("/capabilities/attachment")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        context_window: model.pointer("/limit/context").and_then(Value::as_u64),
        output_limit: model.pointer("/limit/output").and_then(Value::as_u64),
        input_cost: model.pointer("/cost/input").and_then(Value::as_f64),
        output_cost: model.pointer("/cost/output").and_then(Value::as_f64),
        provider_id: provider_id.to_owned(),
        model_id,
        id,
    })
}

pub fn model_options(catalog: &OpenCodeCatalog, visible_models: &[String]) -> Vec<ModelOption> {
    let visible = visible_models
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut models = catalog
        .providers
        .iter()
        .filter(|provider| provider.connected)
        .flat_map(|provider| provider.models.iter())
        .filter(|model| visible.is_empty() || visible.contains(model.id.as_str()))
        .collect::<Vec<_>>();
    models.sort_by(|left, right| {
        model_strength(left)
            .total_cmp(&model_strength(right))
            .then_with(|| left.id.cmp(&right.id))
    });
    let count = models.len();
    let mut options = models
        .into_iter()
        .enumerate()
        .map(|(index, model)| ModelOption {
            id: model.id.clone(),
            label: format!(
                "{} · {}",
                provider_name(catalog, &model.provider_id),
                model.label
            ),
            tier: if count == 1 {
                CapabilityTier::Standard
            } else if count == 2 {
                if index == 0 {
                    CapabilityTier::Fast
                } else {
                    CapabilityTier::Strong
                }
            } else if index < count / 3 {
                CapabilityTier::Fast
            } else if index >= (count * 2) / 3 {
                CapabilityTier::Strong
            } else {
                CapabilityTier::Standard
            },
            available: true,
            compatible: model.tool_call,
            lifecycle: crate::model::ModelLifecycle::Unknown,
            source: crate::model::ModelCatalogSource::RuntimeApi,
            supported_effort_levels: model.variants.clone(),
            default_for_tier: false,
        })
        .collect::<Vec<_>>();
    let provider_defaults = catalog
        .providers
        .iter()
        .filter_map(|provider| provider.default_model.as_deref())
        .collect::<HashSet<_>>();
    for option in &mut options {
        if provider_defaults.contains(option.id.as_str()) {
            option.lifecycle = crate::model::ModelLifecycle::Stable;
        }
    }
    for tier in [
        CapabilityTier::Fast,
        CapabilityTier::Standard,
        CapabilityTier::Strong,
    ] {
        let preferred = options
            .iter()
            .position(|option| {
                option.tier == tier && provider_defaults.contains(option.id.as_str())
            })
            .or_else(|| options.iter().position(|option| option.tier == tier));
        if let Some(index) = preferred {
            options[index].default_for_tier = true;
        }
    }
    options
}

fn provider_name<'a>(catalog: &'a OpenCodeCatalog, provider_id: &'a str) -> &'a str {
    catalog
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .map(|provider| provider.name.as_str())
        .unwrap_or(provider_id)
}

fn model_strength(model: &OpenCodeModel) -> f64 {
    let name = format!("{} {}", model.id, model.label).to_ascii_lowercase();
    let keyword_score = [
        ("nano", -4.0),
        ("mini", -3.0),
        ("flash", -2.0),
        ("haiku", -2.0),
        ("small", -2.0),
        ("pro", 2.0),
        ("max", 3.0),
        ("opus", 3.0),
        ("ultra", 3.0),
        ("reasoning", 2.0),
        ("thinking", 2.0),
    ]
    .into_iter()
    .filter(|(keyword, _)| name.contains(keyword))
    .map(|(_, score)| score)
    .sum::<f64>();
    keyword_score + model.output_cost.unwrap_or_default().max(0.0).ln_1p()
}

pub struct ChannelReader {
    receiver: crate::frame_queue::FrameReceiver,
    buffer: Vec<u8>,
    position: usize,
}

impl ChannelReader {
    fn new(receiver: crate::frame_queue::FrameReceiver) -> Self {
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

/// Whether OpenCode's own auth store (`auth.json` in its XDG data directory —
/// `$XDG_DATA_HOME/opencode`, falling back to `~/.local/share/opencode`) has
/// a saved credential. A presence + non-empty parse check only.
pub fn auth_state() -> AuthState {
    auth_state_from_data_dir(opencode_data_dir())
}

fn opencode_data_dir() -> Option<PathBuf> {
    if let Some(xdg_data_home) = env::var_os("XDG_DATA_HOME") {
        return Some(PathBuf::from(xdg_data_home).join("opencode"));
    }
    env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share/opencode"))
}

fn auth_state_from_data_dir(dir: Option<PathBuf>) -> AuthState {
    let Some(dir) = dir else {
        return AuthState::Unknown;
    };
    // Metadata only: Bridge never opens or parses credential contents.
    let Ok(metadata) = std::fs::metadata(dir.join("auth.json")) else {
        return AuthState::SignedOut;
    };
    if metadata.len() > 0 {
        AuthState::SignedIn
    } else {
        AuthState::SignedOut
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_inventory::ContextObservationProvenance;

    /// G9: the turn's `system` value must be a function of the launch alone.
    /// It is rebuilt on every turn, so anything per-turn folded into it — a
    /// credential contract for a `[secret:]` marker, say — invalidated that
    /// turn's prefix by itself.
    #[test]
    fn summarize_names_the_model_that_writes_the_summary() {
        // `providerID` and `modelID` are required on the wire. `auto` stays
        // false: this request only ever exists because a user asked by hand.
        let body = summarize_body(&ModelRef {
            provider_id: "anthropic".into(),
            model_id: "claude-sonnet-4-5".into(),
        });
        assert_eq!(body["providerID"], "anthropic");
        assert_eq!(body["modelID"], "claude-sonnet-4-5");
        assert_eq!(body["auto"], false);
        assert!(body.get("focus").is_none(), "the endpoint takes no focus");
    }

    #[test]
    fn the_system_value_is_identical_with_and_without_turn_context() {
        let body = |context| {
            prompt_body(
                None,
                None,
                Some("bridge instructions"),
                "verify [secret:sec_reference]",
                context,
            )
        };
        let plain = body(TurnContext::default());
        let with_context = body(TurnContext {
            session: Some("<bridge-session-context schema=\"1\">frame</bridge-session-context>"),
            credentials: Some("trusted broker capability"),
        });
        assert_eq!(plain["system"], with_context["system"]);
        assert_eq!(with_context["system"], "bridge instructions");

        // The context went to the parts instead, ahead of the user's text and
        // without changing it.
        let parts = with_context["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 3);
        assert_eq!(
            parts[0]["text"],
            "<bridge-session-context schema=\"1\">frame</bridge-session-context>"
        );
        assert_eq!(parts[1]["text"], "trusted broker capability");
        assert_eq!(parts[2]["text"], "verify [secret:sec_reference]");
        assert_eq!(plain["parts"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn readiness_probe_backoff_ramps_then_flattens() {
        let observed: Vec<u64> = (0..7).map(|attempt| probe_backoff(attempt).as_millis() as u64).collect();
        assert_eq!(observed, vec![10, 15, 25, 40, 100, 100, 100]);
    }

    #[test]
    fn opencode_context_inventory_covers_start_resume_and_per_turn() {
        let body = prompt_body(
            None,
            None,
            Some("bridge instructions"),
            "hello",
            TurnContext::default(),
        );
        assert_eq!(body["system"], "bridge instructions");
        for phase in [
            ContextLifecyclePhase::Start,
            ContextLifecyclePhase::Resume,
            ContextLifecyclePhase::PerTurn,
        ] {
            let inventories = opencode_context_inventory(phase).unwrap();
            assert!(inventories
                .iter()
                .any(|item| item.scope == ContextInventoryScope::TurnPresented));
            assert!(inventories.iter().flat_map(|item| &item.observations).all(
                |observation| matches!(
                    observation.provenance,
                    ContextObservationProvenance::Unavailable { ref reason }
                        if !reason.is_empty()
                )
            ));
        }
    }

    fn catalog_fixture() -> OpenCodeCatalog {
        let providers = json!({
            "connected": ["opencode-go"],
            "default": {"opencode-go":"standard", "other":"hidden"},
            "all": [
                {
                    "id":"opencode-go", "name":"OpenCode Go", "source":"api",
                    "env":["OPENCODE_API_KEY"],
                    "models": {
                        "flash":{"id":"flash", "name":"Flash", "capabilities":{"reasoning":true,"toolcall":true,"attachment":false}, "limit":{"context":1000,"output":100}, "cost":{"input":0.1,"output":0.2}},
                        "standard":{"id":"standard", "name":"Standard", "capabilities":{"reasoning":true,"toolcall":true,"attachment":true}, "limit":{"context":2000,"output":200}, "cost":{"input":1.0,"output":2.0}},
                        "pro":{"id":"pro", "name":"Pro", "capabilities":{"reasoning":true,"toolcall":true,"attachment":true}, "limit":{"context":3000,"output":300}, "cost":{"input":3.0,"output":6.0}}
                    }
                },
                {"id":"other", "name":"Other", "source":"env", "env":["OTHER_KEY"], "models":{"hidden":{"id":"hidden","name":"Hidden"}}}
            ]
        });
        let auth = json!({
            "opencode-go":[{"type":"api","label":"API key","prompts":[]}],
            "other":[{"type":"oauth","label":"OAuth","prompts":[]}]
        });
        parse_catalog(&providers, &auth, "/managed/opencode", "1.18.3").unwrap()
    }

    #[test]
    fn parses_provider_qualified_models() {
        let model = parse_model("anthropic/claude-sonnet-4").unwrap();
        assert_eq!(model.provider_id, "anthropic");
        assert_eq!(model.model_id, "claude-sonnet-4");
        assert!(parse_model("unqualified").is_none());
    }

    #[test]
    fn session_creation_omits_an_absent_model_variant() {
        let model = parse_model("opencode-go/kimi-k2.7-code").unwrap();
        let automatic = session_create_body(Some(&model), None, None);
        assert_eq!(automatic["model"]["providerID"], "opencode-go");
        assert_eq!(automatic["model"]["id"], "kimi-k2.7-code");
        assert!(automatic["model"].get("variant").is_none());

        let configured = session_create_body(Some(&model), Some("high"), None);
        assert_eq!(configured["model"]["variant"], "high");
    }

    #[test]
    fn rejects_opencode_versions_with_the_incompatible_context_schema() {
        assert!(!is_supported_version("1.17.4"));
        assert!(is_supported_version("1.18.3"));
        assert!(is_supported_version("v1.18.4"));
        assert!(!is_supported_version("unknown"));
        assert!(is_supported_version("1.19"));
        assert!(is_supported_version("2.0"));
        assert!(!is_supported_version("1.18"));
    }

    #[test]
    fn path_ids_reject_url_breaking_characters() {
        assert!(validate_path_id("session id", "ses_01ABC-def.2").is_ok());
        assert!(validate_path_id("session id", "").is_err());
        assert!(validate_path_id("session id", "ses/../auth").is_err());
        assert!(validate_path_id("session id", "ses?x=1").is_err());
        assert!(validate_path_id("session id", "ses id").is_err());
        assert!(validate_path_id("session id", &"a".repeat(129)).is_err());
    }

    #[test]
    fn read_only_transport_fails_closed_before_launch() {
        let error = ensure_read_only_transport_supported(true).unwrap_err();
        assert!(error
            .to_string()
            .contains("refusing to start without isolation"));
        assert!(ensure_read_only_transport_supported(false).is_ok());
    }

    #[test]
    #[cfg(unix)]
    fn resolves_custom_managed_and_system_executables_in_order() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let make_executable = |name: &str| {
            let path = directory.path().join(name);
            std::fs::write(&path, "#!/bin/sh\n").unwrap();
            let mut permissions = std::fs::metadata(&path).unwrap().permissions();
            permissions.set_mode(0o700);
            std::fs::set_permissions(&path, permissions).unwrap();
            path
        };
        let custom = make_executable("custom");
        let managed = make_executable("managed");
        let system = make_executable("system");
        assert_eq!(
            choose_executable(custom.to_str(), &[managed.clone()], Some(system.clone())).unwrap(),
            custom
        );
        assert_eq!(
            choose_executable(None, &[managed.clone()], Some(system.clone())).unwrap(),
            managed
        );
        assert_eq!(
            choose_executable(None, &[], Some(system.clone())).unwrap(),
            system
        );
        assert!(choose_executable(Some("/missing/opencode"), &[], None).is_err());
    }

    #[test]
    fn parses_structured_provider_catalog_without_credentials() {
        let catalog = catalog_fixture();
        assert_eq!(catalog.executable_path, "/managed/opencode");
        assert_eq!(catalog.providers[0].id, "opencode-go");
        assert!(catalog.providers[0].connected);
        assert_eq!(catalog.providers[0].models.len(), 3);
        assert_eq!(
            catalog.providers[0].default_model.as_deref(),
            Some("opencode-go/standard")
        );
        assert_eq!(catalog.providers[0].auth_methods[0].label, "API key");
        assert!(!catalog.providers[1].connected);
        assert!(catalog.providers[1].models.is_empty());
    }

    #[test]
    fn filters_visible_models_and_preserves_qualified_ids() {
        let catalog = catalog_fixture();
        let options = model_options(
            &catalog,
            &["opencode-go/standard".into(), "other/hidden".into()],
        );
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].id, "opencode-go/standard");
    }

    #[test]
    fn assigns_deterministic_bridge_tiers_to_discovered_models() {
        let catalog = catalog_fixture();
        let first = model_options(&catalog, &[]);
        let second = model_options(&catalog, &[]);
        assert_eq!(
            first
                .iter()
                .map(|model| (&model.id, model.tier))
                .collect::<Vec<_>>(),
            second
                .iter()
                .map(|model| (&model.id, model.tier))
                .collect::<Vec<_>>()
        );
        for tier in [
            CapabilityTier::Fast,
            CapabilityTier::Standard,
            CapabilityTier::Strong,
        ] {
            assert_eq!(first.iter().filter(|model| model.tier == tier).count(), 1);
            assert_eq!(
                first
                    .iter()
                    .filter(|model| model.tier == tier && model.default_for_tier)
                    .count(),
                1
            );
        }
    }

    #[test]
    #[ignore = "requires OpenCode 1.18.3+ with an authenticated OpenCode Go subscription"]
    fn live_discovery_reads_opencode_go_from_the_structured_provider_api() {
        let executable = std::env::var("BRIDGE_OPENCODE_LIVE_BINARY")
            .expect("set BRIDGE_OPENCODE_LIVE_BINARY to the OpenCode executable");
        let directory = std::env::current_dir().unwrap();
        let catalog = discover(
            &OpenCodeSettings {
                executable_path: Some(executable),
                visible_models: Vec::new(),
            },
            directory.to_str().unwrap(),
        )
        .unwrap();
        let provider = catalog
            .providers
            .iter()
            .find(|provider| provider.id == "opencode-go")
            .expect("OpenCode did not report its Go provider");
        assert!(provider.connected);
        assert!(!provider.models.is_empty());
        assert!(provider
            .models
            .iter()
            .all(|model| model.id.starts_with("opencode-go/")));
    }

    #[test]
    #[ignore = "requires OpenCode 1.18.3+ with an authenticated OpenCode Go subscription"]
    fn live_chat_streams_an_opencode_go_reply() {
        let executable = std::env::var("BRIDGE_OPENCODE_LIVE_BINARY")
            .expect("set BRIDGE_OPENCODE_LIVE_BINARY to the OpenCode executable");
        let directory = tempfile::tempdir().unwrap();
        let mut started = start_with_settings(
            StartRequest {
                cwd: directory.path().to_str().unwrap(),
                model: Some("opencode-go/glm-5.2"),
                effort: None,
                instructions: None,
                write_mode: None,
                read_only_sandbox: None,
                briefing: None,
                on_progress: None,
            },
            &OpenCodeSettings {
                executable_path: Some(executable),
                visible_models: Vec::new(),
            },
        )
        .unwrap();
        let process_id = started.runtime.process_id();
        started
            .runtime
            .send_turn("Reply exactly: BRIDGE_OPENCODE_CHAT_OK")
            .unwrap();

        let mut reader = started.reader;
        let (sender, receiver) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let mut state = crate::agent::OpenCodeStreamState::default();
            let mut assistant_text = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or_default() == 0 {
                    break;
                }
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                for event in crate::agent::normalize_opencode_message_with_state(&value, &mut state)
                {
                    if event.kind == "message.delta" {
                        assistant_text.push_str(event.text.as_deref().unwrap_or_default());
                    }
                }
                if assistant_text.contains("BRIDGE_OPENCODE_CHAT_OK") {
                    let _ = sender.send(());
                    break;
                }
            }
        });
        receiver
            .recv_timeout(Duration::from_secs(60))
            .expect("OpenCode Go did not stream the reply");
        started.runtime.stop(ShutdownReason::Completed);
        assert!(crate::adapters::process_identity(process_id).is_none());
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

    #[test]
    fn auth_probe_reports_signed_in_for_any_nonempty_store_without_reading_contents() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("auth.json"), "{not valid json").unwrap();
        assert_eq!(
            auth_state_from_data_dir(Some(dir.path().to_path_buf())),
            AuthState::SignedIn
        );
    }

    #[test]
    fn auth_probe_reports_signed_out_when_cli_present_but_store_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            auth_state_from_data_dir(Some(dir.path().to_path_buf())),
            AuthState::SignedOut
        );
    }

    #[test]
    fn auth_probe_reports_signed_out_when_store_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("auth.json"), "").unwrap();
        assert_eq!(
            auth_state_from_data_dir(Some(dir.path().to_path_buf())),
            AuthState::SignedOut
        );
    }

    #[test]
    fn auth_probe_reports_unknown_when_data_dir_is_missing() {
        assert_eq!(auth_state_from_data_dir(None), AuthState::Unknown);
    }
}

#[cfg(test)]
mod model_variant_tests {
    use super::*;
    #[test]
    fn only_advertised_enabled_variants_are_exposed() {
        let model = normalize_model("test", &json!({"id":"model","variants":{"low":{},"ultra":{},"disabled":{"disabled":true}}})).unwrap();
        assert_eq!(model.variants, ["low", "ultra"]);
        assert!(normalize_model("test", &json!({"id":"plain"})).unwrap().variants.is_empty());
    }
}
