//! Protocol-version handshake. The first request on every connection must be
//! `protocol/handshake`; anything else is answered with
//! [`ErrorCode::InvalidRequest`]. The server advertises its capabilities (the
//! method domains it serves) so clients can feature-gate instead of sniffing
//! versions.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::envelope::RpcError;
use crate::error::ErrorCode;
use crate::methods::MethodName;

pub const HANDSHAKE_METHOD: &str = "protocol/handshake";

/// The protocol version this crate describes.
///
/// Versioning policy: a **major** bump means breaking changes (clients must
/// upgrade); a **minor** bump means additive changes (new methods,
/// notifications, or optional fields). Ordinarily a server accepts a client
/// when the majors match and the client's minor is not newer than the server's.
/// Protocol 1.17 is also a minimum-client boundary: an older client cannot
/// represent or disable a persisted automatic-memory setting, so a 1.17-or-
/// newer daemon must reject it rather than run invisible automatic behavior.
///
/// **1.0 is a breaking bump, not a stability claim.** Opening `HarnessId`
/// widened a value domain that appears in *results*: a session can now report
/// `acp:<agent>`, which a 0.8-generated client decodes as an out-of-set enum
/// value. Because `accepts` only compares majors and an upper minor bound,
/// keeping this at 0.9 would let such a client handshake successfully and then
/// fail decoding `state/get_state` — a silent success followed by a failure at
/// the worst possible moment. Widening a result domain is precisely the
/// "clients must upgrade" case this policy reserves the major for.
///
/// The alternative — serving 0.8-compatible responses per connection — was
/// rejected: the only ways to make an `acp:` session fit a 0.8 client are to
/// hide it or to rename it, and both break the guarantee that history never
/// vanishes and is never re-attributed.
///
/// **1.3 adds `config/save_permission_policy`.** The minor bump is what stops a
/// client that needs it from handshaking against a daemon that does not have it
/// and only finding out at `method_not_found` — the daemon outlives the app, so
/// that pairing is routine rather than exotic.
///
/// **1.4 adds the optional `warnings` array to `health/health`** (macOS
/// TCC-protected paths and ad-hoc signing). Defaulted on decode, so a 1.4
/// client still reads a 1.3 daemon's health — the bump only records that a
/// daemon serving 1.4 emits it.
///
/// **1.5 adds workspace branch listing and checkout.** A new desktop must not
/// accept an older daemon and discover the missing methods only after the user
/// opens the branch menu.
/// **1.6 adds exact chat creation identity** for concurrent CLI clients.
///
/// **1.7 adds worker prompt proposal grants and attributed prompt revisions.**
/// A new client must not pair with an older daemon that silently discards
/// `workerPromptProposalRoles` when saving the permission policy.
/// **1.8 adds persistent terminal workspaces, snapshots and sequenced frames.**
///
/// **1.13 integrates native Menu Bar usage, provider collection and preferences.**
/// Menu Bar previews independently used versions 1.8 through 1.12 without the
/// terminal workspace contract. The integrated client must reject both the
/// mainline 1.8 daemon and those preview daemons, rather than accepting a
/// numerically newer preview that is missing terminal methods.
/// **1.14 adds overview summary visibility and separate provider status items.**
/// **1.15 supports expandable favorites; only favorites appear in provider tabs.**
/// **1.16 adds dashboard history origins and nullable usage session counts.**
/// **1.17 enables the `auto_apply` memory extraction mode.** A new client must
/// not pair with an older daemon that still rejects that persisted setting,
/// and an older client must not pair with a daemon that may already hold it.
/// **1.18 adds `config/get_attribution_settings` and
/// `config/save_attribution_settings`.** A new client must not pair with an
/// older daemon that answers both with `method_not_found`, leaving the toggle
/// unable to load or persist; a 1.17 client still pairs with a 1.18 daemon,
/// which simply serves it without attribution methods.
/// **1.19 adds `sessions/search_chats`, `config/get_chat_search_settings` and
/// `config/save_chat_search_settings`.** A new client must not pair with an
/// older daemon, whose sidebar search would fail with `method_not_found` on
/// the first keystroke; a 1.18 client still pairs with a 1.19 daemon.
/// **1.20 adds `sessions/get_context_windows`.** A new client must not pair
/// with an older daemon, whose Context pane would fail with
/// `method_not_found`; a 1.19 client still pairs with a 1.20 daemon.
/// **1.21 adds draft-owned composer dictation, local setup and transcript events.**
/// Pre-voice clients and daemons are rejected to keep transient events compatible.
/// **1.22 adds `config/get_delegation_notify_settings` and
/// `config/save_delegation_notify_settings`.** A new client must not pair with
/// an older daemon, whose notification-level control would fail with
/// `method_not_found`; a 1.21 client still pairs with a 1.22 daemon.
/// **1.23 adds personal project defaults and enforced session-scoped access.**
/// Older clients cannot display that authority, so both directions require 1.23.
/// **1.24 adds `attention/list_attention`, `attention/mark_attention_seen` and
/// the `attention/changed` hint.** A new client must not pair with an older
/// daemon, whose ledger reads would fail with `method_not_found`; a 1.23
/// client still pairs with a 1.24 daemon.
/// **1.25 adds `sessions/deliver_queued_input` and the `escalation` attention
/// kind.** A 1.24 client still pairs with a 1.25 daemon: its `AttentionKind`
/// already carries the `other` fallback, so it decodes `escalation` as
/// `other` and shows nothing for it rather than failing the read.
/// **1.26 adds `attention/unread_summary`.** A new client must not pair with
/// an older daemon, whose unread dots and badge would fail with
/// `method_not_found`; a 1.25 client still pairs with a 1.26 daemon.
/// **1.27 adds `attention/report_presence`.** A new client must not pair with
/// an older daemon, whose presence reports would fail with `method_not_found`;
/// a 1.26 client still pairs with a 1.27 daemon, never reports, and keeps
/// posting its own banners.
/// **1.28 adds `canPostBanners` to `attention/report_presence`**, so a browser
/// tab can own banners. A 1.27 daemon refuses the field, so a new client
/// needs 1.28; a 1.27 client still pairs with a 1.28 daemon.
/// **1.29 adds `attention/post` and the `posted` attention kind.** A 1.28
/// client decodes `posted` as `other` and shows nothing for it, so it still
/// pairs; a new client needs 1.29 for the method.
/// **1.30 adds `sessions/wait`.** A new client must not pair with an older
/// daemon, whose waits would fail with `method_not_found`; a 1.29 client
/// still pairs with a 1.30 daemon.
/// **1.31 adds the `outgoing` domain and its `outgoing/changed` hint.** A new
/// client must not pair with an older daemon, whose draft cards would fail
/// with `method_not_found`; a 1.30 client still pairs with a 1.31 daemon and
/// simply never shows a draft card.
/// **1.32 adds revision-bound outgoing sends and remote receipts.** New clients
/// require this method before offering Send on a draft card. It is also a
/// minimum-client boundary: older clients cannot decode the new persisted
/// sending/sent/failed states, so both sides must upgrade together.
/// **1.33 adds the `trails` domain and its `trails/changed` hint.** A new
/// client must not pair with an older daemon, whose Trails screen would fail
/// with `method_not_found`; a 1.32 client still pairs with a 1.33 daemon and
/// simply never shows trails.
/// **1.34 adds `attention/mark_attention_unread`.** A new client must not
/// pair with an older daemon, whose Mark as unread would fail with
/// `method_not_found`; a 1.33 client still pairs with a 1.34 daemon.
/// **1.35 adds GitHub issues to trails** (`issue` member and event kinds). A
/// 1.33 or 1.34 client reads trails but cannot draw an `issue` member, so a
/// 1.35 daemon refuses those two; a 1.32 client never reads trails and still
/// pairs.
/// **1.36 adds the `apps` domain and its `apps/changed` hint.** A new client
/// must not pair with an older daemon, whose launcher, pins and Store would
/// fail with `method_not_found`; a 1.35 client still pairs with a 1.36 daemon
/// and keeps its own local app list.
/// **1.37 adds `trails/list_trail_candidates`.** A new client must not pair
/// with an older daemon, whose Add picker on a trail would fail with
/// `method_not_found`; a 1.36 client still pairs with a 1.37 daemon.
///
/// **1.38 adds personal apps** (`apps/list_personal_apps`,
/// `apps/get_personal_app`, `apps/lock_personal_app`,
/// `apps/delete_personal_app`). A new client must not pair with an older
/// daemon, whose personal app calls would fail with `method_not_found`; a 1.37
/// client still pairs with a 1.38 daemon.
/// **1.39 adds chat-bound personal app draft previews** and read-only MCP calls.
///
/// **1.40 adds `sessions/record_client_latency`**, the webview's marks for one
/// send. A new client must not pair with an older daemon, which would answer
/// it with `method_not_found`; a 1.39 client still pairs with a 1.40 daemon.
///
/// **1.41 adds opened personal apps**: `apps/get_personal_app_open` and
/// `apps/call_personal_app_tool`, whose writes wait for an approval token.
///
/// **1.42 adds `allowOpencodeAppCourier` to app preferences.** A 1.42 client
/// saving it to a 1.41 daemon would be refused as an unknown field.
///
/// **1.43 adds "My apps": `apps/open_personal_app_surface` and
/// `apps/close_personal_app_surface`, `personal` in `apps/list_apps`, and an
/// optional `sessionId` on opened-app calls (none means the person's own app
/// surface).** A 1.42 client still pairs: it always sends a session id.
///
/// **1.44 adds the background import**: `imports/start_import`,
/// `imports/import_status` and the `imports/progress` event.
///
/// **1.45 adds continuing an imported chat**: `imports/import_continuation`
/// and `imports/continue_imported_chat`.
///
/// **1.46 adds the tools of a chat**: `sessions/get_chat_tools`,
/// `sessions/update_chat_tools` and an optional `toolSelection` on
/// `sessions/create_configured_chat`. A 1.45 client still pairs: it never
/// sends the field.
///
/// **1.47 adds Screen Time**: `usage/record_presence` and
/// `usage/screen_time`. A 1.46 client still pairs: it never calls them.
///
/// **1.48 adds binary terminal input**: an optional `binary` on
/// `terminal/write_terminal`, for xterm's X10 mouse reports. A 1.48 client
/// always sends the field, so it must not pair with a 1.47 daemon, which
/// would refuse it as unknown on every keystroke; a 1.47 client still pairs
/// with a 1.48 daemon.
///
/// **1.49 adds chat-scoped changes**: `workspaces/session_changes`. A 1.48
/// client still pairs: it never calls it.
///
/// **1.50 adds the reviews domain**: `reviews/*` methods and the
/// `reviews/job_updated` notification. Review job settings are their own
/// record, so `ReviewerSettings` keeps its 1.49 shape. A 1.49 client still
/// pairs: it never calls `reviews/*`, and every response it can ask for
/// decodes as before.
///
/// **1.51 adds two lean reads for the app**: `state/get_app_state` (the
/// `state/get_state` document without Bridge's own background runs) and
/// `workspaces/refresh_workspace_stats` (one workspace's Git counts instead of
/// the whole state). `state/get_state` answers exactly as before. Results that
/// carry the app state (`sessions/start_chat`, `workspaces/refresh_workspace`
/// and the other mutations) now leave out the background runs too; no client
/// shows them. A 1.50 client still pairs: it never calls the new methods.
///
/// **1.52 adds the early compaction switch**:
/// `config/get_early_compaction_settings` and
/// `config/save_early_compaction_settings`. A 1.51 client still pairs: it
/// never calls them.
///
/// **1.53 adds review triage and posting**: `reviews/decide_review`,
/// `reviews/post_review`, and `ReviewFinding.decidedAt`. A 1.52 client still
/// pairs: it never calls them, and the new field is optional.
///
/// **1.54 adds event workflows**: the `event` workflow trigger (in summaries,
/// definitions and runs), queue and connector fields on workflow summaries and
/// settings, `viewing` on `connectors/connector_inbox`, and
/// `workflows/preview_workflow_trigger`, `workflows/list_workflow_templates`
/// and `workflows/refresh_workflow_connectors`. A 1.54 daemon refuses older
/// clients: they cannot read an event trigger, and would fail on the first
/// workflow list that holds one. A 1.54 client cannot pair with a 1.53 daemon,
/// which lacks the new methods.
///
/// **1.55 adds Gmail, Linear and Notion event triggers**: three
/// `WorkflowEventKind` values. A 1.55 daemon refuses older clients, which
/// cannot read a workflow that uses one; a 1.55 client cannot pair with a 1.54
/// daemon, which would refuse those triggers.
///
/// **1.56 adds the Reviews dashboard**: `reviews/review_overview`, the
/// optional `repoOwner`, `repoName`, `prTitle` and `prAuthor` on `ReviewJob`,
/// and `summary` on `ReviewJobDetail`. A 1.55 client still pairs: it never
/// calls the new method, and the new fields are optional.
///
/// **1.57 pages memory lists**: `memory/list_memory_records` takes optional
/// `cursor` and `limit` and returns `total` and `nextCursor`, and
/// `memory/review_memory_records` approves and rejects a batch. Additive: a
/// 1.57 daemon serves older clients, which read the first page as before; a
/// 1.57 client cannot pair with a 1.56 daemon, which lacks the batch method.
///
/// **1.58 adds reviewer activity**: `reviews/review_activity` and the
/// `reviews/activity` notification. A 1.57 client still pairs: it never calls
/// the method and ignores the notification.
///
/// **1.59 adds one memory model**: `memory/get_memory_model` and
/// `memory/set_memory_model`. The `harness` and `model` fields on the
/// extraction and consolidation settings become read-only mirrors of it.
/// Additive: a 1.59 daemon serves older clients; a 1.59 client cannot pair
/// with a 1.58 daemon, which lacks the new methods.
///
/// **1.60 adds memory retention**: `memory/get_retention_settings`,
/// `memory/set_retention_settings`, `memory/restore_memory_record`, and
/// `expired` as a listable status. Additive: a 1.60 daemon serves older
/// clients; a 1.60 client cannot pair with a 1.59 daemon.
///
/// **1.61 breaks Screen Time down**: `usage/record_presence` takes optional
/// `chats` (which chat was open each minute) and `usage/screen_time` answers
/// `projects`, `pullRequests`, `hours` and `harnesses`. Additive: a 1.61
/// daemon serves older clients; a 1.61 client cannot pair with a 1.60
/// daemon, which refuses the `chats` field.
pub const PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion {
    major: 1,
    minor: 61,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolVersion {
    pub major: u32,
    pub minor: u32,
}

impl ProtocolVersion {
    /// Whether a server at `self` can serve a client that expects `client`.
    pub fn accepts(self, client: ProtocolVersion) -> bool {
        const AUTOMATIC_MEMORY_MINOR: u32 = 17;
        const INDEPENDENT_VOICE_MINOR: u32 = 21;
        const SCOPED_CHAT_SETTINGS_MINOR: u32 = 23;
        const OUTGOING_SEND_MINOR: u32 = 32;
        const TRAILS_MINOR: u32 = 33;
        const TRAIL_ISSUES_MINOR: u32 = 35;
        const WORKFLOW_EVENTS_MINOR: u32 = 54;
        const MORE_TRIGGERS_MINOR: u32 = 55;
        self.major == client.major
            && client.minor <= self.minor
            && !(self.major == 1
                && self.minor >= AUTOMATIC_MEMORY_MINOR
                && client.minor < AUTOMATIC_MEMORY_MINOR)
            && !(self.major == 1
                && self.minor >= INDEPENDENT_VOICE_MINOR
                && client.minor < INDEPENDENT_VOICE_MINOR)
            && !(self.major == 1
                && self.minor >= SCOPED_CHAT_SETTINGS_MINOR
                && client.minor < SCOPED_CHAT_SETTINGS_MINOR)
            && !(self.major == 1
                && self.minor >= OUTGOING_SEND_MINOR
                && client.minor < OUTGOING_SEND_MINOR)
            && !(self.major == 1
                && self.minor >= TRAIL_ISSUES_MINOR
                && (TRAILS_MINOR..TRAIL_ISSUES_MINOR).contains(&client.minor))
            && !(self.major == 1
                && self.minor >= WORKFLOW_EVENTS_MINOR
                && client.minor < WORKFLOW_EVENTS_MINOR)
            && !(self.major == 1
                && self.minor >= MORE_TRIGGERS_MINOR
                && client.minor < MORE_TRIGGERS_MINOR)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HandshakeRequest {
    pub protocol_version: ProtocolVersion,
    pub client: ClientInfo,
    /// The per-install authentication token, required by hosts that serve
    /// remote-capable transports (the `bridged` daemon). Clients read it from
    /// the token file in the data directory; it never appears in any response.
    /// Version negotiation itself ignores it — enforcement is the host's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HandshakeResponse {
    pub protocol_version: ProtocolVersion,
    pub server: ServerInfo,
    /// The method domains this server serves (sorted, deduplicated).
    pub capabilities: Vec<String>,
    /// A content-addressed identity of the server's own executable, so a
    /// launcher holding a newer binary can tell it is talking to a stale
    /// daemon. `ServerInfo.version` cannot: it is the workspace version, a
    /// constant on every dev build. Optional — a host that has none (tests,
    /// the embedded host, older daemons) omits it, and a launcher treats the
    /// absence as stale when it has a binary of its own to offer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_id: Option<String>,
}

/// Negotiate a connection. Rejection uses the stable
/// [`ErrorCode::IncompatibleProtocol`] code and carries both versions in
/// `data` so clients can render a precise upgrade prompt.
pub fn negotiate(request: &HandshakeRequest) -> Result<HandshakeResponse, RpcError> {
    if !PROTOCOL_VERSION.accepts(request.protocol_version) {
        return Err(RpcError::with_data(
            ErrorCode::IncompatibleProtocol,
            format!(
                "client {} speaks protocol {}.{}, server speaks {}.{}",
                request.client.name,
                request.protocol_version.major,
                request.protocol_version.minor,
                PROTOCOL_VERSION.major,
                PROTOCOL_VERSION.minor,
            ),
            json!({
                "clientProtocolVersion": request.protocol_version,
                "serverProtocolVersion": PROTOCOL_VERSION,
            }),
        ));
    }
    Ok(HandshakeResponse {
        protocol_version: PROTOCOL_VERSION,
        server: ServerInfo {
            name: "bridge".into(),
            // Inherited from [workspace.package], so this is always the
            // application version.
            version: env!("CARGO_PKG_VERSION").into(),
        },
        capabilities: MethodName::domains()
            .iter()
            .map(|domain| (*domain).into())
            .collect(),
        // The host fills this in: negotiation is about the protocol, and only
        // the serving process knows which binary it is.
        build_id: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_bar_client_rejects_daemon_without_usage_overview() {
        let older = ProtocolVersion { major: 1, minor: 8 };
        assert!(!older.accepts(PROTOCOL_VERSION));
    }

    #[test]
    fn menu_layout_client_rejects_daemon_without_customization() {
        let older = ProtocolVersion { major: 1, minor: 9 };
        assert!(!older.accepts(PROTOCOL_VERSION));
    }

    #[test]
    fn manual_refresh_client_rejects_daemon_without_interaction_boundary() {
        assert!(!ProtocolVersion {
            major: 1,
            minor: 10
        }
        .accepts(PROTOCOL_VERSION));
    }

    #[test]
    fn favorites_client_rejects_daemon_that_cannot_persist_provider_order() {
        assert!(!ProtocolVersion {
            major: 1,
            minor: 11
        }
        .accepts(PROTOCOL_VERSION));
    }

    #[test]
    fn integrated_client_rejects_both_pre_menu_and_pre_terminal_daemons() {
        for older in [
            ProtocolVersion { major: 1, minor: 8 },
            ProtocolVersion {
                major: 1,
                minor: 12,
            },
        ] {
            assert!(!older.accepts(PROTOCOL_VERSION));
        }
    }

    #[test]
    fn menu_presentation_client_rejects_daemon_without_new_settings() {
        assert!(!ProtocolVersion {
            major: 1,
            minor: 13
        }
        .accepts(PROTOCOL_VERSION));
        assert!(!ProtocolVersion {
            major: 1,
            minor: 14
        }
        .accepts(PROTOCOL_VERSION));
    }

    #[test]
    fn dashboard_history_client_rejects_a_daemon_without_account_history() {
        let older = ProtocolVersion {
            major: 1,
            minor: 15,
        };
        assert!(!older.accepts(PROTOCOL_VERSION));
    }

    #[test]
    fn automatic_memory_is_a_bidirectional_compatibility_boundary() {
        let before_automatic_memory = ProtocolVersion {
            major: 1,
            minor: 16,
        };
        assert!(!before_automatic_memory.accepts(PROTOCOL_VERSION));
        assert!(!PROTOCOL_VERSION.accepts(before_automatic_memory));
    }

    #[test]
    fn independent_voice_rejects_stale_clients_and_daemons() {
        for minor in [17, 18, 19, 20] {
            let older = ProtocolVersion { major: 1, minor };
            assert!(!older.accepts(PROTOCOL_VERSION));
            assert!(!PROTOCOL_VERSION.accepts(older));
        }
    }

    #[test]
    fn attribution_client_rejects_daemon_without_attribution_settings() {
        // A 1.18 client must not pair with a 1.17 daemon: both attribution
        // calls would fail with `method_not_found` only when the toggle is
        // used. A 1.17 client still pairs with a 1.18 daemon, which serves it
        // without attribution methods.
        let before_attribution = ProtocolVersion { major: 1, minor: 17 };
        assert!(!before_attribution.accepts(PROTOCOL_VERSION));
        assert!(
            ProtocolVersion { major: 1, minor: 18 }.accepts(before_attribution),
            "attribution is additive: a 1.17 client still pairs with a 1.18 daemon"
        );
        assert!(
            ProtocolVersion { major: 1, minor: 18 }.accepts(before_attribution),
            "the 1.17 minimum-client boundary still holds on a 1.18 daemon"
        );
    }

    #[test]
    fn chat_search_client_rejects_daemon_without_search_chats() {
        let before_search = ProtocolVersion { major: 1, minor: 18 };
        assert!(!before_search.accepts(PROTOCOL_VERSION));
        assert!(
            ProtocolVersion { major: 1, minor: 19 }.accepts(before_search),
            "chat search is additive: a 1.18 client still pairs with a 1.19 daemon"
        );
        assert!(ProtocolVersion { major: 1, minor: 19 }.accepts(before_search));
    }

    #[test]
    fn context_windows_client_rejects_daemon_without_the_method() {
        let before = ProtocolVersion { major: 1, minor: 19 };
        assert!(!before.accepts(PROTOCOL_VERSION));
        assert!(
            ProtocolVersion { major: 1, minor: 20 }.accepts(before),
            "context windows are additive: a 1.19 client still pairs with a 1.20 daemon"
        );
        assert!(ProtocolVersion { major: 1, minor: 20 }.accepts(before));
    }

    #[test]
    fn unread_summary_client_rejects_a_daemon_without_the_method() {
        // A 1.26 client reads `attention/unread_summary` for its dots and
        // badge, which a 1.25 daemon answers with `method_not_found`. The
        // method is additive, so a 1.25 client still pairs with a newer daemon.
        let before_unread = ProtocolVersion { major: 1, minor: 25 };
        let unread = ProtocolVersion { major: 1, minor: 26 };
        assert!(!before_unread.accepts(unread));
        assert!(unread.accepts(before_unread), "a 1.25 client still pairs with a 1.26 daemon");
        assert!(
            !PROTOCOL_VERSION.accepts(before_unread),
            "the outgoing send boundary intentionally requires a 1.32 client"
        );
    }

    #[test]
    fn presence_client_rejects_a_daemon_without_the_lease() {
        let before = ProtocolVersion { major: 1, minor: 26 };
        let presence = ProtocolVersion { major: 1, minor: 27 };
        assert!(!before.accepts(presence));
        assert!(presence.accepts(before), "a 1.26 client still pairs with a 1.27 daemon");
        assert!(!PROTOCOL_VERSION.accepts(before), "the outgoing send boundary requires 1.32");
    }

    #[test]
    fn web_banner_client_rejects_a_daemon_without_the_capability() {
        let before = ProtocolVersion { major: 1, minor: 27 };
        let web_banners = ProtocolVersion { major: 1, minor: 28 };
        assert!(!before.accepts(web_banners));
        assert!(web_banners.accepts(before), "a 1.27 client still pairs with a 1.28 daemon");
        assert!(!PROTOCOL_VERSION.accepts(before), "the outgoing send boundary requires 1.32");
    }

    #[test]
    fn post_client_rejects_a_daemon_without_the_method() {
        let before = ProtocolVersion { major: 1, minor: 28 };
        let post = ProtocolVersion { major: 1, minor: 29 };
        assert!(!before.accepts(post));
        assert!(post.accepts(before), "a 1.28 client still pairs with a 1.29 daemon");
        assert!(!PROTOCOL_VERSION.accepts(before), "the outgoing send boundary requires 1.32");
    }

    #[test]
    fn wait_client_rejects_a_daemon_without_the_method() {
        let before = ProtocolVersion { major: 1, minor: 29 };
        let wait = ProtocolVersion { major: 1, minor: 30 };
        assert!(!before.accepts(wait));
        assert!(wait.accepts(before), "a 1.29 client still pairs with a 1.30 daemon");
        assert!(!PROTOCOL_VERSION.accepts(before), "the outgoing send boundary requires 1.32");
    }

    #[test]
    fn trail_issues_require_new_clients_and_daemons() {
        let before = ProtocolVersion { major: 1, minor: 34 };
        let issues = ProtocolVersion { major: 1, minor: 35 };
        assert!(!before.accepts(issues), "a 1.35 client needs the issue kinds");
        assert!(!issues.accepts(before), "a 1.34 client cannot draw an issue member");
        assert!(!issues.accepts(ProtocolVersion { major: 1, minor: 33 }), "nor can the first trails client");
        assert!(issues.accepts(ProtocolVersion { major: 1, minor: 32 }), "a client without trails never meets an issue");
        assert!(issues.accepts(issues));
    }

    #[test]
    fn continue_imported_client_rejects_a_daemon_without_it() {
        let before = ProtocolVersion { major: 1, minor: 44 };
        let continuing = ProtocolVersion { major: 1, minor: 45 };
        assert!(!before.accepts(continuing));
        assert!(continuing.accepts(before), "a 1.44 client still pairs with a 1.45 daemon");
    }

    #[test]
    fn a_1_45_client_pairs_with_a_1_46_daemon() {
        let before = ProtocolVersion { major: 1, minor: 45 };
        let tools = ProtocolVersion { major: 1, minor: 46 };
        assert!(tools.accepts(before), "a 1.45 client still pairs with a 1.46 daemon");
        // ...and with every daemon up to 1.52; 1.53 refuses it (event workflows).
        assert!(ProtocolVersion { major: 1, minor: 52 }.accepts(before));
        assert!(!before.accepts(tools), "a 1.46 client needs the tool methods");
    }

    #[test]
    fn a_1_46_client_pairs_with_a_1_47_daemon() {
        let before = ProtocolVersion { major: 1, minor: 46 };
        let screen_time = ProtocolVersion { major: 1, minor: 47 };
        assert!(screen_time.accepts(before), "a 1.46 client still pairs with a 1.47 daemon");
        assert!(!before.accepts(screen_time), "a 1.47 client needs the Screen Time methods");
    }

    #[test]
    fn a_binary_input_client_rejects_a_daemon_that_refuses_the_field() {
        let before = ProtocolVersion { major: 1, minor: 47 };
        let binary = ProtocolVersion { major: 1, minor: 48 };
        assert!(binary.accepts(before), "a 1.47 client still pairs with a 1.48 daemon");
        assert!(!before.accepts(binary), "a 1.48 client sends `binary` on every terminal write");
    }

    #[test]
    fn a_1_48_client_pairs_with_a_chat_changes_daemon() {
        let before = ProtocolVersion { major: 1, minor: 48 };
        let chat_changes = ProtocolVersion { major: 1, minor: 49 };
        assert!(chat_changes.accepts(before), "a 1.48 client never calls session_changes");
        assert!(!before.accepts(chat_changes), "a 1.49 client needs session_changes");
    }

    #[test]
    fn a_1_49_client_pairs_with_a_reviews_daemon() {
        let before = ProtocolVersion { major: 1, minor: 49 };
        let reviews = ProtocolVersion { major: 1, minor: 50 };
        assert!(reviews.accepts(before), "a 1.49 client never calls reviews/*");
        assert!(!before.accepts(reviews), "a 1.50 client needs the reviews domain");
    }

    #[test]
    fn a_1_50_client_pairs_with_a_lean_state_daemon() {
        let before = ProtocolVersion { major: 1, minor: 50 };
        let lean = ProtocolVersion { major: 1, minor: 51 };
        assert!(lean.accepts(before), "a 1.50 client never calls get_app_state or refresh_workspace_stats");
        assert!(!before.accepts(lean), "a 1.51 app reads its state through get_app_state");
    }

    #[test]
    fn a_1_51_client_pairs_with_an_early_compaction_daemon() {
        let before = ProtocolVersion { major: 1, minor: 51 };
        let early = ProtocolVersion { major: 1, minor: 52 };
        assert!(!before.accepts(early), "a 1.52 client needs the switch methods");
        assert!(early.accepts(before), "a 1.51 client still pairs with a 1.52 daemon");
    }

    #[test]
    fn event_workflows_pair_only_with_a_matching_daemon_and_client() {
        let before = ProtocolVersion { major: 1, minor: 53 };
        let events = ProtocolVersion { major: 1, minor: 54 };
        assert!(!events.accepts(before), "a 1.53 client cannot read an event trigger in a workflow list");
        assert!(!before.accepts(events), "a 1.54 client needs the preview, template and refresh methods");
        assert!(events.accepts(events));
        assert!(!PROTOCOL_VERSION.accepts(ProtocolVersion { major: 1, minor: 17 }), "older clients stay refused");
    }

    #[test]
    fn more_triggers_pair_only_with_a_matching_daemon_and_client() {
        let before = ProtocolVersion { major: 1, minor: 54 };
        let more = ProtocolVersion { major: 1, minor: 55 };
        assert!(!more.accepts(before), "a 1.54 client cannot read a Gmail, Linear or Notion trigger");
        assert!(!before.accepts(more), "a 1.55 client's new triggers would be refused by a 1.54 daemon");
        assert!(more.accepts(more));
    }

    #[test]
    fn a_1_57_client_pairs_with_a_reviewer_activity_daemon() {
        let before = ProtocolVersion { major: 1, minor: 57 };
        let activity = ProtocolVersion { major: 1, minor: 58 };
        assert!(activity.accepts(before), "a 1.57 client never asks for activity");
        assert!(!before.accepts(activity), "a 1.58 client needs reviews/review_activity");
    }

    #[test]
    fn a_1_55_client_pairs_with_a_reviews_dashboard_daemon() {
        let before = ProtocolVersion { major: 1, minor: 55 };
        let dashboard = ProtocolVersion { major: 1, minor: 56 };
        assert!(dashboard.accepts(before), "a 1.55 client never asks for the overview");
        assert!(!before.accepts(dashboard), "a 1.56 client needs reviews/review_overview");
    }

    #[test]
    fn memory_paging_is_additive_for_older_clients() {
        let before = ProtocolVersion { major: 1, minor: 56 };
        let paging = ProtocolVersion { major: 1, minor: 57 };
        assert!(paging.accepts(before), "a 1.56 client reads the first page as before");
        assert!(!before.accepts(paging), "a 1.57 client needs memory/review_memory_records");
        assert!(paging.accepts(paging));
    }

    #[test]
    fn one_memory_model_is_additive_for_older_clients() {
        let before = ProtocolVersion { major: 1, minor: 58 };
        let memory_model = ProtocolVersion { major: 1, minor: 59 };
        assert!(memory_model.accepts(before), "a 1.58 client never asks for the memory model");
        assert!(!before.accepts(memory_model), "a 1.59 client needs memory/get_memory_model");
    }

    #[test]
    fn memory_retention_is_additive_for_older_clients() {
        let before = ProtocolVersion { major: 1, minor: 59 };
        let retention = ProtocolVersion { major: 1, minor: 60 };
        assert!(retention.accepts(before), "a 1.59 client never asks for retention settings");
        assert!(!before.accepts(retention), "a 1.60 client needs memory/get_retention_settings");
    }

    #[test]
    fn a_1_60_client_pairs_with_a_screen_time_breakdown_daemon() {
        let before = ProtocolVersion { major: 1, minor: 60 };
        let breakdown = ProtocolVersion { major: 1, minor: 61 };
        assert_eq!(PROTOCOL_VERSION, breakdown);
        assert!(breakdown.accepts(before), "a 1.60 client never sends chats and ignores the new result fields");
        assert!(!before.accepts(breakdown), "a 1.61 client sends chats, which a 1.60 daemon refuses");
    }

    #[test]
    fn a_1_52_client_pairs_with_a_daemon_that_posts_reviews() {
        let before = ProtocolVersion { major: 1, minor: 52 };
        let posting = ProtocolVersion { major: 1, minor: 53 };
        assert!(posting.accepts(before), "a 1.52 client never decides or posts a review");
        assert!(!before.accepts(posting));
    }

    #[test]
    fn background_import_client_rejects_a_daemon_without_it() {
        let before = ProtocolVersion { major: 1, minor: 43 };
        let import = ProtocolVersion { major: 1, minor: 44 };
        assert!(!before.accepts(import));
        assert!(import.accepts(before), "a 1.43 client still pairs with a 1.44 daemon");
    }

    #[test]
    fn my_apps_client_rejects_a_daemon_without_surfaces() {
        let before = ProtocolVersion { major: 1, minor: 42 };
        let surfaces = ProtocolVersion { major: 1, minor: 43 };
        assert!(!before.accepts(surfaces));
        assert!(surfaces.accepts(before), "a 1.42 client still pairs with a 1.43 daemon");
    }

    #[test]
    fn opencode_courier_preference_client_rejects_an_older_daemon() {
        let before = ProtocolVersion { major: 1, minor: 41 };
        let flag = ProtocolVersion { major: 1, minor: 42 };
        assert!(!before.accepts(flag));
        assert!(flag.accepts(before), "a 1.41 client still pairs with a 1.42 daemon");
    }

    #[test]
    fn opened_apps_client_rejects_a_daemon_without_them() {
        let before = ProtocolVersion { major: 1, minor: 40 };
        let opened = ProtocolVersion { major: 1, minor: 41 };
        assert!(!before.accepts(opened));
        assert!(opened.accepts(before), "a 1.40 client still pairs with a 1.41 daemon");
    }

    #[test]
    fn client_latency_client_rejects_a_daemon_without_the_method() {
        let before = ProtocolVersion { major: 1, minor: 39 };
        let latency = ProtocolVersion { major: 1, minor: 40 };
        assert!(!before.accepts(latency));
        assert!(latency.accepts(before), "a 1.39 client still pairs with a 1.40 daemon");
    }

    #[test]
    fn personal_apps_client_rejects_a_daemon_without_them() {
        let before = ProtocolVersion { major: 1, minor: 37 };
        let personal = ProtocolVersion { major: 1, minor: 38 };
        assert!(!before.accepts(personal));
        assert!(personal.accepts(before), "a 1.37 client still pairs with a 1.38 daemon");
    }

    #[test]
    fn trail_candidates_client_rejects_a_daemon_without_the_method() {
        let before = ProtocolVersion { major: 1, minor: 36 };
        let candidates = ProtocolVersion { major: 1, minor: 37 };
        assert!(!before.accepts(candidates));
        assert!(candidates.accepts(before), "a 1.36 client still pairs with a 1.37 daemon");
    }

    #[test]
    fn mark_unread_client_rejects_a_daemon_without_the_method() {
        let before = ProtocolVersion { major: 1, minor: 33 };
        let unread = ProtocolVersion { major: 1, minor: 34 };
        assert!(!before.accepts(unread));
        assert!(unread.accepts(before), "a 1.33 client still pairs with a 1.34 daemon");
    }

    #[test]
    fn apps_client_rejects_a_daemon_without_the_domain() {
        let before = ProtocolVersion { major: 1, minor: 35 };
        let apps = ProtocolVersion { major: 1, minor: 36 };
        assert!(!before.accepts(apps));
        assert!(apps.accepts(before), "a 1.35 client still pairs with a 1.36 daemon");
    }

    #[test]
    fn outgoing_send_states_require_new_clients_and_daemons() {
        for minor in 23..32 {
            let old = ProtocolVersion { major: 1, minor };
            assert!(!PROTOCOL_VERSION.accepts(old), "client {minor} cannot decode sent states");
            assert!(!old.accepts(PROTOCOL_VERSION));
        }
        assert!(PROTOCOL_VERSION.accepts(PROTOCOL_VERSION));
        assert!(ProtocolVersion { major: 1, minor: PROTOCOL_VERSION.minor + 1 }.accepts(PROTOCOL_VERSION));
    }

    #[test]
    fn a_trails_daemon_still_serves_a_client_without_trails() {
        let without_trails = ProtocolVersion { major: 1, minor: 32 };
        // The last daemon that served it; 1.53 refuses it (event workflows).
        let last = ProtocolVersion { major: 1, minor: 52 };
        assert!(last.accepts(without_trails), "a 1.32 client pairs and never shows trails");
        assert!(!without_trails.accepts(PROTOCOL_VERSION), "a trails client needs a daemon that serves trails");
    }

    fn request(major: u32, minor: u32) -> HandshakeRequest {
        HandshakeRequest {
            protocol_version: ProtocolVersion { major, minor },
            client: ClientInfo {
                name: "test-client".into(),
                version: "1.2.3".into(),
            },
            auth_token: None,
        }
    }

    #[test]
    fn compatible_client_receives_server_identity_and_capabilities() {
        let response = negotiate(&request(PROTOCOL_VERSION.major, PROTOCOL_VERSION.minor)).unwrap();
        assert_eq!(response.protocol_version, PROTOCOL_VERSION);
        assert_eq!(response.server.name, "bridge");
        assert_eq!(response.server.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(response.capabilities, MethodName::domains());
    }

    #[test]
    fn incompatible_client_is_rejected_with_the_stable_code() {
        for incompatible in [
            request(PROTOCOL_VERSION.major + 1, 0),
            request(PROTOCOL_VERSION.major, PROTOCOL_VERSION.minor + 1),
            request(PROTOCOL_VERSION.major, 16),
        ] {
            let error = negotiate(&incompatible).unwrap_err();
            assert_eq!(error.code, ErrorCode::IncompatibleProtocol.code());
            assert_eq!(error.code, 2000, "rejection code is part of the contract");
            let data = error.data.unwrap();
            assert_eq!(
                data["serverProtocolVersion"],
                serde_json::to_value(PROTOCOL_VERSION).unwrap()
            );
            assert!(data["clientProtocolVersion"].is_object());
        }
    }

    #[test]
    fn persistent_terminal_clients_reject_daemons_without_snapshot_recovery() {
        let before_terminal_history = ProtocolVersion { major: 1, minor: 7 };
        assert!(!before_terminal_history.accepts(PROTOCOL_VERSION));
    }

    #[test]
    fn prompt_mutation_clients_cannot_pair_with_daemons_that_discard_worker_grants() {
        let daemon_before_worker_grants = ProtocolVersion { major: 1, minor: 6 };
        assert!(
            !daemon_before_worker_grants.accepts(PROTOCOL_VERSION),
            "a stale daemon would silently discard workerPromptProposalRoles on save"
        );
    }

    #[test]
    fn protocol_0_clients_are_refused_rather_than_served_values_they_cannot_decode() {
        // The regression this major bump exists for: a 0.8 client's generated
        // `HarnessId` is a closed enum over the four built-ins. A session can
        // now report `acp:<agent>`, reachable through `sessions/create_chat`
        // with no installer, so such a client must be turned away at the
        // handshake rather than failing later inside `state/get_state`.
        for stale in [request(0, 9), request(0, 8), request(0, 0)] {
            let error = negotiate(&stale).unwrap_err();
            assert_eq!(error.code, ErrorCode::IncompatibleProtocol.code());
            let data = error.data.unwrap();
            assert_eq!(
                data["serverProtocolVersion"],
                serde_json::to_value(PROTOCOL_VERSION).unwrap(),
                "the rejection tells the client what to upgrade to"
            );
        }
        assert!(
            !PROTOCOL_VERSION.accepts(ProtocolVersion { major: 0, minor: 9 }),
            "opening a result value domain is a breaking change"
        );
    }

    #[test]
    fn handshake_shapes_round_trip() {
        let request = request(PROTOCOL_VERSION.major, PROTOCOL_VERSION.minor);
        let encoded = serde_json::to_string(&request).unwrap();
        assert_eq!(
            serde_json::from_str::<HandshakeRequest>(&encoded).unwrap(),
            request
        );
        assert!(
            encoded.contains("protocolVersion"),
            "wire fields are camelCase"
        );
        assert!(
            !encoded.contains("authToken"),
            "an absent token stays off the wire"
        );
        let response = negotiate(&request).unwrap();
        let encoded = serde_json::to_string(&response).unwrap();
        assert_eq!(
            serde_json::from_str::<HandshakeResponse>(&encoded).unwrap(),
            response
        );
    }

    #[test]
    fn the_auth_token_rides_the_request_and_never_the_response() {
        let mut authenticated = request(PROTOCOL_VERSION.major, PROTOCOL_VERSION.minor);
        authenticated.auth_token = Some("secret-token".into());
        let encoded = serde_json::to_string(&authenticated).unwrap();
        assert!(encoded.contains("\"authToken\":\"secret-token\""));
        assert_eq!(
            serde_json::from_str::<HandshakeRequest>(&encoded).unwrap(),
            authenticated
        );
        // Negotiation ignores the token entirely — hosts enforce it — and no
        // response field can ever echo it.
        let response = negotiate(&authenticated).unwrap();
        assert!(!serde_json::to_string(&response)
            .unwrap()
            .contains("secret-token"));
    }
}
