# Development

## Toolchain

`rust-toolchain.toml` pins Rust 1.98.1 with `rustfmt` and `clippy`. The workspace MSRV field is 1.85 because current `clap` 4.6 requires it. CI installs 1.98.1.

```bash
cargo fmt --all
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo run -p mac-storage -- scan /path/to/fixture
cargo run -p mac-storage -- scan /path/to/fixture --json
cargo run -p mac-storage -- duplicates --json
cargo run -p mac-storage -- large-files
cargo run -p mac-storage -- history
cargo run -p mac-storage -- analyze
cargo run -p mac-storage -- recommendations
cargo run -p mac-storage -- trends
cargo run -p mac-storage -- ui --port 47231
```

`cargo audit` is not part of CI. Run it locally if you want advisory output. Do not fail a release on an advisory that cannot be fixed without abandoning a required crate.

## Layout

```text
apps/cli/          mac-storage binary
crates/common/     domain types and PRODUCT_NAME
crates/scanner/    walk and exclusions
crates/storage/    SQLite and migrations
crates/duplicates/ BLAKE3 grouping over stored files
crates/analyze/    suggestions from a stored scan
docs/              design and status
```

## Database during development

The default database is the platform data directory from the `directories` crate (`…/mac-storage-advisor/mac-storage.sqlite`). Set `MAC_STORAGE_DB` or pass `--db` to keep experiments out of that directory. Tests always set `MAC_STORAGE_DB` to a file inside a temporary fixture.

## Permission tests

Unreadable directories are simulated with mode `000` on a fixture directory. `lstat` of a mode-`000` file that you own still succeeds, so a mode-`000` file is not a useful "unreadable metadata" fixture. If the process is root, mode `000` may still be readable. The test checks `read_dir` and skips the filesystem assertion in that case. A unit test still feeds a permission error and a deleted path through the same entry handler the walker uses, and asserts the later file is counted.

## Safety of the test suite

Fixtures are created under `std::env::temp_dir()`. No test calls `scan_path` on `/`, `/usr`, `/System`, or `/Library`. Protected-root refusal is covered by `validate_root_policy` with a path value and no I/O.

## Clippy

Workspace crates `forbid(unsafe_code)`. Warnings are denied in CI, including tests.
