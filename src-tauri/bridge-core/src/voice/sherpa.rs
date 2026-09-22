//! Supervised bridge to the optional sherpa-onnx local speech installation.
//!
//! Detection is metadata-only. It never loads a model or touches the network;
//! the native helper and model are opened lazily after an explicit voice start.

use super::local::{EngineFailure, VoiceProvider, VoiceStream};
use crate::{adapters, process_ledger};
use serde::Deserialize;
use std::ffi::OsStr;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread;
use std::time::Duration;

pub const ENGINE_VERSION: &str = "1.13.8";
pub const MODEL_ID: &str = "nemotron-speech-streaming-en-0.6b-560ms-int8-2026-04-25";
pub const RUNTIME_ARCHIVE_SHA256: &str =
    "91b96512c4fa1960f8a9ed5360a6c8dda53a4b5015d0590244f14086a234557a";
pub const MODEL_ARCHIVE_SHA256: &str =
    "78e2b79fcf7271553a74402a76b771b09ea40117a39566a79f52235b23db6358";

const READY_TIMEOUT: Duration = Duration::from_secs(20);
const INFERENCE_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_FRAME_BYTES: usize = 64 * 1024;

const COMMAND_APPEND: u8 = 1;
const COMMAND_FINISH: u8 = 2;
const COMMAND_CANCEL: u8 = 3;
const RESPONSE_READY: u8 = 0x80;
const RESPONSE_PARTIAL: u8 = 0x81;
const RESPONSE_FINAL: u8 = 0x82;
const RESPONSE_CLOSED: u8 = 0x83;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InstallManifest {
    schema_version: u32,
    engine_version: String,
    model_id: String,
    runtime_archive_sha256: String,
    model_archive_sha256: String,
}

impl InstallManifest {
    fn is_expected(&self) -> bool {
        self.schema_version == 1
            && self.engine_version == ENGINE_VERSION
            && self.model_id == MODEL_ID
            && self.runtime_archive_sha256 == RUNTIME_ARCHIVE_SHA256
            && self.model_archive_sha256 == MODEL_ARCHIVE_SHA256
    }
}

#[derive(Debug)]
enum HelperFrame {
    Ready,
    Partial(String),
    Final(String),
    Closed,
}

#[derive(Debug, Clone)]
struct SherpaProvider {
    helper: PathBuf,
    runtime: PathBuf,
    model: PathBuf,
    supervised: bool,
}

impl SherpaProvider {
    fn discover(data_dir: &Path) -> Option<Self> {
        // Development-only override for reproducing the pinned native spike.
        // All three values are required so a stale shell variable cannot mix
        // an unreviewed runtime or model into the installed layout.
        let overridden = std::env::var_os("BRIDGE_VOICE_HELPER")
            .zip(std::env::var_os("BRIDGE_VOICE_RUNTIME_DIR"))
            .zip(std::env::var_os("BRIDGE_VOICE_MODEL_DIR"))
            .map(|((helper, runtime), model)| Self {
                helper: helper.into(),
                runtime: runtime.into(),
                model: model.into(),
                supervised: true,
            });
        if let Some(provider) = overridden.filter(Self::paths_exist) {
            return Some(provider);
        }

        let root = data_dir.join(format!("voice/sherpa-onnx-{ENGINE_VERSION}"));
        let manifest: InstallManifest =
            serde_json::from_slice(&fs::read(root.join("install.json")).ok()?).ok()?;
        if !manifest.is_expected() {
            return None;
        }
        let provider = Self {
            helper: resolve_helper()?,
            runtime: root.join("runtime"),
            model: root.join("model"),
            supervised: true,
        };
        provider.paths_exist().then_some(provider)
    }

    fn paths_exist(&self) -> bool {
        executable(&self.helper)
            && self
                .runtime
                .join("lib/libsherpa-onnx-c-api.dylib")
                .is_file()
            && [
                "encoder.int8.onnx",
                "decoder.int8.onnx",
                "joiner.int8.onnx",
                "tokens.txt",
            ]
            .iter()
            .all(|file| self.model.join(file).is_file())
    }
}

fn executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

impl VoiceProvider for SherpaProvider {
    fn supported_locales(&self) -> Vec<String> {
        vec!["en-US".into()]
    }

    fn start(&self, cancelled: Arc<AtomicBool>) -> Result<Box<dyn VoiceStream>, EngineFailure> {
        SherpaStream::spawn(self, cancelled).map(|stream| Box::new(stream) as Box<dyn VoiceStream>)
    }
}

fn resolve_helper() -> Option<PathBuf> {
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            let bundled = directory.join("bridge-voice-helper");
            if bundled.is_file() {
                return Some(bundled);
            }
        }
    }
    let architecture = match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        "x86_64" => "x86_64",
        _ => return None,
    };
    let development = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../binaries/bridge-voice-helper-{architecture}-apple-darwin"
    ));
    development.is_file().then_some(development)
}

/// Discover a complete explicit installation. Capability probes call this at
/// boot only; no model is loaded and no recovery/download is attempted here.
pub fn installed_provider(data_dir: &Path) -> Option<Arc<dyn VoiceProvider>> {
    SherpaProvider::discover(data_dir).map(|provider| Arc::new(provider) as Arc<dyn VoiceProvider>)
}

struct SherpaStream {
    child: Option<Child>,
    input: Option<ChildStdin>,
    frames: mpsc::Receiver<Result<HelperFrame, EngineFailure>>,
    reader: Option<thread::JoinHandle<()>>,
    cancellation_watcher: Option<thread::JoinHandle<()>>,
    watcher_stop: Arc<AtomicBool>,
    launch: Option<process_ledger::LaunchGuard>,
    cancelled: Arc<AtomicBool>,
}

impl SherpaStream {
    fn spawn(provider: &SherpaProvider, cancelled: Arc<AtomicBool>) -> Result<Self, EngineFailure> {
        if cancelled.load(Ordering::Acquire) {
            return Err(EngineFailure::Unavailable);
        }
        let arguments: [&OsStr; 2] = [provider.runtime.as_os_str(), provider.model.as_os_str()];
        let mut command = if provider.supervised {
            adapters::supervised_command(&provider.helper, arguments)
        } else {
            let mut command = std::process::Command::new(&provider.helper);
            command.args(arguments);
            command
        };
        adapters::configure_process_group(&mut command);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(target_os = "macos")]
        command.env("DYLD_LIBRARY_PATH", provider.runtime.join("lib"));
        #[cfg(not(target_os = "macos"))]
        command.env("LD_LIBRARY_PATH", provider.runtime.join("lib"));
        let mut child = command.spawn().map_err(|_| EngineFailure::Unavailable)?;
        let launch = process_ledger::record_launch("voice_helper", MODEL_ID, child.id());
        let Some(input) = child.stdin.take() else {
            let _ = adapters::terminate_process_group(child.id());
            let _ = child.wait();
            return Err(EngineFailure::Unavailable);
        };
        let Some(output) = child.stdout.take() else {
            let _ = adapters::terminate_process_group(child.id());
            let _ = child.wait();
            return Err(EngineFailure::Unavailable);
        };
        let (sender, frames) = mpsc::sync_channel(2);
        let reader = match thread::Builder::new()
            .name("bridge-voice-helper-reader".into())
            .spawn(move || read_helper_frames(output, sender))
        {
            Ok(reader) => reader,
            Err(_) => {
                let _ = adapters::terminate_process_group(child.id());
                let _ = child.wait();
                return Err(EngineFailure::Unavailable);
            }
        };
        let watcher_stop = Arc::new(AtomicBool::new(false));
        let watcher_cancelled = cancelled.clone();
        let watcher_stopped = watcher_stop.clone();
        let pid = child.id();
        let cancellation_watcher = match thread::Builder::new()
            .name("bridge-voice-helper-cancel".into())
            .spawn(move || {
                while !watcher_stopped.load(Ordering::Acquire) {
                    if watcher_cancelled.load(Ordering::Acquire) {
                        let _ = adapters::terminate_process_group(pid);
                        return;
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            }) {
            Ok(watcher) => watcher,
            Err(_) => {
                let _ = adapters::terminate_process_group(child.id());
                let _ = child.wait();
                let _ = reader.join();
                return Err(EngineFailure::Unavailable);
            }
        };
        let mut stream = Self {
            child: Some(child),
            input: Some(input),
            frames,
            reader: Some(reader),
            cancellation_watcher: Some(cancellation_watcher),
            watcher_stop,
            launch: Some(launch),
            cancelled,
        };
        match stream.receive(READY_TIMEOUT) {
            Ok(HelperFrame::Ready) => Ok(stream),
            _ => {
                stream.terminate();
                Err(EngineFailure::Unavailable)
            }
        }
    }

    fn send(&mut self, kind: u8, payload: &[u8]) -> Result<(), EngineFailure> {
        if payload.len() > MAX_FRAME_BYTES || self.cancelled.load(Ordering::Acquire) {
            self.terminate();
            return Err(EngineFailure::Inference);
        }
        let input = self.input.as_mut().ok_or(EngineFailure::Inference)?;
        let mut header = [0_u8; 5];
        header[0] = kind;
        header[1..].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        if input
            .write_all(&header)
            .and_then(|_| input.write_all(payload))
            .and_then(|_| input.flush())
            .is_err()
        {
            self.terminate();
            return Err(EngineFailure::Inference);
        }
        Ok(())
    }

    fn receive(&mut self, timeout: Duration) -> Result<HelperFrame, EngineFailure> {
        match self.frames.recv_timeout(timeout) {
            Ok(frame) => frame,
            Err(_) => {
                self.terminate();
                Err(EngineFailure::Inference)
            }
        }
    }

    fn terminate(&mut self) {
        self.watcher_stop.store(true, Ordering::Release);
        self.input.take();
        if let Some(mut child) = self.child.take() {
            let _ = adapters::terminate_process_group(child.id());
            let _ = child.wait();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        if let Some(watcher) = self.cancellation_watcher.take() {
            let _ = watcher.join();
        }
        self.launch.take();
    }
}

impl VoiceStream for SherpaStream {
    fn append(&mut self, pcm: &[i16]) -> Result<Option<String>, EngineFailure> {
        let mut bytes = Vec::with_capacity(pcm.len() * 2);
        for sample in pcm {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        self.send(COMMAND_APPEND, &bytes)?;
        match self.receive(INFERENCE_TIMEOUT)? {
            HelperFrame::Partial(text) => Ok((!text.is_empty()).then_some(text)),
            _ => Err(EngineFailure::Inference),
        }
    }

    fn finish(&mut self) -> Result<String, EngineFailure> {
        self.send(COMMAND_FINISH, &[])?;
        let text = match self.receive(INFERENCE_TIMEOUT)? {
            HelperFrame::Final(text) => text,
            _ => return Err(EngineFailure::Inference),
        };
        if !matches!(self.receive(Duration::from_secs(1))?, HelperFrame::Closed) {
            return Err(EngineFailure::Inference);
        }
        self.input.take();
        if let Some(mut child) = self.child.take() {
            if !child.wait().is_ok_and(|status| status.success()) {
                return Err(EngineFailure::Inference);
            }
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        self.watcher_stop.store(true, Ordering::Release);
        if let Some(watcher) = self.cancellation_watcher.take() {
            let _ = watcher.join();
        }
        self.launch.take();
        Ok(text)
    }
}

impl Drop for SherpaStream {
    fn drop(&mut self) {
        if let Some(input) = self.input.as_mut() {
            let mut header = [0_u8; 5];
            header[0] = COMMAND_CANCEL;
            let _ = input.write_all(&header).and_then(|_| input.flush());
        }
        self.terminate();
    }
}

fn read_helper_frames(
    mut output: impl Read,
    sender: mpsc::SyncSender<Result<HelperFrame, EngineFailure>>,
) {
    loop {
        let mut header = [0_u8; 5];
        if output.read_exact(&mut header).is_err() {
            let _ = sender.send(Err(EngineFailure::Inference));
            return;
        }
        let length = u32::from_le_bytes(header[1..].try_into().unwrap()) as usize;
        if length > MAX_FRAME_BYTES {
            let _ = sender.send(Err(EngineFailure::Inference));
            return;
        }
        let mut payload = vec![0_u8; length];
        if output.read_exact(&mut payload).is_err() {
            let _ = sender.send(Err(EngineFailure::Inference));
            return;
        }
        let text = || String::from_utf8(payload.clone()).map_err(|_| EngineFailure::Inference);
        let frame = match header[0] {
            RESPONSE_READY if payload.is_empty() => Ok(HelperFrame::Ready),
            RESPONSE_PARTIAL => text().map(HelperFrame::Partial),
            RESPONSE_FINAL => text().map(HelperFrame::Final),
            RESPONSE_CLOSED if payload.is_empty() => Ok(HelperFrame::Closed),
            _ => Err(EngineFailure::Inference),
        };
        if sender.send(frame).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_file(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"fixture").unwrap();
        #[cfg(unix)]
        if path.file_name().is_some_and(|name| name == "helper") {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
    }

    #[test]
    fn installation_requires_the_exact_pinned_manifest_and_files() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture
            .path()
            .join(format!("voice/sherpa-onnx-{ENGINE_VERSION}"));
        let helper = fixture.path().join("helper");
        write_file(&helper);
        write_file(&root.join("runtime/lib/libsherpa-onnx-c-api.dylib"));
        for file in [
            "encoder.int8.onnx",
            "decoder.int8.onnx",
            "joiner.int8.onnx",
            "tokens.txt",
        ] {
            write_file(&root.join("model").join(file));
        }
        fs::write(
            root.join("install.json"),
            serde_json::json!({
                "schemaVersion": 1,
                "engineVersion": ENGINE_VERSION,
                "modelId": MODEL_ID,
                "runtimeArchiveSha256": RUNTIME_ARCHIVE_SHA256,
                "modelArchiveSha256": MODEL_ARCHIVE_SHA256,
            })
            .to_string(),
        )
        .unwrap();

        let manifest: InstallManifest =
            serde_json::from_slice(&fs::read(root.join("install.json")).unwrap()).unwrap();
        assert!(manifest.is_expected());
        let provider = SherpaProvider {
            helper,
            runtime: root.join("runtime"),
            model: root.join("model"),
            supervised: false,
        };
        assert!(provider.paths_exist());
        assert_eq!(provider.supported_locales(), ["en-US"]);

        fs::remove_file(root.join("model/tokens.txt")).unwrap();
        assert!(!provider.paths_exist());
    }

    #[test]
    fn helper_reader_rejects_oversized_frames_before_allocating_payload() {
        let mut bytes = vec![RESPONSE_PARTIAL];
        bytes.extend_from_slice(&((MAX_FRAME_BYTES + 1) as u32).to_le_bytes());
        let (sender, receiver) = mpsc::sync_channel(1);
        read_helper_frames(bytes.as_slice(), sender);
        assert!(matches!(
            receiver.recv().unwrap(),
            Err(EngineFailure::Inference)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_kills_and_reaps_a_blocked_helper() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = tempfile::tempdir().unwrap();
        let helper = fixture.path().join("helper.sh");
        fs::write(
            &helper,
            b"#!/bin/sh\nprintf '\\200\\000\\000\\000\\000'\nsleep 30\n",
        )
        .unwrap();
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
        let provider = SherpaProvider {
            helper,
            runtime: fixture.path().join("runtime"),
            model: fixture.path().join("model"),
            supervised: false,
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut stream = SherpaStream::spawn(&provider, cancelled.clone()).unwrap();
        let pid = stream.child.as_ref().unwrap().id();

        cancelled.store(true, Ordering::Release);
        assert!(stream.append(&[0]).is_err());
        assert!(stream.child.is_none());
        assert!(adapters::process_identity(pid).is_none());
    }
}
