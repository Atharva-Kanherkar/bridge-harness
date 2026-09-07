//! Stable error codes. Codes are part of the contract: they never change
//! meaning and are never reused. Clients branch on `code`, not on message
//! text.
//!
//! Ranges:
//! - `-32700..=-32600` — JSON-RPC 2.0 reserved envelope errors.
//! - `1000..=1999` — runtime errors, mirroring `bridge_core::BridgeError`
//!   variant for variant so the host mapping is total and obvious.
//! - `2000..=2999` — protocol lifecycle errors.
//! - `3000..=3099` — managed agent runtime lifecycle. One code per condition a
//!   caller must be able to act on differently, so distinguishing them never
//!   requires matching on message text.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    // JSON-RPC 2.0 envelope errors.
    ParseError,
    InvalidRequest,
    MethodNotFound,
    InvalidParams,
    InternalError,
    // Runtime errors (one per BridgeError variant).
    Invalid,
    Git,
    Database,
    Io,
    Adapter,
    Pty,
    // Protocol lifecycle errors.
    IncompatibleProtocol,
    Unauthorized,
    Cancelled,
    ShuttingDown,
    Overloaded,
    // Managed agent runtime lifecycle.
    UnsupportedPlatform,
    IntegrityFailure,
    ExternalNotManaged,
    AgentBusy,
    CorruptReceipt,
    VendorPrerequisiteMissing,
    UninstallNotPermitted,
    UnknownAgent,
}

impl ErrorCode {
    pub const ALL: [ErrorCode; 24] = [
        ErrorCode::ParseError,
        ErrorCode::InvalidRequest,
        ErrorCode::MethodNotFound,
        ErrorCode::InvalidParams,
        ErrorCode::InternalError,
        ErrorCode::Invalid,
        ErrorCode::Git,
        ErrorCode::Database,
        ErrorCode::Io,
        ErrorCode::Adapter,
        ErrorCode::Pty,
        ErrorCode::IncompatibleProtocol,
        ErrorCode::Unauthorized,
        ErrorCode::Cancelled,
        ErrorCode::ShuttingDown,
        ErrorCode::Overloaded,
        ErrorCode::UnsupportedPlatform,
        ErrorCode::IntegrityFailure,
        ErrorCode::ExternalNotManaged,
        ErrorCode::AgentBusy,
        ErrorCode::CorruptReceipt,
        ErrorCode::VendorPrerequisiteMissing,
        ErrorCode::UninstallNotPermitted,
        ErrorCode::UnknownAgent,
    ];

    pub const fn code(self) -> i64 {
        match self {
            ErrorCode::ParseError => -32700,
            ErrorCode::InvalidRequest => -32600,
            ErrorCode::MethodNotFound => -32601,
            ErrorCode::InvalidParams => -32602,
            ErrorCode::InternalError => -32603,
            ErrorCode::Invalid => 1000,
            ErrorCode::Git => 1001,
            ErrorCode::Database => 1002,
            ErrorCode::Io => 1003,
            ErrorCode::Adapter => 1004,
            ErrorCode::Pty => 1005,
            ErrorCode::IncompatibleProtocol => 2000,
            ErrorCode::Unauthorized => 2001,
            ErrorCode::Cancelled => 2002,
            ErrorCode::ShuttingDown => 2003,
            ErrorCode::Overloaded => 2004,
            ErrorCode::UnsupportedPlatform => 3000,
            ErrorCode::IntegrityFailure => 3001,
            ErrorCode::ExternalNotManaged => 3002,
            ErrorCode::AgentBusy => 3003,
            ErrorCode::CorruptReceipt => 3004,
            ErrorCode::VendorPrerequisiteMissing => 3005,
            ErrorCode::UninstallNotPermitted => 3006,
            ErrorCode::UnknownAgent => 3007,
        }
    }

    pub fn from_code(code: i64) -> Option<ErrorCode> {
        ErrorCode::ALL.into_iter().find(|candidate| candidate.code() == code)
    }

    /// Stable machine-readable name, used in generated artifacts.
    pub const fn name(self) -> &'static str {
        match self {
            ErrorCode::ParseError => "parse_error",
            ErrorCode::InvalidRequest => "invalid_request",
            ErrorCode::MethodNotFound => "method_not_found",
            ErrorCode::InvalidParams => "invalid_params",
            ErrorCode::InternalError => "internal_error",
            ErrorCode::Invalid => "invalid",
            ErrorCode::Git => "git",
            ErrorCode::Database => "database",
            ErrorCode::Io => "io",
            ErrorCode::Adapter => "adapter",
            ErrorCode::Pty => "pty",
            ErrorCode::IncompatibleProtocol => "incompatible_protocol",
            ErrorCode::Unauthorized => "unauthorized",
            ErrorCode::Cancelled => "cancelled",
            ErrorCode::ShuttingDown => "shutting_down",
            ErrorCode::Overloaded => "overloaded",
            ErrorCode::UnsupportedPlatform => "unsupported_platform",
            ErrorCode::IntegrityFailure => "integrity_failure",
            ErrorCode::ExternalNotManaged => "external_not_managed",
            ErrorCode::AgentBusy => "agent_busy",
            ErrorCode::CorruptReceipt => "corrupt_receipt",
            ErrorCode::VendorPrerequisiteMissing => "vendor_prerequisite_missing",
            ErrorCode::UninstallNotPermitted => "uninstall_not_permitted",
            ErrorCode::UnknownAgent => "unknown_agent",
        }
    }

    /// Human-oriented summary of when the code is emitted.
    pub const fn summary(self) -> &'static str {
        match self {
            ErrorCode::ParseError => "The payload was not valid JSON",
            ErrorCode::InvalidRequest => "The payload was not a valid JSON-RPC request",
            ErrorCode::MethodNotFound => "The method is not in the registry",
            ErrorCode::InvalidParams => "Params did not match the method's schema",
            ErrorCode::InternalError => "The server failed outside the runtime's error domain",
            ErrorCode::Invalid => "The runtime rejected the input",
            ErrorCode::Git => "A Git operation failed",
            ErrorCode::Database => "A database operation failed",
            ErrorCode::Io => "An I/O operation failed",
            ErrorCode::Adapter => "A harness adapter failed",
            ErrorCode::Pty => "A terminal (PTY) operation failed",
            ErrorCode::IncompatibleProtocol => {
                "The client's protocol version is incompatible with this server"
            }
            ErrorCode::Unauthorized => "The connection has not authenticated",
            ErrorCode::Cancelled => "The request was cancelled via $/cancel",
            ErrorCode::ShuttingDown => "The server is shutting down and refused the request",
            ErrorCode::Overloaded => "The server is at capacity and refused the connection or request",
            ErrorCode::UnsupportedPlatform => "No vendor build exists for this operating system and architecture",
            ErrorCode::IntegrityFailure => "The fetched runtime did not match its pinned integrity digest",
            ErrorCode::ExternalNotManaged => "This runtime is user-managed; Bridge holds no receipt for it and will not remove it",
            ErrorCode::AgentBusy => "Another lifecycle operation or a running process is holding this agent",
            ErrorCode::CorruptReceipt => "The managed installation's receipt could not be read or does not describe it",
            ErrorCode::VendorPrerequisiteMissing => "The vendor reported a prerequisite of its own, such as a login or an API key",
            ErrorCode::UninstallNotPermitted => "This installation cannot be removed in its current state",
            ErrorCode::UnknownAgent => "No built-in integration has that agent id",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn codes_and_names_are_unique_and_round_trip() {
        let mut codes = HashSet::new();
        let mut names = HashSet::new();
        for entry in ErrorCode::ALL {
            assert!(codes.insert(entry.code()), "duplicate code {}", entry.code());
            assert!(names.insert(entry.name()), "duplicate name {}", entry.name());
            assert_eq!(ErrorCode::from_code(entry.code()), Some(entry));
        }
        assert_eq!(ErrorCode::from_code(9999), None);
    }

    #[test]
    fn runtime_code_values_are_pinned() {
        // The 1000-range is variant-for-variant with bridge_core::BridgeError.
        // The compiler-enforced guard is the exhaustive `From<&BridgeError>`
        // impl in bridge-core (this crate cannot depend on bridge-core); this
        // test only pins the numeric values as contract.
        let runtime: Vec<_> = ErrorCode::ALL
            .into_iter()
            .filter(|entry| (1000..2000).contains(&entry.code()))
            .collect();
        assert_eq!(
            runtime,
            [
                ErrorCode::Invalid,
                ErrorCode::Git,
                ErrorCode::Database,
                ErrorCode::Io,
                ErrorCode::Adapter,
                ErrorCode::Pty
            ]
        );
    }

    #[test]
    fn reserved_jsonrpc_codes_match_the_spec() {
        assert_eq!(ErrorCode::ParseError.code(), -32700);
        assert_eq!(ErrorCode::InvalidRequest.code(), -32600);
        assert_eq!(ErrorCode::MethodNotFound.code(), -32601);
        assert_eq!(ErrorCode::InvalidParams.code(), -32602);
        assert_eq!(ErrorCode::InternalError.code(), -32603);
    }
}
