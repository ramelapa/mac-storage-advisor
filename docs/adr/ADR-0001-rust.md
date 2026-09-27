# ADR-0001: Rust for the scan core

- Status: Accepted
- Date: 2026-09-27

## Context

The product scans large directory trees and must be deterministic about metadata, symlink policy, and error handling. A later desktop UI should call the same core. The first milestone is a CLI, not a GUI.

## Decision

Implement scanning, domain types, persistence, and the CLI in Rust. Workspace crates forbid `unsafe`. Filesystem behavior uses the standard library and `walkdir`.

## Consequences

- Metadata and exclusion rules are typed and tested the same way on Linux CI and macOS.
- A future Tauri UI can link the same crates. SwiftUI would have required a second implementation or an FFI boundary before the core existed.
- Contributors need a Rust toolchain. `rust-toolchain.toml` pins the version CI uses.

## Alternatives considered

- **Swift for the whole app.** Best macOS integration, weaker Linux CI for the scanner, and a harder path to a shared core.
- **Python.** Faster to prototype and easier to follow a symlink by accident. Weaker control over what gets read.
- **Go.** Reasonable, but the planned UI (Tauri) and the existing Rust ecosystem for this workspace favor Rust.
