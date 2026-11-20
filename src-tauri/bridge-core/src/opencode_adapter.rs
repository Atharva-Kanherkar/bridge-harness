use crate::{
    agent,
    adapters::{AdapterRuntime, ResumeRequest, ShutdownReason, StartRequest, TurnContext},
    tool_selection::ToolLaunch,
    binary,
    context_inventory::{
        AdapterContextInventory, ContextInventoryScope, ContextLifecyclePhase, ContextSegmentClass,
        ContextSegmentObservation,
    },
    delegation::WriteMode,
    diagnostics,
    model::{AuthState, CapabilityTier, ModelOption},
    opencode_v2, BridgeError,
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

/// Which server API an `opencode serve` speaks. 2.x moved every route under
/// `/api`, fixed the basic-auth user, and replaced the event stream; the
/// translation lives in `opencode_v2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServerApi {
    V1,
    V2,
}

impl ServerApi {
    fn of(version: &str) -> Self {
        match parse_version(version) {
            Some((major, _, _)) if major >= 2 => Self::V2,
            _ => Self::V1,
        }
    }

    /// 1.x takes the user from `OPENCODE_SERVER_USERNAME`; 2.x ignores it.
    fn username(self) -> &'static str {
        match self {
            Self::V1 => "bridge",
            Self::V2 => "opencode",
        }
    }

    /// A route that answers only an authenticated caller. 2.x serves its web
    /// app on every other path, so only an `/api` route proves the server.
    fn health_path(self) -> &'static str {
        match self {
            Self::V1 => "/global/health",
            Self::V2 => "/api/info",
        }
    }

    /// 1.x scopes a request with `?directory=`; 2.x reads the
    /// `x-opencode-directory` header the client sends on every request.
    fn url(self, base_url: &str, path: &str, directory: &str) -> String {
        match self {
            Self::V1 => endpoint(base_url, path, directory),
            Self::V2 => format!("{base_url}{path}"),
        }
    }
}

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
    api: ServerApi,
    /// Requests a 2.x server is waiting on, by id; see `opencode_v2`.
    pending: opencode_v2::PendingRequests,
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
    launch(request, None, settings, SessionKind::Chat, None)
}

/// `start_with_settings` for an interactive chat, with the tool launch of
/// the chat.
pub fn start_chat_with_settings(
    request: StartRequest<'_>,
    settings: &OpenCodeSettings,
    tools: Option<&ToolLaunch>,
) -> Result<StartedOpenCode, BridgeError> {
    launch(request, None, settings, SessionKind::Chat, tools)
}

/// A standalone draft completion, with every provider tool family denied.
/// This does not claim the read-only worker sandbox: the session has no
/// authority to read, write or run tools in the first place.
pub fn start_completion_with_settings(
    request: StartRequest<'_>,
    settings: &OpenCodeSettings,
) -> Result<StartedOpenCode, BridgeError> {
    launch(request, None, settings, SessionKind::Completion, None)
}

/// A personal-app courier: a session whose only authority is to ask for one
/// MCP tool, `permission` (OpenCode's `<server>_<tool>` name). Everything else
/// is denied except `execute`, the code-mode tool OpenCode 2 runs MCP calls
/// in; each MCP call inside it still raises its own permission prompt, which
/// Bridge answers. No Bridge tools are attached.
pub fn start_courier_with_settings(
    request: StartRequest<'_>,
    settings: &OpenCodeSettings,
    permission: &str,
    server: &str,
) -> Result<StartedOpenCode, BridgeError> {
    launch(request, None, settings, SessionKind::Courier { permission, server }, None)
}

#[derive(Clone, Copy)]
enum SessionKind<'a> {
    Chat,
    Completion,
    Courier { permission: &'a str, server: &'a str },
}

pub fn resume_with_settings(
    request: ResumeRequest<'_>,
    settings: &OpenCodeSettings,
) -> Result<StartedOpenCode, BridgeError> {
    launch(
        request.as_start(),
        Some(request.provider_session_id),
        settings,
        SessionKind::Chat,
        None,
    )
}

/// `resume_with_settings` for an interactive chat, with the tool launch of
/// the chat.
pub fn resume_chat_with_settings(
    request: ResumeRequest<'_>,
    settings: &OpenCodeSettings,
    tools: Option<&ToolLaunch>,
) -> Result<StartedOpenCode, BridgeError> {
    launch(request.as_start(), Some(request.provider_session_id), settings, SessionKind::Chat, tools)
}

/// The `OPENCODE_CONFIG_CONTENT` a session runs with: unchanged for a worker,
/// and with Bridge's `visualize` server merged in for a chat that has it.
/// After that merge, each server that the tool choice of the chat leaves out
/// gets `enabled: false`, also when no `visualize` server attaches. When the
/// inherited value does not parse, the choice is not applied and the value
/// of today stays.
fn chat_config_content(
    visualize: Option<&crate::mcp_apps::ServerLaunch>,
    existing: Option<&str>,
    tools: Option<&ToolLaunch>,
) -> Option<String> {
    use crate::tool_selection::{merge_opencode, OpenCodeMerge};
    let today = visualize.map(|launch| crate::mcp_apps::merge_opencode_config(existing, launch));
    let off = tools.filter(|tools| tools.harness == "opencode").map_or(&[][..], |tools| tools.servers_off.as_slice());
    if off.is_empty() {
        return today;
    }
    let OpenCodeMerge::Applied(alone) = merge_opencode(existing, off) else { return today };
    let Some(today) = today else { return Some(alone) };
    match merge_opencode(Some(&today), off) {
        OpenCodeMerge::Applied(content) => Some(content),
        OpenCodeMerge::NotApplied => Some(today),
    }
}

/// Set the config content of a session on its command. With no content the
/// command sets nothing, so the server inherits the value of Bridge as is.
fn apply_chat_config(
    command: &mut Command,
    visualize: Option<&crate::mcp_apps::ServerLaunch>,
    existing: Option<&str>,
    tools: Option<&ToolLaunch>,
) {
    if let Some(config) = chat_config_content(visualize, existing, tools) {
        command.env("OPENCODE_CONFIG_CONTENT", config);
    }
}

fn launch(
    request: StartRequest<'_>,
    resume_session_id: Option<&str>,
    settings: &OpenCodeSettings,
    kind: SessionKind<'_>,
    tools: Option<&ToolLaunch>,
) -> Result<StartedOpenCode, BridgeError> {
    ensure_read_only_transport_supported(request.read_only_sandbox)?;
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
    let api = ServerApi::of(&ensure_supported_version(&binary)?);
    ensure_kind_supported(kind, api)?;
    let port = reserve_port()?;
    let base_url = format!("http://127.0.0.1:{port}");
    let server_password = uuid::Uuid::new_v4().to_string();
    let port_argument = port.to_string();
    // A read-only worker's server runs inside the platform sandbox, the same
    // boundary Claude and Codex workers get; without one this is the plain
    // supervised command.
    let mut command = crate::worker_sandbox::command(&binary, request.read_only_sandbox)?;
    command.args(["serve", "--hostname", "127.0.0.1", "--port", &port_argument]);
    binary::hydrate_command_path(&mut command);
    let sandbox_homes = request
        .read_only_sandbox
        .map(|sandbox| SandboxHomes::under(sandbox.output_dir()));
    let host_data = host_opencode_data(request.account);
    if let Some(homes) = &sandbox_homes {
        homes.create()?;
        // 1.x reads its logins from a file at startup; 2.x's live in the
        // database its server creates, seeded once it is up (below).
        if api == ServerApi::V1 {
            if let Some(host) = &host_data {
                seed_legacy_auth(host, &homes.opencode_data(), now_ms())?;
            }
        }
    }
    // An interactive chat gets Bridge's `visualize` server, merged into
    // whatever inline config the user already passes OpenCode.
    let visualize = (matches!(kind, SessionKind::Chat)
        && crate::mcp_apps::attaches_to(request.interactive, request.write_mode, request.read_only_sandbox, request.briefing))
        .then(|| crate::mcp_apps::ServerLaunch::for_session(request.session_id.filter(|_| request.interactive)))
        .flatten();
    apply_chat_config(&mut command, visualize.as_ref(), std::env::var("OPENCODE_CONFIG_CONTENT").ok().as_deref(), tools);
    command
        .current_dir(request.cwd)
        .env("OPENCODE_SERVER_USERNAME", "bridge")
        .env("OPENCODE_SERVER_PASSWORD", &server_password)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        // Piped and tail-captured so a dead server reports its own error
        // instead of a generic exit.
        .stderr(Stdio::piped());
    // Shared per-repository build output is skipped for a read-only worker:
    // its sandbox writes only under its own output folder (see `codex_adapter`).
    if sandbox_homes.is_none() {
        crate::build_cache::apply(&mut command, std::path::Path::new(request.cwd));
    }
    // An extra account runs its own server on its own data folder. A read-only
    // sandbox below moves every writable home into the sandbox instead, seeded
    // from that account's logins.
    crate::provider_accounts::apply_account(&mut command, request.account);
    if let (Some(homes), Some(sandbox)) = (&sandbox_homes, request.read_only_sandbox) {
        for (name, value) in homes.environment() {
            command.env(name, value);
        }
        // The sandbox cannot reach the keychain; a networked worker (e.g. a PR
        // review) needs the host token or every `gh` call returns 401.
        if sandbox.network_allowed() {
            if let Some(token) = crate::worker_sandbox::github_cli_token() {
                command.env("GH_TOKEN", token);
            }
        }
    }
    crate::credential_broker::apply_proxy_pass(&mut command);
    crate::adapters::configure_process_group(&mut command);
    if let Some(on_progress) = request.on_progress {
        on_progress(crate::adapters::StartupPhase::Spawning);
    }
    let spawned_at = std::time::Instant::now();
    let mut child = command.spawn()?;
    let ledger = crate::process_ledger::record_launch("opencode.session", request.cwd, child.id());
    let stderr_tail = crate::adapters::StderrTail::capture(&mut child);
    let client = match build_authenticated_client(api, &server_password, request.cwd) {
        Ok(client) => client,
        Err(error) => {
            stop_child(&mut child);
            return Err(error);
        }
    };
    if let Some(on_progress) = request.on_progress {
        on_progress(crate::adapters::StartupPhase::Handshake);
    }
    // A sandboxed server starts on an empty database and runs every OpenCode
    // migration first, so it gets longer than a server on the user's own.
    let deadline = if sandbox_homes.is_some() { SANDBOXED_STARTUP_DEADLINE } else { STARTUP_DEADLINE };
    if let Err(error) = wait_until_ready(&client, &base_url, &mut child, api, deadline) {
        stop_child(&mut child);
        drop_client_safely(client);
        return Err(error);
    }
    crate::process_ledger::log_spawn_to_ready("opencode", "server_ready", spawned_at);
    if let (Some(homes), Some(host), ServerApi::V2) = (&sandbox_homes, &host_data, api) {
        let seeded = seed_stored_credentials(
            &host.join("opencode.db"),
            &homes.opencode_data().join("opencode.db"),
            now_ms(),
        );
        if let Err(error) = seeded {
            stop_child(&mut child);
            drop_client_safely(client);
            return Err(error);
        }
    }

    let model = request.model.and_then(parse_model);
    let variant = request
        .effort
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let directory = request.cwd.to_owned();

    let session_prefix = match api {
        ServerApi::V1 => "/session",
        ServerApi::V2 => "/api/session",
    };
    let session_result = prepare_session(kind, &client, api, &base_url, &directory).and_then(|()| match resume_session_id {
        Some(session_id) => client
            .get(api.url(&base_url, &format!("{session_prefix}/{session_id}"), &directory))
            .timeout(Duration::from_secs(10))
            .send()
            .map_err(http_error("resume OpenCode session"))
            .and_then(|response| checked_json(response, "resume OpenCode session")),
        None => {
            let mut body = match kind {
                SessionKind::Completion => completion_session_body(model.as_ref(), variant.as_deref()),
                SessionKind::Courier { permission, .. } => courier_session_body(model.as_ref(), variant.as_deref(), permission),
                SessionKind::Chat => session_create_body(model.as_ref(), variant.as_deref(), request.write_mode),
            };
            if api == ServerApi::V2 {
                body = v2_session_body(body);
            }
            client
                .post(api.url(&base_url, session_prefix, &directory))
                .timeout(Duration::from_secs(10))
                .json(&body)
                .send()
                .map_err(http_error("create OpenCode session"))
                .and_then(|response| checked_json(response, "create OpenCode session"))
        }
    });
    // 2.x wraps every result in `{data}`.
    let session_result = session_result.map(|session| match api {
        ServerApi::V1 => session,
        ServerApi::V2 => session.get("data").cloned().unwrap_or(Value::Null),
    });
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
    // 2.x has no per-message `system` value. Bridge's instructions become a
    // durable entry on the session instead, rewritten on every launch so a
    // resumed session carries this launch's authority and not a stale one.
    if api == ServerApi::V2 {
        let entry = api.url(
            &base_url,
            &format!("/api/experimental/session/{session_id}/instructions/entries/bridge"),
            &directory,
        );
        match request.instructions.map(str::trim).filter(|value| !value.is_empty()) {
            Some(instructions) => {
                let attached = client
                    .put(entry)
                    .timeout(Duration::from_secs(10))
                    .json(&json!({"value": instructions}))
                    .send()
                    .map_err(http_error("attach Bridge instructions to the OpenCode session"))
                    .and_then(|response| checked_status(response, "attach Bridge instructions to the OpenCode session"));
                if let Err(error) = attached {
                    stop_child(&mut child);
                    drop_client_safely(client);
                    return Err(error);
                }
            }
            // A resumed session may still hold an earlier launch's entry.
            // Absent is the goal, so a missing entry is not an error.
            None if resume_session_id.is_some() => {
                let _ = client.delete(entry).timeout(Duration::from_secs(10)).send();
            }
            None => {}
        }
    }

    let pending = opencode_v2::PendingRequests::default();
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
        EventStreamPolicy::default(),
        api,
        pending.clone(),
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
            api,
            pending,
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

/// The sandbox modes OpenCode can start in. A read-only worker runs its
/// `opencode serve` inside the read-only sandbox like any other harness; the
/// one thing that sandbox cannot give it is a loopback socket once runtime
/// network is denied outright, so `read_only` is dropped exactly then.
pub(crate) fn sandbox_modes() -> Vec<crate::model::SandboxMode> {
    sandbox_modes_for(crate::worker_sandbox::runtime_network_denied())
}

pub(crate) fn sandbox_modes_for(runtime_network_denied: bool) -> Vec<crate::model::SandboxMode> {
    use crate::model::SandboxMode;
    if runtime_network_denied {
        vec![SandboxMode::WorkspaceWrite, SandboxMode::DangerFullAccess]
    } else {
        SandboxMode::ALL.to_vec()
    }
}

fn read_only_refusal(runtime_network_denied: bool) -> Option<&'static str> {
    runtime_network_denied.then_some(
        "OpenCode cannot run a read-only worker while BRIDGE_READ_ONLY_NETWORK=deny: OpenCode serves its agent over a loopback HTTP server, and that setting blocks every socket. Unset it, or route this worker to Claude or Codex.",
    )
}

fn ensure_read_only_transport_supported(
    sandbox: Option<&crate::worker_sandbox::ReadOnlySandbox>,
) -> Result<(), BridgeError> {
    match sandbox.and_then(|sandbox| read_only_refusal(sandbox.runtime_network_denied())) {
        Some(reason) => Err(BridgeError::Invalid(reason.into())),
        None => Ok(()),
    }
}

/// Where a read-only worker's OpenCode keeps its own state. OpenCode writes
/// its log, database, snapshots and caches under its XDG homes, and the
/// read-only sandbox denies every write outside the worker's output folder, so
/// all of them move there. The user's config home is left alone: it is only
/// read, and it carries their custom providers and MCP servers.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SandboxHomes {
    output: PathBuf,
    data: PathBuf,
    state: PathBuf,
    cache: PathBuf,
    tmp: PathBuf,
}

impl SandboxHomes {
    fn under(output: &Path) -> Self {
        let root = output.join(".opencode");
        Self {
            output: output.to_path_buf(),
            data: root.join("data"),
            state: root.join("state"),
            cache: root.join("cache"),
            tmp: root.join("tmp"),
        }
    }

    fn create(&self) -> Result<(), BridgeError> {
        for dir in [&self.data.join("opencode"), &self.state, &self.cache, &self.tmp] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    /// OpenCode's own data folder inside the sandbox.
    fn opencode_data(&self) -> PathBuf {
        self.data.join("opencode")
    }

    fn environment(&self) -> Vec<(&'static str, &Path)> {
        vec![
            ("XDG_DATA_HOME", self.data.as_path()),
            ("XDG_STATE_HOME", self.state.as_path()),
            ("XDG_CACHE_HOME", self.cache.as_path()),
            ("TMPDIR", self.tmp.as_path()),
            ("BRIDGE_WORKER_OUTPUT_DIR", self.output.as_path()),
        ]
    }
}

/// The OpenCode data folder whose logins a sandboxed worker borrows: an extra
/// account's own data home when the launch is for one, else the user's.
fn host_opencode_data(account: Option<&crate::provider_accounts::AccountLaunch>) -> Option<PathBuf> {
    account
        .and_then(|account| account.value("XDG_DATA_HOME"))
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()).map(PathBuf::from))
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .map(|base| base.join("opencode"))
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// One stored login as a sandboxed worker may hold it, or `None` to leave it
/// out. An OAuth login keeps its access token but loses its refresh token:
/// a refresh can rotate the token the user's own OpenCode depends on, and the
/// rotated one would die with the sandbox. An already-expired OAuth login is
/// useless without that refresh, so it is not copied at all. Keys pass as-is.
fn sandboxed_credential(mut credential: Value, now_ms: i64) -> Option<Value> {
    let object = credential.as_object_mut()?;
    if object.get("type").and_then(Value::as_str) == Some("oauth") {
        if object
            .get("expires")
            .and_then(Value::as_i64)
            .is_some_and(|expires| expires <= now_ms)
        {
            return None;
        }
        if object.contains_key("refresh") {
            object.insert("refresh".into(), json!(""));
        }
    }
    Some(credential)
}

const MAX_AUTH_JSON_BYTES: u64 = 1_048_576;

/// OpenCode 1.x keeps logins in `auth.json`: copy it into the sandbox's data
/// folder, each entry passed through [`sandboxed_credential`]. Returns how
/// many logins were copied; a missing file copies none.
fn seed_legacy_auth(host: &Path, sandbox: &Path, now_ms: i64) -> Result<usize, BridgeError> {
    let source = host.join("auth.json");
    let Ok(metadata) = std::fs::metadata(&source) else {
        return Ok(0);
    };
    if metadata.len() > MAX_AUTH_JSON_BYTES {
        return Err(BridgeError::Invalid(
            "OpenCode's auth.json is too large to hand to a read-only worker".into(),
        ));
    }
    let text = zeroize::Zeroizing::new(std::fs::read_to_string(&source)?);
    if text.trim().is_empty() {
        return Ok(0);
    }
    let Value::Object(entries) = serde_json::from_str::<Value>(&text).map_err(|_| {
        BridgeError::Invalid("OpenCode's auth.json is not valid JSON, so a read-only worker cannot sign in".into())
    })?
    else {
        return Ok(0);
    };
    let sanitized = entries
        .into_iter()
        .filter_map(|(provider, credential)| {
            sandboxed_credential(credential, now_ms).map(|credential| (provider, credential))
        })
        .collect::<serde_json::Map<_, _>>();
    let copied = sanitized.len();
    let body = zeroize::Zeroizing::new(serde_json::to_string(&Value::Object(sanitized))
        .map_err(|error| BridgeError::Invalid(error.to_string()))?);
    write_private(&sandbox.join("auth.json"), body.as_bytes())?;
    Ok(copied)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), BridgeError> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)?;
    Ok(())
}

fn table_columns(db: &rusqlite::Connection, table: &str) -> Result<Vec<String>, rusqlite::Error> {
    let mut statement = db.prepare(&format!("PRAGMA table_info(\"{table}\")"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(columns)
}

/// OpenCode 2.x keeps logins in its database's `credential` table. Copy the
/// user's rows into the sandbox database that the worker's own server has just
/// created, each value passed through [`sandboxed_credential`]. The user's
/// database is opened read-only and never written. Returns how many logins
/// were copied; a missing database or table copies none.
fn seed_stored_credentials(host_db: &Path, sandbox_db: &Path, now_ms: i64) -> Result<usize, BridgeError> {
    use rusqlite::types::Value as Sql;
    if !host_db.exists() {
        return Ok(0);
    }
    let unreadable = |_| {
        BridgeError::Invalid(
            "Could not read OpenCode's stored logins for a read-only worker. Check access to its database.".into(),
        )
    };
    let host = crate::usage_import::opencode::open_read_only(host_db).map_err(unreadable)?;
    let _ = host.busy_timeout(Duration::from_secs(2));
    let host_columns = table_columns(&host, "credential").map_err(|error| unreadable(BridgeError::Db(error)))?;
    if host_columns.is_empty() {
        return Ok(0);
    }
    let sandbox = rusqlite::Connection::open(sandbox_db)?;
    sandbox.busy_timeout(Duration::from_secs(5))?;
    let sandbox_columns = table_columns(&sandbox, "credential")?;
    if sandbox_columns.is_empty() {
        return Err(BridgeError::Adapter(
            "The read-only worker's OpenCode server started without a credential store".into(),
        ));
    }
    // Columns both sides know, in the user's order. Same binary, so normally
    // identical; a column only one side has is left to its default.
    let columns = host_columns
        .into_iter()
        .filter(|column| sandbox_columns.contains(column))
        .collect::<Vec<_>>();
    let Some(value_index) = columns.iter().position(|column| column == "value") else {
        return Err(BridgeError::Adapter(
            "OpenCode's credential store has no value column; this OpenCode version is not supported for read-only workers".into(),
        ));
    };
    let list = columns.iter().map(|column| format!("\"{column}\"")).collect::<Vec<_>>().join(", ");
    let mut select = host
        .prepare(&format!("SELECT {list} FROM credential"))
        .map_err(|error| unreadable(BridgeError::Db(error)))?;
    let rows = select
        .query_map([], |row| {
            (0..columns.len()).map(|index| row.get::<_, Sql>(index)).collect::<Result<Vec<_>, _>>()
        })
        .map_err(|error| unreadable(BridgeError::Db(error)))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| unreadable(BridgeError::Db(error)))?;
    let placeholders = (1..=columns.len()).map(|index| format!("?{index}")).collect::<Vec<_>>().join(", ");
    let insert = format!("INSERT OR REPLACE INTO credential ({list}) VALUES ({placeholders})");
    let transaction = sandbox.unchecked_transaction()?;
    let mut copied = 0;
    for mut row in rows {
        let Sql::Text(text) = &row[value_index] else { continue };
        let text = zeroize::Zeroizing::new(text.clone());
        let Some(credential) = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|credential| sandboxed_credential(credential, now_ms))
        else {
            continue;
        };
        row[value_index] = Sql::Text(credential.to_string());
        transaction.execute(&insert, rusqlite::params_from_iter(row.iter()))?;
        copied += 1;
    }
    transaction.commit()?;
    Ok(copied)
}

/// A courier needs the 2.x API: its MCP tools live in code mode and its
/// permission prompts carry their arguments.
fn ensure_kind_supported(kind: SessionKind<'_>, api: ServerApi) -> Result<(), BridgeError> {
    if matches!(kind, SessionKind::Courier { .. }) && api != ServerApi::V2 {
        return Err(BridgeError::Invalid("Personal apps need OpenCode 2 or newer to call this server".into()));
    }
    Ok(())
}

/// Before a courier session exists, so its tools and permissions are fixed
/// the way Bridge needs them:
/// - wait for the one server to connect (a session's tools are fixed when it
///   is created, and OpenCode connects servers after it is ready);
/// - disconnect every other server in this private OpenCode process, so no
///   other server's tool can share the target's flattened permission name
///   (`fake`+`send_note` and `fake_send`+`note` are both `fake_send_note`).
///   Disconnecting is per process, never written to the person's config;
/// - refuse when OpenCode holds a saved "always allow" for the target: it is
///   appended after the session's rules, so the call would never be asked
///   about and Bridge could not check its arguments.
fn prepare_session(kind: SessionKind<'_>, client: &Client, api: ServerApi, base_url: &str, directory: &str) -> Result<(), BridgeError> {
    let SessionKind::Courier { permission, server } = kind else { return Ok(()) };
    wait_for_mcp_server(client, api, base_url, directory, server);
    let get = |path: &str, action: &'static str| -> Result<Value, BridgeError> {
        client.get(api.url(base_url, path, directory)).timeout(Duration::from_secs(10)).send().map_err(http_error(action))
            .and_then(|response| checked_json(response, action))
    };
    let listed = get("/api/mcp", "list OpenCode MCP servers")?;
    let others: Vec<&str> = listed["data"].as_array().into_iter().flatten().filter_map(|entry| entry["name"].as_str()).filter(|name| *name != server).collect();
    for other in &others {
        let path = format!("/api/experimental/mcp/{}/disconnect", percent_encode_segment(other));
        let response = client.post(api.url(base_url, &path, directory)).timeout(Duration::from_secs(10)).send().map_err(http_error("isolate an OpenCode MCP server"))?;
        if !response.status().is_success() {
            return Err(BridgeError::Invalid("OpenCode would not set aside its other MCP servers for this call, so it was not made".into()));
        }
    }
    // Setting servers aside makes OpenCode rebuild its code-mode tool
    // catalog in the background, and a session created meanwhile starts with
    // none. There is no readiness signal for it, so give it a moment.
    if !others.is_empty() {
        thread::sleep(Duration::from_millis(1500));
    }
    wait_for_mcp_server(client, api, base_url, directory, server);
    let saved = get("/api/permission/saved", "read OpenCode's saved permissions")?;
    if saved_allows(&saved, permission) {
        return Err(BridgeError::Invalid(format!(
            "OpenCode has a saved \"always allow\" covering {permission}, so it would make the call without asking and Bridge could not check it. Remove that saved permission in OpenCode to use this app through OpenCode"
        )));
    }
    Ok(())
}

/// Whether any of OpenCode's saved permissions allows `permission`. Saved
/// rules are allows; their `action` may carry `*` wildcards.
fn saved_allows(saved: &Value, permission: &str) -> bool {
    fn matches(pattern: &str, text: &str) -> bool {
        match pattern.split_once('*') {
            None => pattern == text,
            Some((head, tail)) => text.strip_prefix(head).is_some_and(|rest| (0..=rest.len()).any(|cut| rest.is_char_boundary(cut) && matches(tail, &rest[cut..]))),
        }
    }
    saved["data"].as_array().into_iter().flatten().any(|rule| {
        let effect = rule["effect"].as_str().unwrap_or("allow");
        effect == "allow" && rule["action"].as_str().is_some_and(|action| matches(action, permission))
    })
}

fn percent_encode_segment(text: &str) -> String {
    text.bytes().map(|byte| if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) { (byte as char).to_string() } else { format!("%{byte:02X}") }).collect()
}

/// Wait up to 20 s for `server` to leave OpenCode's connecting states. A
/// server that fails or needs sign-in stops the wait early; the call then
/// reports it. Never an error itself.
fn wait_for_mcp_server(client: &Client, api: ServerApi, base_url: &str, directory: &str, server: &str) {
    let url = api.url(base_url, "/api/mcp", directory);
    let client = client.clone();
    let server = server.to_owned();
    let waited = thread::Builder::new().name("opencode-mcp-wait".into()).spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            let status = client
                .get(url.clone())
                .timeout(Duration::from_secs(5))
                .send()
                .ok()
                .and_then(|response| response.json::<Value>().ok())
                .and_then(|listed| {
                    listed["data"].as_array().and_then(|servers| {
                        servers.iter().find(|entry| entry["name"].as_str() == Some(server.as_str()))
                            .and_then(|entry| entry.pointer("/status/status").and_then(Value::as_str).map(str::to_owned))
                    })
                });
            if matches!(status.as_deref(), Some("connected" | "failed" | "needs_auth" | "disabled")) {
                return;
            }
            thread::sleep(Duration::from_millis(250));
        }
    });
    if let Ok(handle) = waited {
        let _joined = handle.join();
    }
}

/// Deny everything, allow code mode, ask for exactly one MCP tool. The order
/// matters: OpenCode takes the last matching rule.
fn courier_session_body(model: Option<&ModelRef>, variant: Option<&str>, permission: &str) -> Value {
    let mut body = session_create_body(model, variant, Some(WriteMode::ReadOnly));
    body["title"] = json!("Bridge personal app call");
    body["permission"] = json!([
        {"permission":"*", "pattern":"*", "action":"deny"},
        {"permission":"execute", "pattern":"*", "action":"allow"},
        {"permission": permission, "pattern":"*", "action":"ask"},
    ]);
    body
}

fn completion_session_body(model: Option<&ModelRef>, variant: Option<&str>) -> Value {
    let mut body = session_create_body(model, variant, Some(WriteMode::ReadOnly));
    body["title"] = json!("Bridge inline suggestions");
    body["permission"] = json!([{"permission":"*", "pattern":"*", "action":"deny"}]);
    body
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

/// A 1.x session body in 2.x terms: the model reference is already the same
/// shape, the permission rules are not.
fn v2_session_body(body: Value) -> Value {
    let mut converted = json!({
        "title": body.get("title").cloned().unwrap_or(json!("Bridge session")),
        "permissions": opencode_v2::permission_rules(body.get("permission").unwrap_or(&Value::Null)),
    });
    if let Some(model) = body.get("model") {
        converted["model"] = model.clone();
    }
    converted
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

fn build_authenticated_client(
    api: ServerApi,
    server_password: &str,
    directory: &str,
) -> Result<Client, BridgeError> {
    let authorization = format!(
        "Basic {}",
        BASE64.encode(format!("{}:{server_password}", api.username()))
    );
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&authorization).map_err(|error| {
            BridgeError::Adapter(format!("Cannot secure OpenCode server: {error}"))
        })?,
    );
    if api == ServerApi::V2 {
        headers.insert(
            "x-opencode-directory",
            HeaderValue::from_str(&encode_directory(directory)).map_err(|error| {
                BridgeError::Adapter(format!("Cannot scope OpenCode requests: {error}"))
            })?,
        );
    }
    Client::builder()
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(2))
        .timeout(None)
        .build()
        .map_err(|error| BridgeError::Adapter(format!("Cannot create OpenCode client: {error}")))
}

/// 2.x URI-decodes `x-opencode-directory`, so anything a header cannot carry
/// is percent-encoded.
fn encode_directory(directory: &str) -> String {
    directory
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-_.~/".contains(&byte) {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

/// Whether this runtime can ask OpenCode to summarize.
///
/// The endpoint requires the model that writes the summary, so a runtime with
/// no model selected has nothing to name and would be refused on the wire.
/// Answering `Unsupported` sends the request to Bridge's own checkpoint
/// instead of to a call that cannot succeed.
/// 2.x compacts with the session's own model, so it needs none named.
fn compaction_support(api: ServerApi, model: Option<&ModelRef>) -> crate::adapters::NativeCompaction {
    match (api, model) {
        (ServerApi::V2, _) | (ServerApi::V1, Some(_)) => {
            crate::adapters::NativeCompaction::WholeConversation
        }
        (ServerApi::V1, None) => crate::adapters::NativeCompaction::Unsupported,
    }
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

const STARTUP_DEADLINE: Duration = Duration::from_secs(8);
const SANDBOXED_STARTUP_DEADLINE: Duration = Duration::from_secs(20);

fn wait_until_ready(
    client: &Client,
    base_url: &str,
    child: &mut Child,
    api: ServerApi,
    budget: Duration,
) -> Result<(), BridgeError> {
    let deadline = Instant::now() + budget;
    let mut attempt = 0usize;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait()? {
            return Err(BridgeError::Adapter(format!(
                "OpenCode server exited during startup with {status}"
            )));
        }
        if client
            .get(format!("{base_url}{}", api.health_path()))
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
            return verify_server_requires_credentials(base_url, api);
        }
        thread::sleep(probe_backoff(attempt));
        attempt += 1;
    }
    Err(BridgeError::Adapter(
        "OpenCode server did not become ready".into(),
    ))
}

fn verify_server_requires_credentials(base_url: &str, api: ServerApi) -> Result<(), BridgeError> {
    // Runs on a dedicated thread so the probe client (and its response) are
    // dropped there — see drop_client_safely.
    let health_url = format!("{base_url}{}", api.health_path());
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
    // Bridge's notes write tools. OpenCode names MCP tools `<server>_<tool>`
    // and matches permissions by that name, so the catch-all `allow` above
    // would let Ask change a person's notes without asking.
    let notes_action = match write_mode {
        Some(WriteMode::ReadOnly) => Some("deny"),
        Some(WriteMode::Shared | WriteMode::Isolated) => Some("ask"),
        None | Some(WriteMode::Full) => None,
    };
    if let Some(action) = notes_action {
        for tool in crate::notes::tool::WRITE_TOOLS {
            let permission = format!("{}_{tool}", crate::mcp_apps::tool::SERVER_NAME);
            rules.push(json!({"permission": permission, "pattern":"*", "action": action}));
        }
    }
    // A memory proposal waits for the person's review in Bridge, and a change
    // to Bridge's own settings cannot widen authority, so Ask lets both run
    // like a read; a read-only session never proposes, never drafts a message
    // for someone else, never touches Bridge's settings, and never reads the
    // person's other chats (the core refuses those too).
    if write_mode == Some(WriteMode::ReadOnly) {
        for tool in crate::memory_tools::WRITE_TOOLS
            .iter()
            .chain(crate::outgoing_tools::WRITE_TOOLS.iter())
            .chain(crate::settings_tools::TOOLS.iter())
            .chain(crate::history_tools::TOOLS.iter())
        {
            let permission = format!("{}_{tool}", crate::mcp_apps::tool::SERVER_NAME);
            rules.push(json!({"permission": permission, "pattern":"*", "action": "deny"}));
        }
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

/// For 2.x routes that answer `204 No Content` or a body Bridge does not read.
fn checked_status(response: Response, action: &str) -> Result<(), BridgeError> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let body = response.text().unwrap_or_default();
    Err(BridgeError::Adapter(format!("Failed to {action} ({status}): {body}")))
}

fn http_error(action: &'static str) -> impl FnOnce(reqwest::Error) -> BridgeError {
    move |error| BridgeError::Adapter(format!("Failed to {action}: {error}"))
}

/// What the SSE reader does with one decoded `/event` frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameDisposition {
    /// Ours: queue it for normalization.
    Forward,
    /// Another session on the same server. Counted, never queued.
    Foreign,
}

/// Decides which frames on the shared `/event` bus belong to this runtime.
///
/// One `opencode serve` process is private to one Bridge session, but the bus
/// still carries more than the root session's frames. The `task` tool creates
/// a real child session with its own id and `info.parentID`, so the filter
/// owns a *tree* of session ids rather than one. `session.error` may arrive
/// with no `sessionID` at all (skill and plugin failures publish it that way),
/// and `server.heartbeat` / `server.connected` never carry one; on a private
/// server those are ours by construction and are forwarded so the turn can
/// fail visibly and liveness can be measured.
///
/// Session attribution is shared with the adapter registry and normalizer, so
/// a frame the reader admits cannot later fall into a different stream state.
struct SessionFrameFilter {
    root: String,
    owned: HashSet<String>,
    /// Foreign session ids already reported, so a busy stranger logs once.
    reported_foreign: HashSet<String>,
}

impl SessionFrameFilter {
    fn new(root: String) -> Self {
        let mut owned = HashSet::new();
        owned.insert(root.clone());
        Self {
            root,
            owned,
            reported_foreign: HashSet::new(),
        }
    }

    #[cfg(test)]
    fn is_child(&self, session_id: &str) -> bool {
        session_id != self.root && self.owned.contains(session_id)
    }

    fn classify(&mut self, value: &Value) -> FrameDisposition {
        let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
        if is_liveness_event(event_type) {
            return FrameDisposition::Forward;
        }
        let properties = value.get("properties").unwrap_or(&Value::Null);
        if matches!(event_type, "session.created" | "session.updated") {
            let parent = properties
                .pointer("/info/parentID")
                .and_then(Value::as_str);
            let child = properties.pointer("/info/id").and_then(Value::as_str);
            if let (Some(parent), Some(child)) = (parent, child) {
                if self.owned.contains(parent) {
                    self.owned.insert(child.to_owned());
                    return FrameDisposition::Forward;
                }
            }
        }
        match agent::opencode_frame_session(event_type, properties) {
            Some(session_id) if self.owned.contains(session_id) => FrameDisposition::Forward,
            // An unattributed error on a private server can only be ours.
            None if event_type == "session.error" => FrameDisposition::Forward,
            _ => FrameDisposition::Foreign,
        }
    }

    /// An OpenCode `session.error` can omit its id. A private reader knows it
    /// belongs to its root, but downstream code cannot infer that safely once
    /// frames from multiple sessions share the adapter. Stamp it before it
    /// reaches the queue so routing and normalization use the root state.
    fn stamp_unattributed_error(&self, value: &mut Value) {
        let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
        let attributed = value
            .get("properties")
            .and_then(|properties| agent::opencode_frame_session(event_type, properties))
            .is_some();
        if event_type != "session.error" || attributed {
            return;
        }
        if !value.get("properties").is_some_and(Value::is_object) {
            value["properties"] = json!({});
        }
        value["properties"]["sessionID"] = Value::String(self.root.clone());
    }

    /// Root first, then children in stable order. Reconnect recovery needs to
    /// catch up every owned session, not just the parent which happened to own
    /// the socket.
    fn owned_sessions(&self) -> Vec<String> {
        let mut sessions = self.owned.iter().cloned().collect::<Vec<_>>();
        sessions.sort();
        sessions.retain(|session| session != &self.root);
        sessions.insert(0, self.root.clone());
        sessions
    }

    /// Leave evidence of a dropped frame without flooding stderr: one line
    /// per distinct foreign session id (or once for id-less strangers).
    fn note_foreign(&mut self, value: &Value) {
        let session_id = value
            .get("properties")
            .and_then(|properties| {
                let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
                agent::opencode_frame_session(event_type, properties)
            })
            .unwrap_or("<none>")
            .to_owned();
        if self.reported_foreign.insert(session_id.clone()) {
            diagnostics::record(&format!(
                "bridge: opencode session {} dropped a frame from foreign session {} ({})",
                self.root,
                session_id,
                value.get("type").and_then(Value::as_str).unwrap_or("?")
            ));
        }
    }
}

/// Server-level frames that carry no session id and prove the process is alive.
fn is_liveness_event(event_type: &str) -> bool {
    matches!(event_type, "server.heartbeat" | "server.connected")
}

/// Backoff between reconnect attempts after the `/event` body ends while the
/// server is still supposed to be alive. Five rungs, then the disconnect is
/// surfaced as the turn error it always was.
const RECONNECT_BACKOFF_SECONDS: [u64; 5] = [1, 2, 4, 8, 16];

/// How the SSE reader behaves when its body ends. Production uses the
/// constant schedule; tests inject millisecond rungs.
#[derive(Debug, Clone)]
struct EventStreamPolicy {
    reconnect_backoff: Vec<Duration>,
}

impl Default for EventStreamPolicy {
    fn default() -> Self {
        Self {
            reconnect_backoff: RECONNECT_BACKOFF_SECONDS
                .iter()
                .map(|seconds| Duration::from_secs(*seconds))
                .collect(),
        }
    }
}

/// Why one `/event` body stopped yielding frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamEnd {
    /// The consumer dropped the queue: nothing left to forward to.
    ConsumerGone,
    /// The HTTP body ended (server closed, proxy reset, read error).
    BodyEnded,
}

fn connect_event_stream(
    client: &Client,
    base_url: &str,
    directory: &str,
    api: ServerApi,
) -> Result<Response, BridgeError> {
    let path = match api {
        ServerApi::V1 => "/event",
        ServerApi::V2 => "/api/event",
    };
    match client.get(api.url(base_url, path, directory)).send() {
        Ok(response) if response.status().is_success() => Ok(response),
        Ok(response) => Err(BridgeError::Adapter(format!(
            "Failed to connect to OpenCode event stream ({})",
            response.status()
        ))),
        Err(error) => Err(http_error("connect to OpenCode event stream")(error)),
    }
}

/// Forward one `/event` body's frames until it ends or the consumer leaves.
/// A 2.x stream is translated into 1.x frames first, so filtering and
/// everything downstream sees one shape.
fn pump_event_stream(
    response: Response,
    filter: &mut SessionFrameFilter,
    translator: &mut Option<opencode_v2::Translator>,
    sender: &crate::frame_queue::FrameSender,
) -> StreamEnd {
    let mut reader = std::io::BufReader::new(response);
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return StreamEnd::BodyEnded,
            Ok(_) => {}
        }
        let Some(data) = line.trim_end().strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        let frames = match translator {
            Some(translator) => translator.translate(&value),
            None => vec![value],
        };
        for mut value in frames {
            match filter.classify(&value) {
                FrameDisposition::Forward => filter.stamp_unattributed_error(&mut value),
                FrameDisposition::Foreign => {
                    sender.record_foreign_drop();
                    filter.note_foreign(&value);
                    continue;
                }
            }
            // Streaming deltas are the only sheddable frames: their terminal
            // `message.part.updated` carries the complete content. Everything
            // else is durable and back-pressures this socket when the
            // consumer stalls, instead of buffering without bound.
            let transient = value.get("type").and_then(Value::as_str) == Some("message.part.delta");
            let frame = format!("{value}\n");
            let delivered = if transient {
                sender.send_transient(frame).map(|_| ())
            } else {
                sender.send_durable(frame)
            };
            if delivered.is_err() {
                return StreamEnd::ConsumerGone;
            }
        }
    }
}

/// Close the gap a reconnect leaves. `/event` has no replay, so whatever the
/// server published while the socket was down is gone; the session state it
/// still holds is not. Re-emit every assistant snapshot for the root and its
/// already-known children (the normalizer replaces by part id, so repeats are
/// idempotent), then independently ask whether the root is still busy.
fn resync_after_reconnect(
    client: &Client,
    base_url: &str,
    directory: &str,
    session_id: &str,
    owned_sessions: &[String],
    sender: &crate::frame_queue::FrameSender,
) -> Result<(), crate::frame_queue::Disconnected> {
    for owned_session_id in owned_sessions {
        let messages = client
            .get(endpoint(
                base_url,
                &format!("/session/{owned_session_id}/message"),
                directory,
            ))
            .timeout(Duration::from_secs(10))
            .send()
            .ok()
            .and_then(|response| checked_json(response, "resync the OpenCode session").ok())
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default();
        for message in &messages {
            let info = message.get("info").cloned().unwrap_or(Value::Null);
            if info.get("role").and_then(Value::as_str) != Some("assistant") {
                continue;
            }
            let updated = json!({
                "type": "message.updated",
                "properties": { "sessionID": owned_session_id, "info": info }
            });
            sender.send_durable(format!("{updated}\n"))?;
            for part in message.get("parts").and_then(Value::as_array).into_iter().flatten() {
                let snapshot = json!({
                    "type": "message.part.updated",
                    "properties": { "sessionID": owned_session_id, "part": part }
                });
                sender.send_durable(format!("{snapshot}\n"))?;
            }
        }
    }
    // `/session/status` lists only sessions with a live status; an idle
    // session is simply absent.
    let status = client
        .get(endpoint(base_url, "/session/status", directory))
        .timeout(Duration::from_secs(10))
        .send()
        .ok()
        .and_then(|response| checked_json(response, "read OpenCode session status").ok());
    let Some(status) = status else {
        return Ok(());
    };
    let busy = status
        .get(session_id)
        .and_then(|entry| entry.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|kind| kind != "idle");
    if !busy {
        let idle = json!({
            "type": "session.status",
            "properties": { "sessionID": session_id, "status": { "type": "idle" } }
        });
        sender.send_durable(format!("{idle}\n"))?;
    }
    Ok(())
}

/// The 2.x counterpart of `resync_after_reconnect`: the newest page of each
/// owned session's messages, rebuilt as the snapshots the live stream would
/// have produced, then whether the root is still running.
fn resync_v2_after_reconnect(
    client: &Client,
    base_url: &str,
    session_id: &str,
    owned_sessions: &[String],
    sender: &crate::frame_queue::FrameSender,
) -> Result<(), crate::frame_queue::Disconnected> {
    for owned_session_id in owned_sessions {
        let mut messages = client
            .get(format!("{base_url}/api/session/{owned_session_id}/message?order=desc&limit=50"))
            .timeout(Duration::from_secs(10))
            .send()
            .ok()
            .and_then(|response| checked_json(response, "resync the OpenCode session").ok())
            .and_then(|value| value.get("data").and_then(Value::as_array).cloned())
            .unwrap_or_default();
        messages.reverse();
        for frame in opencode_v2::message_snapshots(owned_session_id, &messages) {
            sender.send_durable(format!("{frame}\n"))?;
        }
    }
    // `/api/session/active` lists only running sessions.
    let active = client
        .get(format!("{base_url}/api/session/active"))
        .timeout(Duration::from_secs(10))
        .send()
        .ok()
        .and_then(|response| checked_json(response, "read OpenCode session status").ok());
    let Some(active) = active else {
        return Ok(());
    };
    if active.pointer(&format!("/data/{session_id}")).is_none() {
        let idle = json!({
            "type": "session.status",
            "properties": { "sessionID": session_id, "status": { "type": "idle" } }
        });
        sender.send_durable(format!("{idle}\n"))?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn spawn_event_stream(
    client: Client,
    base_url: String,
    directory: String,
    session_id: String,
    sender: crate::frame_queue::FrameSender,
    shutting_down: Arc<AtomicBool>,
    policy: EventStreamPolicy,
    api: ServerApi,
    pending: opencode_v2::PendingRequests,
) -> Result<(), BridgeError> {
    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut response = match connect_event_stream(&client, &base_url, &directory, api) {
            Ok(response) => response,
            Err(error) => {
                let _ = ready_sender.send(Err(error));
                return;
            }
        };
        if ready_sender.send(Ok(())).is_err() {
            return;
        }
        let mut filter = SessionFrameFilter::new(session_id.clone());
        let mut translator = (api == ServerApi::V2).then(|| opencode_v2::Translator::new(pending));
        loop {
            if pump_event_stream(response, &mut filter, &mut translator, &sender) == StreamEnd::ConsumerGone {
                return;
            }
            // During shutdown the body ending is expected.
            if shutting_down.load(Ordering::SeqCst) {
                return;
            }
            // The server may still be alive and holding the session: a
            // transient disconnect must not end streaming for good.
            let mut reconnected = None;
            for delay in &policy.reconnect_backoff {
                thread::sleep(*delay);
                if shutting_down.load(Ordering::SeqCst) {
                    return;
                }
                if let Ok(next) = connect_event_stream(&client, &base_url, &directory, api) {
                    reconnected = Some(next);
                    break;
                }
            }
            let Some(next) = reconnected else {
                // Every retry failed. Otherwise the turn would silently appear
                // finished, so surface the disconnect exactly once.
                let error_event = json!({
                    "type": "session.error",
                    "properties": {
                        "sessionID": session_id,
                        "error": { "message": "OpenCode event stream disconnected unexpectedly" }
                    }
                });
                let _ = sender.send_durable(format!("{error_event}\n"));
                return;
            };
            let resynced = match api {
                ServerApi::V1 => resync_after_reconnect(
                    &client,
                    &base_url,
                    &directory,
                    &session_id,
                    &filter.owned_sessions(),
                    &sender,
                ),
                ServerApi::V2 => resync_v2_after_reconnect(
                    &client,
                    &base_url,
                    &session_id,
                    &filter.owned_sessions(),
                    &sender,
                ),
            };
            if resynced.is_err() {
                return;
            }
            response = next;
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
    /// One GET on this runtime's server, parsed, off the async runtime.
    fn get_json(&self, path: &str, action: &'static str) -> Result<Value, BridgeError> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| BridgeError::Adapter("OpenCode runtime is stopped".into()))?
            .clone();
        let url = self.api.url(&self.base_url, path, &self.directory);
        thread::Builder::new()
            .name("opencode-get".into())
            .spawn(move || -> Result<Value, BridgeError> {
                let response = client.get(url).timeout(Duration::from_secs(10)).send().map_err(http_error(action))?;
                checked_json(response, action)
            })
            .map_err(|error| BridgeError::Adapter(error.to_string()))?
            .join()
            .map_err(|_| BridgeError::Adapter(format!("Failed to {action}")))?
    }

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
        let url = self.api.url(&self.base_url, path, &self.directory);
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

    /// The session a 2.x request id belongs to: a subagent's request is
    /// answered on its child session. An id this process never saw (raised
    /// before a restart) can only have been the root's.
    fn pending_request(&self, request_id: &str) -> Option<opencode_v2::PendingRequest> {
        self.pending.lock().unwrap().get(request_id).cloned()
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
    /// One configured server's connection status, from the 2.x MCP list.
    /// OpenCode exposes no tool list, so `tools` is absent.
    fn mcp_status(&self, server: &str) -> Result<Value, BridgeError> {
        if self.api != ServerApi::V2 {
            return Err(BridgeError::Invalid("Personal apps need OpenCode 2 or newer".into()));
        }
        let listed = self.get_json("/api/mcp", "list OpenCode MCP servers")?;
        let entry = listed["data"]
            .as_array()
            .and_then(|servers| servers.iter().find(|entry| entry["name"].as_str() == Some(server)));
        Ok(match entry {
            Some(entry) => json!({"name": server, "status": entry.pointer("/status/status").and_then(Value::as_str).unwrap_or("unavailable")}),
            None => json!({"name": server, "status": "unavailable"}),
        })
    }
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
        self.send_turn_with_images(text, context, &[])
    }
    fn supports_images(&self) -> bool { true }
    fn send_turn_with_images(&self, text: &str, context: TurnContext<'_>, images: &[bridge_protocol::messages::TurnImage]) -> Result<(), BridgeError> {
        if self.api == ServerApi::V2 {
            self.request(
                reqwest::Method::POST,
                &format!("/api/session/{}/prompt", self.session_id),
                Some(prompt_body_v2(text, context, images)),
                "send OpenCode turn",
            )?;
            crate::context_inventory::record_runtime_inventory(
                &self.context_inventory,
                opencode_context_inventory(ContextLifecyclePhase::PerTurn)?,
            );
            return Ok(());
        }
        let mut body = prompt_body(
            self.model.as_ref(),
            self.variant.as_deref(),
            self.instructions.as_deref(),
            text,
            context,
        );
        append_images(&mut body, images);
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
        let path = match self.api {
            ServerApi::V1 => format!("/session/{}/abort", self.session_id),
            ServerApi::V2 => format!("/api/session/{}/interrupt", self.session_id),
        };
        self.request(reqwest::Method::POST, &path, None, "interrupt OpenCode turn")
    }
    fn stop_subagent(&self, provider_id: &str, _turn_id: Option<&str>) -> Result<(), BridgeError> {
        let path = match self.api {
            ServerApi::V1 => format!("/session/{provider_id}/abort"),
            ServerApi::V2 => format!("/api/session/{provider_id}/interrupt"),
        };
        self.request(reqwest::Method::POST, &path, None, "interrupt OpenCode subagent")
    }
    fn stop_subagent_request(&self, provider_id: &str, _turn_id: Option<&str>) -> Option<crate::adapters::InterruptRequest> {
        let client = self.client.as_ref()?.clone();
        let path = match self.api {
            ServerApi::V1 => format!("/session/{provider_id}/abort"),
            ServerApi::V2 => format!("/api/session/{provider_id}/interrupt"),
        };
        let url = self.api.url(&self.base_url, &path, &self.directory);
        Some(Box::new(move || {
            client.post(url).timeout(Duration::from_secs(10)).send()
                .and_then(Response::error_for_status)
                .map(|_| ()).map_err(http_error("interrupt OpenCode subagent"))
        }))
    }
    /// `GET /mcp` (V1) or `/api/mcp` (V2) on the server of this chat. The
    /// exchange runs on its own thread, so the client clone drops off any
    /// async runtime.
    fn mcp_list_request(&self) -> Option<crate::adapters::McpListRequest> {
        let client = self.client.as_ref()?.clone();
        let path = match self.api {
            ServerApi::V1 => "/mcp",
            ServerApi::V2 => "/api/mcp",
        };
        let url = self.api.url(&self.base_url, path, &self.directory);
        Some(Box::new(move || {
            let action = "list OpenCode MCP servers";
            thread::Builder::new()
                .name("opencode-mcp-list".into())
                .spawn(move || -> Result<Value, BridgeError> {
                    let response = client.get(url).timeout(Duration::from_secs(10)).send().map_err(http_error(action))?;
                    checked_json(response, action)
                })
                .map_err(|error| BridgeError::Adapter(error.to_string()))?
                .join()
                .map_err(|_| BridgeError::Adapter(format!("Failed to {action}")))?
        }))
    }
    fn interrupt_request(&self) -> Option<crate::adapters::InterruptRequest> {
        let client = self.client.as_ref()?.clone();
        let path = match self.api {
            ServerApi::V1 => format!("/session/{}/abort", self.session_id),
            ServerApi::V2 => format!("/api/session/{}/interrupt", self.session_id),
        };
        let url = self.api.url(&self.base_url, &path, &self.directory);
        Some(Box::new(move || {
            // Off the caller's thread; the steer watchdog does not wait on this.
            client.post(url).timeout(Duration::from_secs(5)).send()
                .and_then(Response::error_for_status)
                .map(|_| ()).map_err(http_error("interrupt OpenCode turn"))
        }))
    }
    /// `/session/{id}/summarize` names the model that writes the summary and
    /// takes no focus, so OpenCode compacts the whole session. Without a known
    /// model there is nothing to name, and the request would be rejected.
    fn native_compaction(&self) -> crate::adapters::NativeCompaction {
        compaction_support(self.api, self.model.as_ref())
    }
    fn compact_native(&self, _focus: Option<&str>) -> Result<(), BridgeError> {
        if self.api == ServerApi::V2 {
            return self.request(
                reqwest::Method::POST,
                &format!("/api/session/{}/compact", self.session_id),
                Some(json!({})),
                "compact the OpenCode session",
            );
        }
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
        if self.api == ServerApi::V2 {
            let session_id = match self.pending_request(request_id) {
                Some(opencode_v2::PendingRequest::Permission { session_id }) => session_id,
                _ => self.session_id.clone(),
            };
            validate_path_id("session id", &session_id)?;
            return self.request(
                reqwest::Method::POST,
                &format!("/api/session/{session_id}/permission/{request_id}/reply"),
                Some(json!({"decision": reply})),
                "resolve OpenCode permission",
            );
        }
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
        if self.api == ServerApi::V2 {
            let Some(opencode_v2::PendingRequest::Form { session_id, fields }) =
                self.pending_request(request_id)
            else {
                return Err(BridgeError::Invalid(
                    "OpenCode is no longer waiting on this question".into(),
                ));
            };
            validate_path_id("session id", &session_id)?;
            let answer = opencode_v2::form_answer(&fields, &answers).map_err(BridgeError::Invalid)?;
            return self.request(
                reqwest::Method::POST,
                &format!("/api/session/{session_id}/form/{request_id}/reply"),
                Some(json!({"answer": answer})),
                "answer OpenCode question",
            );
        }
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
        if self.api == ServerApi::V2 {
            let session_id = match self.pending_request(request_id) {
                Some(opencode_v2::PendingRequest::Form { session_id, .. }) => session_id,
                _ => self.session_id.clone(),
            };
            validate_path_id("session id", &session_id)?;
            return self.request(
                reqwest::Method::DELETE,
                &format!("/api/session/{session_id}/form/{request_id}"),
                None,
                "reject OpenCode question",
            );
        }
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

fn append_images(body: &mut Value, images: &[bridge_protocol::messages::TurnImage]) {
    body["parts"].as_array_mut().expect("prompt parts array").extend(images.iter().map(|image| {
        json!({"type": "file", "mime": image.media_type, "url": format!("data:{};base64,{}", image.media_type, image.base64_data)})
    }));
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
        // OpenCode 1.18.3+ appends this per-message system value after its own
        // provider, environment, MCP, instruction, and skill guidance. It does
        // not replace the built-in system prompt; keeping it role-separated is
        // stronger than converting Bridge authority into an ordinary text part.
        body["system"] = json!(system);
    }
    body
}

/// A 2.x prompt is one text with attachments beside it. Bridge's per-turn
/// context still precedes the user's words; its instructions are a session
/// entry, not part of the prompt.
fn prompt_body_v2(
    text: &str,
    context: TurnContext<'_>,
    images: &[bridge_protocol::messages::TurnImage],
) -> Value {
    let mut blocks = context
        .entries()
        .map(|entry| entry.value.to_string())
        .collect::<Vec<_>>();
    blocks.push(text.to_owned());
    let mut body = json!({ "text": blocks.join("\n\n") });
    if !images.is_empty() {
        body["files"] = Value::Array(
            images
                .iter()
                .enumerate()
                .map(|(index, image)| {
                    json!({
                        "uri": format!("data:{};base64,{}", image.media_type, image.base64_data),
                        "name": format!("image-{}", index + 1),
                    })
                })
                .collect(),
        );
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

/// `--version` prints `1.18.32` on 1.x and `opencode v2.1.1` on 2.x.
fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let value = version.trim();
    let value = value.strip_prefix("opencode ").unwrap_or(value).trim();
    let value = value.strip_prefix('v').unwrap_or(value);
    let parts = value.split('.').collect::<Vec<_>>();
    if parts.len() != 3
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.chars().all(|character| character.is_ascii_digit()))
    {
        return None;
    }
    let parsed = parts
        .iter()
        .map(|part| part.parse::<u64>())
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    Some((parsed[0], parsed[1], parsed[2]))
}

pub fn is_supported_version(version: &str) -> bool {
    parse_version(version).is_some_and(|version| version >= MINIMUM_VERSION)
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
    discover_for(settings, directory, None)
}

/// [`discover`] as an extra account sees it: its own data folder, its own
/// connected providers.
pub fn discover_for(
    settings: &OpenCodeSettings,
    directory: &str,
    account: Option<&crate::provider_accounts::AccountLaunch>,
) -> Result<OpenCodeCatalog, BridgeError> {
    let executable = resolve_executable(settings)?;
    let version = ensure_supported_version(&executable)?;
    let api = ServerApi::of(&version);
    if api == ServerApi::V2 {
        return with_control_server_for(&executable, directory, api, account, |client, base_url| {
            let get = |path: &str, action: &'static str| {
                client
                    .get(format!("{base_url}{path}"))
                    .timeout(Duration::from_secs(20))
                    .send()
                    .map_err(http_error(action))
                    .and_then(|response| checked_json(response, action))
                    .map(|value| value.get("data").cloned().unwrap_or(Value::Null))
            };
            let integrations = get("/api/integration", "discover OpenCode integrations")?;
            let providers = get("/api/provider", "discover OpenCode providers")?;
            let models = get("/api/model", "discover OpenCode models")?;
            // No default is a valid answer (nothing connected yet).
            let default_model = get("/api/model/default", "read the OpenCode default model").ok();
            opencode_v2::parse_catalog(
                &integrations,
                &providers,
                &models,
                default_model.as_ref(),
                executable.to_string_lossy().as_ref(),
                &version,
            )
            .map_err(BridgeError::Adapter)
        });
    }
    with_control_server_for(&executable, directory, api, account, |client, base_url| {
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
    set_provider_api_key_for(settings, directory, provider_id, api_key, None)
}

/// Connect a provider key inside an extra account's data folder: a private
/// OpenCode server for that folder stores it, so Bridge never holds it.
pub fn set_provider_api_key_for(
    settings: &OpenCodeSettings,
    directory: &str,
    provider_id: &str,
    api_key: &str,
    account: Option<&crate::provider_accounts::AccountLaunch>,
) -> Result<OpenCodeCatalog, BridgeError> {
    validate_provider_id(provider_id)?;
    if api_key.trim().is_empty() {
        return Err(BridgeError::Invalid(
            "Provider API key cannot be empty".into(),
        ));
    }
    let executable = resolve_executable(settings)?;
    let api = ServerApi::of(&ensure_supported_version(&executable)?);
    with_control_server_for(&executable, directory, api, account, |client, base_url| {
        if api == ServerApi::V2 {
            let integration_id = v2_integration_for(client, base_url, provider_id)?;
            return checked_status(
                client
                    .post(format!("{base_url}/api/integration/{integration_id}/connect/key"))
                    .timeout(Duration::from_secs(20))
                    .json(&json!({"key": api_key.trim()}))
                    .send()
                    .map_err(http_error("save OpenCode provider authentication"))?,
                "save OpenCode provider authentication",
            );
        }
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
    discover_for(settings, directory, account)
}

pub fn remove_provider_auth(
    settings: &OpenCodeSettings,
    directory: &str,
    provider_id: &str,
) -> Result<OpenCodeCatalog, BridgeError> {
    validate_provider_id(provider_id)?;
    let executable = resolve_executable(settings)?;
    let api = ServerApi::of(&ensure_supported_version(&executable)?);
    with_control_server(&executable, directory, api, |client, base_url| {
        if api == ServerApi::V2 {
            let integration_id = v2_integration_for(client, base_url, provider_id)?;
            let integrations = checked_json(
                client
                    .get(format!("{base_url}/api/integration"))
                    .timeout(Duration::from_secs(20))
                    .send()
                    .map_err(http_error("read OpenCode credentials"))?,
                "read OpenCode credentials",
            )?;
            let credentials = opencode_v2::stored_credentials(
                integrations.get("data").unwrap_or(&Value::Null),
                &integration_id,
            );
            if credentials.is_empty() {
                return Err(BridgeError::Invalid(format!(
                    "OpenCode has no stored credential for {provider_id}; it may come from an environment variable"
                )));
            }
            for credential in credentials {
                validate_path_id("credential id", &credential)?;
                checked_status(
                    client
                        .delete(format!("{base_url}/api/credential/{credential}"))
                        .timeout(Duration::from_secs(20))
                        .send()
                        .map_err(http_error("remove OpenCode provider authentication"))?,
                    "remove OpenCode provider authentication",
                )?;
            }
            return Ok(());
        }
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

/// The 2.x integration that stores a provider's credentials.
fn v2_integration_for(client: &Client, base_url: &str, provider_id: &str) -> Result<String, BridgeError> {
    let providers = checked_json(
        client
            .get(format!("{base_url}/api/provider"))
            .timeout(Duration::from_secs(20))
            .send()
            .map_err(http_error("read OpenCode providers"))?,
        "read OpenCode providers",
    )?;
    let integration_id =
        opencode_v2::integration_for(providers.get("data").unwrap_or(&Value::Null), provider_id);
    validate_provider_id(&integration_id)?;
    Ok(integration_id)
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
    api: ServerApi,
    action: impl FnOnce(&Client, &str) -> Result<T, BridgeError>,
) -> Result<T, BridgeError> {
    with_control_server_for(executable, directory, api, None, action)
}

fn with_control_server_for<T>(
    executable: &Path,
    directory: &str,
    api: ServerApi,
    account: Option<&crate::provider_accounts::AccountLaunch>,
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
    binary::hydrate_command_path(&mut command);
    command
        .current_dir(directory)
        .env("OPENCODE_SERVER_USERNAME", "bridge")
        .env("OPENCODE_SERVER_PASSWORD", &password)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::provider_accounts::apply_account(&mut command, account);
    crate::adapters::configure_process_group(&mut command);
    let child = command.spawn()?;
    let ledger = crate::process_ledger::record_launch("opencode.control", directory, child.id());
    let mut server = ControlServer {
        child,
        _ledger: ledger,
    };
    match build_authenticated_client(api, &password, directory) {
        Ok(client) => {
            let result = wait_until_ready(&client, &base_url, &mut server.child, api, STARTUP_DEADLINE)
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
            speed_tiers: Vec::new(),
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

/// OpenCode 2.x keeps credentials in its database and only imports a legacy
/// `auth.json`, so a fresh 2.x install has none. A discovered catalog with a
/// provider connected through a stored credential is the same proof.
pub fn auth_state_with_catalog(catalog: Option<&OpenCodeCatalog>) -> AuthState {
    let stored = catalog.is_some_and(|catalog| {
        catalog
            .providers
            .iter()
            .any(|provider| provider.connected && provider.source.as_deref() == Some("api"))
    });
    if stored {
        AuthState::SignedIn
    } else {
        auth_state()
    }
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

    fn session_tree_fixture() -> Value {
        serde_json::from_str(include_str!(
            "../../../testing/fixtures/opencode-sse-session-tree-v1.json"
        ))
        .expect("fixture is valid JSON")
    }

    /// A loopback stand-in for `opencode serve` that scripts how `/event`
    /// behaves per connection and answers the resync endpoints.
    fn fake_opencode_server(refuse_reconnects: bool) -> (String, Arc<Mutex<Vec<String>>>) {
        use std::io::Write as _;
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let paths = Arc::new(Mutex::new(Vec::<String>::new()));
        let event_requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = paths.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let seen = seen.clone();
                let event_requests = event_requests.clone();
                thread::spawn(move || {
                    let mut head = std::io::BufReader::new(stream.try_clone().unwrap());
                    let mut request_line = String::new();
                    if head.read_line(&mut request_line).is_err() {
                        return;
                    }
                    loop {
                        let mut header = String::new();
                        if head.read_line(&mut header).is_err() || header == "\r\n" || header.is_empty() {
                            break;
                        }
                    }
                    let path = request_line.split_whitespace().nth(1).unwrap_or("/").to_owned();
                    seen.lock().unwrap().push(path.clone());
                    let json = |stream: &mut std::net::TcpStream, body: &str| {
                        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
                    };
                    if path.starts_with("/event") {
                        let attempt = event_requests.fetch_add(1, Ordering::SeqCst);
                        if attempt >= 1 && (refuse_reconnects || attempt >= 2) {
                            let _ = write!(stream, "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                            return;
                        }
                        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n");
                        if attempt == 0 {
                            // First body: one frame, then the server drops the socket.
                            let _ = write!(stream, "data: {}\n\n", json!({"type":"session.status","properties":{"sessionID":"ses_root","status":{"type":"busy"}}}));
                            let _ = stream.flush();
                            return;
                        }
                        // Second body: alive again; hold it open for the test.
                        let _ = write!(stream, "data: {}\n\n", json!({"type":"server.heartbeat","properties":{}}));
                        let _ = stream.flush();
                        thread::sleep(Duration::from_secs(3));
                    } else if path.starts_with("/session/ses_root/message") {
                        json(&mut stream, &json!([{
                            "info": {"id": "msg_1", "role": "assistant", "sessionID": "ses_root", "time": {"created": 1, "completed": 2}},
                            "parts": [{"id": "prt_1", "messageID": "msg_1", "sessionID": "ses_root", "type": "text", "text": "finished in the dark", "time": {"start": 1, "end": 2}}]
                        }]).to_string());
                    } else if path.starts_with("/session/status") {
                        json(&mut stream, "{}");
                    } else {
                        let _ = write!(stream, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                });
            }
        });
        (base_url, paths)
    }

    fn fast_policy() -> EventStreamPolicy {
        EventStreamPolicy { reconnect_backoff: vec![Duration::from_millis(1); 5] }
    }

    fn frame_type(frame: &str) -> String {
        let value: Value = serde_json::from_str(frame.trim()).unwrap();
        value["type"].as_str().unwrap().to_owned()
    }

    #[test]
    fn stream_reconnects_after_a_dropped_body_and_resyncs_the_turn() {
        let (base_url, paths) = fake_opencode_server(false);
        let client = build_authenticated_client(ServerApi::V1, "pw", "/tmp").unwrap();
        let (sender, receiver, _metrics) =
            crate::frame_queue::bounded_frame_queue(crate::frame_queue::QueueBudget::default());
        spawn_event_stream(
            client,
            base_url,
            "/repo".into(),
            "ses_root".into(),
            sender,
            Arc::new(AtomicBool::new(false)),
            fast_policy(),
            ServerApi::V1,
            Default::default(),
        )
        .unwrap();
        let mut frames = Vec::new();
        for _ in 0..5 {
            frames.push(receiver.recv().expect("the stream survives the drop"));
        }
        assert_eq!(
            frames.iter().map(|frame| frame_type(frame)).collect::<Vec<_>>(),
            vec!["session.status", "message.updated", "message.part.updated", "session.status", "server.heartbeat"],
            "busy from the first body, the resynced snapshot, the synthetic idle, then the live stream again"
        );
        let idle: Value = serde_json::from_str(&frames[3]).unwrap();
        assert_eq!(idle["properties"]["status"]["type"], "idle");
        let snapshot: Value = serde_json::from_str(&frames[2]).unwrap();
        assert_eq!(snapshot["properties"]["sessionID"], "ses_root", "the flat runtime shape the filter and normalizer read");
        assert_eq!(snapshot["properties"]["part"]["text"], "finished in the dark");
        let paths = paths.lock().unwrap();
        assert!(!paths.iter().any(|path| path.contains("limit=")), "resync fetches full histories, never a limit=1 latest-message assumption: {paths:?}");
        assert!(paths.iter().any(|path| path.starts_with("/session/status")), "{paths:?}");
        assert_eq!(paths.iter().filter(|path| path.starts_with("/event")).count(), 2);
    }

    #[test]
    fn stream_gives_up_after_the_retry_budget_with_one_disconnect_error() {
        let (base_url, paths) = fake_opencode_server(true);
        let client = build_authenticated_client(ServerApi::V1, "pw", "/tmp").unwrap();
        let (sender, receiver, _metrics) =
            crate::frame_queue::bounded_frame_queue(crate::frame_queue::QueueBudget::default());
        spawn_event_stream(
            client,
            base_url,
            "/repo".into(),
            "ses_root".into(),
            sender,
            Arc::new(AtomicBool::new(false)),
            fast_policy(),
            ServerApi::V1,
            Default::default(),
        )
        .unwrap();
        assert_eq!(frame_type(&receiver.recv().unwrap()), "session.status");
        let error = receiver.recv().unwrap();
        assert_eq!(frame_type(&error), "session.error");
        assert!(error.contains("disconnected unexpectedly"), "{error}");
        assert!(receiver.recv().is_err(), "the reader thread ends after surfacing the disconnect once");
        let paths = paths.lock().unwrap();
        assert_eq!(paths.iter().filter(|path| path.starts_with("/event")).count(), 1 + RECONNECT_BACKOFF_SECONDS.len(), "{paths:?}");
        assert!(!paths.iter().any(|path| path.starts_with("/session")), "no resync without a reconnect");
    }

    #[test]
    fn shutdown_ends_the_stream_without_a_reconnect_or_an_error() {
        let (base_url, paths) = fake_opencode_server(false);
        let client = build_authenticated_client(ServerApi::V1, "pw", "/tmp").unwrap();
        let (sender, receiver, _metrics) =
            crate::frame_queue::bounded_frame_queue(crate::frame_queue::QueueBudget::default());
        let shutting_down = Arc::new(AtomicBool::new(false));
        spawn_event_stream(
            client,
            base_url,
            "/repo".into(),
            "ses_root".into(),
            sender,
            shutting_down.clone(),
            EventStreamPolicy { reconnect_backoff: vec![Duration::from_millis(200)] },
            ServerApi::V1,
            Default::default(),
        )
        .unwrap();
        assert_eq!(frame_type(&receiver.recv().unwrap()), "session.status");
        shutting_down.store(true, Ordering::SeqCst);
        assert!(receiver.recv().is_err(), "no synthetic error after a deliberate stop");
        assert_eq!(paths.lock().unwrap().iter().filter(|path| path.starts_with("/event")).count(), 1);
    }

    #[test]
    fn filter_admits_the_root_and_its_descendants_and_counts_the_rest() {
        let mut filter = SessionFrameFilter::new("root".into());
        let created = |id: &str, parent: Option<&str>| {
            let mut info = json!({"id": id, "title": id});
            if let Some(parent) = parent {
                info["parentID"] = json!(parent);
            }
            json!({"type": "session.created", "properties": {"sessionID": id, "info": info}})
        };
        let part = |session: &str| {
            json!({"type": "message.part.updated", "properties": {"sessionID": session, "part": {"id": "p", "messageID": "m", "type": "text"}}})
        };
        assert_eq!(filter.classify(&part("root")), FrameDisposition::Forward);
        // A child announced with our id as parent joins the tree...
        assert_eq!(filter.classify(&created("child", Some("root"))), FrameDisposition::Forward);
        assert_eq!(filter.classify(&part("child")), FrameDisposition::Forward);
        assert!(filter.is_child("child") && !filter.is_child("root"));
        // ...transitively.
        assert_eq!(filter.classify(&created("grandchild", Some("child"))), FrameDisposition::Forward);
        assert_eq!(filter.classify(&part("grandchild")), FrameDisposition::Forward);
        // A session with no parent, or a parent we do not own, stays foreign.
        assert_eq!(filter.classify(&created("stranger", None)), FrameDisposition::Foreign);
        assert_eq!(filter.classify(&created("orphan", Some("stranger"))), FrameDisposition::Foreign);
        assert_eq!(filter.classify(&part("stranger")), FrameDisposition::Foreign);
    }

    #[test]
    fn filter_admits_unattributed_session_errors_and_heartbeats() {
        let mut filter = SessionFrameFilter::new("root".into());
        assert_eq!(
            filter.classify(&json!({"type": "session.error", "properties": {"error": {"message": "plugin died"}}})),
            FrameDisposition::Forward
        );
        assert_eq!(
            filter.classify(&json!({"type": "server.heartbeat", "properties": {}})),
            FrameDisposition::Forward
        );
        assert_eq!(
            filter.classify(&json!({"type": "server.connected", "properties": {}})),
            FrameDisposition::Forward
        );
        // An error attributed to a stranger is still theirs.
        assert_eq!(
            filter.classify(&json!({"type": "session.error", "properties": {"sessionID": "other", "error": {}}})),
            FrameDisposition::Foreign
        );
        // Any other id-less frame is not ours to guess at.
        assert_eq!(
            filter.classify(&json!({"type": "todo.updated", "properties": {"todos": []}})),
            FrameDisposition::Foreign
        );
    }

    #[test]
    fn filter_reads_the_flat_session_id_first() {
        // Runtime schema: `properties.sessionID` is flat. This is the shape the
        // captured transcript carries and the one the filter must key on.
        let mut filter = SessionFrameFilter::new("root".into());
        let runtime_shape = json!({"type": "message.part.updated", "properties": {"sessionID": "root", "part": {"id": "p", "messageID": "m", "sessionID": "other", "type": "text"}}});
        assert_eq!(filter.classify(&runtime_shape), FrameDisposition::Forward, "flat id wins even when the nested one disagrees");
        // SDK types: the id only under `part`. Accepted as a fallback so a
        // future drift attributes instead of dropping every part update.
        let sdk_shape = json!({"type": "message.part.updated", "properties": {"part": {"id": "p", "messageID": "m", "sessionID": "root", "type": "text"}}});
        assert_eq!(filter.classify(&sdk_shape), FrameDisposition::Forward);
        let sdk_foreign = json!({"type": "message.part.updated", "properties": {"part": {"id": "p", "messageID": "m", "sessionID": "other", "type": "text"}}});
        assert_eq!(filter.classify(&sdk_foreign), FrameDisposition::Foreign);
    }

    #[test]
    fn session_tree_fixture_replays_with_the_recorded_dispositions() {
        let fixture = session_tree_fixture();
        let root = fixture["root"].as_str().unwrap();
        let mut filter = SessionFrameFilter::new(root.into());
        let (sender, _receiver, metrics) =
            crate::frame_queue::bounded_frame_queue(crate::frame_queue::QueueBudget::default());
        let mut foreign = 0;
        for (index, line) in fixture["lines"].as_array().unwrap().iter().enumerate() {
            let expect = line["expect"].as_str().unwrap();
            let frame = &line["data"];
            let disposition = filter.classify(frame);
            let event_type = frame.get("type").and_then(Value::as_str).unwrap_or("");
            let session = frame.get("properties").and_then(|properties| agent::opencode_frame_session(event_type, properties));
            match expect {
                "own" => {
                    assert_eq!(disposition, FrameDisposition::Forward, "line {index}");
                    assert!(session.is_some_and(|id| id == root), "line {index} is a root frame");
                }
                "child" => {
                    assert_eq!(disposition, FrameDisposition::Forward, "line {index}");
                    assert!(session.is_some_and(|id| filter.is_child(id)), "line {index} is child work");
                }
                "admit" => {
                    assert_eq!(disposition, FrameDisposition::Forward, "line {index}");
                    assert!(session.is_none(), "line {index} carries no session id");
                }
                "foreign" => {
                    assert_eq!(disposition, FrameDisposition::Foreign, "line {index}");
                    sender.record_foreign_drop();
                    filter.note_foreign(frame);
                    foreign += 1;
                }
                other => panic!("unknown disposition {other}"),
            }
        }
        assert!(foreign > 0, "the fixture exercises the drop path");
        assert_eq!(metrics.snapshot().dropped_foreign, foreign);
        assert_eq!(filter.reported_foreign.len(), 1, "one stranger, logged once");
    }

    #[test]
    fn filter_stamps_an_unattributed_error_with_the_root_session() {
        let mut filter = SessionFrameFilter::new("ses_root".into());
        let mut error = json!({"type": "session.error", "properties": {"error": {"message": "plugin died"}}});
        assert_eq!(
            filter.classify(&error),
            FrameDisposition::Forward,
            "an id-less error on a private server is ours"
        );
        filter.stamp_unattributed_error(&mut error);
        assert_eq!(error["properties"]["sessionID"], "ses_root");
        // The stamped frame now keys to the root everywhere: the shared
        // helper the registry router and normalizer use agrees with the
        // filter, so it can never be forwarded then filed under "default".
        let event_type = error.get("type").and_then(Value::as_str).unwrap_or("");
        let session = error.get("properties").and_then(|properties| agent::opencode_frame_session(event_type, properties));
        assert_eq!(session, Some("ses_root"));
        // Attributed frames and non-errors pass through untouched.
        let mut owned = json!({"type": "session.error", "properties": {"sessionID": "ses_root", "error": {}}});
        filter.stamp_unattributed_error(&mut owned);
        assert_eq!(owned["properties"]["sessionID"], "ses_root");
        let mut heartbeat = json!({"type": "server.heartbeat", "properties": {}});
        filter.stamp_unattributed_error(&mut heartbeat);
        assert!(heartbeat["properties"].get("sessionID").is_none());
        // Fallback session shapes use the same helper: an SDK-shaped owned
        // frame admitted by the filter is not later keyed as default.
        let sdk_shape = json!({"type": "message.part.updated", "properties": {"part": {"id": "p", "messageID": "m", "sessionID": "ses_root", "type": "text"}}});
        assert_eq!(
            agent::opencode_frame_session("message.part.updated", &sdk_shape["properties"]),
            Some("ses_root")
        );
    }

    /// A loopback server whose resync answers carry no assistant message: the
    /// reader must still check `/session/status` and close a turn that
    /// finished in the dark, without ever sending a `limit=1` query.
    #[test]
    fn stream_reconnects_when_no_assistant_snapshot_is_available() {
        use std::io::Write as _;
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let paths = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen = paths.clone();
        let event_requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let seen = seen.clone();
                let event_requests = event_requests.clone();
                thread::spawn(move || {
                    let mut head = std::io::BufReader::new(stream.try_clone().unwrap());
                    let mut request_line = String::new();
                    if head.read_line(&mut request_line).is_err() {
                        return;
                    }
                    loop {
                        let mut header = String::new();
                        if head.read_line(&mut header).is_err() || header == "\r\n" || header.is_empty() {
                            break;
                        }
                    }
                    let path = request_line.split_whitespace().nth(1).unwrap_or("/").to_owned();
                    seen.lock().unwrap().push(path.clone());
                    let json = |stream: &mut std::net::TcpStream, body: &str| {
                        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
                    };
                    if path.starts_with("/event") {
                        let attempt = event_requests.fetch_add(1, Ordering::SeqCst);
                        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n");
                        if attempt == 0 {
                            let _ = write!(stream, "data: {}\n\n", json!({"type":"session.status","properties":{"sessionID":"ses_root","status":{"type":"busy"}}}));
                            let _ = stream.flush();
                            return;
                        }
                        let _ = write!(stream, "data: {}\n\n", json!({"type":"server.heartbeat","properties":{}}));
                        let _ = stream.flush();
                        thread::sleep(Duration::from_secs(3));
                    } else if path.starts_with("/session/ses_root/message") {
                        // The turn finished in the dark with no assistant row —
                        // e.g. the last message is the user's — so there is
                        // nothing to replay, only a status to check.
                        json(&mut stream, &json!([{
                            "info": {"id": "msg_0", "role": "user", "sessionID": "ses_root", "time": {"created": 1}},
                            "parts": []
                        }]).to_string());
                    } else if path.starts_with("/session/status") {
                        json(&mut stream, "{}");
                    } else {
                        let _ = write!(stream, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                });
            }
        });
        let client = build_authenticated_client(ServerApi::V1, "pw", "/tmp").unwrap();
        let (sender, receiver, _metrics) =
            crate::frame_queue::bounded_frame_queue(crate::frame_queue::QueueBudget::default());
        spawn_event_stream(
            client,
            base_url,
            "/repo".into(),
            "ses_root".into(),
            sender,
            Arc::new(AtomicBool::new(false)),
            fast_policy(),
            ServerApi::V1,
            Default::default(),
        )
        .unwrap();
        let mut frames = Vec::new();
        for _ in 0..3 {
            frames.push(receiver.recv().expect("busy, synthetic idle, then the live stream"));
        }
        assert_eq!(
            frames.iter().map(|frame| frame_type(frame)).collect::<Vec<_>>(),
            vec!["session.status", "session.status", "server.heartbeat"],
            "no assistant snapshot, yet the turn still closes via status"
        );
        let idle: Value = serde_json::from_str(&frames[1]).unwrap();
        assert_eq!(idle["properties"]["status"]["type"], "idle");
        let paths = paths.lock().unwrap();
        assert!(!paths.iter().any(|path| path.contains("limit=")), "no unverified limit=1 ordering assumption: {paths:?}");
        assert!(paths.iter().any(|path| path.starts_with("/session/status")), "{paths:?}");
    }

    /// After a reconnect the reader replays every owned session — root and
    /// already-known children — so a subagent mid-run when the socket drops
    /// gets its catch-up too.
    #[test]
    fn resync_replays_owned_child_sessions_after_a_disconnect() {
        let client = build_authenticated_client(ServerApi::V1, "pw", "/tmp").unwrap();
        let (sender, receiver, _metrics) =
            crate::frame_queue::bounded_frame_queue(crate::frame_queue::QueueBudget::default());
        // Serve one assistant snapshot per owned session from a loopback
        // server, then verify the replayed frames carry each session's id.
        use std::io::Write as _;
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                thread::spawn(move || {
                    let mut head = std::io::BufReader::new(stream.try_clone().unwrap());
                    let mut request_line = String::new();
                    if head.read_line(&mut request_line).is_err() {
                        return;
                    }
                    loop {
                        let mut header = String::new();
                        if head.read_line(&mut header).is_err() || header == "\r\n" || header.is_empty() {
                            break;
                        }
                    }
                    let path = request_line.split_whitespace().nth(1).unwrap_or("/").to_owned();
                    let json = |stream: &mut std::net::TcpStream, body: &str| {
                        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
                    };
                    if path.starts_with("/session/") && path.contains("/message") {
                        let session = path.split('/').nth(2).unwrap_or("ses_root");
                        let short = &session[4..session.len().min(8)];
                        json(&mut stream, &json!([{
                            "info": {"id": format!("msg_{short}"), "role": "assistant", "sessionID": session, "time": {"created": 1, "completed": 2}},
                            "parts": [{"id": format!("prt_{short}"), "messageID": format!("msg_{short}"), "sessionID": session, "type": "text", "text": format!("work from {session}"), "time": {"start": 1, "end": 2}}]
                        }]).to_string());
                    } else if path.starts_with("/session/status") {
                        json(&mut stream, &json!({"ses_root": {"type": "busy"}}).to_string());
                    } else {
                        let _ = write!(stream, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                });
            }
        });
        resync_after_reconnect(
            &client,
            &base_url,
            "/repo",
            "ses_root",
            &["ses_root".to_owned(), "ses_child".to_owned()],
            &sender,
        )
        .expect("resync delivers");
        let mut seen = std::collections::HashSet::new();
        for _ in 0..4 {
            let frame = receiver.recv().expect("root + child snapshots");
            let value: Value = serde_json::from_str(&frame).unwrap();
            seen.insert((
                value["type"].as_str().unwrap().to_owned(),
                value["properties"]["sessionID"].as_str().unwrap().to_owned(),
            ));
        }
        assert!(seen.contains(&("message.updated".to_owned(), "ses_root".to_owned())), "{seen:?}");
        assert!(seen.contains(&("message.part.updated".to_owned(), "ses_root".to_owned())), "{seen:?}");
        assert!(seen.contains(&("message.updated".to_owned(), "ses_child".to_owned())), "{seen:?}");
        assert!(seen.contains(&("message.part.updated".to_owned(), "ses_child".to_owned())), "{seen:?}");
    }
    #[test]
    fn image_attachments_keep_native_shapes_and_order() {
        let images = vec![
            bridge_protocol::messages::TurnImage { media_type: "image/png".into(), base64_data: "cG5n".into() },
            bridge_protocol::messages::TurnImage { media_type: "image/jpeg".into(), base64_data: "anBlZw==".into() },
        ];
        let mut payload = prompt_body(None, None, Some("trusted instructions"), "describe", TurnContext::default());
        append_images(&mut payload, &images);
        assert_eq!(payload["system"], "trusted instructions");
        assert_eq!(payload["parts"][0]["text"], "describe");
        assert_eq!(payload["parts"][1], json!({"type":"file", "mime":"image/png", "url":"data:image/png;base64,cG5n"}));
        assert_eq!(payload["parts"][2]["mime"], "image/jpeg");
    }

    use crate::context_inventory::ContextObservationProvenance;

    /// G9: the turn's `system` value must be a function of the launch alone.
    /// It is rebuilt on every turn, so anything per-turn folded into it — a
    /// credential contract for a `[secret:]` marker, say — invalidated that
    /// turn's prefix by itself.
    #[test]
    fn a_runtime_with_no_model_cannot_summarize() {
        // The endpoint requires the model that writes the summary. Claiming
        // the capability here would route /compact to a call that fails on the
        // wire instead of to the Bridge checkpoint that would have worked.
        assert_eq!(
            compaction_support(ServerApi::V1, None),
            crate::adapters::NativeCompaction::Unsupported
        );
        assert_eq!(
            compaction_support(ServerApi::V1, Some(&ModelRef {
                provider_id: "anthropic".into(),
                model_id: "claude-sonnet-4-5".into(),
            })),
            crate::adapters::NativeCompaction::WholeConversation
        );
    }

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
    fn the_server_api_follows_the_major_version() {
        assert_eq!(ServerApi::of("1.18.32"), ServerApi::V1);
        assert_eq!(ServerApi::of("opencode v2.1.1"), ServerApi::V2);
        assert_eq!(ServerApi::of("3.0.0"), ServerApi::V2);
        assert_eq!(ServerApi::V2.username(), "opencode");
        assert_eq!(ServerApi::V2.health_path(), "/api/info");
        assert_eq!(
            ServerApi::V2.url("http://127.0.0.1:1", "/api/session", "/repo"),
            "http://127.0.0.1:1/api/session",
            "2.x is scoped by header, not query"
        );
        assert_eq!(encode_directory("/Users/me/my repo/é"), "/Users/me/my%20repo/%C3%A9");
    }

    #[test]
    fn saved_approvals_that_cover_the_courier_tool_are_found() {
        let saved = |rules: Value| json!({"data": rules});
        assert!(saved_allows(&saved(json!([{"id":"a","action":"fake_send_note","resource":"*"}])), "fake_send_note"));
        assert!(saved_allows(&saved(json!([{"id":"a","action":"fake_*","resource":"*"}])), "fake_send_note"));
        assert!(saved_allows(&saved(json!([{"id":"a","action":"*","resource":"*"}])), "fake_send_note"));
        assert!(!saved_allows(&saved(json!([{"id":"a","action":"fake_read","resource":"*"}])), "fake_send_note"));
        assert!(!saved_allows(&saved(json!([{"id":"a","action":"fake_send_note","effect":"deny"}])), "fake_send_note"));
        assert!(!saved_allows(&saved(json!([])), "fake_send_note"));
        assert_eq!(percent_encode_segment("claude.ai Slack"), "claude.ai%20Slack");
    }

    #[test]
    fn a_courier_session_denies_all_but_code_mode_and_asks_for_one_tool() {
        let body = v2_session_body(courier_session_body(None, None, "fake_send_note"));
        assert_eq!(body["permissions"], json!([
            {"action":"*","resource":"*","effect":"deny"},
            {"action":"execute","resource":"*","effect":"allow"},
            {"action":"fake_send_note","resource":"*","effect":"ask"},
        ]), "the last matching rule wins, so the ask comes last");
    }

    #[test]
    fn v2_sessions_carry_the_model_and_v2_permission_rules() {
        let model = ModelRef { provider_id: "opencode-go".into(), model_id: "kimi-k3".into() };
        let body = v2_session_body(session_create_body(Some(&model), Some("high"), Some(WriteMode::Isolated)));
        assert_eq!(body["model"], json!({"providerID":"opencode-go","id":"kimi-k3","variant":"high"}));
        let rules = body["permissions"].as_array().unwrap();
        assert!(rules.contains(&json!({"action":"shell","resource":"*","effect":"ask"})));
        assert!(rules.contains(&json!({"action":"edit","resource":"*","effect":"ask"})));
        assert!(body.get("permission").is_none());
        let completion = v2_session_body(completion_session_body(Some(&model), None));
        assert_eq!(completion["permissions"], json!([{"action":"*","resource":"*","effect":"deny"}]));
    }

    #[test]
    fn v2_prompts_put_turn_context_first_and_images_beside_the_text() {
        let context = TurnContext { session: Some("bridge frame"), credentials: None };
        let images = [bridge_protocol::messages::TurnImage {
            media_type: "image/png".into(),
            base64_data: "iVBORw0KGgo=".into(),
        }];
        let body = prompt_body_v2("describe", context, &images);
        assert_eq!(body["text"], "bridge frame\n\ndescribe");
        assert_eq!(body["files"], json!([{"uri":"data:image/png;base64,iVBORw0KGgo=","name":"image-1"}]));
        assert!(body.get("system").is_none(), "instructions are a session entry on 2.x");
        assert!(prompt_body_v2("hi", TurnContext::default(), &[]).get("files").is_none());
    }

    #[test]
    fn v2_compacts_without_naming_a_model() {
        assert_eq!(
            compaction_support(ServerApi::V2, None),
            crate::adapters::NativeCompaction::WholeConversation
        );
    }

    #[test]
    fn a_v2_stream_is_translated_before_it_is_filtered() {
        use std::io::Write as _;
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else { return };
            let mut head = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            head.read_line(&mut request_line).unwrap();
            assert!(request_line.starts_with("GET /api/event "), "{request_line}");
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n");
            for frame in [
                json!({"id":"evt_1","type":"server.connected","data":{}}),
                json!({"id":"evt_2","type":"session.execution.started","data":{"sessionID":"ses_other"}}),
                json!({"id":"evt_3","type":"session.execution.started","data":{"sessionID":"ses_root"}}),
                json!({"id":"evt_4","type":"session.text.delta","data":{"sessionID":"ses_root","assistantMessageID":"msg_1","ordinal":0,"delta":"hi"}}),
            ] {
                let _ = write!(stream, "id: {}\nevent: message\ndata: {frame}\n\n", frame["id"]);
            }
            let _ = stream.flush();
            thread::sleep(Duration::from_secs(2));
        });
        let client = build_authenticated_client(ServerApi::V2, "pw", "/repo").unwrap();
        let (sender, receiver, metrics) =
            crate::frame_queue::bounded_frame_queue(crate::frame_queue::QueueBudget::default());
        let shutting_down = Arc::new(AtomicBool::new(false));
        spawn_event_stream(
            client,
            base_url,
            "/repo".into(),
            "ses_root".into(),
            sender,
            shutting_down.clone(),
            fast_policy(),
            ServerApi::V2,
            Default::default(),
        )
        .unwrap();
        let frames = (0..3)
            .map(|_| serde_json::from_str::<Value>(receiver.recv().unwrap().trim()).unwrap())
            .collect::<Vec<_>>();
        shutting_down.store(true, Ordering::SeqCst);
        assert_eq!(frames[0]["type"], "server.connected");
        assert_eq!(frames[1], json!({"type":"session.status","properties":{"sessionID":"ses_root","status":{"type":"busy"}}}));
        assert_eq!(frames[2]["type"], "message.part.delta");
        assert_eq!(frames[2]["properties"]["partID"], "msg_1:text:0");
        assert_eq!(metrics.snapshot().dropped_foreign, 1, "another session's frame is still dropped");
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
        assert!(is_supported_version("1.19.0"));
        assert!(is_supported_version("2.0.0"));
        assert!(!is_supported_version("1.18"));
        assert!(!is_supported_version("build 9"));
        assert!(is_supported_version("opencode 1.18.29"));
        assert!(is_supported_version("opencode v2.1.1"));
        assert!(!is_supported_version("opencode v1.17.0"));
        assert!(!is_supported_version("1.18.3-beta.1"));
        assert!(!is_supported_version("1.18.3.1"));
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
    fn completion_session_denies_every_tool_without_altering_worker_permissions() {
        let model = ModelRef { provider_id: "opencode-go".into(), model_id: "test-model".into() };
        let body = completion_session_body(Some(&model), None);
        assert_eq!(body["permission"], json!([{"permission":"*", "pattern":"*", "action":"deny"}]));
        assert_eq!(body["model"]["providerID"], "opencode-go");
        assert_eq!(body["model"]["id"], "test-model");
        assert!(permission_rules(Some(WriteMode::ReadOnly)).as_array().unwrap().iter().any(|rule| rule["action"] == "ask"));
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

    /// Drives a real OpenCode 2.x session through every path that differs
    /// from 1.x: the instruction entry, a permission reply, a question form,
    /// an image attachment and native compaction.
    #[test]
    #[ignore = "requires OpenCode 2.x with an authenticated OpenCode Go subscription"]
    fn live_v2_round_trips_instructions_permissions_questions_images_and_compaction() {
        const RED_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAIAAACQkWg2AAAAF0lEQVR4nGP4z8BAEiJN9aiGUQ1DSgMAkPn/Afnh+ngAAAAASUVORK5CYII=";
        let executable = std::env::var("BRIDGE_OPENCODE_LIVE_BINARY")
            .expect("set BRIDGE_OPENCODE_LIVE_BINARY to the OpenCode executable");
        assert_eq!(
            ServerApi::of(&binary_version_at(Path::new(&executable)).unwrap()),
            ServerApi::V2
        );
        let model = std::env::var("BRIDGE_OPENCODE_LIVE_MODEL")
            .unwrap_or_else(|_| "opencode-go/minimax-m3.1-flash-free".into());
        let directory = tempfile::tempdir().unwrap();
        let started = start_with_settings(
            StartRequest {
                session_id: None,
                cwd: directory.path().to_str().unwrap(),
                model: Some(&model),
                effort: None,
                speed: None,
                instructions: Some("End every reply with the word BANANA."),
                write_mode: Some(WriteMode::Isolated),
                read_only_sandbox: None,
                briefing: None,
                interactive: false,
                account: None,
                on_progress: None,
            },
            &OpenCodeSettings {
                executable_path: Some(executable),
                visible_models: Vec::new(),
            },
        )
        .unwrap();
        let mut runtime = started.runtime;
        let mut reader = started.reader;
        let (sender, events) = mpsc::channel::<crate::agent::NormalizedEvent>();
        thread::spawn(move || {
            let mut state = crate::agent::OpenCodeStreamState::default();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or_default() == 0 {
                    break;
                }
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                for event in crate::agent::normalize_opencode_message_with_state(&value, &mut state) {
                    if sender.send(event).is_err() {
                        return;
                    }
                }
            }
        });
        // Until the turn ends: every reply, plus a hook for requests.
        let run_turn = |runtime: &OpenCodeRuntime, on_request: &dyn Fn(&OpenCodeRuntime, &crate::agent::NormalizedEvent)| {
            let mut replies = String::new();
            let mut kinds = Vec::new();
            loop {
                let event = events
                    .recv_timeout(Duration::from_secs(120))
                    .expect("OpenCode stalled mid-turn");
                kinds.push(event.kind.clone());
                match event.kind.as_str() {
                    "message.completed" => replies.push_str(event.text.as_deref().unwrap_or_default()),
                    "permission.requested" | "question.requested" => on_request(runtime, &event),
                    "turn.completed" => {
                        assert_eq!(event.status.as_deref(), Some("completed"), "{kinds:?} {replies}");
                        return (replies, kinds);
                    }
                    _ => {}
                }
            }
        };
        let no_requests = |_: &OpenCodeRuntime, event: &crate::agent::NormalizedEvent| {
            panic!("unexpected request {event:?}")
        };

        runtime
            .send_turn("Run the shell command `echo perm > p.txt`, then reply in one short sentence.")
            .unwrap();
        let (reply, kinds) = run_turn(&runtime, &|runtime, event| {
            assert_eq!(event.kind, "permission.requested");
            runtime.respond(event.data["requestId"].clone(), "accept").unwrap();
        });
        assert!(kinds.iter().any(|kind| kind == "permission.requested"), "{kinds:?}");
        assert!(kinds.iter().any(|kind| kind == "command.completed"), "{kinds:?}");
        assert_eq!(std::fs::read_to_string(directory.path().join("p.txt")).unwrap().trim(), "perm");
        assert!(reply.contains("BANANA"), "instructions did not reach the model: {reply}");

        runtime
            .send_turn("Use your question tool to ask me whether I prefer red or blue, then tell me my answer.")
            .unwrap();
        let (reply, kinds) = run_turn(&runtime, &|runtime, event| {
            assert_eq!(event.kind, "question.requested");
            runtime
                .answer_question(event.data["requestId"].clone(), json!([["Blue"]]))
                .unwrap();
        });
        assert!(kinds.iter().any(|kind| kind == "question.settled"), "{kinds:?}");
        assert!(reply.to_lowercase().contains("blue"), "{reply}");

        runtime
            .send_turn_with_images(
                "What single color fills this image? Answer with the color name.",
                TurnContext::default(),
                &[bridge_protocol::messages::TurnImage {
                    media_type: "image/png".into(),
                    base64_data: RED_PNG.into(),
                }],
            )
            .unwrap();
        let (reply, _) = run_turn(&runtime, &no_requests);
        assert!(reply.to_lowercase().contains("red"), "{reply}");

        assert_eq!(runtime.native_compaction(), crate::adapters::NativeCompaction::WholeConversation);
        runtime.compact_native(None).unwrap();
        let (_, kinds) = run_turn(&runtime, &no_requests);
        assert!(
            kinds.iter().any(|kind| kind == crate::agent::NATIVE_COMPACTION_KIND),
            "{kinds:?}"
        );
        runtime.stop(ShutdownReason::Completed);
    }

    /// Saves and removes a key through 2.x's integration and credential
    /// routes. It writes a real credential, so it runs only when the whole
    /// process points OpenCode at throwaway XDG directories.
    #[test]
    #[ignore = "requires OpenCode 2.x and BRIDGE_OPENCODE_LIVE_ISOLATED=1 with throwaway XDG_DATA_HOME/XDG_CONFIG_HOME"]
    fn live_v2_saves_and_removes_a_provider_key() {
        assert_eq!(std::env::var("BRIDGE_OPENCODE_LIVE_ISOLATED").as_deref(), Ok("1"));
        let settings = OpenCodeSettings {
            executable_path: Some(
                std::env::var("BRIDGE_OPENCODE_LIVE_BINARY")
                    .expect("set BRIDGE_OPENCODE_LIVE_BINARY to the OpenCode executable"),
            ),
            visible_models: Vec::new(),
        };
        let directory = tempfile::tempdir().unwrap();
        let directory = directory.path().to_str().unwrap();
        let provider = |catalog: &OpenCodeCatalog| {
            catalog.providers.iter().find(|provider| provider.id == "deepseek").cloned().unwrap()
        };
        let before = provider(&discover(&settings, directory).unwrap());
        assert!(!before.connected);
        assert!(before.auth_methods.iter().any(|method| method.kind == "api"));
        let saved = provider(&set_provider_api_key(&settings, directory, "deepseek", "sk-bridge-test").unwrap());
        assert!(saved.connected, "a saved key makes the provider available");
        assert_eq!(saved.source.as_deref(), Some("api"));
        assert!(!saved.models.is_empty());
        let removed = provider(&remove_provider_auth(&settings, directory, "deepseek").unwrap());
        assert!(!removed.connected);
    }

    #[test]
    #[ignore = "requires OpenCode 1.18.3+ with an authenticated OpenCode Go subscription"]
    fn live_chat_streams_an_opencode_go_reply() {
        let executable = std::env::var("BRIDGE_OPENCODE_LIVE_BINARY")
            .expect("set BRIDGE_OPENCODE_LIVE_BINARY to the OpenCode executable");
        let directory = tempfile::tempdir().unwrap();
        let mut started = start_with_settings(
            StartRequest {
                session_id: None,
                cwd: directory.path().to_str().unwrap(),
                model: Some("opencode-go/glm-5.2"),
                effort: None,
                speed: None,
                instructions: None,
                write_mode: None,
                read_only_sandbox: None,
                briefing: None,
                interactive: false,
                account: None,
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
    fn ask_mode_asks_before_notes_writes_in_both_api_versions() {
        // OpenCode applies the last matching rule; MCP tools match by their
        // `<server>_<tool>` name.
        fn resolve_v1(rules: &Value, tool: &str) -> String {
            rules.as_array().unwrap().iter().rev()
                .find(|rule| rule["permission"] == "*" || rule["permission"] == tool)
                .map(|rule| rule["action"].as_str().unwrap().to_owned()).unwrap()
        }
        fn resolve_v2(rules: &Value, tool: &str) -> String {
            rules.as_array().unwrap().iter().rev()
                .find(|rule| rule["action"] == "*" || rule["action"] == tool)
                .map(|rule| rule["effect"].as_str().unwrap().to_owned()).unwrap()
        }
        for tool in ["bridge_notes_create", "bridge_notes_edit", "bridge_notes_append", "bridge_notes_set", "bridge_notes_draw"] {
            for (mode, expected) in [(Some(WriteMode::Shared), "ask"), (Some(WriteMode::Isolated), "ask"), (Some(WriteMode::ReadOnly), "deny"), (Some(WriteMode::Full), "allow")] {
                let v1 = permission_rules(mode);
                let v2 = crate::opencode_v2::permission_rules(&v1);
                assert_eq!(resolve_v1(&v1, tool), expected, "{tool} {mode:?} v1");
                assert_eq!(resolve_v2(&v2, tool), expected, "{tool} {mode:?} v2");
            }
        }
        for tool in ["bridge_notes_search", "bridge_notes_read", "bridge_visualize", "bridge_memory_search", "bridge_chats_search", "bridge_chats_read"] {
            assert_eq!(resolve_v1(&permission_rules(Some(WriteMode::Shared)), tool), "allow", "{tool} reads freely");
        }
        for tool in ["bridge_chats_search", "bridge_chats_read"] {
            let v1 = permission_rules(Some(WriteMode::ReadOnly));
            assert_eq!(resolve_v1(&v1, tool), "deny", "a read-only worker never reads other chats: {tool}");
            assert_eq!(resolve_v2(&crate::opencode_v2::permission_rules(&v1), tool), "deny", "{tool} v2");
        }
        for (mode, expected) in [(Some(WriteMode::Shared), "allow"), (Some(WriteMode::Isolated), "allow"), (Some(WriteMode::ReadOnly), "deny"), (Some(WriteMode::Full), "allow")] {
            let v1 = permission_rules(mode);
            let v2 = crate::opencode_v2::permission_rules(&v1);
            for tool in ["bridge_memory_propose", "bridge_outgoing_draft", "bridge_settings_read", "bridge_settings_change"] {
                assert_eq!(resolve_v1(&v1, tool), expected, "{tool} {mode:?} v1");
                assert_eq!(resolve_v2(&v2, tool), expected, "{tool} {mode:?} v2");
            }
        }
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
    fn a_catalog_with_a_stored_credential_proves_sign_in_without_auth_json() {
        let mut catalog = catalog_fixture();
        for provider in &mut catalog.providers {
            provider.source = Some("env".into());
        }
        let env_only = catalog.clone();
        catalog.providers[0].connected = true;
        catalog.providers[0].source = Some("api".into());
        assert_eq!(auth_state_with_catalog(Some(&catalog)), AuthState::SignedIn);
        assert_eq!(
            auth_state_with_catalog(Some(&env_only)),
            auth_state(),
            "an environment key is not a stored credential; fall back to the file probe"
        );
    }

    #[test]
    fn auth_probe_reports_unknown_when_data_dir_is_missing() {
        assert_eq!(auth_state_from_data_dir(None), AuthState::Unknown);
    }

    fn bridge_launch() -> crate::mcp_apps::ServerLaunch {
        crate::mcp_apps::ServerLaunch {
            command: std::path::PathBuf::from("/Applications/Bridge.app/Contents/MacOS/bridged"),
            args: vec![crate::mcp_apps::HELPER_FLAG.to_owned()],
            env: Vec::new(),
        }
    }

    fn opencode_tools(servers_off: &[&str]) -> ToolLaunch {
        ToolLaunch::from_row(&crate::tool_selection::ToolSelectionRow {
            revision: 1,
            harness: "opencode".into(),
            account: None,
            servers_off: servers_off.iter().map(|name| (*name).to_owned()).collect(),
            codex_apps_off: false,
            claude_skills: None,
            codex_skills_off: Vec::new(),
        })
    }

    /// The `OPENCODE_CONFIG_CONTENT` that the real command sets. `None` means
    /// the command sets nothing, so the server inherits the value as is.
    fn config_env(visualize: Option<&crate::mcp_apps::ServerLaunch>, existing: Option<&str>, tools: Option<&ToolLaunch>) -> Option<String> {
        let mut command = Command::new("/opt/fake/bin/opencode");
        apply_chat_config(&mut command, visualize, existing, tools);
        let set: Vec<Option<String>> = command
            .get_envs()
            .filter(|(key, _)| *key == "OPENCODE_CONFIG_CONTENT")
            .map(|(_, value)| value.map(|value| value.to_string_lossy().into_owned()))
            .collect();
        assert!(set.len() <= 1 && set.iter().all(Option::is_some), "{set:?}");
        set.into_iter().flatten().next()
    }

    fn parsed(content: Option<String>) -> Value {
        serde_json::from_str(&content.expect("the launch sets the config content")).unwrap()
    }

    #[test]
    fn a_chat_without_a_selection_keeps_the_inherited_config_content() {
        let inherited = r#"{"theme":"system","mcp":{"github":{"type":"local","command":["gh-mcp"]}}}"#;
        let today = crate::mcp_apps::merge_opencode_config(Some(inherited), &bridge_launch());
        // No server to attach: the command sets nothing, and the server inherits the value byte for byte.
        assert_eq!(config_env(None, Some(inherited), None), None);
        assert_eq!(config_env(None, None, None), None);
        assert_eq!(config_env(Some(&bridge_launch()), Some(inherited), None), Some(today.clone()));
        // A choice that leaves nothing out, and a choice for another harness, change nothing.
        let nothing_off = opencode_tools(&[]);
        let codex = ToolLaunch { harness: "codex".into(), ..opencode_tools(&["github"]) };
        for tools in [&nothing_off, &codex] {
            assert_eq!(config_env(None, Some(inherited), Some(tools)), None);
            assert_eq!(config_env(None, None, Some(tools)), None);
            assert_eq!(config_env(Some(&bridge_launch()), Some(inherited), Some(tools)), Some(today.clone()));
        }
    }

    #[test]
    fn the_selection_sets_only_enabled_false_leaves_and_keeps_the_rest() {
        let inherited = json!({
            "theme": "system",
            "model": "anthropic/claude",
            "mcp": {
                "github": {"type": "local", "command": ["gh-mcp"], "environment": {"A": "1"}},
                "sentry": {"type": "remote", "url": "https://sentry.example", "enabled": true},
                "linear": {"type": "remote", "url": "https://linear.example"}
            },
            "permission": {"edit": "ask"}
        })
        .to_string();
        let tools = opencode_tools(&["sentry", "github", "figma"]);
        for visualize in [None, Some(bridge_launch())] {
            let mut expected = match &visualize {
                Some(launch) => parsed(Some(crate::mcp_apps::merge_opencode_config(Some(&inherited), launch))),
                None => parsed(Some(inherited.clone())),
            };
            expected["mcp"]["github"]["enabled"] = json!(false);
            expected["mcp"]["sentry"]["enabled"] = json!(false);
            expected["mcp"]["figma"] = json!({"enabled": false});
            let content = config_env(visualize.as_ref(), Some(&inherited), Some(&tools));
            assert_eq!(parsed(content), expected);
        }
        // With no inherited value and no server to attach, the content holds the leaves only.
        let alone = parsed(config_env(None, None, Some(&tools)));
        assert_eq!(alone, json!({"mcp": {"figma": {"enabled": false}, "github": {"enabled": false}, "sentry": {"enabled": false}}}));
    }

    #[test]
    fn an_inherited_config_that_does_not_parse_leaves_the_selection_unapplied() {
        use crate::tool_selection::{merge_opencode, opencode_not_applied_reason, OpenCodeMerge, OPENCODE_CONFIG_REASON};
        let tools = opencode_tools(&["github"]);
        for bad in ["{bad", "", "[]", "\"text\"", "null", r#"{"mcp": []}"#, r#"{"mcp": {"github": true}}"#] {
            assert_eq!(merge_opencode(Some(bad), &tools.servers_off), OpenCodeMerge::NotApplied, "{bad:?}");
            // The command sets nothing, so the inherited value stays as is. It never becomes `{}`.
            assert_eq!(config_env(None, Some(bad), Some(&tools)), None, "{bad:?}");
            // With a server to attach, the launch keeps the value of today.
            let today = crate::mcp_apps::merge_opencode_config(Some(bad), &bridge_launch());
            let content = config_env(Some(&bridge_launch()), Some(bad), Some(&tools));
            assert_eq!(content.as_deref(), Some(today.as_str()), "{bad:?}");
            assert!(!today.contains("github") || bad.contains("github"), "{bad:?}");
            // The picker can show why.
            assert_eq!(opencode_not_applied_reason(Some(bad), &tools.servers_off), Some(OPENCODE_CONFIG_REASON), "{bad:?}");
        }
        assert_eq!(opencode_not_applied_reason(None, &tools.servers_off), None);
        assert_eq!(opencode_not_applied_reason(Some(r#"{"theme":"system"}"#), &tools.servers_off), None);
    }

    #[test]
    fn the_bridge_entry_and_each_permission_rule_stay() {
        let inherited = json!({
            "permission": {"edit": "ask", "bash": {"*": "ask", "git status": "allow"}, "github_*": "deny", "skill": {"deploy": "deny"}},
            "mcp": {"github": {"type": "local", "command": ["gh-mcp"]}}
        })
        .to_string();
        // A launch that skipped the checks names the Bridge server too.
        let forged = ToolLaunch { servers_off: vec!["bridge".into(), "Bridge".into(), "github".into()], ..opencode_tools(&[]) };
        let baseline = parsed(config_env(Some(&bridge_launch()), Some(&inherited), None));
        let content = parsed(config_env(Some(&bridge_launch()), Some(&inherited), Some(&forged)));
        assert_eq!(content["mcp"]["bridge"], baseline["mcp"]["bridge"]);
        assert_eq!(content["mcp"]["bridge"]["enabled"], true);
        assert!(content["mcp"].get("Bridge").is_none());
        assert_eq!(content["permission"], baseline["permission"]);
        assert_eq!(content["permission"], parsed(Some(inherited.clone()))["permission"]);
        assert_eq!(content["mcp"]["github"], json!({"type": "local", "command": ["gh-mcp"], "enabled": false}));
        // Without a server to attach, the permission object stays too, and no Bridge entry appears.
        let alone = parsed(config_env(None, Some(&inherited), Some(&forged)));
        assert_eq!(alone["permission"], baseline["permission"]);
        assert!(alone["mcp"].get("bridge").is_none() && alone["mcp"].get("Bridge").is_none());
        assert_eq!(alone["permission"], parsed(Some(inherited))["permission"]);
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

    #[test]
    fn opencode_chat_sessions_get_mcp_bridge_in_config_content() {
        let launch = crate::mcp_apps::ServerLaunch {
            command: std::path::PathBuf::from("/Applications/Bridge.app/Contents/MacOS/bridged"),
            args: vec![crate::mcp_apps::HELPER_FLAG.to_owned()],
            env: Vec::new(),
        };
        let content: serde_json::Value = serde_json::from_str(
            &chat_config_content(Some(&launch), Some(r#"{"theme":"system"}"#), None).unwrap(),
        )
        .unwrap();
        assert_eq!(content["theme"], "system");
        assert_eq!(content["mcp"]["bridge"]["type"], "local");
        assert_eq!(content["mcp"]["bridge"]["command"][1], crate::mcp_apps::HELPER_FLAG);
    }

    #[test]
    fn opencode_workers_do_not() {
        assert_eq!(chat_config_content(None, Some(r#"{"theme":"system"}"#), None), None);
    }

}

#[cfg(test)]
mod read_only_sandbox_tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn credential_store(path: &Path, rows: &[(&str, &str, Value)]) {
        let db = rusqlite::Connection::open(path).unwrap();
        db.execute_batch(
            "CREATE TABLE credential (
                id text PRIMARY KEY, integration_id text, label text NOT NULL,
                value text NOT NULL, connector_id text, method_id text,
                active integer, time_created integer NOT NULL, time_updated integer NOT NULL
            );",
        )
        .unwrap();
        for (id, integration, value) in rows {
            db.execute(
                "INSERT INTO credential (id, integration_id, label, value, active, time_created, time_updated)
                 VALUES (?1, ?2, ?2, ?3, 1, 1, 2)",
                rusqlite::params![id, integration, value.to_string()],
            )
            .unwrap();
        }
    }

    fn stored(path: &Path) -> Vec<(String, Value)> {
        let db = rusqlite::Connection::open(path).unwrap();
        let mut statement = db.prepare("SELECT integration_id, value FROM credential ORDER BY id").unwrap();
        statement
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .unwrap()
            .map(|row| {
                let (integration, value) = row.unwrap();
                (integration, serde_json::from_str(&value).unwrap())
            })
            .collect()
    }

    fn oauth(expires: i64) -> Value {
        json!({"type":"oauth","access":"access-token","refresh":"refresh-token","expires":expires,"metadata":{"server":"https://opencode.ai"}})
    }

    #[test]
    fn sandbox_environment_points_every_writable_home_into_the_output_dir() {
        let output = Path::new("/tmp/bridge-read-only-workers/abc/output");
        let homes = SandboxHomes::under(output);
        let environment = homes.environment();
        let names = environment.iter().map(|(name, _)| *name).collect::<Vec<_>>();
        assert_eq!(
            names,
            ["XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME", "TMPDIR", "BRIDGE_WORKER_OUTPUT_DIR"]
        );
        for (name, value) in &environment {
            assert!(value.starts_with(output), "{name} escapes the output dir: {}", value.display());
        }
        // The config home is only read: custom providers and MCP servers live there.
        assert!(!names.contains(&"XDG_CONFIG_HOME"));
        assert!(!names.contains(&"HOME"));
        assert_eq!(homes.opencode_data(), output.join(".opencode/data/opencode"));

        let dir = tempfile::tempdir().unwrap();
        let homes = SandboxHomes::under(dir.path());
        homes.create().unwrap();
        for (_, value) in homes.environment() {
            assert!(value.is_dir(), "{} was not created", value.display());
        }
        assert!(homes.opencode_data().is_dir());
    }

    #[test]
    fn seeding_copies_keys_and_strips_oauth_refresh_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let host = dir.path().join("host.db");
        let sandbox = dir.path().join("sandbox.db");
        credential_store(
            &host,
            &[
                ("a", "openai", json!({"type":"key","key":"sk-not-a-real-key"})),
                ("b", "opencode", oauth(NOW + 60_000)),
            ],
        );
        credential_store(&sandbox, &[]);
        assert_eq!(seed_stored_credentials(&host, &sandbox, NOW).unwrap(), 2);
        let copied = stored(&sandbox);
        assert_eq!(copied[0], ("openai".into(), json!({"type":"key","key":"sk-not-a-real-key"})));
        assert_eq!(copied[1].0, "opencode");
        assert_eq!(copied[1].1["access"], "access-token");
        assert_eq!(copied[1].1["refresh"], "", "a sandboxed worker must never hold the refresh token");
        assert_eq!(copied[1].1["metadata"]["server"], "https://opencode.ai");
        // Seeding twice (a relaunch) replaces rather than duplicates.
        assert_eq!(seed_stored_credentials(&host, &sandbox, NOW).unwrap(), 2);
        assert_eq!(stored(&sandbox).len(), 2);
    }

    #[test]
    fn seeding_skips_expired_oauth_and_never_writes_the_host_store() {
        let dir = tempfile::tempdir().unwrap();
        let host = dir.path().join("host.db");
        let sandbox = dir.path().join("sandbox.db");
        credential_store(
            &host,
            &[
                ("a", "opencode", oauth(NOW - 1)),
                ("b", "opencode-go", json!({"type":"key","key":"go-not-a-real-key"})),
            ],
        );
        let before = std::fs::read(&host).unwrap();
        credential_store(&sandbox, &[]);
        assert_eq!(seed_stored_credentials(&host, &sandbox, NOW).unwrap(), 1);
        assert_eq!(stored(&sandbox).iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["opencode-go"]);
        assert_eq!(std::fs::read(&host).unwrap(), before, "the user's store was modified");
        assert_eq!(stored(&host)[0].1["refresh"], "refresh-token");
    }

    #[test]
    fn seeding_refuses_a_sandbox_database_without_a_credential_table() {
        let dir = tempfile::tempdir().unwrap();
        let host = dir.path().join("host.db");
        credential_store(&host, &[("a", "openai", json!({"type":"key","key":"k"}))]);
        let sandbox = dir.path().join("sandbox.db");
        rusqlite::Connection::open(&sandbox).unwrap();
        let error = seed_stored_credentials(&host, &sandbox, NOW).unwrap_err().to_string();
        assert!(error.contains("without a credential store"), "{error}");
    }

    #[test]
    fn legacy_auth_json_is_sanitized_into_the_sandbox() {
        let host = tempfile::tempdir().unwrap();
        let sandbox = tempfile::tempdir().unwrap();
        std::fs::write(
            host.path().join("auth.json"),
            json!({
                "openai": {"type":"api","key":"sk-not-a-real-key"},
                "anthropic": oauth(NOW + 60_000),
                "github-copilot": oauth(NOW - 1),
            })
            .to_string(),
        )
        .unwrap();
        assert_eq!(seed_legacy_auth(host.path(), sandbox.path(), NOW).unwrap(), 2);
        let written = sandbox.path().join("auth.json");
        let copied: Value = serde_json::from_str(&std::fs::read_to_string(&written).unwrap()).unwrap();
        assert_eq!(copied["openai"]["key"], "sk-not-a-real-key");
        assert_eq!(copied["anthropic"]["access"], "access-token");
        assert_eq!(copied["anthropic"]["refresh"], "");
        assert!(copied.get("github-copilot").is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&written).unwrap().permissions().mode() & 0o777, 0o600);
        }

        std::fs::write(host.path().join("auth.json"), "not json").unwrap();
        assert!(seed_legacy_auth(host.path(), sandbox.path(), NOW)
            .unwrap_err()
            .to_string()
            .contains("not valid JSON"));
    }

    #[test]
    fn missing_host_store_seeds_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let sandbox = dir.path().join("sandbox.db");
        credential_store(&sandbox, &[]);
        assert_eq!(seed_stored_credentials(&dir.path().join("absent.db"), &sandbox, NOW).unwrap(), 0);
        assert_eq!(seed_legacy_auth(&dir.path().join("absent"), dir.path(), NOW).unwrap(), 0);
        assert!(!dir.path().join("auth.json").exists());
        // A 1.x database (no credential table) is not an error either.
        let old = dir.path().join("old.db");
        rusqlite::Connection::open(&old).unwrap().execute_batch("CREATE TABLE session (id text);").unwrap();
        assert_eq!(seed_stored_credentials(&old, &sandbox, NOW).unwrap(), 0);
    }

    #[test]
    fn read_only_refuses_only_when_runtime_network_is_denied() {
        assert_eq!(read_only_refusal(false), None);
        let reason = read_only_refusal(true).unwrap();
        assert!(reason.contains("BRIDGE_READ_ONLY_NETWORK=deny"), "{reason}");
        assert!(reason.contains("Claude or Codex"), "{reason}");
        assert!(ensure_read_only_transport_supported(None).is_ok());
    }

    /// A real read-only OpenCode worker: its server runs under the platform
    /// sandbox with its homes in the output folder and the user's logins
    /// seeded, answers from a workspace file, and cannot write the workspace.
    #[test]
    #[ignore = "requires a signed-in OpenCode on PATH (or BRIDGE_OPENCODE_LIVE_BINARY); set BRIDGE_OPENCODE_LIVE_MODEL to pick the model"]
    fn live_read_only_worker_answers_inside_the_sandbox() {
        let executable = std::env::var("BRIDGE_OPENCODE_LIVE_BINARY")
            .ok()
            .or_else(|| which::which("opencode").ok().map(|path| path.to_string_lossy().into_owned()))
            .expect("OpenCode is not installed");
        let model = std::env::var("BRIDGE_OPENCODE_LIVE_MODEL")
            .unwrap_or_else(|_| "opencode-go/minimax-m3.1-flash-free".into());
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("fact.txt"), "The secret word is MARMALADE.\n").unwrap();
        let delegation = crate::delegation::DelegationRequest {
            schema_version: 1,
            role: crate::delegation::WorkerRole::Research,
            objective: "read".into(),
            acceptance_criteria: vec!["report".into()],
            known_facts: vec![],
            decisions: vec![],
            evidence_ids: vec![],
            relevant_files: vec![],
            owned_paths: vec![],
            write_mode: WriteMode::ReadOnly,
            capability_tier: CapabilityTier::Fast,
            effort: crate::delegation::Effort::Low,
            network_access: false,
            writable_output_paths: vec![],
            verification: vec![],
            output_contract: crate::delegation::OutputContract::ResearchResult,
            harness: Some("opencode".into()),
            model: Some(model.clone()),
        };
        let sandbox = crate::worker_sandbox::ReadOnlySandbox::create("live", workspace.path(), &delegation).unwrap();
        let started = start_with_settings(
            StartRequest {
                session_id: None,
                cwd: workspace.path().to_str().unwrap(),
                model: Some(&model),
                effort: None,
                speed: None,
                instructions: Some("Be brief. When asked to run a shell command, run it with your shell tool and paste its exact output."),
                write_mode: Some(WriteMode::ReadOnly),
                read_only_sandbox: Some(&sandbox),
                briefing: None,
                interactive: false,
                account: None,
                on_progress: None,
            },
            &OpenCodeSettings { executable_path: Some(executable), visible_models: Vec::new() },
        )
        .expect("a read-only OpenCode worker starts");
        assert!(
            sandbox.output_dir().join(".opencode/data/opencode").read_dir().unwrap().next().is_some(),
            "OpenCode did not write its state into the sandbox"
        );
        let mut runtime = started.runtime;
        let mut reader = started.reader;
        let (sender, events) = mpsc::channel::<crate::agent::NormalizedEvent>();
        thread::spawn(move || {
            let mut state = crate::agent::OpenCodeStreamState::default();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or_default() == 0 {
                    break;
                }
                let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
                for event in crate::agent::normalize_opencode_message_with_state(&value, &mut state) {
                    if sender.send(event).is_err() {
                        return;
                    }
                }
            }
        });
        let run_turn = |runtime: &OpenCodeRuntime| {
            let mut replies = String::new();
            let mut kinds = Vec::new();
            loop {
                let event = events.recv_timeout(Duration::from_secs(180)).expect("OpenCode stalled mid-turn");
                kinds.push(event.kind.clone());
                match event.kind.as_str() {
                    "message.completed" => replies.push_str(event.text.as_deref().unwrap_or_default()),
                    "error" => replies.push_str(&format!("[error: {} {}]", event.text.as_deref().unwrap_or_default(), event.data)),
                    // Approve everything: the OS sandbox, not the prompt, must stop the write.
                    "permission.requested" => runtime.respond(event.data["requestId"].clone(), "accept").unwrap(),
                    "turn.completed" => return (replies, kinds),
                    _ => {}
                }
            }
        };
        runtime
            .send_turn("Read the file fact.txt in the current directory and reply with only the secret word.")
            .unwrap();
        let (reply, kinds) = run_turn(&runtime);
        assert!(reply.to_uppercase().contains("MARMALADE"), "{reply} {kinds:?}");

        runtime
            .send_turn("Use your shell tool to run exactly: `echo escaped > written.txt; echo exit=$?`. Paste the output verbatim.")
            .unwrap();
        let (reply, kinds) = run_turn(&runtime);
        eprintln!("write attempt: {kinds:?}\n{reply}");
        assert!(
            kinds.iter().any(|kind| kind == "command.completed"),
            "the write was never attempted, so this run proves nothing: {reply} {kinds:?}"
        );
        assert!(
            !workspace.path().join("written.txt").exists(),
            "the sandbox let a read-only worker write the workspace: {reply} {kinds:?}"
        );
        runtime.stop(crate::adapters::ShutdownReason::Completed);
        sandbox.cleanup();
    }
}
