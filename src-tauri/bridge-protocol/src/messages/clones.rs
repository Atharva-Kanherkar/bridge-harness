//! The browser-clones domain: start a throwaway signed-in clone for a session,
//! read its state, take it over, hand it back, and destroy it. macOS-only in
//! the runtime; the wire methods exist on every platform and report "not
//! available" off macOS, so the generated contract is the same everywhere.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Which browser a clone runs, and whose cookie store an import reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CloneBrowserKind {
    Chrome,
    Brave,
}

/// How the clone gets signed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CloneSignInPath {
    /// Copy the approved site's cookies from the user's browser.
    Import,
    /// Read nothing; the person signs in inside the clone.
    SignInInside,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestCloneParams {
    pub session_id: String,
    pub domain: String,
    pub browser: CloneBrowserKind,
    pub sign_in_path: CloneSignInPath,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloneStateParams {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TakeoverCloneParams {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HandBackCloneParams {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DestroyCloneParams {
    pub session_id: String,
}

/// The person's answer to an agent's clone request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolveCloneRequestParams {
    pub session_id: String,
    pub allow: bool,
    pub request_id: String,
    pub sign_in_path: CloneSignInPath,
    pub ttl_minutes: u64,
}

/// What the dock shows about a session's clone. Never carries a cookie value.
/// The `screenshot` stays in the person’s dock; it is not sent to the agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CloneSnapshot {
    pub session_id: String,
    pub clone_id: String,
    pub domain: String,
    pub status: String,
    pub sign_in_path: CloneSignInPath,
    pub minutes_left: u64,
    pub screenshot: Option<String>,
    pub screenshot_redacted_regions: usize,
    /// Set when the agent has asked for a clone and is waiting on the person.
    /// The dock turns this into an Allow/Deny card. `None` once a clone exists.
    pub pending_request: Option<String>,
    pub pending_request_id: Option<String>,
    pub extension_path: Option<String>,
    pub additional_domains: Option<Vec<String>>,
}

/// The session's clone, or `None` when it has no clone (or off macOS).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct CloneStateResult(pub Option<CloneSnapshot>);

/// Input the person sends to a clone they have taken over. Coordinates are a
/// fraction of the viewport (0..1) so the dock's scaled frame maps onto the page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CloneInputEvent {
    Click {
        x: f64,
        y: f64,
    },
    Scroll {
        x: f64,
        y: f64,
        #[serde(rename = "deltaY")]
        delta_y: f64,
    },
    Type {
        text: String,
    },
    Key {
        key: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloneInputParams {
    pub session_id: String,
    pub input: CloneInputEvent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloneSettings {
    pub default_sign_in_path: CloneSignInPath,
    pub ttl_minutes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CloneSettingsSnapshot {
    pub connected: bool,
    pub settings: CloneSettings,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WriteCloneSettingsParams {
    pub settings: CloneSettings,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CloneRequest {
    pub session_id: String,
    pub request_id: String,
    pub domain: String,
    pub extension_path: Option<String>,
    pub additional_domains: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct CloneRequestsResult(pub Vec<CloneRequest>);
