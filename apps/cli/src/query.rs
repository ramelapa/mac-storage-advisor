//! Commands that read a scan already stored in SQLite.

use std::path::PathBuf;

use clap::Args;
use mac_storage_common::{PRODUCT_NAME, PRODUCT_VERSION, SCAN_CONCURRENCY};
use mac_storage_duplicates::{
    find_duplicates_reporting, Candidate, DuplicateAnalysis, DuplicateProgress, HASH_ALGORITHM,
    HASH_CONCURRENCY,
};
use mac_storage_storage::{
    resolve_db_path, Database, NewContentHash, NewDuplicateGroup, NewDuplicateMember, ScanSummary,
    StoredFile,
};
use serde::Serialize;

use crate::{report, Error};

#[derive(Debug, Args)]
pub struct DuplicatesArgs {
    /// Stored scan id. Defaults to the newest scan.
    #[arg(long)]
    scan: Option<i64>,

    /// Re-read duplicate candidates and drop a file whose bytes disagree with BLAKE3.
    #[arg(long)]
    verify: bool,

    /// Print a stable JSON object on stdout. Logs stay on stderr.
    #[arg(long)]
    json: bool,

    /// Debug logging on stderr. File contents are never logged.
    #[arg(long, conflicts_with = "quiet")]
    verbose: bool,

    /// Suppress the human summary and info logs.
    #[arg(long)]
    quiet: bool,

    /// SQLite path. Overrides the MAC_STORAGE_DB environment variable.
    #[arg(long, value_name = "PATH")]
    db: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct LargeArgs {
    /// Stored scan id. Defaults to the newest scan.
    #[arg(long)]
    scan: Option<i64>,

    /// How many files to print.
    #[arg(long, default_value_t = 20)]
    limit: u64,

    /// Omit stored regular files smaller than this logical size.
    #[arg(long, value_name = "BYTES", default_value = "0", value_parser = crate::size::parse_byte_size)]
    min_size: u64,

    /// Print a stable JSON object on stdout. Logs stay on stderr.
    #[arg(long)]
    json: bool,

    /// Debug logging on stderr. File contents are never logged.
    #[arg(long, conflicts_with = "quiet")]
    verbose: bool,

    /// Suppress the human summary and info logs.
    #[arg(long)]
    quiet: bool,

    /// SQLite path. Overrides the MAC_STORAGE_DB environment variable.
    #[arg(long, value_name = "PATH")]
    db: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct HistoryArgs {
    /// How many scans to print, newest first.
    #[arg(long, default_value_t = 20)]
    limit: u64,

    /// Print a stable JSON object on stdout. Logs stay on stderr.
    #[arg(long)]
    json: bool,

    /// Debug logging on stderr. File contents are never logged.
    #[arg(long, conflicts_with = "quiet")]
    verbose: bool,

    /// Suppress the human summary and info logs.
    #[arg(long)]
    quiet: bool,

    /// SQLite path. Overrides the MAC_STORAGE_DB environment variable.
    #[arg(long, value_name = "PATH")]
    db: Option<PathBuf>,
}

pub fn duplicates_command(args: DuplicatesArgs) -> Result<(), Error> {
    crate::init_tracing(args.verbose, args.quiet, args.json);
    let report = run_duplicates(args.db.as_deref(), args.scan, args.verify)?;
    if args.json {
        report::write_value(&report, std::io::stdout())?;
    } else if !args.quiet {
        print!("{}", format_duplicates(&report));
    }
    Ok(())
}

pub(crate) fn run_duplicates(
    db: Option<&std::path::Path>,
    scan: Option<i64>,
    verify: bool,
) -> Result<DuplicatesReport, Error> {
    run_duplicates_reporting(db, scan, verify, |_| {})
}

pub(crate) fn run_duplicates_reporting<F>(
    db: Option<&std::path::Path>,
    scan: Option<i64>,
    verify: bool,
    on_progress: F,
) -> Result<DuplicatesReport, Error>
where
    F: FnMut(DuplicateProgress),
{
    let (mut database, db_path) = open_database(db)?;
    let scan_id = database
        .resolve_scan_id(scan)
        .map_err(|err| Error::Storage(err.to_string()))?;
    let root = database
        .scan_root(scan_id)
        .map_err(|err| Error::Storage(err.to_string()))?;
    let stored = database
        .list_regular_files(scan_id)
        .map_err(|err| Error::Storage(err.to_string()))?;
    tracing::info!(
        scan_id,
        files = stored.len(),
        verify,
        hash_concurrency = HASH_CONCURRENCY,
        "duplicate pass started"
    );
    let analysis =
        find_duplicates_reporting(&candidates(&stored), verify, HASH_CONCURRENCY, on_progress);
    database
        .replace_duplicate_result(scan_id, &hash_rows(&analysis), &group_rows(&analysis))
        .map_err(|err| Error::Storage(err.to_string()))?;
    tracing::info!(
        scan_id,
        groups = analysis.groups.len(),
        hard_link_sets = analysis.hard_link_sets.len(),
        hash_errors = analysis.errors.len(),
        "duplicate pass finished"
    );
    Ok(DuplicatesReport {
        product: PRODUCT_NAME,
        version: PRODUCT_VERSION,
        scan_id,
        root,
        database: db_path,
        duplicate_groups: analysis.groups.clone(),
        hard_link_sets: analysis.hard_link_sets.clone(),
        hash_errors: analysis.errors.clone(),
        verified: verify,
        concurrency: SCAN_CONCURRENCY,
        hash_concurrency: u32::try_from(HASH_CONCURRENCY).unwrap_or(u32::MAX),
        algorithm: HASH_ALGORITHM,
    })
}

pub fn large_files_command(args: LargeArgs) -> Result<(), Error> {
    crate::init_tracing(args.verbose, args.quiet, args.json);
    let report = run_large_files(args.db.as_deref(), args.scan, args.limit, args.min_size)?;
    if args.json {
        report::write_value(&report, std::io::stdout())?;
    } else if !args.quiet {
        print!("{}", format_large(&report));
    }
    Ok(())
}

pub(crate) fn run_large_files(
    db: Option<&std::path::Path>,
    scan: Option<i64>,
    limit: u64,
    min_size: u64,
) -> Result<LargeFilesReport, Error> {
    if limit == 0 {
        return Err(Error::Usage("--limit must be at least 1".into()));
    }
    let (database, db_path) = open_database(db)?;
    let scan_id = database
        .resolve_scan_id(scan)
        .map_err(|err| Error::Storage(err.to_string()))?;
    let root = database
        .scan_root(scan_id)
        .map_err(|err| Error::Storage(err.to_string()))?;
    let files = database
        .largest_files(scan_id, limit, min_size)
        .map_err(|err| Error::Storage(err.to_string()))?;
    Ok(LargeFilesReport {
        product: PRODUCT_NAME,
        version: PRODUCT_VERSION,
        scan_id,
        root,
        database: db_path,
        files: files
            .iter()
            .map(|file| LargeFile {
                file_id: file.id,
                path: file.record.path.clone(),
                logical_size: file.record.logical_size,
                allocated_size: file.record.allocated_size,
            })
            .collect(),
    })
}

pub fn history_command(args: HistoryArgs) -> Result<(), Error> {
    crate::init_tracing(args.verbose, args.quiet, args.json);
    let report = run_history(args.db.as_deref(), args.limit)?;
    if args.json {
        report::write_value(&report, std::io::stdout())?;
    } else if !args.quiet {
        print!("{}", format_history(&report));
    }
    Ok(())
}

pub(crate) fn run_history(
    db: Option<&std::path::Path>,
    limit: u64,
) -> Result<HistoryReport, Error> {
    if limit == 0 {
        return Err(Error::Usage("--limit must be at least 1".into()));
    }
    let (database, db_path) = open_database(db)?;
    let scans = database
        .list_scans(limit)
        .map_err(|err| Error::Storage(err.to_string()))?;
    Ok(HistoryReport {
        product: PRODUCT_NAME,
        version: PRODUCT_VERSION,
        database: db_path,
        scans: scans.iter().map(history_row).collect(),
    })
}

fn open_database(flag: Option<&std::path::Path>) -> Result<(Database, PathBuf), Error> {
    let path = resolve_db_path(flag).map_err(|err| Error::Storage(err.to_string()))?;
    let database = Database::open(&path).map_err(|err| Error::Storage(err.to_string()))?;
    Ok((database, path))
}

fn candidates(files: &[StoredFile]) -> Vec<Candidate> {
    files
        .iter()
        .map(|file| Candidate {
            id: file.id,
            path: file.record.path.clone(),
            logical_size: file.record.logical_size,
            inode: file.record.inode,
            device_id: file.record.device_id,
        })
        .collect()
}

fn hash_rows(analysis: &DuplicateAnalysis) -> Vec<NewContentHash> {
    analysis
        .hashes
        .iter()
        .map(|hash| NewContentHash {
            file_id: hash.file_id,
            algorithm: hash.algorithm.to_owned(),
            sample_hash: Some(hash.sample_hash.clone()),
            full_hash: hash.full_hash.clone(),
            hashed_bytes: hash.hashed_bytes,
        })
        .collect()
}

fn group_rows(analysis: &DuplicateAnalysis) -> Vec<NewDuplicateGroup> {
    analysis
        .groups
        .iter()
        .map(|group| NewDuplicateGroup {
            logical_size: group.logical_size,
            full_hash: group.full_hash.clone(),
            redundant_bytes: group.redundant_bytes,
            members: group
                .members
                .iter()
                .map(|member| NewDuplicateMember {
                    file_id: member.file_id,
                    hard_link_leader: member.hard_link_leader,
                })
                .collect(),
        })
        .collect()
}

fn history_row(scan: &ScanSummary) -> HistoryScan {
    HistoryScan {
        id: scan.id,
        root: scan.root.clone(),
        status: scan.status.as_str().to_owned(),
        directories_scanned: scan.directories_scanned,
        files_scanned: scan.files_scanned,
        logical_bytes: scan.logical_bytes,
        errors: scan.error_count,
        elapsed_ms: scan.elapsed_ms,
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DuplicatesReport {
    product: &'static str,
    version: &'static str,
    scan_id: i64,
    root: PathBuf,
    database: PathBuf,
    algorithm: &'static str,
    verified: bool,
    concurrency: u32,
    hash_concurrency: u32,
    duplicate_groups: Vec<mac_storage_duplicates::DuplicateGroup>,
    hard_link_sets: Vec<mac_storage_duplicates::HardLinkSet>,
    hash_errors: Vec<mac_storage_duplicates::HashError>,
}

#[derive(Debug, Serialize)]
pub(crate) struct LargeFilesReport {
    product: &'static str,
    version: &'static str,
    scan_id: i64,
    root: PathBuf,
    database: PathBuf,
    files: Vec<LargeFile>,
}

#[derive(Debug, Serialize)]
pub(crate) struct LargeFile {
    file_id: i64,
    path: PathBuf,
    logical_size: u64,
    allocated_size: Option<u64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct HistoryReport {
    product: &'static str,
    version: &'static str,
    database: PathBuf,
    scans: Vec<HistoryScan>,
}

#[derive(Debug, Serialize)]
struct HistoryScan {
    id: i64,
    root: PathBuf,
    status: String,
    directories_scanned: u64,
    files_scanned: u64,
    logical_bytes: u64,
    errors: u64,
    elapsed_ms: u64,
}

pub(crate) fn format_duplicates(report: &DuplicatesReport) -> String {
    let mut out = format!(
        "{PRODUCT_NAME} {PRODUCT_VERSION}\nScan: {}\nRoot: {}\nDuplicate groups: {}\nHard-link sets: {}\nRedundant logical bytes: {} ({})\nHash errors: {}\n",
        report.scan_id,
        report.root.display(),
        report.duplicate_groups.len(),
        report.hard_link_sets.len(),
        report
            .duplicate_groups
            .iter()
            .map(|group| group.redundant_bytes)
            .fold(0u64, u64::saturating_add),
        "extra content copies; not bytes the disk will free",
        report.hash_errors.len()
    );
    out.push_str(&format!(
        "Hash workers: {} (capped). The folder walk stays one thread.\n",
        report.hash_concurrency
    ));
    if report.verified {
        out.push_str("Byte verification: on\n");
    }
    for (index, group) in report.duplicate_groups.iter().enumerate() {
        out.push_str(&format!(
            "\nGroup {}: {} logical bytes, {} members, redundant {}\n  hash {}\n",
            index + 1,
            group.logical_size,
            group.members.len(),
            group.redundant_bytes,
            group.full_hash
        ));
        for member in &group.members {
            let role = if member.hard_link_leader {
                "copy"
            } else {
                "hard link"
            };
            out.push_str(&format!("  [{role}] {}\n", member.path.display()));
        }
    }
    if !report.hard_link_sets.is_empty() {
        out.push_str("\nHard links (same inode, redundant bytes 0):\n");
        for set in &report.hard_link_sets {
            out.push_str(&format!(
                "  device {} inode {}, {} logical bytes\n",
                set.device_id, set.inode, set.logical_size
            ));
            for path in &set.paths {
                out.push_str(&format!("    {}\n", path.path.display()));
            }
        }
    }
    for error in &report.hash_errors {
        out.push_str(&format!(
            "\nerror: {}: {}\n",
            error.path.display(),
            error.message
        ));
    }
    out
}

pub(crate) fn format_large(report: &LargeFilesReport) -> String {
    let mut out = format!(
        "{PRODUCT_NAME} {PRODUCT_VERSION}\nScan: {}\nRoot: {}\nLargest files: {}\n",
        report.scan_id,
        report.root.display(),
        report.files.len()
    );
    for file in &report.files {
        out.push_str(&format!(
            "  {} ({})  {}\n",
            file.logical_size,
            report::format_bytes(file.logical_size),
            file.path.display()
        ));
    }
    out
}

pub(crate) fn format_history(report: &HistoryReport) -> String {
    let mut out = format!(
        "{PRODUCT_NAME} {PRODUCT_VERSION}\nScans: {}\n",
        report.scans.len()
    );
    for scan in &report.scans {
        out.push_str(&format!(
            "\n#{}  {}\n    {}  files {}  logical {} ({})  errors {}  elapsed {} ms\n",
            scan.id,
            scan.root.display(),
            scan.status,
            scan.files_scanned,
            scan.logical_bytes,
            report::format_bytes(scan.logical_bytes),
            scan.errors,
            scan.elapsed_ms
        ));
    }
    out
}
