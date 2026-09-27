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
- Architecture, safety, storage, filesystem, roadmap, and ADR documents. Duplicate detection is specified and not implemented.

### Not in this version

- Duplicate detection, content hashing, analyzers, recommendations, trends, history browsing, remediation, and the Tauri UI.
