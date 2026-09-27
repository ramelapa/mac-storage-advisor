# Storage model

Three different numbers show up in disk tools. This project stores the first two and does not compute the third.

| Term | Meaning here | v0.1 |
| --- | --- | --- |
| Logical size | `Metadata::len`. The file's size in bytes, including holes in a sparse file. | Stored on each file. `logical_bytes` sums regular files once per directory entry. |
| Allocated size | Unix `st_blocks * 512` when that field exists. | Stored separately. `None` when the platform cannot report it. Never copied from the logical size. |
| Reclaimable space | Bytes that would become free if a path were removed, after clones, hard links, and snapshots. | Not computed. Do not treat either stored size as reclaimable. |

Hard links add their logical size once per path in scan totals. APFS clones can share extents, so allocated size can overstate unique usage. Sparse files can have a logical size larger than the allocated size. Directory `logical_size` is the directory inode's size, not a rollup of children.

Birth time is `Metadata::created`. On macOS this is usually the APFS birth time. On Linux it is often unavailable; when it is missing the column is NULL. Modified time is not written into the birth-time column.

## Database location

`directories::ProjectDirs` with qualifier `com`, organization `mac-storage`, and application `mac-storage-advisor`. The file is `mac-storage.sqlite` inside the platform data directory.

Override with `--db PATH` or the `MAC_STORAGE_DB` environment variable. The flag wins. Tests set the environment variable so they never open the developer data directory.

If that file lives inside the directory you scan, a later scan will count it like any other file. The default data directory is outside folders such as `~/Downloads`.

## Schema migration 001

`schema_migrations(version, applied_at)` records applied versions. Opening the database applies any missing migration inside a transaction. Migration 1 creates:

- `scans` — root, time range, status (`completed` or `completed_with_errors`), counters, elapsed time, min size, requested threads, product name and version
- `files` — non-directory entries (`file`, `symlink`, `other`), sizes, timestamps, inode, device, permissions, symlink target
- `directories` — directory inode metadata, not a recursive total
- `settings` — `product_name`, `schema_version`
- `exclusions` — patterns applied to that scan, including protected prefixes that were in scope
- `scan_errors` — path and message for non-fatal failures

Indexes:

- `files(path)`, `files(logical_size)`, `files(scan_id)`, `files(modified_time)`
- `directories(path)`, `directories(scan_id)`, `directories(modified_time)`
- `exclusions(scan_id)`, `scan_errors(scan_id)`

Foreign keys are enabled. Child rows reference `scans(id)` and cascade on delete. The application does not delete scan rows in this version.

Timestamps are Unix epoch milliseconds. SQLite integers are signed 64-bit. A value that does not fit is rejected for required counters and stored as NULL for optional inode or device ids.

## Planned tables (not created)

`content_hashes`, `duplicate_groups`, `duplicate_members`, `recommendations`, and `trend_points` wait until those features exist. They are not stubbed in migration 001.

## Memory

The scan is held in memory and then written in one transaction. That is acceptable for the foundation and will not scale to an entire disk without a streaming insert. It is a known limitation.
