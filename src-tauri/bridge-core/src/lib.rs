//! Tauri-free runtime for Bridge.
//!
//! Everything the desktop shell does that is not IPC wiring lives here: the
//! SQLite stores, harness adapters, PTY session runtimes, delegation policy,
//! worktree coordination, and supervision. The [`BridgeCore`] runtime owns
//! that state; hosts (the Tauri shell today, a headless daemon later) wrap it
//! in their own transport.

pub mod adapters;
pub mod agent;
pub mod agent_config;
pub mod binary;
pub mod browser_bridge;
pub mod claude_adapter;
pub mod codex_adapter;
pub mod compaction_controller;
pub mod completion;
pub mod context;
pub mod credential_broker;
pub mod delegation;
pub mod git;
pub mod handoff;
pub mod learning_job;
pub mod learning_router;
pub mod marketplace;
pub mod model;
pub mod model_profiles;
pub mod opencode_adapter;
pub mod orchestrator;
pub mod policy;
pub mod policy_coordinator;
pub mod policy_replay;
pub mod prompt_compiler;
pub mod restoration;
pub mod router_replay;
pub mod routing_policy;
mod runtime;
pub mod secret_interception;
pub mod session_forest;
pub mod session_supervisor;
pub mod skill_marketplace;
pub mod slash;
pub mod store;
pub mod worker_guard;
pub mod worker_lifecycle;
pub mod worker_pool;
pub mod worker_sandbox;
pub mod workspace_files;
pub mod worktree_coordinator;

pub use runtime::{
    start_health_server, BootConfig, BridgeCore, DelegationState, RuntimeSession,
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
