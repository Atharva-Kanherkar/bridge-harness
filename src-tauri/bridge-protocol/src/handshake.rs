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
pub const PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion {
    major: 1,
    minor: 33,
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
    fn outgoing_send_states_require_new_clients_and_daemons() {
        for minor in 23..32 {
            let old = ProtocolVersion { major: 1, minor };
            assert!(!PROTOCOL_VERSION.accepts(old), "client {minor} cannot decode sent states");
            assert!(!old.accepts(PROTOCOL_VERSION));
        }
        assert!(PROTOCOL_VERSION.accepts(PROTOCOL_VERSION));
        assert!(ProtocolVersion { major: 1, minor: 33 }.accepts(PROTOCOL_VERSION));
    }

    #[test]
    fn a_trails_daemon_still_serves_a_client_without_trails() {
        let without_trails = ProtocolVersion { major: 1, minor: 32 };
        assert!(PROTOCOL_VERSION.accepts(without_trails), "a 1.32 client pairs and never shows trails");
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
