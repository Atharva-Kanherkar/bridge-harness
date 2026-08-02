//! JSON-RPC 2.0 envelope: requests, responses, notifications, request IDs,
//! and cancellation.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ErrorCode;

pub const JSONRPC_VERSION: &str = "2.0";

/// Cancellation is a notification, LSP-style: best-effort, and the cancelled
/// request still receives a response (its result, or `ErrorCode::Cancelled`).
pub const CANCEL_METHOD: &str = "$/cancel";

/// A request identifier. Clients choose; servers echo it back verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum RequestId {
    Number(i64),
    String(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RpcRequest {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    pub id: RequestId,
    /// A method from the registry, `protocol/handshake`, or `$/cancel`.
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RpcNotification {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// Exactly one of `result` / `error` is present; constructors uphold the
/// invariant and [`RpcResponse::is_well_formed`] checks a decoded value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RpcResponse {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    pub id: RequestId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RpcError {
    /// A stable code from the [`crate::error::ErrorCode`] registry.
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Parameters of the `$/cancel` notification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CancelParams {
    /// The id of the in-flight request to cancel.
    pub id: RequestId,
}

impl RpcRequest {
    pub fn new(id: RequestId, method: impl Into<String>, params: Option<Value>) -> Self {
        Self { jsonrpc: JSONRPC_VERSION.into(), id, method: method.into(), params }
    }
}

impl RpcNotification {
    pub fn new(method: impl Into<String>, params: Option<Value>) -> Self {
        Self { jsonrpc: JSONRPC_VERSION.into(), method: method.into(), params }
    }

    pub fn cancel(id: RequestId) -> Self {
        Self::new(
            CANCEL_METHOD,
            Some(serde_json::to_value(CancelParams { id }).expect("cancel params serialize")),
        )
    }
}

impl RpcResponse {
    pub fn result(id: RequestId, result: Value) -> Self {
        Self { jsonrpc: JSONRPC_VERSION.into(), id, result: Some(result), error: None }
    }

    pub fn error(id: RequestId, error: RpcError) -> Self {
        Self { jsonrpc: JSONRPC_VERSION.into(), id, result: None, error: Some(error) }
    }

    pub fn is_well_formed(&self) -> bool {
        self.jsonrpc == JSONRPC_VERSION && self.result.is_some() != self.error.is_some()
    }
}

impl RpcError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code: code.code(), message: message.into(), data: None }
    }

    pub fn with_data(code: ErrorCode, message: impl Into<String>, data: Value) -> Self {
        Self { code: code.code(), message: message.into(), data: Some(data) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn round_trip<T>(value: &T) -> T
    where
        T: Serialize + for<'de> Deserialize<'de>,
    {
        serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
    }

    #[test]
    fn requests_round_trip_with_both_id_kinds() {
        for id in [RequestId::Number(7), RequestId::String("turn-7".into())] {
            let request =
                RpcRequest::new(id, "sessions/send_turn", Some(json!({"sessionId":"s"})));
            assert_eq!(round_trip(&request), request);
        }
    }

    #[test]
    fn request_ids_keep_their_json_shape_on_the_wire() {
        let numeric = serde_json::to_value(RequestId::Number(7)).unwrap();
        assert_eq!(numeric, json!(7));
        let text = serde_json::to_value(RequestId::String("7".into())).unwrap();
        assert_eq!(text, json!("7"));
        assert_ne!(numeric, text, "numeric and string ids must stay distinguishable");
    }

    #[test]
    fn responses_carry_exactly_one_outcome() {
        let ok = RpcResponse::result(RequestId::Number(1), json!({"ok":true}));
        let failed = RpcResponse::error(
            RequestId::Number(2),
            RpcError::new(ErrorCode::Invalid, "message cannot be empty"),
        );
        assert!(ok.is_well_formed());
        assert!(failed.is_well_formed());
        assert_eq!(round_trip(&ok), ok);
        assert_eq!(round_trip(&failed), failed);

        let malformed = RpcResponse {
            jsonrpc: JSONRPC_VERSION.into(),
            id: RequestId::Number(3),
            result: Some(json!(1)),
            error: Some(RpcError::new(ErrorCode::InternalError, "both")),
        };
        assert!(!malformed.is_well_formed());
    }

    #[test]
    fn omitted_optionals_stay_off_the_wire() {
        let request = RpcRequest::new(RequestId::Number(1), "health/health", None);
        let wire = serde_json::to_value(&request).unwrap();
        assert!(wire.get("params").is_none());
        let ok = RpcResponse::result(RequestId::Number(1), json!(null));
        let wire = serde_json::to_value(&ok).unwrap();
        assert!(wire.get("error").is_none());
    }

    #[test]
    fn cancellation_is_a_notification_wrapping_the_request_id() {
        let cancel = RpcNotification::cancel(RequestId::String("turn-9".into()));
        assert_eq!(cancel.method, CANCEL_METHOD);
        let params: CancelParams = serde_json::from_value(cancel.params.clone().unwrap()).unwrap();
        assert_eq!(params.id, RequestId::String("turn-9".into()));
        assert_eq!(round_trip(&cancel), cancel);
    }
}
