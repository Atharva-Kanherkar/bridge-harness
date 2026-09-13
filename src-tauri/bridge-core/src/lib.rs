//! Tauri-free runtime for Bridge.
//!
//! Everything the desktop shell does that is not IPC wiring lives here: the
//! SQLite stores, harness adapters, PTY session runtimes, delegation policy,
//! worktree coordination, and supervision. The [`BridgeCore`] runtime owns
//! that state; hosts (the Tauri shell today, a headless daemon later) wrap it
//! in their own transport.

pub mod acp_events;
pub mod acp_registry;
pub mod acp_session;
pub mod adapters;
pub mod agent;
pub mod agent_config;
pub mod agent_integration;
pub mod agent_lifecycle;
pub mod analytics;
pub mod api;
pub mod automations;
pub mod backend_binding;
pub mod binary;
pub mod briefing_conformance;
pub mod briefing_policy;
pub mod build_cache;
pub mod browser_bridge;
pub mod builtin_compatibility;
pub mod capability_projection;
pub mod check_runner;
pub mod claude_adapter;
pub mod claude_import;
pub mod codex_adapter;
pub mod compaction_controller;
pub mod connector_eval;
pub mod connector_inbox;
pub mod connector_runs;
pub mod connector_runs_live;
pub mod connector_surface;
pub mod completion;
pub mod context;
pub mod context_breakdown;
pub mod context_inventory;
pub mod credential_broker;
pub mod cursor_adapter;
pub mod grok_adapter;
pub mod delegation;
pub mod events;
pub mod external_import;
pub mod frame_queue;
pub mod git;
pub mod github_surface;
pub mod github_poll;
pub mod github_policy;
pub mod handoff;
pub mod health;
pub mod learning_job;
pub mod learning_router;
pub mod live_turn;
pub mod managed_agents;
pub mod managed_payload;
pub mod managed_runtime;
pub mod marketplace;
pub mod memory_consolidation;
pub mod memory_consolidation_live;
pub mod memory_extraction;
pub mod memory_extraction_live;
pub mod memory_ledger;
pub mod memory_packet;
pub mod meter;
pub mod meter_sources;
pub mod model;
pub mod model_catalog;
pub mod model_profiles;
pub mod opencode_adapter;
pub mod orchestrator;
pub mod ownership;
pub mod policy;
pub mod policy_coordinator;
pub mod provider_limit;
pub mod policy_replay;
pub mod project_onboarding;
pub mod process_ledger;
pub mod prompt_authority;
pub mod prompt_compiler;
pub mod prompt_sections;
pub mod prompt_studio;
pub mod prompt_mutation_policy;
pub mod prompt_mutations;
pub mod prompts;
/// Test-only: asserts core DTOs and their bridge-protocol mirrors agree.
#[cfg(test)]
mod protocol_mirror;
pub mod restoration;
pub mod router_replay;
pub mod routing_evaluation;
pub mod routing_evaluation_live;
pub mod routing_policy;
mod runtime;
pub mod secret_interception;
pub mod session_context;
pub mod session_forest;
pub mod session_input;
pub mod session_recall;
pub mod session_titles;
pub mod session_supervisor;
pub mod sessions;
pub mod skill_marketplace;
pub mod slash;
pub mod store;
pub mod usage_import;
pub mod suggestion_engine;
pub mod terminal_workspace;
pub mod switch_summary;
pub mod verification_pipeline;
pub mod usage;
pub mod usage_pricing;
pub mod usage_insights;
pub mod usage_summary;
pub mod usage_history;
pub mod verified_catalog;
pub mod worker_adoption;
pub mod worker_guard;
pub mod worker_lifecycle;
pub mod worker_pool;
pub mod worker_retry;
pub mod worker_settings;
pub mod dependency_seed;
pub mod worker_sandbox;
pub mod work;
pub mod work_actions;
pub mod work_brief_parser;
pub mod work_briefing_config;
pub mod work_brief_runner;
pub mod work_brief_store;
pub mod work_briefing_live;
pub mod work_briefing_trigger;
pub mod work_connectors;
pub mod work_evidence;
pub mod work_fingerprint;
pub mod work_reconcile;
pub mod work_task_state;
pub mod work_observation;
pub mod workspace_files;
pub mod workspaces;
pub mod worktree_coordinator;
pub mod worktree_registry;

pub use runtime::{
    start_health_server, BootConfig, BridgeCore, DelegationState, RuntimeSession,
    SessionLifecycleClaim, COMPLETION_VERIFY_TIMEOUT_SECONDS, WORKER_APPROVAL_TIMEOUT_SECONDS,
    WORKER_STALL_TIMEOUT_SECONDS,
};

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("{0}")]
    Invalid(String),
    #[error("Git: {0}")]
    Git(String),
    #[error("Database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("Adapter: {0}")]
    Adapter(String),
    #[error("PTY: {0}")]
    Pty(String),
}
impl Serialize for BridgeError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl From<&BridgeError> for bridge_protocol::ErrorCode {
    /// Exhaustive on purpose — adding a `BridgeError` variant must fail to
    /// compile here until the protocol contract assigns it a stable code.
    fn from(error: &BridgeError) -> Self {
        match error {
            BridgeError::Invalid(_) => bridge_protocol::ErrorCode::Invalid,
            BridgeError::Git(_) => bridge_protocol::ErrorCode::Git,
            BridgeError::Db(_) => bridge_protocol::ErrorCode::Database,
            BridgeError::Io(_) => bridge_protocol::ErrorCode::Io,
            BridgeError::Adapter(_) => bridge_protocol::ErrorCode::Adapter,
            BridgeError::Pty(_) => bridge_protocol::ErrorCode::Pty,
        }
    }
}

// The `HarnessId` ↔ `model::Harness` conversions live beside `Harness` in
// `model.rs`, because the wire id is now an open validated newtype rather than
// an enum mirrored variant for variant. See `model::Harness` for why core
// stays closed over the built-ins while the wire does not.

#[cfg(test)]
mod tests {
    use super::*;
    use bridge_protocol::ErrorCode;

    #[test]
    fn every_bridge_error_variant_maps_to_its_stable_protocol_code() {
        let cases: Vec<(BridgeError, ErrorCode, i64)> = vec![
            (BridgeError::Invalid("x".into()), ErrorCode::Invalid, 1000),
            (BridgeError::Git("x".into()), ErrorCode::Git, 1001),
            (
                BridgeError::Db(rusqlite::Error::QueryReturnedNoRows),
                ErrorCode::Database,
                1002,
            ),
            (
                BridgeError::Io(std::io::Error::new(std::io::ErrorKind::Other, "x")),
                ErrorCode::Io,
                1003,
            ),
            (BridgeError::Adapter("x".into()), ErrorCode::Adapter, 1004),
            (BridgeError::Pty("x".into()), ErrorCode::Pty, 1005),
        ];
        for (error, expected, expected_code) in cases {
            let mapped = ErrorCode::from(&error);
            assert_eq!(mapped, expected, "{error}");
            assert_eq!(mapped.code(), expected_code, "{error}");
        }
    }
}

mod runtime_budget;
