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
/// notifications, or optional fields). A server accepts a client when the
/// majors match and the client's minor is not newer than the server's.
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
pub const PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion { major: 1, minor: 1 };

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolVersion {
    pub major: u32,
    pub minor: u32,
}

impl ProtocolVersion {
    /// Whether a server at `self` can serve a client that expects `client`.
    pub fn accepts(self, client: ProtocolVersion) -> bool {
        self.major == client.major && client.minor <= self.minor
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
        capabilities: MethodName::domains().iter().map(|domain| (*domain).into()).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(major: u32, minor: u32) -> HandshakeRequest {
        HandshakeRequest {
            protocol_version: ProtocolVersion { major, minor },
            client: ClientInfo { name: "test-client".into(), version: "1.2.3".into() },
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
        let older_minor_is_fine = negotiate(&request(PROTOCOL_VERSION.major, 0));
        assert!(older_minor_is_fine.is_ok());
    }

    #[test]
    fn incompatible_client_is_rejected_with_the_stable_code() {
        for incompatible in [
            request(PROTOCOL_VERSION.major + 1, 0),
            request(PROTOCOL_VERSION.major, PROTOCOL_VERSION.minor + 1),
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
        let request = request(PROTOCOL_VERSION.major, 0);
        let encoded = serde_json::to_string(&request).unwrap();
        assert_eq!(serde_json::from_str::<HandshakeRequest>(&encoded).unwrap(), request);
        assert!(encoded.contains("protocolVersion"), "wire fields are camelCase");
        assert!(
            !encoded.contains("authToken"),
            "an absent token stays off the wire — pre-0.6 requests are still valid"
        );
        let response = negotiate(&request).unwrap();
        let encoded = serde_json::to_string(&response).unwrap();
        assert_eq!(serde_json::from_str::<HandshakeResponse>(&encoded).unwrap(), response);
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
        assert!(!serde_json::to_string(&response).unwrap().contains("secret-token"));
    }
}
