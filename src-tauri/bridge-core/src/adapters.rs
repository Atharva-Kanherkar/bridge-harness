use crate::{
    agent, binary, claude_adapter, codex_adapter,
    delegation::WriteMode,
    model::{AdapterDescriptor, CapabilityTier, ModelOption},
    opencode_adapter,
    worker_sandbox::ReadOnlySandbox,
    BridgeError,
};
use serde_json::Value;
use std::{
    any::Any,
    collections::HashMap,
    io::BufRead,
    process::{Command, Stdio},
    sync::{Arc, Mutex, RwLock},
    thread,
    time::Duration,
};

pub trait AdapterRuntime: Send {
    fn process_id(&self) -> u32;
    fn provider_session_id(&self) -> &str;
    fn current_turn(&self) -> Arc<Mutex<Option<String>>>;
    fn send_turn(&self, text: &str) -> Result<(), BridgeError>;
    /// Send a user turn with trusted, application-owned context that must not
    /// be folded into the visible user message. Providers that cannot attach
    /// per-turn context retain their startup instructions and send normally.
    fn send_turn_with_context(
        &self,
        text: &str,
        _application_context: &str,
    ) -> Result<(), BridgeError> {
        self.send_turn(text)
    }
    fn interrupt(&self) -> Result<(), BridgeError>;
    fn respond(&self, request_id: Value, decision: &str) -> Result<(), BridgeError>;
    /// Ask the provider to report current subscription rate-limit usage.
    /// The response arrives asynchronously on the session's event stream.
    /// Providers without an on-demand usage query keep the default no-op.
    fn read_usage(&self) -> Result<(), BridgeError> {
        Ok(())
    }
    fn stop(&mut self, reason: ShutdownReason);
}

#[cfg(unix)]
pub fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(unix))]
pub fn configure_process_group(_command: &mut Command) {}

pub fn process_identity(pid: u32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "lstart=", "-o", "comm="])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let identity = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !identity.is_empty()).then_some(identity)
}

fn process_group_is_running(pid: u32) -> bool {
    let output = Command::new("ps")
        .args(["-ax", "-o", "pgid=", "-o", "stat="])
        .stderr(Stdio::null())
        .output();
    let Ok(output) = output else {
        return false;
    };
    output.status.success()
        && String::from_utf8_lossy(&output.stdout).lines().any(|line| {
            let mut fields = line.split_whitespace();
            fields.next().and_then(|value| value.parse::<u32>().ok()) == Some(pid)
                && fields.next().is_some_and(|state| !state.starts_with('Z'))
        })
}

#[cfg(unix)]
pub fn terminate_process_group(pid: u32) -> bool {
    let target = format!("-{pid}");
    let signal = |value: &str| {
        Command::new("kill")
            .args([value, &target])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    };
    let alive = || process_group_is_running(pid);
    let _ = signal("-TERM");
    for _ in 0..20 {
        if !alive() {
            return true;
        }
        thread::sleep(Duration::from_millis(25));
    }
    let _ = signal("-KILL");
    for _ in 0..20 {
        if !alive() {
            return true;
        }
        thread::sleep(Duration::from_millis(25));
    }
    !alive()
}

#[cfg(not(unix))]
pub fn terminate_process_group(pid: u32) -> bool {
    Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownReason {
    UserStopped,
    UserCancelled,
    Replaced,
    Completed,
    Failed,
    AppShutdown,
}

impl ShutdownReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserStopped => "user_stopped",
            Self::UserCancelled => "user_cancelled",
            Self::Replaced => "replaced",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::AppShutdown => "app_shutdown",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StartRequest<'a> {
    pub cwd: &'a str,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub instructions: Option<&'a str>,
    pub write_mode: Option<WriteMode>,
    pub read_only_sandbox: Option<&'a ReadOnlySandbox>,
}

#[derive(Debug, Clone, Copy)]
pub struct ResumeRequest<'a> {
    pub provider_session_id: &'a str,
    pub cwd: &'a str,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub instructions: Option<&'a str>,
    pub write_mode: Option<WriteMode>,
    pub read_only_sandbox: Option<&'a ReadOnlySandbox>,
}

pub struct StartedAdapter {
    pub runtime: Box<dyn AdapterRuntime>,
    pub reader: Box<dyn BufRead + Send>,
    pub startup_messages: Vec<Value>,
}

pub trait HarnessAdapter: Send + Sync + Any {
    fn as_any(&self) -> &dyn Any;
    fn descriptor(&self) -> AdapterDescriptor;
    fn start(&self, request: StartRequest<'_>) -> Result<StartedAdapter, BridgeError>;
    fn resume(&self, request: ResumeRequest<'_>) -> Result<StartedAdapter, BridgeError>;
    fn supports_native_resume(&self) -> bool;
    fn normalize(&self, value: &Value) -> Vec<agent::NormalizedEvent>;
}

pub struct AdapterRegistry {
    adapters: HashMap<String, Box<dyn HarnessAdapter>>,
}

fn model_options(items: &[(&str, &str, CapabilityTier, bool)]) -> Vec<ModelOption> {
    items
        .iter()
        .map(|(id, label, tier, default_for_tier)| ModelOption {
            id: (*id).into(),
            label: (*label).into(),
            tier: *tier,
            default_for_tier: *default_for_tier,
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelResolution {
    pub requested_tier: CapabilityTier,
    pub actual_model: String,
    pub warning: Option<String>,
}

impl AdapterRegistry {
    /// A registry with no adapters and no background discovery. For hosts and
    /// tests that need a `BridgeCore` without spawning provider processes.
    pub fn empty() -> Self {
        Self { adapters: HashMap::new() }
    }

    pub fn built_in() -> Result<Self, BridgeError> {
        Self::built_in_with_opencode(opencode_adapter::OpenCodeSettings::default())
    }

    pub fn built_in_with_opencode(
        opencode_settings: opencode_adapter::OpenCodeSettings,
    ) -> Result<Self, BridgeError> {
        Self::built_in_with_opencode_notify(opencode_settings, None)
    }

    /// `on_opencode_discovered` fires once the background OpenCode catalog
    /// discovery finishes (successfully or not), so the host can tell the
    /// frontend to re-read adapter availability.
    pub fn built_in_with_opencode_notify(
        opencode_settings: opencode_adapter::OpenCodeSettings,
        on_opencode_discovered: Option<Box<dyn FnOnce() + Send>>,
    ) -> Result<Self, BridgeError> {
        let mut registry = Self {
            adapters: HashMap::new(),
        };
        registry.register(Box::new(CodexAdapter))?;
        registry.register(Box::new(ClaudeAdapter {
            streams: Mutex::new(HashMap::new()),
        }))?;
        registry.register(Box::new(OpenCodeAdapter::new(
            opencode_settings,
            on_opencode_discovered,
        )))?;
        Ok(registry)
    }

    pub fn register(&mut self, adapter: Box<dyn HarnessAdapter>) -> Result<(), BridgeError> {
        let descriptor = adapter.descriptor();
        if descriptor.id.trim().is_empty() {
            return Err(BridgeError::Invalid("Adapter id cannot be empty".into()));
        }
        if self.adapters.contains_key(&descriptor.id) {
            return Err(BridgeError::Invalid(format!(
                "Duplicate adapter id: {}",
                descriptor.id
            )));
        }
        self.adapters.insert(descriptor.id, adapter);
        Ok(())
    }

    pub fn descriptors(&self) -> Vec<AdapterDescriptor> {
        let mut descriptors: Vec<_> = self
            .adapters
            .values()
            .map(|adapter| adapter.descriptor())
            .collect();
        descriptors.sort_by(|a, b| a.id.cmp(&b.id));
        descriptors
    }

    pub fn start(
        &self,
        id: &str,
        request: StartRequest<'_>,
    ) -> Result<StartedAdapter, BridgeError> {
        let adapter = self.adapters.get(id).ok_or_else(|| {
            BridgeError::Invalid(format!("No structured adapter is registered for {id}"))
        })?;
        let descriptor = adapter.descriptor();
        if !descriptor.available {
            return Err(BridgeError::Invalid(
                descriptor
                    .unavailable_reason
                    .unwrap_or_else(|| format!("{} is unavailable", descriptor.label)),
            ));
        }
        adapter.start(request)
    }

    pub fn resume(
        &self,
        id: &str,
        request: ResumeRequest<'_>,
    ) -> Result<StartedAdapter, BridgeError> {
        let adapter = self.adapters.get(id).ok_or_else(|| {
            BridgeError::Invalid(format!("No structured adapter is registered for {id}"))
        })?;
        if !adapter.supports_native_resume() {
            return Err(BridgeError::Invalid(format!(
                "Adapter {id} does not support native resume"
            )));
        }
        adapter.resume(request)
    }

    pub fn supports_native_resume(&self, id: &str) -> bool {
        self.adapters
            .get(id)
            .is_some_and(|adapter| adapter.supports_native_resume())
    }

    pub fn normalize(&self, id: &str, value: &Value) -> Vec<agent::NormalizedEvent> {
        self.adapters
            .get(id)
            .map(|adapter| adapter.normalize(value))
            .unwrap_or_default()
    }

    pub fn refresh_opencode(
        &self,
        settings: opencode_adapter::OpenCodeSettings,
        directory: &str,
    ) -> Result<opencode_adapter::OpenCodeCatalog, BridgeError> {
        self.opencode_adapter()?.refresh(settings, directory)
    }

    pub fn opencode_settings(&self) -> Result<opencode_adapter::OpenCodeSettings, BridgeError> {
        Ok(self.opencode_adapter()?.settings())
    }

    pub fn set_opencode_provider_api_key(
        &self,
        directory: &str,
        provider_id: &str,
        api_key: &str,
    ) -> Result<opencode_adapter::OpenCodeCatalog, BridgeError> {
        let adapter = self.opencode_adapter()?;
        let catalog = opencode_adapter::set_provider_api_key(
            &adapter.settings(),
            directory,
            provider_id,
            api_key,
        )?;
        adapter.replace_catalog(catalog.clone());
        Ok(catalog)
    }

    pub fn remove_opencode_provider_auth(
        &self,
        directory: &str,
        provider_id: &str,
    ) -> Result<opencode_adapter::OpenCodeCatalog, BridgeError> {
        let adapter = self.opencode_adapter()?;
        let catalog =
            opencode_adapter::remove_provider_auth(&adapter.settings(), directory, provider_id)?;
        adapter.replace_catalog(catalog.clone());
        Ok(catalog)
    }

    fn opencode_adapter(&self) -> Result<&OpenCodeAdapter, BridgeError> {
        self.adapters
            .get("opencode")
            .and_then(|adapter| adapter.as_any().downcast_ref::<OpenCodeAdapter>())
            .ok_or_else(|| BridgeError::Invalid("OpenCode adapter is not registered".into()))
    }

    pub fn resolve_model(
        &self,
        id: &str,
        tier: CapabilityTier,
        model_hint: Option<&str>,
    ) -> Result<ModelResolution, BridgeError> {
        let descriptor = self
            .adapters
            .get(id)
            .ok_or_else(|| {
                BridgeError::Invalid(format!("No structured adapter is registered for {id}"))
            })?
            .descriptor();
        let tier_default = descriptor
            .models
            .iter()
            .find(|model| model.tier == tier && model.default_for_tier)
            .or_else(|| descriptor.models.iter().find(|model| model.tier == tier))
            .ok_or_else(|| {
                BridgeError::Invalid(format!(
                    "Adapter {id} does not advertise a {} capability model",
                    tier.as_str()
                ))
            })?;
        let hinted = model_hint.and_then(|hint| {
            descriptor
                .models
                .iter()
                .find(|model| model.id.eq_ignore_ascii_case(hint.trim()))
        });
        let selected = hinted
            .filter(|model| model.tier == tier)
            .unwrap_or(tier_default);
        let warning = model_hint.and_then(|hint| {
            (hinted.is_none() || hinted.is_some_and(|model| model.tier != tier)).then(|| {
                format!(
                    "Model hint {hint:?} is unknown or outside tier {}; using {}",
                    tier.as_str(),
                    tier_default.id
                )
            })
        });
        Ok(ModelResolution {
            requested_tier: tier,
            actual_model: selected.id.clone(),
            warning,
        })
    }
}

struct OpenCodeAdapter {
    streams: Mutex<HashMap<String, agent::OpenCodeStreamState>>,
    settings: RwLock<opencode_adapter::OpenCodeSettings>,
    catalog: Arc<RwLock<Option<opencode_adapter::OpenCodeCatalog>>>,
    catalog_error: Arc<RwLock<Option<String>>>,
}
impl OpenCodeAdapter {
    fn new(
        settings: opencode_adapter::OpenCodeSettings,
        on_discovered: Option<Box<dyn FnOnce() + Send>>,
    ) -> Self {
        let adapter = Self {
            streams: Mutex::new(HashMap::new()),
            settings: RwLock::new(settings.clone()),
            catalog: Arc::new(RwLock::new(None)),
            catalog_error: Arc::new(RwLock::new(None)),
        };
        // Discovery spawns an OpenCode server and can take tens of seconds, and
        // new() runs during app setup — do the initial catalog load off-thread.
        let catalog = adapter.catalog.clone();
        let catalog_error = adapter.catalog_error.clone();
        let directory = std::env::current_dir()
            .ok()
            .and_then(|path| path.to_str().map(str::to_owned))
            .unwrap_or_else(|| ".".into());
        let _ = std::thread::Builder::new()
            .name("opencode-discover".into())
            .spawn(move || {
                match opencode_adapter::discover(&settings, &directory) {
                    Ok(result) => {
                        *catalog.write().unwrap() = Some(result);
                        *catalog_error.write().unwrap() = None;
                    }
                    Err(error) => {
                        *catalog_error.write().unwrap() = Some(error.to_string());
                    }
                }
                if let Some(notify) = on_discovered {
                    notify();
                }
            });
        adapter
    }

    fn refresh(
        &self,
        settings: opencode_adapter::OpenCodeSettings,
        directory: &str,
    ) -> Result<opencode_adapter::OpenCodeCatalog, BridgeError> {
        *self.settings.write().unwrap() = settings.clone();
        match opencode_adapter::discover(&settings, directory) {
            Ok(catalog) => {
                *self.catalog.write().unwrap() = Some(catalog.clone());
                *self.catalog_error.write().unwrap() = None;
                Ok(catalog)
            }
            Err(error) => {
                // Keep the last known-good catalog so a transient discovery
                // failure does not degrade a working setup; the error is
                // surfaced alongside it.
                *self.catalog_error.write().unwrap() = Some(error.to_string());
                Err(error)
            }
        }
    }

    fn settings(&self) -> opencode_adapter::OpenCodeSettings {
        self.settings.read().unwrap().clone()
    }

    fn ensure_model_is_selectable(&self, model: Option<&str>) -> Result<(), BridgeError> {
        let Some(model) = model else {
            return Ok(());
        };
        let selectable = self
            .descriptor()
            .models
            .into_iter()
            .any(|option| option.id == model);
        selectable.then_some(()).ok_or_else(|| {
            BridgeError::Invalid(format!(
                "OpenCode model {model:?} is not exposed by a connected provider or is hidden"
            ))
        })
    }

    fn replace_catalog(&self, catalog: opencode_adapter::OpenCodeCatalog) {
        *self.catalog.write().unwrap() = Some(catalog);
        *self.catalog_error.write().unwrap() = None;
    }
}
impl HarnessAdapter for OpenCodeAdapter {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn descriptor(&self) -> AdapterDescriptor {
        let catalog = self.catalog.read().unwrap().clone();
        let models = catalog
            .as_ref()
            .map(|catalog| {
                opencode_adapter::model_options(
                    catalog,
                    &self.settings.read().unwrap().visible_models,
                )
            })
            .unwrap_or_default();
        let default_model = models
            .iter()
            .find(|model| model.tier == CapabilityTier::Standard && model.default_for_tier)
            .or_else(|| models.iter().find(|model| model.default_for_tier))
            .map(|model| model.id.clone());
        let available = catalog.is_some() && !models.is_empty();
        let unavailable_reason = if catalog.is_some() && models.is_empty() {
            Some("OpenCode has no connected provider models selected".into())
        } else {
            self.catalog_error.read().unwrap().clone()
        };
        AdapterDescriptor {
            id: "opencode".into(),
            label: "OpenCode".into(),
            available,
            version: catalog.as_ref().map(|catalog| catalog.version.clone()),
            capabilities: [
                "messages",
                "streaming",
                "reasoning",
                "plans",
                "tools",
                "commands",
                "file_changes",
                "approvals",
                "usage",
                "history",
                "interrupt",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            unavailable_reason,
            models,
            default_model,
        }
    }
    fn start(&self, request: StartRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        self.ensure_model_is_selectable(request.model)?;
        let settings = self.settings();
        let started = opencode_adapter::start_with_settings(request, &settings)?;
        Ok(StartedAdapter {
            runtime: Box::new(started.runtime),
            reader: Box::new(started.reader),
            startup_messages: started.startup_messages,
        })
    }
    fn resume(&self, request: ResumeRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        self.ensure_model_is_selectable(request.model)?;
        let settings = self.settings();
        let started = opencode_adapter::resume_with_settings(request, &settings)?;
        Ok(StartedAdapter {
            runtime: Box::new(started.runtime),
            reader: Box::new(started.reader),
            startup_messages: started.startup_messages,
        })
    }
    fn supports_native_resume(&self) -> bool {
        self.catalog.read().unwrap().is_some()
    }
    fn normalize(&self, value: &Value) -> Vec<agent::NormalizedEvent> {
        let session_key = value
            .pointer("/properties/sessionID")
            .and_then(Value::as_str)
            .unwrap_or("default")
            .to_owned();
        let mut streams = self.streams.lock().unwrap();
        let state = streams.entry(session_key).or_default();
        agent::normalize_opencode_message_with_state(value, state)
    }
}

struct CodexAdapter;
impl HarnessAdapter for CodexAdapter {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn descriptor(&self) -> AdapterDescriptor {
        let version = codex_adapter::binary_version();
        AdapterDescriptor {
            id: "codex".into(),
            label: "Codex".into(),
            available: version.is_some(),
            version,
            capabilities: [
                "messages",
                "streaming",
                "reasoning",
                "plans",
                "tools",
                "commands",
                "file_changes",
                "approvals",
                "usage",
                "history",
                "interrupt",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            unavailable_reason: binary::resolve("codex")
                .is_none()
                .then(|| "Codex binary is not installed".into()),
            models: model_options(&[
                ("gpt-5.6-luna", "GPT Luna", CapabilityTier::Fast, true),
                ("gpt-5.6-terra", "GPT Terra", CapabilityTier::Standard, true),
                ("gpt-5.6-sol", "GPT Sol", CapabilityTier::Strong, true),
                (
                    "gpt-5.3-codex",
                    "GPT-5.3 Codex",
                    CapabilityTier::Standard,
                    false,
                ),
            ]),
            default_model: Some("gpt-5.6-luna".into()),
        }
    }
    fn start(&self, request: StartRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        let started = codex_adapter::start(request)?;
        Ok(StartedAdapter {
            runtime: Box::new(started.runtime),
            reader: Box::new(started.reader),
            startup_messages: started.startup_messages,
        })
    }
    fn resume(&self, request: ResumeRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        let started = codex_adapter::resume(request)?;
        Ok(StartedAdapter {
            runtime: Box::new(started.runtime),
            reader: Box::new(started.reader),
            startup_messages: started.startup_messages,
        })
    }
    fn supports_native_resume(&self) -> bool {
        codex_adapter::supports_native_resume()
    }
    fn normalize(&self, value: &Value) -> Vec<agent::NormalizedEvent> {
        if value.get("id").is_some() && value.get("method").is_some() {
            agent::normalize_codex_request(value).into_iter().collect()
        } else {
            agent::normalize_codex_message(value)
        }
    }
}

struct ClaudeAdapter {
    streams: Mutex<HashMap<String, agent::ClaudeStreamState>>,
}
impl HarnessAdapter for ClaudeAdapter {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn descriptor(&self) -> AdapterDescriptor {
        let version = claude_adapter::binary_version();
        AdapterDescriptor {
            id: "claude".into(),
            label: "Claude Code".into(),
            available: version.is_some(),
            version,
            capabilities: [
                "messages",
                "streaming",
                "reasoning",
                "tools",
                "commands",
                "approvals",
                "usage",
                "interrupt",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            unavailable_reason: claude_adapter::unavailable_reason(),
            models: model_options(&[
                ("haiku", "Claude Haiku", CapabilityTier::Fast, true),
                ("sonnet", "Claude Sonnet", CapabilityTier::Standard, true),
                ("opus", "Claude Opus", CapabilityTier::Strong, false),
                ("fable", "Claude Fable", CapabilityTier::Strong, true),
            ]),
            default_model: Some("sonnet".into()),
        }
    }
    fn start(&self, request: StartRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        let started = claude_adapter::start(request)?;
        Ok(StartedAdapter {
            runtime: Box::new(started.runtime),
            reader: Box::new(started.reader),
            startup_messages: started.startup_messages,
        })
    }
    fn resume(&self, request: ResumeRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        let started = claude_adapter::resume(request)?;
        Ok(StartedAdapter {
            runtime: Box::new(started.runtime),
            reader: Box::new(started.reader),
            startup_messages: started.startup_messages,
        })
    }
    fn supports_native_resume(&self) -> bool {
        claude_adapter::supports_native_resume()
    }
    fn normalize(&self, value: &Value) -> Vec<agent::NormalizedEvent> {
        let session_key = value
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or("default")
            .to_owned();
        let mut streams = self.streams.lock().unwrap();
        let state = streams.entry(session_key).or_default();
        agent::normalize_claude_message_with_state(value, state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fake;
    impl HarnessAdapter for Fake {
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn descriptor(&self) -> AdapterDescriptor {
            AdapterDescriptor {
                id: "fake".into(),
                label: "Fake".into(),
                available: true,
                version: Some("1".into()),
                capabilities: vec!["messages".into()],
                unavailable_reason: None,
                models: vec![],
                default_model: None,
            }
        }
        fn start(&self, _request: StartRequest<'_>) -> Result<StartedAdapter, BridgeError> {
            Err(BridgeError::Invalid("not launched in registry test".into()))
        }
        fn resume(&self, _request: ResumeRequest<'_>) -> Result<StartedAdapter, BridgeError> {
            Err(BridgeError::Invalid("not resumed in registry test".into()))
        }
        fn supports_native_resume(&self) -> bool {
            false
        }
        fn normalize(&self, _value: &Value) -> Vec<agent::NormalizedEvent> {
            vec![]
        }
    }
    #[test]
    fn rejects_duplicate_ids() {
        let mut registry = AdapterRegistry {
            adapters: HashMap::new(),
        };
        registry.register(Box::new(Fake)).unwrap();
        let error = registry.register(Box::new(Fake)).unwrap_err();
        assert!(error.to_string().contains("Duplicate adapter id"));
    }
    #[test]
    fn capabilities_are_discovered_through_registry() {
        let mut registry = AdapterRegistry {
            adapters: HashMap::new(),
        };
        registry.register(Box::new(Fake)).unwrap();
        assert_eq!(registry.descriptors()[0].capabilities, vec!["messages"]);
    }

    #[test]
    fn every_advertised_model_has_one_tier_and_each_populated_tier_has_one_default() {
        let registry = AdapterRegistry::built_in().unwrap();
        for descriptor in registry.descriptors() {
            if !descriptor.available {
                continue;
            }
            assert!(!descriptor.models.is_empty());
            for tier in [
                CapabilityTier::Fast,
                CapabilityTier::Standard,
                CapabilityTier::Strong,
            ] {
                let models = descriptor
                    .models
                    .iter()
                    .filter(|model| model.tier == tier)
                    .collect::<Vec<_>>();
                if models.is_empty() {
                    continue;
                }
                assert_eq!(
                    models.iter().filter(|model| model.default_for_tier).count(),
                    1,
                    "{} must have exactly one {} default",
                    descriptor.id,
                    tier.as_str()
                );
            }
        }
    }

    #[test]
    fn tier_resolution_is_deterministic_and_falls_back_safely() {
        let registry = AdapterRegistry::built_in().unwrap();
        let default = registry
            .resolve_model("claude", CapabilityTier::Strong, None)
            .unwrap();
        assert_eq!(default.actual_model, "fable");
        assert!(default.warning.is_none());

        let known = registry
            .resolve_model("claude", CapabilityTier::Strong, Some("opus"))
            .unwrap();
        assert_eq!(known.actual_model, "opus");
        assert!(known.warning.is_none());

        for hint in ["not-installed", "haiku"] {
            let fallback = registry
                .resolve_model("claude", CapabilityTier::Strong, Some(hint))
                .unwrap();
            assert_eq!(fallback.actual_model, "fable");
            assert!(fallback
                .warning
                .as_deref()
                .is_some_and(|text| text.contains(hint)));
        }
    }
}
