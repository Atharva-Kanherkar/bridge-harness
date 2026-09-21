use super::*;
use crate::{events::EventReceiver, BridgeCore};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::sync::{atomic::AtomicUsize, Condvar};

#[derive(Default)]
struct FakeProvider {
    starts: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
    fail_start: bool,
    fail_append: bool,
    fail_finish: bool,
    append_delay: Option<Duration>,
    block_start: Option<Arc<Gate>>,
    block_append: Option<Arc<Gate>>,
}

#[derive(Default)]
struct Gate(Mutex<bool>, Condvar);
impl Gate {
    fn wait(&self) {
        drop(
            self.1
                .wait_while(self.0.lock().unwrap(), |open| !*open)
                .unwrap(),
        );
    }
    fn open(&self) {
        *self.0.lock().unwrap() = true;
        self.1.notify_all();
    }
}

struct FakeStream {
    provider: FakeProvider,
    samples: usize,
}
impl VoiceProvider for FakeProvider {
    fn supported_locales(&self) -> Vec<String> {
        vec!["en".into()]
    }
    fn start(&self, _: Arc<AtomicBool>) -> Result<Box<dyn VoiceStream>, EngineFailure> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = &self.block_start {
            gate.wait();
        }
        if self.fail_start {
            return Err(EngineFailure::Unavailable);
        }
        Ok(Box::new(FakeStream {
            provider: Self {
                starts: self.starts.clone(),
                drops: self.drops.clone(),
                fail_append: self.fail_append,
                fail_finish: self.fail_finish,
                append_delay: self.append_delay,
                block_append: self.block_append.clone(),
                ..Self::default()
            },
            samples: 0,
        }))
    }
}
impl VoiceStream for FakeStream {
    fn append(&mut self, pcm: &[i16]) -> Result<Option<String>, EngineFailure> {
        if let Some(delay) = self.provider.append_delay {
            std::thread::sleep(delay);
        }
        if let Some(gate) = &self.provider.block_append {
            gate.wait();
        }
        if self.provider.fail_append {
            return Err(EngineFailure::Inference);
        }
        assert!(pcm.iter().all(|sample| *sample == 1 || *sample == -2));
        self.samples += pcm.len();
        Ok(Some(
            if self.samples <= 2 {
                "write a cash"
            } else {
                "write a cache"
            }
            .into(),
        ))
    }
    fn finish(&mut self) -> Result<String, EngineFailure> {
        if self.provider.fail_finish {
            return Err(EngineFailure::Inference);
        }
        Ok(if self.samples == 0 {
            ""
        } else {
            "write a cache"
        }
        .into())
    }
}
impl Drop for FakeStream {
    fn drop(&mut self) {
        self.provider.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn params(owner: &str) -> wire::VoiceStartParams {
    wire::VoiceStartParams {
        owner_key: owner.into(),
        session_id: None,
        provider: wire::VoiceProviderId::Local,
    }
}
fn audio(id: &str, sequence: u32) -> wire::VoiceAppendParams {
    wire::VoiceAppendParams {
        voice_session_id: id.into(),
        sequence,
        data: STANDARD.encode([1, 0, 254, 255]),
        samples_per_channel: 2,
    }
}
fn wait_idle(service: &LocalVoiceService) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while service.is_busy() {
        assert!(
            Instant::now() < deadline,
            "worker did not release its lease"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn event(events: &mut EventReceiver) -> wire::VoiceTranscriptEvent {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Ok(CoreEvent::VoiceTranscript(event)) = events.try_recv() {
            return event;
        }
        assert!(Instant::now() < deadline, "voice event did not arrive");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn fresh_draft_flow_never_creates_a_coding_session_or_adapter() {
    let dir = tempfile::tempdir().unwrap();
    let mut core = BridgeCore::for_tests(dir.path());
    let fake = Arc::new(FakeProvider::default());
    core.voice.local = LocalVoiceService::new(Some(fake.clone()));
    let caps =
        super::super::capabilities(&core, wire::VoiceCapabilitiesParams { session_id: None })
            .unwrap();
    assert_eq!(caps.providers[0].state, wire::VoiceAvailability::Ready);
    assert_eq!(
        caps.providers[0].processing,
        wire::VoiceProcessingLocation::OnDevice
    );
    assert!(caps.selected_provider.is_none());
    assert_eq!(
        fake.starts.load(Ordering::SeqCst),
        0,
        "probe must not load the engine"
    );
    let mut events = core.events.subscribe();
    let started = super::super::start(&core, params("draft:claude:1")).unwrap();
    assert!(started.session_id.is_none());
    let id = &started.voice_session_id;
    assert_eq!(event(&mut events).kind, wire::VoiceTranscriptKind::Started);
    super::super::append(&core, audio(id, 0)).unwrap();
    assert_eq!(event(&mut events).text.as_deref(), Some("write a cash"));
    super::super::append(&core, audio(id, 1)).unwrap();
    let partial = event(&mut events);
    assert_eq!(partial.kind, wire::VoiceTranscriptKind::Partial);
    assert_eq!(partial.text.as_deref(), Some("write a cache"));
    super::super::stop(
        &core,
        wire::VoiceStopParams {
            voice_session_id: id.clone(),
        },
    )
    .unwrap();
    let final_event = event(&mut events);
    assert_eq!(final_event.kind, wire::VoiceTranscriptKind::Final);
    assert_eq!(final_event.owner_key, "draft:claude:1");
    assert_eq!(event(&mut events).kind, wire::VoiceTranscriptKind::Closed);
    wait_idle(&core.voice.local);
    assert!(core.adapters.lock().unwrap().is_empty());
    assert_eq!(
        core.db
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(fake.drops.load(Ordering::SeqCst), 1);
}

#[test]
fn missing_model_is_setup_not_a_hidden_remote_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let core = BridgeCore::for_tests(dir.path());
    for session_id in [None, Some("stale-chat".into())] {
        let caps = super::super::capabilities(&core, wire::VoiceCapabilitiesParams { session_id })
            .unwrap();
        assert_eq!(caps.providers[0].state, wire::VoiceAvailability::NeedsSetup);
        assert_eq!(
            caps.providers[0].recovery_action,
            Some(wire::VoiceRecoveryAction::Setup)
        );
        assert!(caps.selected_provider.is_none());
    }
    assert!(super::super::start(&core, params("draft"))
        .unwrap_err()
        .to_string()
        .contains("setup"));
    assert!(core.adapters.lock().unwrap().is_empty());
    assert!(!core.voice.local.is_busy());
}

#[test]
fn audio_validation_and_sequence_rejection_do_not_reach_the_engine() {
    let service = LocalVoiceService::new(Some(Arc::new(FakeProvider::default())));
    let started = service.start(params("draft"), EventBus::new()).unwrap();
    let id = &started.voice_session_id;
    assert!(service
        .append(audio(id, 1))
        .unwrap_err()
        .to_string()
        .contains("sequence"));
    let mut malformed = audio(id, 0);
    malformed.data = "!".into();
    assert!(service.append(malformed).is_err());
    let mut oversized = audio(id, 0);
    oversized.data = "A".repeat(100_000);
    assert!(service.append(oversized).is_err());
    service.append(audio(id, 0)).unwrap();
    service.stop(id).unwrap();
    wait_idle(&service);
    assert!(service.append(audio(id, 1)).is_err());
}

#[test]
fn engine_failures_release_the_take_and_emit_sanitized_errors() {
    for phase in 0..3 {
        let fake = Arc::new(FakeProvider {
            fail_start: phase == 0,
            fail_append: phase == 1,
            fail_finish: phase == 2,
            ..FakeProvider::default()
        });
        let service = LocalVoiceService::new(Some(fake.clone()));
        let bus = EventBus::new();
        let mut events = bus.subscribe();
        let result = service.start(params("draft"), bus);
        if phase == 0 {
            assert!(result.is_err());
        } else {
            let id = result.unwrap().voice_session_id;
            assert_eq!(event(&mut events).kind, wire::VoiceTranscriptKind::Started);
            if phase == 1 {
                assert!(service.append(audio(&id, 0)).is_err());
            } else {
                assert!(service.stop(&id).is_err());
            }
        }
        let error = event(&mut events);
        assert_eq!(error.kind, wire::VoiceTranscriptKind::Error);
        assert!(error.text.is_none());
        wait_idle(&service);
        assert_eq!(fake.drops.load(Ordering::SeqCst), usize::from(phase != 0));
    }
}

#[test]
fn startup_timeout_holds_busy_until_late_engine_is_released() {
    let gate = Arc::new(Gate::default());
    let fake = Arc::new(FakeProvider {
        block_start: Some(gate.clone()),
        ..FakeProvider::default()
    });
    let mut service = LocalVoiceService::new(Some(fake.clone()));
    service.limits.startup = Duration::from_millis(20);
    let bus = EventBus::new();
    let mut events = bus.subscribe();
    assert!(service
        .start(params("draft"), bus.clone())
        .unwrap_err()
        .to_string()
        .contains("ready in time"));
    assert!(service.is_busy());
    assert!(service.start(params("same-owner"), bus).is_err());
    gate.open();
    wait_idle(&service);
    assert_eq!(fake.starts.load(Ordering::SeqCst), 1);
    assert_eq!(fake.drops.load(Ordering::SeqCst), 1);
    assert!(
        events.try_recv().is_err(),
        "cancelled startup must not emit late ready"
    );
}

#[test]
fn append_timeout_discards_late_text_and_preserves_worker_ownership() {
    let gate = Arc::new(Gate::default());
    let mut service = LocalVoiceService::new(Some(Arc::new(FakeProvider {
        block_append: Some(gate.clone()),
        ..FakeProvider::default()
    })));
    service.limits.rpc = Duration::from_millis(20);
    let bus = EventBus::new();
    let mut events = bus.subscribe();
    let id = service
        .start(params("draft"), bus.clone())
        .unwrap()
        .voice_session_id;
    event(&mut events);
    assert!(service
        .append(audio(&id, 0))
        .unwrap_err()
        .to_string()
        .contains("timed out"));
    assert!(service.is_busy());
    assert!(service.start(params("draft"), bus).is_err());
    gate.open();
    wait_idle(&service);
    assert!(events.try_recv().is_err());
}

#[test]
fn idle_take_expires_without_another_client_request() {
    let mut service = LocalVoiceService::new(Some(Arc::new(FakeProvider::default())));
    service.limits.lifetime = Duration::from_millis(30);
    let bus = EventBus::new();
    let mut events = bus.subscribe();
    service.start(params("draft"), bus).unwrap();
    assert_eq!(event(&mut events).kind, wire::VoiceTranscriptKind::Started);
    let expired = event(&mut events);
    assert_eq!(expired.kind, wire::VoiceTranscriptKind::Error);
    assert!(expired.error.unwrap().contains("time limit"));
    wait_idle(&service);
}

#[test]
fn old_id_cannot_cancel_or_write_to_a_replacement_with_the_same_owner() {
    let service = LocalVoiceService::new(Some(Arc::new(FakeProvider::default())));
    let bus = EventBus::new();
    let old = service
        .start(params("draft"), bus.clone())
        .unwrap()
        .voice_session_id;
    service.cancel(&old);
    service.cancel(&old);
    wait_idle(&service);
    let new = service
        .start(params("draft"), bus)
        .unwrap()
        .voice_session_id;
    assert_ne!(old, new);
    service.cancel(&old);
    assert!(service.append(audio(&old, 0)).is_err());
    service.append(audio(&new, 0)).unwrap();
    service.stop(&new).unwrap();
    wait_idle(&service);
}

#[test]
fn queue_backpressure_cancels_instead_of_retaining_unbounded_audio() {
    let service = LocalVoiceService::default();
    let (commands, _held_receiver) = mpsc::sync_channel(0);
    let cancelled = Arc::new(AtomicBool::new(false));
    *service.active.lock().unwrap() = Some(Active {
        id: "full".into(),
        commands,
        cancelled: cancelled.clone(),
    });
    assert!(service
        .append(audio("full", 0))
        .unwrap_err()
        .to_string()
        .contains("queue"));
    assert!(cancelled.load(Ordering::Acquire));
}

#[test]
fn dropping_service_cancels_idle_engine() {
    let fake = Arc::new(FakeProvider::default());
    let service = LocalVoiceService::new(Some(fake.clone()));
    service.start(params("draft"), EventBus::new()).unwrap();
    let active = service.active.clone();
    drop(service);
    let deadline = Instant::now() + Duration::from_secs(2);
    while active.lock().unwrap().is_some() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(fake.drops.load(Ordering::SeqCst), 1);
}

#[test]
fn no_partial_is_published_when_inference_returns_after_the_take_deadline() {
    let mut service = LocalVoiceService::new(Some(Arc::new(FakeProvider {
        append_delay: Some(Duration::from_millis(50)),
        ..FakeProvider::default()
    })));
    service.limits.lifetime = Duration::from_millis(30);
    let bus = EventBus::new();
    let mut events = bus.subscribe();
    let id = service
        .start(params("draft"), bus)
        .unwrap()
        .voice_session_id;
    assert_eq!(event(&mut events).kind, wire::VoiceTranscriptKind::Started);
    assert!(service.append(audio(&id, 0)).is_err());
    assert_eq!(event(&mut events).kind, wire::VoiceTranscriptKind::Error);
    wait_idle(&service);
    assert!(events.try_recv().is_err());
}

#[test]
fn session_audio_budget_is_enforced_before_inference() {
    let service = LocalVoiceService::new(Some(Arc::new(FakeProvider::default())));
    let id = service
        .start(params("draft"), EventBus::new())
        .unwrap()
        .voice_session_id;
    let data = STANDARD.encode([1, 0].repeat(32_768));
    for sequence in 0..64 {
        service
            .append(wire::VoiceAppendParams {
                voice_session_id: id.clone(),
                sequence,
                data: data.clone(),
                samples_per_channel: 32_768,
            })
            .unwrap();
    }
    assert!(service
        .append(audio(&id, 64))
        .unwrap_err()
        .to_string()
        .contains("session exceeds"));
    wait_idle(&service);
}
