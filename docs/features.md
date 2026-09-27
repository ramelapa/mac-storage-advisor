# Features

Status values: Planned, In Progress, Done, Deferred, Experimental.

Only rows with automated tests are Done. Duplicate detection is not started.

| ID | Feature | Status | Release | Description |
| --- | --- | --- | --- | --- |
| F-001 | Recursive directory scan | Done | 0.1 | Walk a selected directory and count directories, regular files, and logical bytes. |
| F-002 | File and directory metadata | Done | 0.1 | Path, name, extension, logical size, allocated size when `st_blocks` exists, timestamps when the OS provides them, inode, device, permissions, file type, symlink status. |
| F-003 | Symlink-safe walk | Done | 0.1 | Do not follow symlinks. Loops finish. Broken symlinks are recorded. A symlink scan root is refused. |
| F-004 | Exclusions | Done | 0.1 | Name, component-wise prefix, and simple glob exclusions. Documented in `docs/architecture.md`. |
| F-005 | Protected macOS roots | Done | 0.1 | Exact protected prefixes are refused. Those prefixes are skipped only when they are descendants of the scan root. |
| F-006 | Scan error collection | Done | 0.1 | Permission errors, missing entries, and unreadable directories are recorded. The scan continues. |
| F-007 | SQLite scan persistence | Done | 0.1 | Migrations, scan header, file rows, directory rows, exclusions, and error rows. |
| F-008 | CLI `scan` | Done | 0.1 | `mac-storage scan <PATH>` with human output: directories, files, logical bytes, errors, elapsed time. |
| F-009 | JSON scan report | Done | 0.1 | `--json` prints a stable serde object with those fields plus error path and message. No file contents. |
| F-010 | Minimum size filter | Done | 0.1 | `--min-size` still counts every regular file. Smaller files are omitted from persisted rows. |
| F-011 | Duplicate detection | Planned | 0.2 | Size group, then inode/hard link, then sample BLAKE3, then full BLAKE3, then optional byte compare. Not implemented. |
| F-012 | Content hashing | Planned | 0.2 | BLAKE3 streaming hashes. The crate is not a dependency yet. |
| F-013 | Hard-link grouping | Planned | 0.2 | Inode and device are stored now. Grouping is not implemented. Logical bytes still count each path. |
| F-014 | Large file report | Planned | 0.2 | List the largest files from a stored scan. |
| F-015 | History command | Planned | 0.2 | Read previous scan rows. Persistence exists; the command does not. |
| F-016 | Stale file analyzer | Planned | 0.3 | Surface old files using stored timestamps. No policy engine yet. |
| F-017 | Downloads analyzer | Planned | 0.3 | Review typical download locations. Not implemented. |
| F-018 | Developer artifact analyzer | Planned | 0.3 | `target`, `node_modules`, and similar build outputs. Exclusion by name works today; there is no analyzer. |
| F-019 | Recommendation engine | Planned | 0.3 | Non-destructive suggestions. `RiskLevel` and `RecommendationCategory` exist as types only. |
| F-020 | Storage trends | Planned | 0.3 | Compare logical and allocated totals across scans. |
| F-021 | Tauri desktop UI | Planned | 0.3 | Share the Rust core. No UI crate in this workspace. |
| F-022 | Move to Trash | Planned | 0.3 | Later remediation moves items to the OS Trash. Not implemented. |
| F-023 | Parallel scanning | Planned | 0.2 | `--threads` is accepted and recorded. Execution is one thread. |
| F-024 | iCloud placeholder handling | Planned | 0.3 | Do not treat dataless files as reclaimable without an explicit model. Not implemented. |
| F-025 | APFS clone awareness | Planned | 0.3 | Allocated size is `st_blocks * 512`, not unique physical usage. Clone sharing is not computed. |
| F-026 | AI suggestions | Deferred | Future | Out of scope for the local deterministic core. |
| F-027 | Cloud sync | Deferred | Future | Conflicts with local-first. No upload path exists. |
| F-028 | Cross-device inventory | Deferred | Future | Not part of MVP. |
| F-029 | Automatic deletion | Deferred | Future | Will not be implemented. |
| F-030 | Permanent delete | Deferred | Future | Will not exist. Trash is the only future removal mechanism. |
| F-031 | Photo or video similarity | Deferred | Future | Not content hashing and not part of MVP. |
| F-032 | Semantic duplicate detection | Deferred | Future | Not part of MVP. |
| F-033 | Windows product | Deferred | Future | The engine avoids Unix-only APIs where practical. There is no Windows product. |
| F-034 | Linux product | Deferred | Future | Tests run on Linux. Packaged support is macOS. |
| F-035 | Filesystem watcher | Deferred | Future | `notify` is not a dependency. |
