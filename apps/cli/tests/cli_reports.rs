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
            "mac-storage-report-{}-{}-{}",
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

fn run(dir: &TempDir, args: &[&str]) -> std::process::Output {
    Command::new(bin())
        .args(args)
        .env("MAC_STORAGE_DB", dir.db())
        .env_remove("RUST_LOG")
        .output()
        .unwrap()
}

#[test]
fn duplicates_large_files_and_history_round_trip() {
    let dir = TempDir::new();
    let marker = b"SENTINEL_DO_NOT_PRINT_9f3a2c";
    fs::write(dir.path.join("same-a.txt"), marker).unwrap();
    fs::write(dir.path.join("same-b.txt"), marker).unwrap();
    fs::write(dir.path.join("other.txt"), b"different-bytes").unwrap();
    fs::write(dir.path.join("empty.dat"), b"").unwrap();
    fs::write(dir.path.join("bigger.bin"), vec![1u8; 40]).unwrap();

    let scanned = run(
        &dir,
        &["scan", dir.path.to_str().unwrap(), "--json", "--quiet"],
    );
    assert!(
        scanned.status.success(),
        "{}",
        String::from_utf8_lossy(&scanned.stderr)
    );
    let scan_json: serde_json::Value = serde_json::from_slice(&scanned.stdout).unwrap();
    let logical_before = scan_json["logical_bytes"].as_u64().unwrap();
    let scan_id = scan_json["scan_id"].as_i64().unwrap();

    let duplicates = run(&dir, &["duplicates", "--json", "--verify"]);
    assert!(
        duplicates.status.success(),
        "{}",
        String::from_utf8_lossy(&duplicates.stderr)
    );
    assert!(!duplicates.stdout.windows(marker.len()).any(|w| w == marker));
    let dup_json: serde_json::Value = serde_json::from_slice(&duplicates.stdout).unwrap();
    assert_eq!(dup_json["scan_id"], scan_id);
    assert_eq!(dup_json["algorithm"], "blake3");
    assert_eq!(dup_json["verified"], true);
    assert_eq!(dup_json["concurrency"], 1);
    let groups = dup_json["duplicate_groups"].as_array().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["logical_size"], marker.len());
    assert_eq!(groups[0]["redundant_bytes"], marker.len());
    assert_eq!(groups[0]["members"].as_array().unwrap().len(), 2);
    assert!(dup_json["hash_errors"].as_array().unwrap().is_empty());

    let db = Database::open(&dir.db()).unwrap();
    let stored = db.load_duplicate_groups(scan_id).unwrap();
    assert_eq!(stored.len(), 1);
    let loaded = db.load_scan(scan_id).unwrap();
    assert_eq!(loaded.scan.statistics.logical_bytes, logical_before);
    drop(db);

    let large = run(&dir, &["large-files", "--json", "--limit", "2"]);
    assert!(
        large.status.success(),
        "{}",
        String::from_utf8_lossy(&large.stderr)
    );
    let large_json: serde_json::Value = serde_json::from_slice(&large.stdout).unwrap();
    let files = large_json["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert!(
        files[0]["logical_size"].as_u64().unwrap() >= files[1]["logical_size"].as_u64().unwrap()
    );
    assert_eq!(files[0]["logical_size"], 40);

    let history = run(&dir, &["history", "--json"]);
    assert!(history.status.success());
    let history_json: serde_json::Value = serde_json::from_slice(&history.stdout).unwrap();
    assert_eq!(history_json["scans"].as_array().unwrap().len(), 1);
    assert_eq!(history_json["scans"][0]["id"], scan_id);
    assert_eq!(history_json["scans"][0]["logical_bytes"], logical_before);
}

#[test]
fn reports_fail_when_no_scan_exists() {
    let dir = TempDir::new();
    let output = run(&dir, &["duplicates", "--json"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no scans"));
    assert!(output.stdout.is_empty());
}

#[test]
fn help_lists_the_new_commands_and_still_names_planned_ones() {
    let output = Command::new(bin()).arg("--help").output().unwrap();
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("duplicates"));
    assert!(text.contains("large-files"));
    assert!(text.contains("history"));
    assert!(text.contains("recommendations"));
    assert!(text.contains("mac-storage ui"));
}
