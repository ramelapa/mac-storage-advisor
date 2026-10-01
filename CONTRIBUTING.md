# Contributing

Mac Storage Advisor is local-first and non-destructive. Changes that weaken those rules need an ADR update, not a quiet code path.

## Setup

Install Rust with the toolchain in `rust-toolchain.toml` (1.98.1), including `rustfmt` and `clippy`.

```bash
cargo test --workspace
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

See [docs/development.md](docs/development.md) for the command list.

## Tests

Use synthetic directories under the process temp dir. Do not point tests at a home directory, `/`, `/System`, `/usr`, `/Library`, `/private`, `/bin`, or `/sbin`. Protected-root behavior is tested with pure path checks, not by walking those trees.

`chmod 000` does not deny access when the test process is privileged. The permission test detects that and skips the filesystem assertion. Error aggregation is still covered by a unit test that does not depend on mode bits.

## Scope of this repository

Scan, duplicate grouping, large-file listing, history, suggestion review, `mac-storage ui`, user-confirmed Trash, `doctor`, `mac-storage window`, and the macOS app bundle are implemented. `blake3` is a dependency of `crates/duplicates`. `tiny_http` serves the localhost page. `tauri` opens that page in a native window. The `trash` crate moves files to the OS Trash and does not permanently delete them. Do not add `notify` or `rayon` until that roadmap item is actually being built. Linux CI installs the WebKit development libraries before `cargo test`.

Do not add a permanent-delete API. See [docs/safety-model.md](docs/safety-model.md).

## Pull requests

- Keep commits focused.
- Update [CHANGELOG.md](CHANGELOG.md) and [docs/features.md](docs/features.md) when a feature's status changes.
- Do not mark a feature Done unless a test demonstrates it.
- `cargo fmt` and `cargo clippy --workspace --all-targets --all-features -- -D warnings` must pass.

## License

Contributions are dual-licensed under MIT OR Apache-2.0. See [LICENSE](LICENSE).
