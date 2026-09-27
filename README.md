# Mac Storage Advisor

Local, non-destructive scan of a directory on macOS. This repository is the v0.1 foundation: it walks a tree, records metadata in SQLite, and prints a summary. The display name is a placeholder (`PRODUCT_NAME` in `crates/common`). Crate and binary names stay `mac-storage`.

It does not hash files, upload anything, or delete anything. Duplicate detection is specified and not implemented.

## What works now

- `mac-storage scan <PATH>` walks a directory recursively.
- Metadata for files, directories, symlinks, and other node types when `lstat` succeeds.
- Logical size and, on Unix, allocated size (`st_blocks * 512`). Allocated size is not treated as reclaimable space.
- Exclusions (name, path prefix, glob) and a conservative macOS protected-prefix policy.
- Symlinks are not followed. Loops finish. Broken symlinks are recorded. A symlink used as the scan root is refused.
- Permission errors and missing entries are recorded. The scan continues.
- Results are stored in SQLite (migrations included).
- `--json` prints a stable summary plus error path and message. File contents are not included.

## Planned

Duplicates, large and stale files, Downloads and developer-artifact reviews, recommendations, trends, a history command, move-to-Trash, and a Tauri UI. See [docs/features.md](docs/features.md) and [docs/roadmap.md](docs/roadmap.md).

Not in the MVP: AI suggestions, cloud sync, cross-device inventory, automatic or permanent deletion, photo/video similarity, semantic duplicates, and Windows or Linux as packaged products.

## Safety

The tool reads directory listings and metadata. It does not open file contents. Permanent delete will not exist. A later release may move user-confirmed paths to Trash. Details: [docs/safety-model.md](docs/safety-model.md).

Protected prefixes `/System`, `/private`, `/bin`, `/sbin`, `/usr`, and `/Library` are skipped only when they fall inside the scan root (for example when the root is `/`). Passing `~/Downloads` scans that folder. Passing exactly `/usr` is refused unless `--allow-protected-roots` is set. An explicit root under a prefix, such as `/usr/local/myproject`, is scanned.

## Architecture

| Path | Role |
| --- | --- |
| `crates/common` | Domain types and the product name |
| `crates/scanner` | `walkdir` scan, exclusions, metadata |
| `crates/storage` | SQLite via `rusqlite` (bundled) and migrations |
| `apps/cli` | `mac-storage` binary |

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
```

The database defaults to the platform data directory (`mac-storage.sqlite` under the `mac-storage-advisor` application directory). Override it with `--db` or `MAC_STORAGE_DB`.

## CLI

```text
mac-storage scan <PATH>
mac-storage scan <PATH> --json
mac-storage scan <PATH> --exclude node_modules --exclude target --min-size 1MiB
```

| Flag | Behavior |
| --- | --- |
| `--json` | Stable JSON on stdout. Logs stay on stderr. |
| `--verbose` | Debug logs on stderr. Paths and sizes only, never contents. |
| `--quiet` | No human summary and no info logs. |
| `--exclude` | Repeatable. Name, prefix, or glob. See below. |
| `--threads` | Accepted and stored. Scanning uses one thread. |
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
  "version": "0.1.0",
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

Help text lists planned commands that are not implemented: `duplicates`, `large-files`, `developer`, `report`, `history`, `recommendations`, `doctor`.

## Development

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Tests use temporary fixtures only. See [docs/development.md](docs/development.md) and [CONTRIBUTING.md](CONTRIBUTING.md).

## Project status

v0.1 scan foundation. Feature status is authoritative in [docs/features.md](docs/features.md). Duplicate detection is the next implementation step (size grouping) and has not been started.

## Limitations

- Single-threaded. `--threads` does not parallelize.
- The whole scan is held in memory, then inserted in one transaction.
- Logical bytes count each hard-link path separately.
- Allocated size is not unique physical usage and is not reclaimable space.
- Birth time may be missing on Linux. It is not replaced with modified time.
- Exclusion matching is case-sensitive.
- macOS aliases, iCloud placeholders, APFS clones, and snapshots are not interpreted.
- If the SQLite file sits inside the scan root, a later scan counts it.
- No desktop UI.

## License

MIT OR Apache-2.0. See [LICENSE](LICENSE).
