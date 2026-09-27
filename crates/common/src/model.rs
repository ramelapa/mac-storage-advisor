use std::path::PathBuf;
use std::time::SystemTime;

use serde::Serialize;

/// Threads this version actually uses. `--threads` is recorded, not parallelized.
pub const SCAN_CONCURRENCY: u32 = 1;

/// What the user asked to scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanTarget {
    pub root: PathBuf,
    /// Exclusion patterns as the user typed them, plus protected prefixes that applied.
    pub exclusions: Vec<String>,
    /// Regular files smaller than this are counted but not persisted.
    pub min_logical_size: u64,
    /// Value of `--threads`. Scanning still uses [`SCAN_CONCURRENCY`].
    pub threads: u32,
    /// Dev override. Exact protected macOS roots are refused when this is false.
    pub allow_protected_roots: bool,
    /// When true, tracing redacts paths. Reports and the database still store them.
    pub redact_paths: bool,
}

impl ScanTarget {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            exclusions: Vec::new(),
            min_logical_size: 0,
            threads: 1,
            allow_protected_roots: false,
            redact_paths: false,
        }
    }
}

/// A finished scan header. File and directory rows live beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scan {
    pub id: i64,
    pub target: ScanTarget,
    pub started_at: SystemTime,
    pub finished_at: SystemTime,
    pub status: ScanStatus,
    pub statistics: ScanStatistics,
    pub product_name: String,
    pub product_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanStatus {
    Completed,
    CompletedWithErrors,
}

impl ScanStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::CompletedWithErrors => "completed_with_errors",
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "completed" => Some(Self::Completed),
            "completed_with_errors" => Some(Self::CompletedWithErrors),
            _ => None,
        }
    }
}

/// Aggregates for one scan.
///
/// `logical_bytes` sums regular files only, once per directory entry. Hard
/// links are not collapsed. `allocated_bytes` sums `st_blocks * 512` where the
/// platform provides it and is not a reclaimable-space estimate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanStatistics {
    pub directories_scanned: u64,
    pub files_scanned: u64,
    pub logical_bytes: u64,
    pub allocated_bytes: u64,
    /// False when at least one regular file had no allocated size.
    pub allocated_bytes_complete: bool,
    pub symlinks: u64,
    pub other_entries: u64,
    pub errors: u64,
    pub skipped: u64,
    pub files_below_min_size: u64,
    pub elapsed_ms: u64,
}

impl Default for ScanStatistics {
    fn default() -> Self {
        Self {
            directories_scanned: 0,
            files_scanned: 0,
            logical_bytes: 0,
            allocated_bytes: 0,
            allocated_bytes_complete: true,
            symlinks: 0,
            other_entries: 0,
            errors: 0,
            skipped: 0,
            files_below_min_size: 0,
            elapsed_ms: 0,
        }
    }
}

/// Full in-memory result of a scan, before or after it is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanSnapshot {
    pub root: PathBuf,
    pub started_at: SystemTime,
    pub finished_at: SystemTime,
    pub status: ScanStatus,
    pub statistics: ScanStatistics,
    pub files: Vec<FileRecord>,
    pub directories: Vec<DirectoryRecord>,
    pub errors: Vec<ScanErrorRecord>,
    pub exclusions: Vec<String>,
    pub threads_requested: u32,
    pub min_logical_size: u64,
    pub allow_protected_roots: bool,
    pub redact_paths: bool,
}

impl ScanSnapshot {
    pub fn to_scan(&self, id: i64, product_name: &str, product_version: &str) -> Scan {
        Scan {
            id,
            target: ScanTarget {
                root: self.root.clone(),
                exclusions: self.exclusions.clone(),
                min_logical_size: self.min_logical_size,
                threads: self.threads_requested,
                allow_protected_roots: self.allow_protected_roots,
                redact_paths: self.redact_paths,
            },
            started_at: self.started_at,
            finished_at: self.finished_at,
            status: self.status,
            statistics: self.statistics.clone(),
            product_name: product_name.to_owned(),
            product_version: product_version.to_owned(),
        }
    }
}

/// Metadata for a non-directory entry (regular file, symlink, or other).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRecord {
    pub path: PathBuf,
    pub filename: String,
    pub extension: Option<String>,
    pub logical_size: u64,
    pub allocated_size: Option<u64>,
    pub created: Option<SystemTime>,
    pub modified: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    pub inode: Option<u64>,
    pub device_id: Option<u64>,
    pub permissions: Option<u32>,
    pub kind: FileKind,
    pub is_symlink: bool,
    pub is_broken_symlink: bool,
    pub link_target: Option<PathBuf>,
}

/// Metadata for a directory that was entered. `logical_size` is the directory
/// inode's size, not a recursive total of children.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryRecord {
    pub path: PathBuf,
    pub filename: String,
    pub logical_size: u64,
    pub allocated_size: Option<u64>,
    pub created: Option<SystemTime>,
    pub modified: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    pub inode: Option<u64>,
    pub device_id: Option<u64>,
    pub permissions: Option<u32>,
}

/// Non-fatal scan problem. `message` describes the I/O failure and never includes file contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScanErrorRecord {
    pub path: Option<PathBuf>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    File,
    Directory,
    Symlink,
    Other,
}

impl FileKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "dir",
            Self::Symlink => "symlink",
            Self::Other => "other",
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "file" => Some(Self::File),
            "dir" => Some(Self::Directory),
            "symlink" => Some(Self::Symlink),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

/// Placeholder for a future recommendation. The scanner does not assign these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Info,
    Low,
    Medium,
    High,
}

/// Placeholder categories for a future recommendation engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecommendationCategory {
    Duplicates,
    LargeFiles,
    StaleFiles,
    Downloads,
    DeveloperArtifacts,
    Caches,
    Other,
}

/// Stable JSON object printed by `mac-storage scan --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScanReport {
    pub product: String,
    pub version: String,
    pub scan_id: i64,
    pub root: PathBuf,
    pub directories_scanned: u64,
    pub files_scanned: u64,
    pub logical_bytes: u64,
    pub allocated_bytes: u64,
    pub allocated_bytes_complete: bool,
    pub symlinks: u64,
    pub other_entries: u64,
    pub errors: u64,
    pub skipped: u64,
    pub files_below_min_size: u64,
    pub elapsed_ms: u64,
    pub threads_requested: u32,
    pub concurrency: u32,
    pub min_logical_size: u64,
    pub database: PathBuf,
    pub error_details: Vec<ScanErrorRecord>,
}
