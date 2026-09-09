//! The worktree inventory: what exists on this disk, what it costs, and what
//! the retention caps are. Read-only — reclaiming is the sweep's job, and a
//! client asking for a deletion is a separate capability.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One inventoried checkout. Mirrors
/// `bridge_core::worktree_registry::WorktreeInventoryEntry`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeInventoryEntry {
    pub id: String,
    /// `orchestrator`, `worker`, or `github`.
    pub kind: String,
    pub repo_root: String,
    pub path: String,
    pub branch: Option<String>,
    pub owner_session_id: Option<String>,
    pub owner_workspace_id: Option<String>,
    /// `active`, `idle`, `orphaned`, `unverifiable`, or `external`.
    pub state: String,
    /// The last assessment: `reclaimable`, `pushed_unmerged`, `at_risk`,
    /// `retained`, or `unverifiable`. `null` until a sweep has looked at it.
    pub disposition: Option<String>,
    /// Why it is being kept, in words meant for a person.
    pub retained_reason: Option<String>,
    pub assessed_at: Option<String>,
    pub size_bytes: Option<i64>,
    pub size_measured_at: Option<String>,
    pub created_at: String,
    pub last_used_at: String,
    pub idle_seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct WorktreeInventoryResult(pub Vec<WorktreeInventoryEntry>);

/// Mirrors `bridge_core::worktree_registry::WorktreeRepositoryUsage`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeRepositoryUsage {
    pub repo_root: String,
    pub count: i64,
    pub size_bytes: i64,
    pub reclaimable_bytes: i64,
    pub over_budget: bool,
}

/// Mirrors `bridge_core::worktree_registry::WorktreeUsage`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeUsage {
    pub total_count: i64,
    pub total_bytes: i64,
    pub reclaimable_count: i64,
    pub reclaimable_bytes: i64,
    pub retained_count: i64,
    pub max_total_bytes: i64,
    pub max_per_repo: i64,
    pub worker_idle_ttl_seconds: i64,
    pub orchestrator_idle_ttl_seconds: i64,
    pub github_idle_ttl_seconds: i64,
    pub repositories: Vec<WorktreeRepositoryUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReclaimWorktreeParams {
    pub worktree_id: String,
}

/// What an explicit reclaim did, or why it did not. Mirrors
/// `bridge_core::worktree_registry::WorktreeReclaimResult`.
///
/// A refusal is a *result*, not an error: "no, and here is why" is the useful
/// answer for a button, and `detail` carries the reason in words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeReclaimResult {
    pub reclaimed: bool,
    pub bytes_freed: i64,
    pub disposition: String,
    pub detail: Option<String>,
}

/// Mirrors `bridge_core::worktree_registry::SweepOutcome`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeSweepResult {
    pub removed: usize,
    pub removed_bytes: u64,
    pub retained: usize,
    pub retained_bytes: u64,
    pub over_budget_bytes: u64,
    pub skipped: usize,
    pub measurements_truncated: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArchiveChatParams {
    pub session_id: String,
}

/// What archiving a chat did. Mirrors
/// `bridge_core::worktree_registry::ArchiveChatResult`.
///
/// `worktreeDetail` is set when the chat's checkout was *kept* — archiving a
/// conversation does not require first resolving its uncommitted work, so the
/// reason comes back rather than the archive failing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveChatResult {
    pub archived: bool,
    pub bytes_freed: i64,
    pub worktree_detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListArchivedChatsParams {
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub offset: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArchivedChat {
    pub id: String,
    pub title: String,
    pub harness: String,
    pub workspace_title: Option<String>,
    pub archived_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArchivedChatsResult {
    pub chats: Vec<ArchivedChat>,
    pub has_more: bool,
}
