//! Filesystem metadata collected without reading file contents.
//!
//! Logical size is `Metadata::len`. Allocated size, when the Unix `stat`
//! fields exist, is `st_blocks * 512`. Those numbers are stored separately.
//! Allocated size is not logical size, and neither number is reclaimable
//! space (APFS clones and sparse files make that a different question).
//!
//! Birth time is `Metadata::created`, which succeeds on macOS and typically
//! fails on Linux. A missing birth time stays `None`; modified time is never
//! copied into it.

use std::fs::Metadata;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use mac_storage_common::{DirectoryRecord, FileKind, FileRecord, ScanErrorRecord};

pub struct CollectedMeta {
    pub logical_size: u64,
    pub allocated_size: Option<u64>,
    pub created: Option<SystemTime>,
    pub modified: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    pub inode: Option<u64>,
    pub device_id: Option<u64>,
    pub permissions: Option<u32>,
    pub kind: FileKind,
    pub is_symlink: bool,
    pub is_broken_symlink: bool,
    pub link_target: Option<PathBuf>,
}

pub fn classify(is_symlink: bool, is_dir: bool, is_file: bool) -> FileKind {
    if is_symlink {
        FileKind::Symlink
    } else if is_dir {
        FileKind::Directory
    } else if is_file {
        FileKind::File
    } else {
        FileKind::Other
    }
}

pub fn collect_metadata(path: &Path) -> Result<CollectedMeta, ScanErrorRecord> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) => {
            return Err(ScanErrorRecord {
                path: Some(path.to_path_buf()),
                message: format!("failed to read metadata: {err}"),
            });
        }
    };

    let file_type = meta.file_type();
    let is_symlink = file_type.is_symlink();
    let kind = classify(is_symlink, file_type.is_dir(), file_type.is_file());
    let (is_broken_symlink, link_target) = if is_symlink {
        let link_target = std::fs::read_link(path).ok();
        let broken = matches!(
            std::fs::metadata(path),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound
        );
        (broken, link_target)
    } else {
        (false, None)
    };

    Ok(CollectedMeta {
        logical_size: meta.len(),
        allocated_size: allocated_size(&meta),
        created: meta.created().ok(),
        modified: meta.modified().ok(),
        accessed: meta.accessed().ok(),
        inode: inode_of(&meta),
        device_id: device_of(&meta),
        permissions: mode_of(&meta),
        kind,
        is_symlink,
        is_broken_symlink,
        link_target,
    })
}

pub fn file_record(path: &Path, meta: &CollectedMeta) -> FileRecord {
    FileRecord {
        path: path.to_path_buf(),
        filename: filename_of(path),
        extension: extension_of(path),
        logical_size: meta.logical_size,
        allocated_size: meta.allocated_size,
        created: meta.created,
        modified: meta.modified,
        accessed: meta.accessed,
        inode: meta.inode,
        device_id: meta.device_id,
        permissions: meta.permissions,
        kind: meta.kind,
        is_symlink: meta.is_symlink,
        is_broken_symlink: meta.is_broken_symlink,
        link_target: meta.link_target.clone(),
    }
}

pub fn directory_record(path: &Path, meta: &CollectedMeta) -> DirectoryRecord {
    DirectoryRecord {
        path: path.to_path_buf(),
        filename: filename_of(path),
        logical_size: meta.logical_size,
        allocated_size: meta.allocated_size,
        created: meta.created,
        modified: meta.modified,
        accessed: meta.accessed,
        inode: meta.inode,
        device_id: meta.device_id,
        permissions: meta.permissions,
    }
}

fn filename_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn extension_of(path: &Path) -> Option<String> {
    path.extension()
        .map(|ext| ext.to_string_lossy().into_owned())
}

#[cfg(unix)]
fn allocated_size(meta: &Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(meta.blocks().saturating_mul(512))
}

#[cfg(not(unix))]
fn allocated_size(_meta: &Metadata) -> Option<u64> {
    None
}

#[cfg(unix)]
fn inode_of(meta: &Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(meta.ino())
}

#[cfg(not(unix))]
fn inode_of(_meta: &Metadata) -> Option<u64> {
    None
}

#[cfg(unix)]
fn device_of(meta: &Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(meta.dev())
}

#[cfg(not(unix))]
fn device_of(_meta: &Metadata) -> Option<u64> {
    None
}

#[cfg(unix)]
fn mode_of(meta: &Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(meta.permissions().mode())
}

#[cfg(not(unix))]
fn mode_of(_meta: &Metadata) -> Option<u32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_prefers_symlink_over_directory() {
        assert_eq!(classify(true, true, false), FileKind::Symlink);
        assert_eq!(classify(false, true, false), FileKind::Directory);
        assert_eq!(classify(false, false, true), FileKind::File);
        assert_eq!(classify(false, false, false), FileKind::Other);
    }
}
