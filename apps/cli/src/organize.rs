//! Category view and a confirmed move into a suggested folder.
//!
//! Categories are a reading of the stored scan. A file is renamed only when
//! the caller passes the move confirmation phrase. Nothing is deleted.

use std::path::{Path, PathBuf};

use clap::Args;
use mac_storage_analyze::{
    categorize_files, files_in_category, suggest_place, CategorizedFile, FileCategorySummary,
    InventoryFile, PlaceSuggestion,
};
use mac_storage_common::{Error, PRODUCT_NAME, PRODUCT_VERSION};
use mac_storage_remediate::{move_file, PlaceError, MOVE_CONFIRMATION_PHRASE};
use mac_storage_storage::{resolve_db_path, Database, StorageError, StoredFile};
use serde::Serialize;

use crate::report;

#[derive(Debug, Args)]
pub struct CategoryArgs {
    /// Stored scan id. Defaults to the newest scan.
    #[arg(long)]
    scan: Option<i64>,

    /// List files in one category: documents, images, media, archives, installers, developer, or other.
    #[arg(long)]
    category: Option<String>,

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
pub struct PlaceArgs {
    /// File to suggest a name and folder for. It does not have to be in the scan.
    #[arg(long, value_name = "PATH")]
    path: PathBuf,

    /// Stored scan used to see where similar files already live. Defaults to the newest scan.
    #[arg(long)]
    scan: Option<i64>,

    /// Move the file only when this is exactly `move file`.
    #[arg(long, value_name = "PHRASE")]
    confirm: Option<String>,

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
pub struct CategoriesReport {
    pub product: &'static str,
    pub version: &'static str,
    pub scan_id: i64,
    pub root: PathBuf,
    pub database: PathBuf,
    pub categories: Vec<FileCategorySummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<CategorizedFile>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlaceReport {
    pub product: &'static str,
    pub version: &'static str,
    pub scan_id: Option<i64>,
    pub database: PathBuf,
    pub suggestion: PlaceSuggestion,
    pub confirmation: &'static str,
    pub moved: bool,
}

pub fn categories_command(args: CategoryArgs) -> Result<(), Error> {
    crate::init_tracing(args.verbose, args.quiet, args.json);
    let report = run_categories(args.db.as_deref(), args.scan, args.category.as_deref())?;
    if args.json {
        report::write_value(&report, std::io::stdout())?;
    } else if !args.quiet {
        print!("{}", format_categories(&report));
    }
    Ok(())
}

pub fn place_command(args: PlaceArgs) -> Result<(), Error> {
    crate::init_tracing(args.verbose, args.quiet, args.json);
    let report = run_place(
        args.db.as_deref(),
        args.scan,
        &args.path,
        args.confirm.as_deref(),
    )?;
    if args.json {
        report::write_value(&report, std::io::stdout())?;
    } else if !args.quiet {
        print!("{}", format_place(&report));
    }
    Ok(())
}

pub(crate) fn run_categories(
    db: Option<&Path>,
    scan: Option<i64>,
    category: Option<&str>,
) -> Result<CategoriesReport, Error> {
    let (db_path, scan_id, root, files) = load_inventory(db, scan)?;
    let categories = categorize_files(&files);
    let files = match category.map(str::trim).filter(|value| !value.is_empty()) {
        None => None,
        Some(name) => Some(
            files_in_category(&files, name)
                .ok_or_else(|| Error::Usage(format!("unknown category {name}")))?,
        ),
    };
    Ok(CategoriesReport {
        product: PRODUCT_NAME,
        version: PRODUCT_VERSION,
        scan_id,
        root,
        database: db_path,
        categories,
        files,
    })
}

pub(crate) fn run_place(
    db: Option<&Path>,
    scan: Option<i64>,
    path: &Path,
    confirm: Option<&str>,
) -> Result<PlaceReport, Error> {
    let path = expand_user_path(path);
    if path.as_os_str().is_empty() {
        return Err(Error::Usage("path is required".into()));
    }
    let (database, db_path) = open_database(db)?;
    let (scan_id, files) = match database.resolve_scan_id(scan) {
        Ok(scan_id) => {
            let stored = database
                .list_regular_files(scan_id)
                .map_err(|err| Error::Storage(err.to_string()))?;
            (Some(scan_id), inventory_files(&stored))
        }
        Err(StorageError::NoScans) if scan.is_none() => (None, Vec::new()),
        Err(err) => return Err(Error::Storage(err.to_string())),
    };
    drop(database);
    let suggestion = suggest_place(&path, &files);
    let moved = match confirm.map(str::trim) {
        None => false,
        Some(phrase) => match move_file(&suggestion.source, &suggestion.suggested_path, phrase) {
            Ok(_) => true,
            Err(PlaceError::NotConfirmed) => {
                return Err(Error::Usage(PlaceError::NotConfirmed.to_string()));
            }
            Err(PlaceError::Refused { reason }) => return Err(Error::Usage(reason)),
        },
    };
    Ok(PlaceReport {
        product: PRODUCT_NAME,
        version: PRODUCT_VERSION,
        scan_id,
        database: db_path,
        suggestion,
        confirmation: MOVE_CONFIRMATION_PHRASE,
        moved,
    })
}

pub(crate) fn format_categories(report: &CategoriesReport) -> String {
    let mut out = format!(
        "{PRODUCT_NAME} {PRODUCT_VERSION}\nScan: {}\nRoot: {}\nCategories are a view of the stored scan. Nothing is moved.\n",
        report.scan_id,
        report.root.display()
    );
    for category in &report.categories {
        out.push_str(&format!(
            "{}: {} files, {} logical bytes, {} not stored on this Mac\n",
            category.name, category.file_count, category.logical_bytes, category.dataless_files
        ));
    }
    if let Some(files) = &report.files {
        out.push_str(&format!("\nFiles: {}\n", files.len()));
        for file in files {
            out.push_str(&format!(
                "  {}  {} bytes{}\n",
                file.path.display(),
                file.logical_size,
                if file.is_dataless {
                    "  (not stored on this Mac)"
                } else {
                    ""
                }
            ));
        }
    }
    out
}

pub(crate) fn format_place(report: &PlaceReport) -> String {
    let suggestion = &report.suggestion;
    format!(
        "{PRODUCT_NAME} {PRODUCT_VERSION}\n\
         File: {}\n\
         Category: {}\n\
         Suggested name: {}\n\
         Suggested folder: {}\n\
         Suggested path: {}\n\
         {}\n\
         Moved: {}\n\
         To move it, run the same command with --confirm \"{}\". Nothing is deleted.\n",
        suggestion.source.display(),
        suggestion.category,
        suggestion.suggested_name,
        suggestion.suggested_directory.display(),
        suggestion.suggested_path.display(),
        suggestion.reason,
        if report.moved { "yes" } else { "no" },
        report.confirmation
    )
}

fn load_inventory(
    db: Option<&Path>,
    scan: Option<i64>,
) -> Result<(PathBuf, i64, PathBuf, Vec<InventoryFile>), Error> {
    let (database, db_path) = open_database(db)?;
    let scan_id = database
        .resolve_scan_id(scan)
        .map_err(|err| Error::Storage(err.to_string()))?;
    let root = database
        .scan_root(scan_id)
        .map_err(|err| Error::Storage(err.to_string()))?;
    let stored = database
        .list_regular_files(scan_id)
        .map_err(|err| Error::Storage(err.to_string()))?;
    Ok((db_path, scan_id, root, inventory_files(&stored)))
}

fn inventory_files(stored: &[StoredFile]) -> Vec<InventoryFile> {
    stored
        .iter()
        .map(|file| InventoryFile {
            path: file.record.path.clone(),
            logical_size: file.record.logical_size,
            allocated_size: file.record.allocated_size,
            modified: file.record.modified,
            extension: file.record.extension.clone(),
            is_dataless: file.record.is_dataless,
        })
        .collect()
}

fn open_database(flag: Option<&Path>) -> Result<(Database, PathBuf), Error> {
    let path = resolve_db_path(flag).map_err(|err| Error::Storage(err.to_string()))?;
    let database = Database::open(&path).map_err(|err| Error::Storage(err.to_string()))?;
    Ok((database, path))
}

fn expand_user_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let home = std::env::var_os("HOME").filter(|value| !value.is_empty());
    if text == "~" {
        return home
            .map(PathBuf::from)
            .unwrap_or_else(|| path.to_path_buf());
    }
    if let Some(rest) = text.strip_prefix("~/") {
        if let Some(home) = home {
            return PathBuf::from(home).join(rest);
        }
    }
    path.to_path_buf()
}
