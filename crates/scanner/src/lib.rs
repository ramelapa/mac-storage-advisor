//! Recursive directory scan.
//!
//! The walk uses [`walkdir`] with `follow_links(false)`. Symlinks are recorded
//! and never descended into, which also prevents symlink loops. Scanning does
//! not read file contents. Per-entry failures are stored on the snapshot and
//! do not abort the walk.
//!
//! This version is single-threaded. `ScanTarget::threads` is recorded and
//! logged; it does not fan out reads.

#![forbid(unsafe_code)]

mod exclude;
mod metadata;

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use mac_storage_common::{
    FileKind, ScanSnapshot, ScanStatistics, ScanStatus, ScanTarget, SCAN_CONCURRENCY,
};
use walkdir::WalkDir;

pub use exclude::{is_exact_protected_root, protected_prefixes_under, ExclusionSet};

use exclude::normalize_lexical;
use metadata::{collect_metadata, directory_record, file_record, CollectedMeta};

/// Failures that stop a scan before any walk, or that prevent the walk from starting.
#[derive(Debug, thiserror::Error)]
pub enum ScanFatal {
    #[error("scan root does not exist: {0}")]
    RootNotFound(PathBuf),
    #[error("scan root is not a directory: {0}")]
    RootNotDirectory(PathBuf),
    #[error("refusing to follow symlink at scan root: {0}")]
    RootIsSymlink(PathBuf),
    #[error(
        "refusing to scan protected root {0} (pass --allow-protected-roots to override for tests)"
    )]
    ProtectedRoot(PathBuf),
    #[error("exclusion `{pattern}` matches the scan root; refusing to exclude the entire scan")]
    ExclusionMatchesRoot { pattern: String },
    #[error("invalid exclusion `{pattern}`: {reason}")]
    InvalidExclusion { pattern: String, reason: String },
    #[error("failed to resolve scan root: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug)]
struct ScanState {
    stats: ScanStatistics,
    files: Vec<mac_storage_common::FileRecord>,
    directories: Vec<mac_storage_common::DirectoryRecord>,
    errors: Vec<mac_storage_common::ScanErrorRecord>,
}

impl ScanState {
    fn new() -> Self {
        Self {
            stats: ScanStatistics::default(),
            files: Vec::new(),
            directories: Vec::new(),
            errors: Vec::new(),
        }
    }
}

#[derive(Debug)]
enum WalkItem {
    Entry(PathBuf),
    Failed {
        path: Option<PathBuf>,
        message: String,
    },
}

/// Scan `target.root`. Recorded I/O errors are on the returned snapshot.
pub fn scan_path(target: &ScanTarget) -> Result<ScanSnapshot, ScanFatal> {
    let started = Instant::now();
    let started_at = SystemTime::now();
    let root = resolve_root(&target.root)?;

    if target.threads > SCAN_CONCURRENCY {
        tracing::info!(
            requested = target.threads,
            concurrency = SCAN_CONCURRENCY,
            "scan is single-threaded in this version; --threads is recorded only"
        );
    }

    let node = match std::fs::symlink_metadata(&root) {
        Ok(meta) => meta,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            return Err(ScanFatal::RootNotFound(root));
        }
        Err(err) => {
            log_started(target.redact_paths, &root);
            let mut state = ScanState::new();
            push_error(
                &mut state,
                Some(root.clone()),
                format!("failed to read scan root metadata: {err}"),
            );
            let snapshot = finish(target, root, started_at, started, state);
            log_finished(&snapshot.statistics);
            return Ok(snapshot);
        }
    };

    let file_type = node.file_type();
    if file_type.is_symlink() {
        return Err(ScanFatal::RootIsSymlink(root));
    }
    if !file_type.is_dir() {
        return Err(ScanFatal::RootNotDirectory(root));
    }
    if !target.allow_protected_roots && is_exact_protected_root(&root) {
        return Err(ScanFatal::ProtectedRoot(root));
    }

    log_started(target.redact_paths, &root);

    let exclusions = ExclusionSet::compile(&root, target)?;
    let mut skipped = 0u64;
    let min_logical_size = target.min_logical_size;
    let redact_entries = target.redact_paths;
    let redact_skips = target.redact_paths;
    let walker = WalkDir::new(&root)
        .follow_links(false)
        .contents_first(false)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            if exclusions.matches(entry.path()) {
                skipped += 1;
                if redact_skips {
                    tracing::debug!("skipped excluded path");
                } else {
                    tracing::debug!(path = %entry.path().display(), "skipped excluded path");
                }
                false
            } else {
                true
            }
        });

    let mut state = ScanState::new();
    for entry in walker {
        let item = match entry {
            Ok(dir_entry) => WalkItem::Entry(dir_entry.path().to_path_buf()),
            Err(err) => WalkItem::Failed {
                path: err.path().map(Path::to_path_buf),
                message: err.to_string(),
            },
        };
        handle_item(&mut state, item, min_logical_size, redact_entries);
    }
    state.stats.skipped = skipped;

    let snapshot = finish(target, root, started_at, started, state);
    log_finished(&snapshot.statistics);
    Ok(snapshot)
}

fn log_started(redact: bool, root: &Path) {
    if redact {
        tracing::info!("scan started");
    } else {
        tracing::info!(root = %root.display(), "scan started");
    }
}

fn log_finished(stats: &ScanStatistics) {
    tracing::info!(
        directories = stats.directories_scanned,
        files = stats.files_scanned,
        logical_bytes = stats.logical_bytes,
        skipped = stats.skipped,
        errors = stats.errors,
        elapsed_ms = stats.elapsed_ms,
        "scan finished"
    );
}

fn resolve_root(root: &Path) -> Result<PathBuf, ScanFatal> {
    let absolute = std::path::absolute(root)?;
    Ok(normalize_lexical(&absolute))
}

fn finish(
    target: &ScanTarget,
    root: PathBuf,
    started_at: SystemTime,
    started: Instant,
    state: ScanState,
) -> ScanSnapshot {
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let mut stats = state.stats;
    stats.elapsed_ms = elapsed_ms;
    let status = if state.errors.is_empty() {
        ScanStatus::Completed
    } else {
        ScanStatus::CompletedWithErrors
    };
    let exclusions = ExclusionSet::persisted_patterns(&root, &target.exclusions);
    ScanSnapshot {
        root,
        started_at,
        finished_at: SystemTime::now(),
        status,
        statistics: stats,
        files: state.files,
        directories: state.directories,
        errors: state.errors,
        exclusions,
        threads_requested: target.threads,
        min_logical_size: target.min_logical_size,
        allow_protected_roots: target.allow_protected_roots,
        redact_paths: target.redact_paths,
    }
}

fn handle_item(state: &mut ScanState, item: WalkItem, min_logical_size: u64, redact: bool) {
    match item {
        WalkItem::Failed { path, message } => push_error(state, path, message),
        WalkItem::Entry(path) => match collect_metadata(&path) {
            Ok(meta) => accept_meta(state, &path, &meta, min_logical_size, redact),
            Err(err) => {
                if redact {
                    tracing::warn!(message = %err.message, "entry failed");
                } else {
                    tracing::warn!(path = %path.display(), message = %err.message, "entry failed");
                }
                state.stats.errors = state.stats.errors.saturating_add(1);
                state.errors.push(err);
            }
        },
    }
}

fn accept_meta(
    state: &mut ScanState,
    path: &Path,
    meta: &CollectedMeta,
    min_logical_size: u64,
    redact: bool,
) {
    if !meta.path_is_utf8(path) {
        push_error(
            state,
            Some(path.to_path_buf()),
            "path is not valid UTF-8; stored fields may be lossy".to_owned(),
        );
    }

    match meta.kind {
        FileKind::Directory => {
            state.stats.directories_scanned = state.stats.directories_scanned.saturating_add(1);
            if redact {
                tracing::debug!("directory");
            } else {
                tracing::debug!(path = %path.display(), "directory");
            }
            state.directories.push(directory_record(path, meta));
        }
        FileKind::File => {
            state.stats.files_scanned = state.stats.files_scanned.saturating_add(1);
            state.stats.logical_bytes = state.stats.logical_bytes.saturating_add(meta.logical_size);
            match meta.allocated_size {
                Some(size) => {
                    state.stats.allocated_bytes = state.stats.allocated_bytes.saturating_add(size);
                }
                None => state.stats.allocated_bytes_complete = false,
            }
            if redact {
                tracing::debug!(logical_size = meta.logical_size, "file");
            } else {
                tracing::debug!(path = %path.display(), logical_size = meta.logical_size, "file");
            }
            if meta.logical_size >= min_logical_size {
                state.files.push(file_record(path, meta));
            } else {
                state.stats.files_below_min_size =
                    state.stats.files_below_min_size.saturating_add(1);
            }
        }
        FileKind::Symlink => {
            state.stats.symlinks = state.stats.symlinks.saturating_add(1);
            if redact {
                tracing::debug!(broken = meta.is_broken_symlink, "symlink");
            } else {
                tracing::debug!(
                    path = %path.display(),
                    broken = meta.is_broken_symlink,
                    "symlink"
                );
            }
            state.files.push(file_record(path, meta));
        }
        FileKind::Other => {
            state.stats.other_entries = state.stats.other_entries.saturating_add(1);
            state.files.push(file_record(path, meta));
        }
    }
}

fn push_error(state: &mut ScanState, path: Option<PathBuf>, message: String) {
    tracing::warn!(message = %message, "scan error");
    state.stats.errors = state.stats.errors.saturating_add(1);
    state
        .errors
        .push(mac_storage_common::ScanErrorRecord { path, message });
}

impl CollectedMeta {
    fn path_is_utf8(&self, path: &Path) -> bool {
        path.to_str().is_some()
            && self
                .link_target
                .as_ref()
                .map(|target| target.to_str().is_some())
                .unwrap_or(true)
    }
}

/// Root policy used before any walk. `kind` is determined by the caller so tests
/// do not have to touch protected system directories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootKind {
    Missing,
    Symlink,
    Directory,
    Other,
}

pub fn validate_root_policy(
    path: &Path,
    kind: RootKind,
    allow_protected_roots: bool,
) -> Result<(), ScanFatal> {
    match kind {
        RootKind::Missing => Err(ScanFatal::RootNotFound(path.to_path_buf())),
        RootKind::Symlink => Err(ScanFatal::RootIsSymlink(path.to_path_buf())),
        RootKind::Other => Err(ScanFatal::RootNotDirectory(path.to_path_buf())),
        RootKind::Directory if !allow_protected_roots && is_exact_protected_root(path) => {
            Err(ScanFatal::ProtectedRoot(path.to_path_buf()))
        }
        RootKind::Directory => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mac_storage_common::ScanErrorRecord;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    fn target(root: &Path) -> ScanTarget {
        ScanTarget::new(root.to_path_buf())
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "mac-storage-scan-{}-{}-{}",
                std::process::id(),
                n,
                nanos
            ));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            relax_permissions(&self.path);
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn relax_permissions(path: &Path) {
        let Ok(meta) = fs::symlink_metadata(path) else {
            return;
        };
        if meta.file_type().is_symlink() {
            return;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = fs::set_permissions(path, perms);
        }
        if meta.is_dir() {
            if let Ok(read) = fs::read_dir(path) {
                for entry in read.flatten() {
                    relax_permissions(&entry.path());
                }
            }
        }
    }

    #[test]
    fn protected_root_is_refused_without_a_filesystem_walk() {
        let err = validate_root_policy(Path::new("/usr"), RootKind::Directory, false).unwrap_err();
        assert!(matches!(err, ScanFatal::ProtectedRoot(_)));
        assert!(validate_root_policy(Path::new("/usr"), RootKind::Directory, true).is_ok());
        assert!(validate_root_policy(Path::new("/usr/local"), RootKind::Directory, false).is_ok());
        let link = validate_root_policy(Path::new("/tmp/link"), RootKind::Symlink, false);
        assert!(matches!(link, Err(ScanFatal::RootIsSymlink(_))));
    }

    #[test]
    fn errors_do_not_abort_later_entries() {
        let dir = TempDir::new();
        let good = dir.path.join("good.txt");
        let gone = dir.path.join("gone.txt");
        let also = dir.path.join("also.txt");
        fs::write(&good, b"hello").unwrap();
        fs::write(&gone, b"bye").unwrap();
        fs::remove_file(&gone).unwrap();
        fs::write(&also, b"zz").unwrap();

        let mut state = ScanState::new();
        handle_item(&mut state, WalkItem::Entry(good.clone()), 0, false);
        handle_item(
            &mut state,
            WalkItem::Failed {
                path: Some(dir.path.join("locked")),
                message: "permission denied".into(),
            },
            0,
            false,
        );
        handle_item(&mut state, WalkItem::Entry(gone), 0, false);
        handle_item(&mut state, WalkItem::Entry(also), 0, false);

        assert_eq!(state.stats.files_scanned, 2);
        assert_eq!(state.errors.len(), 2);
        assert_eq!(state.stats.errors, 2);
        assert!(state
            .errors
            .iter()
            .any(|err| err.message.contains("permission denied")));
        assert!(state.errors.iter().any(|err| {
            err.path
                .as_ref()
                .is_some_and(|path| path.ends_with("gone.txt"))
        }));
        assert!(state.files.iter().any(|file| file.filename == "good.txt"));
        assert!(state.files.iter().any(|file| file.filename == "also.txt"));
    }

    #[test]
    fn recursive_scan_counts_nested_files() {
        let dir = TempDir::new();
        fs::write(dir.path.join("a.txt"), b"aaaa").unwrap();
        fs::create_dir(dir.path.join("d1")).unwrap();
        fs::write(dir.path.join("d1").join("b.txt"), b"bb").unwrap();
        fs::create_dir(dir.path.join("d1").join("d2")).unwrap();
        fs::write(dir.path.join("d1").join("d2").join("c.txt"), b"c").unwrap();

        let snap = scan_path(&target(&dir.path)).unwrap();
        assert_eq!(snap.statistics.directories_scanned, 3);
        assert_eq!(snap.statistics.files_scanned, 3);
        assert_eq!(snap.statistics.logical_bytes, 7);
        assert_eq!(snap.statistics.errors, 0);
        assert_eq!(snap.status, ScanStatus::Completed);
    }

    #[test]
    fn metadata_captured_for_a_normal_file() {
        let dir = TempDir::new();
        let path = dir.path.join("notes.txt");
        fs::write(&path, b"hello").unwrap();
        let sparse = dir.path.join("sparse.bin");
        let file = fs::File::create(&sparse).unwrap();
        file.set_len(1_048_576).unwrap();
        drop(file);

        let snap = scan_path(&target(&dir.path)).unwrap();
        let notes = snap
            .files
            .iter()
            .find(|file| file.filename == "notes.txt")
            .unwrap();
        assert_eq!(notes.kind, FileKind::File);
        assert_eq!(notes.logical_size, 5);
        assert_eq!(notes.extension.as_deref(), Some("txt"));
        assert!(!notes.is_symlink);
        assert!(notes.modified.is_some());
        assert!(notes.path.is_absolute());
        assert!(notes.path.ends_with("notes.txt"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let meta = fs::symlink_metadata(&path).unwrap();
            assert_eq!(notes.inode, Some(meta.ino()));
            assert_eq!(notes.device_id, Some(meta.dev()));
            assert!(notes.permissions.is_some());
            assert_eq!(
                notes.allocated_size,
                Some(meta.blocks().saturating_mul(512))
            );
        }

        #[cfg(target_os = "linux")]
        {
            let meta = fs::symlink_metadata(&path).unwrap();
            // Birth time is whatever `Metadata::created` returns. On Linux that
            // is often `None`. Never substitute modified time.
            assert_eq!(notes.created, meta.created().ok());
            if notes.created.is_none() {
                assert!(notes.modified.is_some());
            }
        }

        let sparse_rec = snap
            .files
            .iter()
            .find(|file| file.filename == "sparse.bin")
            .unwrap();
        assert_eq!(sparse_rec.logical_size, 1_048_576);
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let meta = fs::symlink_metadata(&sparse).unwrap();
            let allocated = meta.blocks().saturating_mul(512);
            assert_eq!(sparse_rec.allocated_size, Some(allocated));
            if allocated != meta.len() {
                assert_ne!(sparse_rec.allocated_size, Some(sparse_rec.logical_size));
            }
        }
    }

    #[test]
    fn zero_byte_file_is_counted_and_recorded() {
        let dir = TempDir::new();
        fs::write(dir.path.join("empty.dat"), b"").unwrap();
        let snap = scan_path(&target(&dir.path)).unwrap();
        assert_eq!(snap.statistics.files_scanned, 1);
        assert_eq!(snap.statistics.logical_bytes, 0);
        let file = &snap.files[0];
        assert_eq!(file.logical_size, 0);
        assert_eq!(file.kind, FileKind::File);
    }

    #[test]
    fn min_size_filters_persistence_but_not_stats() {
        let dir = TempDir::new();
        fs::write(dir.path.join("big.txt"), b"0123456789").unwrap();
        fs::write(dir.path.join("small.txt"), b"ab").unwrap();
        let mut options = target(&dir.path);
        options.min_logical_size = 5;
        let snap = scan_path(&options).unwrap();
        assert_eq!(snap.statistics.files_scanned, 2);
        assert_eq!(snap.statistics.logical_bytes, 12);
        assert_eq!(snap.statistics.files_below_min_size, 1);
        assert_eq!(snap.files.len(), 1);
        assert_eq!(snap.files[0].filename, "big.txt");
    }

    #[test]
    fn exclusion_skips_a_directory() {
        let dir = TempDir::new();
        fs::write(dir.path.join("keep.txt"), b"keep").unwrap();
        fs::create_dir(dir.path.join("skip")).unwrap();
        fs::write(dir.path.join("skip").join("hidden.txt"), b"hide").unwrap();
        let mut options = target(&dir.path);
        options.exclusions.push("skip".into());
        let snap = scan_path(&options).unwrap();
        assert_eq!(snap.statistics.files_scanned, 1);
        assert_eq!(snap.statistics.skipped, 1);
        assert!(snap.files.iter().all(|file| file.filename != "hidden.txt"));
        assert!(snap
            .directories
            .iter()
            .all(|directory| directory.filename != "skip"));
    }

    #[test]
    fn empty_directory_scans() {
        let dir = TempDir::new();
        let snap = scan_path(&target(&dir.path)).unwrap();
        assert_eq!(snap.statistics.directories_scanned, 1);
        assert_eq!(snap.statistics.files_scanned, 0);
        assert_eq!(snap.statistics.errors, 0);
    }

    #[test]
    fn missing_root_is_fatal() {
        let missing = std::env::temp_dir().join(format!(
            "mac-storage-missing-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let err = scan_path(&target(&missing)).unwrap_err();
        assert!(matches!(err, ScanFatal::RootNotFound(_)));
    }

    #[test]
    fn file_root_is_fatal() {
        let dir = TempDir::new();
        let file = dir.path.join("solo.txt");
        fs::write(&file, b"x").unwrap();
        let err = scan_path(&target(&file)).unwrap_err();
        assert!(matches!(err, ScanFatal::RootNotDirectory(_)));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_is_not_followed_and_loop_finishes() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new();
        let outside = dir.path.join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("secret.txt"), b"do-not-count").unwrap();
        let root = dir.path.join("root");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("own.txt"), b"own").unwrap();
        symlink(dir.path.join("outside"), root.join("outlink")).unwrap();
        symlink("loop2", root.join("loop1")).unwrap();
        symlink("loop1", root.join("loop2")).unwrap();
        symlink("missing-target", root.join("broken")).unwrap();
        symlink(root.join("own.txt"), root.join("filelink")).unwrap();

        let started = Instant::now();
        let snap = scan_path(&target(&root)).unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "symlink loop should not hang"
        );
        assert_eq!(snap.statistics.files_scanned, 1);
        assert_eq!(snap.statistics.logical_bytes, 3);
        assert!(snap.files.iter().all(|file| file.filename != "secret.txt"));
        let broken = snap
            .files
            .iter()
            .find(|file| file.filename == "broken")
            .unwrap();
        assert_eq!(broken.kind, FileKind::Symlink);
        assert!(broken.is_symlink);
        assert!(broken.is_broken_symlink);
        assert_eq!(
            broken.link_target.as_deref(),
            Some(Path::new("missing-target"))
        );
        let outlink = snap
            .files
            .iter()
            .find(|file| file.filename == "outlink")
            .unwrap();
        assert_eq!(outlink.kind, FileKind::Symlink);
        assert!(!outlink.is_broken_symlink);
        assert_eq!(snap.statistics.errors, 0);
        assert!(snap.statistics.symlinks >= 4);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_scan_root_is_refused() {
        use std::os::unix::fs::symlink;
        let dir = TempDir::new();
        let real = dir.path.join("real");
        fs::create_dir(&real).unwrap();
        fs::write(real.join("a.txt"), b"a").unwrap();
        let link = dir.path.join("link");
        symlink(&real, &link).unwrap();
        let err = scan_path(&target(&link)).unwrap_err();
        assert!(matches!(err, ScanFatal::RootIsSymlink(_)));
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_directory_is_recorded_and_scan_continues() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new();
        let probe = dir.path.join("probe");
        fs::create_dir(&probe).unwrap();
        fs::write(probe.join("x.txt"), b"x").unwrap();
        let mut perms = fs::metadata(&probe).unwrap().permissions();
        perms.set_mode(0o0);
        fs::set_permissions(&probe, perms).unwrap();
        let enforced = fs::read_dir(&probe).is_err();
        let mut restore = fs::symlink_metadata(&probe).unwrap().permissions();
        restore.set_mode(0o755);
        fs::set_permissions(&probe, restore).unwrap();
        if !enforced {
            // chmod 000 does not deny access when the process is privileged.
            eprintln!("skipping permission-denied scan: mode 000 is still readable");
            return;
        }

        let root = dir.path.join("tree");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("visible.txt"), b"see-me").unwrap();
        fs::write(root.join("also.txt"), b"too").unwrap();
        let locked = root.join("locked");
        fs::create_dir(&locked).unwrap();
        fs::write(locked.join("secret.txt"), b"hidden").unwrap();
        let mut perms = fs::metadata(&locked).unwrap().permissions();
        perms.set_mode(0o0);
        fs::set_permissions(&locked, perms).unwrap();

        let snap = scan_path(&target(&root)).unwrap();
        assert!(snap.statistics.errors >= 1);
        assert_eq!(snap.status, ScanStatus::CompletedWithErrors);
        assert!(snap.files.iter().any(|file| file.filename == "visible.txt"));
        assert!(snap.files.iter().any(|file| file.filename == "also.txt"));
        assert!(snap.files.iter().all(|file| file.filename != "secret.txt"));
        assert!(snap.errors.iter().any(|err| {
            err.path
                .as_ref()
                .is_some_and(|path| path.ends_with("locked"))
                || err.message.to_ascii_lowercase().contains("denied")
                || err.message.to_ascii_lowercase().contains("permission")
        }));

        let mut restore = fs::symlink_metadata(&locked).unwrap().permissions();
        restore.set_mode(0o755);
        fs::set_permissions(&locked, restore).unwrap();
    }

    #[test]
    fn scan_does_not_retain_file_contents() {
        let dir = TempDir::new();
        let marker = "SENTINEL_DO_NOT_PERSIST_9f3a2c";
        fs::write(dir.path.join("note.txt"), marker.as_bytes()).unwrap();
        let snap = scan_path(&target(&dir.path)).unwrap();
        let rendered = format!("{snap:?}");
        assert!(!rendered.contains(marker));
        assert!(snap.files.iter().any(|file| file.filename == "note.txt"));
    }

    #[test]
    fn recorded_error_shape_has_path_and_message_only() {
        let err = ScanErrorRecord {
            path: Some(PathBuf::from("/tmp/example")),
            message: "permission denied".into(),
        };
        assert!(err.path.is_some());
        assert!(!err.message.is_empty());
    }
}
