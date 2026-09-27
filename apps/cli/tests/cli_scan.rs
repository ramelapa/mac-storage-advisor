use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use mac_storage_storage::Database;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_mac-storage")
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mac-storage-cli-{}-{}-{}",
            std::process::id(),
            n,
            nanos
        ));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn db(&self) -> PathBuf {
        self.path.join("advisor.sqlite")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn scan(dir: &TempDir, extra: &[&str]) -> std::process::Output {
    Command::new(bin())
        .args(["scan", dir.path.to_str().unwrap()])
        .args(extra)
        .env("MAC_STORAGE_DB", dir.db())
        .env_remove("RUST_LOG")
        .output()
        .unwrap()
}

#[test]
fn scan_json_reports_counts_and_round_trips_rows() {
    let dir = TempDir::new();
    fs::write(dir.path.join("a.txt"), b"hello").unwrap();
    fs::write(dir.path.join("empty.dat"), b"").unwrap();
    fs::create_dir(dir.path.join("sub")).unwrap();
    fs::write(dir.path.join("sub").join("b.txt"), b"xyz").unwrap();

    let output = scan(&dir, &["--json", "--quiet"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["product"], "Mac Storage Advisor");
    assert_eq!(json["directories_scanned"], 2);
    assert_eq!(json["files_scanned"], 3);
    assert_eq!(json["logical_bytes"], 8);
    assert_eq!(json["errors"], 0);
    assert_eq!(json["concurrency"], 1);
    assert!(json["elapsed_ms"].as_u64().is_some());
    assert!(json["error_details"].as_array().unwrap().is_empty());
    assert!(json.get("file_contents").is_none());

    let scan_id = json["scan_id"].as_i64().unwrap();
    let db = Database::open(&dir.db()).unwrap();
    let loaded = db.load_scan(scan_id).unwrap();
    assert_eq!(loaded.scan.statistics.files_scanned, 3);
    assert_eq!(loaded.scan.statistics.directories_scanned, 2);
    assert_eq!(loaded.files.len(), 3);
    assert_eq!(loaded.directories.len(), 2);
    let notes = loaded
        .files
        .iter()
        .find(|file| file.filename == "a.txt")
        .unwrap();
    assert_eq!(notes.logical_size, 5);
    assert_eq!(notes.extension.as_deref(), Some("txt"));
    assert_eq!(loaded.scan.product_name, "Mac Storage Advisor");
}

#[test]
fn json_and_database_do_not_contain_file_contents() {
    let dir = TempDir::new();
    let marker = b"SENTINEL_DO_NOT_PERSIST_9f3a2c";
    fs::write(dir.path.join("note.txt"), marker).unwrap();
    let output = scan(&dir, &["--json"]);
    assert!(output.status.success());
    assert!(!output.stdout.windows(marker.len()).any(|w| w == marker));
    assert!(!output.stderr.windows(marker.len()).any(|w| w == marker));
    let db_bytes = fs::read(dir.db()).unwrap();
    assert!(!db_bytes.windows(marker.len()).any(|w| w == marker));
}

#[test]
fn exclude_flag_skips_a_directory() {
    let dir = TempDir::new();
    fs::write(dir.path.join("keep.txt"), b"keep").unwrap();
    fs::create_dir(dir.path.join("skip")).unwrap();
    fs::write(dir.path.join("skip").join("hidden.txt"), b"hide").unwrap();
    let output = scan(&dir, &["--json", "--exclude", "skip"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["files_scanned"], 1);
    assert_eq!(json["logical_bytes"], 4);
    assert!(json["skipped"].as_u64().unwrap() >= 1);
}

#[test]
fn threads_flag_is_recorded_but_concurrency_stays_one() {
    let dir = TempDir::new();
    fs::write(dir.path.join("a.txt"), b"a").unwrap();
    let output = scan(&dir, &["--json", "--threads", "4"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["threads_requested"], 4);
    assert_eq!(json["concurrency"], 1);
}

#[test]
fn min_size_keeps_counts_and_filters_rows() {
    let dir = TempDir::new();
    fs::write(dir.path.join("big.txt"), b"0123456789").unwrap();
    fs::write(dir.path.join("small.txt"), b"ab").unwrap();
    let output = scan(&dir, &["--json", "--min-size", "5"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["files_scanned"], 2);
    assert_eq!(json["logical_bytes"], 12);
    assert_eq!(json["files_below_min_size"], 1);
    let db = Database::open(&dir.db()).unwrap();
    let loaded = db.load_scan(json["scan_id"].as_i64().unwrap()).unwrap();
    assert_eq!(loaded.files.len(), 1);
    assert_eq!(loaded.files[0].filename, "big.txt");
}

#[test]
fn quiet_suppresses_the_human_summary() {
    let dir = TempDir::new();
    fs::write(dir.path.join("a.txt"), b"a").unwrap();
    let output = scan(&dir, &["--quiet"]);
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn missing_path_is_a_fatal_error() {
    let dir = TempDir::new();
    let missing = dir.path.join("does-not-exist");
    let output = Command::new(bin())
        .args(["scan", missing.to_str().unwrap(), "--json"])
        .env("MAC_STORAGE_DB", dir.db())
        .env_remove("RUST_LOG")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("does not exist") || stderr.contains("error:"));
}

#[test]
fn help_lists_scan_and_planned_commands() {
    let output = Command::new(bin()).arg("--help").output().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("scan"));
    assert!(text.contains("duplicates"));
    assert!(text.contains("recommendations"));
    assert!(text.contains("Mac Storage Advisor"));
}
