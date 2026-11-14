//! Trails: one page per piece of work.
//!
//! A trail groups the chats, notes, pull requests and drafted messages that
//! belong to one piece of work. It is a read model over records Bridge already
//! keeps: membership rows point at them, the timeline is derived on read, and
//! removing a member or archiving a trail changes none of the records.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The longest trail title, in characters.
pub const MAX_TRAIL_TITLE_CHARS: usize = 120;

/// What kind of record a member points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum TrailMemberKind {
    /// A Bridge Notes page, by page id.
    Note,
    /// A chat, by session id.
    Session,
    /// A pull request attached to a chat, by its URL.
    PullRequest,
    /// A message a chat drafted, by outgoing id.
    Outgoing,
}

impl TrailMemberKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Session => "session",
            Self::PullRequest => "pullRequest",
            Self::Outgoing => "outgoing",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "note" => Some(Self::Note),
            "session" => Some(Self::Session),
            "pullRequest" => Some(Self::PullRequest),
            "outgoing" => Some(Self::Outgoing),
            _ => None,
        }
    }
}

/// A reference to one record: the session id, page id, pull request URL or
/// outgoing id, by `kind`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrailMemberRef {
    pub kind: TrailMemberKind,
    #[serde(rename = "ref")]
    pub reference: String,
}

/// Who put a member on its trail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum TrailAddedBy {
    Person,
    /// Followed a hard link from another member (a PR attached in a member
    /// chat, a fork of one, ...). Announced once, with an Undo.
    Auto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrailMember {
    pub kind: TrailMemberKind,
    #[serde(rename = "ref")]
    pub reference: String,
    pub added_by: TrailAddedBy,
    pub added_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum TrailState {
    /// Something waits on the person: a failing check, an unsent draft, a
    /// chat asking for input.
    NeedsYou,
    InProgress,
    /// Every pull request on the trail merged or closed, at least one merged.
    Shipped,
}

/// How far the work has come: 0 nothing yet, 1 plan (a note), 2 chats, 3 a
/// pull request, 4 shipped. The highest reached wins.
pub type TrailStage = u8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrailHarnessCount {
    pub harness: String,
    pub chats: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrailLastEvent {
    pub title: String,
    pub at: String,
}

/// One trail as a list row shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrailSummary {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    pub state: TrailState,
    /// Why the trail needs the person, when it does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_you_reason: Option<String>,
    pub stage: TrailStage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event: Option<TrailLastEvent>,
    pub harnesses: Vec<TrailHarnessCount>,
    /// Chats on the trail, for a chat's "Trail" chip.
    pub session_ids: Vec<String>,
    pub member_count: u32,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum TrailEventKind {
    Note,
    Chat,
    PullRequest,
    /// A pull request's checks, as of the last read.
    Checks,
    Message,
    /// A member whose record no longer exists.
    Removed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum TrailTone {
    Plain,
    Success,
    Warning,
    Danger,
}

/// One row of a trail's timeline. Derived on read from the member's own
/// record; nothing here is stored on the trail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrailEvent {
    /// Stable within a read: `<kind>:<ref>` plus a suffix for a second event
    /// of the same member.
    pub id: String,
    pub kind: TrailEventKind,
    pub at: String,
    pub title: String,
    /// The secondary line: harness and model, check counts, a draft's state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// A quoted excerpt, for a message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub tone: TrailTone,
    pub member: TrailMemberRef,
    pub added_by: TrailAddedBy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum TrailOpenKind {
    FailingCheck,
    UnsentDraft,
    ChatWaiting,
}

/// Something on the trail that waits on the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrailOpenItem {
    pub kind: TrailOpenKind,
    pub label: String,
    pub member: TrailMemberRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrailDetail {
    pub summary: TrailSummary,
    pub members: Vec<TrailMember>,
    /// Oldest first.
    pub events: Vec<TrailEvent>,
    pub open: Vec<TrailOpenItem>,
    /// The newest pull request's head branch, for the page's branch chip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

/// A member the automatic sweep added, as `trails/changed` reports it once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrailMemberAdded {
    pub trail_id: String,
    pub trail_title: String,
    pub member: TrailMemberRef,
    /// What the toast names: `PR #12`, `Chat “Fix login”`, `Note “Plan”`.
    pub label: String,
}

/// One thing a new chat on the trail can start with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrailBriefItem {
    /// `note:<id>`, `pr:<url>`, `chat:<id>`, `message:<id>`.
    pub id: String,
    pub label: String,
    /// `ceil(bytes / 4)` of the exact text a carry writes.
    pub tokens: u32,
    pub default_on: bool,
}

// --- methods -----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListTrailsParams {
    #[serde(default)]
    pub include_archived: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListTrailsResult {
    /// Needs you first, then in progress, then shipped; newest first within each.
    pub trails: Vec<TrailSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadTrailParams {
    pub trail_id: String,
    /// Re-read member pull requests from GitHub before deriving.
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReadTrailResult {
    pub trail: TrailDetail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateTrailParams {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// A chat to put on the new trail first; its project becomes the trail's
    /// when `workspaceId` is not given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateTrailResult {
    pub trail: TrailSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateTrailParams {
    pub trail_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTrailResult {
    pub trail: TrailSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AddTrailMemberParams {
    pub trail_id: String,
    pub member: TrailMemberRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddTrailMemberResult {
    pub trail: TrailSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoveTrailMemberParams {
    pub trail_id: String,
    pub member: TrailMemberRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RemoveTrailMemberResult {
    pub trail: TrailSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewTrailBriefParams {
    pub trail_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PreviewTrailBriefResult {
    pub items: Vec<TrailBriefItem>,
    /// The default-on items' tokens.
    pub total_tokens: u32,
    /// What a new chat on the default window can carry; a carry over the
    /// target chat's own budget is refused.
    pub budget_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CarryTrailBriefParams {
    pub trail_id: String,
    /// A new chat, before its first message.
    pub session_id: String,
    /// Brief item ids from `preview_trail_brief`.
    pub items: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CarryTrailBriefResult {
    /// Entries written into the chat's history.
    pub carried: u32,
    pub trail: TrailSummary,
}

/// The `trails/changed` notification: refetch hint, plus what the automatic
/// sweep added (empty for a plain change).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrailsChanged {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trail_id: Option<String>,
    #[serde(default)]
    pub added: Vec<TrailMemberAdded>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn member_refs_are_camel_case_on_the_wire() {
        let member = TrailMemberRef { kind: TrailMemberKind::PullRequest, reference: "https://github.com/o/r/pull/3".into() };
        assert_eq!(serde_json::to_value(&member).unwrap(), json!({"kind": "pullRequest", "ref": "https://github.com/o/r/pull/3"}));
        assert!(serde_json::from_value::<TrailMemberRef>(json!({"kind": "note", "ref": "p", "extra": 1})).is_err());
        for kind in [TrailMemberKind::Note, TrailMemberKind::Session, TrailMemberKind::PullRequest, TrailMemberKind::Outgoing] {
            assert_eq!(serde_json::to_value(kind).unwrap(), json!(kind.as_str()));
            assert_eq!(TrailMemberKind::parse(kind.as_str()), Some(kind));
        }
    }

    #[test]
    fn events_round_trip_camel_case() {
        let event = TrailEvent {
            id: "session:s".into(),
            kind: TrailEventKind::PullRequest,
            at: "2026-11-14T00:00:00Z".into(),
            title: "#3 Fix".into(),
            detail: None,
            excerpt: None,
            harness: Some("codex".into()),
            url: None,
            tone: TrailTone::Danger,
            member: TrailMemberRef { kind: TrailMemberKind::Session, reference: "s".into() },
            added_by: TrailAddedBy::Auto,
        };
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["kind"], "pullRequest");
        assert_eq!(value["addedBy"], "auto");
        assert_eq!(value["member"]["ref"], "s");
        assert_eq!(serde_json::from_value::<TrailEvent>(value).unwrap(), event);
    }

    #[test]
    fn params_refuse_unknown_fields() {
        assert!(serde_json::from_value::<ListTrailsParams>(json!({"includeArchived": true, "x": 1})).is_err());
        assert!(serde_json::from_value::<ReadTrailParams>(json!({"trailId": "t", "x": 1})).is_err());
        assert!(serde_json::from_value::<CreateTrailParams>(json!({"title": "t", "x": 1})).is_err());
        assert!(serde_json::from_value::<UpdateTrailParams>(json!({"trailId": "t", "x": 1})).is_err());
        assert!(serde_json::from_value::<AddTrailMemberParams>(json!({"trailId": "t", "member": {"kind": "note", "ref": "p"}, "x": 1})).is_err());
        assert!(serde_json::from_value::<RemoveTrailMemberParams>(json!({"trailId": "t", "member": {"kind": "note", "ref": "p"}, "x": 1})).is_err());
        assert!(serde_json::from_value::<PreviewTrailBriefParams>(json!({"trailId": "t", "x": 1})).is_err());
        assert!(serde_json::from_value::<CarryTrailBriefParams>(json!({"trailId": "t", "sessionId": "s", "items": [], "x": 1})).is_err());
        let read: ReadTrailParams = serde_json::from_value(json!({"trailId": "t"})).unwrap();
        assert!(!read.refresh);
    }
}
