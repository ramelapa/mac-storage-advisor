# Features

Status values: Planned, In Progress, Done, Deferred, Experimental.

Only rows with automated tests are Done. v0.11.0 shows each top-level folder as a percent of the scan, like the macOS storage bar, and opens a category inside that folder for review. Nothing is moved until Trash is confirmed. The folder walk stays one thread.

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
| F-011 | Duplicate detection | Done | 0.2 | Size group, then inode/hard link, then sample BLAKE3, then full BLAKE3, then optional `--verify`. Tests cover identical files, distinct same-size files, and a missing path. v0.10 reads those hashes with at most four workers. One worker and four workers return the same groups. |
| F-012 | Content hashing | Done | 0.2 | Streaming BLAKE3. Sample window is 64 KiB. Unique sample hashes are not fully hashed. Contents are not stored. v0.10 reports sample and full-hash progress. A unique sample still skips the full read. |
| F-013 | Hard-link grouping | Done | 0.2 | Same `(device_id, inode)` is one content copy. Every path is listed. Redundant bytes do not multiply those paths. Scan `logical_bytes` still count each path. |
| F-014 | Large file report | Done | 0.2 | `mac-storage large-files` lists the largest stored regular files. |
| F-015 | History command | Done | 0.2 | `mac-storage history` reads previous scan rows, newest first. |
| F-016 | Stale file analyzer | Done | 0.3 | Files whose modified time is at least 180 days old, skipping build directories and Office lock files. |
| F-017 | Downloads analyzer | Done | 0.3 | When the scan root is named Downloads, group user files into installers, archives, documents, media, and other. |
| F-018 | Developer artifact analyzer | Done | 0.3 | Rolls up `node_modules`, `target`, `.venv`, `site-packages`, and similar directories. The earliest matching component wins. |
| F-019 | Recommendation engine | Done | 0.3 | Suggestions only. Document duplicates stay separate from package-metadata duplicates and `~$` lock files. Nothing is deleted. |
| F-020 | Storage trends | Done | 0.3 | Compare logical and allocated totals across scans of the same root. Different folders are not subtracted. |
| F-021 | Tauri desktop UI | Done | 0.6 | `mac-storage window` opens the existing local page in a Tauri window. The page still talks only to `127.0.0.1`. There is no second frontend. |
| F-022 | Move to Trash | Done | 0.5 | `mac-storage trash` moves a named, inventoried path to the OS Trash only when `--confirm` is exactly `move to trash`. Protected paths, the scan root, unknown paths, and iCloud placeholders are refused. A symlink is moved as a link; its target stays. |
| F-023 | Parallel scanning | Deferred | 0.2 | The folder walk stays one thread. `--threads` is stored. `SCAN_CONCURRENCY` is 1. Duplicate hashing is separate and capped at `HASH_CONCURRENCY` (4). |
| F-024 | iCloud placeholder handling | Done | 0.5 | macOS `SF_DATALESS` is stored. Those files are left out of the stale and Downloads tallies. Their logical size is not treated as local disk usage, and Trash refuses them. |
| F-025 | APFS clone awareness | Done | 0.5 | When allocated size is below logical size, the gap is reported as sparse, compressed, or shared extents. Clone groups are not inferred. The gap is not reclaimable space. |
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
| F-036 | Local interactive UI | Done | 0.4 | `mac-storage ui` listens on `127.0.0.1` and shows scan, duplicate, large-file, suggestion, and trend results. The page also runs the command-line verbs. It does not upload or delete files. Results are split into sections so one long page is not the only way to read them. |
| F-037 | Database check | Done | 0.6 | `mac-storage doctor` confirms the file is this app's database, the schema matches, and SQLite's integrity check passes. It lists recent Trash moves. It does not change user files. A foreign database is left untouched. |
| F-038 | Folder totals | Done | 0.7 | `mac-storage folders` groups stored regular files under the scan root. Deeper files roll into the ancestor at `--depth`. iCloud placeholder bytes stay separate. Directory inode sizes are not added, and the totals are not reclaimable space. |
| F-039 | Mac application | Done | 0.8 | `scripts/build-mac-app.sh` builds `Mac Storage Advisor.app` on macOS from the same binary. Opening the app, with no advisor command, starts the local window. A subcommand still runs as the command line. The app is not sandboxed, does not request Full Disk Access, and does not listen outside `127.0.0.1`. |
| F-040 | Duplicate browser | Done | 0.8.1 | The interactive duplicate list can be filtered, sorted, grouped by folder or file type, and paged. Each group stays collapsed until opened. Listing extra copies for Trash does not move them. |
| F-041 | Mac overview | Done | 0.9.0 | Scan this Mac walks the home folder and updates file and byte counts while the walk runs. The overview chart compares top-level folders and points at unchanged files, developer folders, Library, and Downloads. Those hints are not free space, and the chart does not move files. |
| F-042 | File categories | Done | 0.10.0 | `mac-storage categories` groups stored regular files into documents, images, media, archives, installers, developer artifacts, and other. A category lists those paths. Files inside a developer-artifact directory stay in developer. iCloud placeholder bytes are omitted from the category size. Nothing is moved or renamed. |
| F-043 | Suggested place | Done | 0.10.0 | `mac-storage place --path FILE` suggests a file name and folder from the name, the extension, and where other files of that extension already live. The suggestion skips developer-artifact trees, iCloud placeholders, and protected macOS paths. The file moves only when `--confirm` is exactly `move file`. An existing destination, a symlink, a directory, and a protected path are refused. Nothing is permanently deleted. |
| F-044 | Storage bar | Done | 0.11.0 | `mac-storage storage` shows each top-level folder as a percent of the local files in the scan. The shares add up to 100%. iCloud placeholder bytes are not part of the percent. `--folder` opens that folder's categories, and `--category` lists the largest local files to review. Nothing is moved. |
