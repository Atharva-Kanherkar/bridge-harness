//! Disk space across the whole Mac, not only Bridge's own worktrees.
//!
//! A daemon connection answers one request at a time, and walking a home
//! folder can take minutes, so no request here waits on a walk. Listings read
//! one directory, fill in whatever sizes are already known, and queue the rest
//! for a small pool of background walkers. The client asks again and the
//! numbers arrive as they land. Each walk also keeps the totals of the folders
//! a few levels below the one it measured, so drilling in is usually instant.
//!
//! Deleting moves to the Trash by default. Bridge refuses a short list of
//! places where a deletion breaks the machine or Bridge itself; everything
//! else in the home folder and `/Applications` is the person's to decide.

use bridge_protocol::messages as wire;
use std::collections::{HashMap, HashSet, VecDeque};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

const WORKERS: usize = 3;
const FRESH_FOR: Duration = Duration::from_secs(15 * 60);
/// How many levels below a measured folder keep their own totals.
const RECORD_DEPTH: usize = 3;
/// A listing returns at most this many children, largest first.
const LISTING_LIMIT: usize = 400;

/// Never walked: other volumes, and the APFS paths that reach the data volume
/// a second time (the root already reaches it through firmlinks).
const SKIPPED: &[&str] = &["/System/Volumes", "/Volumes", "/dev", "/net", "/home", "/Network", "/.vol"];

#[derive(Debug, Clone, Copy)]
struct Measured {
    bytes: u64,
    items: u64,
    partial: bool,
    at: Instant,
}

#[derive(Default)]
struct State {
    sizes: HashMap<PathBuf, Measured>,
    queue: VecDeque<PathBuf>,
    pending: HashSet<PathBuf>,
    started: bool,
}

/// The background measurer. One per process: every client sees the same
/// cache, and a second window does not start a second walk of the same disk.
pub struct Scanner {
    state: Mutex<State>,
    wake: Condvar,
}

impl Scanner {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { state: Mutex::new(State::default()), wake: Condvar::new() })
    }

    pub fn global() -> Arc<Self> {
        static GLOBAL: OnceLock<Arc<Scanner>> = OnceLock::new();
        GLOBAL.get_or_init(Scanner::new).clone()
    }

    /// The last measurement of `path`, and whether a new one is pending.
    /// A missing or stale measurement is queued; `urgent` puts it first.
    fn lookup(self: &Arc<Self>, path: &Path, urgent: bool) -> (Option<Measured>, bool) {
        let mut state = self.state.lock().unwrap_or_else(|poison| poison.into_inner());
        let known = state.sizes.get(path).copied();
        let fresh = known.is_some_and(|measured| measured.at.elapsed() < FRESH_FOR);
        if fresh {
            return (known, false);
        }
        if state.pending.insert(path.to_path_buf()) {
            if urgent {
                state.queue.push_front(path.to_path_buf());
            } else {
                state.queue.push_back(path.to_path_buf());
            }
        } else if urgent {
            // Already queued behind something the person stopped looking at.
            if let Some(index) = state.queue.iter().position(|queued| queued == path) {
                let queued = state.queue.remove(index).expect("index is in range");
                state.queue.push_front(queued);
            }
        }
        if !state.started {
            state.started = true;
            for index in 0..WORKERS {
                let scanner = Arc::clone(self);
                let _ = std::thread::Builder::new()
                    .name(format!("bridge-disk-scan-{index}"))
                    // Recursion follows folder depth; node_modules can nest deep.
                    .stack_size(16 * 1024 * 1024)
                    .spawn(move || scanner.work());
            }
        }
        drop(state);
        self.wake.notify_one();
        (known, true)
    }

    fn work(self: Arc<Self>) {
        loop {
            let path = {
                let mut state = self.state.lock().unwrap_or_else(|poison| poison.into_inner());
                loop {
                    if let Some(path) = state.queue.pop_front() {
                        break path;
                    }
                    state = self.wake.wait(state).unwrap_or_else(|poison| poison.into_inner());
                }
            };
            let mut recorded = Vec::new();
            let mut seen = HashSet::new();
            let (bytes, items, partial) = self.walk(&path, 0, &mut seen, &mut recorded);
            let mut state = self.state.lock().unwrap_or_else(|poison| poison.into_inner());
            let at = Instant::now();
            for (child, measured) in recorded {
                state.sizes.insert(child, measured);
            }
            state.sizes.insert(path.clone(), Measured { bytes, items, partial, at });
            state.pending.remove(&path);
        }
    }

    /// Allocated bytes under `path`, without following symlinks, counting a
    /// hard-linked file once.
    fn walk(
        &self,
        path: &Path,
        depth: usize,
        seen: &mut HashSet<(u64, u64)>,
        recorded: &mut Vec<(PathBuf, Measured)>,
    ) -> (u64, u64, bool) {
        let Ok(meta) = std::fs::symlink_metadata(path) else { return (0, 0, true) };
        let own = meta.blocks() * 512;
        if !meta.is_dir() {
            if meta.nlink() > 1 && !seen.insert((meta.dev(), meta.ino())) {
                return (0, 1, false);
            }
            return (own, 1, false);
        }
        if is_skipped(path) {
            return (0, 0, false);
        }
        // A fresh measurement of a subfolder stands in for walking it again.
        if depth > 0 {
            let state = self.state.lock().unwrap_or_else(|poison| poison.into_inner());
            if let Some(measured) = state.sizes.get(path).filter(|measured| measured.at.elapsed() < FRESH_FOR) {
                return (measured.bytes, measured.items, measured.partial);
            }
        }
        let Ok(children) = std::fs::read_dir(path) else { return (own, 1, true) };
        let (mut bytes, mut items, mut partial) = (own, 1, false);
        for child in children {
            let Ok(child) = child else {
                partial = true;
                continue;
            };
            let (child_bytes, child_items, child_partial) = self.walk(&child.path(), depth + 1, seen, recorded);
            bytes += child_bytes;
            items += child_items;
            partial |= child_partial;
        }
        if depth > 0 && depth <= RECORD_DEPTH {
            recorded.push((path.to_path_buf(), Measured { bytes, items, partial, at: Instant::now() }));
        }
        (bytes, items, partial)
    }

    /// Forget every measurement at or under `path` so the next look walks it.
    fn forget(&self, path: &Path) {
        let mut state = self.state.lock().unwrap_or_else(|poison| poison.into_inner());
        state.sizes.retain(|known, _| !known.starts_with(path));
    }

    /// `path` is gone: drop its measurements and take its size off every
    /// folder above it, which is cheaper and truer than re-walking them.
    fn removed(&self, path: &Path, bytes: u64) {
        let mut state = self.state.lock().unwrap_or_else(|poison| poison.into_inner());
        state.sizes.retain(|known, _| !known.starts_with(path));
        for ancestor in path.ancestors().skip(1) {
            if let Some(measured) = state.sizes.get_mut(ancestor) {
                measured.bytes = measured.bytes.saturating_sub(bytes);
            }
        }
    }

    fn known_bytes(&self, path: &Path) -> Option<u64> {
        let state = self.state.lock().unwrap_or_else(|poison| poison.into_inner());
        state.sizes.get(path).map(|measured| measured.bytes)
    }
}

fn is_skipped(path: &Path) -> bool {
    SKIPPED.iter().any(|skipped| path == Path::new(skipped))
}

/// The places Bridge will not delete, and why.
#[derive(Debug, Clone)]
pub struct Guard {
    pub home: PathBuf,
    pub data_dir: PathBuf,
    pub worktrees: PathBuf,
}

/// Folders macOS and apps expect to exist. Their contents are fair game.
const STANDARD_FOLDERS: &[&str] = &["Applications", "Desktop", "Documents", "Downloads", "Library", "Movies", "Music", "Pictures", "Public"];
const LIBRARY_ROOTS: &[&str] = &[
    "Application Support",
    "CloudStorage",
    "Containers",
    "Group Containers",
    "Mail",
    "Messages",
    "Mobile Documents",
    "Preferences",
];

impl Guard {
    /// The same guard with its own paths resolved, for comparing against a
    /// resolved target (`/var` is `/private/var` once links are followed).
    fn resolved(&self) -> Guard {
        let real = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        Guard { home: real(&self.home), data_dir: real(&self.data_dir), worktrees: real(&self.worktrees) }
    }

    pub fn protected_reason(&self, path: &Path) -> Option<String> {
        if !path.is_absolute() || path.components().any(|part| matches!(part, Component::ParentDir | Component::CurDir)) {
            return Some("Not a plain absolute path.".into());
        }
        if self.home.starts_with(path) {
            return Some("Your home folder or one of its parents.".into());
        }
        if self.data_dir.starts_with(path) || path.starts_with(&self.data_dir) {
            return Some("Bridge's own data. Deleting it would break Bridge.".into());
        }
        if path.starts_with(&self.worktrees) {
            return Some("A Bridge worktree. Reclaim it under Worktrees so its safety checks run.".into());
        }
        if let Ok(relative) = path.strip_prefix(&self.home) {
            let parts: Vec<&str> = relative.iter().filter_map(|part| part.to_str()).collect();
            if parts.iter().any(|part| matches!(*part, ".ssh" | ".gnupg")) {
                return Some("Holds your keys.".into());
            }
            if parts.starts_with(&["Library", "Keychains"]) {
                return Some("Holds your passwords and certificates.".into());
            }
            match parts.as_slice() {
                [".Trash"] => return Some("The Trash itself. Use Empty Trash.".into()),
                [folder] if STANDARD_FOLDERS.contains(folder) => {
                    return Some("A standard macOS folder. Delete what is inside it instead.".into())
                }
                ["Library", folder] if LIBRARY_ROOTS.contains(folder) => {
                    return Some("Apps keep their settings and data here. Delete what is inside it instead.".into())
                }
                _ => return None,
            }
        }
        if path.starts_with("/Applications") && path != Path::new("/Applications") {
            return None;
        }
        Some("Outside your home folder. Bridge leaves system files alone.".into())
    }
}

fn kind(meta: &std::fs::Metadata, path: &Path) -> &'static str {
    if meta.file_type().is_symlink() {
        "symlink"
    } else if meta.is_dir() {
        let bundle = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| matches!(extension, "app" | "bundle" | "framework" | "photoslibrary" | "musiclibrary" | "xcarchive"));
        if bundle { "package" } else { "directory" }
    } else {
        "file"
    }
}

fn timestamp(time: std::io::Result<SystemTime>) -> Option<String> {
    time.ok().map(|time| chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339())
}

/// One folder's children, largest first, with the sizes known so far.
pub fn list(scanner: &Arc<Scanner>, guard: &Guard, path: &Path, refresh: bool) -> wire::DiskListing {
    if refresh {
        scanner.forget(path);
    }
    let parent = path.parent().map(|parent| parent.display().to_string());
    let mut listing = wire::DiskListing {
        path: path.display().to_string(),
        parent,
        size_bytes: 0,
        measuring: false,
        unreadable: None,
        entries: Vec::new(),
        omitted_count: 0,
        omitted_bytes: 0,
    };
    let children = match std::fs::read_dir(path) {
        Ok(children) => children,
        Err(error) => {
            listing.unreadable = Some(match error.kind() {
                std::io::ErrorKind::PermissionDenied => {
                    "macOS keeps this folder private from Bridge. Finder can still show it.".into()
                }
                _ => error.to_string(),
            });
            return listing;
        }
    };
    for child in children.flatten() {
        let child_path = child.path();
        if is_skipped(&child_path) {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(&child_path) else { continue };
        let kind = kind(&meta, &child_path);
        let (size_bytes, item_count, measuring, partial) = if meta.is_dir() {
            let (known, measuring) = scanner.lookup(&child_path, true);
            (known.map(|measured| measured.bytes), known.map(|measured| measured.items), measuring, known.is_some_and(|measured| measured.partial))
        } else {
            (Some(meta.blocks() * 512), Some(1), false, false)
        };
        listing.measuring |= measuring;
        listing.size_bytes += size_bytes.unwrap_or(0);
        listing.entries.push(wire::DiskEntry {
            name: child.file_name().to_string_lossy().into_owned(),
            path: child_path.display().to_string(),
            kind: kind.into(),
            size_bytes,
            item_count,
            measuring,
            partial,
            modified_at: timestamp(meta.modified()),
            protected_reason: guard.protected_reason(&child_path),
        });
    }
    listing.entries.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes).then_with(|| a.name.cmp(&b.name)));
    if listing.entries.len() > LISTING_LIMIT {
        let omitted = listing.entries.split_off(LISTING_LIMIT);
        listing.omitted_count = omitted.len() as u64;
        listing.omitted_bytes = omitted.iter().filter_map(|entry| entry.size_bytes).sum();
    }
    listing
}

struct Known {
    id: &'static str,
    label: &'static str,
    group: &'static str,
    relative: &'static str,
    safety: &'static str,
    description: &'static str,
}

const KNOWN: &[Known] = &[
    Known { id: "xcode-derived-data", label: "Xcode build data", group: "developer", relative: "Library/Developer/Xcode/DerivedData", safety: "safe", description: "Intermediate build products. Xcode rebuilds them on the next build." },
    Known { id: "xcode-device-support", label: "Xcode device support", group: "developer", relative: "Library/Developer/Xcode/iOS DeviceSupport", safety: "safe", description: "Debug symbols for every iOS version you have plugged in. Re-downloaded when needed." },
    Known { id: "xcode-archives", label: "Xcode archives", group: "developer", relative: "Library/Developer/Xcode/Archives", safety: "review", description: "Builds you archived for distribution. Keep any you may need to symbolicate." },
    Known { id: "simulators", label: "Simulator devices", group: "developer", relative: "Library/Developer/CoreSimulator/Devices", safety: "review", description: "Every simulator and the apps on it. `xcrun simctl delete unavailable` removes only the stale ones." },
    Known { id: "simulator-caches", label: "Simulator caches", group: "developer", relative: "Library/Developer/CoreSimulator/Caches", safety: "safe", description: "Dyld and runtime caches. Simulators rebuild them." },
    Known { id: "docker", label: "Docker disk image", group: "developer", relative: "Library/Containers/com.docker.docker/Data/vms", safety: "review", description: "All images, containers, and volumes. `docker system prune` is gentler than deleting it." },
    Known { id: "android-avd", label: "Android emulators", group: "developer", relative: ".android/avd", safety: "review", description: "Android virtual devices and their data." },
    Known { id: "rustup", label: "Rust toolchains", group: "developer", relative: ".rustup/toolchains", safety: "review", description: "Installed Rust toolchains. Old nightlies are usually safe to drop." },
    Known { id: "ollama", label: "Ollama models", group: "developer", relative: ".ollama/models", safety: "review", description: "Downloaded local models. `ollama rm` removes one at a time." },
    Known { id: "npm", label: "npm cache", group: "caches", relative: ".npm", safety: "safe", description: "Package tarballs npm can download again." },
    Known { id: "bun", label: "Bun cache", group: "caches", relative: ".bun/install/cache", safety: "safe", description: "Packages Bun can download again." },
    Known { id: "pnpm", label: "pnpm store", group: "caches", relative: "Library/pnpm/store", safety: "safe", description: "The content-addressed store pnpm links from. Re-downloaded on install." },
    Known { id: "cargo-registry", label: "Cargo registry", group: "caches", relative: ".cargo/registry", safety: "safe", description: "Crate sources Cargo can fetch again." },
    Known { id: "go-mod", label: "Go module cache", group: "caches", relative: "go/pkg/mod", safety: "safe", description: "Module sources Go can fetch again." },
    Known { id: "gradle", label: "Gradle caches", group: "caches", relative: ".gradle/caches", safety: "safe", description: "Dependencies and build caches Gradle can rebuild." },
    Known { id: "maven", label: "Maven repository", group: "caches", relative: ".m2/repository", safety: "safe", description: "Artifacts Maven can download again." },
    Known { id: "xdg-cache", label: "Tool caches", group: "caches", relative: ".cache", safety: "safe", description: "Caches from command-line tools, including model downloads." },
    Known { id: "app-caches", label: "App caches", group: "caches", relative: "Library/Caches", safety: "review", description: "Every app's cache. Apps rebuild them, though a few sign you out." },
    Known { id: "logs", label: "Logs", group: "caches", relative: "Library/Logs", safety: "safe", description: "Diagnostic logs from apps and the system." },
    Known { id: "downloads", label: "Downloads", group: "files", relative: "Downloads", safety: "review", description: "Installers and files you downloaded. Often the biggest forgotten pile." },
    Known { id: "ios-backups", label: "iPhone and iPad backups", group: "files", relative: "Library/Application Support/MobileSync/Backup", safety: "review", description: "Local device backups. Keep the latest if you have no iCloud backup." },
];

pub fn overview(scanner: &Arc<Scanner>, home: &Path) -> wire::DiskOverview {
    let mut measuring = false;
    let suggestions = KNOWN
        .iter()
        .filter_map(|known| {
            let path = home.join(known.relative);
            std::fs::symlink_metadata(&path).ok().filter(|meta| meta.is_dir())?;
            let (measured, pending) = scanner.lookup(&path, false);
            measuring |= pending;
            Some(wire::DiskSuggestion {
                id: known.id.into(),
                label: known.label.into(),
                group: known.group.into(),
                description: known.description.into(),
                path: path.display().to_string(),
                size_bytes: measured.map(|measured| measured.bytes),
                measuring: pending,
                safety: known.safety.into(),
            })
        })
        .collect();
    wire::DiskOverview { volume: volume(home), home: home.display().to_string(), suggestions, measuring }
}

fn volume(path: &Path) -> Option<wire::DiskVolume> {
    use std::os::unix::ffi::OsStrExt;
    let path_c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: statvfs fills the zeroed struct and reads only the C string.
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path_c.as_ptr(), &mut stats) } != 0 {
        return None;
    }
    let block = stats.f_frsize as u64;
    let total_bytes = stats.f_blocks as u64 * block;
    let free_bytes = stats.f_bavail as u64 * block;
    Some(wire::DiskVolume {
        mount_point: "/".into(),
        total_bytes,
        free_bytes,
        used_bytes: total_bytes.saturating_sub(free_bytes),
    })
}

/// Delete or trash each path the guard allows. One refusal does not stop the
/// rest; it comes back with its reason.
///
/// The text of a path can sit under the home folder while a symlinked folder
/// inside it leads anywhere, and that folder can be swapped between a check
/// and a delete. So the folder holding each target is opened once, the guard
/// is asked about where that handle really is, and the delete or move then
/// happens relative to the handle: nothing is resolved by path again.
pub fn delete(scanner: &Arc<Scanner>, guard: &Guard, paths: &[String], permanent: bool) -> wire::DiskDeleteResult {
    let mut result = wire::DiskDeleteResult { deleted: Vec::new(), failed: Vec::new(), bytes_freed: 0, trashed: !permanent };
    let resolved_guard = guard.resolved();
    for raw in paths {
        let path = PathBuf::from(raw);
        let outcome = guard
            .protected_reason(&path)
            .map_or_else(|| open_parent(guard, &path), Err)
            .and_then(|parent| match resolved_guard.protected_reason(&parent.real.join(&parent.name)) {
                Some(reason) => Err(reason),
                None => Ok(parent),
            })
            .and_then(|parent| remove_at(scanner, &parent, &path, &guard.home, permanent));
        match outcome {
            Ok(bytes) => {
                scanner.removed(&path, bytes);
                result.bytes_freed += bytes;
                result.deleted.push(raw.clone());
            }
            Err(reason) => result.failed.push(wire::DiskDeleteFailure { path: raw.clone(), reason }),
        }
    }
    result
}

/// The folder holding a target, opened once, and where it really is.
struct Parent {
    dir: cap_std::fs::Dir,
    real: PathBuf,
    name: std::ffi::OsString,
}

/// Opens the parent of `path` beneath the home folder or `/Applications`.
/// cap-std refuses any step, symlinked or not, that leaves that root.
fn open_parent(guard: &Guard, path: &Path) -> Result<Parent, String> {
    let root = if path.starts_with(&guard.home) { guard.home.clone() } else { PathBuf::from("/Applications") };
    let relative = path.strip_prefix(&root).map_err(|_| "Outside the folders Bridge may delete from.".to_string())?;
    let name = relative.file_name().ok_or_else(|| "Not a file or folder name.".to_string())?.to_owned();
    let unreachable = |error: std::io::Error| format!("Bridge could not open the folder that holds it: {error}");
    let root_dir = cap_std::fs::Dir::open_ambient_dir(&root, cap_std::ambient_authority()).map_err(unreachable)?;
    let dir = match relative.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        Some(parent) => root_dir.open_dir(parent).map_err(unreachable)?,
        None => root_dir,
    };
    let real = handle_path(&dir).ok_or_else(|| "Bridge could not tell where this path really leads.".to_string())?;
    Ok(Parent { dir, real, name })
}

/// Where an open directory handle really is, whatever path reached it.
#[cfg(target_os = "macos")]
fn handle_path(dir: &cap_std::fs::Dir) -> Option<PathBuf> {
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    let mut buffer = vec![0u8; libc::PATH_MAX as usize];
    // SAFETY: F_GETPATH writes at most PATH_MAX bytes into the buffer.
    if unsafe { libc::fcntl(dir.as_raw_fd(), libc::F_GETPATH, buffer.as_mut_ptr()) } == -1 {
        return None;
    }
    let length = buffer.iter().position(|byte| *byte == 0)?;
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(&buffer[..length])))
}

#[cfg(target_os = "linux")]
fn handle_path(dir: &cap_std::fs::Dir) -> Option<PathBuf> {
    use std::os::fd::AsRawFd;
    std::fs::read_link(format!("/proc/self/fd/{}", dir.as_raw_fd())).ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn handle_path(_dir: &cap_std::fs::Dir) -> Option<PathBuf> {
    None
}

/// Delete or trash `parent.name` relative to the handle. Returns the bytes
/// freed, as far as they were measured.
fn remove_at(scanner: &Arc<Scanner>, parent: &Parent, path: &Path, home: &Path, permanent: bool) -> Result<u64, String> {
    let meta = parent.dir.symlink_metadata(&parent.name).map_err(|_| "It no longer exists.".to_string())?;
    let bytes = if meta.is_dir() {
        scanner.known_bytes(path).unwrap_or(0)
    } else {
        std::fs::symlink_metadata(parent.real.join(&parent.name)).map(|meta| meta.blocks() * 512).unwrap_or(0)
    };
    let outcome = if !permanent {
        cap_std::fs::Dir::open_ambient_dir(home.join(".Trash"), cap_std::ambient_authority())
            .and_then(|trash| move_into(&parent.dir, &parent.name, &trash, TRASH_NAME_ATTEMPTS))
    } else if meta.is_dir() {
        // cap-std removes a tree through handles without following links.
        parent.dir.remove_dir_all(&parent.name)
    } else {
        parent.dir.remove_file(&parent.name)
    };
    outcome.map(|()| bytes).map_err(|error| describe(&error, permanent))
}

fn describe(error: &std::io::Error, permanent: bool) -> String {
    if error.raw_os_error() == Some(libc::EXDEV) {
        return "It is on another volume, so it cannot go to this Trash. Delete it permanently instead.".into();
    }
    match error.kind() {
        std::io::ErrorKind::PermissionDenied => "macOS did not allow it. Finder may be able to remove it.".into(),
        _ if permanent => format!("Could not delete it: {error}"),
        _ => format!("Could not move it to the Trash: {error}"),
    }
}

const TRASH_NAME_ATTEMPTS: usize = 1000;

/// Move `name` into the Trash under a name nothing there uses. Each attempt
/// is one atomic no-replace rename, so two moves of the same name at once can
/// never overwrite each other; a taken name just tries the next one.
fn move_into(from: &cap_std::fs::Dir, name: &std::ffi::OsStr, trash: &cap_std::fs::Dir, attempts: usize) -> std::io::Result<()> {
    let base = name.to_string_lossy().into_owned();
    let stamp = chrono::Local::now().format("%H.%M.%S").to_string();
    for attempt in 0..attempts {
        let candidate = match attempt {
            0 => base.clone(),
            1 => format!("{base} {stamp}"),
            _ => format!("{base} {stamp} {attempt}"),
        };
        match rename_no_replace(from, name, trash, std::ffi::OsStr::new(&candidate)) {
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            other => return other,
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "the Trash already holds too many items with this name"))
}

fn rename_no_replace(from: &cap_std::fs::Dir, from_name: &std::ffi::OsStr, to: &cap_std::fs::Dir, to_name: &std::ffi::OsStr) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    let from_c = std::ffi::CString::new(from_name.as_bytes())?;
    let to_c = std::ffi::CString::new(to_name.as_bytes())?;
    #[cfg(target_os = "macos")]
    // SAFETY: both names are NUL-terminated and both handles are open.
    let status = unsafe { libc::renameatx_np(from.as_raw_fd(), from_c.as_ptr(), to.as_raw_fd(), to_c.as_ptr(), libc::RENAME_EXCL) };
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: both names are NUL-terminated and both handles are open.
    let status = unsafe { libc::renameat2(from.as_raw_fd(), from_c.as_ptr(), to.as_raw_fd(), to_c.as_ptr(), libc::RENAME_NOREPLACE) };
    #[cfg(not(any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))))]
    let status: i32 = {
        let _ = (from, to, from_c, to_c);
        return Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "no atomic no-replace rename on this platform"));
    };
    if status == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

/// Ask Finder to empty the Trash: Bridge cannot read it itself.
pub fn empty_trash() -> wire::EmptyTrashResult {
    if !cfg!(target_os = "macos") {
        return wire::EmptyTrashResult { emptied: false, detail: Some("Emptying the Trash is only supported on macOS.".into()) };
    }
    let child = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", "tell application \"Finder\" to empty trash"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => return wire::EmptyTrashResult { emptied: false, detail: Some(error.to_string()) },
    };
    // The first run asks the person to let Bridge control Finder; give them
    // time to answer, but do not hold the connection forever.
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return wire::EmptyTrashResult { emptied: true, detail: None },
            Ok(Some(_)) => {
                let mut stderr = String::new();
                if let Some(mut pipe) = child.stderr.take() {
                    use std::io::Read;
                    let _ = pipe.read_to_string(&mut stderr);
                }
                let detail = if stderr.contains("-1743") {
                    "macOS blocked Bridge from controlling Finder. Allow it under Privacy & Security > Automation.".into()
                } else {
                    stderr.trim().to_string()
                };
                return wire::EmptyTrashResult { emptied: false, detail: Some(detail) };
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(200)),
            Ok(None) => {
                let _ = child.kill();
                return wire::EmptyTrashResult { emptied: false, detail: Some("Finder did not answer in time.".into()) };
            }
            Err(error) => return wire::EmptyTrashResult { emptied: false, detail: Some(error.to_string()) },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard(home: &Path) -> Guard {
        Guard { home: home.into(), data_dir: home.join("Library/Application Support/bridge"), worktrees: home.join("Library/Application Support/bridge/worktrees") }
    }

    fn settle(scanner: &Arc<Scanner>, guard: &Guard, path: &Path) -> wire::DiskListing {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let listing = list(scanner, guard, path, false);
            if !listing.measuring || Instant::now() > deadline {
                return listing;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn listings_measure_folders_in_the_background_and_sort_largest_first() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join("big/inner")).unwrap();
        std::fs::write(home.path().join("big/inner/blob"), vec![1u8; 256 * 1024]).unwrap();
        std::fs::write(home.path().join("small.txt"), b"hi").unwrap();
        let scanner = Scanner::new();
        let guard = guard(home.path());
        let listing = settle(&scanner, &guard, home.path());
        assert!(!listing.measuring);
        assert_eq!(listing.entries[0].name, "big");
        assert!(listing.entries[0].size_bytes.unwrap() >= 256 * 1024);
        assert_eq!(listing.entries[0].kind, "directory");
        // The walk kept the subfolder's own total, so drilling in is instant.
        let inner = list(&scanner, &guard, &home.path().join("big"), false);
        assert!(!inner.measuring, "inner was measured by the parent walk");
    }

    #[test]
    fn the_guard_refuses_the_places_that_break_the_machine_or_bridge() {
        let home = Path::new("/Users/someone");
        let guard = guard(home);
        for refused in ["/", "/Users", "/Users/someone", "/Users/someone/Library", "/Users/someone/Documents", "/Users/someone/.Trash", "/Users/someone/.ssh/id_ed25519", "/Users/someone/Library/Keychains/login.keychain-db", "/Users/someone/Library/Application Support", "/Users/someone/Library/Application Support/bridge/bridge.db", "/System/Library", "/usr/local/bin", "/Applications", "/Users/someone/../other"] {
            assert!(guard.protected_reason(Path::new(refused)).is_some(), "{refused} should be refused");
        }
        for allowed in ["/Users/someone/Downloads/installer.dmg", "/Users/someone/.npm", "/Users/someone/Library/Caches", "/Users/someone/Library/Application Support/Slack", "/Users/someone/code/app/node_modules", "/Applications/Old.app"] {
            assert_eq!(guard.protected_reason(Path::new(allowed)), None, "{allowed} should be allowed");
        }
    }

    #[test]
    fn delete_removes_permanently_and_reports_refusals_per_path() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join("cache/deep")).unwrap();
        std::fs::write(home.path().join("cache/deep/file"), vec![0u8; 8192]).unwrap();
        let scanner = Scanner::new();
        let guard = guard(home.path());
        settle(&scanner, &guard, home.path());
        let target = home.path().join("cache").display().to_string();
        let result = delete(&scanner, &guard, &[target.clone(), home.path().join("Library").display().to_string(), home.path().join("gone").display().to_string()], true);
        assert_eq!(result.deleted, vec![target]);
        assert!(result.bytes_freed >= 8192);
        assert_eq!(result.failed.len(), 2);
        assert!(!home.path().join("cache").exists());
    }

    #[test]
    fn a_symlinked_folder_cannot_carry_a_delete_outside_the_home_folder() {
        let home = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("precious"), b"keep").unwrap();
        std::os::unix::fs::symlink(outside.path(), home.path().join("link")).unwrap();
        // And one that leads into Bridge's own data.
        let guard = guard(home.path());
        std::fs::create_dir_all(&guard.data_dir).unwrap();
        std::fs::write(guard.data_dir.join("bridge.db"), b"db").unwrap();
        std::os::unix::fs::symlink(&guard.data_dir, home.path().join("sneaky")).unwrap();
        let scanner = Scanner::new();
        let targets = [home.path().join("link/precious"), home.path().join("sneaky/bridge.db")].map(|path| path.display().to_string());
        let result = delete(&scanner, &guard, &targets, true);
        assert!(result.deleted.is_empty(), "{:?}", result.deleted);
        assert_eq!(result.failed.len(), 2);
        assert!(outside.path().join("precious").exists());
        assert!(guard.data_dir.join("bridge.db").exists());
        // The link itself is the person's to remove; that never follows it.
        let link = home.path().join("link").display().to_string();
        assert_eq!(delete(&scanner, &guard, &[link], true).deleted.len(), 1);
        assert!(outside.path().join("precious").exists());
    }

    fn open(path: &Path) -> cap_std::fs::Dir {
        cap_std::fs::Dir::open_ambient_dir(path, cap_std::ambient_authority()).unwrap()
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn trashing_never_replaces_something_already_in_the_trash() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".Trash")).unwrap();
        std::fs::write(home.path().join(".Trash/report.pdf"), b"old").unwrap();
        std::fs::write(home.path().join("report.pdf"), b"new").unwrap();
        move_into(&open(home.path()), "report.pdf".as_ref(), &open(&home.path().join(".Trash")), TRASH_NAME_ATTEMPTS).unwrap();
        assert_eq!(std::fs::read(home.path().join(".Trash/report.pdf")).unwrap(), b"old");
        assert_eq!(names(&home.path().join(".Trash")).len(), 2);
    }

    #[test]
    fn concurrent_moves_of_the_same_name_both_survive() {
        for _ in 0..25 {
            let home = tempfile::tempdir().unwrap();
            let trash = home.path().join(".Trash");
            std::fs::create_dir_all(&trash).unwrap();
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let movers: Vec<_> = ["one", "two"].into_iter().map(|source| {
                let folder = home.path().join(source);
                std::fs::create_dir_all(&folder).unwrap();
                std::fs::write(folder.join("report.pdf"), source).unwrap();
                let (barrier, trash) = (Arc::clone(&barrier), trash.clone());
                std::thread::spawn(move || {
                    let (from, to) = (open(&folder), open(&trash));
                    barrier.wait();
                    move_into(&from, "report.pdf".as_ref(), &to, TRASH_NAME_ATTEMPTS).unwrap();
                })
            }).collect();
            movers.into_iter().for_each(|mover| mover.join().unwrap());
            let mut contents: Vec<_> = names(&trash).iter().map(|name| std::fs::read_to_string(trash.join(name)).unwrap()).collect();
            contents.sort();
            assert_eq!(contents, ["one", "two"]);
        }
    }

    #[test]
    fn a_trash_with_every_name_taken_refuses_rather_than_replacing() {
        let home = tempfile::tempdir().unwrap();
        let trash = home.path().join(".Trash");
        std::fs::create_dir_all(&trash).unwrap();
        std::fs::write(home.path().join("a.txt"), b"new").unwrap();
        std::fs::write(trash.join("a.txt"), b"old").unwrap();
        // One attempt means only the plain name, which is taken.
        assert!(move_into(&open(home.path()), "a.txt".as_ref(), &open(&trash), 1).is_err());
        assert_eq!(std::fs::read(trash.join("a.txt")).unwrap(), b"old");
        assert!(home.path().join("a.txt").exists());
    }

    #[test]
    fn swapping_a_folder_for_a_symlink_after_the_check_cannot_redirect_the_delete() {
        let home = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join("a")).unwrap();
        std::fs::write(home.path().join("a/victim"), b"mine").unwrap();
        std::fs::write(outside.path().join("victim"), b"keep").unwrap();
        let guard = guard(home.path());
        let target = home.path().join("a/victim");
        let parent = open_parent(&guard, &target).unwrap();
        assert_eq!(guard.resolved().protected_reason(&parent.real.join(&parent.name)), None);
        // The race: after the check, the folder is moved away and replaced
        // with a symlink to somewhere protected.
        std::fs::rename(home.path().join("a"), home.path().join("a-moved")).unwrap();
        std::os::unix::fs::symlink(outside.path(), home.path().join("a")).unwrap();
        remove_at(&Scanner::new(), &parent, &target, home.path(), true).unwrap();
        assert!(outside.path().join("victim").exists(), "the delete followed the swapped symlink");
        assert!(!home.path().join("a-moved/victim").exists());
    }
}
