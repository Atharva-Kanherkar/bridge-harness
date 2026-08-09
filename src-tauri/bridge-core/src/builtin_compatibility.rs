//! Stable compatibility metadata for Bridge's built-in agent integrations.
//!
//! This is deliberately independent of local installation state. The managed
//! payload work can change where a runtime is found, but it must not silently
//! change the transport, credential boundary, or normalized Bridge surface.

use crate::model::SandboxMode;
use serde::Serialize;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialOwner {
    Vendor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeResumeContract {
    WhenRuntimeAvailable,
    WhenProtocolAdvertises,
    WhenCatalogDiscovered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSource {
    Static,
    RuntimeCatalog,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltInAgentContract {
    pub id: &'static str,
    pub label: &'static str,
    pub transport: &'static str,
    pub runtime_source: &'static str,
    pub credential_owner: CredentialOwner,
    pub native_resume: NativeResumeContract,
    pub capabilities: &'static [&'static str],
    pub sandbox_modes: &'static [SandboxMode],
    pub model_source: ModelSource,
    pub model_ids: &'static [&'static str],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltInCompatibilityReport {
    pub schema_version: u32,
    pub agents: &'static [BuiltInAgentContract],
}

const ALL_SANDBOXES: &[SandboxMode] = &SandboxMode::ALL;
const OPENCODE_SANDBOXES: &[SandboxMode] =
    &[SandboxMode::WorkspaceWrite, SandboxMode::DangerFullAccess];

const CODEX_CAPABILITIES: &[&str] = &[
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
];

const CLAUDE_CAPABILITIES: &[&str] = &[
    "messages",
    "streaming",
    "reasoning",
    "tools",
    "commands",
    "approvals",
    "usage",
    "interrupt",
];

const OPENCODE_CAPABILITIES: &[&str] = CODEX_CAPABILITIES;

const BUILT_IN_AGENTS: &[BuiltInAgentContract] = &[
    BuiltInAgentContract {
        id: "claude",
        label: "Claude Code",
        transport: "claude_agent_sdk_sidecar",
        runtime_source: "@anthropic-ai/claude-agent-sdk via Bridge Node sidecar",
        credential_owner: CredentialOwner::Vendor,
        native_resume: NativeResumeContract::WhenRuntimeAvailable,
        capabilities: CLAUDE_CAPABILITIES,
        sandbox_modes: ALL_SANDBOXES,
        model_source: ModelSource::Static,
        model_ids: &["haiku", "sonnet", "opus", "fable"],
    },
    BuiltInAgentContract {
        id: "codex",
        label: "Codex",
        transport: "codex_app_server_stdio",
        runtime_source: "codex app-server --listen stdio://",
        credential_owner: CredentialOwner::Vendor,
        native_resume: NativeResumeContract::WhenProtocolAdvertises,
        capabilities: CODEX_CAPABILITIES,
        sandbox_modes: ALL_SANDBOXES,
        model_source: ModelSource::Static,
        model_ids: &[
            "gpt-5.6-luna",
            "gpt-5.6-terra",
            "gpt-5.6-sol",
            "gpt-5.3-codex",
        ],
    },
    BuiltInAgentContract {
        id: "opencode",
        label: "OpenCode",
        transport: "opencode_authenticated_loopback_http",
        runtime_source: "opencode serve --hostname 127.0.0.1 --port <bridge-port>",
        credential_owner: CredentialOwner::Vendor,
        native_resume: NativeResumeContract::WhenCatalogDiscovered,
        capabilities: OPENCODE_CAPABILITIES,
        sandbox_modes: OPENCODE_SANDBOXES,
        model_source: ModelSource::RuntimeCatalog,
        model_ids: &[],
    },
];

pub fn built_in_agent_contracts() -> &'static [BuiltInAgentContract] {
    BUILT_IN_AGENTS
}

pub fn compatibility_report() -> BuiltInCompatibilityReport {
    BuiltInCompatibilityReport {
        schema_version: SCHEMA_VERSION,
        agents: BUILT_IN_AGENTS,
    }
}

#[cfg(test)]
mod builtin_compatibility_tests {
    use super::*;
    use crate::adapters::AdapterRegistry;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn built_in_contract_has_exactly_the_existing_agents() {
        let ids = built_in_agent_contracts()
            .iter()
            .map(|agent| agent.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, ["claude", "codex", "opencode"]);
        assert_eq!(ids.iter().copied().collect::<HashSet<_>>().len(), ids.len());
    }

    #[test]
    fn built_in_contract_preserves_transport_runtime_and_vendor_auth_boundaries() {
        for agent in built_in_agent_contracts() {
            assert_eq!(agent.credential_owner, CredentialOwner::Vendor);
            assert!(!agent.transport.is_empty());
            assert!(!agent.runtime_source.is_empty());
            assert!(!agent.runtime_source.contains("token"));
            assert!(!agent.runtime_source.contains("key="));
        }
        assert_eq!(
            built_in_agent_contracts()[0].transport,
            "claude_agent_sdk_sidecar"
        );
        assert_eq!(
            built_in_agent_contracts()[1].transport,
            "codex_app_server_stdio"
        );
        assert_eq!(
            built_in_agent_contracts()[2].transport,
            "opencode_authenticated_loopback_http"
        );
    }

    #[test]
    fn built_in_contract_matches_adapter_descriptors() {
        let registry = AdapterRegistry::built_in().unwrap();
        let descriptors = registry
            .descriptors()
            .into_iter()
            .map(|descriptor| (descriptor.id.clone(), descriptor))
            .collect::<HashMap<_, _>>();
        let contracts = built_in_agent_contracts();

        assert_eq!(descriptors.len(), contracts.len());
        for contract in contracts {
            let descriptor = descriptors
                .get(contract.id)
                .unwrap_or_else(|| panic!("{} adapter is not registered", contract.id));
            assert_eq!(descriptor.label, contract.label);
            assert_eq!(
                descriptor.capabilities,
                contract.capabilities.iter().copied().collect::<Vec<_>>()
            );
            assert_eq!(descriptor.sandbox_modes, contract.sandbox_modes);

            let model_ids = descriptor
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>();
            match contract.model_source {
                ModelSource::Static => {
                    assert_eq!(model_ids, contract.model_ids, "{} models", contract.id);
                }
                ModelSource::RuntimeCatalog => assert!(contract.model_ids.is_empty()),
            }
        }
    }

    #[test]
    fn compatibility_report_is_deterministic_and_schema_versioned() {
        let first = serde_json::to_string_pretty(&compatibility_report()).unwrap();
        let second = serde_json::to_string_pretty(&compatibility_report()).unwrap();
        assert_eq!(first, second);
        let value: serde_json::Value = serde_json::from_str(&first).unwrap();
        assert_eq!(value["schemaVersion"], SCHEMA_VERSION);
        assert_eq!(value["agents"].as_array().unwrap().len(), 3);
    }
}
