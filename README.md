# Mac Storage Advisor

Local, non-destructive inventory of a directory on macOS. `scan` walks a tree and records metadata in SQLite. `duplicates` then hashes those regular files locally. The display name is a placeholder (`PRODUCT_NAME` in `crates/common`). Crate and binary names stay `mac-storage`.

The same binary has two run modes. The command line is `mac-storage scan` and the other report commands. `mac-storage ui` serves a page on `127.0.0.1` with an interactive view and a command-line view. Both use the same database.

Nothing is uploaded or deleted. Duplicate groups report extra content copies. That number is not bytes the disk will free.

## What works now

- `mac-storage scan <PATH>` walks a directory recursively.
- Metadata for files, directories, symlinks, and other node types when `lstat` succeeds.
- Logical size and, on Unix, allocated size (`st_blocks * 512`). Allocated size is not treated as reclaimable space.
- Exclusions (name, path prefix, glob) and a conservative macOS protected-prefix policy.
- Symlinks are not followed. Loops finish. Broken symlinks are recorded. A symlink used as the scan root is refused.
- Permission errors and missing entries are recorded. The scan continues.
- Results are stored in SQLite (migrations included).
- `--json` prints a stable summary plus error path and message. File contents are not included.
- `mac-storage duplicates` groups identical regular files from a stored scan (size, then inode, then sample BLAKE3, then full BLAKE3). `--verify` re-reads candidates.
- `mac-storage large-files` lists the largest stored regular files.
- `mac-storage history` lists previous scans.
- `mac-storage analyze` reviews stale files, a Downloads folder, and developer-artifact directories.
- `mac-storage recommendations` prints suggestions only. It does not delete anything.
- `mac-storage trends` compares totals for the same folder across scans.
- `mac-storage ui` opens a local page on `127.0.0.1` (default port 47231). The page scans a folder and shows large files, duplicates, suggestions, history, and trends. Its command-line view runs those same commands. It is not a system shell.

## Planned

Move-to-Trash, a Tauri window, iCloud placeholder handling, and APFS clone awareness. See [docs/features.md](docs/features.md) and [docs/roadmap.md](docs/roadmap.md).

Not in the MVP: AI suggestions, cloud sync, cross-device inventory, automatic or permanent deletion, photo/video similarity, semantic duplicates, and Windows or Linux as packaged products.

## Safety

`scan` reads directory listings and metadata. `duplicates` opens file contents only to hash them on this machine. Permanent delete will not exist. A later release may move user-confirmed paths to Trash. Details: [docs/safety-model.md](docs/safety-model.md).

Protected prefixes `/System`, `/private`, `/bin`, `/sbin`, `/usr`, and `/Library` are skipped only when they fall inside the scan root (for example when the root is `/`). Passing `~/Downloads` scans that folder. Passing exactly `/usr` is refused unless `--allow-protected-roots` is set. An explicit root under a prefix, such as `/usr/local/myproject`, is scanned.

## Architecture

| Path | Role |
| --- | --- |
| `crates/common` | Domain types and the product name |
| `crates/scanner` | `walkdir` scan, exclusions, metadata |
| `crates/duplicates` | BLAKE3 duplicate grouping over stored regular files |
| `crates/storage` | SQLite via `rusqlite` (bundled) and migrations |
| `apps/cli` | `mac-storage` binary: command line and the local page |

The scanner does not depend on storage. `walkdir` is used instead of `ignore` so symlink policy and the exclusion language stay explicit. Diagram and rules: [docs/architecture.md](docs/architecture.md).

## Install

Requires Rust 1.98.1 (`rust-toolchain.toml`).

```bash
cargo install --path apps/cli
mac-storage scan ~/Downloads
```

Or without installing:

```bash
cargo run -p mac-storage -- scan ~/Downloads
cargo run -p mac-storage -- ui
```

The database defaults to this app's own data directory and does not replace any other application's database. On macOS that file is `~/Library/Application Support/com.mac-storage.mac-storage-advisor/mac-storage.sqlite`. Override it with `--db` or `MAC_STORAGE_DB`. A path that already points at a different SQLite file is refused.

## CLI

```text
mac-storage scan <PATH>
mac-storage scan <PATH> --json
mac-storage scan <PATH> --exclude node_modules --exclude target --min-size 1MiB
mac-storage duplicates
mac-storage duplicates --verify --json
mac-storage large-files --limit 20
mac-storage history
mac-storage analyze
mac-storage recommendations
mac-storage trends
mac-storage ui
mac-storage ui --port 47231 --db /path/to/mac-storage.sqlite
```

| Flag | Behavior |
| --- | --- |
| `--json` | Stable JSON on stdout. Logs stay on stderr. |
| `--verbose` | Debug logs on stderr. Paths and sizes only, never contents. |
| `--quiet` | No human summary and no info logs. |
| `--exclude` | Repeatable. Name, prefix, or glob. See below. |
| `--threads` | Accepted and stored on `scan`. Scanning and hashing use one thread. |
| `--min-size` | Bytes, or `K`/`KB`/`KiB`/`M`/`MiB`/`G`/`GiB`/`T`/`TiB`. Every regular file is still counted. Files below the threshold are not persisted. |
| `--redact-paths` | Redact paths in logs. JSON and SQLite still store them. |
| `--db` | SQLite file. Overrides `MAC_STORAGE_DB`. |
| `--allow-protected-roots` | Dev override for an exact protected prefix. Does not enable deletion. |

Exit status `0` means the scan finished, even when some entries were recorded as errors. Exit status `1` means the scan could not start (missing path, symlink root, protected root, database failure).

Human output includes directories scanned, files scanned, logical bytes, errors, and elapsed time. Info logs on stderr repeat the same counters for tracing.

`--json` shape:

```json
{
  "product": "Mac Storage Advisor",
  "version": "0.4.0",
  "scan_id": 1,
  "root": "/path/to/fixture",
  "directories_scanned": 2,
  "files_scanned": 2,
  "logical_bytes": 8,
  "allocated_bytes": 8192,
  "allocated_bytes_complete": true,
  "symlinks": 0,
  "other_entries": 0,
  "errors": 0,
  "skipped": 0,
  "files_below_min_size": 0,
  "elapsed_ms": 1,
  "threads_requested": 1,
  "concurrency": 1,
  "min_logical_size": 0,
  "database": "/path/to/mac-storage.sqlite",
  "error_details": []
}
```

`allocated_bytes` depends on the filesystem. `error_details` items are `{ "path", "message" }`.

### Exclusion examples

- `--exclude node_modules` skips directories whose name is `node_modules`.
- `--exclude sub/cache` skips that path relative to the scan root, and its descendants.
- `--exclude '*.dmg'` skips files whose name matches. The quotes are for the shell.
- `--exclude 'Downloads/*.dmg'` matches that suffix, not files in a nested folder.
- `--exclude '/tmp/**/*.txt'` is an absolute glob.

`duplicates`, `large-files`, `history`, `analyze`, and `recommendations` read the newest scan unless `--scan ID` is set. They exit 1 when the database has no scans. Hash errors are listed and do not abort the rest of the group. `doctor` is still only named in the help text.

`duplicates --json` includes `duplicate_groups`, `hard_link_sets`, and `hash_errors`. File bytes are not in that object. `redundant_bytes` counts extra content copies after hard links are collapsed. Zero-byte files are not reported as duplicates.

`analyze` skips build directories and `~$` Office lock files when it counts stale files. Duplicate groups that sit entirely inside those directories are reported as package metadata, not as extra documents. `trends` only subtracts two scans of the same root.

## Development

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Tests use temporary fixtures only. See [docs/development.md](docs/development.md) and [CONTRIBUTING.md](CONTRIBUTING.md).

## Project status

v0.4 adds `mac-storage ui`, a localhost page with an interactive view and a command-line view, on top of the v0.3 suggestions. Feature status is authoritative in [docs/features.md](docs/features.md). Trash and a Tauri window are still later work. `--threads` stays single-threaded.

## Limitations

- Single-threaded. `--threads` does not parallelize.
- The whole scan is held in memory, then inserted in one transaction.
- Logical bytes count each hard-link path separately.
- Allocated size is not unique physical usage and is not reclaimable space.
- Birth time may be missing on Linux. It is not replaced with modified time.
- Exclusion matching is case-sensitive.
- macOS aliases, iCloud placeholders, APFS clones, and snapshots are not interpreted.
- If the SQLite file sits inside the scan root, a later scan counts it.
- The local page listens on `127.0.0.1` only and handles one request at a time. A Tauri window is not part of this version.

## License

MIT OR Apache-2.0. See [LICENSE](LICENSE).
