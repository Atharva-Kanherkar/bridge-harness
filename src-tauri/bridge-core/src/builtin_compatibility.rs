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
    /// Offered only where the agent advertised `session/resume` at the
    /// handshake. Distinct from `when_protocol_advertises`, which is Codex's
    /// runtime feature list: this is a capability the agent states per build,
    /// and an agent that advertises history replay without it gets a
    /// checkpoint handoff rather than a resume Bridge cannot perform.
    WhenSessionAdvertises,
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
    pub default_model_id: Option<&'static str>,
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
pub(crate) const CURSOR_SANDBOXES: &[SandboxMode] =
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
    "briefings",
];

const CLAUDE_CAPABILITIES: &[&str] = &[
    "messages",
    "streaming",
    "reasoning",
    "tools",
    "commands",
    // Claude's adapter normalizes Edit/Write/MultiEdit/NotebookEdit into
    // file_change.* events (synthesizing a unified diff), so it advertises the
    // same file_changes surface as Codex/OpenCode rather than flat tool rows.
    "file_changes",
    "approvals",
    "usage",
    "interrupt",
    "briefings",
    // Claude alone: the sidecar drives one streaming-input query, so a user
    // message written mid-turn is folded into the turn in flight. Codex and
    // OpenCode would take a second concurrent turn instead, so they must not
    // advertise this — Bridge queues their follow-ups.
    "steering",
];

const OPENCODE_CAPABILITIES: &[&str] = CODEX_CAPABILITIES;

/// Cursor is reached through the shared ACP client, so its surface is whatever
/// that client normalizes. `interrupt` is here because a cancel is a real
/// protocol notification the runtime sends, and image attachments are not,
/// because the client sends a text content block and advertising more than it
/// sends would route an image turn into a refusal at the seam. The adapter
/// descriptor reads this list rather than restating it.
pub(crate) const CURSOR_CAPABILITIES: &[&str] = &[
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
        model_source: ModelSource::RuntimeCatalog,
        model_ids: &[],
        default_model_id: Some("sonnet"),
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
        model_source: ModelSource::RuntimeCatalog,
        model_ids: &[],
        default_model_id: Some("gpt-5.6-luna"),
    },
    BuiltInAgentContract {
        id: "cursor",
        label: "Cursor",
        transport: "cursor_agent_acp_stdio",
        runtime_source: "cursor-agent acp",
        credential_owner: CredentialOwner::Vendor,
        native_resume: NativeResumeContract::WhenSessionAdvertises,
        capabilities: CURSOR_CAPABILITIES,
        sandbox_modes: CURSOR_SANDBOXES,
        model_source: ModelSource::RuntimeCatalog,
        model_ids: &[],
        default_model_id: None,
    },
    BuiltInAgentContract {
        id: "grok",
        label: "Grok Build",
        transport: "grok_agent_acp_stdio",
        runtime_source: "grok agent --no-leader stdio",
        credential_owner: CredentialOwner::Vendor,
        native_resume: NativeResumeContract::WhenSessionAdvertises,
        capabilities: CURSOR_CAPABILITIES,
        sandbox_modes: CURSOR_SANDBOXES,
        model_source: ModelSource::RuntimeCatalog,
        model_ids: &[],
        default_model_id: None,
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
        default_model_id: None,
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
    use crate::agent::{self, ClaudeStreamState, NormalizedEvent, OpenCodeStreamState};
    use serde::Deserialize;
    use serde_json::Value;
    use std::collections::{HashMap, HashSet};

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct EventFixtureDocument {
        schema_version: u32,
        providers: Vec<ProviderEventFixture>,
    }

    #[derive(Debug, Deserialize)]
    struct ProviderEventFixture {
        id: String,
        messages: Vec<Value>,
        expected: Vec<NormalizedSummary>,
    }

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "camelCase")]
    struct NormalizedSummary {
        kind: String,
        #[serde(default)]
        item_id: Option<String>,
        #[serde(default)]
        role: Option<String>,
        #[serde(default)]
        status: Option<String>,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        request_id: Option<Value>,
        #[serde(default)]
        turn_id: Option<String>,
        #[serde(default)]
        input_tokens: Option<u64>,
        #[serde(default)]
        output_tokens: Option<u64>,
        #[serde(default)]
        cached_input_tokens: Option<u64>,
        #[serde(default)]
        cache_write_tokens: Option<u64>,
        #[serde(default)]
        reasoning_tokens: Option<u64>,
        #[serde(default)]
        cost: Option<Value>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        provider: Option<String>,
        #[serde(default)]
        will_retry: Option<bool>,
        #[serde(default)]
        is_error: Option<bool>,
    }

    impl From<NormalizedEvent> for NormalizedSummary {
        fn from(event: NormalizedEvent) -> Self {
            let usage = event.data.get("usage").unwrap_or(&event.data);
            Self {
                kind: event.kind,
                item_id: event.item_id,
                role: event.role,
                status: event.status,
                title: event.title,
                text: event.text,
                request_id: event.data.get("requestId").cloned(),
                turn_id: event
                    .data
                    .get("turnId")
                    .or_else(|| event.data.pointer("/turn/id"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                input_tokens: usage
                    .get("input_tokens")
                    .or_else(|| usage.get("inputTokens"))
                    .and_then(Value::as_u64),
                output_tokens: usage
                    .get("output_tokens")
                    .or_else(|| usage.get("outputTokens"))
                    .and_then(Value::as_u64),
                // Adapters do not share one spelling for a cache read:
                // OpenCode says `cached_input_tokens`, Codex is normalized to
                // the ledger's own `cache_read_tokens`.
                cached_input_tokens: usage
                    .get("cached_input_tokens")
                    .or_else(|| usage.get("cachedInputTokens"))
                    .or_else(|| usage.get("cache_read_tokens"))
                    .and_then(Value::as_u64),
                cache_write_tokens: usage
                    .get("cache_write_tokens")
                    .or_else(|| usage.get("cacheWriteTokens"))
                    .and_then(Value::as_u64),
                reasoning_tokens: usage
                    .get("reasoning_tokens")
                    .or_else(|| usage.get("reasoningTokens"))
                    .and_then(Value::as_u64),
                cost: event
                    .data
                    .get("cost")
                    .or_else(|| event.data.get("totalCostUsd"))
                    .filter(|value| !value.is_null())
                    .cloned(),
                model: event
                    .data
                    .get("model")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                provider: event
                    .data
                    .get("provider")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                will_retry: event.data.get("willRetry").and_then(Value::as_bool),
                is_error: event.data.get("is_error").and_then(Value::as_bool),
            }
        }
    }

    #[test]
    fn built_in_contract_has_exactly_the_existing_agents() {
        let ids = built_in_agent_contracts()
            .iter()
            .map(|agent| agent.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, ["claude", "codex", "cursor", "grok", "opencode"]);
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
            "cursor_agent_acp_stdio"
        );
        assert_eq!(
            built_in_agent_contracts()[3].transport,
            "grok_agent_acp_stdio"
        );
        assert_eq!(
            built_in_agent_contracts()[4].transport,
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
                    assert_eq!(
                        descriptor.default_model.as_deref(),
                        contract.default_model_id,
                        "{} default model",
                        contract.id
                    );
                }
                ModelSource::RuntimeCatalog => assert!(contract.model_ids.is_empty()),
            }
        }
    }

    #[test]
    fn compatibility_report_is_deterministic_and_schema_versioned() {
        let actual = serde_json::to_string_pretty(&compatibility_report()).unwrap();
        let expected =
            include_str!("../../../testing/fixtures/builtin-compatibility-report-v1.json")
                .trim_end();
        assert_eq!(actual, expected);
        let value: serde_json::Value = serde_json::from_str(&actual).unwrap();
        assert_eq!(value["schemaVersion"], SCHEMA_VERSION);
        assert_eq!(value["agents"].as_array().unwrap().len(), 5);
        let lowercase = actual.to_ascii_lowercase();
        for forbidden in [
            "api_key",
            "apikey",
            "access_token",
            "authorization",
            "bearer ",
            "password",
            "sk-ant-",
        ] {
            assert!(
                !lowercase.contains(forbidden),
                "compatibility report contains secret-shaped field {forbidden:?}"
            );
        }
    }

    #[test]
    fn representative_provider_streams_match_normalized_snapshots() {
        let fixtures: EventFixtureDocument = serde_json::from_str(include_str!(
            "../../../testing/fixtures/builtin-adapter-events-v1.json"
        ))
        .unwrap();
        assert_eq!(fixtures.schema_version, 1);
        assert_eq!(fixtures.providers.len(), 3);
        assert_eq!(
            fixtures
                .providers
                .iter()
                .map(|fixture| fixture.id.as_str())
                .collect::<HashSet<_>>(),
            HashSet::from(["claude", "codex", "opencode"])
        );

        for fixture in fixtures.providers {
            let actual = normalize_fixture(&fixture.id, &fixture.messages)
                .into_iter()
                .map(NormalizedSummary::from)
                .collect::<Vec<_>>();
            assert_eq!(actual, fixture.expected, "{} fixture drifted", fixture.id);
        }
    }

    fn normalize_fixture(id: &str, messages: &[Value]) -> Vec<NormalizedEvent> {
        match id {
            "claude" => {
                let mut state = ClaudeStreamState::default();
                messages
                    .iter()
                    .flat_map(|message| {
                        agent::normalize_claude_message_with_state(message, &mut state)
                    })
                    .collect()
            }
            "codex" => messages
                .iter()
                .flat_map(|message| {
                    if message.get("id").is_some() && message.get("method").is_some() {
                        agent::normalize_codex_request(message)
                            .into_iter()
                            .collect()
                    } else {
                        agent::normalize_codex_message(message)
                    }
                })
                .collect(),
            "opencode" => {
                let mut state = OpenCodeStreamState::default();
                messages
                    .iter()
                    .flat_map(|message| {
                        agent::normalize_opencode_message_with_state(message, &mut state)
                    })
                    .collect()
            }
            other => panic!("unknown built-in fixture {other}"),
        }
    }
}
