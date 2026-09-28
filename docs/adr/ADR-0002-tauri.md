# ADR-0002: Tauri for the future desktop UI

- Status: Accepted (not implemented). v0.4 ships a localhost page first; see [ADR-0006](ADR-0006-local-ui.md).
- Date: 2026-09-27

## Context

The MVP includes a desktop UI. The scan core has to stay testable without a window. This iteration ships only the CLI.

## Decision

Use Tauri for the desktop UI when that work starts. The UI will call the existing scanner and storage crates. Tauri is not a workspace dependency in v0.1, and CI does not build a frontend.

## Consequences

- The CLI and the future UI share exclusion rules, the safety model, and the SQLite schema.
- Frontend work can wait until scans and reports are trustworthy.
- Tauri adds a system webview and a JavaScript shell later. That cost is deferred.

## Alternatives considered

- **SwiftUI.** Native macOS, but splits the product into two languages before the scan behavior is finished.
- **Electron.** Heavier runtime for a tool whose job is to be careful with local files.
- **CLI only.** Enough for v0.1. The MVP still calls for a UI, so "never a GUI" was rejected.
