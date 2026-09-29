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

/// What the dock shows about a session's clone. Never carries a cookie value.
/// The `screenshot` is a redacted frame the surface renders directly.
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
}

/// The session's clone, or `None` when it has no clone (or off macOS).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct CloneStateResult(pub Option<CloneSnapshot>);
