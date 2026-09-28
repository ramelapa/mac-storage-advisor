use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

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
            "mac-storage-doctor-{}-{}-{}",
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
fn doctor_accepts_a_fresh_database_and_rejects_a_foreign_file() {
    let dir = TempDir::new();
    let folder = dir.path.join("folder");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("note.txt"), b"hello").unwrap();

    let fresh = run(&dir, &["doctor", "--json"]);
    assert!(
        fresh.status.success(),
        "{}",
        String::from_utf8_lossy(&fresh.stderr)
    );
    let fresh_json: serde_json::Value = serde_json::from_slice(&fresh.stdout).unwrap();
    assert_eq!(fresh_json["healthy"], true);
    assert_eq!(fresh_json["scan_count"], 0);
    assert_eq!(fresh_json["schema_version"], "3");

    let scanned = run(&dir, &["scan", folder.to_str().unwrap(), "--quiet"]);
    assert!(scanned.status.success());
    let after = run(&dir, &["doctor", "--json"]);
    assert!(after.status.success());
    let after_json: serde_json::Value = serde_json::from_slice(&after.stdout).unwrap();
    assert_eq!(after_json["healthy"], true);
    assert_eq!(after_json["scan_count"], 1);
    assert_eq!(after_json["newest_scan"]["files_scanned"], 1);

    let foreign = dir.path.join("foreign.sqlite");
    fs::write(&foreign, b"this is not the advisor database").unwrap();
    let rejected = Command::new(bin())
        .args(["doctor", "--json", "--db"])
        .arg(&foreign)
        .env_remove("MAC_STORAGE_DB")
        .env_remove("RUST_LOG")
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    let rejected_json: serde_json::Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert_eq!(rejected_json["healthy"], false);
    assert_eq!(rejected_json["opened"], false);
    assert_eq!(
        fs::read(&foreign).unwrap(),
        b"this is not the advisor database"
    );
}
