use std::io::Write;
use std::path::Path;

use mac_storage_common::{Scan, ScanReport, ScanSnapshot, SCAN_CONCURRENCY};

use crate::Error;

pub fn build_report(scan: &Scan, snapshot: &ScanSnapshot, database: &Path) -> ScanReport {
    let stats = &snapshot.statistics;
    ScanReport {
        product: scan.product_name.clone(),
        version: scan.product_version.clone(),
        scan_id: scan.id,
        root: snapshot.root.clone(),
        directories_scanned: stats.directories_scanned,
        files_scanned: stats.files_scanned,
        logical_bytes: stats.logical_bytes,
        allocated_bytes: stats.allocated_bytes,
        allocated_bytes_complete: stats.allocated_bytes_complete,
        symlinks: stats.symlinks,
        other_entries: stats.other_entries,
        errors: stats.errors,
        skipped: stats.skipped,
        files_below_min_size: stats.files_below_min_size,
        elapsed_ms: stats.elapsed_ms,
        threads_requested: snapshot.threads_requested,
        concurrency: SCAN_CONCURRENCY,
        min_logical_size: snapshot.min_logical_size,
        database: database.to_path_buf(),
        error_details: snapshot.errors.clone(),
    }
}

pub fn write_json(report: &ScanReport, mut out: impl Write) -> Result<(), Error> {
    serde_json::to_writer_pretty(&mut out, report)
        .map_err(|err| Error::Scan(format!("failed to write JSON report: {err}")))?;
    writeln!(out).map_err(Error::from)?;
    Ok(())
}

pub fn format_human(report: &ScanReport) -> String {
    let mut text = format!(
        "{product} {version}\n\
         Root: {root}\n\
         Directories scanned: {dirs}\n\
         Files scanned: {files}\n\
         Logical bytes: {logical} ({logical_pretty})\n\
         {allocated}\n\
         Errors: {errors}\n\
         Skipped: {skipped}\n\
         Elapsed: {elapsed} ms\n\
         Database: {database}\n",
        product = report.product,
        version = report.version,
        root = report.root.display(),
        dirs = report.directories_scanned,
        files = report.files_scanned,
        logical = report.logical_bytes,
        logical_pretty = format_bytes(report.logical_bytes),
        allocated = format_allocated(report),
        errors = report.errors,
        skipped = report.skipped,
        elapsed = report.elapsed_ms,
        database = report.database.display(),
    );
    if report.threads_requested != report.concurrency {
        text.push_str(&format!(
            "Note: --threads {} was recorded; this version scans on {} thread.\n",
            report.threads_requested, report.concurrency
        ));
    }
    for err in &report.error_details {
        match &err.path {
            Some(path) => text.push_str(&format!("error: {}: {}\n", path.display(), err.message)),
            None => text.push_str(&format!("error: {}\n", err.message)),
        }
    }
    text
}

fn format_allocated(report: &ScanReport) -> String {
    if report.allocated_bytes_complete {
        format!(
            "Allocated bytes: {} ({})",
            report.allocated_bytes,
            format_bytes(report.allocated_bytes)
        )
    } else if report.allocated_bytes == 0 {
        "Allocated bytes: unavailable".to_owned()
    } else {
        format!(
            "Allocated bytes: {} ({}, partial)",
            report.allocated_bytes,
            format_bytes(report.allocated_bytes)
        )
    }
}

pub(crate) fn write_value(value: &impl serde::Serialize, mut out: impl Write) -> Result<(), Error> {
    serde_json::to_writer_pretty(&mut out, value)
        .map_err(|err| Error::Scan(format!("failed to write JSON report: {err}")))?;
    writeln!(out).map_err(Error::from)?;
    Ok(())
}

pub(crate) fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mac_storage_common::{ScanErrorRecord, PRODUCT_NAME};
    use std::path::PathBuf;

    fn sample() -> ScanReport {
        ScanReport {
            product: PRODUCT_NAME.to_owned(),
            version: "0.1.0".into(),
            scan_id: 3,
            root: PathBuf::from("/tmp/fixture"),
            directories_scanned: 2,
            files_scanned: 4,
            logical_bytes: 4096,
            allocated_bytes: 8192,
            allocated_bytes_complete: true,
            symlinks: 1,
            other_entries: 0,
            errors: 1,
            skipped: 1,
            files_below_min_size: 0,
            elapsed_ms: 12,
            threads_requested: 4,
            concurrency: 1,
            min_logical_size: 0,
            database: PathBuf::from("/tmp/test.sqlite"),
            error_details: vec![ScanErrorRecord {
                path: Some(PathBuf::from("/tmp/fixture/locked")),
                message: "permission denied".into(),
            }],
        }
    }

    #[test]
    fn human_report_includes_required_fields() {
        let text = format_human(&sample());
        assert!(text.contains("Directories scanned: 2"));
        assert!(text.contains("Files scanned: 4"));
        assert!(text.contains("Logical bytes: 4096"));
        assert!(text.contains("Errors: 1"));
        assert!(text.contains("Elapsed: 12 ms"));
        assert!(text.contains("permission denied"));
        assert!(text.contains("this version scans on 1 thread"));
        assert!(text.contains(PRODUCT_NAME));
    }

    #[test]
    fn json_report_is_stable_and_has_no_extra_content_field() {
        let json = serde_json::to_value(sample()).unwrap();
        assert_eq!(json["directories_scanned"], 2);
        assert_eq!(json["files_scanned"], 4);
        assert_eq!(json["logical_bytes"], 4096);
        assert_eq!(json["errors"], 1);
        assert_eq!(json["elapsed_ms"], 12);
        assert_eq!(json["concurrency"], 1);
        assert_eq!(json["error_details"][0]["message"], "permission denied");
        assert_eq!(json["error_details"][0]["path"], "/tmp/fixture/locked");
        assert!(json.get("contents").is_none());
        assert!(json.get("file_contents").is_none());
    }
}
