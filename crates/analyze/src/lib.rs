//! Reviews a stored scan and emits suggestions.
//!
//! This crate does not read file contents, hash files, or delete paths.
//! Logical bytes are inventory totals, not a promise of free disk space.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use mac_storage_common::{RecommendationCategory, RiskLevel};
use mac_storage_scanner::is_protected_path;
use serde::Serialize;

/// Files last modified at least this long ago are stale. Artifact trees are
/// not included in that count.
pub const DEFAULT_STALE_DAYS: u64 = 180;

const MEDIUM_DUPLICATE_BYTES: u64 = 1024 * 1024;
const PREVIEW: usize = 6;

/// One regular file from a stored scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryFile {
    pub path: PathBuf,
    pub logical_size: u64,
    pub allocated_size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub extension: Option<String>,
    pub is_dataless: bool,
}

/// A content-duplicate group already computed for the scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateSet {
    pub logical_size: u64,
    pub redundant_bytes: u64,
    pub paths: Vec<PathBuf>,
}

/// One stored scan total, used to compare the same root over time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrendPoint {
    pub scan_id: i64,
    pub root: PathBuf,
    pub logical_bytes: u64,
    pub allocated_bytes: u64,
    pub files_scanned: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdvisorReport {
    pub stale: StaleSummary,
    pub downloads: DownloadsSummary,
    pub developer: DeveloperSummary,
    pub office_locks: OfficeLockSummary,
    pub dataless: DatalessSummary,
    pub sparse_or_shared: SparseSummary,
    pub recommendations: Vec<Recommendation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StaleSummary {
    pub older_than_days: u64,
    pub file_count: u64,
    pub logical_bytes: u64,
    pub files: Vec<PathSize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DownloadsSummary {
    /// True when the scan root's last component is `Downloads`.
    pub applies: bool,
    pub categories: Vec<DownloadCategory>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DownloadCategory {
    pub name: &'static str,
    pub file_count: u64,
    pub logical_bytes: u64,
    pub largest: Vec<PathSize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeveloperSummary {
    pub trees: Vec<ArtifactTree>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ArtifactTree {
    pub path: PathBuf,
    pub kind: &'static str,
    pub file_count: u64,
    pub logical_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DatalessSummary {
    pub file_count: u64,
    pub logical_bytes: u64,
    pub files: Vec<PathSize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SparseSummary {
    pub file_count: u64,
    /// Logical size minus allocated size, summed. This gap is not reclaimable.
    pub gap_bytes: u64,
    pub files: Vec<PathSize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OfficeLockSummary {
    pub file_count: u64,
    pub logical_bytes: u64,
    pub files: Vec<PathSize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PathSize {
    pub path: PathBuf,
    pub logical_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Recommendation {
    pub category: RecommendationCategory,
    pub risk: RiskLevel,
    pub title: String,
    pub detail: String,
    pub logical_bytes: u64,
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrendSeries {
    pub root: PathBuf,
    /// Oldest scan first.
    pub points: Vec<TrendPoint>,
    /// Newest logical total minus the previous scan of this same root.
    pub logical_delta: Option<i64>,
    pub allocated_delta: Option<i64>,
}

/// Build the review for one scan.
///
/// `now` and `stale_after` are arguments so tests do not depend on the clock.
/// Files inside a developer-artifact directory, and Office `~$` lock files,
/// are left out of the stale and Downloads tallies.
pub fn advise(
    root: &Path,
    files: &[InventoryFile],
    duplicates: &[DuplicateSet],
    now: SystemTime,
    stale_after: Duration,
) -> AdvisorReport {
    let stale_days = days_in(stale_after);
    let mut stale_files = Vec::new();
    let mut categories: BTreeMap<&'static str, Vec<&InventoryFile>> = BTreeMap::new();
    let mut trees: BTreeMap<PathBuf, ArtifactAccum> = BTreeMap::new();
    let mut office = Vec::new();
    let mut dataless_files = Vec::new();
    let mut sparse_files = Vec::new();
    let downloads_root = is_downloads_root(root);

    for file in files {
        if file.is_dataless {
            dataless_files.push(file);
            continue;
        }
        if is_sparse_or_shared(file) {
            sparse_files.push(file);
        }
        if is_office_lock(&file.path) {
            office.push(file);
            continue;
        }
        if let Some((tree, kind)) = artifact_root(&file.path) {
            let entry = trees.entry(tree).or_insert(ArtifactAccum {
                kind,
                file_count: 0,
                logical_bytes: 0,
            });
            entry.file_count += 1;
            entry.logical_bytes = entry.logical_bytes.saturating_add(file.logical_size);
            continue;
        }
        if let Some(modified) = file.modified {
            if is_stale(modified, now, stale_after) {
                stale_files.push(file);
            }
        }
        if downloads_root {
            categories
                .entry(download_category(file.extension.as_deref()))
                .or_default()
                .push(file);
        }
    }

    stale_files.sort_by(|left, right| {
        right
            .logical_size
            .cmp(&left.logical_size)
            .then(left.path.cmp(&right.path))
    });
    office.sort_by(|left, right| left.path.cmp(&right.path));

    let stale = StaleSummary {
        older_than_days: stale_days,
        file_count: u64::try_from(stale_files.len()).unwrap_or(u64::MAX),
        logical_bytes: sum_sizes(&stale_files),
        files: preview_files(&stale_files),
    };
    let downloads = DownloadsSummary {
        applies: downloads_root,
        categories: if downloads_root {
            category_totals(&categories)
        } else {
            Vec::new()
        },
    };
    let mut developer_trees: Vec<ArtifactTree> = trees
        .into_iter()
        .map(|(path, accum)| ArtifactTree {
            path,
            kind: accum.kind,
            file_count: accum.file_count,
            logical_bytes: accum.logical_bytes,
        })
        .collect();
    developer_trees.sort_by(|left, right| {
        right
            .logical_bytes
            .cmp(&left.logical_bytes)
            .then(left.path.cmp(&right.path))
    });
    let developer = DeveloperSummary {
        trees: developer_trees,
    };
    let office_locks = OfficeLockSummary {
        file_count: u64::try_from(office.len()).unwrap_or(u64::MAX),
        logical_bytes: sum_sizes(&office),
        files: preview_files(&office),
    };
    let dataless = DatalessSummary {
        file_count: count_of(&dataless_files),
        logical_bytes: sum_sizes(&dataless_files),
        files: preview_files(&dataless_files),
    };
    sparse_files.sort_by(|left, right| {
        gap_of(right)
            .cmp(&gap_of(left))
            .then(left.path.cmp(&right.path))
    });
    let sparse_or_shared = SparseSummary {
        file_count: count_of(&sparse_files),
        gap_bytes: sparse_files
            .iter()
            .fold(0u64, |sum, file| sum.saturating_add(gap_of(file))),
        files: preview_files(&sparse_files),
    };
    let recommendations = recommendations_for(
        &stale,
        &downloads,
        &developer,
        &office_locks,
        &dataless,
        &sparse_or_shared,
        duplicates,
    );
    AdvisorReport {
        stale,
        downloads,
        developer,
        office_locks,
        dataless,
        sparse_or_shared,
        recommendations,
    }
}

fn is_sparse_or_shared(file: &InventoryFile) -> bool {
    match file.allocated_size {
        Some(allocated) => file.logical_size > allocated,
        None => false,
    }
}

fn gap_of(file: &InventoryFile) -> u64 {
    match file.allocated_size {
        Some(allocated) if file.logical_size > allocated => file.logical_size - allocated,
        _ => 0,
    }
}

fn count_of(files: &[&InventoryFile]) -> u64 {
    u64::try_from(files.len()).unwrap_or(u64::MAX)
}

/// Compare scans of the same root. Scans of different folders are not subtracted.
pub fn trends(mut points: Vec<TrendPoint>) -> Vec<TrendSeries> {
    points.sort_by(|left, right| {
        left.root
            .cmp(&right.root)
            .then(left.scan_id.cmp(&right.scan_id))
    });
    let mut series = Vec::new();
    let mut current: Option<TrendSeries> = None;
    for point in points {
        let same = current
            .as_ref()
            .is_some_and(|series| series.root == point.root);
        if !same {
            if let Some(done) = current.take() {
                series.push(done);
            }
            current = Some(TrendSeries {
                root: point.root.clone(),
                points: Vec::new(),
                logical_delta: None,
                allocated_delta: None,
            });
        }
        let bucket = current.as_mut().expect("series just inserted");
        if let Some(previous) = bucket.points.last() {
            bucket.logical_delta = Some(signed_delta(point.logical_bytes, previous.logical_bytes));
            bucket.allocated_delta = Some(signed_delta(
                point.allocated_bytes,
                previous.allocated_bytes,
            ));
        }
        bucket.points.push(point);
    }
    if let Some(done) = current {
        series.push(done);
    }
    series
}

pub fn stale_duration(days: u64) -> Duration {
    Duration::from_secs(days.saturating_mul(24 * 60 * 60))
}

struct ArtifactAccum {
    kind: &'static str,
    file_count: u64,
    logical_bytes: u64,
}

fn recommendations_for(
    stale: &StaleSummary,
    downloads: &DownloadsSummary,
    developer: &DeveloperSummary,
    office: &OfficeLockSummary,
    dataless: &DatalessSummary,
    sparse: &SparseSummary,
    duplicates: &[DuplicateSet],
) -> Vec<Recommendation> {
    let mut recommendations = Vec::new();
    let mut artifact_groups = 0u64;
    let mut artifact_bytes = 0u64;

    if duplicates.is_empty() {
        recommendations.push(Recommendation {
            category: RecommendationCategory::Duplicates,
            risk: RiskLevel::Info,
            title: "Duplicate groups have not been stored".into(),
            detail: "Run `mac-storage duplicates` before expecting content copies in this list. This command does not hash files itself.".into(),
            logical_bytes: 0,
            paths: Vec::new(),
        });
    }

    for set in duplicates {
        if set.redundant_bytes == 0 || set.paths.len() < 2 {
            continue;
        }
        if set.paths.iter().all(|path| is_office_lock(path)) {
            continue;
        }
        if set.paths.iter().all(|path| artifact_root(path).is_some()) {
            artifact_groups += 1;
            artifact_bytes = artifact_bytes.saturating_add(set.redundant_bytes);
            continue;
        }
        let risk = if set.redundant_bytes >= MEDIUM_DUPLICATE_BYTES {
            RiskLevel::Medium
        } else {
            RiskLevel::Low
        };
        recommendations.push(Recommendation {
            category: RecommendationCategory::Duplicates,
            risk,
            title: "Identical files".into(),
            detail: format!(
                "{} copies share the same bytes ({} logical bytes each). Extra copies account for {} logical bytes. Hard links are not multiplied, and nothing will be deleted.",
                set.paths.len(),
                set.logical_size,
                set.redundant_bytes
            ),
            logical_bytes: set.redundant_bytes,
            paths: set.paths.iter().take(PREVIEW).cloned().collect(),
        });
    }

    if artifact_groups > 0 {
        recommendations.push(Recommendation {
            category: RecommendationCategory::Duplicates,
            risk: RiskLevel::Info,
            title: "Duplicate groups inside build directories".into(),
            detail: format!(
                "{artifact_groups} identical groups sit entirely inside dependency or build directories. They are usually package metadata, not extra documents."
            ),
            logical_bytes: artifact_bytes,
            paths: Vec::new(),
        });
    }

    for tree in developer.trees.iter().take(PREVIEW) {
        recommendations.push(Recommendation {
            category: RecommendationCategory::DeveloperArtifacts,
            risk: RiskLevel::Low,
            title: format!("Build directory ({})", tree.kind),
            detail: format!(
                "Review {}. It contains {} files and {} logical bytes. This suggestion does not delete it.",
                tree.path.display(),
                tree.file_count,
                tree.logical_bytes
            ),
            logical_bytes: tree.logical_bytes,
            paths: vec![tree.path.clone()],
        });
    }

    if stale.file_count > 0 {
        recommendations.push(Recommendation {
            category: RecommendationCategory::StaleFiles,
            risk: RiskLevel::Low,
            title: format!("Files unchanged for {} days", stale.older_than_days),
            detail: format!(
                "{} files outside build directories have a modified time at least {} days old, totaling {} logical bytes.",
                stale.file_count, stale.older_than_days, stale.logical_bytes
            ),
            logical_bytes: stale.logical_bytes,
            paths: stale.files.iter().take(PREVIEW).map(|file| file.path.clone()).collect(),
        });
    }

    if let Some(installers) = downloads
        .categories
        .iter()
        .find(|category| category.name == "installers")
    {
        if installers.file_count > 0 {
            recommendations.push(Recommendation {
                category: RecommendationCategory::Downloads,
                risk: RiskLevel::Low,
                title: "Installer images are still in Downloads".into(),
                detail: format!(
                    "{} installer file(s) account for {} logical bytes. This is a review, not a cleanup.",
                    installers.file_count, installers.logical_bytes
                ),
                logical_bytes: installers.logical_bytes,
                paths: installers
                    .largest
                    .iter()
                    .take(PREVIEW)
                    .map(|file| file.path.clone())
                    .collect(),
            });
        }
    }

    if office.file_count > 0 {
        recommendations.push(Recommendation {
            category: RecommendationCategory::Caches,
            risk: RiskLevel::Info,
            title: "Office lock files".into(),
            detail: "Names starting with ~$ are editor lock files. Matching hashes among them are not extra copies of the document.".into(),
            logical_bytes: office.logical_bytes,
            paths: office.files.iter().take(PREVIEW).map(|file| file.path.clone()).collect(),
        });
    }

    if dataless.file_count > 0 {
        recommendations.push(Recommendation {
            category: RecommendationCategory::CloudPlaceholders,
            risk: RiskLevel::Info,
            title: "Not stored on this Mac".into(),
            detail: "These files are iCloud placeholders. Their logical size is not disk space used on this Mac. Moving a placeholder to Trash can remove the copy in iCloud, so this advisor will not move it.".into(),
            logical_bytes: dataless.logical_bytes,
            paths: dataless.files.iter().take(PREVIEW).map(|file| file.path.clone()).collect(),
        });
    }

    if sparse.file_count > 0 {
        recommendations.push(Recommendation {
            category: RecommendationCategory::SharedExtents,
            risk: RiskLevel::Info,
            title: "Allocated size is below the logical size".into(),
            detail: format!(
                "{} files report less allocated space than their logical size, a gap of {} bytes. That can be a sparse file, compression, or shared APFS extents. The gap is not space you free by deleting a different copy.",
                sparse.file_count, sparse.gap_bytes
            ),
            logical_bytes: sparse.gap_bytes,
            paths: sparse.files.iter().take(PREVIEW).map(|file| file.path.clone()).collect(),
        });
    }

    recommendations.sort_by(|left, right| {
        risk_rank(left.risk)
            .cmp(&risk_rank(right.risk))
            .then(right.logical_bytes.cmp(&left.logical_bytes))
            .then(left.title.cmp(&right.title))
    });
    recommendations
}

fn category_totals(grouped: &BTreeMap<&'static str, Vec<&InventoryFile>>) -> Vec<DownloadCategory> {
    let mut categories: Vec<DownloadCategory> = grouped
        .iter()
        .map(|(name, files)| {
            let mut largest: Vec<PathSize> = files
                .iter()
                .map(|file| PathSize {
                    path: file.path.clone(),
                    logical_size: file.logical_size,
                })
                .collect();
            largest.sort_by(|left, right| {
                right
                    .logical_size
                    .cmp(&left.logical_size)
                    .then(left.path.cmp(&right.path))
            });
            largest.truncate(PREVIEW);
            DownloadCategory {
                name,
                file_count: u64::try_from(files.len()).unwrap_or(u64::MAX),
                logical_bytes: files
                    .iter()
                    .fold(0u64, |sum, file| sum.saturating_add(file.logical_size)),
                largest,
            }
        })
        .collect();
    categories.sort_by(|left, right| {
        right
            .logical_bytes
            .cmp(&left.logical_bytes)
            .then(left.name.cmp(right.name))
    });
    categories
}

fn preview_files(files: &[&InventoryFile]) -> Vec<PathSize> {
    files
        .iter()
        .take(PREVIEW)
        .map(|file| PathSize {
            path: file.path.clone(),
            logical_size: file.logical_size,
        })
        .collect()
}

fn sum_sizes(files: &[&InventoryFile]) -> u64 {
    files
        .iter()
        .fold(0u64, |sum, file| sum.saturating_add(file.logical_size))
}

fn is_downloads_root(root: &Path) -> bool {
    root.file_name().and_then(|name| name.to_str()) == Some("Downloads")
}

fn is_office_lock(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("~$"))
}

fn artifact_root(path: &Path) -> Option<(PathBuf, &'static str)> {
    let mut acc = PathBuf::new();
    for component in path.components() {
        acc.push(component);
        let Some(name) = component.as_os_str().to_str() else {
            continue;
        };
        if let Some(kind) = artifact_kind(name) {
            return Some((acc, kind));
        }
    }
    None
}

fn artifact_kind(name: &str) -> Option<&'static str> {
    match name {
        "node_modules" => Some("node_modules"),
        "target" => Some("target"),
        ".venv" => Some(".venv"),
        "venv" => Some("venv"),
        "site-packages" => Some("site-packages"),
        "__pycache__" => Some("__pycache__"),
        ".tox" => Some(".tox"),
        ".gradle" => Some(".gradle"),
        _ if name.ends_with(".egg-info") || name.ends_with(".dist-info") => Some("python metadata"),
        _ => None,
    }
}

fn download_category(extension: Option<&str>) -> &'static str {
    match extension.unwrap_or("").to_ascii_lowercase().as_str() {
        "dmg" | "pkg" | "iso" => "installers",
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "7z" | "rar" => "archives",
        "pdf" | "doc" | "docx" | "ppt" | "pptx" | "xls" | "xlsx" | "txt" | "md" => "documents",
        "mp4" | "mov" | "wmv" | "mp3" | "jpeg" | "jpg" | "png" | "gif" | "heic" | "mid" => "media",
        _ => "other",
    }
}

fn is_stale(modified: SystemTime, now: SystemTime, stale_after: Duration) -> bool {
    now.duration_since(modified)
        .is_ok_and(|age| age >= stale_after)
}

fn days_in(duration: Duration) -> u64 {
    duration.as_secs() / (24 * 60 * 60)
}

fn risk_rank(risk: RiskLevel) -> u8 {
    match risk {
        RiskLevel::High => 0,
        RiskLevel::Medium => 1,
        RiskLevel::Low => 2,
        RiskLevel::Info => 3,
    }
}

/// One folder's stored regular files. `logical_bytes` omits iCloud placeholders.
/// Neither number is reclaimable space, and directory inode sizes are not added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FolderTotal {
    pub path: PathBuf,
    pub local_files: u64,
    pub logical_bytes: u64,
    pub dataless_files: u64,
    pub dataless_bytes: u64,
}

/// Group stored regular files under `root` into folder totals.
///
/// `depth` is how many path components below the root form a group. A file
/// directly in the root stays on the root row. Files deeper than `depth` roll
/// into that ancestor. Placeholder logical size is kept separate.
pub fn folder_totals(root: &Path, files: &[InventoryFile], depth: u32) -> Vec<FolderTotal> {
    let depth = depth.max(1);
    let mut groups: BTreeMap<PathBuf, FolderAccum> = BTreeMap::new();
    for file in files {
        let Some(path) = folder_bucket(root, &file.path, depth) else {
            continue;
        };
        let entry = groups.entry(path).or_default();
        if file.is_dataless {
            entry.dataless_files += 1;
            entry.dataless_bytes = entry.dataless_bytes.saturating_add(file.logical_size);
        } else {
            entry.local_files += 1;
            entry.logical_bytes = entry.logical_bytes.saturating_add(file.logical_size);
        }
    }
    let mut folders: Vec<FolderTotal> = groups
        .into_iter()
        .map(|(path, accum)| FolderTotal {
            path,
            local_files: accum.local_files,
            logical_bytes: accum.logical_bytes,
            dataless_files: accum.dataless_files,
            dataless_bytes: accum.dataless_bytes,
        })
        .collect();
    folders.sort_by(|left, right| {
        right
            .logical_bytes
            .cmp(&left.logical_bytes)
            .then(right.dataless_bytes.cmp(&left.dataless_bytes))
            .then(left.path.cmp(&right.path))
    });
    folders
}

#[derive(Default)]
struct FolderAccum {
    local_files: u64,
    logical_bytes: u64,
    dataless_files: u64,
    dataless_bytes: u64,
}

fn folder_bucket(root: &Path, path: &Path, depth: u32) -> Option<PathBuf> {
    let relative = path.strip_prefix(root).ok()?;
    let parts: Vec<_> = relative.components().collect();
    if parts.is_empty() {
        return None;
    }
    if parts.len() == 1 {
        return Some(root.to_path_buf());
    }
    let keep = usize::try_from(depth)
        .unwrap_or(usize::MAX)
        .min(parts.len() - 1)
        .max(1);
    let mut bucket = root.to_path_buf();
    for component in parts.iter().take(keep) {
        bucket.push(component);
    }
    Some(bucket)
}

fn signed_delta(newest: u64, previous: u64) -> i64 {
    let newest = i64::try_from(newest).unwrap_or(i64::MAX);
    let previous = i64::try_from(previous).unwrap_or(i64::MAX);
    newest.saturating_sub(previous)
}

/// One slice of a storage bar. `percent_hundredths` is a share of the local
/// bytes in that bar: 10000 is 100%. The shares in one bar add up to 10000
/// when the total is not zero. They are not free space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StorageSegment {
    pub label: String,
    pub path: PathBuf,
    pub category: Option<String>,
    pub logical_bytes: u64,
    pub file_count: u64,
    pub percent_hundredths: u32,
}

/// A stored file shown while reviewing one category. Nothing has been moved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CleanupCandidate {
    pub path: PathBuf,
    pub logical_size: u64,
    pub category: String,
    pub extension: Option<String>,
}

/// Turn byte amounts into shares of 100.00% (10000 hundredths).
///
/// The largest remainders get the leftover hundredths. Ties go to the earlier
/// amount, so the same inputs always produce the same shares.
pub fn percent_hundredths(amounts: &[u64]) -> Vec<u32> {
    let total: u64 = amounts
        .iter()
        .fold(0, |sum, amount| sum.saturating_add(*amount));
    if total == 0 {
        return vec![0; amounts.len()];
    }
    let total = u128::from(total);
    let mut shares = Vec::with_capacity(amounts.len());
    let mut remainders = Vec::with_capacity(amounts.len());
    let mut assigned = 0u32;
    for (index, amount) in amounts.iter().enumerate() {
        let product = u128::from(*amount) * 10_000;
        let share = u32::try_from(product / total).unwrap_or(u32::MAX);
        assigned = assigned.saturating_add(share);
        shares.push(share);
        remainders.push((product % total, index));
    }
    remainders.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
    let mut leftover = 10_000u32.saturating_sub(assigned);
    let mut cursor = 0;
    while leftover > 0 && !remainders.is_empty() {
        let index = remainders[cursor % remainders.len()].1;
        shares[index] = shares[index].saturating_add(1);
        leftover -= 1;
        cursor += 1;
    }
    shares
}

/// Top-level folders as shares of the local bytes in the scan.
///
/// Placeholder bytes are left out. A folder with no local bytes is omitted.
/// Files that sit directly in the scan root use the label `In this folder`.
pub fn folder_shares(root: &Path, folders: &[FolderTotal]) -> Vec<StorageSegment> {
    let visible: Vec<&FolderTotal> = folders
        .iter()
        .filter(|folder| folder.logical_bytes > 0)
        .collect();
    let percents = percent_hundredths(
        &visible
            .iter()
            .map(|folder| folder.logical_bytes)
            .collect::<Vec<_>>(),
    );
    visible
        .into_iter()
        .zip(percents)
        .map(|(folder, percent_hundredths)| StorageSegment {
            label: if folder.path == root {
                "In this folder".to_owned()
            } else {
                folder
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("folder")
                    .to_owned()
            },
            path: folder.path.clone(),
            category: None,
            logical_bytes: folder.logical_bytes,
            file_count: folder.local_files,
            percent_hundredths,
        })
        .collect()
}

/// Categories as shares of the local bytes in `files`.
///
/// Empty categories are omitted. Order follows [`CATEGORY_ORDER`].
pub fn category_shares(folder: &Path, files: &[InventoryFile]) -> Vec<StorageSegment> {
    let summaries = categorize_files(files);
    let visible: Vec<_> = summaries
        .iter()
        .filter(|category| category.logical_bytes > 0)
        .collect();
    let percents = percent_hundredths(
        &visible
            .iter()
            .map(|category| category.logical_bytes)
            .collect::<Vec<_>>(),
    );
    visible
        .into_iter()
        .zip(percents)
        .map(|(category, percent_hundredths)| StorageSegment {
            label: category.name.to_owned(),
            path: folder.to_path_buf(),
            category: Some(category.name.to_owned()),
            logical_bytes: category.logical_bytes,
            file_count: category.file_count.saturating_sub(category.dataless_files),
            percent_hundredths,
        })
        .collect()
}

/// Stored files whose depth-1 folder is `folder`.
pub fn files_under_folder(
    root: &Path,
    folder: &Path,
    files: &[InventoryFile],
) -> Vec<InventoryFile> {
    files
        .iter()
        .filter(|file| folder_bucket(root, &file.path, 1).as_deref() == Some(folder))
        .cloned()
        .collect()
}

/// Largest local files to review. Placeholders are omitted. Nothing is moved.
pub fn cleanup_candidates(
    files: &[InventoryFile],
    category: Option<&str>,
    limit: usize,
) -> Vec<CleanupCandidate> {
    let mut rows: Vec<CleanupCandidate> = files
        .iter()
        .filter(|file| !file.is_dataless)
        .filter(|file| {
            category.is_none_or(|name| file_category(&file.path, file.extension.as_deref()) == name)
        })
        .map(|file| CleanupCandidate {
            category: file_category(&file.path, file.extension.as_deref()).to_owned(),
            extension: file.extension.clone(),
            logical_size: file.logical_size,
            path: file.path.clone(),
        })
        .collect();
    rows.sort_by(|left, right| {
        right
            .logical_size
            .cmp(&left.logical_size)
            .then(left.path.cmp(&right.path))
    });
    rows.truncate(limit);
    rows
}

/// What to tell a person before they review a category. This does not move files.
pub fn cleanup_hint(category: &str) -> &'static str {
    match category {
        "documents" => "Large documents are listed for review. Add any you no longer need to Trash, then confirm. Nothing is moved from this list.",
        "images" => "Images are listed largest first. Add the ones you do not want, then confirm Trash. Nothing is moved from this list.",
        "media" => "Video and audio are often the largest files. Review them, then confirm Trash yourself. Nothing is moved from this list.",
        "archives" => "Archives can be unpacked copies. Review them before Trash. Nothing is moved from this list.",
        "installers" => "Installer images are often safe to remove after the app is installed. Confirm Trash yourself. Nothing is moved from this list.",
        "developer" => "Developer folders can be recreated by a build. Review the path, then confirm Trash if you want it gone. Nothing is moved from this list.",
        _ => "These files did not match a clearer category. Review the paths before Trash. Nothing is moved from this list.",
    }
}

/// Stable category order for a stored scan. This is a view. Nothing is moved.
pub const CATEGORY_ORDER: [&str; 7] = [
    "documents",
    "images",
    "media",
    "archives",
    "installers",
    "developer",
    "other",
];

/// One category of stored regular files. `logical_bytes` omits iCloud placeholders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileCategorySummary {
    pub name: &'static str,
    pub file_count: u64,
    pub logical_bytes: u64,
    pub dataless_files: u64,
}

/// A stored file shown inside a category. The path is not changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CategorizedFile {
    pub path: PathBuf,
    pub logical_size: u64,
    pub extension: Option<String>,
    pub is_dataless: bool,
}

/// Where a newly saved file could go, based on its name and files already scanned.
/// Accepting it is a separate confirmed move. This function does not touch the disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlaceSuggestion {
    pub source: PathBuf,
    pub suggested_name: String,
    pub suggested_directory: PathBuf,
    pub suggested_path: PathBuf,
    pub category: &'static str,
    pub reason: String,
}

/// Summarize every category, including ones with no files, in [`CATEGORY_ORDER`].
pub fn categorize_files(files: &[InventoryFile]) -> Vec<FileCategorySummary> {
    CATEGORY_ORDER
        .into_iter()
        .map(|name| {
            let matched: Vec<&InventoryFile> = files
                .iter()
                .filter(|file| file_category(&file.path, file.extension.as_deref()) == name)
                .collect();
            let dataless_files = matched.iter().filter(|file| file.is_dataless).count();
            FileCategorySummary {
                name,
                file_count: u64::try_from(matched.len()).unwrap_or(u64::MAX),
                logical_bytes: matched
                    .iter()
                    .filter(|file| !file.is_dataless)
                    .fold(0u64, |sum, file| sum.saturating_add(file.logical_size)),
                dataless_files: u64::try_from(dataless_files).unwrap_or(u64::MAX),
            }
        })
        .collect()
}

/// Files in one category, sorted by path. `None` means the category name is unknown.
pub fn files_in_category(files: &[InventoryFile], category: &str) -> Option<Vec<CategorizedFile>> {
    if !CATEGORY_ORDER.contains(&category) {
        return None;
    }
    let mut listed: Vec<CategorizedFile> = files
        .iter()
        .filter(|file| file_category(&file.path, file.extension.as_deref()) == category)
        .map(|file| CategorizedFile {
            path: file.path.clone(),
            logical_size: file.logical_size,
            extension: file.extension.clone(),
            is_dataless: file.is_dataless,
        })
        .collect();
    listed.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.logical_size.cmp(&right.logical_size))
    });
    Some(listed)
}

/// Classify a stored path. A file inside a developer-artifact directory stays
/// in `developer` even when its extension would otherwise be a document or image.
pub fn file_category(path: &Path, extension: Option<&str>) -> &'static str {
    if artifact_root(path).is_some() {
        return "developer";
    }
    match extension.unwrap_or("").to_ascii_lowercase().as_str() {
        "pdf" | "doc" | "docx" | "ppt" | "pptx" | "xls" | "xlsx" | "txt" | "md" | "rtf" | "csv"
        | "pages" | "key" | "numbers" | "odt" | "ods" | "odp" => "documents",
        "jpg" | "jpeg" | "png" | "gif" | "heic" | "heif" | "webp" | "tif" | "tiff" | "bmp"
        | "svg" | "raw" | "cr2" | "nef" => "images",
        "mp4" | "mov" | "m4v" | "avi" | "mkv" | "wmv" | "mp3" | "m4a" | "wav" | "aac" | "flac"
        | "aiff" => "media",
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "7z" | "rar" | "xz" => "archives",
        "dmg" | "pkg" | "iso" | "exe" | "msi" => "installers",
        _ => "other",
    }
}

/// Suggest a file name and folder for `source` from the stored scan.
///
/// The folder is the one that already holds the most files of the same
/// extension, skipping iCloud placeholders, developer-artifact trees, and
/// protected macOS paths. The name drops a trailing `copy` or ` (1)` style
/// suffix. Nothing is renamed or moved.
pub fn suggest_place(source: &Path, inventory: &[InventoryFile]) -> PlaceSuggestion {
    let source = source.to_path_buf();
    let original_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("untitled")
        .to_owned();
    let suggested_name = suggested_file_name(&original_name);
    let extension = Path::new(&original_name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());
    let category = file_category(&source, extension.as_deref());
    let current_dir = source
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();

    let mut counts: BTreeMap<PathBuf, u64> = BTreeMap::new();
    for file in inventory {
        if file.is_dataless || file.path == source {
            continue;
        }
        if artifact_root(&file.path).is_some() {
            continue;
        }
        let file_ext = file
            .extension
            .as_deref()
            .map(|ext| ext.to_ascii_lowercase());
        if file_ext != extension {
            continue;
        }
        let Some(parent) = file.path.parent() else {
            continue;
        };
        if parent.as_os_str().is_empty() || is_protected_path(parent) {
            continue;
        }
        let entry = counts.entry(parent.to_path_buf()).or_default();
        *entry = entry.saturating_add(1);
    }

    let suggested_directory = counts
        .iter()
        .max_by(|left, right| left.1.cmp(right.1).then_with(|| right.0.cmp(left.0)))
        .map(|(path, _)| path.clone())
        .unwrap_or_else(|| current_dir.clone());
    let peer_count = counts.get(&suggested_directory).copied().unwrap_or(0);
    let suggested_path = suggested_directory.join(&suggested_name);
    let ext_label = extension.unwrap_or_else(|| "file".into());
    let mut reason = if peer_count == 0 {
        format!(
            "No other {ext_label} files in this scan, so the suggestion keeps the current folder."
        )
    } else {
        format!(
            "{peer_count} other {ext_label} file{} already live in {}.",
            if peer_count == 1 { "" } else { "s" },
            suggested_directory.display()
        )
    };
    if suggested_name != original_name {
        reason.push_str(" The suggested name drops a copy or download suffix.");
    }
    if suggested_path == source {
        reason.push_str(" The file is already in that place.");
    }
    PlaceSuggestion {
        source,
        suggested_name,
        suggested_directory,
        suggested_path,
        category,
        reason,
    }
}

/// Drop a trailing ` copy` or ` (digits)` from a file name. The extension stays.
pub fn suggested_file_name(file_name: &str) -> String {
    let (stem, extension) = match file_name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() && !extension.is_empty() => {
            (stem, Some(extension))
        }
        _ => (file_name, None),
    };
    let mut cleaned = stem.trim().to_owned();
    loop {
        let next = strip_name_suffix(&cleaned);
        if next == cleaned {
            break;
        }
        cleaned = next;
    }
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        return file_name.to_owned();
    }
    match extension {
        Some(extension) => format!("{cleaned}.{extension}"),
        None => cleaned,
    }
}

fn strip_name_suffix(stem: &str) -> String {
    let lower = stem.to_ascii_lowercase();
    for suffix in [" - copy", " copy"] {
        if lower.ends_with(suffix) && stem.len() >= suffix.len() {
            let keep = stem.len() - suffix.len();
            return stem[..keep].trim_end().to_owned();
        }
    }
    if let Some(open) = stem.rfind(" (") {
        let inside = &stem[open + 2..];
        if let Some(digits) = inside.strip_suffix(')') {
            if !digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit()) {
                return stem[..open].trim_end().to_owned();
            }
        }
    }
    stem.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(
        path: &str,
        size: u64,
        age_days: Option<u64>,
        extension: Option<&str>,
    ) -> InventoryFile {
        let modified = age_days.map(|days| {
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_000 * 24 * 60 * 60)
                - Duration::from_secs(days * 24 * 60 * 60)
        });
        InventoryFile {
            path: PathBuf::from(path),
            logical_size: size,
            allocated_size: Some(size),
            modified,
            extension: extension.map(str::to_owned),
            is_dataless: false,
        }
    }

    fn now() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_000 * 24 * 60 * 60)
    }

    #[test]
    fn stale_files_skip_build_trees_and_missing_mtime() {
        let files = vec![
            file("/Users/ram/Downloads/old.pdf", 50, Some(400), Some("pdf")),
            file("/Users/ram/Downloads/new.pdf", 10, Some(1), Some("pdf")),
            file("/Users/ram/Downloads/unknown.pdf", 9, None, Some("pdf")),
            file(
                "/Users/ram/Downloads/proj/.venv/lib/old.py",
                80,
                Some(400),
                Some("py"),
            ),
        ];
        let report = advise(
            Path::new("/Users/ram/Downloads"),
            &files,
            &[],
            now(),
            stale_duration(180),
        );
        assert_eq!(report.stale.file_count, 1);
        assert_eq!(report.stale.logical_bytes, 50);
        assert_eq!(
            report.stale.files[0].path,
            PathBuf::from("/Users/ram/Downloads/old.pdf")
        );
        assert_eq!(report.developer.trees.len(), 1);
        assert_eq!(report.developer.trees[0].kind, ".venv");
        assert!(report.developer.trees[0].path.ends_with("proj/.venv"));
    }

    #[test]
    fn site_packages_inside_venv_is_one_tree() {
        let files = vec![file(
            "/tmp/Downloads/app/.venv/lib/site-packages/pkg/a.py",
            4,
            Some(1),
            Some("py"),
        )];
        let report = advise(
            Path::new("/tmp/Downloads"),
            &files,
            &[],
            now(),
            stale_duration(180),
        );
        assert_eq!(report.developer.trees.len(), 1);
        assert!(report.developer.trees[0].path.ends_with(".venv"));
    }

    #[test]
    fn downloads_review_classifies_installers_and_ignores_other_roots() {
        let files = vec![
            file("/Users/ram/Downloads/Grok.dmg", 100, Some(1), Some("DMG")),
            file("/Users/ram/Downloads/notes.pdf", 20, Some(1), Some("pdf")),
        ];
        let report = advise(
            Path::new("/Users/ram/Downloads"),
            &files,
            &[],
            now(),
            stale_duration(180),
        );
        assert!(report.downloads.applies);
        let installers = report
            .downloads
            .categories
            .iter()
            .find(|category| category.name == "installers")
            .unwrap();
        assert_eq!(installers.file_count, 1);
        assert_eq!(installers.logical_bytes, 100);
        let other_root = advise(
            Path::new("/Users/ram/Documents"),
            &files,
            &[],
            now(),
            stale_duration(180),
        );
        assert!(!other_root.downloads.applies);
        assert!(other_root.downloads.categories.is_empty());
    }

    #[test]
    fn suggestions_separate_documents_from_package_metadata_and_lock_files() {
        let files = vec![
            file(
                "/Users/ram/Downloads/~$notes.docx",
                162,
                Some(1),
                Some("docx"),
            ),
            file(
                "/Users/ram/Downloads/proj/.venv/a.py",
                10,
                Some(1),
                Some("py"),
            ),
        ];
        let duplicates = vec![
            DuplicateSet {
                logical_size: 3_000_000,
                redundant_bytes: 3_000_000,
                paths: vec![
                    PathBuf::from("/Users/ram/Downloads/a.pdf"),
                    PathBuf::from("/Users/ram/Downloads/a (1).pdf"),
                ],
            },
            DuplicateSet {
                logical_size: 2,
                redundant_bytes: 96,
                paths: vec![
                    PathBuf::from("/Users/ram/Downloads/proj/.venv/INSTALLER"),
                    PathBuf::from("/Users/ram/Downloads/proj/.venv/other/INSTALLER"),
                ],
            },
            DuplicateSet {
                logical_size: 162,
                redundant_bytes: 162,
                paths: vec![
                    PathBuf::from("/Users/ram/Downloads/~$one.docx"),
                    PathBuf::from("/Users/ram/Downloads/~$two.docx"),
                ],
            },
        ];
        let report = advise(
            Path::new("/Users/ram/Downloads"),
            &files,
            &duplicates,
            now(),
            stale_duration(180),
        );
        let identical = report
            .recommendations
            .iter()
            .find(|item| item.title == "Identical files")
            .unwrap();
        assert_eq!(identical.risk, RiskLevel::Medium);
        assert_eq!(identical.logical_bytes, 3_000_000);
        assert!(report.recommendations.iter().any(|item| {
            item.title == "Duplicate groups inside build directories"
                && item.risk == RiskLevel::Info
        }));
        assert!(report
            .recommendations
            .iter()
            .any(|item| item.category == RecommendationCategory::Caches));
        assert!(report
            .recommendations
            .iter()
            .all(|item| item.risk != RiskLevel::High));
        let small = DuplicateSet {
            logical_size: 5,
            redundant_bytes: 5,
            paths: vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")],
        };
        let small_report = advise(
            Path::new("/tmp/box"),
            &[],
            &[small],
            now(),
            stale_duration(180),
        );
        assert_eq!(small_report.recommendations[0].risk, RiskLevel::Low);
    }

    #[test]
    fn dataless_files_are_not_stale_and_sparse_gaps_are_not_reclaimable() {
        let mut cloud = file(
            "/Users/ram/Downloads/old.pdf",
            5_000,
            Some(400),
            Some("pdf"),
        );
        cloud.is_dataless = true;
        cloud.allocated_size = Some(0);
        let mut sparse = file(
            "/Users/ram/Documents/movie.mov",
            10_000,
            Some(1),
            Some("mov"),
        );
        sparse.allocated_size = Some(1_000);
        let report = advise(
            Path::new("/Users/ram/Downloads"),
            &[cloud, sparse],
            &[],
            now(),
            stale_duration(180),
        );
        assert_eq!(report.stale.file_count, 0);
        assert_eq!(report.dataless.file_count, 1);
        assert_eq!(report.dataless.logical_bytes, 5_000);
        assert_eq!(report.sparse_or_shared.file_count, 1);
        assert_eq!(report.sparse_or_shared.gap_bytes, 9_000);
        assert!(report.recommendations.iter().any(|item| {
            item.category == RecommendationCategory::CloudPlaceholders
                && item.risk == RiskLevel::Info
                && item.detail.contains("will not move")
        }));
        assert!(report.recommendations.iter().any(|item| {
            item.category == RecommendationCategory::SharedExtents
                && item.detail.contains("not space you free")
        }));
        assert!(report
            .downloads
            .categories
            .iter()
            .all(|category| category.name != "documents" || category.file_count == 0));
    }

    #[test]
    fn trends_do_not_subtract_different_roots() {
        let series = trends(vec![
            TrendPoint {
                scan_id: 1,
                root: PathBuf::from("/Users/ram/Downloads"),
                logical_bytes: 100,
                allocated_bytes: 200,
                files_scanned: 2,
            },
            TrendPoint {
                scan_id: 2,
                root: PathBuf::from("/Users/ram/Documents"),
                logical_bytes: 40,
                allocated_bytes: 40,
                files_scanned: 1,
            },
            TrendPoint {
                scan_id: 3,
                root: PathBuf::from("/Users/ram/Downloads"),
                logical_bytes: 80,
                allocated_bytes: 150,
                files_scanned: 2,
            },
        ]);
        assert_eq!(series.len(), 2);
        let downloads = series
            .iter()
            .find(|item| item.root.ends_with("Downloads"))
            .unwrap();
        assert_eq!(downloads.points.len(), 2);
        assert_eq!(downloads.logical_delta, Some(-20));
        assert_eq!(downloads.allocated_delta, Some(-50));
        let documents = series
            .iter()
            .find(|item| item.root.ends_with("Documents"))
            .unwrap();
        assert_eq!(documents.logical_delta, None);
    }

    #[test]
    fn folder_totals_roll_children_and_keep_placeholders_separate() {
        let root = Path::new("/tmp/scan");
        let mut cloud = file("/tmp/scan/cloud/old.pdf", 5_000, None, Some("pdf"));
        cloud.is_dataless = true;
        let files = vec![
            file("/tmp/scan/notes.txt", 5, None, Some("txt")),
            file("/tmp/scan/sub/b.txt", 30, None, Some("txt")),
            file("/tmp/scan/sub/nested/c.txt", 7, None, Some("txt")),
            file("/tmp/scan/other/d.txt", 2, None, Some("txt")),
            cloud,
            file("/tmp/elsewhere/nope.txt", 100, None, Some("txt")),
        ];
        let shallow = folder_totals(root, &files, 1);
        assert_eq!(shallow.len(), 4);
        let sub = shallow
            .iter()
            .find(|row| row.path.ends_with("sub"))
            .unwrap();
        assert_eq!(sub.logical_bytes, 37);
        assert_eq!(sub.local_files, 2);
        assert_eq!(sub.dataless_files, 0);
        let here = shallow.iter().find(|row| row.path == root).unwrap();
        assert_eq!(here.logical_bytes, 5);
        assert_eq!(here.local_files, 1);
        let cloud_row = shallow
            .iter()
            .find(|row| row.path.ends_with("cloud"))
            .unwrap();
        assert_eq!(cloud_row.logical_bytes, 0);
        assert_eq!(cloud_row.dataless_bytes, 5_000);
        assert_eq!(cloud_row.dataless_files, 1);
        assert!(shallow.iter().all(|row| !row.path.ends_with("nope.txt")));
        assert!(shallow[0].logical_bytes >= shallow[1].logical_bytes);

        let deep = folder_totals(root, &files, 2);
        let nested = deep
            .iter()
            .find(|row| row.path.ends_with("nested"))
            .unwrap();
        assert_eq!(nested.logical_bytes, 7);
        let sub_only = deep.iter().find(|row| row.path.ends_with("sub")).unwrap();
        assert_eq!(sub_only.logical_bytes, 30);
        assert_eq!(sub_only.local_files, 1);
    }

    #[test]
    fn categories_group_extensions_and_leave_placeholders_out_of_the_bytes() {
        let mut cloud = file("/tmp/scan/Photos/trip.heic", 9_000, None, Some("heic"));
        cloud.is_dataless = true;
        let files = vec![
            file("/tmp/scan/Docs/notes.pdf", 100, None, Some("pdf")),
            file("/tmp/scan/Docs/sheet.xlsx", 40, None, Some("xlsx")),
            file("/tmp/scan/Photos/cat.jpg", 20, None, Some("jpg")),
            cloud,
            file("/tmp/scan/Movies/clip.mp4", 80, None, Some("mp4")),
            file("/tmp/scan/Downloads/app.dmg", 50, None, Some("dmg")),
            file("/tmp/scan/Downloads/src.zip", 10, None, Some("zip")),
            file(
                "/tmp/scan/proj/node_modules/pkg/index.js",
                5,
                None,
                Some("js"),
            ),
            file("/tmp/scan/proj/main.rs", 7, None, Some("rs")),
        ];
        let summary = categorize_files(&files);
        assert_eq!(
            summary.iter().map(|item| item.name).collect::<Vec<_>>(),
            CATEGORY_ORDER.to_vec()
        );
        let documents = summary
            .iter()
            .find(|item| item.name == "documents")
            .unwrap();
        assert_eq!(documents.file_count, 2);
        assert_eq!(documents.logical_bytes, 140);
        let images = summary.iter().find(|item| item.name == "images").unwrap();
        assert_eq!(images.file_count, 2);
        assert_eq!(images.logical_bytes, 20);
        assert_eq!(images.dataless_files, 1);
        let media = summary.iter().find(|item| item.name == "media").unwrap();
        assert_eq!(media.file_count, 1);
        assert_eq!(media.logical_bytes, 80);
        assert_eq!(
            summary
                .iter()
                .find(|item| item.name == "developer")
                .unwrap()
                .file_count,
            1
        );
        assert_eq!(
            summary
                .iter()
                .find(|item| item.name == "other")
                .unwrap()
                .file_count,
            1
        );
        let docs = files_in_category(&files, "documents").unwrap();
        assert_eq!(docs.len(), 2);
        assert!(docs[0].path.ends_with("notes.pdf"));
        assert!(files_in_category(&files, "secrets").is_none());
        let developer = files_in_category(&files, "developer").unwrap();
        assert!(developer[0].path.ends_with("index.js"));
    }

    #[test]
    fn place_suggestion_uses_the_common_folder_and_drops_a_copy_suffix() {
        let files = vec![
            file("/tmp/scan/Documents/taxes/2024.pdf", 10, None, Some("pdf")),
            file("/tmp/scan/Documents/taxes/2023.pdf", 10, None, Some("pdf")),
            file("/tmp/scan/Downloads/random.pdf", 10, None, Some("pdf")),
            file(
                "/tmp/scan/proj/node_modules/left.pdf",
                10,
                None,
                Some("pdf"),
            ),
            file("/tmp/scan/Pictures/cat.jpg", 4, None, Some("jpg")),
        ];
        let suggestion = suggest_place(Path::new("/tmp/scan/Downloads/Invoice (1).pdf"), &files);
        assert_eq!(suggestion.category, "documents");
        assert_eq!(suggestion.suggested_name, "Invoice.pdf");
        assert_eq!(
            suggestion.suggested_directory,
            PathBuf::from("/tmp/scan/Documents/taxes")
        );
        assert_eq!(
            suggestion.suggested_path,
            PathBuf::from("/tmp/scan/Documents/taxes/Invoice.pdf")
        );
        assert!(!suggestion.reason.contains("node_modules"));

        let tied = vec![
            file("/tmp/scan/b/one.pdf", 1, None, Some("pdf")),
            file("/tmp/scan/a/two.pdf", 1, None, Some("pdf")),
        ];
        let tie = suggest_place(Path::new("/tmp/scan/Downloads/note.pdf"), &tied);
        assert_eq!(tie.suggested_directory, PathBuf::from("/tmp/scan/a"));

        let alone = suggest_place(Path::new("/tmp/scan/Downloads/song.mp3"), &files);
        assert_eq!(alone.category, "media");
        assert_eq!(
            alone.suggested_directory,
            PathBuf::from("/tmp/scan/Downloads")
        );
        assert_eq!(alone.suggested_name, "song.mp3");
        assert_eq!(suggested_file_name("Report - Copy.pdf"), "Report.pdf");
        assert_eq!(
            suggested_file_name("final (draft).txt"),
            "final (draft).txt"
        );

        let protected = vec![file("/usr/share/doc/manual.pdf", 1, None, Some("pdf"))];
        let stay = suggest_place(Path::new("/tmp/scan/Downloads/manual (2).pdf"), &protected);
        assert_eq!(
            stay.suggested_directory,
            PathBuf::from("/tmp/scan/Downloads")
        );
        assert_eq!(stay.suggested_name, "manual.pdf");

        let mut cloud = file("/tmp/scan/iCloud/only.pdf", 1, None, Some("pdf"));
        cloud.is_dataless = true;
        let ignored = suggest_place(Path::new("/tmp/scan/Desktop/new.pdf"), &[cloud]);
        assert_eq!(
            ignored.suggested_directory,
            PathBuf::from("/tmp/scan/Desktop")
        );
    }

    #[test]
    fn storage_shares_sum_to_the_whole_bar_and_skip_placeholders() {
        let shares = percent_hundredths(&[1, 1, 1]);
        assert_eq!(shares, vec![3334, 3333, 3333]);
        assert_eq!(shares.iter().sum::<u32>(), 10_000);
        assert_eq!(percent_hundredths(&[0, 0]), vec![0, 0]);
        assert_eq!(percent_hundredths(&[5, 0, 5]), vec![5000, 0, 5000]);

        let root = Path::new("/tmp/scan");
        let mut cloud = file("/tmp/scan/Cloud/only.pdf", 9_000, None, Some("pdf"));
        cloud.is_dataless = true;
        let files = vec![
            file("/tmp/scan/Documents/a.pdf", 60, None, Some("pdf")),
            file("/tmp/scan/Documents/b.pdf", 40, None, Some("pdf")),
            file("/tmp/scan/Downloads/app.dmg", 100, None, Some("dmg")),
            file("/tmp/scan/notes.txt", 10, None, Some("txt")),
            cloud,
        ];
        let folders = folder_totals(root, &files, 1);
        let bar = folder_shares(root, &folders);
        assert_eq!(
            bar.iter().map(|item| item.percent_hundredths).sum::<u32>(),
            10_000
        );
        assert!(bar.iter().all(|item| item.logical_bytes > 0));
        assert!(bar.iter().all(|item| item.label != "Cloud"));
        let documents = bar.iter().find(|item| item.label == "Documents").unwrap();
        assert_eq!(documents.logical_bytes, 100);
        assert_eq!(documents.percent_hundredths, 4762);
        let downloads = bar.iter().find(|item| item.label == "Downloads").unwrap();
        assert_eq!(downloads.percent_hundredths, 4762);
        let here = bar
            .iter()
            .find(|item| item.label == "In this folder")
            .unwrap();
        assert_eq!(here.percent_hundredths, 476);

        let inside = files_under_folder(root, Path::new("/tmp/scan/Documents"), &files);
        assert_eq!(inside.len(), 2);
        let categories = category_shares(Path::new("/tmp/scan/Documents"), &inside);
        assert_eq!(categories.len(), 1);
        assert_eq!(categories[0].category.as_deref(), Some("documents"));
        assert_eq!(categories[0].percent_hundredths, 10_000);

        let review = cleanup_candidates(&inside, Some("documents"), 10);
        assert_eq!(review.len(), 2);
        assert!(review[0].path.ends_with("a.pdf"));
        assert!(cleanup_candidates(&files, Some("documents"), 10)
            .iter()
            .all(|file| !file.path.ends_with("only.pdf")));
        assert!(cleanup_hint("installers").contains("Nothing is moved"));
    }
}
