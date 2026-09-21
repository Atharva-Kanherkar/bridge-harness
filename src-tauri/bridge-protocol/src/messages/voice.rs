//! Composer dictation over a provider-owned speech transport.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Encoding Bridge currently accepts from the webview capture pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VoiceAudioEncoding {
    PcmS16Le,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VoiceProviderCapability {
    pub provider: String,
    pub available: bool,
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
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VoiceCapabilitiesResult {
    pub session_id: String,
    pub providers: Vec<VoiceProviderCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_provider: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VoiceStartParams {
    pub session_id: String,
    pub provider: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VoiceStartResult {
    pub voice_session_id: String,
    pub session_id: String,
    pub provider: String,
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
    pub session_id: String,
    pub provider: String,
    pub kind: VoiceTranscriptKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
