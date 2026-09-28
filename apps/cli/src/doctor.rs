//! Check that the local database is this product's file and still readable.

use std::path::{Path, PathBuf};

use clap::Args;
use mac_storage_common::{PRODUCT_NAME, PRODUCT_VERSION};
use mac_storage_storage::{Database, SCHEMA_VERSION};
use serde::Serialize;

use crate::{report, Error};

#[derive(Debug, Args)]
pub struct DoctorArgs {
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

pub fn doctor_command(args: DoctorArgs) -> Result<(), Error> {
    crate::init_tracing(args.verbose, args.quiet, args.json);
    let report = run_doctor(args.db.as_deref())?;
    if args.json {
        report::write_value(&report, std::io::stdout())?;
    } else if !args.quiet {
        print!("{}", format_doctor(&report));
    }
    if report.healthy {
        Ok(())
    } else {
        Err(Error::Usage(
            "database check failed; see the report above".into(),
        ))
    }
}

pub(crate) fn run_doctor(db: Option<&Path>) -> Result<DoctorReport, Error> {
    let db_path =
        mac_storage_storage::resolve_db_path(db).map_err(|err| Error::Storage(err.to_string()))?;
    let database = match Database::open(&db_path) {
        Ok(database) => database,
        Err(err) => {
            return Ok(failed_open(&db_path, &err.to_string()));
        }
    };
    let product_name = database
        .setting("product_name")
        .map_err(|err| Error::Storage(err.to_string()))?
        .unwrap_or_default();
    let schema_version = database
        .setting("schema_version")
        .map_err(|err| Error::Storage(err.to_string()))?
        .unwrap_or_default();
    let integrity = database
        .integrity_report()
        .map_err(|err| Error::Storage(err.to_string()))?;
    let scan_count = database
        .scan_count()
        .map_err(|err| Error::Storage(err.to_string()))?;
    let newest = database
        .list_scans(1)
        .map_err(|err| Error::Storage(err.to_string()))?
        .into_iter()
        .next();
    let trash_events = database
        .list_trash_events(20)
        .map_err(|err| Error::Storage(err.to_string()))?;
    let trash_event_count = database
        .trash_event_count()
        .map_err(|err| Error::Storage(err.to_string()))?;

    let mut checks = Vec::new();
    checks.push(check(
        "product",
        if product_name == PRODUCT_NAME {
            CheckStatus::Pass
        } else {
            CheckStatus::Fail
        },
        format!("product name is {product_name}"),
    ));
    checks.push(check(
        "schema",
        if schema_version == SCHEMA_VERSION {
            CheckStatus::Pass
        } else {
            CheckStatus::Fail
        },
        format!("schema version is {schema_version}"),
    ));
    checks.push(check(
        "integrity",
        if integrity == "ok" {
            CheckStatus::Pass
        } else {
            CheckStatus::Fail
        },
        format!("integrity check is {integrity}"),
    ));

    let newest_scan = newest.map(|scan| DoctorScan {
        id: scan.id,
        root: scan.root,
        status: scan.status.as_str().to_owned(),
        files_scanned: scan.files_scanned,
        logical_bytes: scan.logical_bytes,
        error_count: scan.error_count,
    });
    if let Some(scan) = &newest_scan {
        let present = std::fs::symlink_metadata(&scan.root).is_ok();
        checks.push(check(
            "newest_root",
            CheckStatus::Note,
            if present {
                format!(
                    "{} scans stored. Newest is {} at {}, and that folder is still on disk.",
                    scan_count,
                    scan.id,
                    scan.root.display()
                )
            } else {
                format!(
                    "{} scans stored. Newest is {} at {}, which is not on disk now. The stored scan is unchanged.",
                    scan_count,
                    scan.id,
                    scan.root.display()
                )
            },
        ));
    } else {
        checks.push(check(
            "newest_root",
            CheckStatus::Note,
            "no scans are stored yet".into(),
        ));
    }
    checks.push(check(
        "placeholders",
        CheckStatus::Note,
        if cfg!(target_os = "macos") {
            "iCloud placeholder detection uses SF_DATALESS on this Mac".into()
        } else {
            "iCloud placeholder detection is not available on this OS, so scans record that flag as false".into()
        },
    ));
    checks.push(check(
        "trash_log",
        CheckStatus::Note,
        format!("{trash_event_count} path(s) recorded as moved to Trash"),
    ));

    let healthy = checks.iter().all(|item| item.status != CheckStatus::Fail);
    Ok(DoctorReport {
        product: PRODUCT_NAME,
        version: PRODUCT_VERSION,
        database: db_path,
        opened: true,
        healthy,
        schema_version,
        scan_count,
        newest_scan,
        trash_event_count,
        recent_trash: trash_events
            .into_iter()
            .map(|event| DoctorTrash {
                scan_id: event.scan_id,
                path: event.path,
            })
            .collect(),
        checks,
    })
}

fn failed_open(database: &Path, message: &str) -> DoctorReport {
    DoctorReport {
        product: PRODUCT_NAME,
        version: PRODUCT_VERSION,
        database: database.to_path_buf(),
        opened: false,
        healthy: false,
        schema_version: String::new(),
        scan_count: 0,
        newest_scan: None,
        trash_event_count: 0,
        recent_trash: Vec::new(),
        checks: vec![check(
            "database",
            CheckStatus::Fail,
            format!("could not open the database: {message}"),
        )],
    }
}

fn check(name: &'static str, status: CheckStatus, detail: String) -> DoctorCheck {
    DoctorCheck {
        name,
        status,
        detail,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CheckStatus {
    Pass,
    Fail,
    Note,
}

#[derive(Debug, Serialize)]
pub(crate) struct DoctorReport {
    pub(crate) product: &'static str,
    pub(crate) version: &'static str,
    pub(crate) database: PathBuf,
    pub(crate) opened: bool,
    pub(crate) healthy: bool,
    pub(crate) schema_version: String,
    pub(crate) scan_count: u64,
    pub(crate) newest_scan: Option<DoctorScan>,
    pub(crate) trash_event_count: u64,
    pub(crate) recent_trash: Vec<DoctorTrash>,
    pub(crate) checks: Vec<DoctorCheck>,
}

#[derive(Debug, Serialize)]
pub(crate) struct DoctorScan {
    pub(crate) id: i64,
    pub(crate) root: PathBuf,
    pub(crate) status: String,
    pub(crate) files_scanned: u64,
    pub(crate) logical_bytes: u64,
    pub(crate) error_count: u64,
}

#[derive(Debug, Serialize)]
pub(crate) struct DoctorTrash {
    pub(crate) scan_id: i64,
    pub(crate) path: PathBuf,
}

#[derive(Debug, Serialize)]
pub(crate) struct DoctorCheck {
    pub(crate) name: &'static str,
    pub(crate) status: CheckStatus,
    pub(crate) detail: String,
}

pub(crate) fn format_doctor(report: &DoctorReport) -> String {
    let mut out = format!(
        "{PRODUCT_NAME} {PRODUCT_VERSION}\nDatabase: {}\nStatus: {}\n",
        report.database.display(),
        if report.healthy {
            "healthy"
        } else {
            "check failed"
        }
    );
    for item in &report.checks {
        let label = match item.status {
            CheckStatus::Pass => "pass",
            CheckStatus::Fail => "fail",
            CheckStatus::Note => "note",
        };
        out.push_str(&format!("{label}  {}\n", item.detail));
    }
    for event in &report.recent_trash {
        out.push_str(&format!(
            "  scan {}  {}\n",
            event.scan_id,
            event.path.display()
        ));
    }
    out
}
