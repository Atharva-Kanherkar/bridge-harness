//! Harness-independent streaming service. No database, adapter, credentials,
//! model download, or microphone access lives at this boundary.
use super::{capability, decode_audio, start_result, MAX_SESSION_BYTES};
use crate::{
    events::{CoreEvent, EventBus},
    BridgeError,
};
use bridge_protocol::messages as wire;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Deliberately bounded/sanitized: engine diagnostics may contain private text.
#[derive(Debug, Clone, Copy)]
pub enum EngineFailure {
    Unavailable,
    Inference,
}

pub trait VoiceStream: Send {
    /// A complete provisional hypothesis; it may revise earlier words.
    fn append(&mut self, pcm: &[i16]) -> Result<Option<String>, EngineFailure>;
    /// Finalize exactly once, including any trailing speech. Drop cancels.
    fn finish(&mut self) -> Result<String, EngineFailure>;
}

pub trait VoiceProvider: Send + Sync {
    /// This is metadata only: never download or load a model during a probe.
    fn supported_locales(&self) -> Vec<String>;
    /// Implementations must observe cancellation; a native helper must be
    /// killed/reaped before this stream is dropped. Busy ownership remains
    /// held until the worker actually returns, even after client cancellation.
    fn start(&self, cancelled: Arc<AtomicBool>) -> Result<Box<dyn VoiceStream>, EngineFailure>;
}

#[derive(Clone, Copy)]
struct Limits {
    startup: Duration,
    rpc: Duration,
    lifetime: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            startup: Duration::from_secs(30),
            rpc: Duration::from_secs(10),
            lifetime: Duration::from_secs(150),
        }
    }
}

enum Command {
    Append {
        sequence: u32,
        pcm: Vec<i16>,
        reply: mpsc::Sender<Result<(), &'static str>>,
    },
    Finish {
        reply: mpsc::Sender<Result<(), &'static str>>,
    },
    Cancel,
}

struct Active {
    id: String,
    cancelled: Arc<AtomicBool>,
    commands: mpsc::SyncSender<Command>,
}

pub struct LocalVoiceService {
    provider: Arc<Mutex<Option<Arc<dyn VoiceProvider>>>>,
    active: Arc<Mutex<Option<Active>>>,
    limits: Limits,
}

impl Default for LocalVoiceService {
    fn default() -> Self {
        Self::new(None)
    }
}

fn invalid(message: &str) -> BridgeError {
    BridgeError::Invalid(message.into())
}

impl LocalVoiceService {
    pub fn new(provider: Option<Arc<dyn VoiceProvider>>) -> Self {
        Self {
            provider: Arc::new(Mutex::new(provider)),
            active: Arc::new(Mutex::new(None)),
            limits: Limits::default(),
        }
    }

    pub fn capability(&self) -> wire::VoiceProviderCapability {
        let provider = self.provider.lock().unwrap().clone();
        let (state, reason) = if provider.is_some() {
            (wire::VoiceAvailability::Ready, None)
        } else {
            (
                wire::VoiceAvailability::NeedsSetup,
                Some(
                    "Local dictation is not installed. Engine and model setup are still required."
                        .into(),
                ),
            )
        };
        let mut result = capability(wire::VoiceProviderId::Local, state, reason);
        result.supported_locales = self
            .provider
            .lock()
            .unwrap()
            .as_ref()
            .map(|p| p.supported_locales())
            .unwrap_or_default();
        result
    }

    pub fn set_provider(&self, provider: Option<Arc<dyn VoiceProvider>>) {
        *self.provider.lock().unwrap() = provider;
    }

    pub(crate) fn provider_slot(&self) -> Arc<Mutex<Option<Arc<dyn VoiceProvider>>>> {
        self.provider.clone()
    }

    pub fn is_busy(&self) -> bool {
        self.active.lock().unwrap().is_some()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.active
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|take| take.id == id)
    }

    pub fn start(
        &self,
        params: wire::VoiceStartParams,
        events: EventBus,
    ) -> Result<wire::VoiceStartResult, BridgeError> {
        let provider = self
            .provider
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| invalid("Local dictation needs engine and model setup"))?;
        let id = Uuid::new_v4().to_string();
        // Bounded even for concurrent/misbehaving clients: two queued PCM chunks.
        let (commands, incoming) = mpsc::sync_channel(2);
        let (ready, readiness) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        {
            let mut active = self.active.lock().unwrap();
            if active.is_some() {
                return Err(invalid(
                    "A dictation is active or still releasing its engine",
                ));
            }
            *active = Some(Active {
                id: id.clone(),
                cancelled: cancelled.clone(),
                commands,
            });
        }
        let worker = Worker {
            id: id.clone(),
            params: params.clone(),
            cancelled,
            events,
            active: self.active.clone(),
            deadline: Instant::now() + self.limits.lifetime,
        };
        // The guard is dropped after the stream, including an unwinding worker.
        let launch = std::thread::Builder::new()
            .name("bridge-dictation".into())
            .spawn(move || {
                let _ = worker.run(provider, incoming, ready);
            });
        if launch.is_err() {
            // Builder drops its closure (and Worker guard) on spawn failure.
            return Err(invalid("Could not start the local dictation worker"));
        }
        match readiness.recv_timeout(self.limits.startup) {
            Ok(Ok(())) => Ok(start_result(id, params)),
            Ok(Err(message)) => Err(invalid(message)),
            Err(_) => {
                self.cancel(&id);
                Err(invalid("Local dictation did not become ready in time"))
            }
        }
    }

    fn request(
        &self,
        id: &str,
        command: Command,
        reply: mpsc::Receiver<Result<(), &'static str>>,
    ) -> Result<(), BridgeError> {
        let commands = {
            let active = self.active.lock().unwrap();
            let take = active
                .as_ref()
                .filter(|take| take.id == id && !take.cancelled.load(Ordering::Acquire))
                .ok_or_else(|| invalid("Voice session is no longer active"))?;
            take.commands.clone()
        };
        if commands.try_send(command).is_err() {
            self.cancel(id);
            return Err(invalid("Local dictation input queue is full or closed"));
        }
        match reply.recv_timeout(self.limits.rpc) {
            Ok(result) => result.map_err(invalid),
            Err(_) => {
                self.cancel(id);
                Err(invalid("Local dictation operation timed out"))
            }
        }
    }

    pub fn append(&self, params: wire::VoiceAppendParams) -> Result<(), BridgeError> {
        let bytes = decode_audio(&params)?;
        let pcm = bytes
            .chunks_exact(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let (reply, received) = mpsc::channel();
        self.request(
            &params.voice_session_id,
            Command::Append {
                sequence: params.sequence,
                pcm,
                reply,
            },
            received,
        )
    }

    pub fn stop(&self, id: &str) -> Result<(), BridgeError> {
        let (reply, received) = mpsc::channel();
        self.request(id, Command::Finish { reply }, received)
    }

    pub fn cancel(&self, id: &str) {
        let active = self.active.lock().unwrap();
        if let Some(take) = active.as_ref().filter(|take| take.id == id) {
            take.cancelled.store(true, Ordering::Release);
            let _ = take.commands.try_send(Command::Cancel);
        }
    }
}

impl Drop for LocalVoiceService {
    fn drop(&mut self) {
        if let Some(take) = self.active.lock().unwrap().as_ref() {
            take.cancelled.store(true, Ordering::Release);
            let _ = take.commands.try_send(Command::Cancel);
        }
    }
}

struct Worker {
    id: String,
    params: wire::VoiceStartParams,
    cancelled: Arc<AtomicBool>,
    events: EventBus,
    active: Arc<Mutex<Option<Active>>>,
    deadline: Instant,
}

impl Worker {
    fn check_live(&self) -> Result<(), &'static str> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err("Local dictation was cancelled");
        }
        if Instant::now() >= self.deadline {
            self.error("Local dictation reached its time limit");
            return Err("Local dictation reached its time limit");
        }
        Ok(())
    }

    fn emit(&self, kind: wire::VoiceTranscriptKind, text: Option<String>, error: Option<String>) {
        // Serialize emission with cancellation. Never relabel an old result
        // with a replacement take's ID, even when its owner is unchanged.
        let active = self.active.lock().unwrap();
        if self.cancelled.load(Ordering::Acquire)
            || !active.as_ref().is_some_and(|take| take.id == self.id)
        {
            return;
        }
        self.events
            .publish(CoreEvent::VoiceTranscript(wire::VoiceTranscriptEvent {
                voice_session_id: self.id.clone(),
                owner_key: self.params.owner_key.clone(),
                session_id: self.params.session_id.clone(),
                provider: wire::VoiceProviderId::Local,
                kind,
                text,
                error,
            }));
    }

    fn error(&self, message: &str) {
        self.emit(wire::VoiceTranscriptKind::Error, None, Some(message.into()));
    }

    fn run(
        &self,
        provider: Arc<dyn VoiceProvider>,
        commands: mpsc::Receiver<Command>,
        ready: mpsc::Sender<Result<(), &'static str>>,
    ) -> Result<(), ()> {
        let mut stream = match provider.start(self.cancelled.clone()) {
            Ok(stream) => stream,
            Err(_) => {
                self.error("Local dictation engine could not load");
                let _ = ready.send(Err("Local dictation engine could not load"));
                return Err(());
            }
        };
        if let Err(message) = self.check_live() {
            let _ = ready.send(Err(message));
            return Err(());
        }
        self.emit(wire::VoiceTranscriptKind::Started, None, None);
        if ready.send(Ok(())).is_err() {
            return Ok(());
        }
        let mut sequence = 0;
        let mut total_bytes = 0usize;
        loop {
            if self.cancelled.load(Ordering::Acquire) {
                return Ok(());
            }
            let Some(remaining) = self.deadline.checked_duration_since(Instant::now()) else {
                self.error("Local dictation reached its time limit");
                return Err(());
            };
            let command = match commands.recv_timeout(remaining) {
                Ok(command) => command,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    self.error("Local dictation reached its time limit");
                    return Err(());
                }
                Err(_) => return Ok(()),
            };
            if self.cancelled.load(Ordering::Acquire) {
                return Ok(());
            }
            match command {
                Command::Cancel => return Ok(()),
                Command::Append {
                    sequence: received,
                    pcm,
                    reply,
                } => {
                    if received != sequence {
                        let _ = reply.send(Err("Voice chunk sequence is out of order"));
                        continue;
                    }
                    if total_bytes + pcm.len() * 2 > MAX_SESSION_BYTES as usize {
                        self.error("Voice session exceeds the negotiated limit");
                        let _ = reply.send(Err("Voice session exceeds the negotiated limit"));
                        return Err(());
                    }
                    match stream.append(&pcm) {
                        Ok(partial) => {
                            if let Err(message) = self.check_live() {
                                let _ = reply.send(Err(message));
                                return Err(());
                            }
                            total_bytes += pcm.len() * 2;
                            sequence += 1;
                            if let Some(text) = partial {
                                if text.len() > 64_000 {
                                    self.error("Local dictation transcript exceeded its limit");
                                    let _ = reply
                                        .send(Err("Local dictation transcript exceeded its limit"));
                                    return Err(());
                                }
                                self.emit(wire::VoiceTranscriptKind::Partial, Some(text), None);
                            }
                            let _ = reply.send(Ok(()));
                        }
                        Err(_) => {
                            self.error("Local dictation inference failed");
                            let _ = reply.send(Err("Local dictation inference failed"));
                            return Err(());
                        }
                    }
                }
                Command::Finish { reply } => match stream.finish() {
                    Ok(text) if text.len() <= 64_000 => {
                        if let Err(message) = self.check_live() {
                            let _ = reply.send(Err(message));
                            return Err(());
                        }
                        self.emit(wire::VoiceTranscriptKind::Final, Some(text), None);
                        self.emit(wire::VoiceTranscriptKind::Closed, None, None);
                        let _ = reply.send(Ok(()));
                        return Ok(());
                    }
                    _ => {
                        self.error("Local dictation could not finalize");
                        let _ = reply.send(Err("Local dictation could not finalize"));
                        return Err(());
                    }
                },
            }
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        let mut active = self.active.lock().unwrap();
        if active.as_ref().is_some_and(|take| take.id == self.id) {
            *active = None;
        }
    }
}

#[cfg(test)]
mod tests;
