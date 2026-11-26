//! A fresh content measurement of the checkout used by completion.

use super::RepositoryStamp;
use crate::{git, BridgeError};
use cap_std::fs::{Dir, Metadata, MetadataExt};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ffi::{CString, OsStr};
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

pub(crate) fn repository_stamp(path: &Path) -> Result<RepositoryStamp, BridgeError> {
    Ok(measure_repository(path)?.stamp)
}

pub(crate) struct RepositoryMeasurement {
    pub stamp: RepositoryStamp,
    /// Materials visible to a verifier, independent of the worktree location
    /// and the index changes made when adoption captures a commit.
    pub checkout_digest: [u8; 32],
    revision_digest: [u8; 32],
}

pub(crate) fn measure_repository(path: &Path) -> Result<RepositoryMeasurement, BridgeError> {
    measure(&worktree_root(path)?)
}

fn worktree_root(path: &Path) -> Result<PathBuf, BridgeError> {
    let output = git_read(path, &["rev-parse", "--show-toplevel"])?;
    let bytes = output
        .strip_suffix(b"\n")
        .filter(|root| !root.is_empty())
        .ok_or_else(|| invalid("Git did not report a worktree root"))?;
    let root = PathBuf::from(OsStr::from_bytes(bytes));
    if !root.is_absolute() {
        return Err(invalid("Git reported a relative worktree root"));
    }
    Ok(root)
}

#[derive(PartialEq, Eq)]
struct Inventory {
    head: String,
    index: Vec<u8>,
    staged_paths: Vec<u8>,
    untracked: Vec<u8>,
}

impl Inventory {
    fn read(root: &Path) -> Result<Self, BridgeError> {
        let head = git_read(root, &["rev-parse", "--verify", "HEAD"])?;
        let head = std::str::from_utf8(&head)
            .map_err(|_| invalid("invalid Git HEAD"))?
            .trim();
        if head.is_empty() || !head.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(invalid("invalid Git HEAD"));
        }
        Ok(Self {
            head: head.to_owned(),
            index: git_read(root, &["ls-files", "--stage", "-z"])?,
            // An intent-to-add empty file and a staged empty file share an
            // index blob ID. Only the staged file belongs in the cached diff.
            staged_paths: git_read(
                root,
                &[
                    "diff",
                    "--cached",
                    "--no-renames",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--ignore-submodules=none",
                    "--ita-invisible-in-index",
                    "--name-only",
                    "-z",
                    head,
                    "--",
                ],
            )?,
            untracked: git_read(root, &["ls-files", "--others", "--exclude-standard", "-z"])?,
        })
    }

    fn paths(&self) -> Result<BTreeMap<&[u8], bool>, BridgeError> {
        let mut paths = BTreeMap::new();
        for record in self
            .index
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty())
        {
            let tab = record
                .iter()
                .position(|byte| *byte == b'\t')
                .ok_or_else(|| invalid("invalid Git index entry"))?;
            let gitlink = record.starts_with(b"160000 ");
            paths
                .entry(&record[tab + 1..])
                .and_modify(|existing| *existing |= gitlink)
                .or_insert(gitlink);
        }
        for path in self
            .untracked
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
        {
            if path.ends_with(b"/") {
                return Err(invalid("an untracked nested repository cannot be measured"));
            }
            paths.entry(path).or_insert(false);
        }
        Ok(paths)
    }
}

type Fingerprint = (u64, u64, u32, u64, i64, i64, i64, i64);

/// One worktree's collected state, kept until the whole tree is validated.
struct Worktree {
    root: PathBuf,
    identity: (u64, u64),
    inventory: Inventory,
    fingerprints: Vec<(PathBuf, Option<Fingerprint>)>,
}

impl Worktree {
    fn open_observed_root(&self) -> Result<Dir, BridgeError> {
        let directory = Dir::open_ambient_dir(&self.root, cap_std::ambient_authority())?;
        let metadata = directory.dir_metadata()?;
        if (metadata.dev(), metadata.ino()) != self.identity {
            return Err(changed());
        }
        Ok(directory)
    }
}

fn measure(root: &Path) -> Result<RepositoryMeasurement, BridgeError> {
    let mut worktrees = Vec::new();
    let measurement = collect(root, &mut worktrees)?;
    validate_tree(&worktrees)?;
    Ok(measurement)
}

fn collect(
    root: &Path,
    worktrees: &mut Vec<Worktree>,
) -> Result<RepositoryMeasurement, BridgeError> {
    let directory = Dir::open_ambient_dir(root, cap_std::ambient_authority())?;
    let root_meta = directory.dir_metadata()?;
    let inventory = Inventory::read(root)?;
    let mut revision = Sha256::new();
    let mut checkout = Sha256::new();
    field(&mut revision, &inventory.index);
    field(&mut revision, &inventory.staged_paths);
    let mut fingerprints = Vec::new();
    for (bytes, gitlink) in inventory.paths()? {
        let path = Path::new(OsStr::from_bytes(bytes));
        let entry = checkout_entry(&directory, path)?;
        if let Some(entry) = &entry {
            field(&mut checkout, bytes);
            if let Some(child) = hash_entry(&mut checkout, root, path, gitlink, entry, worktrees)? {
                field(&mut revision, bytes);
                field(&mut revision, &child.revision_digest);
            }
        }
        fingerprints.push((
            path.to_owned(),
            entry.map(|entry| fingerprint(&entry.metadata)),
        ));
    }
    let checkout_digest: [u8; 32] = checkout.finalize().into();
    field(&mut revision, &checkout_digest);
    let revision_digest: [u8; 32] = revision.finalize().into();
    let mut stamp = Sha256::new();
    field(&mut stamp, b"completion repository stamp v2");
    field(&mut stamp, root.as_os_str().as_bytes());
    field(&mut stamp, &revision_digest);
    let head = inventory.head.clone();
    worktrees.push(Worktree {
        root: root.to_owned(),
        identity: (root_meta.dev(), root_meta.ino()),
        inventory,
        fingerprints,
    });
    Ok(RepositoryMeasurement {
        stamp: RepositoryStamp {
            head,
            dirty_digest: format!("sha256:v2:{:x}", stamp.finalize()),
        },
        checkout_digest,
        revision_digest,
    })
}

/// Worktrees are collected child-first. Reread the parent inventories first,
/// then recheck every tracked path after all Git commands have finished.
fn validate_tree(worktrees: &[Worktree]) -> Result<(), BridgeError> {
    for worktree in worktrees.iter().rev() {
        if Inventory::read(&worktree.root)? != worktree.inventory {
            return Err(changed());
        }
    }
    for worktree in worktrees {
        let directory = worktree.open_observed_root()?;
        for (path, before) in &worktree.fingerprints {
            let after =
                checkout_entry(&directory, path)?.map(|entry| fingerprint(&entry.metadata));
            if *before != after {
                return Err(changed());
            }
        }
        // Reopen the path to catch replacement while the descriptor was in use.
        worktree.open_observed_root()?;
    }
    Ok(())
}

struct Entry<'a> {
    dir: Dir,
    name: &'a OsStr,
    metadata: Metadata,
}

fn checkout_entry<'a>(root: &Dir, path: &'a Path) -> Result<Option<Entry<'a>>, BridgeError> {
    let mut dir = root.try_clone()?;
    let mut components = path.components().peekable();
    while let Some(Component::Normal(name)) = components.next() {
        if components.peek().is_none() {
            return match dir.symlink_metadata(name) {
                Ok(metadata) => Ok(Some(Entry {
                    dir,
                    name,
                    metadata,
                })),
                Err(error) if missing(&error) => Ok(None),
                Err(error) => Err(error.into()),
            };
        }
        // A tracked descendant is absent when that parent was replaced with a
        // file or link. Git lists the replacement separately if it is visible.
        dir = match open_at(&dir, name, libc::O_DIRECTORY) {
            Ok(child) => Dir::from_std_file(child),
            Err(error) if missing(&error) || error.raw_os_error() == Some(libc::ELOOP) => {
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        };
    }
    Err(invalid("Git reported an invalid checkout path"))
}

fn hash_entry(
    digest: &mut Sha256,
    root: &Path,
    path: &Path,
    gitlink: bool,
    entry: &Entry<'_>,
    worktrees: &mut Vec<Worktree>,
) -> Result<Option<RepositoryMeasurement>, BridgeError> {
    let file_type = entry.metadata.file_type();
    if file_type.is_symlink() {
        field(digest, b"symlink");
        let target = entry.dir.read_link_contents(entry.name)?;
        field(digest, target.as_os_str().as_bytes());
    } else if file_type.is_file() {
        field(digest, b"file");
        field(digest, &(entry.metadata.mode() & 0o111).to_be_bytes());
        let mut file = open_at(&entry.dir, entry.name, 0)?;
        let expected = fingerprint(&entry.metadata);
        let opened = Metadata::from_file(&file)?;
        if !opened.is_file() || fingerprint(&opened) != expected {
            return Err(changed());
        }
        let mut contents = Sha256::new();
        io::copy(&mut file, &mut contents)?;
        if fingerprint(&Metadata::from_file(&file)?) != expected {
            return Err(changed());
        }
        field(digest, &contents.finalize());
    } else if file_type.is_dir() {
        field(digest, b"directory");
        if gitlink {
            return hash_submodule(digest, root, path, entry, worktrees);
        }
    } else {
        return Err(invalid("a special worktree file cannot be measured"));
    }
    Ok(None)
}

fn hash_submodule(
    digest: &mut Sha256,
    root: &Path,
    path: &Path,
    entry: &Entry<'_>,
    worktrees: &mut Vec<Worktree>,
) -> Result<Option<RepositoryMeasurement>, BridgeError> {
    let directory = Dir::from_std_file(open_at(&entry.dir, entry.name, libc::O_DIRECTORY)?);
    match directory.symlink_metadata(".git") {
        Ok(metadata) if metadata.is_dir() || metadata.is_file() => {
            let child_root = root.join(path);
            if worktree_root(&child_root)? != child_root {
                return Err(invalid("Git reported a different submodule worktree"));
            }
            let child = collect(&child_root, worktrees)?;
            #[cfg(test)]
            tests::after_child_measurement();
            field(digest, b"submodule");
            field(digest, child.stamp.head.as_bytes());
            field(digest, &child.checkout_digest);
            Ok(Some(child))
        }
        Err(error) if missing(&error) && directory.entries()?.next().is_none() => {
            field(digest, b"uninitialized submodule");
            Ok(None)
        }
        _ => Err(invalid("submodule checkout could not be measured")),
    }
}

fn open_at(dir: &Dir, name: &OsStr, flags: libc::c_int) -> io::Result<File> {
    let name = CString::new(name.as_bytes())?;
    // The name is one path component. The live directory descriptor scopes
    // the lookup. NOFOLLOW and NONBLOCK also cover a link or FIFO swap.
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // The descriptor is new and owned by the returned file.
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn fingerprint(m: &Metadata) -> Fingerprint {
    (
        m.dev(),
        m.ino(),
        m.mode(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    )
}

fn field(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_be_bytes());
    digest.update(bytes);
}

fn missing(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
    )
}

fn changed() -> BridgeError {
    invalid("checkout changed while measuring it; retry verification")
}

fn invalid(message: &str) -> BridgeError {
    BridgeError::Invalid(format!("completion repository stamp: {message}"))
}

fn git_read(path: &Path, args: &[&str]) -> Result<Vec<u8>, BridgeError> {
    let mut command = git::git_command(path);
    command
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR");
    let output = git::command_output_deadline(&mut command, Duration::from_secs(30))?;
    // ls-files can omit an unreadable directory and still exit successfully.
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    if !diagnostic.trim().is_empty() {
        return Err(invalid(diagnostic.trim()));
    }
    if !output.status.success() {
        return Err(invalid(&format!("Git returned {}", output.status)));
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::measure_repository;
    use crate::completion::tests::{git, repository};
    use std::cell::Cell;
    use std::path::{Path, PathBuf};

    thread_local! {
        static EDIT_AFTER_CHILD_MEASUREMENT: Cell<Option<PathBuf>> = const { Cell::new(None) };
    }

    pub(super) fn after_child_measurement() {
        if let Some(file) = EDIT_AFTER_CHILD_MEASUREMENT.with(Cell::take) {
            std::fs::write(file, b"edited while the parent measures\n").unwrap();
        }
    }

    #[test]
    fn child_file_edit_after_the_child_measurement_is_rejected() {
        let (_parent_fixture, parent) = repository();
        let (_origin_fixture, origin) = repository();
        let parent = Path::new(&parent);
        git(
            parent,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "-q",
                &origin,
                "nested",
            ],
        );
        let child_file = parent.join("nested").join("README.md");
        let baseline = measure_repository(parent).unwrap();

        EDIT_AFTER_CHILD_MEASUREMENT.with(|edit| edit.set(Some(child_file)));
        let measured = measure_repository(parent);

        let error = measured
            .err()
            .expect("a child edit during the parent measurement must fail the stamp");
        assert!(
            error.to_string().contains("changed while measuring"),
            "{error:?}"
        );
        let after = measure_repository(parent).unwrap();
        assert_eq!(baseline.stamp.head, after.stamp.head);
        assert_ne!(baseline.stamp.dirty_digest, after.stamp.dirty_digest);
    }
}
