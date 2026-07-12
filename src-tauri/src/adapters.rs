use crate::{
    agent, binary, claude_adapter, codex_adapter, orchestrator,
    model::{AdapterDescriptor, ModelOption},
    BridgeError,
};
use serde_json::Value;
use std::{
    collections::HashMap,
    io::BufRead,
    sync::{Arc, Mutex},
};

pub trait AdapterRuntime: Send {
    fn provider_session_id(&self) -> &str;
    fn current_turn(&self) -> Arc<Mutex<Option<String>>>;
    fn send_turn(&self, text: &str) -> Result<(), BridgeError>;
    fn interrupt(&self) -> Result<(), BridgeError>;
    fn respond(&self, request_id: Value, decision: &str) -> Result<(), BridgeError>;
    fn stop(&mut self);
}

pub struct StartedAdapter {
    pub runtime: Box<dyn AdapterRuntime>,
    pub reader: Box<dyn BufRead + Send>,
    pub startup_messages: Vec<Value>,
}

pub trait HarnessAdapter: Send + Sync {
    fn descriptor(&self) -> AdapterDescriptor;
    fn start(
        &self,
        cwd: &str,
        model: Option<&str>,
        effort: Option<&str>,
        instructions: Option<&str>,
    ) -> Result<StartedAdapter, BridgeError>;
    fn normalize(&self, value: &Value) -> Vec<agent::NormalizedEvent>;
}

pub struct AdapterRegistry {
    adapters: HashMap<String, Box<dyn HarnessAdapter>>,
}

fn model_options(items: &[(&str, &str)]) -> Vec<ModelOption> {
    items
        .iter()
        .map(|(id, label)| ModelOption {
            id: (*id).into(),
            label: (*label).into(),
        })
        .collect()
}

impl AdapterRegistry {
    pub fn built_in() -> Result<Self, BridgeError> {
        let mut registry = Self {
            adapters: HashMap::new(),
        };
        registry.register(Box::new(CodexAdapter))?;
        registry.register(Box::new(ClaudeAdapter {
            streams: Mutex::new(HashMap::new()),
        }))?;
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
        cwd: &str,
        model: Option<&str>,
        effort: Option<&str>,
        instructions: Option<&str>,
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
        adapter.start(cwd, model, effort, instructions)
    }

    pub fn normalize(&self, id: &str, value: &Value) -> Vec<agent::NormalizedEvent> {
        self.adapters
            .get(id)
            .map(|adapter| adapter.normalize(value))
            .unwrap_or_default()
    }
}

struct CodexAdapter;
impl HarnessAdapter for CodexAdapter {
    fn descriptor(&self) -> AdapterDescriptor {
        let version = codex_adapter::binary_version();
        AdapterDescriptor {
            id: "codex".into(),
            label: "Orchestrator".into(),
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
                ("gpt-5.6-luna", "GPT Luna"),
                ("gpt-5.6-terra", "GPT Terra"),
                ("gpt-5.6-sol", "GPT Sol"),
                ("gpt-5.3-codex", "GPT-5.3 Codex"),
            ]),
            default_model: Some(orchestrator::MODEL.into()),
        }
    }
    fn start(
        &self,
        cwd: &str,
        model: Option<&str>,
        effort: Option<&str>,
        instructions: Option<&str>,
    ) -> Result<StartedAdapter, BridgeError> {
        let started = codex_adapter::start(cwd, model, effort, instructions)?;
        Ok(StartedAdapter {
            runtime: Box::new(started.runtime),
            reader: Box::new(started.reader),
            startup_messages: started.startup_messages,
        })
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
            unavailable_reason: binary::resolve("claude")
                .is_none()
                .then(|| "Claude Code binary is not installed".into()),
            models: model_options(&[
                ("sonnet", "Claude Sonnet"),
                ("opus", "Claude Opus"),
                ("haiku", "Claude Haiku"),
                ("fable", "Claude Fable"),
            ]),
            default_model: Some("sonnet".into()),
        }
    }
    fn start(
        &self,
        cwd: &str,
        model: Option<&str>,
        effort: Option<&str>,
        instructions: Option<&str>,
    ) -> Result<StartedAdapter, BridgeError> {
        let started = claude_adapter::start(cwd, model, effort, instructions)?;
        Ok(StartedAdapter {
            runtime: Box::new(started.runtime),
            reader: Box::new(started.reader),
            startup_messages: started.startup_messages,
        })
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
        fn start(
            &self,
            _cwd: &str,
            _model: Option<&str>,
            _effort: Option<&str>,
            _instructions: Option<&str>,
        ) -> Result<StartedAdapter, BridgeError> {
            Err(BridgeError::Invalid("not launched in registry test".into()))
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
}
