# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Cargo workspace with `mac-storage-common`, `mac-storage-scanner`, `mac-storage-storage`, and the `mac-storage` CLI.
- Configurable display name (`PRODUCT_NAME` in `crates/common`). The current placeholder is "Mac Storage Advisor".
- Recursive directory scan that records metadata and does not read file contents.
- Symlink policy: links are not followed, loops finish, broken links are recorded, and a symlink scan root is refused.
- Exclusion rules (name, path prefix, glob) and default refusal/skipping of protected macOS prefixes.
- Non-fatal collection of scan errors (permission denied, missing entries, unreadable directories).
- SQLite persistence with schema migrations (`scans`, `files`, `directories`, `settings`, `exclusions`, `scan_errors`).
- `mac-storage scan <PATH>` with `--json`, `--verbose`, `--quiet`, `--exclude`, `--threads`, and `--min-size`.
- `--threads` is accepted and stored. Scanning in this version is single-threaded.
- GitHub Actions workflow for `cargo fmt`, `cargo clippy -D warnings`, and `cargo test` on Ubuntu and macOS.
- Architecture, safety, storage, filesystem, roadmap, and ADR documents.
- `mac-storage duplicates`: size groups, hard-link collapse, sample BLAKE3 (64 KiB), full BLAKE3, and optional `--verify` byte comparison. Zero-byte files are not reported. Redundant bytes do not multiply hard links and are not a free-space promise.
- SQLite migration 002: `content_hashes`, `duplicate_groups`, `duplicate_members`.
- `mac-storage large-files` and `mac-storage history`.
- Decision: `--threads` stays recorded only. Scanning and hashing remain one thread (`SCAN_CONCURRENCY = 1`).
- `mac-storage analyze`, `recommendations`, and `trends`. Suggestions separate document duplicates from dependency-tree duplicates and Office `~$` lock files. Nothing is deleted. Trends compare scans of the same root only.
- `mac-storage ui` serves a page on `127.0.0.1` (default port 47231). The page has an interactive view and a command-line view. Both call the same scan, duplicate, and suggestion code and the same SQLite file. The listener refuses any other host. The page is not a system shell.
- macOS `SF_DATALESS` is stored. Placeholder files are left out of stale and Downloads totals, and Trash refuses them.
- Files whose allocated size is below their logical size are reported as a sparse, compressed, or shared-extent gap. That gap is not reclaimable space, and clone groups are not inferred.
- `mac-storage trash` moves named paths from a stored scan to the OS Trash only when `--confirm` is exactly `move to trash`. Without that phrase, nothing is moved.
- `mac-storage doctor` checks product identity, schema version, and SQLite integrity. It lists recent Trash moves. A file that is not this app's database is left unchanged.
- `mac-storage window` opens the existing local page in a Tauri window. The listener remains `127.0.0.1`.

### Not in this version

- Parallel scanning and APFS clone grouping. Permanent delete will not exist.
