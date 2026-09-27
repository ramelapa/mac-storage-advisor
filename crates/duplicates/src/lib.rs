//! Duplicate grouping for regular files already recorded by a scan.
//!
//! The scan crate does not read file contents. This pass does, locally, and
//! only to hash. It never deletes a path and never treats a size match as a
//! duplicate.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::Serialize;

/// First-window size for the sample hash. Files this size or smaller are
/// fully covered by that single read.
pub const SAMPLE_BYTES: u64 = 64 * 1024;

/// Hash algorithm stored with every content hash row.
pub const HASH_ALGORITHM: &str = "blake3";

/// A persisted regular file the duplicate pass is allowed to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub id: i64,
    pub path: PathBuf,
    pub logical_size: u64,
    pub inode: Option<u64>,
    pub device_id: Option<u64>,
}

/// Result of one duplicate pass. Size groups are not duplicates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateAnalysis {
    pub groups: Vec<DuplicateGroup>,
    pub hard_link_sets: Vec<HardLinkSet>,
    pub hashes: Vec<ContentHash>,
    pub errors: Vec<HashError>,
}

/// Regular files whose full BLAKE3 hash matches. `redundant_bytes` counts
/// extra content copies, not hard-link paths, and is not a promise of free space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DuplicateGroup {
    pub logical_size: u64,
    pub full_hash: String,
    pub redundant_bytes: u64,
    pub members: Vec<DuplicateMember>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DuplicateMember {
    pub file_id: i64,
    pub path: PathBuf,
    pub hard_link_leader: bool,
}

/// Paths that share one inode. Extra names do not add redundant bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HardLinkSet {
    pub logical_size: u64,
    pub device_id: u64,
    pub inode: u64,
    pub paths: Vec<HardLinkPath>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HardLinkPath {
    pub file_id: i64,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentHash {
    pub file_id: i64,
    pub algorithm: &'static str,
    pub sample_hash: String,
    pub full_hash: Option<String>,
    pub hashed_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HashError {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Identity {
    Inode(u64, u64),
    Solo(i64),
}

struct Cluster {
    leader: usize,
    members: Vec<usize>,
}

struct Sample {
    hash: String,
    bytes: u64,
    entire_file: bool,
}

struct HashedLeader {
    cluster_index: usize,
    sample: Sample,
    full_hash: Option<String>,
    hashed_bytes: u64,
}

/// Group `files` from a single scan.
///
/// Zero-byte files are ignored. Hard links collapse to one hash read. A
/// sample hash that is unique in its size group does not get a full hash.
/// `verify` re-reads survivors and drops a group member whose bytes disagree
/// with the hash.
pub fn find_duplicates(files: &[Candidate], verify: bool) -> DuplicateAnalysis {
    let hard_link_sets = hard_link_sets(files);
    let mut by_size: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    for (index, file) in files.iter().enumerate() {
        if file.logical_size == 0 {
            continue;
        }
        by_size.entry(file.logical_size).or_default().push(index);
    }

    let mut groups = Vec::new();
    let mut hashes = Vec::new();
    let mut errors = Vec::new();

    for (logical_size, indexes) in by_size {
        let clusters = clusters_of(files, &indexes);
        if clusters.len() < 2 {
            continue;
        }
        let mut leaders = Vec::new();
        for (cluster_index, cluster) in clusters.iter().enumerate() {
            let leader = &files[cluster.leader];
            match sample_hash(&leader.path) {
                Ok(sample) => leaders.push(HashedLeader {
                    cluster_index,
                    hashed_bytes: sample.bytes,
                    full_hash: None,
                    sample,
                }),
                Err(err) => errors.push(HashError {
                    path: leader.path.clone(),
                    message: format!("could not read file for hashing: {err}"),
                }),
            }
        }

        let mut by_sample: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (index, leader) in leaders.iter().enumerate() {
            by_sample
                .entry(leader.sample.hash.clone())
                .or_default()
                .push(index);
        }

        let mut survivors = Vec::new();
        for indexes in by_sample.values() {
            if indexes.len() == 1 {
                let leader = &leaders[indexes[0]];
                hashes.push(ContentHash {
                    file_id: files[clusters[leader.cluster_index].leader].id,
                    algorithm: HASH_ALGORITHM,
                    sample_hash: leader.sample.hash.clone(),
                    full_hash: None,
                    hashed_bytes: leader.sample.bytes,
                });
            } else {
                survivors.extend(indexes.iter().copied());
            }
        }

        let mut by_full: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for index in survivors {
            let path = &files[clusters[leaders[index].cluster_index].leader].path;
            let full = if leaders[index].sample.entire_file {
                Ok((
                    leaders[index].sample.hash.clone(),
                    leaders[index].sample.bytes,
                ))
            } else {
                full_hash(path)
            };
            match full {
                Ok((hash, nbytes)) => {
                    leaders[index].full_hash = Some(hash.clone());
                    leaders[index].hashed_bytes = nbytes;
                    hashes.push(ContentHash {
                        file_id: files[clusters[leaders[index].cluster_index].leader].id,
                        algorithm: HASH_ALGORITHM,
                        sample_hash: leaders[index].sample.hash.clone(),
                        full_hash: Some(hash.clone()),
                        hashed_bytes: nbytes,
                    });
                    by_full.entry(hash).or_default().push(index);
                }
                Err(err) => errors.push(HashError {
                    path: path.clone(),
                    message: format!("could not read file for hashing: {err}"),
                }),
            }
        }

        for (hash, mut leader_indexes) in by_full {
            if verify {
                leader_indexes =
                    confirm_bytes(files, &clusters, &leaders, &leader_indexes, &mut errors);
            }
            if leader_indexes.len() < 2 {
                continue;
            }
            groups.push(group_from(
                files,
                &clusters,
                &leaders,
                &leader_indexes,
                logical_size,
                hash,
            ));
        }
    }

    groups.sort_by(|left, right| {
        right
            .logical_size
            .cmp(&left.logical_size)
            .then_with(|| left.full_hash.cmp(&right.full_hash))
    });
    hashes.sort_by_key(|hash| hash.file_id);
    errors.sort_by(|left, right| left.path.cmp(&right.path));

    DuplicateAnalysis {
        groups,
        hard_link_sets,
        hashes,
        errors,
    }
}

fn confirm_bytes(
    files: &[Candidate],
    clusters: &[Cluster],
    leaders: &[HashedLeader],
    leader_indexes: &[usize],
    errors: &mut Vec<HashError>,
) -> Vec<usize> {
    let Some(first) = leader_indexes.first().copied() else {
        return Vec::new();
    };
    let baseline = &files[clusters[leaders[first].cluster_index].leader].path;
    let mut kept = vec![first];
    for index in leader_indexes.iter().copied().skip(1) {
        let path = &files[clusters[leaders[index].cluster_index].leader].path;
        match files_identical(baseline, path) {
            Ok(true) => kept.push(index),
            Ok(false) => errors.push(HashError {
                path: path.clone(),
                message: "byte comparison disagreed with the BLAKE3 hash; the file was left out of the duplicate group".into(),
            }),
            Err(err) => errors.push(HashError {
                path: path.clone(),
                message: format!("could not read file for byte comparison: {err}"),
            }),
        }
    }
    kept
}

fn group_from(
    files: &[Candidate],
    clusters: &[Cluster],
    leaders: &[HashedLeader],
    leader_indexes: &[usize],
    logical_size: u64,
    full_hash: String,
) -> DuplicateGroup {
    let mut members = Vec::new();
    for index in leader_indexes {
        let cluster = &clusters[leaders[*index].cluster_index];
        for member_index in &cluster.members {
            let file = &files[*member_index];
            members.push(DuplicateMember {
                file_id: file.id,
                path: file.path.clone(),
                hard_link_leader: *member_index == cluster.leader,
            });
        }
    }
    members.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.file_id.cmp(&right.file_id))
    });
    let copies = u64::try_from(leader_indexes.len()).unwrap_or(u64::MAX);
    DuplicateGroup {
        logical_size,
        full_hash,
        redundant_bytes: logical_size.saturating_mul(copies.saturating_sub(1)),
        members,
    }
}

fn hard_link_sets(files: &[Candidate]) -> Vec<HardLinkSet> {
    let mut grouped: BTreeMap<(u64, u64, u64), Vec<&Candidate>> = BTreeMap::new();
    for file in files {
        if file.logical_size == 0 {
            continue;
        }
        if let (Some(device), Some(inode)) = (file.device_id, file.inode) {
            grouped
                .entry((device, inode, file.logical_size))
                .or_default()
                .push(file);
        }
    }
    let mut sets = Vec::new();
    for ((device_id, inode, logical_size), mut paths) in grouped {
        if paths.len() < 2 {
            continue;
        }
        paths.sort_by(|left, right| left.path.cmp(&right.path).then(left.id.cmp(&right.id)));
        sets.push(HardLinkSet {
            logical_size,
            device_id,
            inode,
            paths: paths
                .into_iter()
                .map(|file| HardLinkPath {
                    file_id: file.id,
                    path: file.path.clone(),
                })
                .collect(),
        });
    }
    sets.sort_by(|left, right| {
        right
            .logical_size
            .cmp(&left.logical_size)
            .then(left.device_id.cmp(&right.device_id))
            .then(left.inode.cmp(&right.inode))
    });
    sets
}

fn clusters_of(files: &[Candidate], indexes: &[usize]) -> Vec<Cluster> {
    let mut grouped: BTreeMap<Identity, Vec<usize>> = BTreeMap::new();
    for index in indexes {
        let file = &files[*index];
        let identity = match (file.device_id, file.inode) {
            (Some(device), Some(inode)) => Identity::Inode(device, inode),
            _ => Identity::Solo(file.id),
        };
        grouped.entry(identity).or_default().push(*index);
    }
    let mut clusters = Vec::new();
    for mut members in grouped.into_values() {
        members.sort_by(|&left, &right| {
            files[left]
                .path
                .cmp(&files[right].path)
                .then(files[left].id.cmp(&files[right].id))
        });
        let leader = members[0];
        clusters.push(Cluster { leader, members });
    }
    clusters.sort_by(|left, right| {
        files[left.leader]
            .path
            .cmp(&files[right.leader].path)
            .then(files[left.leader].id.cmp(&files[right.leader].id))
    });
    clusters
}

fn sample_hash(path: &Path) -> io::Result<Sample> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 8192];
    let mut remaining = SAMPLE_BYTES;
    let mut bytes = 0u64;
    while remaining > 0 {
        let want = remaining.min(buf.len() as u64) as usize;
        let read = read_some(&mut file, &mut buf[..want])?;
        if read == 0 {
            return Ok(Sample {
                hash: hasher.finalize().to_hex().to_string(),
                bytes,
                entire_file: true,
            });
        }
        hasher.update(&buf[..read]);
        bytes += read as u64;
        remaining -= read as u64;
    }
    Ok(Sample {
        hash: hasher.finalize().to_hex().to_string(),
        bytes,
        entire_file: false,
    })
}

fn full_hash(path: &Path) -> io::Result<(String, u64)> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 8192];
    let mut bytes = 0u64;
    loop {
        let read = read_some(&mut file, &mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
        bytes += read as u64;
    }
    Ok((hasher.finalize().to_hex().to_string(), bytes))
}

fn files_identical(left: &Path, right: &Path) -> io::Result<bool> {
    let mut left = File::open(left)?;
    let mut right = File::open(right)?;
    let mut left_buf = [0u8; 8192];
    let mut right_buf = [0u8; 8192];
    loop {
        let left_n = read_some(&mut left, &mut left_buf)?;
        let right_n = read_some(&mut right, &mut right_buf)?;
        if left_n != right_n || left_buf[..left_n] != right_buf[..right_n] {
            return Ok(false);
        }
        if left_n == 0 {
            return Ok(true);
        }
    }
}

fn read_some(file: &mut File, buf: &mut [u8]) -> io::Result<usize> {
    loop {
        match file.read(buf) {
            Ok(n) => return Ok(n),
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

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
                "mac-storage-dup-{}-{}-{}",
                std::process::id(),
                n,
                nanos
            ));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn candidate(id: i64, path: PathBuf, inode: Option<u64>, device: Option<u64>) -> Candidate {
        let logical_size = fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
        Candidate {
            id,
            path,
            logical_size,
            inode,
            device_id: device,
        }
    }

    fn identity(path: &Path) -> (u64, u64) {
        use std::os::unix::fs::MetadataExt;
        let meta = fs::metadata(path).unwrap();
        (meta.dev(), meta.ino())
    }

    #[test]
    fn identical_files_form_one_group_and_different_bytes_do_not() {
        let dir = TempDir::new();
        let left = dir.path.join("left.txt");
        let right = dir.path.join("right.txt");
        let other = dir.path.join("other.txt");
        fs::write(&left, b"hello").unwrap();
        fs::write(&right, b"hello").unwrap();
        fs::write(&other, b"world").unwrap();
        let (dev, ino_left) = identity(&left);
        let (_, ino_right) = identity(&right);
        let (_, ino_other) = identity(&other);
        let analysis = find_duplicates(
            &[
                candidate(1, left, Some(ino_left), Some(dev)),
                candidate(2, right, Some(ino_right), Some(dev)),
                candidate(3, other, Some(ino_other), Some(dev)),
            ],
            false,
        );
        assert!(analysis.errors.is_empty());
        assert_eq!(analysis.groups.len(), 1);
        assert_eq!(analysis.groups[0].logical_size, 5);
        assert_eq!(analysis.groups[0].redundant_bytes, 5);
        assert_eq!(analysis.groups[0].members.len(), 2);
        let other_hash = analysis
            .hashes
            .iter()
            .find(|hash| hash.file_id == 3)
            .unwrap();
        assert!(other_hash.full_hash.is_none());
        assert!(!analysis.groups[0].full_hash.is_empty());
    }

    #[test]
    fn zero_byte_files_are_not_duplicates() {
        let dir = TempDir::new();
        let first = dir.path.join("a");
        let second = dir.path.join("b");
        fs::write(&first, b"").unwrap();
        fs::write(&second, b"").unwrap();
        let analysis = find_duplicates(
            &[
                candidate(1, first, Some(1), Some(1)),
                candidate(2, second, Some(2), Some(1)),
            ],
            false,
        );
        assert!(analysis.groups.is_empty());
        assert!(analysis.hashes.is_empty());
        assert!(analysis.hard_link_sets.is_empty());
    }

    #[test]
    fn hard_links_do_not_multiply_redundant_bytes() {
        let dir = TempDir::new();
        let original = dir.path.join("original");
        let link = dir.path.join("link");
        let copy = dir.path.join("copy");
        fs::write(&original, b"hello").unwrap();
        fs::hard_link(&original, &link).unwrap();
        fs::write(&copy, b"hello").unwrap();
        let (dev, ino) = identity(&original);
        let (_, copy_ino) = identity(&copy);
        assert_eq!(identity(&link).1, ino);
        let analysis = find_duplicates(
            &[
                candidate(1, original, Some(ino), Some(dev)),
                candidate(2, link.clone(), Some(ino), Some(dev)),
                candidate(3, copy, Some(copy_ino), Some(dev)),
            ],
            true,
        );
        assert_eq!(analysis.groups.len(), 1);
        assert_eq!(analysis.groups[0].redundant_bytes, 5);
        assert_eq!(analysis.groups[0].members.len(), 3);
        let leaders = analysis.groups[0]
            .members
            .iter()
            .filter(|member| member.hard_link_leader)
            .count();
        assert_eq!(leaders, 2);
        assert_eq!(analysis.hard_link_sets.len(), 1);
        assert_eq!(analysis.hard_link_sets[0].paths.len(), 2);
        assert!(analysis.hard_link_sets[0]
            .paths
            .iter()
            .any(|path| path.path == link));
        assert_eq!(analysis.hashes.len(), 2);
    }

    #[test]
    fn same_prefix_and_different_tail_is_not_a_duplicate() {
        let dir = TempDir::new();
        let mut prefix = vec![b'a'; SAMPLE_BYTES as usize];
        let mut left = prefix.clone();
        left.extend_from_slice(b"LEFT-TAIL");
        prefix.extend_from_slice(b"RIGHT-END");
        let left_path = dir.path.join("left.bin");
        let right_path = dir.path.join("right.bin");
        let unique = dir.path.join("unique.bin");
        fs::write(&left_path, &left).unwrap();
        fs::write(&right_path, &prefix).unwrap();
        let mut other = vec![b'b'; SAMPLE_BYTES as usize];
        other.extend_from_slice(b"OTHER!!!!");
        fs::write(&unique, &other).unwrap();
        let analysis = find_duplicates(
            &[
                candidate(1, left_path, Some(1), Some(1)),
                candidate(2, right_path, Some(2), Some(1)),
                candidate(3, unique, Some(3), Some(1)),
            ],
            true,
        );
        assert!(analysis.groups.is_empty(), "{analysis:?}");
        let unique_hash = analysis
            .hashes
            .iter()
            .find(|hash| hash.file_id == 3)
            .unwrap();
        assert!(unique_hash.full_hash.is_none());
        assert!(analysis
            .hashes
            .iter()
            .filter(|hash| hash.file_id != 3)
            .all(|hash| hash.full_hash.is_some()));
    }

    #[test]
    fn a_missing_file_is_an_error_and_does_not_abort_the_pair() {
        let dir = TempDir::new();
        let left = dir.path.join("left.txt");
        let right = dir.path.join("right.txt");
        fs::write(&left, b"hello").unwrap();
        fs::write(&right, b"hello").unwrap();
        let missing = dir.path.join("gone.txt");
        let analysis = find_duplicates(
            &[
                candidate(1, left, Some(1), Some(1)),
                candidate(2, right, Some(2), Some(1)),
                Candidate {
                    id: 3,
                    path: missing,
                    logical_size: 5,
                    inode: Some(3),
                    device_id: Some(1),
                },
            ],
            false,
        );
        assert_eq!(analysis.groups.len(), 1);
        assert_eq!(analysis.errors.len(), 1);
        assert!(!analysis.errors[0].message.contains("hello"));
    }

    #[test]
    fn byte_comparison_rejects_different_files() {
        let dir = TempDir::new();
        let left = dir.path.join("left");
        let right = dir.path.join("right");
        fs::write(&left, b"aaaa").unwrap();
        fs::write(&right, b"bbbb").unwrap();
        assert!(!files_identical(&left, &right).unwrap());
        assert!(files_identical(&left, &left).unwrap());
    }
}
