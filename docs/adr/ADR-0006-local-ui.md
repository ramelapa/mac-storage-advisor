# ADR-0006: Localhost page for the interactive UI

- Status: Accepted
- Date: 2026-09-28

## Context

The command line can scan, hash, and suggest. Reading those reports in a terminal is enough for a script, and it is a poor way to browse groups, large files, and suggestions together. [ADR-0002](ADR-0002-tauri.md) still calls for a Tauri window later. That window is a larger dependency than this slice needs.

## Decision

`mac-storage ui` is the UI mode. It listens on `127.0.0.1` only, default port 47231, and serves one HTML page from the same binary. The page has two views:

- Interactive: scan a folder and read large files, duplicates, suggestions, history, and trends.
- Command line: the same verbs (`scan`, `duplicates`, `large-files`, `history`, `analyze`, `recommendations`, `trends`). This box is not a system shell.

Both views call the existing scanner, storage, duplicates, and analyze crates and the database selected by `--db` or `MAC_STORAGE_DB`. Requests whose Host header is not `127.0.0.1` or `localhost` on that port are refused. Nothing is uploaded or deleted.

## Consequences

- One binary, two run modes, one SQLite file.
- The page handles one request at a time, matching the single-threaded scan and hash passes.
- Tauri remains planned. This page does not replace that decision.
- CI stays `cargo fmt`, `clippy -D warnings`, and `cargo test`. There is no frontend build.

## Alternatives considered

- **Tauri now.** Still the planned desktop shell. It adds a system webview before the reports are easy to read.
- **A second web application.** A separate Node or Next.js app would not share the Rust scan path as directly, and it would be a second thing to run.
- **Shelling out from the page to `mac-storage`.** The page would depend on `PATH` and a second process. Calling the library functions keeps one database session and the same error text.
