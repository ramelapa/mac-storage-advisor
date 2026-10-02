# Mac Storage Advisor

Local inventory of a directory on macOS. `scan` walks a tree and records metadata in SQLite. `duplicates` then hashes those regular files locally. The display name is `PRODUCT_NAME` in `crates/common` (`Mac Storage Advisor`). Crate and binary names stay `mac-storage`.

One binary, one database, three ways to use it: the command line, `mac-storage ui` (a page on `127.0.0.1`), and `mac-storage window` (that same page in a native window). On a Mac, `scripts/build-mac-app.sh` packages that window as `Mac Storage Advisor.app`. Opening the app starts the window.

Nothing is uploaded or permanently deleted. `trash` moves a path only after you confirm it, and only to the operating-system Trash. Duplicate groups report extra content copies. That number is not bytes the disk will free.

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
- `mac-storage folders` groups those stored regular files by folder. Files deeper than `--depth` roll into that ancestor. iCloud placeholder bytes stay separate. The totals are not free space, and directory inode sizes are not included.
- `mac-storage history` lists previous scans.
- On macOS, `scan` records iCloud placeholders (`SF_DATALESS`). `analyze` leaves those files out of the stale and Downloads tallies. When allocated size is below logical size, that gap is reported and is not called free space.
- `mac-storage analyze` reviews stale files, a Downloads folder, and developer-artifact directories.
- `mac-storage recommendations` prints suggestions only. It does not move files.
- `mac-storage trends` compares totals for the same folder across scans. Different folders are not subtracted.
- `mac-storage ui` opens a local page on `127.0.0.1` (default port 47231). Scan this Mac walks your home folder and shows progress while it runs. The overview chart compares the largest top-level folders and points at unchanged files, developer folders, Library, and Downloads. Those hints are not free space. The page also shows large files, duplicates, suggestions, history, trends, a database check, and a Trash preview, one section at a time. Duplicate groups can be filtered, grouped by folder or file type, and paged. Its command-line view runs those same commands. It is not a system shell.
- `mac-storage window` opens that same page in a Tauri window. The listener is still `127.0.0.1`.
- On macOS, `scripts/build-mac-app.sh` builds `Mac Storage Advisor.app` and a disk image. Opening the app starts that window and uses the same database. A command typed at the binary inside the app still runs as the command line.
- `mac-storage trash --path <PATH>` previews a move and does not touch the file. Adding `--confirm "move to trash"` moves that inventoried path to the OS Trash. iCloud placeholders, protected macOS paths, unknown paths, and the scan folder itself are refused. A symlink is moved as a link; its target stays.
- `mac-storage doctor` checks that the database belongs to this app, that the schema matches, and that SQLite's integrity check passes. It lists recent Trash moves and does not change files. A foreign database is left untouched.

## Planned

Nothing from the current roadmap is waiting. Later ideas are listed as deferred in [docs/features.md](docs/features.md) and [docs/roadmap.md](docs/roadmap.md).

Not in the MVP: AI suggestions, cloud sync, cross-device inventory, automatic or permanent deletion, photo/video similarity, semantic duplicates, and Windows or Linux as packaged products.

## Safety

`scan` reads directory listings and metadata. `duplicates` opens file contents only to hash them on this machine. Permanent delete does not exist. `trash` moves a user-confirmed path to the operating-system Trash, which the OS can restore until it is emptied. Automatic deletion is not implemented. Details: [docs/safety-model.md](docs/safety-model.md).

Protected prefixes `/System`, `/private`, `/bin`, `/sbin`, `/usr`, and `/Library` are skipped only when they fall inside the scan root (for example when the root is `/`). Passing `~/Downloads` scans that folder. Passing exactly `/usr` is refused unless `--allow-protected-roots` is set. An explicit root under a prefix, such as `/usr/local/myproject`, is scanned.

## Architecture

| Path | Role |
| --- | --- |
| `crates/common` | Domain types and the product name |
| `crates/scanner` | `walkdir` scan, exclusions, metadata, iCloud dataless flag |
| `crates/duplicates` | BLAKE3 duplicate grouping over stored regular files |
| `crates/analyze` | Stale files, Downloads, artifact trees, suggestions, trends, placeholder and extent notes |
| `crates/storage` | SQLite via `rusqlite` (bundled) and migrations |
| `crates/remediate` | Confirmed move to the OS Trash |
| `apps/cli` | `mac-storage` binary: command line, local page, and native window |

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
cargo run -p mac-storage -- window
```

On a Mac, build an app you can open from Finder:

```bash
./scripts/build-mac-app.sh
open "target/release/bundle/macos/Mac Storage Advisor.app"
```

The script is macOS-only. It ad-hoc signs the app so it opens on the machine that built it. Set `APPLE_SIGNING_IDENTITY` to a Developer ID when you want a signature for other Macs. If Gatekeeper refuses an ad-hoc app, right-click the app, choose Open, then Open. The app is not sandboxed and does not request Full Disk Access. It listens on `127.0.0.1` only.

On Linux, building `window` needs the WebKit development libraries (`libwebkit2gtk-4.1-dev` and GTK). A Mac already has that webview. GitHub Actions installs the Linux packages before the tests.

The database defaults to this app's own data directory and does not replace any other application's database. On macOS that file is `~/Library/Application Support/com.mac-storage.mac-storage-advisor/mac-storage.sqlite`. On Linux it is `~/.local/share/mac-storage-advisor/mac-storage.sqlite`. Override it with `--db` or `MAC_STORAGE_DB`. A path that already points at a different SQLite file is refused.

## CLI

```text
mac-storage scan <PATH>
mac-storage scan <PATH> --json
mac-storage scan <PATH> --exclude node_modules --exclude target --min-size 1MiB
mac-storage duplicates
mac-storage duplicates --verify --json
mac-storage large-files --limit 20
mac-storage folders --depth 1
mac-storage history
mac-storage analyze
mac-storage recommendations
mac-storage trends
mac-storage trash --path ~/Downloads/old.dmg
mac-storage trash --path ~/Downloads/old.dmg --confirm "move to trash"
mac-storage doctor
mac-storage ui
mac-storage window
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
  "version": "0.9.0",
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

`duplicates`, `large-files`, `history`, `analyze`, and `recommendations` read the newest scan unless `--scan ID` is set. They exit 1 when the database has no scans. Hash errors are listed and do not abort the rest of the group. `doctor` exits 1 when the database cannot be opened or an integrity check fails.

`duplicates --json` includes `duplicate_groups`, `hard_link_sets`, and `hash_errors`. File bytes are not in that object. `redundant_bytes` counts extra content copies after hard links are collapsed. Zero-byte files are not reported as duplicates.

`analyze` skips build directories and `~$` Office lock files when it counts stale files. Duplicate groups that sit entirely inside those directories are reported as package metadata, not as extra documents. iCloud placeholders are left out of the stale and Downloads tallies, and Trash will not move them. `trends` only subtracts two scans of the same root.

## Development

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Tests use temporary fixtures only. See [docs/development.md](docs/development.md) and [CONTRIBUTING.md](CONTRIBUTING.md).

## Project status

v0.9 is the current release. It can scan your home folder with progress on screen, show which top-level folders hold the space, find identical files, suggest what to review, compare the same folder over time, move a confirmed path to Trash, check the local database, and show those results on a localhost page, in a native window, or in Mac Storage Advisor.app. A long duplicate list can be filtered and paged instead of scrolled as one block. Feature status is authoritative in [docs/features.md](docs/features.md). `--threads` stays single-threaded.

## Limitations

- Single-threaded. `--threads` does not parallelize.
- The whole scan is held in memory, then inserted in one transaction.
- A directory row still stores that directory's own inode size. Folder totals are computed from stored regular files when `folders` runs. Files omitted by `--min-size` are not in that rollup.
- Logical bytes count each hard-link path separately.
- Allocated size is not unique physical usage and is not reclaimable space.
- Birth time may be missing on Linux. It is not replaced with modified time.
- Exclusion matching is case-sensitive.
- On macOS, iCloud placeholders are recorded with `SF_DATALESS`. Their logical size is not treated as local disk usage. Linux scans store that flag as false.
- An allocated size below the logical size is reported as a sparse, compressed, or shared-extent gap. That gap is not reclaimable space. Clone families and APFS snapshots are not computed.
- macOS aliases are recorded as normal files. They are not resolved.
- If the SQLite file sits inside the scan root, a later scan counts it.
- The local page and the native window listen on `127.0.0.1` only. The page handles one request at a time.

## License

MIT OR Apache-2.0. See [LICENSE](LICENSE).
