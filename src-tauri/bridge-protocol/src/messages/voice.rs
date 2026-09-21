//! Composer dictation belongs to a draft, independently of a coding session.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Encoding Bridge currently accepts from the webview capture pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VoiceAudioEncoding {
    PcmS16Le,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum VoiceProviderId {
    Local,
    Codex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum VoiceAvailability {
    Ready,
    NeedsSetup,
    Unsupported,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum VoiceRecoveryAction {
    Setup,
    Retry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum VoiceProcessingLocation {
    OnDevice,
    Remote,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VoiceProviderCapability {
    pub provider: VoiceProviderId,
    pub state: VoiceAvailability,
    pub processing: VoiceProcessingLocation,
    pub supported_locales: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_action: Option<VoiceRecoveryAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
    pub encoding: VoiceAudioEncoding,
    pub sample_rate: u32,
    pub channels: u16,
    pub max_chunk_bytes: u32,
    pub max_session_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VoiceCapabilitiesParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VoiceCapabilitiesResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub providers: Vec<VoiceProviderCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_provider: Option<VoiceProviderId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VoiceStartParams {
    pub owner_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub provider: VoiceProviderId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VoiceStartResult {
    pub voice_session_id: String,
    pub owner_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub provider: VoiceProviderId,
    pub encoding: VoiceAudioEncoding,
    pub sample_rate: u32,
    pub channels: u16,
    pub max_chunk_bytes: u32,
    pub max_session_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VoiceAppendParams {
    pub voice_session_id: String,
    pub sequence: u32,
    /// Base64-encoded bytes in the encoding negotiated by `voice/start`.
    pub data: String,
    pub samples_per_channel: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VoiceStopParams {
    pub voice_session_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VoiceCancelParams {
    pub voice_session_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum VoiceTranscriptKind {
    Started,
    /// A complete, revisable hypothesis, not an append-only delta.
    Partial,
    Delta,
    Final,
    Error,
    Closed,
}

/// Live-only dictation state. The ids make stale frames harmless after a chat
/// switch, retry, or cancellation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VoiceTranscriptEvent {
    pub voice_session_id: String,
    pub owner_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub provider: VoiceProviderId,
    pub kind: VoiceTranscriptKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_drafts_need_no_coding_session() {
        let probe: VoiceCapabilitiesParams = serde_json::from_str("{}").unwrap();
        assert!(probe.session_id.is_none());
        let start: VoiceStartParams = serde_json::from_value(serde_json::json!({
            "ownerKey": "draft:project:1", "provider": "local"
        }))
        .unwrap();
        assert!(start.session_id.is_none());
        assert_eq!(start.provider, VoiceProviderId::Local);
    }

    #[test]
    fn unknown_providers_and_missing_owners_are_rejected() {
        for value in [
            serde_json::json!({"ownerKey":"draft", "provider":"automatic"}),
            serde_json::json!({"provider":"local"}),
        ] {
            assert!(serde_json::from_value::<VoiceStartParams>(value).is_err());
        }
    }
}
