//! The taste layer: rules learned from what a person keeps, edits and drops
//! in reviews, and the decisions behind each one. Rules are memory records
//! with `layer = taste`, approved, edited and deleted through the memory
//! methods; these list them across repositories, show their evidence, and add
//! one by hand.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::MemoryRecord;

/// The kinds a taste rule can have.
pub const TASTE_RULE_KINDS: [&str; 3] = ["review_avoid", "review_prefer", "code_prefer"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListTasteRulesParams {
    /// `active` (default), `proposed`, `paused`, `rejected` or `expired`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

/// Every taste rule in one status, in every repository, newest change first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListTasteRulesResult {
    pub rules: Vec<MemoryRecord>,
    /// Rules given to at least ten reviews and cited by none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub never_used: Vec<String>,
}

/// Pause, resume or pin a taste rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetTasteRuleStateParams {
    pub record_id: String,
    /// `active` or `paused`; absent leaves it as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Absent leaves the pin as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetTasteEvidenceParams {
    pub record_id: String,
}

/// One decision behind a rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TasteSignal {
    pub id: String,
    pub at: String,
    /// `review_decision`, `review_fixed`, `review_thread` or `permission`.
    pub source: String,
    /// `kept`, `edited`, `dropped`, `fixed`, `resolved`, `ignored`,
    /// `allowed` or `declined`.
    pub verdict: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_number: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    /// How much this decision counts toward the rule.
    pub weight: f64,
}

/// The decisions behind a rule, oldest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TasteEvidence {
    pub signals: Vec<TasteSignal>,
    /// Links whose decision was swept after a year: counted, no longer shown.
    pub swept: u32,
}

/// A rule the person writes themselves. It is active at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AddTasteRuleParams {
    /// `project:<owner>/<repo>`, or `account:local` for every repository.
    pub scope_key: String,
    /// One of [`TASTE_RULE_KINDS`].
    pub kind: String,
    pub body: String,
    /// Absent applies to every path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_globs: Option<Vec<String>>,
}

/// A checkout of the repository, where rule files are read and written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TasteCheckout {
    pub workspace_id: String,
    pub title: String,
    pub path: String,
}

/// One instruction file as export would leave it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TasteFileChange {
    /// `AGENTS.md`, `CLAUDE.md` or `.cursor/BUGBOT.md`, from the checkout's root.
    pub path: String,
    pub before: String,
    pub after: String,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportTasteRulesParams {
    /// `project:<owner>/<repo>`.
    pub scope_key: String,
    /// The checkout to write into; absent uses the first one Bridge knows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// Files to write; absent is all three.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targets: Option<Vec<String>>,
    /// False previews; true writes. Bridge never commits.
    pub write: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExportTasteRulesResult {
    pub checkouts: Vec<TasteCheckout>,
    pub workspace_id: String,
    pub files: Vec<TasteFileChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportTasteRulesParams {
    pub scope_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// False previews; true applies.
    pub apply: bool,
}

/// A rule whose text the file changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TasteRuleUpdate {
    pub record_id: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportTasteRulesResult {
    pub checkouts: Vec<TasteCheckout>,
    pub workspace_id: String,
    /// New bullets, which become rules you wrote.
    pub added: Vec<String>,
    pub updated: Vec<TasteRuleUpdate>,
    /// Active rules the files no longer list. Reported, never deleted.
    pub missing: Vec<String>,
}
