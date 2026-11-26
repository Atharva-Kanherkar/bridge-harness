use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Named account scope. Never SQL NULL; never inferred from a missing workspace.
pub const ACCOUNT_MEMORY_SCOPE: &str = "account:local";
pub const MAX_MEMORY_BODY_CHARS: usize = 4000;
pub const MAX_MEMORY_LIST_LIMIT: u32 = 50;
/// The kind vocabulary, in the order surfaces present it.
pub const MEMORY_KINDS: [&str; 4] = ["preference", "fact", "decision", "constraint"];

/// Explicit pin. Scope is always written as `account:local` by the server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveMemoryRecordParams {
    pub body: String,
    /// `preference` (default), `fact`, `decision`, or `constraint`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Optional session that originated the slash/command. Not a scope key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListMemoryRecordsParams {
    pub scope_key: String,
    /// `active` (default), `proposed`, or `expired` (retired or expired
    /// records a person can restore). Nothing else lists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// The `nextCursor` of the previous page. Absent reads the first page. A
    /// cursor that does not parse, or belongs to another status, is refused
    /// rather than read as the first page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// Records per page, 1 to [`MAX_MEMORY_LIST_LIMIT`]. Absent is the maximum.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteMemoryRecordParams {
    pub record_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRecord {
    pub id: String,
    pub scope_key: String,
    pub kind: String,
    pub body: String,
    pub provenance: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_session_id: Option<String>,
    /// Written only by producers that can mean it (the extractor). An explicit
    /// save keeps it NULL, and surfaces render unknown — never a fake number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence_bps: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    /// Set when this record replaced an earlier one through supersede.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    /// The instant this record's claim began to hold. Validity is an interval,
    /// not a flag: a record that never reached active carries an empty one,
    /// where `validTo` equals `validFrom`.
    pub valid_from: String,
    /// The instant the claim stopped holding. Absent while the record is
    /// active; a supersession, a tombstone, or an expiry closes it, and a
    /// closed record stays queryable as history.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<String>,
    /// When set, the instant after which the record stops applying. The sweep
    /// closes the interval at this instant rather than at the moment it ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// Records making competing claims about one subject share this key. At
    /// most one member of a group is active, so a packet can never carry two.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict_group: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Read a scope as it stood at an instant: exactly the records whose validity
/// interval contains it. As of now this is the active set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListMemoryRecordsAsOfParams {
    pub scope_key: String,
    /// RFC 3339. The instant to read the scope at.
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListMemoryRecordsResult {
    pub scope_key: String,
    pub records: Vec<MemoryRecord>,
    /// Every record of the listed status in the scope, not just this page.
    #[serde(default)]
    pub total: i64,
    /// Pass back as `cursor` for the next page. Absent on the last page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Edit is supersession: a new record replaces the old, which stays history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SupersedeMemoryRecordParams {
    pub record_id: String,
    pub body: String,
    /// Inherited from the superseded record when omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApproveMemoryRecordParams {
    pub record_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RejectMemoryRecordParams {
    pub record_id: String,
}

/// The most record ids one review batch may name, approvals and rejections
/// together.
pub const MAX_MEMORY_REVIEW_BATCH: usize = 200;

/// Approve and reject many proposals in one transaction. An id may appear in
/// one list only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewMemoryRecordsParams {
    pub scope_key: String,
    #[serde(default)]
    pub approve: Vec<String>,
    #[serde(default)]
    pub reject: Vec<String>,
}

/// A record the batch left alone, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryReviewRefusal {
    pub id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewMemoryRecordsResult {
    pub scope_key: String,
    pub approved: Vec<String>,
    pub rejected: Vec<String>,
    pub refused: Vec<MemoryReviewRefusal>,
}

/// Extraction has three account-local modes. `propose` is the safe default and
/// queues every gated candidate for review. `remember` stops future extraction;
/// new memory is then saved only explicitly, while existing active extracted
/// records remain active. `auto_apply` runs the same extractor but promotes
/// only candidates at or above 90% confidence grounded in a cited, visible user
/// message and cleared by the deterministic stability, tombstone, and conflict
/// guards. Other validated candidates that fit the scope budget remain proposed
/// for review; invalid, unsafe, duplicate, or over-budget output is refused.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateExtractionSettingsParams {
    pub mode: String,
    /// Deprecated and ignored: extraction runs on the memory model
    /// (`memory/set_memory_model`). Accepted for one release so older
    /// clients still decode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryExtractionRun {
    pub status: String,
    pub proposal_count: i64,
    /// Candidates the run found already remembered, so it wrote nothing for
    /// them. Absent from a pre-1.62 daemon, which reads as 0.
    #[serde(default)]
    pub known_count: i64,
    pub observed_tokens: i64,
    pub spend_microusd: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryExtractionSettings {
    pub scope_key: String,
    pub mode: String,
    /// Deprecated read-only mirror of the resolved memory model
    /// (`memory/get_memory_model`). Absent when none resolves.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The newest run in this scope, spend observed — never a simulated zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run: Option<MemoryExtractionRun>,
}

/// Consolidation is opt-in per scope. `off` is the default and means nothing
/// automatic; `propose` runs on the memory model once a scope has been quiet
/// for the debounce window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateConsolidationSettingsParams {
    pub mode: String,
    /// Deprecated and ignored: consolidation runs on the memory model
    /// (`memory/set_memory_model`). Accepted for one release so older
    /// clients still decode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// How many records the scope may hold before every write is refused.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_records: Option<i64>,
    /// Whether a proposal may ask for removal. Off by default; with it off an
    /// operation asking for removal is refused rather than downgraded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_removal: Option<bool>,
    /// How long a full scope waits before its dream becomes due.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debounce_seconds: Option<i64>,
    /// The local hour, 0-23, the nightly dream becomes due.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dream_hour: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryConsolidationRun {
    /// The run's id, for `memory/get_dream_report`. Absent from a pre-1.63
    /// daemon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// What queued it: `nightly`, `manual`, `full`, or `turn` before 1.63.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    /// One of `queued`, `running`, `completed`, `failed`, or `cancelled`.
    pub status: String,
    pub applied_count: i64,
    pub refused_count: i64,
    pub observed_tokens: i64,
    pub spend_microusd: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryConsolidationSettings {
    pub scope_key: String,
    pub mode: String,
    /// Deprecated read-only mirror of the resolved memory model
    /// (`memory/get_memory_model`). Absent when none resolves.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub max_records: i64,
    /// Deprecated and ignored: a dream's retire is restorable, so it needs no
    /// opt-in.
    pub allow_removal: bool,
    pub debounce_seconds: i64,
    /// The local hour, 0-23, the nightly dream becomes due. 3 from a pre-1.63
    /// daemon.
    #[serde(default = "default_dream_hour")]
    pub dream_hour: i64,
    /// What the scope currently holds against `maxRecords`. A write that would
    /// take this past the budget is refused; nothing is ever evicted for it.
    pub held_records: i64,
    /// The newest settled run in this scope, spend observed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run: Option<MemoryConsolidationRun>,
}

/// The harness and model the user pinned for memory jobs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryModelChoice {
    pub harness: String,
    pub model: String,
}

/// Where the memory model came from: the user's pin, the new-chat default,
/// or what a new chat starts on when that default is unset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryModelSource {
    User,
    ChatDefault,
    Standard,
}

/// The one harness and model every memory job runs on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedMemoryModel {
    pub harness: String,
    /// Absent runs the harness on its default model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub source: MemoryModelSource,
}

/// A choice the resolver passed over, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkippedMemoryModel {
    pub harness: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub source: MemoryModelSource,
    pub reason: String,
}

/// Extraction, consolidation and automatic approval all run on `resolved`.
/// It is absent only when no choice can run a tool-free turn; `skipped` then
/// says why for each.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryModelSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pinned: Option<MemoryModelChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved: Option<ResolvedMemoryModel>,
    pub skipped: Vec<SkippedMemoryModel>,
}

/// Pin both a harness and a model, or neither to clear the pin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetMemoryModelParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryInjectionSettings {
    pub scope_key: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetMemoryInjectionParams {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetPacketAuditParams {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryPacketItem {
    pub record_id: String,
    pub body: String,
    pub kind: String,
    pub reason: String,
}

/// The newest retrieval audit for a session. An empty selection means the
/// session started without a packet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryPacketAudit {
    pub session_id: String,
    pub selected: Vec<MemoryPacketItem>,
    pub token_estimate: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

/// One record's recall history over the last 14 days of packet audits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRecallStat {
    pub id: String,
    pub recalls: i64,
    /// Day bucket of the last recall (0 = 13 days ago, 13 = today); -1 if none.
    pub last_recalled_day: i64,
    /// recalls / packets built in the window.
    pub in_packet_ratio: f64,
    /// 14-day recall series, oldest first.
    pub daily: Vec<i64>,
}

/// How many packets left a record out for one reason, as the audit coded it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryExclusionCount {
    pub code: String,
    pub count: i64,
}

/// Active records of one kind and how many of them reached a packet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryKindUse {
    pub kind: String,
    pub active: i64,
    /// Active records of this kind recalled at least once in the window.
    pub recalled: i64,
    pub recalls: i64,
}

/// Recall analytics for the account scope, folded from the retrieval audits.
/// It measures delivery (what reached a prompt), not whether the model used it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRecallStats {
    pub per_record: Vec<MemoryRecallStat>,
    /// Packets built per day bucket, oldest first, 14 entries.
    pub injections_per_day: Vec<i64>,
    pub budget_chars_used: i64,
    pub budget_chars_max: i64,
    /// Packets built in the window, and how many carried at least one memory.
    pub packets: i64,
    pub packets_with_memories: i64,
    pub active_records: i64,
    /// Active records recalled at least once in the window.
    pub recalled_records: i64,
    pub by_kind: Vec<MemoryKindUse>,
    pub exclusions: Vec<MemoryExclusionCount>,
}

/// One settled extraction or consolidation run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryActivityEntry {
    /// `extraction`, `consolidation`, or `retention` (a sweep that expired
    /// proposals or retired unused memories).
    pub source: String,
    pub status: String,
    /// Proposals written (extraction) or changes applied (consolidation).
    pub applied: i64,
    pub refused: i64,
    /// Candidates an extraction run found already remembered. 0 for other
    /// sources and from a pre-1.62 daemon.
    #[serde(default)]
    pub known: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub at: String,
    /// The run's id. Present for consolidation runs from a 1.63 daemon, so a
    /// dream can be opened and undone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Observed spend. 0 for sweeps and from a pre-1.63 daemon.
    #[serde(default)]
    pub spend_microusd: i64,
    /// What a dream changed, by kind. Absent for other sources.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dream: Option<MemoryDreamTally>,
}

/// What one dream applied, by kind, and how many of those were undone.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDreamTally {
    pub merged: i64,
    pub corrected: i64,
    pub expired: i64,
    pub grouped: i64,
    pub retired: i64,
    pub approved: i64,
    pub rejected: i64,
    pub kept: i64,
    pub undone: i64,
}

fn default_dream_hour() -> i64 {
    3
}

/// "Dream now": the run that will dream, and whether it was already open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDreamStarted {
    pub run_id: String,
    pub already_open: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetDreamReportParams {
    pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UndoDreamParams {
    pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UndoDreamOperationParams {
    pub run_id: String,
    /// The change's `index` in the report.
    pub op_index: i64,
}

/// One record as a dream change saw it, for the before and after of a diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDreamRecord {
    pub id: String,
    pub body: String,
    pub kind: String,
    pub status: String,
    /// A memory the person saved themselves.
    pub pin: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

/// One applied change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDreamOperation {
    pub index: i64,
    /// `merge`, `correct`, `expire`, `group`, `retire`, `approve` or `reject`.
    pub operation: String,
    /// A rejection's reason or a group's subject.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The records it changed, as they were.
    pub before: Vec<MemoryDreamRecord>,
    /// The records it changed or created, as it left them.
    pub after: Vec<MemoryDreamRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undone_at: Option<String>,
}

/// Everything one dream did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDreamReport {
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    pub status: String,
    pub at: String,
    pub spend_microusd: i64,
    pub operations: Vec<MemoryDreamOperation>,
    pub kept: i64,
    /// Operations the gate refused.
    pub refused: i64,
    /// Records past the night's batches, left for the next dream.
    pub deferred: i64,
}

/// A change undo all left alone, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDreamSkip {
    pub index: i64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDreamUndoResult {
    pub report: MemoryDreamReport,
    pub skipped: Vec<MemoryDreamSkip>,
}

/// Settled runs, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryActivityLog {
    pub entries: Vec<MemoryActivityEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetInsightsParams {
    /// Run the analysis again. Without it only the stored report is read, so
    /// opening the tab never sends memory bodies to a provider.
    #[serde(default)]
    pub refresh: bool,
}

/// The Memory Insights report. `stats` and `kinds` are Bridge's own figures;
/// the prose is the model's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryInsightsReport {
    pub headline: String,
    pub summary: String,
    pub highlights: Vec<crate::messages::UsageInsightHighlight>,
    pub themes: Vec<crate::messages::UsageInsightTheme>,
    pub recommendations: Vec<String>,
    pub stats: MemoryRecallStats,
    pub memories_analysed: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryInsightsResult {
    pub status: crate::messages::UsageInsightsStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<MemoryInsightsReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// What memory exists, so surfaces can be honest about what they do not own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryCapabilities {
    pub ledger: MemoryLedgerCapability,
    /// Provider-owned memory commands currently reachable through the slash
    /// catalog. An unavailable adapter contributes nothing.
    pub provider_native: Vec<ProviderMemoryCommand>,
}

/// The Bridge half: the local ledger's contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryLedgerCapability {
    pub exists: bool,
    pub scope_key: String,
    pub max_body_chars: u32,
    pub kinds: Vec<String>,
}

/// A provider-owned memory command in the catalog. It stays on that provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderMemoryCommand {
    pub harness: String,
    pub command: String,
    pub description: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn list_pages_and_review_batches_have_camel_case_wire_shapes() {
        let params: ListMemoryRecordsParams = serde_json::from_value(json!({
            "scopeKey": "account:local",
            "status": "proposed",
            "cursor": "abc",
            "limit": 25
        }))
        .unwrap();
        assert_eq!(params.cursor.as_deref(), Some("abc"));
        assert_eq!(params.limit, Some(25));
        let result = ListMemoryRecordsResult {
            scope_key: "account:local".into(),
            records: Vec::new(),
            total: 138,
            next_cursor: Some("next".into()),
        };
        let wire = serde_json::to_value(&result).unwrap();
        assert_eq!(wire["total"], json!(138));
        assert_eq!(wire["nextCursor"], json!("next"));
        let last = ListMemoryRecordsResult { next_cursor: None, ..result };
        assert!(serde_json::to_value(&last).unwrap().get("nextCursor").is_none());

        let review: ReviewMemoryRecordsParams =
            serde_json::from_value(json!({ "scopeKey": "account:local", "approve": ["a"] })).unwrap();
        assert_eq!(review.approve, vec!["a".to_string()]);
        assert!(review.reject.is_empty(), "an omitted list is empty");
        assert!(serde_json::from_value::<ReviewMemoryRecordsParams>(json!({
            "scopeKey": "account:local",
            "approveAll": true
        }))
        .is_err());
    }

    #[test]
    fn memory_model_settings_have_camel_case_wire_shapes() {
        let settings = MemoryModelSettings {
            pinned: None,
            resolved: Some(ResolvedMemoryModel {
                harness: "claude".into(),
                model: Some("claude-haiku-5-5".into()),
                source: MemoryModelSource::ChatDefault,
            }),
            skipped: vec![SkippedMemoryModel {
                harness: "codex".into(),
                model: None,
                source: MemoryModelSource::User,
                reason: "no per-tool authority".into(),
            }],
        };
        let wire = serde_json::to_value(&settings).unwrap();
        assert!(wire.get("pinned").is_none(), "no pin is absent, not null");
        assert_eq!(wire["resolved"]["source"], json!("chat_default"));
        assert_eq!(wire["resolved"]["model"], json!("claude-haiku-5-5"));
        assert_eq!(wire["skipped"][0]["source"], json!("user"));
        assert!(wire["skipped"][0].get("model").is_none());

        let clear: SetMemoryModelParams = serde_json::from_value(json!({})).unwrap();
        assert_eq!(clear, SetMemoryModelParams { harness: None, model: None });
        let pin: SetMemoryModelParams =
            serde_json::from_value(json!({ "harness": "claude", "model": "claude-haiku-5-5" })).unwrap();
        assert_eq!(pin.model.as_deref(), Some("claude-haiku-5-5"));
        assert!(serde_json::from_value::<SetMemoryModelParams>(json!({ "account": "x" })).is_err());
    }

    #[test]
    fn already_known_counts_default_to_zero_from_an_older_daemon() {
        let run: MemoryExtractionRun = serde_json::from_value(json!({
            "status": "completed", "proposalCount": 1, "observedTokens": 10, "spendMicrousd": 5, "updatedAt": "now"
        }))
        .unwrap();
        assert_eq!(run.known_count, 0);
        let wire = serde_json::to_value(MemoryExtractionRun { known_count: 3, ..run }).unwrap();
        assert_eq!(wire["knownCount"], json!(3));
        let entry: MemoryActivityEntry = serde_json::from_value(json!({
            "source": "extraction", "status": "completed", "applied": 2, "refused": 0, "at": "now"
        }))
        .unwrap();
        assert_eq!(entry.known, 0);
        assert_eq!(serde_json::to_value(MemoryActivityEntry { known: 4, ..entry }).unwrap()["known"], json!(4));
    }

    #[test]
    fn dream_shapes_are_camel_case_and_default_from_an_older_daemon() {
        let entry: MemoryActivityEntry = serde_json::from_value(json!({
            "source": "consolidation", "status": "completed", "applied": 2, "refused": 0, "at": "now"
        }))
        .unwrap();
        assert_eq!((entry.run_id.as_deref(), entry.spend_microusd, entry.dream.as_ref()), (None, 0, None));
        let wire = serde_json::to_value(MemoryActivityEntry {
            run_id: Some("r".into()),
            spend_microusd: 40_000,
            dream: Some(MemoryDreamTally { merged: 6, approved: 4, ..MemoryDreamTally::default() }),
            ..entry
        })
        .unwrap();
        assert_eq!(wire["runId"], json!("r"));
        assert_eq!(wire["spendMicrousd"], json!(40_000));
        assert_eq!(wire["dream"]["merged"], json!(6));
        assert_eq!(wire["dream"]["undone"], json!(0));

        let settings: MemoryConsolidationSettings = serde_json::from_value(json!({
            "scopeKey": "account:local", "mode": "propose", "maxRecords": 200, "allowRemoval": false,
            "debounceSeconds": 600, "heldRecords": 3
        }))
        .unwrap();
        assert_eq!(settings.dream_hour, 3, "a pre-1.63 daemon dreams at 03:00");
        let update: UpdateConsolidationSettingsParams =
            serde_json::from_value(json!({ "mode": "off", "dreamHour": 5 })).unwrap();
        assert_eq!(update.dream_hour, Some(5));

        let undo: UndoDreamOperationParams = serde_json::from_value(json!({ "runId": "r", "opIndex": 2 })).unwrap();
        assert_eq!(undo.op_index, 2);
        assert!(serde_json::from_value::<UndoDreamParams>(json!({ "runId": "r", "x": 1 })).is_err());
        let report = MemoryDreamReport {
            run_id: "r".into(),
            trigger: Some("nightly".into()),
            status: "completed".into(),
            at: "now".into(),
            spend_microusd: 1,
            operations: vec![MemoryDreamOperation {
                index: 0,
                operation: "merge".into(),
                reason: None,
                before: vec![MemoryDreamRecord { id: "a".into(), body: "b".into(), kind: "fact".into(), status: "active".into(), pin: false, expires_at: None }],
                after: vec![],
                undone_at: None,
            }],
            kept: 1,
            refused: 0,
            deferred: 0,
        };
        let wire = serde_json::to_value(MemoryDreamUndoResult { report, skipped: vec![MemoryDreamSkip { index: 0, reason: "changed".into() }] }).unwrap();
        assert_eq!(wire["report"]["runId"], json!("r"));
        assert_eq!(wire["report"]["spendMicrousd"], json!(1));
        assert!(wire["report"]["operations"][0].get("undoneAt").is_none());
        assert!(wire["report"]["operations"][0]["before"][0].get("expiresAt").is_none());
        assert_eq!(wire["skipped"][0]["index"], json!(0));
        let started = serde_json::to_value(MemoryDreamStarted { run_id: "r".into(), already_open: true }).unwrap();
        assert_eq!(started["alreadyOpen"], json!(true));
    }

    #[test]
    fn save_and_list_refuse_unknown_fields_and_null_scope() {
        assert!(
            serde_json::from_value::<SaveMemoryRecordParams>(json!({})).is_err(),
            "body is required"
        );
        assert!(serde_json::from_value::<SaveMemoryRecordParams>(json!({
            "body": "pin",
            "scopeKey": "account:local"
        }))
        .is_err());
        assert!(serde_json::from_value::<SaveMemoryRecordParams>(json!({
            "body": "pin",
            "workspaceId": "w"
        }))
        .is_err());
        assert!(
            serde_json::from_value::<ListMemoryRecordsParams>(json!({})).is_err(),
            "scopeKey is required"
        );
        assert!(serde_json::from_value::<ListMemoryRecordsParams>(json!({
            "scopeKey": "account:local",
            "includeDeleted": true
        }))
        .is_err());
        assert!(serde_json::from_value::<SaveMemoryRecordParams>(json!({ "body": "pin" })).is_ok());
        assert!(serde_json::from_value::<ListMemoryRecordsParams>(json!({
            "scopeKey": "account:local"
        }))
        .is_ok());
    }
}

/// How the account scope cleans itself up, and what it holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRetentionSettings {
    pub scope_key: String,
    /// Days a proposal waits for review before it expires: 3, 7, 14 or 30.
    pub proposal_ttl_days: i64,
    /// Days without use before an active memory that is not a pin retires:
    /// 30, 90 or 180. Absent means never.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retire_unused_after_days: Option<i64>,
    /// Proposals the scope may hold at once, apart from the active budget.
    pub max_proposals: i64,
    /// The active budget (`max_records` on the consolidation settings).
    pub max_active: i64,
    pub held_active: i64,
    pub held_proposals: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetRetentionSettingsParams {
    pub proposal_ttl_days: i64,
    /// Absent or null: never retire unused memories.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retire_unused_after_days: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_proposals: Option<i64>,
}

/// Bring back a retired or expired memory as a new active record that
/// supersedes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestoreMemoryRecordParams {
    pub record_id: String,
}
