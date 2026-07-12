use crate::{agent, codex_adapter, model::AdapterDescriptor, BridgeError};
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
    fn start(&self, cwd: &str) -> Result<StartedAdapter, BridgeError>;
    fn normalize(&self, value: &Value) -> Vec<agent::NormalizedEvent>;
}

pub struct AdapterRegistry {
    adapters: HashMap<String, Box<dyn HarnessAdapter>>,
}

impl AdapterRegistry {
    pub fn built_in() -> Result<Self, BridgeError> {
        let mut registry = Self {
            adapters: HashMap::new(),
        };
        registry.register(Box::new(CodexAdapter))?;
        registry.register(Box::new(UnavailableAdapter {
            descriptor: AdapterDescriptor {
                id: "claude".into(), label: "Claude Code".into(), available: false, version: None, capabilities: vec![],
                unavailable_reason: Some("Structured Claude adapter is not installed; Bridge will never fall back to its TUI".into()),
            },
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

    pub fn start(&self, id: &str, cwd: &str) -> Result<StartedAdapter, BridgeError> {
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
        adapter.start(cwd)
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
            unavailable_reason: which::which("codex")
                .is_err()
                .then(|| "Codex binary is not installed".into()),
        }
    }
    fn start(&self, cwd: &str) -> Result<StartedAdapter, BridgeError> {
        let started = codex_adapter::start(cwd)?;
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

struct UnavailableAdapter {
    descriptor: AdapterDescriptor,
}
impl HarnessAdapter for UnavailableAdapter {
    fn descriptor(&self) -> AdapterDescriptor {
        self.descriptor.clone()
    }
    fn start(&self, _cwd: &str) -> Result<StartedAdapter, BridgeError> {
        Err(BridgeError::Invalid(
            self.descriptor
                .unavailable_reason
                .clone()
                .unwrap_or_else(|| "Adapter unavailable".into()),
        ))
    }
    fn normalize(&self, _value: &Value) -> Vec<agent::NormalizedEvent> {
        vec![]
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
            }
        }
        fn start(&self, _cwd: &str) -> Result<StartedAdapter, BridgeError> {
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
