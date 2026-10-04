//! Folder totals from regular files already stored for one scan.

use std::path::PathBuf;

use clap::Args;
use mac_storage_analyze::{
    category_shares, cleanup_candidates, cleanup_hint, files_under_folder, folder_shares,
    folder_totals, CleanupCandidate, FolderTotal, InventoryFile, StorageSegment, CATEGORY_ORDER,
};
use mac_storage_common::{PRODUCT_NAME, PRODUCT_VERSION};
use mac_storage_storage::{resolve_db_path, Database};
use serde::Serialize;

use crate::{report, Error};

#[derive(Debug, Args)]
pub struct FolderArgs {
    /// Stored scan id. Defaults to the newest scan.
    #[arg(long)]
    scan: Option<i64>,

    /// How many components below the scan root to group. Deeper files roll into that folder.
    #[arg(long, default_value_t = 1)]
    depth: u32,

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

pub fn folders_command(args: FolderArgs) -> Result<(), Error> {
    crate::init_tracing(args.verbose, args.quiet, args.json);
    let report = run_folders(args.db.as_deref(), args.scan, args.depth)?;
    if args.json {
        report::write_value(&report, std::io::stdout())?;
    } else if !args.quiet {
        print!("{}", format_folders(&report));
    }
    Ok(())
}

pub(crate) fn run_folders(
    db: Option<&std::path::Path>,
    scan: Option<i64>,
    depth: u32,
) -> Result<FolderReport, Error> {
    if depth == 0 {
        return Err(Error::Usage("--depth must be at least 1".into()));
    }
    let db_path = resolve_db_path(db).map_err(|err| Error::Storage(err.to_string()))?;
    let database = Database::open(&db_path).map_err(|err| Error::Storage(err.to_string()))?;
    let scan_id = database
        .resolve_scan_id(scan)
        .map_err(|err| Error::Storage(err.to_string()))?;
    let root = database
        .scan_root(scan_id)
        .map_err(|err| Error::Storage(err.to_string()))?;
    let stored = database
        .list_regular_files(scan_id)
        .map_err(|err| Error::Storage(err.to_string()))?;
    let files: Vec<InventoryFile> = stored
        .iter()
        .map(|file| InventoryFile {
            path: file.record.path.clone(),
            logical_size: file.record.logical_size,
            allocated_size: file.record.allocated_size,
            modified: file.record.modified,
            extension: file.record.extension.clone(),
            is_dataless: file.record.is_dataless,
        })
        .collect();
    let folders = folder_totals(&root, &files, depth);
    let logical_bytes = folders
        .iter()
        .fold(0u64, |sum, row| sum.saturating_add(row.logical_bytes));
    let dataless_bytes = folders
        .iter()
        .fold(0u64, |sum, row| sum.saturating_add(row.dataless_bytes));
    Ok(FolderReport {
        product: PRODUCT_NAME,
        version: PRODUCT_VERSION,
        scan_id,
        root,
        database: db_path,
        depth,
        logical_bytes,
        dataless_bytes,
        folders,
    })
}

#[derive(Debug, Serialize)]
pub(crate) struct FolderReport {
    pub(crate) product: &'static str,
    pub(crate) version: &'static str,
    pub(crate) scan_id: i64,
    pub(crate) root: PathBuf,
    pub(crate) database: PathBuf,
    pub(crate) depth: u32,
    pub(crate) logical_bytes: u64,
    pub(crate) dataless_bytes: u64,
    pub(crate) folders: Vec<FolderTotal>,
}

pub(crate) fn format_folders(report: &FolderReport) -> String {
    let mut out = format!(
        "{PRODUCT_NAME} {PRODUCT_VERSION}\nScan: {}\nRoot: {}\nDepth: {}\n\
         Stored regular files grouped by folder. Not reclaimable space. \
         Placeholder bytes are not local disk usage. Directory inode sizes are not included.\n",
        report.scan_id,
        report.root.display(),
        report.depth
    );
    if report.folders.is_empty() {
        out.push_str("No stored regular files.\n");
        return out;
    }
    for folder in &report.folders {
        out.push_str(&format!(
            "  {} local bytes, {} files",
            folder.logical_bytes, folder.local_files
        ));
        if folder.dataless_files > 0 {
            out.push_str(&format!(
                ", {} placeholder bytes, {} files",
                folder.dataless_bytes, folder.dataless_files
            ));
        }
        out.push_str(&format!("  {}\n", folder.path.display()));
    }
    out
}

#[derive(Debug, Args)]
pub struct StorageArgs {
    /// Stored scan id. Defaults to the newest scan.
    #[arg(long)]
    scan: Option<i64>,

    /// Top-level folder to open. A name such as Documents, or its full path.
    #[arg(long)]
    folder: Option<String>,

    /// Category inside that folder: documents, images, media, archives, installers, developer, or other.
    #[arg(long)]
    category: Option<String>,

    /// How many files to list when a category is open.
    #[arg(long, default_value_t = 40)]
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

#[derive(Debug, Clone, Serialize)]
pub(crate) struct StorageReport {
    pub(crate) product: &'static str,
    pub(crate) version: &'static str,
    pub(crate) scan_id: i64,
    pub(crate) root: PathBuf,
    pub(crate) database: PathBuf,
    pub(crate) total_local_bytes: u64,
    pub(crate) dataless_bytes: u64,
    pub(crate) segments: Vec<StorageSegment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) drill: Option<StorageDrill>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct StorageDrill {
    pub(crate) folder: PathBuf,
    pub(crate) label: String,
    pub(crate) category: Option<String>,
    pub(crate) total_local_bytes: u64,
    pub(crate) categories: Vec<StorageSegment>,
    pub(crate) files: Vec<CleanupCandidate>,
    pub(crate) hint: String,
}

pub fn storage_command(args: StorageArgs) -> Result<(), Error> {
    crate::init_tracing(args.verbose, args.quiet, args.json);
    let report = run_storage(
        args.db.as_deref(),
        args.scan,
        args.folder.as_deref(),
        args.category.as_deref(),
        args.limit,
    )?;
    if args.json {
        report::write_value(&report, std::io::stdout())?;
    } else if !args.quiet {
        print!("{}", format_storage(&report));
    }
    Ok(())
}

pub(crate) fn run_storage(
    db: Option<&std::path::Path>,
    scan: Option<i64>,
    folder: Option<&str>,
    category: Option<&str>,
    limit: u64,
) -> Result<StorageReport, Error> {
    if limit == 0 || limit > 200 {
        return Err(Error::Usage("--limit must be from 1 to 200".into()));
    }
    let category = category.map(str::trim).filter(|value| !value.is_empty());
    if category.is_some() && folder.is_none() {
        return Err(Error::Usage(
            "--category needs --folder; nothing was moved".into(),
        ));
    }
    if let Some(name) = category {
        if !CATEGORY_ORDER.contains(&name) {
            return Err(Error::Usage(format!("unknown category {name}")));
        }
    }
    let (scan_id, root, db_path, files) = load_files(db, scan)?;
    let totals = folder_totals(&root, &files, 1);
    let total_local_bytes = totals
        .iter()
        .fold(0u64, |sum, row| sum.saturating_add(row.logical_bytes));
    let dataless_bytes = totals
        .iter()
        .fold(0u64, |sum, row| sum.saturating_add(row.dataless_bytes));
    let segments = folder_shares(&root, &totals);
    let drill = match folder.map(str::trim).filter(|value| !value.is_empty()) {
        None => None,
        Some(requested) => {
            let chosen = segments
                .iter()
                .find(|segment| {
                    segment.label == requested || segment.path.to_string_lossy() == requested
                })
                .ok_or_else(|| {
                    Error::Usage(format!(
                        "folder {requested} is not a top-level folder in this scan"
                    ))
                })?;
            let inside = files_under_folder(&root, &chosen.path, &files);
            let categories = category_shares(&chosen.path, &inside);
            let file_limit = usize::try_from(limit).unwrap_or(usize::MAX);
            let files = match category {
                Some(name) => cleanup_candidates(&inside, Some(name), file_limit),
                None => Vec::new(),
            };
            let hint = category
                .map(cleanup_hint)
                .unwrap_or("Choose a category to review the largest files. Nothing is moved.")
                .to_owned();
            Some(StorageDrill {
                folder: chosen.path.clone(),
                label: chosen.label.clone(),
                category: category.map(str::to_owned),
                total_local_bytes: chosen.logical_bytes,
                categories,
                files,
                hint,
            })
        }
    };
    Ok(StorageReport {
        product: PRODUCT_NAME,
        version: PRODUCT_VERSION,
        scan_id,
        root,
        database: db_path,
        total_local_bytes,
        dataless_bytes,
        segments,
        drill,
    })
}

pub(crate) fn format_storage(report: &StorageReport) -> String {
    let mut out = format!(
        "{PRODUCT_NAME} {PRODUCT_VERSION}\nScan: {}\nRoot: {}\n\
         Local files in this scan: {} bytes. Each percent is that folder's share of those bytes. \
         This is not free space, and iCloud placeholders are not included.\n",
        report.scan_id,
        report.root.display(),
        report.total_local_bytes
    );
    if report.segments.is_empty() {
        out.push_str("No local files in this scan.\n");
    }
    for segment in &report.segments {
        out.push_str(&format!(
            "  {}  {} bytes  {} files  {}\n",
            format_percent(segment.percent_hundredths),
            segment.logical_bytes,
            segment.file_count,
            segment.label
        ));
    }
    if let Some(drill) = &report.drill {
        let share = report
            .segments
            .iter()
            .find(|segment| segment.path == drill.folder)
            .map(|segment| segment.percent_hundredths)
            .unwrap_or(0);
        out.push_str(&format!(
            "\n{} is {} of this scan ({} bytes). Categories below share that folder. Nothing is moved.\n",
            drill.label,
            format_percent(share),
            drill.total_local_bytes
        ));
        for category in &drill.categories {
            out.push_str(&format!(
                "  {}  {} bytes  {} files  {}\n",
                format_percent(category.percent_hundredths),
                category.logical_bytes,
                category.file_count,
                category.label
            ));
        }
        if let Some(name) = &drill.category {
            out.push_str(&format!("\n{name}: {}\n", drill.hint));
            if drill.files.is_empty() {
                out.push_str("No local files in this category.\n");
            }
            for file in &drill.files {
                out.push_str(&format!(
                    "  {} bytes  {}\n",
                    file.logical_size,
                    file.path.display()
                ));
            }
        }
    }
    out
}

fn format_percent(hundredths: u32) -> String {
    format!("{}.{:02}%", hundredths / 100, hundredths % 100)
}

fn load_files(
    db: Option<&std::path::Path>,
    scan: Option<i64>,
) -> Result<(i64, PathBuf, PathBuf, Vec<InventoryFile>), Error> {
    let db_path = resolve_db_path(db).map_err(|err| Error::Storage(err.to_string()))?;
    let database = Database::open(&db_path).map_err(|err| Error::Storage(err.to_string()))?;
    let scan_id = database
        .resolve_scan_id(scan)
        .map_err(|err| Error::Storage(err.to_string()))?;
    let root = database
        .scan_root(scan_id)
        .map_err(|err| Error::Storage(err.to_string()))?;
    let stored = database
        .list_regular_files(scan_id)
        .map_err(|err| Error::Storage(err.to_string()))?;
    let files = stored
        .iter()
        .map(|file| InventoryFile {
            path: file.record.path.clone(),
            logical_size: file.record.logical_size,
            allocated_size: file.record.allocated_size,
            modified: file.record.modified,
            extension: file.record.extension.clone(),
            is_dataless: file.record.is_dataless,
        })
        .collect();
    Ok((scan_id, root, db_path, files))
}
