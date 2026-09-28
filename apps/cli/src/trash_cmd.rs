//! Move inventoried paths to the OS Trash after an explicit confirmation phrase.

use std::path::PathBuf;
use std::time::SystemTime;

use clap::Args;
use mac_storage_common::{PRODUCT_NAME, PRODUCT_VERSION};
use mac_storage_remediate::{
    commit, inspect_os, plan, trash_os, Inventoried, TrashOutcome, TrashPlan, CONFIRMATION_PHRASE,
};
use mac_storage_storage::Database;
use serde::Serialize;

use crate::{report, Error};

#[derive(Debug, Args)]
pub struct TrashArgs {
    /// Stored scan id. Defaults to the newest scan.
    #[arg(long)]
    scan: Option<i64>,

    /// Path recorded by that scan. Repeat for each path. Relative paths join the scan root.
    #[arg(long = "path", value_name = "PATH")]
    path: Vec<PathBuf>,

    /// Must be exactly `move to trash`. Without it, nothing is moved.
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

pub fn trash_command(args: TrashArgs) -> Result<(), Error> {
    crate::init_tracing(args.verbose, args.quiet, args.json);
    let report = run_trash(
        args.db.as_deref(),
        args.scan,
        &args.path,
        args.confirm.as_deref(),
    )?;
    if args.json {
        report::write_value(&report, std::io::stdout())?;
    } else if !args.quiet {
        print!("{}", format_trash(&report));
    }
    Ok(())
}

pub(crate) fn run_trash(
    db: Option<&std::path::Path>,
    scan: Option<i64>,
    paths: &[PathBuf],
    confirm: Option<&str>,
) -> Result<TrashReport, Error> {
    let db_path =
        mac_storage_storage::resolve_db_path(db).map_err(|err| Error::Storage(err.to_string()))?;
    let mut database = Database::open(&db_path).map_err(|err| Error::Storage(err.to_string()))?;
    let scan_id = database
        .resolve_scan_id(scan)
        .map_err(|err| Error::Storage(err.to_string()))?;
    let root = database
        .scan_root(scan_id)
        .map_err(|err| Error::Storage(err.to_string()))?;
    let inventory = database
        .list_inventory(scan_id)
        .map_err(|err| Error::Storage(err.to_string()))?
        .into_iter()
        .map(|entry| Inventoried {
            path: entry.path,
            is_dataless: entry.is_dataless,
            is_symlink: entry.is_symlink,
        })
        .collect::<Vec<_>>();
    let planned = plan(&root, &inventory, paths);
    let confirmation = confirm.unwrap_or("");
    if confirm.is_none() {
        return Ok(preview_report(scan_id, &db_path, &planned));
    }
    match commit(&planned, confirmation, inspect_os, trash_os) {
        Ok(outcome) => {
            if !outcome.moved.is_empty() {
                database
                    .record_trash_events(scan_id, &outcome.moved, SystemTime::now())
                    .map_err(|err| Error::Storage(err.to_string()))?;
            }
            Ok(applied_report(scan_id, &db_path, &planned, &outcome))
        }
        Err(err) => Err(Error::Usage(err.to_string())),
    }
}

fn preview_report(scan_id: i64, database: &std::path::Path, planned: &TrashPlan) -> TrashReport {
    TrashReport {
        product: PRODUCT_NAME,
        version: PRODUCT_VERSION,
        scan_id,
        root: planned.root.clone(),
        database: database.to_path_buf(),
        confirmation_phrase: CONFIRMATION_PHRASE,
        moved: Vec::new(),
        failed: None,
        accepted: planned.accepted.clone(),
        refused: planned
            .refused
            .iter()
            .map(|item| RefusedPath {
                path: item.path.clone(),
                reason: item.reason.clone(),
            })
            .collect(),
        applied: false,
    }
}

fn applied_report(
    scan_id: i64,
    database: &std::path::Path,
    planned: &TrashPlan,
    outcome: &TrashOutcome,
) -> TrashReport {
    TrashReport {
        product: PRODUCT_NAME,
        version: PRODUCT_VERSION,
        scan_id,
        root: planned.root.clone(),
        database: database.to_path_buf(),
        confirmation_phrase: CONFIRMATION_PHRASE,
        moved: outcome.moved.clone(),
        failed: outcome.failed.as_ref().map(|failure| RefusedPath {
            path: failure.path.clone(),
            reason: failure.message.clone(),
        }),
        accepted: planned.accepted.clone(),
        refused: Vec::new(),
        applied: true,
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct TrashReport {
    pub(crate) product: &'static str,
    pub(crate) version: &'static str,
    pub(crate) scan_id: i64,
    pub(crate) root: PathBuf,
    pub(crate) database: PathBuf,
    pub(crate) confirmation_phrase: &'static str,
    pub(crate) applied: bool,
    pub(crate) accepted: Vec<PathBuf>,
    pub(crate) refused: Vec<RefusedPath>,
    pub(crate) moved: Vec<PathBuf>,
    pub(crate) failed: Option<RefusedPath>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RefusedPath {
    pub(crate) path: PathBuf,
    pub(crate) reason: String,
}

pub(crate) fn format_trash(report: &TrashReport) -> String {
    let mut out = format!(
        "{PRODUCT_NAME} {PRODUCT_VERSION}\nScan: {}\nRoot: {}\n",
        report.scan_id,
        report.root.display()
    );
    if report.applied {
        out.push_str(&format!("Moved to Trash: {}\n", report.moved.len()));
        for path in &report.moved {
            out.push_str(&format!("  {}\n", path.display()));
        }
        if let Some(failed) = &report.failed {
            out.push_str(&format!(
                "Stopped: {}: {}\n",
                failed.path.display(),
                failed.reason
            ));
        }
    } else {
        out.push_str("Nothing was moved.\n");
        out.push_str(&format!(
            "Would move {} path(s). Refused {}.\n",
            report.accepted.len(),
            report.refused.len()
        ));
        for path in &report.accepted {
            out.push_str(&format!("  {}\n", path.display()));
        }
        for item in &report.refused {
            out.push_str(&format!(
                "  refused {}: {}\n",
                item.path.display(),
                item.reason
            ));
        }
        out.push_str(&format!(
            "To move the accepted paths, repeat the command with --confirm \"{CONFIRMATION_PHRASE}\".\n"
        ));
    }
    out
}
