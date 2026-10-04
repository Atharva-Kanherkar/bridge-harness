//! Disk space across the whole Mac: how full the volume is, what each folder
//! costs, and the one place a person can delete what they no longer want.
//!
//! Measuring a home folder can take minutes, and a daemon connection answers
//! requests one at a time, so nothing here blocks on a walk. A listing comes
//! back at once with the sizes already known; the rest are measured in the
//! background and arrive on the next request, flagged `measuring` until then.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The volume the home folder lives on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DiskVolume {
    pub mount_point: String,
    pub total_bytes: u64,
    /// What a new file could use right now.
    pub free_bytes: u64,
    pub used_bytes: u64,
}

/// One child of a listed folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DiskEntry {
    pub name: String,
    pub path: String,
    /// `directory`, `package` (an `.app` or similar bundle), `file`, or
    /// `symlink`. A symlink is never followed and costs only itself.
    pub kind: String,
    /// Allocated bytes. `null` until the first measurement lands.
    pub size_bytes: Option<u64>,
    pub item_count: Option<u64>,
    /// A measurement is queued or running for this entry.
    pub measuring: bool,
    /// Some descendants could not be read, so the size is a floor.
    pub partial: bool,
    pub modified_at: Option<String>,
    /// Why Bridge will not delete this, in words meant for a person.
    pub protected_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScanDirectoryParams {
    /// Absolute path. Omitted means the home folder.
    #[serde(default)]
    pub path: Option<String>,
    /// Drop cached sizes under this path and measure again.
    #[serde(default)]
    pub refresh: bool,
}

/// A folder's children, largest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DiskListing {
    pub path: String,
    pub parent: Option<String>,
    /// The sum of the children measured so far.
    pub size_bytes: u64,
    /// Any child is still being measured.
    pub measuring: bool,
    /// The folder itself could not be listed (often macOS privacy protection).
    pub unreadable: Option<String>,
    pub entries: Vec<DiskEntry>,
    /// Children left out of a very large folder: the smallest ones.
    pub omitted_count: u64,
    pub omitted_bytes: u64,
}

/// A known place that tends to grow quietly: build caches, package stores,
/// simulators, logs, downloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DiskSuggestion {
    pub id: String,
    pub label: String,
    /// `developer`, `caches`, `files`, or `system`.
    pub group: String,
    pub description: String,
    pub path: String,
    pub size_bytes: Option<u64>,
    pub measuring: bool,
    /// `safe` when the owning tool rebuilds it on demand; `review` when it
    /// may hold something a person wants to keep.
    pub safety: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DiskOverview {
    pub volume: Option<DiskVolume>,
    pub home: String,
    pub suggestions: Vec<DiskSuggestion>,
    pub measuring: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeletePathsParams {
    pub paths: Vec<String>,
    /// Delete outright instead of moving to the Trash.
    #[serde(default)]
    pub permanent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DiskDeleteFailure {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DiskDeleteResult {
    pub deleted: Vec<String>,
    pub failed: Vec<DiskDeleteFailure>,
    /// The last measured size of what was removed; zero when never measured.
    pub bytes_freed: u64,
    /// Moved to the Trash rather than deleted outright.
    pub trashed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EmptyTrashResult {
    pub emptied: bool,
    pub detail: Option<String>,
}
