//! Reviews a stored scan. Suggestions only; nothing is deleted.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use clap::Args;
use mac_storage_analyze::{
    advise, stale_duration, trends, AdvisorReport, DuplicateSet, InventoryFile, TrendPoint,
    TrendSeries,
};
use mac_storage_common::{PRODUCT_NAME, PRODUCT_VERSION};
use mac_storage_storage::Database;
use serde::Serialize;

use crate::{report, size, Error};

#[derive(Debug, Args)]
pub struct AnalyzeArgs {
    /// Stored scan id. Defaults to the newest scan.
    #[arg(long)]
    scan: Option<i64>,

    /// Modified-time age for the stale-file review.
    #[arg(long, value_name = "DAYS", default_value = "180d", value_parser = size::parse_days)]
    older_than: u64,

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
pub struct TrendsArgs {
    /// How many stored scans to include, newest first.
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

pub fn analyze_command(args: AnalyzeArgs) -> Result<(), Error> {
    review_command(args, false)
}

pub fn recommendations_command(args: AnalyzeArgs) -> Result<(), Error> {
    review_command(args, true)
}

pub fn trends_command(args: TrendsArgs) -> Result<(), Error> {
    crate::init_tracing(args.verbose, args.quiet, args.json);
    let report = run_trends(args.db.as_deref(), args.limit)?;
    if args.json {
        report::write_value(&report, std::io::stdout())?;
    } else if !args.quiet {
        print!("{}", format_trends(&report));
    }
    Ok(())
}

pub(crate) fn run_trends(db: Option<&Path>, limit: u64) -> Result<TrendsReport, Error> {
    if limit == 0 {
        return Err(Error::Usage("--limit must be at least 1".into()));
    }
    let (database, db_path) = open_database(db)?;
    let scans = database
        .list_scans(limit)
        .map_err(|err| Error::Storage(err.to_string()))?;
    let series = trends(
        scans
            .iter()
            .map(|scan| TrendPoint {
                scan_id: scan.id,
                root: scan.root.clone(),
                logical_bytes: scan.logical_bytes,
                allocated_bytes: scan.allocated_bytes,
                files_scanned: scan.files_scanned,
            })
            .collect(),
    );
    Ok(TrendsReport {
        product: PRODUCT_NAME,
        version: PRODUCT_VERSION,
        database: db_path,
        series,
    })
}

fn review_command(args: AnalyzeArgs, recommendations_only: bool) -> Result<(), Error> {
    crate::init_tracing(args.verbose, args.quiet, args.json);
    let report = run_review(args.db.as_deref(), args.scan, args.older_than)?;
    if args.json {
        if recommendations_only {
            report::write_value(&recommendations_view(&report), std::io::stdout())?;
        } else {
            report::write_value(&report, std::io::stdout())?;
        }
    } else if !args.quiet {
        if recommendations_only {
            print!("{}", format_recommendations(&report));
        } else {
            print!("{}", format_analyze(&report));
        }
    }
    Ok(())
}

pub(crate) fn run_review(
    db: Option<&Path>,
    scan: Option<i64>,
    older_than: u64,
) -> Result<ReviewReport, Error> {
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
    let groups = database
        .load_duplicate_groups(scan_id)
        .map_err(|err| Error::Storage(err.to_string()))?;
    let by_id: HashMap<i64, PathBuf> = stored
        .iter()
        .map(|file| (file.id, file.record.path.clone()))
        .collect();
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
    let duplicates = groups
        .iter()
        .map(|group| DuplicateSet {
            logical_size: group.logical_size,
            redundant_bytes: group.redundant_bytes,
            paths: group
                .members
                .iter()
                .filter_map(|member| by_id.get(&member.file_id).cloned())
                .collect(),
        })
        .collect::<Vec<_>>();
    let advised = advise(
        &root,
        &files,
        &duplicates,
        SystemTime::now(),
        stale_duration(older_than),
    );
    Ok(ReviewReport {
        product: PRODUCT_NAME,
        version: PRODUCT_VERSION,
        scan_id,
        root,
        database: db_path,
        older_than_days: older_than,
        duplicates_loaded: !groups.is_empty(),
        report: advised,
    })
}

fn open_database(flag: Option<&Path>) -> Result<(Database, PathBuf), Error> {
    let path = mac_storage_storage::resolve_db_path(flag)
        .map_err(|err| Error::Storage(err.to_string()))?;
    let database = Database::open(&path).map_err(|err| Error::Storage(err.to_string()))?;
    Ok((database, path))
}

#[derive(Debug, Serialize)]
pub(crate) struct ReviewReport {
    product: &'static str,
    version: &'static str,
    scan_id: i64,
    root: PathBuf,
    database: PathBuf,
    older_than_days: u64,
    duplicates_loaded: bool,
    #[serde(flatten)]
    report: AdvisorReport,
}

#[derive(Debug, Serialize)]
pub(crate) struct RecommendationsView<'a> {
    product: &'static str,
    version: &'static str,
    scan_id: i64,
    root: &'a Path,
    database: &'a Path,
    recommendations: &'a [mac_storage_analyze::Recommendation],
}

#[derive(Debug, Serialize)]
pub(crate) struct TrendsReport {
    product: &'static str,
    version: &'static str,
    database: PathBuf,
    series: Vec<TrendSeries>,
}

pub(crate) fn recommendations_view(report: &ReviewReport) -> RecommendationsView<'_> {
    RecommendationsView {
        product: report.product,
        version: report.version,
        scan_id: report.scan_id,
        root: &report.root,
        database: &report.database,
        recommendations: &report.report.recommendations,
    }
}

pub(crate) fn format_analyze(report: &ReviewReport) -> String {
    let body = &report.report;
    let mut out = format!(
        "{PRODUCT_NAME} {PRODUCT_VERSION}\nScan: {}\nRoot: {}\nSuggestions only. Logical bytes are not bytes the disk will free.\n",
        report.scan_id,
        report.root.display()
    );
    out.push_str(&format!(
        "\nStale files: {} older than {} days, {} logical bytes\n",
        body.stale.file_count, body.stale.older_than_days, body.stale.logical_bytes
    ));
    for file in &body.stale.files {
        out.push_str(&format!(
            "  {}  {}\n",
            file.logical_size,
            file.path.display()
        ));
    }
    if body.downloads.applies {
        out.push_str("\nDownloads review:\n");
        if body.downloads.categories.is_empty() {
            out.push_str("  No user files outside build directories.\n");
        }
        for category in &body.downloads.categories {
            out.push_str(&format!(
                "  {}: {} files, {} logical bytes\n",
                category.name, category.file_count, category.logical_bytes
            ));
        }
    } else {
        out.push_str("\nDownloads review: scan root is not a folder named Downloads.\n");
    }
    out.push_str(&format!(
        "\nDeveloper artifacts: {}\n",
        body.developer.trees.len()
    ));
    for tree in &body.developer.trees {
        out.push_str(&format!(
            "  {}  {} files, {} logical bytes  {}\n",
            tree.kind,
            tree.file_count,
            tree.logical_bytes,
            tree.path.display()
        ));
    }
    out.push_str(&format!(
        "\nOffice lock files: {}, {} logical bytes\n",
        body.office_locks.file_count, body.office_locks.logical_bytes
    ));
    out.push_str(&format!(
        "\nNot stored on this Mac: {}, {} logical bytes\n",
        body.dataless.file_count, body.dataless.logical_bytes
    ));
    for file in &body.dataless.files {
        out.push_str(&format!(
            "  {}  {}\n",
            file.logical_size,
            file.path.display()
        ));
    }
    out.push_str(&format!(
        "\nAllocated below logical: {}, gap {} bytes (not reclaimable)\n",
        body.sparse_or_shared.file_count, body.sparse_or_shared.gap_bytes
    ));
    out.push_str("\nRecommendations:\n");
    out.push_str(&recommendation_lines(report));
    out
}

pub(crate) fn format_recommendations(report: &ReviewReport) -> String {
    let mut out = format!(
        "{PRODUCT_NAME} {PRODUCT_VERSION}\nRecommendations: {}\nNothing is deleted.\n",
        report.report.recommendations.len()
    );
    out.push_str(&recommendation_lines(report));
    out
}

fn recommendation_lines(report: &ReviewReport) -> String {
    let mut out = String::new();
    if !report.duplicates_loaded {
        out.push_str("No duplicate groups are stored for this scan yet.\n");
    }
    for item in &report.report.recommendations {
        out.push_str(&format!(
            "\n[{}] {} — {} logical bytes\n  {}\n",
            risk_label(item.risk),
            item.title,
            item.logical_bytes,
            item.detail
        ));
        for path in &item.paths {
            out.push_str(&format!("  {}\n", path.display()));
        }
    }
    out
}

pub(crate) fn format_trends(report: &TrendsReport) -> String {
    let mut out = format!(
        "{PRODUCT_NAME} {PRODUCT_VERSION}\nFolders: {}\nAllocated bytes are not reclaimable space.\n",
        report.series.len()
    );
    for series in &report.series {
        out.push_str(&format!("\n{}\n", series.root.display()));
        for point in series.points.iter().rev() {
            out.push_str(&format!(
                "  #{}  logical {}  allocated {}  files {}\n",
                point.scan_id, point.logical_bytes, point.allocated_bytes, point.files_scanned
            ));
        }
        match series.logical_delta {
            Some(delta) => out.push_str(&format!(
                "  Change since the previous scan of this folder: logical {delta}, allocated {}\n",
                series.allocated_delta.unwrap_or(0)
            )),
            None => out.push_str("  Only one scan of this folder so far.\n"),
        }
    }
    out
}

fn risk_label(risk: mac_storage_common::RiskLevel) -> &'static str {
    match risk {
        mac_storage_common::RiskLevel::High => "high",
        mac_storage_common::RiskLevel::Medium => "medium",
        mac_storage_common::RiskLevel::Low => "low",
        mac_storage_common::RiskLevel::Info => "info",
    }
}
