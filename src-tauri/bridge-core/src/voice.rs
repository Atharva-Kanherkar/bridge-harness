//! Session-safe composer dictation.
//!
//! Local speech is independent of coding sessions. The legacy Codex transport
//! remains explicit and experimental; there is no cross-provider fallback.

pub mod local;
pub mod sherpa;

use crate::{events::CoreEvent, BridgeCore, BridgeError};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use bridge_protocol::messages as wire;
use rusqlite::{params, OptionalExtension};
use std::{collections::HashMap, sync::Mutex};
use uuid::Uuid;

pub const SAMPLE_RATE: u32 = 16_000;
pub const CHANNELS: u16 = 1;
pub const MAX_CHUNK_BYTES: u32 = 64 * 1024;
pub const MAX_SESSION_BYTES: u32 = 4 * 1024 * 1024;

#[derive(Debug, Clone)]
struct ActiveVoiceSession {
    id: String,
    owner_key: String,
    session_id: String,
    provider_thread_id: String,
    next_sequence: u32,
    total_bytes: u32,
    created_at: std::time::Instant,
}

#[derive(Default)]
pub struct VoiceService {
    sessions: Mutex<HashMap<String, ActiveVoiceSession>>,
    admission: Mutex<()>,
    pub local: local::LocalVoiceService,
    pub local_install: sherpa::InstallManager,
}

impl VoiceService {
    pub fn for_data_dir(data_dir: &std::path::Path) -> Self {
        Self {
            local: local::LocalVoiceService::new(sherpa::installed_provider(data_dir)),
            local_install: sherpa::InstallManager::new(data_dir.to_path_buf()),
            ..Self::default()
        }
    }
}

#[derive(Debug)]
struct SessionBinding {
    harness: String,
    status: String,
    active_turn_id: Option<String>,
    kind: String,
    provider_session_id: Option<String>,
}

fn binding(core: &BridgeCore, session_id: &str) -> Result<SessionBinding, BridgeError> {
    core.db
        .lock()
        .unwrap()
        .query_row(
            "SELECT harness,status,active_turn_id,kind,provider_session_id FROM sessions WHERE id=?1",
            params![session_id],
            |row| {
                Ok(SessionBinding {
                    harness: row.get(0)?,
                    status: row.get(1)?,
                    active_turn_id: row.get(2)?,
                    kind: row.get(3)?,
                    provider_session_id: row.get(4)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| BridgeError::Invalid(format!("Session {session_id} not found")))
}

fn capability(
    provider: wire::VoiceProviderId,
    state: wire::VoiceAvailability,
    unavailable_reason: Option<String>,
) -> wire::VoiceProviderCapability {
    wire::VoiceProviderCapability {
        provider,
        state,
        processing: match provider {
            wire::VoiceProviderId::Local => wire::VoiceProcessingLocation::OnDevice,
            wire::VoiceProviderId::Codex => wire::VoiceProcessingLocation::Remote,
        },
        supported_locales: vec![],
        recovery_action: match state {
            wire::VoiceAvailability::NeedsSetup => Some(wire::VoiceRecoveryAction::Setup),
            wire::VoiceAvailability::Failed => Some(wire::VoiceRecoveryAction::Retry),
            _ => None,
        },
        unavailable_reason,
        encoding: wire::VoiceAudioEncoding::PcmS16Le,
        sample_rate: SAMPLE_RATE,
        channels: CHANNELS,
        max_chunk_bytes: MAX_CHUNK_BYTES,
        max_session_bytes: MAX_SESSION_BYTES,
    }
}

pub fn capabilities(
    core: &BridgeCore,
    params: wire::VoiceCapabilitiesParams,
) -> Result<wire::VoiceCapabilitiesResult, BridgeError> {
    // Missing/stale coding sessions affect only the experimental provider.
    // The local probe must work before any session row or adapter exists.
    let codex = match params.session_id.as_deref() {
        Some(id) => match codex_capability(core, id) {
            Ok(capability) => capability,
            Err(_) => capability(
                wire::VoiceProviderId::Codex,
                wire::VoiceAvailability::Unsupported,
                Some("The coding session is not available for experimental Codex dictation".into()),
            ),
        },
        None => capability(
            wire::VoiceProviderId::Codex,
            wire::VoiceAvailability::Unsupported,
            Some("Experimental Codex dictation requires a live Codex chat".into()),
        ),
    };
    Ok(wire::VoiceCapabilitiesResult {
        session_id: params.session_id,
        // Probing cannot opt a user into a provider or an external account.
        selected_provider: None,
        providers: vec![core.voice.local.capability(), codex],
    })
}

fn codex_capability(
    core: &BridgeCore,
    session_id: &str,
) -> Result<wire::VoiceProviderCapability, BridgeError> {
    let session = binding(core, session_id)?;
    let reason = if session.harness != "codex" {
        Some("Voice dictation is currently available for Codex chats only".into())
    } else if session.kind != "direct" {
        Some("Voice dictation is available only in direct chats".into())
    } else if session.active_turn_id.is_some() || session.status == "working" {
        Some("Wait for the current turn to finish before dictating".into())
    } else {
        let adapters = core.adapters.lock().unwrap();
        match adapters.get(session_id) {
            Some(runtime) if runtime.supports_voice_dictation() => None,
            Some(_) => Some("This Codex runtime does not expose realtime dictation".into()),
            None => Some("Start the Codex chat before dictating".into()),
        }
    };
    Ok(capability(
        wire::VoiceProviderId::Codex,
        if reason.is_none() {
            wire::VoiceAvailability::Ready
        } else {
            wire::VoiceAvailability::Unsupported
        },
        reason,
    ))
}

fn start_result(id: String, params: wire::VoiceStartParams) -> wire::VoiceStartResult {
    wire::VoiceStartResult {
        voice_session_id: id,
        owner_key: params.owner_key,
        session_id: params.session_id,
        provider: params.provider,
        encoding: wire::VoiceAudioEncoding::PcmS16Le,
        sample_rate: SAMPLE_RATE,
        channels: CHANNELS,
        max_chunk_bytes: MAX_CHUNK_BYTES,
        max_session_bytes: MAX_SESSION_BYTES,
    }
}

pub fn start(
    core: &BridgeCore,
    params: wire::VoiceStartParams,
) -> Result<wire::VoiceStartResult, BridgeError> {
    if params.owner_key.trim().is_empty() || params.owner_key.len() > 256 {
        return Err(BridgeError::Invalid(
            "Voice draft owner must contain 1 to 256 bytes".into(),
        ));
    }
    let _admission = core.voice.admission.lock().unwrap();
    // Preserve the legacy transport's opportunistic expiry until its separate
    // active-watchdog/correlated-close rework lands. Local takes expire actively.
    core.voice
        .sessions
        .lock()
        .unwrap()
        .retain(|_, voice| voice.created_at.elapsed() < std::time::Duration::from_secs(300));
    if core.voice.local.is_busy() || !core.voice.sessions.lock().unwrap().is_empty() {
        return Err(BridgeError::Invalid(
            "A dictation is active or still releasing its engine".into(),
        ));
    }
    if params.provider == wire::VoiceProviderId::Local {
        return core.voice.local.start(params, core.events.clone());
    }
    let session_id = params.session_id.as_deref().ok_or_else(|| {
        BridgeError::Invalid("Experimental Codex dictation requires a coding session".into())
    })?;
    let provider = codex_capability(core, session_id)?;
    if provider.state != wire::VoiceAvailability::Ready {
        return Err(BridgeError::Invalid(
            provider
                .unavailable_reason
                .clone()
                .unwrap_or_else(|| "Voice unavailable".into()),
        ));
    }
    let session = binding(core, session_id)?;
    let provider_thread_id = session
        .provider_session_id
        .ok_or_else(|| BridgeError::Invalid("The Codex chat has no live provider thread".into()))?;
    let id = Uuid::new_v4().to_string();
    {
        let mut sessions = core.voice.sessions.lock().unwrap();
        sessions.insert(
            id.clone(),
            ActiveVoiceSession {
                id: id.clone(),
                owner_key: params.owner_key.clone(),
                session_id: session_id.to_owned(),
                provider_thread_id,
                next_sequence: 0,
                total_bytes: 0,
                created_at: std::time::Instant::now(),
            },
        );
    }
    // Keep lookup failure inside the result: an early `?` here used to skip
    // lease cleanup if the runtime disappeared after the capability check.
    let result = match core.adapters.lock().unwrap().get(session_id) {
        Some(runtime) => runtime.voice_start(),
        None => Err(BridgeError::Invalid(
            "The Codex runtime is not active".into(),
        )),
    };
    if let Err(error) = result {
        core.voice.sessions.lock().unwrap().remove(&id);
        return Err(error);
    }
    Ok(start_result(id, params))
}

fn decode_audio(params: &wire::VoiceAppendParams) -> Result<Vec<u8>, BridgeError> {
    // Bound the encoded input before allocating the decoded buffer. Transport
    // limits should not rely on a well-behaved frontend.
    if params.data.len() > (MAX_CHUNK_BYTES as usize).div_ceil(3) * 4 {
        return Err(BridgeError::Invalid(
            "Voice audio chunk exceeds the negotiated limit".into(),
        ));
    }
    let bytes = STANDARD
        .decode(&params.data)
        .map_err(|_| BridgeError::Invalid("Voice audio is not valid base64".into()))?;
    if bytes.len() > MAX_CHUNK_BYTES as usize {
        return Err(BridgeError::Invalid(
            "Voice audio chunk exceeds the negotiated limit".into(),
        ));
    }
    if bytes.is_empty()
        || bytes.len() != params.samples_per_channel as usize * 2 * CHANNELS as usize
    {
        return Err(BridgeError::Invalid(
            "Voice sample count does not match PCM payload size".into(),
        ));
    }
    Ok(bytes)
}

pub fn append(core: &BridgeCore, params: wire::VoiceAppendParams) -> Result<(), BridgeError> {
    if core.voice.local.contains(&params.voice_session_id) {
        return core.voice.local.append(params);
    }
    let bytes = decode_audio(&params)?;
    // Serialize writes for one voice session and commit the sequence only
    // after the provider accepted the frame. A failed or partial transport
    // write makes the lease unusable rather than inviting an ambiguous retry.
    let mut sessions = core.voice.sessions.lock().unwrap();
    let (session_id, total) = {
        let voice = sessions
            .get(&params.voice_session_id)
            .ok_or_else(|| BridgeError::Invalid("Voice session is no longer active".into()))?;
        if params.sequence != voice.next_sequence {
            return Err(BridgeError::Invalid(format!(
                "Expected voice sequence {}, got {}",
                voice.next_sequence, params.sequence
            )));
        }
        let total = voice.total_bytes.saturating_add(bytes.len() as u32);
        if total > MAX_SESSION_BYTES {
            return Err(BridgeError::Invalid(
                "Voice session exceeds the negotiated limit".into(),
            ));
        }
        (voice.session_id.clone(), total)
    };
    let adapters = core.adapters.lock().unwrap();
    let result = match adapters.get(&session_id) {
        Some(runtime) => runtime.voice_append(
            &params.data,
            SAMPLE_RATE,
            CHANNELS,
            params.samples_per_channel,
        ),
        None => Err(BridgeError::Invalid(
            "The Codex runtime is not active".into(),
        )),
    };
    drop(adapters);
    match result {
        Ok(()) => {
            let voice = sessions
                .get_mut(&params.voice_session_id)
                .expect("voice lease is held across its provider append");
            voice.next_sequence += 1;
            voice.total_bytes = total;
            Ok(())
        }
        Err(error) => {
            sessions.remove(&params.voice_session_id);
            Err(error)
        }
    }
}

pub fn stop(core: &BridgeCore, params: wire::VoiceStopParams) -> Result<(), BridgeError> {
    if core.voice.local.contains(&params.voice_session_id) {
        return core.voice.local.stop(&params.voice_session_id);
    }
    let session_id = core
        .voice
        .sessions
        .lock()
        .unwrap()
        .get(&params.voice_session_id)
        .map(|voice| voice.session_id.clone())
        .ok_or_else(|| BridgeError::Invalid("Voice session is no longer active".into()))?;
    let adapters = core.adapters.lock().unwrap();
    let result = match adapters.get(&session_id) {
        Some(runtime) => runtime.voice_stop(),
        None => Err(BridgeError::Invalid(
            "The Codex runtime is not active".into(),
        )),
    };
    drop(adapters);
    if result.is_err() {
        core.voice
            .sessions
            .lock()
            .unwrap()
            .remove(&params.voice_session_id);
    }
    result
}

pub fn cancel(core: &BridgeCore, params: wire::VoiceCancelParams) -> Result<(), BridgeError> {
    if core.voice.local.contains(&params.voice_session_id) {
        core.voice.local.cancel(&params.voice_session_id);
        return Ok(());
    }
    let Some(voice) = core
        .voice
        .sessions
        .lock()
        .unwrap()
        .remove(&params.voice_session_id)
    else {
        return Ok(());
    };
    if let Some(runtime) = core.adapters.lock().unwrap().get(&voice.session_id) {
        let _ = runtime.voice_stop();
    }
    Ok(())
}

/// Consume provider realtime frames before the ordinary conversation
/// normalizer. Returns true even for a stale realtime frame so cancellation
/// cannot turn a late provider error into a durable conversation error.
pub fn handle_codex_notification(
    core: &BridgeCore,
    session_id: &str,
    value: &serde_json::Value,
) -> bool {
    let Some(method) = value.get("method").and_then(serde_json::Value::as_str) else {
        return false;
    };
    let kind = match method {
        "thread/realtime/started" => wire::VoiceTranscriptKind::Started,
        // Codex also emits an item-scoped delta without a role. Consuming both
        // would duplicate text and could admit assistant speech, so composer
        // dictation uses only the flat role-bearing transcript stream.
        "thread/realtime/transcript/delta" => wire::VoiceTranscriptKind::Delta,
        "thread/realtime/transcript/done" => wire::VoiceTranscriptKind::Final,
        "thread/realtime/error" => wire::VoiceTranscriptKind::Error,
        "thread/realtime/closed" => wire::VoiceTranscriptKind::Closed,
        "thread/realtime/itemAdded"
        | "thread/realtime/item/started"
        | "thread/realtime/item/transcript/delta"
        | "thread/realtime/item/completed"
        | "thread/realtime/outputAudio/delta"
        | "thread/realtime/sdp" => return true,
        _ => return false,
    };
    let provider_thread_id = value
        .pointer("/params/threadId")
        .and_then(serde_json::Value::as_str);
    let role = value
        .pointer("/params/role")
        .or_else(|| value.pointer("/params/item/role"))
        .or_else(|| value.pointer("/params/itemRole"))
        .and_then(serde_json::Value::as_str);
    if matches!(
        kind,
        wire::VoiceTranscriptKind::Delta | wire::VoiceTranscriptKind::Final
    ) && role != Some("user")
    {
        return true;
    }
    let active = core
        .voice
        .sessions
        .lock()
        .unwrap()
        .values()
        .find(|voice| {
            voice.session_id == session_id
                && provider_thread_id.is_none_or(|thread| thread == voice.provider_thread_id)
        })
        .cloned();
    let Some(voice) = active else { return true };
    let text = value
        .pointer("/params/delta")
        .or_else(|| value.pointer("/params/transcript"))
        .or_else(|| value.pointer("/params/text"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let error = (kind == wire::VoiceTranscriptKind::Error).then(|| {
        value
            .pointer("/params/error/message")
            .or_else(|| value.pointer("/params/message"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Codex realtime dictation failed")
            .to_owned()
    });
    core.events
        .publish(CoreEvent::VoiceTranscript(wire::VoiceTranscriptEvent {
            voice_session_id: voice.id.clone(),
            owner_key: voice.owner_key,
            session_id: Some(voice.session_id),
            provider: wire::VoiceProviderId::Codex,
            kind,
            text,
            error,
        }));
    if matches!(
        kind,
        wire::VoiceTranscriptKind::Error | wire::VoiceTranscriptKind::Closed
    ) {
        core.voice.sessions.lock().unwrap().remove(&voice.id);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert_voice(core: &BridgeCore) {
        core.voice.sessions.lock().unwrap().insert(
            "voice-1".into(),
            ActiveVoiceSession {
                id: "voice-1".into(),
                owner_key: "draft-1".into(),
                session_id: "chat-1".into(),
                provider_thread_id: "thread-1".into(),
                next_sequence: 0,
                total_bytes: 0,
                created_at: std::time::Instant::now(),
            },
        );
    }

    #[test]
    fn pcm_contract_is_bounded_and_explicit() {
        let cap = capability(
            wire::VoiceProviderId::Codex,
            wire::VoiceAvailability::Ready,
            None,
        );
        assert_eq!(cap.sample_rate, 16_000);
        assert_eq!(cap.channels, 1);
        assert_eq!(cap.encoding, wire::VoiceAudioEncoding::PcmS16Le);
        assert!(cap.max_session_bytes > cap.max_chunk_bytes);
    }

    #[test]
    fn transcript_frames_are_scoped_to_the_voice_and_provider_session() {
        let fixture = tempfile::tempdir().unwrap();
        let core = BridgeCore::for_tests(fixture.path());
        insert_voice(&core);
        let mut events = core.events.subscribe();
        assert!(handle_codex_notification(
            &core,
            "chat-1",
            &serde_json::json!({"method":"thread/realtime/transcript/delta","params":{"threadId":"other","delta":"wrong"}}),
        ));
        assert!(matches!(
            events.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
        assert!(handle_codex_notification(
            &core,
            "chat-1",
            &serde_json::json!({"method":"thread/realtime/transcript/delta","params":{"threadId":"thread-1","role":"user","delta":"hello"}}),
        ));
        let event = events.try_recv().unwrap();
        let CoreEvent::VoiceTranscript(payload) = event else {
            panic!("voice event")
        };
        assert_eq!(payload.voice_session_id, "voice-1");
        assert_eq!(payload.text.as_deref(), Some("hello"));
    }

    #[test]
    fn item_deltas_and_non_user_transcripts_never_reach_the_composer() {
        let fixture = tempfile::tempdir().unwrap();
        let core = BridgeCore::for_tests(fixture.path());
        insert_voice(&core);
        let mut events = core.events.subscribe();
        for notification in [
            serde_json::json!({"method":"thread/realtime/item/transcript/delta","params":{"threadId":"thread-1","itemId":"i1","delta":"duplicate"}}),
            serde_json::json!({"method":"thread/realtime/transcript/delta","params":{"threadId":"thread-1","role":"assistant","delta":"assistant speech"}}),
            serde_json::json!({"method":"thread/realtime/transcript/delta","params":{"threadId":"thread-1","delta":"unscoped speech"}}),
        ] {
            assert!(handle_codex_notification(&core, "chat-1", &notification));
        }
        assert!(matches!(
            events.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn final_text_keeps_the_lease_until_the_provider_closes() {
        let fixture = tempfile::tempdir().unwrap();
        let core = BridgeCore::for_tests(fixture.path());
        insert_voice(&core);
        let mut events = core.events.subscribe();
        assert!(handle_codex_notification(
            &core,
            "chat-1",
            &serde_json::json!({"method":"thread/realtime/transcript/done","params":{"threadId":"thread-1","role":"user","text":"first phrase"}}),
        ));
        let CoreEvent::VoiceTranscript(final_event) = events.try_recv().unwrap() else {
            panic!("voice event")
        };
        assert_eq!(final_event.kind, wire::VoiceTranscriptKind::Final);
        assert!(core.voice.sessions.lock().unwrap().contains_key("voice-1"));

        assert!(handle_codex_notification(
            &core,
            "chat-1",
            &serde_json::json!({"method":"thread/realtime/closed","params":{"threadId":"thread-1"}}),
        ));
        let CoreEvent::VoiceTranscript(closed_event) = events.try_recv().unwrap() else {
            panic!("voice event")
        };
        assert_eq!(closed_event.kind, wire::VoiceTranscriptKind::Closed);
        assert!(!core.voice.sessions.lock().unwrap().contains_key("voice-1"));
    }

    #[test]
    fn cancellation_is_idempotent_and_late_transcripts_are_discarded() {
        let fixture = tempfile::tempdir().unwrap();
        let core = BridgeCore::for_tests(fixture.path());
        insert_voice(&core);
        cancel(
            &core,
            wire::VoiceCancelParams {
                voice_session_id: "voice-1".into(),
            },
        )
        .unwrap();
        cancel(
            &core,
            wire::VoiceCancelParams {
                voice_session_id: "voice-1".into(),
            },
        )
        .unwrap();
        let mut events = core.events.subscribe();
        assert!(handle_codex_notification(
            &core,
            "chat-1",
            &serde_json::json!({"method":"thread/realtime/transcript/delta","params":{"threadId":"thread-1","role":"user","delta":"late"}}),
        ));
        assert!(matches!(
            events.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn stop_failure_retires_the_voice_lease() {
        let fixture = tempfile::tempdir().unwrap();
        let core = BridgeCore::for_tests(fixture.path());
        insert_voice(&core);
        let error = stop(
            &core,
            wire::VoiceStopParams {
                voice_session_id: "voice-1".into(),
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("runtime is not active"));
        assert!(!core.voice.sessions.lock().unwrap().contains_key("voice-1"));
    }

    #[test]
    fn append_enforces_order_and_invalidates_an_uncertain_transport() {
        let fixture = tempfile::tempdir().unwrap();
        let core = BridgeCore::for_tests(fixture.path());
        insert_voice(&core);
        let data = STANDARD.encode([0_u8, 0]);
        let error = append(
            &core,
            wire::VoiceAppendParams {
                voice_session_id: "voice-1".into(),
                sequence: 1,
                data: data.clone(),
                samples_per_channel: 1,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("Expected voice sequence 0"));
        assert_eq!(
            core.voice.sessions.lock().unwrap()["voice-1"].next_sequence,
            0
        );

        let error = append(
            &core,
            wire::VoiceAppendParams {
                voice_session_id: "voice-1".into(),
                sequence: 0,
                data,
                samples_per_channel: 1,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("runtime is not active"));
        assert!(!core.voice.sessions.lock().unwrap().contains_key("voice-1"));
    }

    #[test]
    fn append_rejects_malformed_and_over_budget_audio_before_transport() {
        let fixture = tempfile::tempdir().unwrap();
        let core = BridgeCore::for_tests(fixture.path());
        insert_voice(&core);
        let mismatch = append(
            &core,
            wire::VoiceAppendParams {
                voice_session_id: "voice-1".into(),
                sequence: 0,
                data: STANDARD.encode([0_u8, 0]),
                samples_per_channel: 2,
            },
        )
        .unwrap_err();
        assert!(mismatch.to_string().contains("sample count"));

        core.voice
            .sessions
            .lock()
            .unwrap()
            .get_mut("voice-1")
            .unwrap()
            .total_bytes = MAX_SESSION_BYTES;
        let over_budget = append(
            &core,
            wire::VoiceAppendParams {
                voice_session_id: "voice-1".into(),
                sequence: 0,
                data: STANDARD.encode([0_u8, 0]),
                samples_per_channel: 1,
            },
        )
        .unwrap_err();
        assert!(over_budget.to_string().contains("session exceeds"));

        let oversized = append(
            &core,
            wire::VoiceAppendParams {
                voice_session_id: "voice-1".into(),
                sequence: 0,
                data: STANDARD.encode(vec![0_u8; MAX_CHUNK_BYTES as usize + 2]),
                samples_per_channel: MAX_CHUNK_BYTES / 2 + 1,
            },
        )
        .unwrap_err();
        assert!(oversized.to_string().contains("chunk exceeds"));
        assert!(core.voice.sessions.lock().unwrap().contains_key("voice-1"));
    }
}
