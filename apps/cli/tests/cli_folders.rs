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
            "mac-storage-folders-{}-{}-{}",
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
fn folders_roll_nested_files_into_the_child_directory() {
    let dir = TempDir::new();
    let folder = dir.path.join("folder");
    fs::create_dir_all(folder.join("sub").join("nested")).unwrap();
    fs::create_dir_all(folder.join("other")).unwrap();
    fs::write(folder.join("notes.txt"), b"hello").unwrap();
    fs::write(folder.join("sub").join("b.txt"), vec![1u8; 30]).unwrap();
    fs::write(
        folder.join("sub").join("nested").join("c.txt"),
        vec![2u8; 7],
    )
    .unwrap();
    fs::write(folder.join("other").join("d.txt"), b"xy").unwrap();

    let scanned = run(&dir, &["scan", folder.to_str().unwrap(), "--quiet"]);
    assert!(
        scanned.status.success(),
        "{}",
        String::from_utf8_lossy(&scanned.stderr)
    );

    let listed = run(&dir, &["folders", "--json", "--depth", "1"]);
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(json["depth"], 1);
    assert_eq!(json["logical_bytes"], 5 + 30 + 7 + 2);
    assert_eq!(json["dataless_bytes"], 0);
    let folders = json["folders"].as_array().unwrap();
    let sub = folders
        .iter()
        .find(|row| row["path"].as_str().unwrap().ends_with("/sub"))
        .unwrap();
    assert_eq!(sub["logical_bytes"], 37);
    assert_eq!(sub["local_files"], 2);
    let root_row = folders
        .iter()
        .find(|row| row["path"].as_str().unwrap() == folder.to_str().unwrap())
        .unwrap();
    assert_eq!(root_row["logical_bytes"], 5);
    assert_eq!(root_row["local_files"], 1);
    assert!(
        folders[0]["logical_bytes"].as_u64().unwrap()
            >= folders[1]["logical_bytes"].as_u64().unwrap()
    );

    let deeper = run(&dir, &["folders", "--json", "--depth", "2"]);
    assert!(deeper.status.success());
    let deep: serde_json::Value = serde_json::from_slice(&deeper.stdout).unwrap();
    let nested = deep["folders"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["path"].as_str().unwrap().ends_with("/nested"))
        .unwrap();
    assert_eq!(nested["logical_bytes"], 7);
    let sub_only = deep["folders"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["path"].as_str().unwrap().ends_with("/sub"))
        .unwrap();
    assert_eq!(sub_only["logical_bytes"], 30);

    let rejected = run(&dir, &["folders", "--depth", "0"]);
    assert!(!rejected.status.success());
}

#[test]
fn storage_bar_percents_sum_to_one_hundred_and_drill_does_not_move_files() {
    let dir = TempDir::new();
    let folder = dir.path.join("folder");
    fs::create_dir_all(folder.join("sub").join("nested")).unwrap();
    fs::create_dir_all(folder.join("other")).unwrap();
    fs::write(folder.join("notes.txt"), b"hello").unwrap();
    fs::write(folder.join("sub").join("b.txt"), vec![1u8; 30]).unwrap();
    fs::write(
        folder.join("sub").join("nested").join("c.txt"),
        vec![2u8; 7],
    )
    .unwrap();
    fs::write(folder.join("other").join("d.txt"), b"xy").unwrap();
    let scanned = run(&dir, &["scan", folder.to_str().unwrap(), "--quiet"]);
    assert!(
        scanned.status.success(),
        "{}",
        String::from_utf8_lossy(&scanned.stderr)
    );

    let listed = run(&dir, &["storage", "--json"]);
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    let segments = json["segments"].as_array().unwrap();
    let hundredths: u64 = segments
        .iter()
        .map(|item| item["percent_hundredths"].as_u64().unwrap())
        .sum();
    assert_eq!(hundredths, 10_000);
    assert!(json.get("drill").is_none() || json["drill"].is_null());
    let sub = segments.iter().find(|item| item["label"] == "sub").unwrap();
    assert!(sub["percent_hundredths"].as_u64().unwrap() > 5_000);
    assert!(!listed
        .stdout
        .windows(b"hello".len())
        .any(|window| window == b"hello"));

    let drilled = run(
        &dir,
        &[
            "storage",
            "--folder",
            "sub",
            "--category",
            "documents",
            "--json",
        ],
    );
    assert!(
        drilled.status.success(),
        "{}",
        String::from_utf8_lossy(&drilled.stderr)
    );
    let drill: serde_json::Value = serde_json::from_slice(&drilled.stdout).unwrap();
    let files = drill["drill"]["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert!(files[0]["path"].as_str().unwrap().ends_with("b.txt"));
    assert!(files
        .iter()
        .all(|file| !file["path"].as_str().unwrap().ends_with("notes.txt")));
    assert!(drill["drill"]["hint"]
        .as_str()
        .unwrap()
        .contains("Nothing is moved"));
    assert_eq!(
        fs::read(folder.join("sub").join("b.txt")).unwrap().len(),
        30
    );

    let missing = run(&dir, &["storage", "--folder", "missing"]);
    assert!(!missing.status.success());
    let bare = run(&dir, &["storage", "--category", "documents"]);
    assert!(!bare.status.success());
}
