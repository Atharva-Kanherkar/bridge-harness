//! Apps: the features Bridge shows through the launcher, the dock, the sidebar
//! pins and chat cards, each described by a manifest that core owns.
//!
//! A manifest says only what any app may declare: where it shows up, the
//! agent tools it brings, the events it records and what memory may keep from
//! them. Privileges a built-in has (its badge source, whether it can be
//! turned off, its prompt text) are wired in core, never read from a
//! manifest, so a community manifest cannot claim them.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The most apps a person can pin to the sidebar.
pub const MAX_APP_PINS: usize = 3;

/// Where an app came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AppSource {
    /// Ships with Bridge.
    Builtin,
    /// Installed by the person.
    Community,
    /// A source a newer daemon knows and this client does not.
    #[serde(other)]
    Other,
}

/// A place in Bridge an app can show up besides the launcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AppSlot {
    /// A full screen of its own.
    Screen,
    /// A pane in a chat's dock.
    Dock,
    /// A card inside the conversation.
    Card,
    /// A command in the launcher.
    Launcher,
    /// A slot a newer daemon knows and this client does not. Core refuses it
    /// in a manifest.
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppView {
    pub slot: AppSlot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// One agent tool an app brings, as named on the `bridge` MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppTool {
    pub name: String,
    /// The tool changes something outside the chat, so it runs only when the
    /// person allows it. A manifest can make a tool ask; it can never make a
    /// tool skip an approval that policy requires.
    pub asks_first: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppManifest {
    /// Kebab-case, at most 40 characters, unique among apps.
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub source: AppSource,
    pub views: Vec<AppView>,
    pub tools: Vec<AppTool>,
    /// Event types the app records, each prefixed with its namespace.
    pub events: Vec<String>,
    /// Plain-language lines that limit what memory may keep from the app's
    /// events. Empty means memory never reads them.
    pub memory: Vec<String>,
}

/// Something waiting on the person in an app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppBadge {
    pub count: u32,
}

/// An app as every client sees it: its manifest and its state on this Mac.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppSummary {
    pub manifest: AppManifest,
    pub enabled: bool,
    /// False for the apps Bridge needs to manage the others (the Store).
    pub can_disable: bool,
    /// The sidebar position, from 0, or null when the app is not pinned.
    pub pinned: Option<u32>,
    /// Null when the app has no badge, is off, or its count could not be read.
    pub badge: Option<AppBadge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListAppsResult {
    pub apps: Vec<AppSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetAppPinnedParams {
    pub id: String,
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetAppEnabledParams {
    pub id: String,
    pub enabled: bool,
}

// ── Personal apps ──────────────────────────────────────────────────────────
//
// An app the person's own harness built for one MCP server: a view, the tools
// it reads with and the tools it writes with. Bridge never holds the server's
// credential; the harness that owns the connection makes every call. The
// person locks each version, and the lock binds to the exact content they
// previewed (its draft id and content hash).

/// One locked version of a personal app, without its view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PersonalAppSummary {
    /// Kebab-case, at most 40 characters, never a built-in app's id.
    pub id: String,
    /// 1 for the first lock, one more for each lock that changed the content.
    pub version: u32,
    pub name: String,
    /// What the app is for, in the person's words. Agents read it to decide
    /// when to open the app.
    pub intent: String,
    /// The harness whose MCP connection the app uses.
    pub harness: String,
    /// The MCP server as that harness configures it (`claude.ai Slack`).
    pub server: String,
    /// Tools the view may call without asking. Each is a read by Bridge's
    /// read vocabulary.
    pub reads: Vec<String>,
    /// Tools the view may call only after the person approves the exact call.
    pub writes: Vec<String>,
    /// `sha256:` over the app's canonical content.
    pub content_hash: String,
    pub view_bytes: u32,
    pub locked_at: String,
}

/// One locked version of a personal app, with its view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PersonalApp {
    pub app: PersonalAppSummary,
    /// The HTML the app renders in Bridge's sandboxed frame.
    pub view_html: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListPersonalAppsResult {
    /// The latest version of each app, most recently locked first.
    pub apps: Vec<PersonalAppSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetPersonalAppParams {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LockPersonalAppParams {
    pub draft_id: String,
    /// The hash the person saw in the preview. A draft whose content no longer
    /// hashes to it is refused.
    pub content_hash: String,
    /// Lock even though this version can do more than the last one. Without
    /// it, a widening version is reported and nothing is stored.
    #[serde(default)]
    pub allow_wider: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LockPersonalAppResult {
    /// A new version was stored.
    #[serde(rename_all = "camelCase")]
    Locked { app: PersonalAppSummary },
    /// The latest version already has this content; nothing was stored.
    #[serde(rename_all = "camelCase")]
    Unchanged { app: PersonalAppSummary },
    /// This version can do more than the locked one. Nothing was stored; lock
    /// again with `allowWider` to accept it.
    #[serde(rename_all = "camelCase")]
    Widens {
        /// Tools that were not reads before and are now.
        added_reads: Vec<String>,
        /// Tools the app could not call before and now may, with approval.
        added_writes: Vec<String>,
        /// The app now uses another harness or server.
        target_changed: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeletePersonalAppParams {
    pub id: String,
}

/// A preview is bound to its originating chat and the exact reviewed content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetPersonalAppDraftParams {
    pub session_id: String,
    pub draft_id: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewPersonalAppToolParams {
    pub session_id: String,
    pub draft_id: String,
    pub content_hash: String,
    pub tool: String,
    pub arguments: serde_json::Value,
}

/// Trusted stored content for a draft card. Locked cards still preview writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PersonalAppDraft {
    pub draft_id: String,
    pub app_id: String,
    pub content_hash: String,
    pub locked: bool,
    pub name: String,
    pub intent: String,
    pub harness: String,
    pub server: String,
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    pub view_html: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_args: Option<serde_json::Value>,
}

/// A locked app opened in a chat, bound to that chat and the exact version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetPersonalAppOpenParams {
    pub session_id: String,
    pub open_id: String,
    pub content_hash: String,
}

/// Trusted stored content for an opened app's card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PersonalAppOpen {
    pub open_id: String,
    pub app_id: String,
    pub version: u32,
    pub content_hash: String,
    pub name: String,
    pub intent: String,
    pub harness: String,
    pub server: String,
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    pub view_html: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefill: Option<serde_json::Value>,
}

/// One tool call from an opened app's view. A write needs a second call that
/// carries the approval token the first one returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallPersonalAppToolParams {
    pub session_id: String,
    pub open_id: String,
    pub content_hash: String,
    pub tool: String,
    pub arguments: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PersonalAppCallResult {
    /// The tool ran; its MCP result.
    #[serde(rename_all = "camelCase")]
    Result { result: serde_json::Value },
    /// A write that waits for the person. `arguments_json` is the exact
    /// canonical JSON that approving `token` sends, byte for byte.
    #[serde(rename_all = "camelCase")]
    ApprovalRequired { token: String, harness: String, server: String, tool: String, arguments_json: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn round_trip<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(value: T) {
        let text = serde_json::to_string(&value).unwrap();
        assert_eq!(serde_json::from_str::<T>(&text).unwrap(), value);
    }

    fn manifest() -> AppManifest {
        AppManifest {
            id: "outgoing".into(),
            name: "Outgoing".into(),
            description: "Messages a chat drafts for you".into(),
            version: "1.0.0".into(),
            source: AppSource::Builtin,
            views: vec![AppView { slot: AppSlot::Card, title: None }],
            tools: vec![AppTool { name: "outgoing_draft".into(), asks_first: false }],
            events: vec!["outgoing.sent".into()],
            memory: vec!["the people and channels the person writes to".into()],
        }
    }

    #[test]
    fn every_apps_type_round_trips() {
        round_trip(manifest());
        let summary = AppSummary { manifest: manifest(), enabled: true, can_disable: true, pinned: Some(0), badge: Some(AppBadge { count: 2 }) };
        round_trip(summary.clone());
        round_trip(ListAppsResult { apps: vec![summary] });
        round_trip(SetAppPinnedParams { id: "notes".into(), pinned: true });
        round_trip(SetAppEnabledParams { id: "notes".into(), enabled: false });
    }

    #[test]
    fn wire_names_are_camel_case() {
        let summary = AppSummary { manifest: manifest(), enabled: false, can_disable: true, pinned: None, badge: None };
        let value = serde_json::to_value(&summary).unwrap();
        assert_eq!(value["canDisable"], true);
        assert_eq!(value["pinned"], serde_json::Value::Null);
        assert_eq!(value["manifest"]["tools"][0]["asksFirst"], false);
        assert_eq!(value["manifest"]["views"][0], json!({"slot": "card"}));
    }

    fn personal() -> PersonalAppSummary {
        PersonalAppSummary {
            id: "slack-dm".into(),
            version: 2,
            name: "Slack DM".into(),
            intent: "message a teammate on Slack".into(),
            harness: "claude".into(),
            server: "claude.ai Slack".into(),
            reads: vec!["slack_search_users".into()],
            writes: vec!["slack_send_message".into()],
            content_hash: "sha256:00".into(),
            view_bytes: 12,
            locked_at: "2026-11-16T00:00:00Z".into(),
        }
    }

    #[test]
    fn personal_app_types_round_trip_in_camel_case() {
        round_trip(PersonalApp { app: personal(), view_html: "<p>hi</p>".into() });
        round_trip(ListPersonalAppsResult { apps: vec![personal()] });
        round_trip(GetPersonalAppParams { id: "slack-dm".into() });
        round_trip(DeletePersonalAppParams { id: "slack-dm".into() });
        round_trip(LockPersonalAppParams { draft_id: "d".into(), content_hash: "sha256:00".into(), allow_wider: true });
        for result in [
            LockPersonalAppResult::Locked { app: personal() },
            LockPersonalAppResult::Unchanged { app: personal() },
            LockPersonalAppResult::Widens { added_reads: vec!["a_read".into()], added_writes: vec![], target_changed: true },
        ] {
            round_trip(result);
        }
        let app = serde_json::to_value(PersonalApp { app: personal(), view_html: "<p>hi</p>".into() }).unwrap();
        assert_eq!(app["viewHtml"], "<p>hi</p>");
        assert_eq!(app["app"]["contentHash"], "sha256:00");
        assert_eq!(app["app"]["lockedAt"], "2026-11-16T00:00:00Z");
        let widens = serde_json::to_value(LockPersonalAppResult::Widens { added_reads: vec![], added_writes: vec!["x".into()], target_changed: false }).unwrap();
        assert_eq!(widens, json!({"kind": "widens", "addedReads": [], "addedWrites": ["x"], "targetChanged": false}));
        let params: LockPersonalAppParams = serde_json::from_value(json!({"draftId": "d", "contentHash": "h"})).unwrap();
        assert!(!params.allow_wider, "allowWider defaults to false");
    }

    #[test]
    fn personal_app_params_refuse_unknown_fields() {
        assert!(serde_json::from_value::<GetPersonalAppParams>(json!({"id": "a", "x": 1})).is_err());
        assert!(serde_json::from_value::<DeletePersonalAppParams>(json!({"id": "a", "x": 1})).is_err());
        assert!(serde_json::from_value::<LockPersonalAppParams>(json!({"draftId": "d", "contentHash": "h", "x": 1})).is_err());
    }

    #[test]
    fn open_and_call_params_round_trip_and_refuse_unknown_fields() {
        round_trip(GetPersonalAppOpenParams { session_id:"s".into(),open_id:"o".into(),content_hash:"h".into() });
        round_trip(CallPersonalAppToolParams { session_id:"s".into(),open_id:"o".into(),content_hash:"h".into(),tool:"send".into(),arguments:json!({"to":"Ada"}),approval_token:Some("t".into()) });
        assert!(serde_json::from_value::<CallPersonalAppToolParams>(json!({"sessionId":"s","openId":"o","contentHash":"h","tool":"send","arguments":{},"approved":true})).is_err());
        assert_eq!(serde_json::to_value(PersonalAppCallResult::ApprovalRequired { token:"t".into(),harness:"codex".into(),server:"s".into(),tool:"send".into(),arguments_json:"{}".into() }).unwrap()["kind"], "approvalRequired");
    }

    #[test]
    fn draft_preview_params_round_trip_and_refuse_unknown_fields() {
        round_trip(GetPersonalAppDraftParams { session_id:"s".into(),draft_id:"d".into(),content_hash:"h".into() });
        round_trip(PreviewPersonalAppToolParams { session_id:"s".into(),draft_id:"d".into(),content_hash:"h".into(),tool:"get_user".into(),arguments:json!({"id":1}) });
        assert!(serde_json::from_value::<GetPersonalAppDraftParams>(json!({"sessionId":"s","draftId":"d","contentHash":"h","html":"untrusted"})).is_err());
        assert!(serde_json::from_value::<PreviewPersonalAppToolParams>(json!({"sessionId":"s","draftId":"d","contentHash":"h","tool":"get_user","arguments":{},"allowWrite":true})).is_err());
    }

    #[test]
    fn unknown_sources_and_slots_decode_as_other() {
        assert_eq!(serde_json::from_value::<AppSource>(json!("signed")).unwrap(), AppSource::Other);
        assert_eq!(serde_json::from_value::<AppSlot>(json!("statusBar")).unwrap(), AppSlot::Other);
    }

    #[test]
    fn params_refuse_unknown_fields() {
        assert!(serde_json::from_value::<SetAppPinnedParams>(json!({"id": "notes", "pinned": true, "x": 1})).is_err());
        assert!(serde_json::from_value::<SetAppEnabledParams>(json!({"id": "notes", "enabled": true, "x": 1})).is_err());
    }
}
