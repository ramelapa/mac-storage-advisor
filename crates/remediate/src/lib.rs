//! Move inventoried paths to the operating-system Trash.
//!
//! Nothing is moved unless the caller passes [`CONFIRMATION_PHRASE`].
//! Permanent delete is not implemented. iCloud placeholders are refused
//! because trashing them can remove the copy stored in iCloud.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use mac_storage_scanner::{is_protected_path, normalize_lexical};

/// The only confirmation this crate accepts for Trash. A boolean flag is not enough.
pub const CONFIRMATION_PHRASE: &str = "move to trash";

/// The only confirmation this crate accepts for moving a file to a suggested folder.
pub const MOVE_CONFIRMATION_PHRASE: &str = "move file";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inventoried {
    pub path: PathBuf,
    pub is_dataless: bool,
    pub is_symlink: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashPlan {
    pub root: PathBuf,
    pub accepted: Vec<PathBuf>,
    pub refused: Vec<Refusal>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashOutcome {
    pub moved: Vec<PathBuf>,
    pub failed: Option<MoveFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveFailure {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemediateError {
    /// The confirmation text did not match. No path was moved.
    NotConfirmed,
    /// Every requested path was refused, or none were named.
    NothingToMove { refused: Vec<Refusal> },
    /// At least one path was refused, so none were moved.
    Refused { refused: Vec<Refusal> },
}

impl std::fmt::Display for RemediateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConfirmed => write!(
                f,
                "nothing was moved; confirmation must be exactly \"{CONFIRMATION_PHRASE}\""
            ),
            Self::NothingToMove { refused } => {
                if refused.is_empty() {
                    write!(f, "name at least one path; nothing was moved")
                } else {
                    write!(f, "nothing was moved; {}", refused[0].reason)
                }
            }
            Self::Refused { refused } => write!(
                f,
                "nothing was moved; {}",
                refused
                    .first()
                    .map(|item| item.reason.as_str())
                    .unwrap_or("a path was refused")
            ),
        }
    }
}

impl std::error::Error for RemediateError {}

/// Decide which requested paths may move. This function does not touch the filesystem.
pub fn plan(root: &Path, inventory: &[Inventoried], requested: &[PathBuf]) -> TrashPlan {
    let root = normalize_lexical(root);
    let mut accepted = Vec::new();
    let mut refused = Vec::new();
    if requested.is_empty() {
        refused.push(Refusal {
            path: root.clone(),
            reason: "name at least one path that this scan recorded".into(),
        });
    }
    for raw in requested {
        let path = resolve_under(&root, raw);
        if let Some(reason) = refuse_reason(&root, inventory, &path) {
            refused.push(Refusal { path, reason });
            continue;
        }
        if accepted.iter().any(|existing: &PathBuf| existing == &path) {
            continue;
        }
        accepted.push(path);
    }
    TrashPlan {
        root,
        accepted,
        refused,
    }
}

/// Move every accepted path, and only after the confirmation phrase matches.
///
/// `inspect` re-reads metadata immediately before the move. A dataless file
/// or a missing path stops the move before the first path is touched.
/// `trash_one` must move that path to Trash without following a symlink.
pub fn commit<I, T>(
    planned: &TrashPlan,
    confirmation: &str,
    inspect: I,
    trash_one: T,
) -> Result<TrashOutcome, RemediateError>
where
    I: Fn(&Path) -> Result<Inspection, String>,
    T: Fn(&Path) -> Result<(), String>,
{
    if confirmation != CONFIRMATION_PHRASE {
        return Err(RemediateError::NotConfirmed);
    }
    if planned.accepted.is_empty() {
        return Err(RemediateError::NothingToMove {
            refused: planned.refused.clone(),
        });
    }
    if !planned.refused.is_empty() {
        return Err(RemediateError::Refused {
            refused: planned.refused.clone(),
        });
    }
    for path in &planned.accepted {
        match inspect(path) {
            Ok(inspection) if !inspection.exists => {
                return Err(RemediateError::Refused {
                    refused: vec![Refusal {
                        path: path.clone(),
                        reason: "path is not on disk anymore; nothing was moved".into(),
                    }],
                });
            }
            Ok(inspection) if inspection.is_dataless => {
                return Err(RemediateError::Refused {
                    refused: vec![Refusal {
                        path: path.clone(),
                        reason: "refusing to move an iCloud placeholder; nothing was moved".into(),
                    }],
                });
            }
            Ok(_) => {}
            Err(message) => {
                return Err(RemediateError::Refused {
                    refused: vec![Refusal {
                        path: path.clone(),
                        reason: format!("could not read metadata, so nothing was moved: {message}"),
                    }],
                });
            }
        }
    }
    let mut moved = Vec::new();
    for path in &planned.accepted {
        if let Err(message) = trash_one(path) {
            return Ok(TrashOutcome {
                moved,
                failed: Some(MoveFailure {
                    path: path.clone(),
                    message,
                }),
            });
        }
        moved.push(path.clone());
    }
    Ok(TrashOutcome {
        moved,
        failed: None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inspection {
    pub exists: bool,
    pub is_dataless: bool,
}

/// Move one path with the operating-system Trash. Symlinks are not followed.
pub fn trash_os(path: &Path) -> Result<(), String> {
    trash::delete(path).map_err(|err| err.to_string())
}

/// A file that was renamed into the suggested folder. The old path is gone; nothing was deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceOutcome {
    pub source: PathBuf,
    pub destination: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaceError {
    /// The confirmation text did not match. The file was not moved.
    NotConfirmed,
    /// The move was refused. The source file is still in place.
    Refused { reason: String },
}

impl std::fmt::Display for PlaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConfirmed => write!(
                f,
                "nothing was moved; confirmation must be exactly \"{MOVE_CONFIRMATION_PHRASE}\""
            ),
            Self::Refused { reason } => write!(f, "{reason}"),
        }
    }
}

impl std::error::Error for PlaceError {}

/// Rename `source` to `destination` only when `confirmation` is [`MOVE_CONFIRMATION_PHRASE`].
///
/// This does not overwrite an existing path, follow a symlink, move a directory,
/// or touch a protected macOS path or an iCloud placeholder. A failed rename
/// leaves the source where it was. There is no permanent delete.
pub fn move_file(
    source: &Path,
    destination: &Path,
    confirmation: &str,
) -> Result<PlaceOutcome, PlaceError> {
    if confirmation != MOVE_CONFIRMATION_PHRASE {
        return Err(PlaceError::NotConfirmed);
    }
    let source = normalize_lexical(source);
    let destination = normalize_lexical(destination);
    if source == destination {
        return Err(PlaceError::Refused {
            reason: "the file is already in the suggested place; nothing was moved".into(),
        });
    }
    if is_protected_path(&source) || is_protected_path(&destination) {
        return Err(PlaceError::Refused {
            reason: "refusing to move a protected macOS path; nothing was moved".into(),
        });
    }
    match std::fs::symlink_metadata(&source) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(PlaceError::Refused {
                reason: "refusing to move a symlink; nothing was moved".into(),
            });
        }
        Ok(meta) if !meta.is_file() => {
            return Err(PlaceError::Refused {
                reason: "only a regular file can be moved; nothing was moved".into(),
            });
        }
        Ok(meta) if dataless_metadata(&meta) => {
            return Err(PlaceError::Refused {
                reason: "refusing to move an iCloud placeholder; nothing was moved".into(),
            });
        }
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(PlaceError::Refused {
                reason: "file is not on disk; nothing was moved".into(),
            });
        }
        Err(err) => {
            return Err(PlaceError::Refused {
                reason: format!("could not read the file, so nothing was moved: {err}"),
            });
        }
    }
    if std::fs::symlink_metadata(&destination).is_ok() {
        return Err(PlaceError::Refused {
            reason: "a file is already at the suggested path; nothing was moved".into(),
        });
    }
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    match std::fs::symlink_metadata(parent) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(PlaceError::Refused {
                reason: "the suggested folder is not a directory; nothing was moved".into(),
            });
        }
        Err(_) => {
            return Err(PlaceError::Refused {
                reason: "the suggested folder does not exist; nothing was moved".into(),
            });
        }
    }
    std::fs::rename(&source, &destination).map_err(|err| PlaceError::Refused {
        reason: format!("the file was not moved: {err}"),
    })?;
    Ok(PlaceOutcome {
        source,
        destination,
    })
}

/// `lstat` the path. On macOS, `SF_DATALESS` is reported. Other platforms are not dataless.
pub fn inspect_os(path: &Path) -> Result<Inspection, String> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => Ok(Inspection {
            exists: true,
            is_dataless: dataless_metadata(&meta),
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Inspection {
            exists: false,
            is_dataless: false,
        }),
        Err(err) => Err(err.to_string()),
    }
}

fn refuse_reason(root: &Path, inventory: &[Inventoried], path: &Path) -> Option<String> {
    if is_protected_path(path) && !explicit_project_under_prefix(root) {
        return Some("refusing to move a protected macOS path".into());
    }
    if path == root {
        return Some("refusing to move the scan folder itself".into());
    }
    if !path.starts_with(root) {
        return Some("path is outside the scan folder".into());
    }
    let Some(entry) = inventory
        .iter()
        .find(|entry| normalize_lexical(&entry.path) == path)
    else {
        return Some("path was not recorded by this scan".into());
    };
    if entry.is_dataless {
        return Some(
            "refusing to move an iCloud placeholder; that can remove the copy in iCloud".into(),
        );
    }
    None
}

fn explicit_project_under_prefix(root: &Path) -> bool {
    is_protected_path(root) && !mac_storage_scanner::is_exact_protected_root(root)
}

fn resolve_under(root: &Path, raw: &Path) -> PathBuf {
    let joined = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        root.join(raw)
    };
    normalize_lexical(&joined)
}

#[cfg(target_os = "macos")]
fn dataless_metadata(meta: &std::fs::Metadata) -> bool {
    use std::os::macos::fs::MetadataExt;
    mac_storage_scanner::is_dataless_flags(meta.st_flags())
}

#[cfg(not(target_os = "macos"))]
fn dataless_metadata(_meta: &std::fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn entry(path: &str, dataless: bool) -> Inventoried {
        Inventoried {
            path: PathBuf::from(path),
            is_dataless: dataless,
            is_symlink: false,
        }
    }

    #[test]
    fn preview_rules_refuse_the_root_protected_paths_and_placeholders() {
        let root = Path::new("/tmp/project");
        let inventory = vec![
            entry("/tmp/project/keep.txt", false),
            entry("/tmp/project/cloud.pdf", true),
            entry("/usr/bin/ls", false),
        ];
        let planned = plan(
            root,
            &inventory,
            &[
                PathBuf::from("/tmp/project"),
                PathBuf::from("/tmp/project/cloud.pdf"),
                PathBuf::from("/usr/bin/ls"),
                PathBuf::from("/tmp/project/missing.txt"),
                PathBuf::from("/tmp/other/file.txt"),
                PathBuf::from("/tmp/project/../project/keep.txt"),
            ],
        );
        assert_eq!(
            planned.accepted,
            vec![PathBuf::from("/tmp/project/keep.txt")]
        );
        let reasons: Vec<_> = planned
            .refused
            .iter()
            .map(|item| item.reason.as_str())
            .collect();
        assert!(reasons.iter().any(|reason| reason.contains("scan folder")));
        assert!(reasons.iter().any(|reason| reason.contains("iCloud")));
        assert!(reasons.iter().any(|reason| reason.contains("protected")));
        assert!(reasons.iter().any(|reason| reason.contains("not recorded")));
        assert!(reasons.iter().any(|reason| reason.contains("outside")));
    }

    #[test]
    fn a_project_rooted_under_usr_local_can_name_its_own_files() {
        let root = Path::new("/usr/local/myproject");
        let inventory = vec![entry("/usr/local/myproject/notes.txt", false)];
        let planned = plan(
            root,
            &inventory,
            &[PathBuf::from("/usr/local/myproject/notes.txt")],
        );
        assert!(planned.refused.is_empty());
        assert_eq!(planned.accepted.len(), 1);
    }

    #[test]
    fn the_wrong_phrase_does_not_call_trash() {
        let calls = AtomicUsize::new(0);
        let planned = TrashPlan {
            root: PathBuf::from("/tmp/project"),
            accepted: vec![PathBuf::from("/tmp/project/a.txt")],
            refused: Vec::new(),
        };
        let result = commit(
            &planned,
            "yes",
            |_| {
                Ok(Inspection {
                    exists: true,
                    is_dataless: false,
                })
            },
            |_| {
                calls.fetch_add(1, Ordering::Relaxed);
                Ok(())
            },
        );
        assert!(matches!(result, Err(RemediateError::NotConfirmed)));
        assert_eq!(calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_refused_path_blocks_the_whole_move() {
        let calls = AtomicUsize::new(0);
        let planned = TrashPlan {
            root: PathBuf::from("/tmp/project"),
            accepted: vec![PathBuf::from("/tmp/project/a.txt")],
            refused: vec![Refusal {
                path: PathBuf::from("/tmp/project/cloud.pdf"),
                reason: "placeholder".into(),
            }],
        };
        let result = commit(
            &planned,
            CONFIRMATION_PHRASE,
            |_| {
                Ok(Inspection {
                    exists: true,
                    is_dataless: false,
                })
            },
            |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        );
        assert!(matches!(result, Err(RemediateError::Refused { .. })));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn confirmed_move_uses_trash_and_leaves_a_symlink_target() {
        let root = std::env::temp_dir().join(format!(
            "mac-storage-trash-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sub")).unwrap();
        let target = root.join("sub").join("target.txt");
        let link = root.join("link.txt");
        let extra = root.join("stay.txt");
        fs::write(&target, b"kept").unwrap();
        fs::write(&extra, b"stay").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let inventory = vec![
            Inventoried {
                path: link.clone(),
                is_dataless: false,
                is_symlink: true,
            },
            Inventoried {
                path: extra.clone(),
                is_dataless: false,
                is_symlink: false,
            },
        ];
        let planned = plan(&root, &inventory, std::slice::from_ref(&link));
        assert!(planned.refused.is_empty(), "{planned:?}");
        let outcome = commit(&planned, CONFIRMATION_PHRASE, inspect_os, trash_os).unwrap();
        assert_eq!(outcome.moved, vec![normalize_lexical(&link)]);
        assert!(outcome.failed.is_none());
        assert!(!link.exists(), "symlink path should have moved to Trash");
        assert_eq!(fs::read(&target).unwrap(), b"kept");
        assert_eq!(fs::read(&extra).unwrap(), b"stay");
        let _ = fs::remove_dir_all(&root);
    }

    fn place_dir() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "mac-storage-place-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("Documents")).unwrap();
        root
    }

    #[test]
    fn a_file_moves_only_when_the_phrase_matches() {
        let root = place_dir();
        let source = root.join("Invoice (1).pdf");
        let destination = root.join("Documents").join("Invoice.pdf");
        fs::write(&source, b"tax").unwrap();

        let refused = move_file(&source, &destination, "yes").unwrap_err();
        assert!(matches!(refused, PlaceError::NotConfirmed));
        assert_eq!(fs::read(&source).unwrap(), b"tax");
        assert!(!destination.exists());

        let moved = move_file(&source, &destination, MOVE_CONFIRMATION_PHRASE).unwrap();
        assert_eq!(moved.destination, normalize_lexical(&destination));
        assert!(!source.exists());
        assert_eq!(fs::read(&destination).unwrap(), b"tax");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_existing_destination_a_symlink_and_a_protected_path_stay_put() {
        let root = place_dir();
        let source = root.join("notes.txt");
        let destination = root.join("Documents").join("notes.txt");
        fs::write(&source, b"local").unwrap();
        fs::write(&destination, b"already").unwrap();
        let blocked = move_file(&source, &destination, MOVE_CONFIRMATION_PHRASE).unwrap_err();
        assert!(matches!(blocked, PlaceError::Refused { .. }));
        assert_eq!(fs::read(&source).unwrap(), b"local");
        assert_eq!(fs::read(&destination).unwrap(), b"already");

        let link = root.join("link.txt");
        std::os::unix::fs::symlink(&source, &link).unwrap();
        let link_dest = root.join("Documents").join("link.txt");
        let refused_link = move_file(&link, &link_dest, MOVE_CONFIRMATION_PHRASE).unwrap_err();
        assert!(matches!(refused_link, PlaceError::Refused { .. }));
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert!(!link_dest.exists());

        let protected = move_file(
            Path::new("/usr/bin/true"),
            &root.join("true"),
            MOVE_CONFIRMATION_PHRASE,
        )
        .unwrap_err();
        assert!(matches!(protected, PlaceError::Refused { .. }));
        assert!(root.join("true").symlink_metadata().is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
