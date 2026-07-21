use crate::BridgeError;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
};
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::{fs::PermissionsExt, net::UnixListener};

pub const NATIVE_HOST_NAME: &str = "dev.bridge.deck.browser";
pub const CHROME_EXTENSION_ID: &str = "jocamgijenfmpopdfecjfnjdnohhoool";
const LEASE_MINUTES: i64 = 30;
const AUDIT_LIMIT: usize = 250;
const DEBUG_LIMIT: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTab {
    pub id: i64,
    pub title: String,
    pub url: String,
    pub domain: Option<String>,
    pub fav_icon_url: Option<String>,
    pub attached: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TabLease {
    pub id: String,
    pub tab_id: i64,
    pub domain: String,
    pub status: String,
    pub permission: String,
    pub attached_at: String,
    pub expires_at: String,
    pub last_activity_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserAuditEvent {
    pub id: String,
    pub kind: String,
    pub summary: String,
    pub command_id: Option<String>,
    pub domain: Option<String>,
    pub created_at: String,
    pub data: Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TokenAccounting {
    pub snapshots: u64,
    pub full_snapshots: u64,
    pub delta_snapshots: u64,
    pub serialized_bytes: u64,
    pub estimated_input_tokens: u64,
    pub screenshot_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserApproval {
    pub id: String,
    pub command_id: String,
    pub action: String,
    pub effect: String,
    pub domain: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SiteMetric {
    pub domain: String,
    pub actions: u64,
    pub successes: u64,
    pub failures: u64,
    pub total_latency_ms: u64,
    pub input_tokens: u64,
    pub screenshots: u64,
    pub interventions: u64,
    pub approvals: u64,
    pub duplicate_side_effects: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserBridgeSnapshot {
    pub transport_connected: bool,
    pub extension_id: &'static str,
    pub extension_path: String,
    pub native_host_installed: bool,
    pub native_host_manifest_path: Option<String>,
    pub tabs: Vec<BrowserTab>,
    pub lease: Option<TabLease>,
    pub status: String,
    pub capture_active: bool,
    pub capture_error: Option<String>,
    pub screenshot: Option<String>,
    pub screenshot_redacted_regions: usize,
    pub elements: Vec<Value>,
    pub viewport: Value,
    pub prompt_injection_suspected: bool,
    pub prompt_injection_signals: Vec<String>,
    pub token_accounting: TokenAccounting,
    pub pending_approval: Option<BrowserApproval>,
    pub audit: Vec<BrowserAuditEvent>,
    pub debug_events: Vec<Value>,
    pub site_metrics: Vec<SiteMetric>,
    pub remote_provider: Option<RemoteBrowserConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserActionRequest {
    pub kind: String,
    pub element_id: Option<String>,
    pub text: Option<String>,
    pub url: Option<String>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub tab_id: Option<i64>,
    pub sensitive_kind: Option<String>,
    pub expected_domain: Option<String>,
    pub actor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserRouteRequest {
    pub structured_api_available: bool,
    pub needs_user_auth: bool,
    pub needs_isolation: bool,
    pub needs_parallelism: bool,
    pub needs_geo_or_proxy: bool,
    pub unattended: bool,
    pub dom_control_available: bool,
    pub remote_provider_configured: bool,
    pub task_class: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserRouteDecision {
    pub route: String,
    pub reason: String,
    pub requires_user_grant: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBrowserConfig {
    pub endpoint: String,
    pub bearer_token_env: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSkill {
    pub id: String,
    pub name: String,
    pub domains: Vec<String>,
    pub description: String,
    pub steps: Vec<Value>,
}

#[derive(Debug, Clone)]
struct QueuedCommand {
    id: String,
    action: Value,
    kind: String,
    domain: String,
    started_at: DateTime<Utc>,
}

#[derive(Default)]
struct Inner {
    transport_connected: bool,
    native_host_manifest_path: Option<PathBuf>,
    tabs: Vec<BrowserTab>,
    lease: Option<TabLease>,
    status: String,
    capture_active: bool,
    capture_error: Option<String>,
    screenshot: Option<String>,
    screenshot_redacted_regions: usize,
    elements: Vec<Value>,
    viewport: Value,
    prompt_injection_suspected: bool,
    prompt_injection_signals: Vec<String>,
    token_accounting: TokenAccounting,
    pending_approval: Option<BrowserApproval>,
    pending_approval_command: Option<QueuedCommand>,
    inflight: HashMap<String, QueuedCommand>,
    completed: HashMap<String, bool>,
    audit: VecDeque<BrowserAuditEvent>,
    debug_events: VecDeque<Value>,
    site_metrics: HashMap<String, SiteMetric>,
    remote_provider: Option<RemoteBrowserConfig>,
}

pub struct BrowserBridgeSupervisor {
    inner: Mutex<Inner>,
    outbound: Mutex<Option<(u64, mpsc::Sender<Value>)>>,
    next_connection: AtomicU64,
    extension_path: PathBuf,
    socket_path: PathBuf,
    metrics_path: PathBuf,
    remote_config_path: PathBuf,
    audit_path: PathBuf,
}

impl BrowserBridgeSupervisor {
    pub fn start(extension_path: PathBuf, metrics_path: PathBuf) -> Arc<Self> {
        let site_metrics = fs::read(&metrics_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<SiteMetric>>(&bytes).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|metric| (metric.domain.clone(), metric))
            .collect();
        let remote_config_path = metrics_path.with_file_name("browser-remote-config.json");
        let remote_provider = fs::read(&remote_config_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        let audit_path = metrics_path.with_file_name("browser-audit.json");
        let audit = fs::read(&audit_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<VecDeque<BrowserAuditEvent>>(&bytes).ok())
            .unwrap_or_default();
        let supervisor = Arc::new(Self {
            inner: Mutex::new(Inner {
                status: "not_attached".into(),
                site_metrics,
                remote_provider,
                audit,
                ..Inner::default()
            }),
            outbound: Mutex::new(None),
            next_connection: AtomicU64::new(1),
            extension_path,
            socket_path: std::env::temp_dir().join("dev.bridge.deck.browser.sock"),
            metrics_path,
            remote_config_path,
            audit_path,
        });
        #[cfg(unix)]
        Self::start_socket(Arc::clone(&supervisor));
        supervisor
    }

    #[cfg(unix)]
    fn start_socket(supervisor: Arc<Self>) {
        let socket_path = supervisor.socket_path.clone();
        thread::spawn(move || {
            let pid_path = socket_path.with_extension("sock.pid");
            if socket_path.exists() {
                let active = fs::read_to_string(&pid_path)
                    .ok()
                    .map(|pid| pid.trim().to_owned())
                    .is_some_and(|pid| {
                        std::process::Command::new("kill")
                            .args(["-0", &pid])
                            .status()
                            .is_ok_and(|status| status.success())
                    });
                if active {
                    supervisor.audit(
                        "transport.error",
                        "Another Bridge instance already owns the browser socket".into(),
                        None,
                        json!({}),
                    );
                    return;
                }
                let _ = fs::remove_file(&socket_path);
                let _ = fs::remove_file(&pid_path);
            }
            let listener = match UnixListener::bind(&socket_path) {
                Ok(listener) => listener,
                Err(error) => {
                    supervisor.audit(
                        "transport.error",
                        format!("Native messaging socket failed: {error}"),
                        None,
                        json!({}),
                    );
                    return;
                }
            };
            let _ = fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600));
            let _ = fs::write(&pid_path, std::process::id().to_string());
            let _ = fs::set_permissions(&pid_path, fs::Permissions::from_mode(0o600));
            for connection in listener.incoming() {
                let stream = match connection {
                    Ok(stream) => stream,
                    Err(_) => continue,
                };
                let write_stream = match stream.try_clone() {
                    Ok(stream) => stream,
                    Err(_) => continue,
                };
                let (tx, rx) = mpsc::channel::<Value>();
                let connection_id = supervisor.next_connection.fetch_add(1, Ordering::Relaxed);
                *supervisor.outbound.lock().unwrap() = Some((connection_id, tx));
                supervisor.inner.lock().unwrap().transport_connected = true;
                supervisor.audit(
                    "transport.connected",
                    "Browser extension connected".into(),
                    None,
                    json!({}),
                );
                thread::spawn(move || {
                    let mut writer = write_stream;
                    while let Ok(message) = rx.recv() {
                        if writeln!(writer, "{message}").is_err() {
                            break;
                        }
                        if writer.flush().is_err() {
                            break;
                        }
                    }
                });
                let reader_supervisor = Arc::clone(&supervisor);
                thread::spawn(move || {
                    for line in BufReader::new(stream).lines() {
                        let Ok(line) = line else { break };
                        if let Ok(message) = serde_json::from_str::<Value>(&line) {
                            reader_supervisor.handle_extension_event(message);
                        }
                    }
                    let mut outbound = reader_supervisor.outbound.lock().unwrap();
                    if outbound
                        .as_ref()
                        .is_some_and(|(id, _)| *id == connection_id)
                    {
                        *outbound = None;
                        reader_supervisor.inner.lock().unwrap().transport_connected = false;
                        drop(outbound);
                        reader_supervisor.audit(
                            "transport.disconnected",
                            "Browser extension disconnected; side effects will not be replayed"
                                .into(),
                            None,
                            json!({}),
                        );
                    }
                });
            }
        });
    }

    pub fn snapshot(&self) -> BrowserBridgeSnapshot {
        let mut inner = self.inner.lock().unwrap();
        if let Some(lease) = inner.lease.as_mut() {
            if lease.status == "active"
                && DateTime::parse_from_rfc3339(&lease.expires_at)
                    .is_ok_and(|expires| expires < Utc::now())
            {
                lease.status = "expired".into();
                inner.status = "paused".into();
            }
        }
        let mut metrics = inner.site_metrics.values().cloned().collect::<Vec<_>>();
        metrics.sort_by(|a, b| a.domain.cmp(&b.domain));
        BrowserBridgeSnapshot {
            transport_connected: inner.transport_connected,
            extension_id: CHROME_EXTENSION_ID,
            extension_path: self.extension_path.to_string_lossy().into_owned(),
            native_host_installed: inner
                .native_host_manifest_path
                .as_ref()
                .is_some_and(|path| path.exists()),
            native_host_manifest_path: inner
                .native_host_manifest_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            tabs: inner.tabs.clone(),
            lease: inner.lease.clone(),
            status: inner.status.clone(),
            capture_active: inner.capture_active,
            capture_error: inner.capture_error.clone(),
            screenshot: inner.screenshot.clone(),
            screenshot_redacted_regions: inner.screenshot_redacted_regions,
            elements: inner.elements.clone(),
            viewport: inner.viewport.clone(),
            prompt_injection_suspected: inner.prompt_injection_suspected,
            prompt_injection_signals: inner.prompt_injection_signals.clone(),
            token_accounting: inner.token_accounting.clone(),
            pending_approval: inner.pending_approval.clone(),
            audit: inner.audit.iter().cloned().rev().collect(),
            debug_events: inner.debug_events.iter().cloned().rev().collect(),
            site_metrics: metrics,
            remote_provider: inner.remote_provider.clone(),
        }
    }

    pub fn install_native_host(&self, executable: &Path) -> Result<PathBuf, BridgeError> {
        #[cfg(not(target_os = "macos"))]
        return Err(BridgeError::Invalid(
            "Chrome native-host registration is currently supported on macOS".into(),
        ));
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var_os("HOME")
                .ok_or_else(|| BridgeError::Invalid("HOME is unavailable".into()))?;
            let application_support = PathBuf::from(home).join("Library/Application Support");
            let manifest = json!({
                "name": NATIVE_HOST_NAME,
                "description": "Bridge authenticated-tab native messaging relay",
                "path": executable,
                "type": "stdio",
                "allowed_origins": [format!("chrome-extension://{CHROME_EXTENSION_ID}/")]
            });
            let mut installed = Vec::new();
            for (index, browser_root) in
                ["Google/Chrome", "Chromium", "BraveSoftware/Brave-Browser"]
                    .into_iter()
                    .enumerate()
            {
                let root = application_support.join(browser_root);
                if index > 0 && !root.exists() {
                    continue;
                }
                let directory = root.join("NativeMessagingHosts");
                fs::create_dir_all(&directory)?;
                let path = directory.join(format!("{NATIVE_HOST_NAME}.json"));
                fs::write(
                    &path,
                    serde_json::to_vec_pretty(&manifest)
                        .map_err(|error| BridgeError::Invalid(error.to_string()))?,
                )?;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
                installed.push(path);
            }
            let path = installed.first().cloned().ok_or_else(|| {
                BridgeError::Invalid("No Chromium native-host location is available".into())
            })?;
            self.inner.lock().unwrap().native_host_manifest_path = Some(path.clone());
            self.audit(
                "host.installed",
                "Chromium native messaging host registered".into(),
                None,
                json!({"paths": installed}),
            );
            Ok(path)
        }
    }

    pub fn issue(&self, request: BrowserActionRequest) -> Result<String, BridgeError> {
        let mut inner = self.inner.lock().unwrap();
        if request.kind == "list_tabs" {
            drop(inner);
            return self.send_command(json!({"kind": "list_tabs"}), "list_tabs", "browser");
        }
        if request.kind == "attach" {
            let tab_id = request
                .tab_id
                .ok_or_else(|| BridgeError::Invalid("tabId is required".into()))?;
            let lease_id = Uuid::new_v4().to_string();
            drop(inner);
            return self.send_command(
                json!({"kind": "attach", "tabId": tab_id, "leaseId": lease_id}),
                "attach",
                "browser",
            );
        }
        let lease = inner.lease.clone().ok_or_else(|| {
            BridgeError::Invalid("Attach a tab before issuing browser actions".into())
        })?;
        let user_action = request.actor.as_deref() == Some("user");
        if lease.status != "active" && !(user_action && lease.status == "paused") {
            return Err(BridgeError::Invalid(format!(
                "Tab lease is {}",
                lease.status
            )));
        }
        if request
            .expected_domain
            .as_deref()
            .is_some_and(|domain| domain != lease.domain)
        {
            return Err(BridgeError::Invalid(
                "The page domain changed; attach or approve the new domain before continuing"
                    .into(),
            ));
        }
        if request.kind == "navigate" {
            let destination = request
                .url
                .as_deref()
                .and_then(|url| reqwest::Url::parse(url).ok())
                .and_then(|url| url.host_str().map(str::to_owned));
            if destination.as_deref() != Some(lease.domain.as_str()) {
                return Err(BridgeError::Invalid(
                    "Navigation across the granted domain requires a new tab grant".into(),
                ));
            }
        }
        if lease.permission == "read_only"
            && matches!(
                request.kind.as_str(),
                "click" | "type" | "scroll" | "navigate"
            )
        {
            return Err(BridgeError::Invalid(
                "This lease is read-only; grant Interact before changing the page".into(),
            ));
        }
        let command_id = Uuid::new_v4().to_string();
        let target_name = inner
            .elements
            .iter()
            .find(|element| {
                request
                    .element_id
                    .as_deref()
                    .is_some_and(|id| element.get("id").and_then(Value::as_str) == Some(id))
                    || request.kind == "click_at"
                        && request
                            .x
                            .zip(request.y)
                            .is_some_and(|(x, y)| bounds_contain(element.get("bounds"), x, y))
            })
            .and_then(|element| element.get("name"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let action = json!({
            "kind": request.kind, "elementId": request.element_id, "text": request.text,
            "url": request.url, "x": request.x, "y": request.y, "targetName": target_name
        });
        let sensitive = request.sensitive_kind.or_else(|| {
            sensitive_action(
                action
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                &action,
            )
        });
        if let Some(sensitive_kind) = sensitive {
            let effect = approval_effect(&sensitive_kind, &lease.domain);
            let approval = BrowserApproval {
                id: Uuid::new_v4().to_string(),
                command_id: command_id.clone(),
                action: sensitive_kind.clone(),
                effect,
                domain: lease.domain.clone(),
                created_at: Utc::now().to_rfc3339(),
            };
            inner.pending_approval_command = Some(QueuedCommand {
                id: command_id.clone(),
                action,
                kind: sensitive_kind,
                domain: lease.domain.clone(),
                started_at: Utc::now(),
            });
            inner.pending_approval = Some(approval.clone());
            inner.status = "waiting_for_you".into();
            metric(&mut inner, &lease.domain).approvals += 1;
            self.persist_metrics(&inner);
            drop(inner);
            self.audit(
                "approval.requested",
                approval.effect,
                Some(command_id.clone()),
                json!({"approvalId": approval.id}),
            );
            return Ok(command_id);
        }
        drop(inner);
        self.send_command_with_id(command_id.clone(), action, &request.kind, &lease.domain)?;
        Ok(command_id)
    }

    pub fn set_permission(&self, permission: &str) -> Result<(), BridgeError> {
        if !matches!(permission, "read_only" | "interact") {
            return Err(BridgeError::Invalid(
                "Permission must be read_only or interact".into(),
            ));
        }
        let mut inner = self.inner.lock().unwrap();
        let lease = inner
            .lease
            .as_mut()
            .ok_or_else(|| BridgeError::Invalid("No active lease".into()))?;
        lease.permission = permission.into();
        drop(inner);
        self.audit(
            "lease.permission_changed",
            format!("Tab permission changed to {permission}"),
            None,
            json!({}),
        );
        Ok(())
    }

    pub fn resolve_approval(&self, approval_id: &str, allow: bool) -> Result<(), BridgeError> {
        let mut inner = self.inner.lock().unwrap();
        let approval = inner
            .pending_approval
            .clone()
            .ok_or_else(|| BridgeError::Invalid("No browser approval is pending".into()))?;
        if approval.id != approval_id {
            return Err(BridgeError::Invalid(
                "Browser approval no longer matches".into(),
            ));
        }
        let command = inner.pending_approval_command.take();
        inner.pending_approval = None;
        inner.status = if allow {
            "acting".into()
        } else {
            "paused".into()
        };
        drop(inner);
        self.audit(
            "approval.resolved",
            if allow {
                format!("Approved: {}", approval.effect)
            } else {
                format!("Denied: {}", approval.effect)
            },
            Some(approval.command_id),
            json!({"allowed": allow}),
        );
        if allow {
            if let Some(mut command) = command {
                command.action["approvalGranted"] = Value::Bool(true);
                self.send_queued(command)?;
            }
        }
        Ok(())
    }

    pub fn takeover(&self) -> Result<(), BridgeError> {
        let domain = {
            let mut inner = self.inner.lock().unwrap();
            let transitioned = inner
                .lease
                .as_ref()
                .is_some_and(|lease| lease.status == "active");
            inner.status = "paused".into();
            if let Some(lease) = inner.lease.as_mut() {
                lease.status = "paused".into();
            }
            let domain = inner
                .lease
                .as_ref()
                .map(|lease| lease.domain.clone())
                .unwrap_or_else(|| "browser".into());
            if transitioned {
                metric(&mut inner, &domain).interventions += 1;
            }
            self.persist_metrics(&inner);
            domain
        };
        self.audit(
            "user.takeover",
            "User took control; agent input paused immediately".into(),
            None,
            json!({"domain": domain}),
        );
        Ok(())
    }

    pub fn detach(&self) -> Result<String, BridgeError> {
        self.send_command(
            json!({"kind": "detach", "reason": "bridge_detach"}),
            "detach",
            "browser",
        )
    }

    pub fn configure_remote(&self, config: Option<RemoteBrowserConfig>) -> Result<(), BridgeError> {
        if let Some(value) = config.as_ref() {
            validate_remote_endpoint(&value.endpoint)?;
            if value.bearer_token_env.trim().is_empty() {
                return Err(BridgeError::Invalid("bearerTokenEnv is required".into()));
            }
        }
        self.inner.lock().unwrap().remote_provider = config.clone();
        if let Some(value) = config {
            fs::write(
                &self.remote_config_path,
                serde_json::to_vec_pretty(&value)
                    .map_err(|error| BridgeError::Invalid(error.to_string()))?,
            )?;
        } else if self.remote_config_path.exists() {
            fs::remove_file(&self.remote_config_path)?;
        }
        Ok(())
    }

    pub fn start_remote_session(&self, initial_url: &str) -> Result<Value, BridgeError> {
        let config = self
            .inner
            .lock()
            .unwrap()
            .remote_provider
            .clone()
            .ok_or_else(|| {
                BridgeError::Invalid("Remote browser provider is not configured".into())
            })?;
        if !config.enabled {
            return Err(BridgeError::Invalid(
                "Remote browser provider is disabled".into(),
            ));
        }
        validate_remote_endpoint(&config.endpoint)?;
        let token = std::env::var(&config.bearer_token_env)
            .map_err(|_| BridgeError::Invalid(format!("{} is not set", config.bearer_token_env)))?;
        let client = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| {
                BridgeError::Invalid(format!("Remote browser client failed: {error}"))
            })?;
        let mut response = client
            .post(format!(
                "{}/sessions",
                config.endpoint.trim_end_matches('/')
            ))
            .bearer_auth(token)
            .json(&json!({"initialUrl": initial_url, "source": "bridge"}))
            .send()
            .map_err(|error| {
                BridgeError::Invalid(format!("Remote browser request failed: {error}"))
            })?;
        let status = response.status();
        let mut bytes = Vec::new();
        response.by_ref().take(1_048_577).read_to_end(&mut bytes)?;
        if bytes.len() > 1_048_576 {
            return Err(BridgeError::Invalid(
                "Remote browser response exceeds 1 MiB".into(),
            ));
        }
        let body = serde_json::from_slice::<Value>(&bytes).map_err(|error| {
            BridgeError::Invalid(format!("Remote browser returned invalid JSON: {error}"))
        })?;
        if !status.is_success() {
            return Err(BridgeError::Invalid(format!(
                "Remote browser returned {status}"
            )));
        }
        Ok(body)
    }

    fn send_command(&self, action: Value, kind: &str, domain: &str) -> Result<String, BridgeError> {
        let id = Uuid::new_v4().to_string();
        self.send_command_with_id(id.clone(), action, kind, domain)?;
        Ok(id)
    }

    fn send_command_with_id(
        &self,
        id: String,
        action: Value,
        kind: &str,
        domain: &str,
    ) -> Result<(), BridgeError> {
        self.send_queued(QueuedCommand {
            id,
            action,
            kind: kind.into(),
            domain: domain.into(),
            started_at: Utc::now(),
        })
    }

    fn send_queued(&self, command: QueuedCommand) -> Result<(), BridgeError> {
        let sender = self
            .outbound
            .lock()
            .unwrap()
            .as_ref()
            .map(|(_, sender)| sender.clone())
            .ok_or_else(|| BridgeError::Invalid("Browser extension is not connected".into()))?;
        let message = json!({"id": command.id, "action": command.action});
        sender
            .send(message)
            .map_err(|_| BridgeError::Invalid("Browser extension disconnected".into()))?;
        let mut inner = self.inner.lock().unwrap();
        inner.inflight.insert(command.id.clone(), command.clone());
        if let Some(lease) = inner.lease.as_mut() {
            let now = Utc::now();
            lease.last_activity_at = now.to_rfc3339();
            lease.expires_at = (now + Duration::minutes(LEASE_MINUTES)).to_rfc3339();
        }
        drop(inner);
        self.audit(
            "command.queued",
            format!("Queued browser action {}", command.kind),
            Some(command.id),
            json!({"domain": command.domain}),
        );
        Ok(())
    }

    fn handle_extension_event(&self, message: Value) {
        let event_type = message
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let payload = message.get("payload").cloned().unwrap_or(Value::Null);
        let mut inner = self.inner.lock().unwrap();
        match event_type {
            "paired" => {
                inner.transport_connected = true;
                inner.status = if inner.lease.is_some() {
                    "reading".into()
                } else {
                    "not_attached".into()
                };
            }
            "tab_catalog" => {
                inner.tabs =
                    serde_json::from_value(payload.get("tabs").cloned().unwrap_or(json!([])))
                        .unwrap_or_default()
            }
            "attached" => {
                if let Ok(tab) = serde_json::from_value::<BrowserTab>(
                    payload.get("tab").cloned().unwrap_or(Value::Null),
                ) {
                    let now = Utc::now();
                    let domain = tab.domain.clone().unwrap_or_default();
                    inner
                        .tabs
                        .iter_mut()
                        .for_each(|candidate| candidate.attached = candidate.id == tab.id);
                    inner.lease = Some(TabLease {
                        id: payload
                            .get("leaseId")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .into(),
                        tab_id: tab.id,
                        domain: domain.clone(),
                        status: "active".into(),
                        permission: "read_only".into(),
                        attached_at: now.to_rfc3339(),
                        expires_at: (now + Duration::minutes(LEASE_MINUTES)).to_rfc3339(),
                        last_activity_at: now.to_rfc3339(),
                    });
                    inner.status = "reading".into();
                    inner.capture_active = false;
                    inner.capture_error = None;
                    drop(inner);
                    self.audit(
                        "tab.attached",
                        format!("Attached {} with read-only permission", tab.title),
                        None,
                        json!({"domain": domain}),
                    );
                    return;
                }
            }
            "detached" => {
                inner.lease = None;
                inner.status = "not_attached".into();
                inner.capture_active = false;
                inner.capture_error = None;
                inner.screenshot = None;
                inner.tabs.iter_mut().for_each(|tab| tab.attached = false);
            }
            "navigation" => {
                if let Ok(tab) = serde_json::from_value::<BrowserTab>(
                    payload.get("tab").cloned().unwrap_or(Value::Null),
                ) {
                    if let Some(lease) = inner.lease.as_mut() {
                        if tab.domain.as_deref() != Some(lease.domain.as_str()) {
                            lease.status = "domain_blocked".into();
                            inner.status = "waiting_for_you".into();
                        }
                    }
                }
            }
            "snapshot" => {
                let bytes = payload
                    .get("serializedBytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let tokens = payload
                    .get("estimatedTokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let delta = payload
                    .get("delta")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                inner.token_accounting.snapshots += 1;
                inner.token_accounting.serialized_bytes += bytes;
                inner.token_accounting.estimated_input_tokens += tokens;
                if delta {
                    inner.token_accounting.delta_snapshots += 1
                } else {
                    inner.token_accounting.full_snapshots += 1
                }
                inner.prompt_injection_signals = serde_json::from_value(
                    payload
                        .get("promptInjectionSignals")
                        .cloned()
                        .unwrap_or(json!([])),
                )
                .unwrap_or_default();
                inner.prompt_injection_suspected = !inner.prompt_injection_signals.is_empty()
                    || payload
                        .get("elements")
                        .and_then(Value::as_array)
                        .is_some_and(|elements| {
                            elements.iter().any(|element| {
                                element
                                    .get("promptInjectionSuspected")
                                    .and_then(Value::as_bool)
                                    == Some(true)
                            })
                        });
                if let Some(elements) = payload.get("elements").and_then(Value::as_array) {
                    if !delta {
                        inner.elements = elements.clone();
                    } else {
                        for element in elements {
                            if let Some(id) = element.get("id").and_then(Value::as_str) {
                                if let Some(index) = inner.elements.iter().position(|candidate| {
                                    candidate.get("id").and_then(Value::as_str) == Some(id)
                                }) {
                                    inner.elements[index] = element.clone();
                                } else {
                                    inner.elements.push(element.clone());
                                }
                            }
                        }
                    }
                }
                if delta {
                    let removed = payload
                        .get("removedIds")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect::<std::collections::HashSet<_>>();
                    inner.elements.retain(|element| {
                        element
                            .get("id")
                            .and_then(Value::as_str)
                            .is_none_or(|id| !removed.contains(id))
                    });
                }
                inner.viewport = payload.get("viewport").cloned().unwrap_or(Value::Null);
                inner.status = if inner.prompt_injection_suspected {
                    "waiting_for_you".into()
                } else {
                    "reading".into()
                };
                if let Some(domain) = inner.lease.as_ref().map(|lease| lease.domain.clone()) {
                    metric(&mut inner, &domain).input_tokens += tokens;
                }
            }
            "screenshot" => {
                inner.screenshot = payload
                    .get("dataUrl")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                inner.screenshot_redacted_regions = payload
                    .get("redactedRegions")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                inner.token_accounting.screenshot_count += 1;
                if let Some(domain) = inner.lease.as_ref().map(|lease| lease.domain.clone()) {
                    metric(&mut inner, &domain).screenshots += 1;
                }
            }
            "frame" => {
                inner.capture_active = true;
                inner.capture_error = None;
                inner.screenshot = payload
                    .get("dataUrl")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                inner.screenshot_redacted_regions = payload
                    .get("redactedRegions")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
            }
            "capture_status" => {
                let (active, error) = capture_status(&payload);
                inner.capture_active = active;
                inner.capture_error = error;
            }
            "debug_event" => {
                push_bounded(&mut inner.debug_events, payload, DEBUG_LIMIT);
            }
            "command_result" => {
                let id = payload
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let ok = payload.get("ok").and_then(Value::as_bool).unwrap_or(false);
                let replayed = payload
                    .get("replayed")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if let Some(command) = inner.inflight.remove(&id) {
                    let elapsed =
                        (Utc::now() - command.started_at).num_milliseconds().max(0) as u64;
                    let site = metric(&mut inner, &command.domain);
                    site.actions += 1;
                    site.total_latency_ms += elapsed;
                    if ok {
                        site.successes += 1
                    } else {
                        site.failures += 1
                    }
                    if replayed && sensitive_action(&command.kind, &command.action).is_some() {
                        site.duplicate_side_effects += 0;
                    }
                    inner.completed.insert(id.clone(), ok);
                    inner.status = if ok {
                        "reading".into()
                    } else {
                        "paused".into()
                    };
                    self.persist_metrics(&inner);
                    drop(inner);
                    self.audit(
                        "command.completed",
                        format!(
                            "{} {}",
                            command.kind,
                            if ok { "completed" } else { "failed" }
                        ),
                        Some(id),
                        payload,
                    );
                    return;
                }
            }
            _ => {}
        }
    }

    fn audit(&self, kind: &str, summary: String, command_id: Option<String>, data: Value) {
        let domain = self
            .inner
            .lock()
            .unwrap()
            .lease
            .as_ref()
            .map(|lease| lease.domain.clone());
        let mut inner = self.inner.lock().unwrap();
        push_bounded(
            &mut inner.audit,
            BrowserAuditEvent {
                id: Uuid::new_v4().to_string(),
                kind: kind.into(),
                summary,
                command_id,
                domain,
                created_at: Utc::now().to_rfc3339(),
                data,
            },
            AUDIT_LIMIT,
        );
        persist_json(&self.audit_path, &inner.audit);
    }

    fn persist_metrics(&self, inner: &Inner) {
        let mut values = inner.site_metrics.values().collect::<Vec<_>>();
        values.sort_by(|a, b| a.domain.cmp(&b.domain));
        persist_json(&self.metrics_path, &values);
    }
}

pub fn route_browser(request: BrowserRouteRequest) -> BrowserRouteDecision {
    if request.structured_api_available {
        return decision(
            "mcp_api",
            "A reliable structured operation is available",
            false,
        );
    }
    if request.needs_geo_or_proxy
        || request.unattended
        || (request.needs_parallelism && request.remote_provider_configured)
    {
        return decision(
            "remote_browser",
            "The task needs remote availability, scale, or network location",
            false,
        );
    }
    if request.needs_user_auth {
        return decision(
            "attached_tab",
            "The task needs the user's authenticated browser state",
            true,
        );
    }
    if request.needs_isolation
        || request.needs_parallelism
        || matches!(
            request.task_class.as_deref(),
            Some("automated_test" | "untrusted_site" | "isolated_qa" | "parallel_qa")
        )
    {
        return decision(
            "local_headless",
            "Isolation, reproducibility, or parallel QA takes priority",
            false,
        );
    }
    if request.dom_control_available {
        return decision(
            "attached_tab",
            "Semantic DOM control is available and keeps the work visible",
            true,
        );
    }
    decision(
        "computer_use",
        "No structured or DOM control path is available",
        true,
    )
}

pub fn bundled_skills() -> Vec<BrowserSkill> {
    [
        include_str!("../../browser-skills/github.json"),
        include_str!("../../browser-skills/google-workspace.json"),
        include_str!("../../browser-skills/notion.json"),
    ]
    .iter()
    .filter_map(|source| serde_json::from_str(source).ok())
    .collect()
}

fn decision(route: &str, reason: &str, requires_user_grant: bool) -> BrowserRouteDecision {
    BrowserRouteDecision {
        route: route.into(),
        reason: reason.into(),
        requires_user_grant,
    }
}

fn metric<'a>(inner: &'a mut Inner, domain: &str) -> &'a mut SiteMetric {
    inner
        .site_metrics
        .entry(domain.into())
        .or_insert_with(|| SiteMetric {
            domain: domain.into(),
            actions: 0,
            successes: 0,
            failures: 0,
            total_latency_ms: 0,
            input_tokens: 0,
            screenshots: 0,
            interventions: 0,
            approvals: 0,
            duplicate_side_effects: 0,
        })
}

fn push_bounded<T>(queue: &mut VecDeque<T>, value: T, limit: usize) {
    queue.push_back(value);
    while queue.len() > limit {
        queue.pop_front();
    }
}

fn persist_json(path: &Path, value: &impl Serialize) {
    let Ok(bytes) = serde_json::to_vec_pretty(value) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let temporary = path.with_extension("json.tmp");
    if fs::write(&temporary, bytes).is_ok() {
        let _ = fs::rename(temporary, path);
    }
}

fn sensitive_action(kind: &str, action: &Value) -> Option<String> {
    let combined = format!(
        "{} {}",
        kind,
        action
            .get("targetName")
            .and_then(Value::as_str)
            .unwrap_or_default()
    )
    .to_lowercase();
    [
        ("submit", "submit"),
        ("send", "send"),
        ("delete", "delete"),
        ("purchase", "purchase"),
        ("buy", "purchase"),
        ("pay", "purchase"),
        ("publish", "publish"),
        ("share", "publish"),
        ("merge", "submit"),
        ("credential", "credential"),
        ("payment", "payment"),
    ]
    .into_iter()
    .find(|(pattern, _)| combined.contains(pattern))
    .map(|(_, effect)| effect.to_owned())
}

fn bounds_contain(bounds: Option<&Value>, x: f64, y: f64) -> bool {
    let Some(bounds) = bounds else { return false };
    let left = bounds.get("x").and_then(Value::as_f64).unwrap_or(f64::MAX);
    let top = bounds.get("y").and_then(Value::as_f64).unwrap_or(f64::MAX);
    let width = bounds.get("width").and_then(Value::as_f64).unwrap_or(0.0);
    let height = bounds.get("height").and_then(Value::as_f64).unwrap_or(0.0);
    x >= left && x <= left + width && y >= top && y <= top + height
}

fn approval_effect(kind: &str, domain: &str) -> String {
    match kind {
        "send" => format!("Send information from {domain} to another person or service"),
        "delete" => format!("Delete data on {domain}"),
        "purchase" | "payment" => format!("Commit a purchase or payment on {domain}"),
        "publish" => format!("Publish content publicly from {domain}"),
        "credential" => format!("Change credentials on {domain}"),
        _ => format!("Submit a form with an external effect on {domain}"),
    }
}

fn capture_status(payload: &Value) -> (bool, Option<String>) {
    let active = payload
        .get("active")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let error = payload
        .get("error")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned);
    (active, error)
}

fn validate_remote_endpoint(endpoint: &str) -> Result<(), BridgeError> {
    let url = reqwest::Url::parse(endpoint)
        .map_err(|_| BridgeError::Invalid("Remote browser endpoint must be a valid URL".into()))?;
    if url.scheme() != "https" {
        return Err(BridgeError::Invalid(
            "Remote browser endpoint must use HTTPS".into(),
        ));
    }
    let host = url.host_str().unwrap_or_default();
    if host == "localhost"
        || host.ends_with(".local")
        || host.parse::<std::net::IpAddr>().is_ok_and(is_local_ip)
    {
        return Err(BridgeError::Invalid(
            "Remote browser endpoint may not target a local or private address".into(),
        ));
    }
    Ok(())
}

fn is_local_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        std::net::IpAddr::V6(ip) => {
            ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> BrowserRouteRequest {
        BrowserRouteRequest {
            structured_api_available: false,
            needs_user_auth: false,
            needs_isolation: false,
            needs_parallelism: false,
            needs_geo_or_proxy: false,
            unattended: false,
            dom_control_available: true,
            remote_provider_configured: false,
            task_class: None,
        }
    }

    #[test]
    fn router_uses_specified_priority_order() {
        let mut value = request();
        value.structured_api_available = true;
        value.needs_user_auth = true;
        assert_eq!(route_browser(value).route, "mcp_api");
        let mut value = request();
        value.needs_user_auth = true;
        value.needs_isolation = true;
        assert_eq!(route_browser(value).route, "attached_tab");
        let mut value = request();
        value.needs_user_auth = true;
        value.needs_geo_or_proxy = true;
        assert_eq!(route_browser(value).route, "remote_browser");
        let mut value = request();
        value.needs_geo_or_proxy = true;
        assert_eq!(route_browser(value).route, "remote_browser");
        let mut value = request();
        value.needs_isolation = true;
        assert_eq!(route_browser(value).route, "local_headless");
        let mut value = request();
        value.dom_control_available = false;
        assert_eq!(route_browser(value).route, "computer_use");
    }

    #[test]
    fn sensitive_effects_are_gated() {
        assert_eq!(
            sensitive_action("click", &json!({"targetName": "Delete account"})),
            Some("delete".into())
        );
        assert_eq!(sensitive_action("scroll", &json!({})), None);
    }

    #[test]
    fn remote_provider_rejects_unsafe_endpoints() {
        assert!(validate_remote_endpoint("http://browser.example.com").is_err());
        assert!(validate_remote_endpoint("https://127.0.0.1").is_err());
        assert!(validate_remote_endpoint("https://browser.example.com").is_ok());
    }

    #[test]
    fn common_site_skills_are_bundled() {
        let skills = bundled_skills();
        assert_eq!(skills.len(), 3);
        assert!(skills.iter().all(|skill| !skill.steps.is_empty()));
    }

    #[test]
    fn capture_failure_is_preserved_for_the_browser_surface() {
        assert_eq!(
            capture_status(&json!({"active": false, "error": "activeTab permission required"})),
            (false, Some("activeTab permission required".into()))
        );
        assert_eq!(capture_status(&json!({"active": true})), (true, None));
    }
}
