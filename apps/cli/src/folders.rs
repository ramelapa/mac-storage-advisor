//! Folder totals from regular files already stored for one scan.

use std::path::PathBuf;

use clap::Args;
use mac_storage_analyze::{folder_totals, FolderTotal, InventoryFile};
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
