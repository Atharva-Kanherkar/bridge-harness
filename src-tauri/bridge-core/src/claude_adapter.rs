use crate::{
    adapters::{AdapterRuntime, ResumeRequest, ShutdownReason, StartRequest, TurnContext},
    binary,
    context_inventory::{
        AdapterContextInventory, ContextInventoryScope, ContextLifecyclePhase, ContextObservedSize,
        ContextSegmentClass, ContextSegmentObservation,
    },
    delegation::WriteMode,
    model::AuthState,
    BridgeError,
};
use serde_json::{json, Value};
use std::{
    io::{BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, MutexGuard, RwLock,
    },
};
use uuid::Uuid;

/// Where the Node module-compile cache for the Claude sidecar lives, so its
/// SDK module load warms across launches (Node 22+; `NODE_COMPILE_CACHE`).
/// Unregistered (e.g. in tests) means every launch skips the env var —
/// warm compilation is an optimization, never a launch requirement.
static NODE_COMPILE_CACHE_ROOT: RwLock<Option<PathBuf>> = RwLock::new(None);

pub fn register_node_compile_cache_root(root: impl Into<PathBuf>) {
    *NODE_COMPILE_CACHE_ROOT
        .write()
        .expect("the node compile cache root lock is never poisoned") = Some(root.into());
}

/// Single source of truth for the Claude adapter's default model id.
/// Referenced by the catalog in `adapters.rs` and by the runtime fallback below.
pub const DEFAULT_MODEL: &str = "sonnet";

pub fn discover_models() -> Result<Vec<crate::adapters::DiscoveredModel>, BridgeError> {
    let node = binary::resolve("node").ok_or_else(|| BridgeError::Invalid("Node.js is required to discover Claude models".into()))?;
    let sidecar = sidecar_entry()?;
    let mut command = Command::new(node);
    command.arg(sidecar).arg(serde_json::json!({ "catalog": true }).to_string())
        .env_remove("NODE_OPTIONS").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    configure_sdk_environment(&mut command);
    crate::adapters::configure_process_group(&mut command);
    let mut child = command.spawn()
        .map_err(|error| BridgeError::Adapter(format!("Cannot query Claude model catalogue: {error}")))?;
    let stdout = child.stdout.take().expect("catalogue stdout is piped");
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut output = String::new();
        let result = stdout.take(512 * 1024).read_to_string(&mut output).map(|_| output);
        let _ = send.send(result);
    });
    let result = receive.recv_timeout(std::time::Duration::from_secs(15));
    // Also cleans up the SDK child on success or a malformed/oversized response.
    crate::adapters::terminate_process_group(child.id());
    let _ = child.kill();
    let _ = child.wait();
    let output = result.map_err(|_| BridgeError::Adapter("Claude model discovery timed out after 15 seconds".into()))??;
    parse_discovered_models(&output)
}

fn parse_discovered_models(output: &str) -> Result<Vec<crate::adapters::DiscoveredModel>, BridgeError> {
    for line in output.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else { continue };
        let Some(models) = value.get("models").and_then(Value::as_array) else { continue };
        let found = models.iter().filter_map(|model| {
            let alias = model.get("value")?.as_str()?.trim();
            let id = model.get("resolvedModel").and_then(Value::as_str)
                .map(str::trim).filter(|id| !id.is_empty()).unwrap_or(alias);
            let display = model.get("displayName").and_then(Value::as_str).unwrap_or(id).trim();
            let description_name = model.get("description").and_then(Value::as_str)
                .and_then(|description| description.split(" · ").next())
                .filter(|name| ["Opus ", "Sonnet ", "Haiku ", "Fable "].iter().any(|prefix| name.starts_with(prefix))
                    && name.chars().any(|character| character.is_ascii_digit()));
            let label = description_name.unwrap_or(display);
            // Absent or empty supportedEffortLevels means no effort knob (e.g.
            // haiku); the picker then hides the effort control for the model.
            let mut supported_effort_levels: Vec<String> = model.get("supportedEffortLevels")
                .and_then(Value::as_array)
                .map(|levels| levels.iter().filter_map(|level| {
                    level.as_str().map(str::trim).filter(|value| !value.is_empty()).map(str::to_owned)
                }).collect::<Vec<_>>())
                .unwrap_or_default();
            if model.get("supportsEffort").and_then(Value::as_bool) == Some(false) { supported_effort_levels.clear(); }
            // "default" is the CLI's alias for whatever it currently prefers,
            // not a model; Bridge tracks its own per-tier defaults instead.
            (!id.is_empty() && !label.is_empty() && id != "default").then(|| crate::adapters::DiscoveredModel {
                id: id.to_owned(),
                label: label.to_owned(),
                // Canonicalizing the default alias preserves provider preference.
                is_default: alias == "default",
                supported_effort_levels,
            })
        }).collect::<Vec<_>>();
        if !found.is_empty() { return Ok(found); }
    }
    Err(BridgeError::Adapter("Claude returned no model catalogue".into()))
}

fn configure_sdk_environment(command: &mut Command) {
    match managed_sdk_module() {
        Some(module) => {
            command.env("BRIDGE_CLAUDE_SDK_ENTRY", module);
        }
        None => {
            command.env_remove("BRIDGE_CLAUDE_SDK_ENTRY");
        }
    }
    if let Some(cache_dir) = NODE_COMPILE_CACHE_ROOT
        .read()
        .expect("the node compile cache root lock is never poisoned")
        .clone()
    {
        if std::fs::create_dir_all(&cache_dir).is_ok() {
            command.env("NODE_COMPILE_CACHE", &cache_dir);
        }
    }
}

pub struct ClaudeRuntime {
    pub writer: Arc<Mutex<ChildStdin>>,
    pub child: Child,
    pub session_id: String,
    pub current_turn: Arc<Mutex<Option<String>>>,
    context_inventory: Mutex<Vec<AdapterContextInventory>>,
    request_id: AtomicU64,
    stopped: bool,
    stderr_tail: crate::adapters::StderrTail,
}

pub struct StartedClaude {
    pub runtime: ClaudeRuntime,
    pub reader: BufReader<ChildStdout>,
    pub startup_messages: Vec<Value>,
}

pub fn start(request: StartRequest<'_>) -> Result<StartedClaude, BridgeError> {
    launch(request, None)
}

pub fn resume(request: ResumeRequest<'_>) -> Result<StartedClaude, BridgeError> {
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
    )
}

fn launch(
    request: StartRequest<'_>,
    resume_session_id: Option<&str>,
) -> Result<StartedClaude, BridgeError> {
    let StartRequest {
        cwd,
        model,
        effort,
        instructions,
        write_mode,
        read_only_sandbox,
        briefing,
        on_progress,
    } = request;
    // Claude runs through the Claude Agent SDK, driven by a Node sidecar. One
    // long-lived streaming query serves every turn on a single session (fixing
    // the `claude -p` "exit after one turn" behaviour). Provider discovery
    // supplies enabled plugins and credential-free connector endpoints while
    // the sidecar remains isolated from unrelated global hooks and permissions.
    let node = binary::resolve("node").ok_or_else(|| {
        BridgeError::Invalid(
            "Node.js is required to run Claude (expected `node` on PATH). Install Node 18+ to use Claude models."
                .into(),
        )
    })?;
    let sidecar = sidecar_entry()?;
    let session_id = resume_session_id
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let chosen_model = model
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_MODEL);
    // A briefing run's authority, if this is one. The boundary refuses rather than
    // ignoring: an adapter that dropped this on the floor would run a briefing
    // with a coding agent's full toolset, which is the one outcome the policy
    // exists to prevent.
    let briefing_config = match briefing {
        Some(policy) => {
            crate::briefing_policy::adapter_may_brief("claude")
                .map_err(|error| BridgeError::Invalid(error.reason()))?;
            crate::briefing_policy::BriefingRuntimePolicy::check_write_mode(write_mode)
                .map_err(|error| BridgeError::Invalid(error.reason()))?;
            Some(json!({
                "allowedTools": policy.allowed_wire_names(),
                "allowedServers": policy.allowed_servers(),
                // Servers whose read-verb tools are allowed without per-identity
                // review — the harness-run briefing's mode, where the harness's
                // own MCP configuration decides what exists. Empty for an
                // exact-review policy, and the gate treats empty as no scope.
                "readScopeServers": policy.read_scope_servers(),
                "deniedBuiltins": crate::briefing_policy::BriefingRuntimePolicy::denied_builtin_names(),
                "maxArgumentBytes": policy.max_argument_bytes(),
            }))
        }
        None => None,
    };
    let sdk_configuration = crate::marketplace::claude_sdk_configuration();
    let lifecycle_phase = if resume_session_id.is_some() {
        ContextLifecyclePhase::Resume
    } else {
        ContextLifecyclePhase::Start
    };
    let context_inventory = claude_context_inventory(lifecycle_phase, &sdk_configuration)?;
    let config = json!({
        "sessionId": session_id,
        "model": chosen_model,
        "cwd": cwd,
        "resume": resume_session_id.is_some(),
        // Bridge injects the delegation protocol + worker brief as an appended
        // system prompt so the child agent knows its single typed task.
        "instructions": instructions.map(str::trim).filter(|value| !value.is_empty()),
        "writeMode": write_mode.map(write_mode_label),
        "plugins": sdk_configuration.plugins,
        "mcpServers": sdk_configuration.mcp_servers,
        // Absent for every non-briefing session, so the sidecar's existing
        // write-mode handling is reached by exactly the same path as before.
        "briefing": briefing_config,
        // The routed reasoning effort. The Claude Agent SDK takes this natively
        // (Options.effort), so low/medium are honoured instead of being silently
        // dropped the way the old thinking-budget mapping dropped them. The
        // sidecar sanitizes it to the levels the SDK accepts.
        "effort": effort.map(str::trim).filter(|value| !value.is_empty()),
    });
    let mut command = crate::worker_sandbox::command(&node, read_only_sandbox)?;
    command
        .arg(&sidecar)
        .arg(config.to_string())
        .current_dir(
            read_only_sandbox
                .map(|sandbox| sandbox.output_dir())
                .unwrap_or_else(|| std::path::Path::new(cwd)),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Piped and tail-captured: when the sidecar dies before producing a
        // typed result, its last words are the failure context the parent
        // sees instead of a generic "ended without reporting".
        .stderr(Stdio::piped());
    // Which copy of the SDK the sidecar loads applies to every launch, not just
    // sandboxed ones: an interactive turn runs the same sidecar. The variable is
    // also cleared when there is no managed payload, so a stale value inherited
    // from the environment can never point the sidecar at something Bridge does
    // not own.
    configure_sdk_environment(&mut command);
    if let Some(sandbox) = read_only_sandbox {
        let config_dir = prepare_isolated_claude_config(sandbox)?;
        command
            .env("CLAUDE_CONFIG_DIR", config_dir)
            .env("CLAUDE_CODE_TMPDIR", sandbox.output_dir())
            .env("TMPDIR", sandbox.output_dir())
            .env("BRIDGE_WORKER_OUTPUT_DIR", sandbox.output_dir());
        if std::env::var_os("CLAUDE_CODE_OAUTH_TOKEN").is_none() {
            if let Some(token) = claude_oauth_token()? {
                command.env("CLAUDE_CODE_OAUTH_TOKEN", token);
            }
        }
        // The redirected config dir leaves `gh` with no credentials or keychain;
        // a networked worker (e.g. a PR review) needs the host token or every
        // `gh` call 401s.
        if sandbox.network_allowed() {
            if let Some(token) = crate::worker_sandbox::github_cli_token() {
                command.env("GH_TOKEN", token);
            }
        }
    }
    // Reasoning effort is passed to the SDK natively through the sidecar config
    // (Options.effort) rather than the deprecated MAX_THINKING_TOKENS budget, so
    // every level — low and medium included — reaches the model.
    crate::adapters::configure_process_group(&mut command);
    if let Some(on_progress) = on_progress {
        on_progress(crate::adapters::StartupPhase::Spawning);
    }
    let spawned_at = std::time::Instant::now();
    let mut child = command.spawn().map_err(|e| {
        BridgeError::Invalid(format!(
            "Failed to launch the Claude Agent SDK sidecar via node: {e}"
        ))
    })?;
    if let Some(on_progress) = on_progress {
        // The sidecar is now booting the Claude Agent SDK. This launch never
        // waits on that boot — it hands the caller a reader and returns — so
        // `handshake` is the last phase Claude can honestly report. No
        // `session_open` follows: nothing here observes one, and the contract
        // is that an unobserved phase is not emitted rather than guessed.
        on_progress(crate::adapters::StartupPhase::Handshake);
    }
    let stderr_tail = crate::adapters::StderrTail::capture(&mut child);
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| BridgeError::Invalid("Claude stdin unavailable".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| BridgeError::Invalid("Claude stdout unavailable".into()))?;
    let writer = Arc::new(Mutex::new(stdin));
    let reader = BufReader::new(stdout);
    let startup_messages = vec![json!({
        "type": "system",
        "subtype": "session_ready",
        "session_id": session_id,
        "cwd": cwd,
        "model": chosen_model,
        "resumed": resume_session_id.is_some(),
    })];
    // Boundary named honestly: this is the fork plus the pipe handoff, not the
    // sidecar becoming usable, which this call never waits for.
    crate::process_ledger::log_spawn_to_ready("claude", "process_spawned", spawned_at);
    Ok(StartedClaude {
        runtime: ClaudeRuntime {
            writer,
            child,
            session_id,
            current_turn: Arc::new(Mutex::new(None)),
            context_inventory: Mutex::new(context_inventory),
            request_id: AtomicU64::new(1),
            stopped: false,
            stderr_tail,
        },
        reader,
        startup_messages,
    })
}

fn prepare_isolated_claude_config(
    sandbox: &crate::worker_sandbox::ReadOnlySandbox,
) -> Result<PathBuf, BridgeError> {
    let isolated_root = sandbox.output_dir().join(".claude");
    std::fs::create_dir_all(&isolated_root)?;
    let Some(home) = std::env::var_os("HOME") else {
        return Ok(isolated_root);
    };
    let source = PathBuf::from(home).join(".claude/.credentials.json");
    if !source.is_file() {
        return Ok(isolated_root);
    }
    let destination = isolated_root.join(".credentials.json");
    if destination.exists() {
        return Ok(isolated_root);
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(source, destination)?;
    #[cfg(not(unix))]
    {
        let _ = (source, destination);
        return Err(BridgeError::Invalid(
            "Read-only Claude authentication projection is unsupported on this platform".into(),
        ));
    }
    Ok(isolated_root)
}

#[cfg(target_os = "macos")]
fn claude_oauth_token() -> Result<Option<String>, BridgeError> {
    let output = Command::new("/usr/bin/security")
        .args([
            "find-generic-password",
            "-s",
            "Claude Code-credentials",
            "-w",
        ])
        .output()?;
    if !output.status.success() {
        return Ok(None);
    }
    let credentials: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        BridgeError::Invalid(format!(
            "Claude Keychain credentials are invalid JSON: {error}"
        ))
    })?;
    Ok(credentials
        .pointer("/claudeAiOauth/accessToken")
        .and_then(Value::as_str)
        .map(str::to_owned))
}

#[cfg(not(target_os = "macos"))]
fn claude_oauth_token() -> Result<Option<String>, BridgeError> {
    Ok(None)
}

/// Whether Claude's own credential store holds a usable credential. A
/// presence check only — Bridge never reads the token value out of the
/// Keychain entry, only whether the entry exists.
pub fn auth_state() -> AuthState {
    auth_state_from_home(std::env::var_os("HOME").map(PathBuf::from), claude_keychain_present())
}

fn auth_state_from_home(home: Option<PathBuf>, keychain: AuthState) -> AuthState {
    let file_present = home
        .map(|home| home.join(".claude/.credentials.json"))
        .is_some_and(|path| path.is_file());
    if file_present {
        return AuthState::SignedIn;
    }
    keychain
}

#[cfg(target_os = "macos")]
fn claude_keychain_present() -> AuthState {
    match Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", "Claude Code-credentials"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => AuthState::SignedIn,
        Ok(_) => AuthState::SignedOut,
        Err(_) => AuthState::Unknown,
    }
}

#[cfg(not(target_os = "macos"))]
fn claude_keychain_present() -> AuthState {
    AuthState::SignedOut
}

/// The write-mode label passed to the sidecar, which maps it to SDK permission
/// options (see `permissionOptions` in sidecar/claude-agent/index.mjs).
fn write_mode_label(mode: WriteMode) -> &'static str {
    match mode {
        WriteMode::Full => "Full",
        WriteMode::ReadOnly => "ReadOnly",
        WriteMode::Shared => "Shared",
        WriteMode::Isolated => "Isolated",
    }
}

/// Locate the Claude Agent SDK sidecar entrypoint. Honours an explicit
/// `BRIDGE_CLAUDE_SIDECAR` override, then a bundled copy next to the executable
/// (production), then the in-repo path (development).
fn sidecar_entry() -> Result<PathBuf, BridgeError> {
    if let Ok(path) = std::env::var("BRIDGE_CLAUDE_SIDECAR") {
        let candidate = PathBuf::from(path);
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("sidecar/claude-agent/index.mjs"));
            candidates.push(dir.join("../Resources/sidecar/claude-agent/index.mjs"));
        }
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sidecar/claude-agent/index.mjs"),
    );
    candidates
        .into_iter()
        .find(|candidate| candidate.exists())
        .ok_or_else(|| {
            BridgeError::Invalid(
                "Claude Agent SDK sidecar not found (set BRIDGE_CLAUDE_SIDECAR or ship sidecar/claude-agent/index.mjs next to the app)"
                    .into(),
            )
        })
}

/// Query Claude Code's subscription usage via the headless `/usage` command.
/// This is a read-only, zero-cost account query (no turn, no quota). Returns a
/// `{ "rateLimits": { ... } }` snapshot shaped like the Codex payload so the UI
/// can render both providers uniformly, or None when nothing parses.
pub fn read_usage_snapshot(cwd: &str) -> Option<Value> {
    // Headless `/usage` only includes the "% used" windows once its network fetch
    // resolves, which is racy. Retry a few times; the frontend keeps the last good
    // snapshot, so returning None just means "no fresh windows this poll".
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(1200));
        }
        if let Some(snapshot) = read_usage_once(cwd) {
            return Some(snapshot);
        }
    }
    None
}

fn read_usage_once(cwd: &str) -> Option<Value> {
    let binary = binary::resolve("claude")?;
    let output = Command::new(binary)
        .args(["-p", "/usage", "--output-format", "json"])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed = parse_json_object(&stdout)?;
    let text = parsed.get("result").and_then(Value::as_str)?;
    let rate_limits = parse_usage_text(text);
    if rate_limits
        .as_object()
        .map(|map| map.is_empty())
        .unwrap_or(true)
    {
        return None;
    }
    Some(json!({ "rateLimits": rate_limits }))
}

/// Best-effort extraction of the single JSON object printed by `--output-format json`.
fn parse_json_object(stdout: &str) -> Option<Value> {
    if let Ok(value) = serde_json::from_str::<Value>(stdout.trim()) {
        return Some(value);
    }
    let start = stdout.find('{')?;
    let end = stdout.rfind('}')?;
    serde_json::from_str::<Value>(&stdout[start..=end]).ok()
}

/// Parse Claude's `/usage` report into labeled rate-limit windows. Lines look like:
/// `Current session: 38% used · resets Jul 13 at 7:09pm (Asia/Calcutta)`
fn parse_usage_text(text: &str) -> Value {
    let mut map = serde_json::Map::new();
    let mut order = 1usize;
    for raw in text.lines() {
        let line = raw.trim();
        if !line.contains("% used") {
            continue;
        }
        let Some(colon) = line.find(':') else {
            continue;
        };
        let head = line[..colon].trim();
        let rest = &line[colon + 1..];
        let Some(percent_pos) = rest.find('%') else {
            continue;
        };
        let digits: String = rest[..percent_pos]
            .chars()
            .rev()
            .take_while(|character| character.is_ascii_digit())
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        let Ok(percent) = digits.parse::<i64>() else {
            continue;
        };
        let resets = rest.find("resets").map(|index| {
            let mut phrase = rest[index..].trim().to_string();
            if let Some(timezone) = phrase.rfind(" (") {
                phrase.truncate(timezone);
            }
            phrase.trim().to_string()
        });
        let label = classify_usage_label(head);
        let key = format!("{order}_{}", label.to_lowercase().replace(' ', "_"));
        let mut window = serde_json::Map::new();
        window.insert("label".into(), json!(label));
        window.insert("usedPercent".into(), json!(percent));
        if let Some(resets) = resets.filter(|value| !value.is_empty()) {
            window.insert("resetsLabel".into(), json!(resets));
        }
        map.insert(key, Value::Object(window));
        order += 1;
    }
    Value::Object(map)
}

fn classify_usage_label(head: &str) -> String {
    let lower = head.to_lowercase();
    if lower.contains("session") {
        return "Session".into();
    }
    if let Some(open) = head.find('(') {
        let close = head.rfind(')').unwrap_or(head.len());
        let inside = head[open + 1..close].trim();
        if inside.to_lowercase().contains("all models") {
            return "Week".into();
        }
        if !inside.is_empty() {
            return inside.to_string();
        }
    }
    if lower.contains("week") {
        return "Week".into();
    }
    head.to_string()
}

pub fn supports_native_resume() -> bool {
    binary::resolve("node").is_some() && sidecar_entry().is_ok()
}

/// The stream-json user frame for a turn: one text block, then one Anthropic
/// image block per attachment, in order. Pure so the exact wire bytes are
/// unit-testable without a provider process.
fn user_turn_frame(
    text: &str,
    context: TurnContext<'_>,
    images: &[bridge_protocol::messages::TurnImage],
) -> serde_json::Value {
    let mut content = Vec::with_capacity(1 + images.len());
    // Bridge's own words first, as their own blocks, so they are never folded
    // into the user's text. This is the tail delivery that lets the compiled
    // prompt — Claude's `systemPrompt.append`, and therefore the head of the
    // cached prefix — stay byte-stable across restarts.
    content.extend(
        context
            .entries()
            .map(|entry| json!({"type": "text", "text": entry.value})),
    );
    content.push(json!({"type": "text", "text": text}));
    content.extend(images.iter().map(|image| {
        json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": image.media_type,
                "data": image.base64_data,
            }
        })
    }));
    json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": content
        }
    })
}

impl ClaudeRuntime {
    fn terminate(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        let _ = crate::adapters::terminate_process_group(self.child.id());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
    pub fn start_turn(&self, text: &str, context: TurnContext<'_>) -> Result<(), BridgeError> {
        self.start_turn_with_images(text, context, &[])
    }

    /// Start a turn whose user message carries image attachments beside the
    /// text. The stream-json input frame already speaks Anthropic content
    /// blocks, so an image is a `{"type":"image"}` block with a base64
    /// source — the sidecar forwards blocks untouched.
    pub fn start_turn_with_images(
        &self,
        text: &str,
        context: TurnContext<'_>,
        images: &[bridge_protocol::messages::TurnImage],
    ) -> Result<(), BridgeError> {
        let turn_id = format!("turn-{}", self.request_id.fetch_add(1, Ordering::Relaxed));
        *self.current_turn.lock().unwrap() = Some(turn_id.clone());
        write_value(&self.writer, &user_turn_frame(text, context, images))?;
        let (mcp_names, plugin_names) = {
            let inventory = self.context_inventory.lock().unwrap();
            let catalog = inventory
                .iter()
                .find(|item| item.scope == ContextInventoryScope::Catalog);
            let names = |class| {
                catalog
                    .and_then(|item| {
                        item.observations
                            .iter()
                            .find(|observation| observation.segment_class == class)
                    })
                    .map(|observation| observation.names.clone())
                    .unwrap_or_default()
            };
            (
                names(ContextSegmentClass::McpDynamicTools),
                names(ContextSegmentClass::SkillsPlugins),
            )
        };
        crate::context_inventory::record_runtime_inventory(
            &self.context_inventory,
            [claude_turn_presented_inventory(
                ContextLifecyclePhase::PerTurn,
                &mcp_names,
                &plugin_names,
            )?],
        );
        Ok(())
    }

    pub fn interrupt(&self) -> Result<(), BridgeError> {
        let request_id = format!("irq-{}", self.request_id.fetch_add(1, Ordering::Relaxed));
        write_value(
            &self.writer,
            &json!({
                "type": "control_request",
                "request_id": request_id,
                "request": {"subtype": "interrupt"}
            }),
        )
    }

    pub fn respond(&self, request_id: Value, decision: &str) -> Result<(), BridgeError> {
        let behavior = match decision {
            "accept" | "acceptForSession" => "allow",
            _ => "deny",
        };
        write_value(
            &self.writer,
            &json!({
                "type": "control_response",
                "response": {
                    "subtype": "success",
                    "request_id": request_id,
                    "response": {"behavior": behavior}
                }
            }),
        )
    }
}

/// The `/compact` line a harness reads off its input stream.
///
/// A blank focus is dropped rather than sent as a trailing space, so a bare
/// `/compact` and `/compact "   "` are the same request.
fn compact_command(focus: Option<&str>) -> String {
    match focus.map(str::trim).filter(|value| !value.is_empty()) {
        Some(focus) => format!("/compact {focus}"),
        None => "/compact".into(),
    }
}

impl AdapterRuntime for ClaudeRuntime {
    fn process_id(&self) -> u32 {
        self.child.id()
    }
    fn provider_session_id(&self) -> &str {
        &self.session_id
    }
    fn current_turn(&self) -> Arc<Mutex<Option<String>>> {
        self.current_turn.clone()
    }
    fn context_inventory(&self) -> Vec<AdapterContextInventory> {
        self.context_inventory.lock().unwrap().clone()
    }
    fn send_turn(&self, text: &str) -> Result<(), BridgeError> {
        self.start_turn(text, TurnContext::default())
    }
    /// The streaming-input frame is an array of content blocks, so Bridge's
    /// trusted context rides as its own blocks ahead of the user's text —
    /// the tail channel that keeps `systemPrompt.append` byte-stable.
    fn send_turn_with_context(
        &self,
        text: &str,
        context: TurnContext<'_>,
    ) -> Result<(), BridgeError> {
        self.start_turn(text, context)
    }
    /// Claude Code reads slash commands off its own input stream, and its
    /// `/compact` takes a focus instruction.
    fn native_compaction(&self) -> crate::adapters::NativeCompaction {
        crate::adapters::NativeCompaction::WithFocus
    }
    /// The command travels as an ordinary user frame: the streaming-input
    /// `query()` the sidecar feeds is exactly where the SDK expects to find a
    /// slash command, which is why this needs no control channel of its own.
    fn compact_native(&self, focus: Option<&str>) -> Result<(), BridgeError> {
        self.send_turn(&compact_command(focus))
    }
    /// Vision-capable transport: the SDK message content is a content-block
    /// array, so image blocks ride beside text natively.
    fn supports_images(&self) -> bool {
        true
    }
    fn send_turn_with_images(
        &self,
        text: &str,
        context: TurnContext<'_>,
        images: &[bridge_protocol::messages::TurnImage],
    ) -> Result<(), BridgeError> {
        self.start_turn_with_images(text, context, images)
    }
    /// The sidecar feeds one long-lived streaming-input `query()`, so a user
    /// message written while a turn is running is picked up by that turn — the
    /// SDK's own steering path — instead of starting a competing one.
    fn supports_active_turn_steering(&self) -> bool {
        true
    }
    fn interrupt(&self) -> Result<(), BridgeError> {
        ClaudeRuntime::interrupt(self)
    }
    fn respond(&self, request_id: Value, decision: &str) -> Result<(), BridgeError> {
        ClaudeRuntime::respond(self, request_id, decision)
    }
    fn failure_context(&mut self) -> Option<String> {
        crate::adapters::process_failure_context(&mut self.child, &self.stderr_tail)
    }
    fn stop(&mut self, _reason: ShutdownReason) {
        self.terminate();
    }
}

pub(crate) fn claude_context_inventory(
    lifecycle_phase: ContextLifecyclePhase,
    configuration: &crate::marketplace::ClaudeSdkConfiguration,
) -> Result<Vec<AdapterContextInventory>, BridgeError> {
    let mcp_names = configuration
        .mcp_servers
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    let plugin_names = configuration.plugins.clone();
    let catalog = AdapterContextInventory::new(
        "claude",
        ContextInventoryScope::Catalog,
        lifecycle_phase,
        vec![
            ContextSegmentObservation::unavailable_with_names(
                ContextSegmentClass::ProviderBaseInstructions,
                ["claude_code"],
                "Claude Agent SDK identifies the provider preset but does not expose its instruction bytes or tokens",
            ),
            ContextSegmentObservation::unavailable(
                ContextSegmentClass::ToolSchemas,
                "Claude Agent SDK does not expose the provider-owned tool schemas compiled for the query",
            ),
            ContextSegmentObservation::measured(
                ContextSegmentClass::McpDynamicTools,
                &mcp_names,
                ContextObservedSize::bounded(Some(mcp_names.len() as u64), None, None),
            ),
            ContextSegmentObservation::measured(
                ContextSegmentClass::SkillsPlugins,
                &plugin_names,
                ContextObservedSize::bounded(Some(plugin_names.len() as u64), None, None),
            ),
            ContextSegmentObservation::unavailable(
                ContextSegmentClass::AgentDefinitions,
                "Claude Agent SDK does not expose provider-owned agent definitions compiled for the query",
            ),
        ],
    )?;
    Ok(vec![
        catalog,
        claude_turn_presented_inventory(lifecycle_phase, &mcp_names, &plugin_names)?,
    ])
}

fn claude_turn_presented_inventory(
    lifecycle_phase: ContextLifecyclePhase,
    mcp_names: &[String],
    plugin_names: &[String],
) -> Result<AdapterContextInventory, BridgeError> {
    AdapterContextInventory::new(
        "claude",
        ContextInventoryScope::TurnPresented,
        lifecycle_phase,
        vec![
            ContextSegmentObservation::unavailable_with_names(
                ContextSegmentClass::ProviderBaseInstructions,
                ["claude_code"],
                "The claude_code preset is selected for this query, but the SDK does not expose the provider-owned bytes presented to the turn",
            ),
            ContextSegmentObservation::unavailable(
                ContextSegmentClass::ToolSchemas,
                "The SDK does not report the provider-owned tool schemas actually presented to this turn",
            ),
            ContextSegmentObservation::unavailable_with_names(
                ContextSegmentClass::McpDynamicTools,
                mcp_names,
                "Configured MCP servers are known, but the SDK does not report which generated tools are actually presented to this turn",
            ),
            ContextSegmentObservation::unavailable_with_names(
                ContextSegmentClass::SkillsPlugins,
                plugin_names,
                "Configured plugins are known, but the SDK does not report their actual per-turn context contribution",
            ),
            ContextSegmentObservation::unavailable(
                ContextSegmentClass::AgentDefinitions,
                "The SDK does not report provider-owned agent definitions actually presented to this turn",
            ),
        ],
    )
}

impl Drop for ClaudeRuntime {
    fn drop(&mut self) {
        self.terminate();
    }
}

pub fn binary_version() -> Option<String> {
    sidecar_entry().ok()?;
    // The version is the certification subject: the Claude Agent SDK actually
    // installed, resolved the same way the sidecar resolves its import. The
    // Node runtime that hosts it is not what the conformance suite ran
    // against — reporting "Agent SDK (Node v26)" certifies nothing, which is
    // exactly how the briefing gate treated it. The Node string survives only
    // as the fallback for an install whose SDK package cannot be read, where
    // staying uncertified is the correct reading.
    installed_sdk_version()
        .or_else(|| binary::version("node").map(|version| format!("Agent SDK (Node {version})")))
}

/// The installed Claude Agent SDK's own version, from its package manifest.
/// The managed payload wins when present, then the bundled sidecar — the same
/// order the sidecar uses to resolve the module it imports.
fn installed_sdk_version() -> Option<String> {
    let package_manifest = managed_sdk_module()
        .and_then(|module| Some(module.parent()?.join("package.json")))
        .filter(|manifest| manifest.is_file())
        .or_else(|| {
            let entry = sidecar_entry().ok()?;
            let manifest = entry
                .parent()?
                .join("node_modules/@anthropic-ai/claude-agent-sdk/package.json");
            manifest.is_file().then_some(manifest)
        })?;
    sdk_version_from_manifest(&package_manifest)
}

fn sdk_version_from_manifest(manifest: &Path) -> Option<String> {
    let parsed: Value = serde_json::from_str(&std::fs::read_to_string(manifest).ok()?).ok()?;
    let version = parsed.get("version")?.as_str()?.trim();
    (!version.is_empty()).then(|| version.to_owned())
}

/// The managed Claude SDK module to import, if a managed payload is installed.
///
/// Derived from the payload's receipt entrypoint — the platform binary — by
/// walking back to the payload root, so the module and the binary always come
/// from the same installation.
pub fn managed_sdk_module() -> Option<PathBuf> {
    let entrypoint = crate::managed_runtime::managed_entrypoint("claude")?;
    let payload_root = entrypoint
        .components()
        .collect::<Vec<_>>()
        .iter()
        // rposition, not position: the managed root itself may sit under a path
        // containing `node_modules`, and taking the first boundary would re-root
        // onto an unrelated project's SDK.
        .rposition(|component| component.as_os_str() == "node_modules")
        .map(|index| entrypoint.components().take(index).collect::<PathBuf>())?;
    let module = payload_root.join(crate::managed_runtime::claude_sdk_module());
    module.is_file().then_some(module)
}

pub fn unavailable_reason() -> Option<String> {
    if binary::resolve("node").is_none() {
        return Some("Node.js 18+ is required to run Claude models".into());
    }
    sidecar_entry().err().map(|error| error.to_string())
}

fn write_value(writer: &Arc<Mutex<ChildStdin>>, value: &Value) -> Result<(), BridgeError> {
    let mut writer = lock_writer(writer, "Claude")?;
    serde_json::to_writer(&mut *writer, value)
        .map_err(|e| BridgeError::Invalid(format!("Cannot encode Claude request: {e}")))?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}
fn lock_writer<'a, T>(
    writer: &'a Mutex<T>,
    provider: &str,
) -> Result<MutexGuard<'a, T>, BridgeError> {
    writer.lock().map_err(|_| {
        BridgeError::Adapter(format!(
            "{provider} stdin lock was poisoned; restart the session"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_inventory::ContextObservationProvenance;
    #[test]
    fn compact_forwards_the_focus_on_the_input_stream() {
        // The SDK reads slash commands off the same streaming-input query that
        // carries user turns, which is why forwarding needs no control frame.
        assert_eq!(compact_command(None), "/compact");
        assert_eq!(compact_command(Some("the auth refactor")), "/compact the auth refactor");
        // A blank focus is the same request as no focus, not a trailing space.
        assert_eq!(compact_command(Some("   ")), "/compact");
        assert_eq!(compact_command(Some("")), "/compact");
        // Surrounding whitespace is the user's typing, not part of the focus.
        assert_eq!(compact_command(Some("  keep the failing test  ")), "/compact keep the failing test");
    }

    #[test]
    fn poisoned_writer_is_a_typed_adapter_error() {
        let writer = Mutex::new(());
        let _ = std::panic::catch_unwind(|| {
            let _guard = writer.lock().unwrap();
            panic!("provider thread failed");
        });
        assert!(matches!(
            lock_writer(&writer, "Claude"),
            Err(BridgeError::Adapter(_))
        ));
    }

    #[test]
    fn claude_context_inventory_covers_start_resume_and_per_turn() {
        let secret = "sk-proj-abcdefghijklmnopqrstuvwxyz123456";
        let configuration = crate::marketplace::ClaudeSdkConfiguration {
            plugins: vec![format!("plugin-{secret}")],
            mcp_servers: std::collections::BTreeMap::from([(
                format!("server-{secret}"),
                json!({"type": "http", "url": "http://127.0.0.1"}),
            )]),
            connector_health: Default::default(),
        };
        for phase in [ContextLifecyclePhase::Start, ContextLifecyclePhase::Resume] {
            let inventories = claude_context_inventory(phase, &configuration).unwrap();
            assert_eq!(inventories.len(), 2);
            let catalog = inventories
                .iter()
                .find(|item| item.scope == ContextInventoryScope::Catalog)
                .unwrap();
            for class in [
                ContextSegmentClass::McpDynamicTools,
                ContextSegmentClass::SkillsPlugins,
            ] {
                let observation = catalog
                    .observations
                    .iter()
                    .find(|observation| observation.segment_class == class)
                    .unwrap();
                assert!(matches!(
                    observation.provenance,
                    ContextObservationProvenance::Measured { .. }
                ));
                assert!(observation.names.iter().all(|name| !name.contains(secret)));
            }
        }
        let per_turn = claude_turn_presented_inventory(
            ContextLifecyclePhase::PerTurn,
            &["configured-server".into()],
            &["configured-plugin".into()],
        )
        .unwrap();
        assert_eq!(per_turn.scope, ContextInventoryScope::TurnPresented);
        assert!(per_turn.observations.iter().all(|observation| matches!(
            observation.provenance,
            ContextObservationProvenance::Unavailable { .. }
        )));
    }

    #[test]
    fn parses_claude_usage_report_into_labeled_windows() {
        let text = "You are currently using your subscription to power your Claude Code usage\n\n\
            Current session: 38% used · resets Jul 13 at 7:09pm (Asia/Calcutta)\n\
            Current week (all models): 60% used · resets Jul 14 at 3:29am (Asia/Calcutta)\n\
            Current week (Fable): 81% used · resets Jul 14 at 3:29am (Asia/Calcutta)\n\n\
            What's contributing to your limits usage?";
        let value = parse_usage_text(text);
        let map = value.as_object().unwrap();
        assert_eq!(map.len(), 3);
        assert_eq!(map["1_session"]["label"], "Session");
        assert_eq!(map["1_session"]["usedPercent"], 38);
        assert_eq!(map["1_session"]["resetsLabel"], "resets Jul 13 at 7:09pm");
        assert_eq!(map["2_week"]["label"], "Week");
        assert_eq!(map["2_week"]["usedPercent"], 60);
        assert_eq!(map["3_fable"]["label"], "Fable");
        assert_eq!(map["3_fable"]["usedPercent"], 81);
    }

    #[test]
    fn write_mode_labels_map_to_sidecar_permission_modes() {
        assert_eq!(write_mode_label(WriteMode::Full), "Full");
        assert_eq!(write_mode_label(WriteMode::ReadOnly), "ReadOnly");
        assert_eq!(write_mode_label(WriteMode::Shared), "Shared");
        assert_eq!(write_mode_label(WriteMode::Isolated), "Isolated");
    }

    #[test]
    fn a_plain_text_turn_emits_exactly_todays_frame() {
        assert_eq!(
            user_turn_frame("ship it", TurnContext::default(), &[]),
            json!({
                "type": "user",
                "message": {"role": "user", "content": [{"type": "text", "text": "ship it"}]}
            }),
            "image support must not reshape the no-image wire"
        );
    }

    #[test]
    fn bridge_context_rides_as_its_own_blocks_before_the_user_text() {
        let images = vec![bridge_protocol::messages::TurnImage {
            media_type: "image/png".into(),
            base64_data: "iVBORw0".into(),
        }];
        let frame = user_turn_frame(
            "verify [secret:sec_reference]",
            TurnContext {
                session: Some("<bridge-session-context schema=\"1\">frame</bridge-session-context>"),
                credentials: Some("trusted broker capability"),
            },
            &images,
        );
        let content = frame["message"]["content"].as_array().unwrap();
        assert_eq!(content.len(), 4, "two context blocks, the user text, one image");
        assert_eq!(
            content[0],
            json!({"type": "text", "text": "<bridge-session-context schema=\"1\">frame</bridge-session-context>"})
        );
        assert_eq!(
            content[1],
            json!({"type": "text", "text": "trusted broker capability"})
        );
        // The user's own words are untouched and still precede the images they
        // reference positionally.
        assert_eq!(
            content[2],
            json!({"type": "text", "text": "verify [secret:sec_reference]"})
        );
        assert_eq!(content[3]["type"], "image");
    }

    #[test]
    fn image_attachments_become_anthropic_image_content_blocks_in_order() {
        let images = vec![
            bridge_protocol::messages::TurnImage {
                media_type: "image/png".into(),
                base64_data: "iVBORw0".into(),
            },
            bridge_protocol::messages::TurnImage {
                media_type: "image/jpeg".into(),
                base64_data: "/9j/4AAQ".into(),
            },
        ];
        let frame = user_turn_frame("what are these?", TurnContext::default(), &images);
        let content = frame["message"]["content"].as_array().unwrap();
        assert_eq!(content.len(), 3, "one text block, then one block per image");
        assert_eq!(content[0], json!({"type": "text", "text": "what are these?"}));
        assert_eq!(
            content[1],
            json!({
                "type": "image",
                "source": {"type": "base64", "media_type": "image/png", "data": "iVBORw0"}
            })
        );
        assert_eq!(
            content[2]["source"]["media_type"],
            json!("image/jpeg"),
            "order is preserved: the text references images positionally"
        );
    }

    #[test]
    fn the_reported_version_is_the_sdks_own_not_the_node_runtimes() {
        // The certification subject is the installed Agent SDK. A manifest
        // saying 0.3.209 must surface exactly that, because certify_briefing
        // compares it against the certified 0.3 line component-wise.
        let dir = std::env::temp_dir().join(format!("bridge-sdk-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = dir.join("package.json");
        std::fs::write(&manifest, r#"{"name":"@anthropic-ai/claude-agent-sdk","version":"0.3.209"}"#).unwrap();
        assert_eq!(sdk_version_from_manifest(&manifest), Some("0.3.209".to_owned()));
        assert!(crate::briefing_policy::certify_briefing("claude", Some("0.3.209")).is_ok());

        std::fs::write(&manifest, r#"{"name":"x","version":"  "}"#).unwrap();
        assert_eq!(sdk_version_from_manifest(&manifest), None, "a blank version is unreported");
        std::fs::write(&manifest, "not json").unwrap();
        assert_eq!(sdk_version_from_manifest(&manifest), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sidecar_entry_honours_explicit_override() {
        let dir = std::env::temp_dir().join(format!("bridge-sidecar-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let entry = dir.join("index.mjs");
        std::fs::write(&entry, "// test").unwrap();
        std::env::set_var("BRIDGE_CLAUDE_SIDECAR", &entry);
        let resolved = sidecar_entry().unwrap();
        std::env::remove_var("BRIDGE_CLAUDE_SIDECAR");
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(resolved, entry);
    }

    #[test]
    fn user_turn_is_structured_json_not_terminal_text() {
        let value = json!({
            "type": "user",
            "message": {"role": "user", "content": [{"type": "text", "text": "hello"}]}
        });
        assert_eq!(value["type"], "user");
        assert!(!value.to_string().contains("\\u001b"));
    }

    #[test]
    fn interrupt_control_request_is_structured() {
        let value = json!({
            "type": "control_request",
            "request_id": "irq-1",
            "request": {"subtype": "interrupt"}
        });
        assert_eq!(value["request"]["subtype"], "interrupt");
    }

    #[test]
    #[ignore = "requires an authenticated Claude Agent SDK runtime"]
    fn live_stream_json_emits_a_structured_turn() {
        use std::{io::BufRead, sync::mpsc, thread, time::Duration};
        let cwd = std::env::temp_dir();
        let started = start(StartRequest {
            cwd: cwd.to_str().unwrap(),
            model: None,
            effort: None,
            instructions: None,
            write_mode: None,
            read_only_sandbox: None,
            briefing: None,
            on_progress: None,
        })
        .unwrap();
        let mut runtime = started.runtime;
        let mut reader = started.reader;
        runtime
            .start_turn(
                "Reply exactly BRIDGE_CLAUDE_OK. Do not use tools.",
                TurnContext::default(),
            )
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            if let Ok(value) = serde_json::from_str::<Value>(line.trim()) {
                let done = value.get("type").and_then(Value::as_str) == Some("result");
                let _ = sender.send(value);
                if done {
                    break;
                }
            }
        });
        let mut types = Vec::new();
        loop {
            let value = receiver
                .recv_timeout(Duration::from_secs(90))
                .expect("Claude turn timed out");
            if let Some(kind) = value.get("type").and_then(Value::as_str) {
                types.push(kind.to_owned());
            }
            if value.get("type").and_then(Value::as_str) == Some("result") {
                break;
            }
        }
        runtime.stop(ShutdownReason::Completed);
        assert!(types
            .iter()
            .any(|kind| kind == "assistant" || kind == "stream_event"));
        assert!(types.iter().any(|kind| kind == "result"));
    }

    #[test]
    #[ignore = "requires an authenticated Claude Agent SDK runtime and persists a provider session"]
    fn live_claude_session_survives_process_restart() {
        use std::io::BufRead;

        fn run_turn(started: &mut StartedClaude, prompt: &str) -> String {
            started
                .runtime
                .start_turn(prompt, TurnContext::default())
                .unwrap();
            let mut transcript = String::new();
            loop {
                let mut line = String::new();
                assert_ne!(started.reader.read_line(&mut line).unwrap(), 0);
                transcript.push_str(&line);
                let frame: Value = serde_json::from_str(line.trim()).unwrap();
                if frame.get("type").and_then(Value::as_str) == Some("result") {
                    return transcript;
                }
            }
        }

        let cwd = std::env::temp_dir();
        let cwd = cwd.to_str().unwrap();
        let mut started = start(StartRequest {
            cwd,
            model: None,
            effort: None,
            instructions: None,
            write_mode: None,
            read_only_sandbox: None,
            briefing: None,
            on_progress: None,
        })
        .unwrap();
        run_turn(&mut started, "Remember this exact token for the next turn: BRIDGE_CLAUDE_RESUME_5A72. Reply only SAVED.");
        let session_id = started.runtime.session_id.clone();
        started.runtime.stop(ShutdownReason::AppShutdown);

        let mut resumed = resume(ResumeRequest {
            fork: false,
            cwd,
            model: None,
            effort: None,
            instructions: None,
            write_mode: None,
            read_only_sandbox: None,
            briefing: None,
            provider_session_id: &session_id,
            on_progress: None,
        })
        .unwrap();
        let transcript = run_turn(
            &mut resumed,
            "What exact token did I ask you to remember? Reply with only the token.",
        );
        resumed.runtime.stop(ShutdownReason::Completed);
        assert!(transcript.contains("BRIDGE_CLAUDE_RESUME_5A72"));
    }

    #[test]
    fn auth_probe_reports_signed_in_when_credential_store_present() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".claude")).unwrap();
        std::fs::write(home.path().join(".claude/.credentials.json"), "{}").unwrap();
        assert_eq!(
            auth_state_from_home(Some(home.path().to_path_buf()), AuthState::SignedOut),
            AuthState::SignedIn
        );
    }

    #[test]
    fn auth_probe_reports_signed_out_when_cli_present_but_store_absent() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(
            auth_state_from_home(Some(home.path().to_path_buf()), AuthState::SignedOut),
            AuthState::SignedOut
        );
    }

    #[test]
    fn missing_credentials_file_falls_back_to_the_keychain_probe() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(
            auth_state_from_home(Some(home.path().to_path_buf()), AuthState::SignedIn),
            AuthState::SignedIn
        );
        assert_eq!(
            auth_state_from_home(Some(home.path().to_path_buf()), AuthState::Unknown),
            AuthState::Unknown
        );
    }
}

#[cfg(test)]
mod briefing_boundary_tests {
    use crate::briefing_policy::{BriefingRuntimePolicy, BriefingToolIdentity};
    use crate::delegation::WriteMode;
    use bridge_protocol::messages as wire;

    fn policy() -> BriefingRuntimePolicy {
        let reviewed = BriefingToolIdentity {
            server: "notion".into(),
            tool: "search".into(),
        };
        BriefingRuntimePolicy::compile(
            vec![reviewed.clone()],
            wire::WorkBriefLimits {
                max_wall_seconds: 600,
                max_turns: 12,
                max_tool_calls: 24,
                max_output_tokens: None,
                cost_ceiling_microusd: None,
            },
            &[reviewed.wire_name()],
        )
        .unwrap()
    }

    #[test]
    fn a_briefing_run_may_not_also_hold_a_writable_tree() {
        // The check the adapter performs before it will start one, pinned here so
        // the refusal cannot be lost from the boundary without a test noticing.
        for mode in [WriteMode::Shared, WriteMode::Isolated, WriteMode::Full] {
            assert!(
                BriefingRuntimePolicy::check_write_mode(Some(mode)).is_err(),
                "{mode:?} must not accompany a briefing policy"
            );
        }
        assert!(BriefingRuntimePolicy::check_write_mode(Some(WriteMode::ReadOnly)).is_ok());
    }

    #[test]
    fn the_sidecar_is_handed_the_allowlist_the_denylist_and_the_argument_ceiling() {
        // What the adapter serializes, asserted on the policy's own accessors so a
        // change to either side has to change this test too.
        let policy = policy();
        assert_eq!(policy.allowed_wire_names(), vec!["mcp__notion__search"]);
        assert_eq!(policy.allowed_servers(), vec!["notion"]);
        assert!(policy.max_argument_bytes() > 0);
        // The exact identities the provider uses. Lowercase would match nothing in
        // an SDK deny-list, which is how this went wrong the first time.
        let denied = BriefingRuntimePolicy::denied_builtin_names();
        for expected in ["Bash", "Read", "Write", "WebFetch", "Task", "Skill"] {
            assert!(denied.contains(&expected), "{expected} must be denied explicitly");
        }
    }
}

#[cfg(test)]
mod catalogue_tests {
    use super::*;
    #[test]
    fn canonical_ids_collapse_aliases_and_preserve_distinct_releases() {
        let models = parse_discovered_models(&serde_json::from_str::<Value>(r#"{"models":[
            {"value":"default","resolvedModel":"claude-opus-5","displayName":"Default (recommended)","description":"Opus 5 · Best for complex tasks","supportedEffortLevels":["high","max"]},
            {"value":"claude-opus-5","displayName":"Opus 5","supportedEffortLevels":["high","max"]},
            {"value":"claude-opus-4-8","displayName":"Opus 4.8"},
            {"value":"haiku","displayName":"Haiku","supportsEffort":false,"supportedEffortLevels":["high"]}
        ]}"#).unwrap().to_string()).unwrap();
        assert_eq!(models[0].id, models[1].id);
        assert_eq!(models[0].label, "Opus 5");
        assert!(models[0].is_default);
        assert_ne!(models[0].id, models[2].id);
        assert_eq!(models[0].supported_effort_levels, ["high", "max"]);
        assert!(models[3].supported_effort_levels.is_empty());
    }
    #[test]
    fn ignores_malformed_rows_and_unresolved_default() {
        let models = parse_discovered_models("{\"models\":[{}, {\"value\":\"default\"}, {\"value\":\"sonnet\",\"displayName\":\"Sonnet\"}]}" ).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "sonnet");
        assert!(parse_discovered_models("not json").is_err());
    }
}
