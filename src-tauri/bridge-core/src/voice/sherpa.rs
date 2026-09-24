//! Supervised bridge to the optional sherpa-onnx local speech installation.
//!
//! Detection is metadata-only. It never loads a model or touches the network;
//! the native helper and model are opened lazily after an explicit voice start.

use super::local::{EngineFailure, LocalVoiceService, VoiceProvider, VoiceStream};
use crate::{adapters, process_ledger, BridgeError};
use bridge_protocol::messages as wire;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::thread;
use std::time::Duration;

pub const ENGINE_VERSION: &str = "1.13.8";
pub const MODEL_ID: &str = "nemotron-speech-streaming-en-0.6b-560ms-int8-2026-04-25";
pub const RUNTIME_ARCHIVE_SHA256: &str =
    "91b96512c4fa1960f8a9ed5360a6c8dda53a4b5015d0590244f14086a234557a";
pub const MODEL_ARCHIVE_SHA256: &str =
    "78e2b79fcf7271553a74402a76b771b09ea40117a39566a79f52235b23db6358";
pub const RUNTIME_ARCHIVE_BYTES: u64 = 18_252_168;
pub const MODEL_ARCHIVE_BYTES: u64 = 463_945_051;
pub const DOWNLOAD_BYTES: u64 = RUNTIME_ARCHIVE_BYTES + MODEL_ARCHIVE_BYTES;
pub const INSTALLED_BYTES: u64 = 662 * 1024 * 1024;

const RUNTIME_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.8/sherpa-onnx-v1.13.8-osx-arm64-shared-no-tts.tar.bz2";
const MODEL_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemotron-speech-streaming-en-0.6b-560ms-int8-2026-04-25.tar.bz2";
const RUNTIME_DIRECTORY: &str = "sherpa-onnx-v1.13.8-osx-arm64-shared-no-tts";
const MODEL_DIRECTORY: &str = "sherpa-onnx-nemotron-speech-streaming-en-0.6b-560ms-int8-2026-04-25";

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InstallManifest {
    schema_version: u32,
    install_id: String,
    engine_version: String,
    model_id: String,
    runtime_archive_sha256: String,
    model_archive_sha256: String,
}

impl InstallManifest {
    fn is_expected(&self) -> bool {
        self.schema_version == 1
            && uuid::Uuid::parse_str(&self.install_id).is_ok()
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

        let root = data_dir.join("voice/local");
        let manifest: InstallManifest =
            serde_json::from_slice(&fs::read(root.join("active.json")).ok()?).ok()?;
        if !manifest.is_expected() {
            return None;
        }
        let install = root.join("installs").join(&manifest.install_id);
        let installed_manifest: InstallManifest =
            serde_json::from_slice(&fs::read(install.join("install.json")).ok()?).ok()?;
        if installed_manifest.install_id != manifest.install_id || !installed_manifest.is_expected()
        {
            return None;
        }
        let provider = Self {
            helper: resolve_helper()?,
            runtime: install.join("runtime"),
            model: install.join("model"),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetupPhase {
    NotInstalled,
    DownloadingRuntime,
    DownloadingModel,
    Installing,
    Ready,
    Failed,
    Unsupported,
}

#[derive(Debug, Clone)]
struct SetupSnapshot {
    phase: SetupPhase,
    downloaded_bytes: u64,
    reason: Option<String>,
}

impl SetupSnapshot {
    fn status(&self) -> wire::VoiceLocalStatusResult {
        wire::VoiceLocalStatusResult {
            state: match self.phase {
                SetupPhase::NotInstalled => wire::VoiceLocalSetupState::NotInstalled,
                SetupPhase::DownloadingRuntime => wire::VoiceLocalSetupState::DownloadingRuntime,
                SetupPhase::DownloadingModel => wire::VoiceLocalSetupState::DownloadingModel,
                SetupPhase::Installing => wire::VoiceLocalSetupState::Installing,
                SetupPhase::Ready => wire::VoiceLocalSetupState::Ready,
                SetupPhase::Failed => wire::VoiceLocalSetupState::Failed,
                SetupPhase::Unsupported => wire::VoiceLocalSetupState::Unsupported,
            },
            engine_version: ENGINE_VERSION.into(),
            model_id: MODEL_ID.into(),
            locale: "en-US".into(),
            download_bytes: DOWNLOAD_BYTES,
            installed_bytes: INSTALLED_BYTES,
            downloaded_bytes: self.downloaded_bytes.min(DOWNLOAD_BYTES),
            reason: self.reason.clone(),
        }
    }
}

#[derive(Clone)]
pub struct InstallManager {
    root: Option<PathBuf>,
    state: Arc<Mutex<SetupSnapshot>>,
}

impl Default for InstallManager {
    fn default() -> Self {
        Self {
            root: None,
            state: Arc::new(Mutex::new(SetupSnapshot {
                phase: SetupPhase::Unsupported,
                downloaded_bytes: 0,
                reason: Some("Local dictation setup is unavailable in this runtime".into()),
            })),
        }
    }
}

impl InstallManager {
    pub fn new(data_dir: PathBuf) -> Self {
        let supported = cfg!(all(target_os = "macos", target_arch = "aarch64"));
        let installed = supported && installed_provider(&data_dir).is_some();
        let (phase, reason) = if !supported {
            (
                SetupPhase::Unsupported,
                Some(
                    "The evaluated local speech runtime currently supports Apple silicon Macs"
                        .into(),
                ),
            )
        } else if installed {
            (SetupPhase::Ready, None)
        } else {
            (SetupPhase::NotInstalled, None)
        };
        Self {
            root: Some(data_dir.join("voice/local")),
            state: Arc::new(Mutex::new(SetupSnapshot {
                phase,
                downloaded_bytes: if installed { DOWNLOAD_BYTES } else { 0 },
                reason,
            })),
        }
    }

    pub fn status(&self) -> wire::VoiceLocalStatusResult {
        self.state.lock().unwrap().status()
    }

    pub fn start(
        &self,
        local: &LocalVoiceService,
    ) -> Result<wire::VoiceLocalStatusResult, BridgeError> {
        let Some(root) = self.root.clone() else {
            return Err(BridgeError::Invalid(
                "Local dictation setup is unavailable in this runtime".into(),
            ));
        };
        if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            return Err(BridgeError::Invalid(
                "The evaluated local speech runtime currently supports Apple silicon Macs".into(),
            ));
        }
        if resolve_helper().is_none() {
            return Err(BridgeError::Invalid(
                "The packaged local dictation helper is unavailable".into(),
            ));
        }
        {
            let mut state = self.state.lock().unwrap();
            if matches!(
                state.phase,
                SetupPhase::DownloadingRuntime
                    | SetupPhase::DownloadingModel
                    | SetupPhase::Installing
                    | SetupPhase::Ready
            ) {
                return Ok(state.status());
            }
            state.phase = SetupPhase::DownloadingRuntime;
            state.downloaded_bytes = 0;
            state.reason = None;
        }
        let manager = self.clone();
        let provider_slot = local.provider_slot();
        if thread::Builder::new()
            .name("bridge-voice-install".into())
            .spawn(move || match manager.install(&root) {
                Ok(provider) => {
                    *provider_slot.lock().unwrap() = Some(Arc::new(provider));
                    *manager.state.lock().unwrap() = SetupSnapshot {
                        phase: SetupPhase::Ready,
                        downloaded_bytes: DOWNLOAD_BYTES,
                        reason: None,
                    };
                }
                Err(reason) => {
                    *manager.state.lock().unwrap() = SetupSnapshot {
                        phase: SetupPhase::Failed,
                        downloaded_bytes: 0,
                        reason: Some(reason),
                    };
                }
            })
            .is_err()
        {
            *self.state.lock().unwrap() = SetupSnapshot {
                phase: SetupPhase::Failed,
                downloaded_bytes: 0,
                reason: Some("Could not start the local dictation installer".into()),
            };
            return Err(BridgeError::Invalid(
                "Could not start the local dictation installer".into(),
            ));
        }
        Ok(self.status())
    }

    pub fn remove(
        &self,
        local: &LocalVoiceService,
    ) -> Result<wire::VoiceLocalStatusResult, BridgeError> {
        if local.is_busy() {
            return Err(BridgeError::Invalid(
                "Finish or cancel dictation before removing the local model".into(),
            ));
        }
        {
            let state = self.state.lock().unwrap();
            if matches!(
                state.phase,
                SetupPhase::DownloadingRuntime
                    | SetupPhase::DownloadingModel
                    | SetupPhase::Installing
            ) {
                return Err(BridgeError::Invalid(
                    "Local dictation setup is still in progress".into(),
                ));
            }
        }
        let Some(root) = self.root.as_ref() else {
            return Err(BridgeError::Invalid(
                "Local dictation setup is unavailable in this runtime".into(),
            ));
        };
        let active_path = root.join("active.json");
        let active = fs::read(&active_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<InstallManifest>(&bytes).ok());
        let removal = (|| -> Result<(), BridgeError> {
            if active_path.exists() {
                fs::remove_file(&active_path)?;
            }
            if let Some(active) = active.filter(InstallManifest::is_expected) {
                let install = root.join("installs").join(active.install_id);
                if install.is_dir() {
                    fs::remove_dir_all(install)?;
                }
            }
            Ok(())
        })();
        local.set_provider(None);
        if let Err(error) = removal {
            *self.state.lock().unwrap() = SetupSnapshot {
                phase: SetupPhase::Failed,
                downloaded_bytes: 0,
                reason: Some("Could not completely remove the local dictation installation".into()),
            };
            return Err(error);
        }
        *self.state.lock().unwrap() = SetupSnapshot {
            phase: SetupPhase::NotInstalled,
            downloaded_bytes: 0,
            reason: None,
        };
        Ok(self.status())
    }

    fn install(&self, root: &Path) -> Result<SherpaProvider, String> {
        let previous = fs::read(root.join("active.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<InstallManifest>(&bytes).ok())
            .filter(InstallManifest::is_expected);
        fs::create_dir_all(root.join("installs"))
            .map_err(|_| "Could not prepare local dictation storage".to_owned())?;
        let install_id = uuid::Uuid::new_v4().to_string();
        let staging = root.join("installs").join(format!(".{install_id}.staging"));
        let final_path = root.join("installs").join(&install_id);
        fs::create_dir(&staging)
            .map_err(|_| "Could not prepare local dictation staging".to_owned())?;
        let mut cleanup = StagingCleanup(Some(staging.clone()));
        let runtime_archive = staging.join("runtime.tar.bz2");
        let model_archive = staging.join("model.tar.bz2");
        self.download(
            RUNTIME_URL,
            &runtime_archive,
            RUNTIME_ARCHIVE_BYTES,
            RUNTIME_ARCHIVE_SHA256,
            SetupPhase::DownloadingRuntime,
            0,
        )?;
        self.download(
            MODEL_URL,
            &model_archive,
            MODEL_ARCHIVE_BYTES,
            MODEL_ARCHIVE_SHA256,
            SetupPhase::DownloadingModel,
            RUNTIME_ARCHIVE_BYTES,
        )?;
        {
            let mut state = self.state.lock().unwrap();
            state.phase = SetupPhase::Installing;
            state.downloaded_bytes = DOWNLOAD_BYTES;
        }
        extract_archive(
            &runtime_archive,
            &staging.join("runtime-extract"),
            RUNTIME_DIRECTORY,
            &staging.join("runtime"),
        )?;
        extract_archive(
            &model_archive,
            &staging.join("model-extract"),
            MODEL_DIRECTORY,
            &staging.join("model"),
        )?;
        let _ = fs::remove_file(&runtime_archive);
        let _ = fs::remove_file(&model_archive);
        let manifest = InstallManifest {
            schema_version: 1,
            install_id: install_id.clone(),
            engine_version: ENGINE_VERSION.into(),
            model_id: MODEL_ID.into(),
            runtime_archive_sha256: RUNTIME_ARCHIVE_SHA256.into(),
            model_archive_sha256: MODEL_ARCHIVE_SHA256.into(),
        };
        fs::write(
            staging.join("install.json"),
            serde_json::to_vec_pretty(&manifest)
                .map_err(|_| "Could not record local dictation installation")?,
        )
        .map_err(|_| "Could not record local dictation installation".to_owned())?;
        let provider = SherpaProvider {
            helper: resolve_helper().ok_or("The packaged local dictation helper is unavailable")?,
            runtime: staging.join("runtime"),
            model: staging.join("model"),
            supervised: true,
        };
        if !provider.paths_exist() {
            return Err("The downloaded local dictation files are incomplete".into());
        }
        fs::rename(&staging, &final_path)
            .map_err(|_| "Could not activate the local dictation installation".to_owned())?;
        cleanup.0 = None;
        write_active_manifest(root, &manifest)?;
        if let Some(previous) = previous.filter(|item| item.install_id != install_id) {
            let previous_path = root.join("installs").join(previous.install_id);
            if previous_path.is_dir() {
                let _ = fs::remove_dir_all(previous_path);
            }
        }
        let provider = SherpaProvider {
            helper: resolve_helper().ok_or("The packaged local dictation helper is unavailable")?,
            runtime: final_path.join("runtime"),
            model: final_path.join("model"),
            supervised: true,
        };
        if !provider.paths_exist() {
            return Err("The activated local dictation installation is incomplete".into());
        }
        Ok(provider)
    }

    #[allow(clippy::too_many_arguments)]
    fn download(
        &self,
        url: &str,
        destination: &Path,
        expected_bytes: u64,
        expected_sha256: &str,
        phase: SetupPhase,
        completed_bytes: u64,
    ) -> Result<(), String> {
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(15 * 60))
            .user_agent(concat!("Bridge/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| "Could not initialize the local dictation download".to_owned())?;
        let mut response = client
            .get(url)
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .map_err(|_| "Could not download the local dictation files".to_owned())?;
        if response
            .content_length()
            .is_some_and(|length| length != expected_bytes)
        {
            return Err("The local dictation download size did not match its pin".into());
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .map_err(|_| "Could not create the local dictation download".to_owned())?;
        copy_verified(&mut response, &mut file, expected_bytes, expected_sha256, |downloaded| {
            let mut state = self.state.lock().unwrap();
            state.phase = phase;
            state.downloaded_bytes = completed_bytes + downloaded;
        })?;
        file.sync_all()
            .map_err(|_| "Could not finish storing the local dictation download".to_owned())?;
        Ok(())
    }
}

fn copy_verified(
    mut source: impl Read,
    mut destination: impl Write,
    expected_bytes: u64,
    expected_sha256: &str,
    mut progress: impl FnMut(u64),
) -> Result<(), String> {
    let mut digest = Sha256::new();
    let mut downloaded = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = source
            .read(&mut buffer)
            .map_err(|_| "The local dictation download was interrupted".to_owned())?;
        if count == 0 {
            break;
        }
        downloaded = downloaded.saturating_add(count as u64);
        if downloaded > expected_bytes {
            return Err("The local dictation download exceeded its pinned size".into());
        }
        digest.update(&buffer[..count]);
        destination
            .write_all(&buffer[..count])
            .map_err(|_| "Could not store the local dictation download".to_owned())?;
        progress(downloaded);
    }
    if downloaded != expected_bytes || format!("{:x}", digest.finalize()) != expected_sha256 {
        return Err("The local dictation download failed checksum verification".into());
    }
    Ok(())
}

struct StagingCleanup(Option<PathBuf>);

impl Drop for StagingCleanup {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = fs::remove_dir_all(path);
        }
    }
}

fn extract_archive(
    archive_path: &Path,
    extraction_root: &Path,
    expected_directory: &str,
    destination: &Path,
) -> Result<(), String> {
    fs::create_dir(extraction_root)
        .map_err(|_| "Could not prepare local dictation extraction".to_owned())?;
    let archive = File::open(archive_path)
        .map_err(|_| "Could not open the verified local dictation archive".to_owned())?;
    let decoder = bzip2::read::BzDecoder::new(archive);
    let mut archive = tar::Archive::new(decoder);
    archive
        .unpack(extraction_root)
        .map_err(|_| "Could not extract the verified local dictation archive".to_owned())?;
    let source = extraction_root.join(expected_directory);
    if !source.is_dir() {
        return Err("The verified local dictation archive has an unexpected layout".into());
    }
    fs::rename(source, destination)
        .map_err(|_| "Could not stage the local dictation files".to_owned())?;
    fs::remove_dir_all(extraction_root)
        .map_err(|_| "Could not finish local dictation extraction".to_owned())?;
    Ok(())
}

fn write_active_manifest(root: &Path, manifest: &InstallManifest) -> Result<(), String> {
    let pending = root.join(format!(".active-{}.tmp", manifest.install_id));
    let mut cleanup = PendingFileCleanup(Some(pending.clone()));
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|_| "Could not record the active local dictation model".to_owned())?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)
        .map_err(|_| "Could not record the active local dictation model".to_owned())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "Could not record the active local dictation model".to_owned())?;
    fs::rename(&pending, root.join("active.json"))
        .map_err(|_| "Could not activate the local dictation model".to_owned())?;
    cleanup.0 = None;
    Ok(())
}

struct PendingFileCleanup(Option<PathBuf>);

impl Drop for PendingFileCleanup {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = fs::remove_file(path);
        }
    }
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
        let install_id = uuid::Uuid::new_v4().to_string();
        let root = fixture
            .path()
            .join("voice/local/installs")
            .join(&install_id);
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
                "installId": install_id,
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

    #[test]
    fn verified_copy_enforces_size_and_digest_before_accepting_download() {
        let payload = b"pinned voice archive";
        let digest = format!("{:x}", Sha256::digest(payload));
        let mut stored = Vec::new();
        let mut progress = Vec::new();
        copy_verified(payload.as_slice(), &mut stored, payload.len() as u64, &digest, |bytes| progress.push(bytes)).unwrap();
        assert_eq!(stored, payload);
        assert_eq!(progress.last().copied(), Some(payload.len() as u64));

        let oversized = copy_verified(payload.as_slice(), Vec::new(), 4, &digest, |_| {}).unwrap_err();
        assert!(oversized.contains("exceeded"));
        let wrong_digest = copy_verified(payload.as_slice(), Vec::new(), payload.len() as u64, &"0".repeat(64), |_| {}).unwrap_err();
        assert!(wrong_digest.contains("checksum"));
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
