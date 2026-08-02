//! JSON-RPC 2.0 envelope: requests, responses, notifications, request IDs,
//! and cancellation.
//!
//! The types here enforce the JSON-RPC 2.0 spec at the type level:
//! `jsonrpc` only ever holds the literal `"2.0"`, `params` is structured
//! (object or array) whenever present, numeric ids are constrained to
//! JavaScript-safe integers so they echo verbatim across every client, a
//! response is *either* a success or a failure (never both, never neither —
//! and a `null` result is a valid success), and response ids admit `null`
//! for the parse-error case where the request id could not be recovered.

use schemars::gen::SchemaGenerator;
use schemars::schema::Schema;
use schemars::JsonSchema;
use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;

use crate::error::ErrorCode;

pub const JSONRPC_VERSION: &str = "2.0";

/// Cancellation is a notification, LSP-style: best-effort, and the cancelled
/// request still receives a response (its result, or `ErrorCode::Cancelled`).
pub const CANCEL_METHOD: &str = "$/cancel";

/// The largest integer JavaScript can represent exactly
/// (`Number.MAX_SAFE_INTEGER`). Numeric request ids beyond this range would
/// silently change value in a browser client, so the wire rejects them.
pub const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

/// The literal `"2.0"`. Deserialization rejects anything else, so a decoded
/// envelope is always a JSON-RPC 2.0 envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JsonRpcVersion;

impl Serialize for JsonRpcVersion {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(JSONRPC_VERSION)
    }
}

impl<'de> Deserialize<'de> for JsonRpcVersion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let version = String::deserialize(deserializer)?;
        if version == JSONRPC_VERSION {
            Ok(JsonRpcVersion)
        } else {
            Err(de::Error::custom(format!(
                "unsupported JSON-RPC version {version:?}; expected \"2.0\""
            )))
        }
    }
}

impl JsonSchema for JsonRpcVersion {
    fn schema_name() -> String {
        "JsonRpcVersion".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        serde_json::from_value(serde_json::json!({
            "type": "string",
            "const": JSONRPC_VERSION,
        }))
        .unwrap()
    }
}

/// A request identifier. Clients choose; servers echo it back verbatim.
/// Numeric ids are limited to JavaScript-safe integers (|id| ≤ 2^53 − 1) so
/// "verbatim" holds for every client.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(i64),
    String(String),
}

impl<'de> Deserialize<'de> for RequestId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct IdVisitor;
        impl<'de> Visitor<'de> for IdVisitor {
            type Value = RequestId;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a string, or an integer within ±(2^53 - 1)")
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<RequestId, E> {
                if value.abs() > MAX_SAFE_INTEGER {
                    return Err(E::custom(format!(
                        "numeric request id {value} exceeds JavaScript's safe integer range"
                    )));
                }
                Ok(RequestId::Number(value))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<RequestId, E> {
                i64::try_from(value)
                    .ok()
                    .filter(|value| *value <= MAX_SAFE_INTEGER)
                    .map(RequestId::Number)
                    .ok_or_else(|| {
                        E::custom(format!(
                            "numeric request id {value} exceeds JavaScript's safe integer range"
                        ))
                    })
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<RequestId, E> {
                Ok(RequestId::String(value.to_owned()))
            }
            fn visit_string<E: de::Error>(self, value: String) -> Result<RequestId, E> {
                Ok(RequestId::String(value))
            }
        }
        deserializer.deserialize_any(IdVisitor)
    }
}

impl JsonSchema for RequestId {
    fn schema_name() -> String {
        "RequestId".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        serde_json::from_value(serde_json::json!({
            "description": "A request identifier. Clients choose; servers echo it back verbatim. Numeric ids must stay within JavaScript's safe integer range.",
            "anyOf": [
                { "type": "integer", "minimum": -MAX_SAFE_INTEGER, "maximum": MAX_SAFE_INTEGER },
                { "type": "string" },
            ],
        }))
        .unwrap()
    }
}

/// The id on a response: the request's id, or `null` when the request id
/// could not be recovered (parse errors, invalid requests) as JSON-RPC
/// requires.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ResponseId {
    Id(RequestId),
    Null,
}

impl From<RequestId> for ResponseId {
    fn from(id: RequestId) -> Self {
        ResponseId::Id(id)
    }
}

/// Structured parameters: JSON-RPC allows only an object or an array when
/// `params` is present.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Params(Value);

impl Params {
    pub fn new(value: Value) -> Result<Params, InvalidParams> {
        match value {
            Value::Object(_) | Value::Array(_) => Ok(Params(value)),
            other => Err(InvalidParams(other)),
        }
    }

    pub fn as_value(&self) -> &Value {
        &self.0
    }

    pub fn into_value(self) -> Value {
        self.0
    }
}

/// The rejected value, so callers can report what they were handed.
#[derive(Debug, Clone, PartialEq)]
pub struct InvalidParams(pub Value);

impl std::fmt::Display for InvalidParams {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(formatter, "params must be a JSON object or array, got {}", self.0)
    }
}

impl std::error::Error for InvalidParams {}

impl TryFrom<Value> for Params {
    type Error = InvalidParams;
    fn try_from(value: Value) -> Result<Params, InvalidParams> {
        Params::new(value)
    }
}

impl<'de> Deserialize<'de> for Params {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Params::new(value).map_err(de::Error::custom)
    }
}

impl JsonSchema for Params {
    fn schema_name() -> String {
        "Params".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        serde_json::from_value(serde_json::json!({
            "description": "Structured parameters: an object or an array, per JSON-RPC 2.0.",
            "type": ["object", "array"],
        }))
        .unwrap()
    }
}

/// The `params` property schema: a plain reference to [`Params`]. Without
/// this override, `Option<Params>` would publish `Params | null`, but the
/// wire rejects a literal `null` — the key must be omitted instead.
fn params_field_schema(generator: &mut SchemaGenerator) -> Schema {
    generator.subschema_for::<Params>()
}

/// Strict `params` field decoding: a present key must hold an object or an
/// array — a literal `null` is rejected rather than treated as omitted, so
/// the Rust behavior matches the published schema exactly.
fn deserialize_params<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Params>, D::Error> {
    let value = Value::deserialize(deserializer)?;
    if value.is_null() {
        return Err(de::Error::custom(
            "params must be a JSON object or array when present; omit the key instead of sending null",
        ));
    }
    Params::new(value).map(Some).map_err(de::Error::custom)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RpcRequest {
    pub jsonrpc: JsonRpcVersion,
    pub id: RequestId,
    /// A method from the registry, `protocol/handshake`, or `$/cancel`.
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "deserialize_params")]
    #[schemars(schema_with = "params_field_schema")]
    pub params: Option<Params>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RpcNotification {
    pub jsonrpc: JsonRpcVersion,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "deserialize_params")]
    #[schemars(schema_with = "params_field_schema")]
    pub params: Option<Params>,
}

/// A response is exactly one of success or failure. `deny_unknown_fields` on
/// the variants makes a payload carrying both `result` and `error` (or
/// neither) undecodable, and a `null` result is a decodable success because
/// variant selection keys on the *presence* of `result`, not its value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum RpcResponse {
    Success(RpcSuccess),
    Failure(RpcFailure),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RpcSuccess {
    pub jsonrpc: JsonRpcVersion,
    pub id: ResponseId,
    /// Present even when `null` — commands returning nothing succeed with
    /// `"result": null`.
    pub result: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RpcFailure {
    pub jsonrpc: JsonRpcVersion,
    pub id: ResponseId,
    pub error: RpcError,
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
    pub fn new(id: RequestId, method: impl Into<String>, params: Option<Params>) -> Self {
        Self { jsonrpc: JsonRpcVersion, id, method: method.into(), params }
    }
}

impl RpcNotification {
    pub fn new(method: impl Into<String>, params: Option<Params>) -> Self {
        Self { jsonrpc: JsonRpcVersion, method: method.into(), params }
    }

    pub fn cancel(id: RequestId) -> Self {
        let params = serde_json::to_value(CancelParams { id }).expect("cancel params serialize");
        Self::new(CANCEL_METHOD, Some(Params::new(params).expect("cancel params are an object")))
    }
}

impl RpcResponse {
    pub fn result(id: impl Into<ResponseId>, result: Value) -> Self {
        RpcResponse::Success(RpcSuccess { jsonrpc: JsonRpcVersion, id: id.into(), result })
    }

    pub fn error(id: impl Into<ResponseId>, error: RpcError) -> Self {
        RpcResponse::Failure(RpcFailure { jsonrpc: JsonRpcVersion, id: id.into(), error })
    }

    /// The response to a payload whose request id could not be recovered.
    pub fn parse_error(message: impl Into<String>) -> Self {
        Self::error(ResponseId::Null, RpcError::new(ErrorCode::ParseError, message))
    }

    pub fn id(&self) -> &ResponseId {
        match self {
            RpcResponse::Success(success) => &success.id,
            RpcResponse::Failure(failure) => &failure.id,
        }
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

    fn params(value: Value) -> Option<Params> {
        Some(Params::new(value).unwrap())
    }

    #[test]
    fn requests_round_trip_with_both_id_kinds() {
        for id in [RequestId::Number(7), RequestId::String("turn-7".into())] {
            let request =
                RpcRequest::new(id, "sessions/send_turn", params(json!({"sessionId":"s"})));
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
    fn numeric_ids_beyond_javascripts_safe_range_are_rejected() {
        assert_eq!(
            serde_json::from_value::<RequestId>(json!(MAX_SAFE_INTEGER)).unwrap(),
            RequestId::Number(MAX_SAFE_INTEGER)
        );
        assert_eq!(
            serde_json::from_value::<RequestId>(json!(-MAX_SAFE_INTEGER)).unwrap(),
            RequestId::Number(-MAX_SAFE_INTEGER)
        );
        // 2^53 parses as 2^53 - 1 wherever JavaScript touches it; a client
        // echoing it back would correlate the response with the wrong request.
        for unsafe_id in [json!(MAX_SAFE_INTEGER + 1), json!(-(MAX_SAFE_INTEGER + 1))] {
            let error = serde_json::from_value::<RequestId>(unsafe_id).unwrap_err();
            assert!(error.to_string().contains("safe integer"), "{error}");
        }
    }

    #[test]
    fn non_2_0_envelopes_are_rejected() {
        let request = json!({"jsonrpc":"1.0","id":1,"method":"health/health"});
        let error = serde_json::from_value::<RpcRequest>(request).unwrap_err();
        assert!(error.to_string().contains("unsupported JSON-RPC version"), "{error}");
        assert!(serde_json::from_value::<RpcResponse>(
            json!({"jsonrpc":"1.0","id":1,"result":null})
        )
        .is_err());
        let wire = serde_json::to_value(RpcNotification::new("x", None)).unwrap();
        assert_eq!(wire["jsonrpc"], json!("2.0"));
    }

    #[test]
    fn params_must_be_structured() {
        assert!(Params::new(json!({"a":1})).is_ok());
        assert!(Params::new(json!([1, 2])).is_ok());
        for invalid in [json!("text"), json!(3), json!(true), json!(null)] {
            assert!(Params::new(invalid.clone()).is_err());
            let request = json!({"jsonrpc":"2.0","id":1,"method":"m","params":invalid});
            assert!(
                serde_json::from_value::<RpcRequest>(request).is_err(),
                "params {invalid} must be rejected"
            );
        }
    }

    #[test]
    fn null_results_are_valid_successes_and_round_trip() {
        let response = RpcResponse::result(RequestId::Number(1), Value::Null);
        let wire = serde_json::to_value(&response).unwrap();
        assert_eq!(wire, json!({"jsonrpc":"2.0","id":1,"result":null}));
        assert_eq!(round_trip(&response), response);
    }

    #[test]
    fn responses_are_exactly_one_of_success_or_failure() {
        let ok = RpcResponse::result(RequestId::Number(1), json!({"ok":true}));
        let failed = RpcResponse::error(
            RequestId::Number(2),
            RpcError::new(ErrorCode::Invalid, "message cannot be empty"),
        );
        assert_eq!(round_trip(&ok), ok);
        assert_eq!(round_trip(&failed), failed);

        let both = json!({"jsonrpc":"2.0","id":3,"result":1,"error":{"code":1000,"message":"x"}});
        assert!(serde_json::from_value::<RpcResponse>(both).is_err());
        let neither = json!({"jsonrpc":"2.0","id":3});
        assert!(serde_json::from_value::<RpcResponse>(neither).is_err());
    }

    #[test]
    fn parse_errors_respond_with_a_null_id() {
        let response = RpcResponse::parse_error("payload was not JSON");
        let wire = serde_json::to_value(&response).unwrap();
        assert_eq!(wire["id"], Value::Null);
        assert_eq!(wire["error"]["code"], json!(ErrorCode::ParseError.code()));
        assert_eq!(round_trip(&response), response);
    }

    #[test]
    fn omitted_optionals_stay_off_the_wire() {
        let request = RpcRequest::new(RequestId::Number(1), "health/health", None);
        let wire = serde_json::to_value(&request).unwrap();
        assert!(wire.get("params").is_none());
        let ok = RpcResponse::result(RequestId::Number(1), json!({"ok":true}));
        let wire = serde_json::to_value(&ok).unwrap();
        assert!(wire.get("error").is_none());
    }

    #[test]
    fn cancellation_is_a_notification_wrapping_the_request_id() {
        let cancel = RpcNotification::cancel(RequestId::String("turn-9".into()));
        assert_eq!(cancel.method, CANCEL_METHOD);
        let params: CancelParams =
            serde_json::from_value(cancel.params.clone().unwrap().into_value()).unwrap();
        assert_eq!(params.id, RequestId::String("turn-9".into()));
        assert_eq!(round_trip(&cancel), cancel);
    }
}
