# Duplicate detection

**Status: implemented in v0.2.** `mac-storage duplicates` follows this pipeline. Do not treat size groups as duplicates. Zero-byte files are skipped. `--verify` is the optional byte comparison and is off unless requested.

## Goal

Report sets of regular files that have the same content, without reading more bytes than necessary, and without deleting anything.

## Pipeline

1. **Size group.** Within one scan, group persisted regular files by `logical_size`. A group of one cannot be a duplicate. Zero-byte files are a group of their own and must not be reported as interesting duplicates by default.
2. **Inode / hard link.** Files that share `(device_id, inode)` are the same inode. Count them once for content identity and list every path. Do not hash them against each other. Logical-byte totals in v0.1 still count each path; duplicate detection must not silently change those totals.
3. **Sample BLAKE3.** For remaining groups, hash a fixed sample window (design default: the first 64 KiB, or the whole file if it is smaller). Drop files whose sample hash is unique in the group.
4. **Full BLAKE3.** Hash the entire remaining candidates with streaming BLAKE3. Equal full hashes are duplicate candidates.
5. **Optional byte verify.** A later flag may re-read candidates and `memcmp` them before showing a group. This is for distrust of the hash, not the default, because the full hash is the identity step.

## Constraints

- Hashes are computed locally and stored in SQLite. They are never uploaded.
- Symlinks are not hashed as if they were the target. Hash the target only when the scan recorded that path as a regular file.
- Do not follow symlinks to reach a file outside the scan.
- Sparse files and APFS clones can share physical blocks while having different logical contents, or the reverse. Duplicate detection compares logical content, not allocated size, and must not claim that deleting one path frees `logical_size` bytes.
- Hard links must not be presented as reclaiming `n * size` bytes.
- The scanner in v0.1 does not read file contents. Hashing is a separate pass over paths already stored for a scan.

## Storage

Migration 002 creates:

- `content_hashes (scan_id, file_id, algorithm, sample_hash, full_hash, hashed_bytes)`
- `duplicate_groups (scan_id, logical_size, full_hash, redundant_bytes)`
- `duplicate_members (group_id, file_id, hard_link_leader)`

Hard-link sets are derived from `files.device_id` and `files.inode` when the command runs. They are not a separate table. Re-running `duplicates` replaces hash and group rows for that scan. Scan totals are left unchanged.

## Out of scope

Photo or video similarity, perceptual hashes, and semantic "same document" matching are Future items, not this pipeline.
