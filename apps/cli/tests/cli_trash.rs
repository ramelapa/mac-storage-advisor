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
            "mac-storage-trash-cli-{}-{}-{}",
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
fn trash_waits_for_the_phrase_and_does_not_follow_a_symlink() {
    let dir = TempDir::new();
    let folder = dir.path.join("folder");
    fs::create_dir_all(folder.join("sub")).unwrap();
    let target = folder.join("sub").join("target.txt");
    let link = folder.join("link.txt");
    let stay = folder.join("stay.txt");
    fs::write(&target, b"kept").unwrap();
    fs::write(&stay, b"stay").unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let scanned = run(&dir, &["scan", folder.to_str().unwrap(), "--quiet"]);
    assert!(
        scanned.status.success(),
        "{}",
        String::from_utf8_lossy(&scanned.stderr)
    );

    let preview = run(&dir, &["trash", "--path", link.to_str().unwrap(), "--json"]);
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    let preview_json: serde_json::Value = serde_json::from_slice(&preview.stdout).unwrap();
    assert_eq!(preview_json["applied"], false);
    assert!(preview_json["moved"].as_array().unwrap().is_empty());
    assert!(link.exists() || target.exists());
    assert!(fs::symlink_metadata(&link).is_ok());
    assert_eq!(fs::read(&target).unwrap(), b"kept");

    let wrong = run(
        &dir,
        &[
            "trash",
            "--path",
            link.to_str().unwrap(),
            "--confirm",
            "yes",
        ],
    );
    assert!(!wrong.status.success());
    assert!(fs::symlink_metadata(&link).is_ok());

    let moved = run(
        &dir,
        &[
            "trash",
            "--path",
            link.to_str().unwrap(),
            "--confirm",
            "move to trash",
            "--json",
        ],
    );
    assert!(
        moved.status.success(),
        "{}",
        String::from_utf8_lossy(&moved.stderr)
    );
    let moved_json: serde_json::Value = serde_json::from_slice(&moved.stdout).unwrap();
    assert_eq!(moved_json["applied"], true);
    assert_eq!(moved_json["moved"].as_array().unwrap().len(), 1);
    assert!(fs::symlink_metadata(&link).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"kept");
    assert_eq!(fs::read(&stay).unwrap(), b"stay");

    let outside = dir.path.join("outside.txt");
    fs::write(&outside, b"nope").unwrap();
    let refused = run(
        &dir,
        &[
            "trash",
            "--path",
            outside.to_str().unwrap(),
            "--confirm",
            "move to trash",
        ],
    );
    assert!(!refused.status.success());
    assert_eq!(fs::read(&outside).unwrap(), b"nope");
}
