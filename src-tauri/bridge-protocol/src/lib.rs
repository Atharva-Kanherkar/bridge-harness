//! The versioned RPC contract every Bridge client speaks.
//!
//! Bridge is RPC-shaped: commands, approvals, cancellation, and bidirectional
//! events. This crate is the single source of truth for that contract — the
//! JSON-RPC 2.0 envelope, the method namespace, stable error codes, and the
//! protocol-version handshake. Hosts (the Tauri shell, the `bridged` daemon)
//! implement it; clients consume the generated JSON Schemas and TypeScript
//! types instead of hand-written mirrors.
//!
//! Wire format: JSON-RPC 2.0 semantics over a Unix-domain socket for local
//! clients, WebSocket for browser/remote clients, and a Tauri invoke/event
//! compatibility adapter during migration. See `docs/protocol/README.md`.

pub mod envelope;
pub mod error;
pub mod handshake;
pub mod methods;
pub mod tsgen;

pub use envelope::{
    CancelParams, InvalidParams, JsonRpcVersion, Params, RequestId, ResponseId, RpcError,
    RpcFailure, RpcNotification, RpcRequest, RpcResponse, RpcSuccess, CANCEL_METHOD,
    JSONRPC_VERSION, MAX_SAFE_INTEGER,
};
pub use error::ErrorCode;
pub use handshake::{
    negotiate, ClientInfo, HandshakeRequest, HandshakeResponse, ProtocolVersion, ServerInfo,
    HANDSHAKE_METHOD, PROTOCOL_VERSION,
};
pub use methods::MethodName;
