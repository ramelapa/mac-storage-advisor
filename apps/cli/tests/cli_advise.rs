use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
            "mac-storage-advise-{}-{}-{}",
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
fn analyze_recommendations_and_trends_use_a_stored_scan() {
    let dir = TempDir::new();
    let downloads = dir.path.join("Downloads");
    fs::create_dir_all(downloads.join("proj/.venv/lib")).unwrap();
    fs::write(downloads.join("same-a.txt"), b"same-bytes").unwrap();
    fs::write(downloads.join("same-b.txt"), b"same-bytes").unwrap();
    fs::write(downloads.join("Installer.dmg"), b"disk-image").unwrap();
    fs::write(downloads.join("~$notes.docx"), b"lock").unwrap();
    fs::write(downloads.join("proj/.venv/lib/a.py"), b"package").unwrap();
    let old = downloads.join("old-notes.txt");
    fs::write(&old, b"aged").unwrap();
    let aged = SystemTime::now() - Duration::from_secs(400 * 24 * 60 * 60);
    File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(aged)
        .unwrap();

    let root = downloads.to_str().unwrap();
    let scanned = run(&dir, &["scan", root, "--json", "--quiet"]);
    assert!(
        scanned.status.success(),
        "{}",
        String::from_utf8_lossy(&scanned.stderr)
    );
    let duplicates = run(&dir, &["duplicates", "--quiet"]);
    assert!(
        duplicates.status.success(),
        "{}",
        String::from_utf8_lossy(&duplicates.stderr)
    );

    let analyzed = run(&dir, &["analyze", "--json", "--older-than", "180d"]);
    assert!(
        analyzed.status.success(),
        "{}",
        String::from_utf8_lossy(&analyzed.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&analyzed.stdout).unwrap();
    assert_eq!(json["stale"]["file_count"], 1);
    assert_eq!(json["downloads"]["applies"], true);
    let installers = json["downloads"]["categories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|category| category["name"] == "installers")
        .unwrap();
    assert_eq!(installers["file_count"], 1);
    assert_eq!(json["developer"]["trees"].as_array().unwrap().len(), 1);
    assert_eq!(json["office_locks"]["file_count"], 1);
    let titles: Vec<&str> = json["recommendations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["title"].as_str().unwrap())
        .collect();
    assert!(titles.contains(&"Identical files"));
    assert!(titles.iter().any(|title| title.contains("Build directory")));
    assert!(titles.contains(&"Office lock files"));
    assert!(!analyzed
        .stdout
        .windows(b"same-bytes".len())
        .any(|w| w == b"same-bytes"));

    let suggested = run(&dir, &["recommendations", "--json"]);
    assert!(suggested.status.success());
    let suggested_json: serde_json::Value = serde_json::from_slice(&suggested.stdout).unwrap();
    assert!(suggested_json["recommendations"].as_array().unwrap().len() >= 3);
    assert!(suggested_json.get("stale").is_none());

    let trends = run(&dir, &["trends", "--json"]);
    assert!(trends.status.success());
    let trends_json: serde_json::Value = serde_json::from_slice(&trends.stdout).unwrap();
    assert_eq!(trends_json["series"].as_array().unwrap().len(), 1);
    assert!(trends_json["series"][0]["logical_delta"].is_null());

    let mut extra = File::create(downloads.join("another.txt")).unwrap();
    write!(extra, "more").unwrap();
    let again = run(&dir, &["scan", root, "--quiet"]);
    assert!(
        again.status.success(),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );
    let trends = run(&dir, &["trends", "--json"]);
    let trends_json: serde_json::Value = serde_json::from_slice(&trends.stdout).unwrap();
    let delta = trends_json["series"][0]["logical_delta"].as_i64().unwrap();
    assert!(delta > 0, "{delta}");
}
