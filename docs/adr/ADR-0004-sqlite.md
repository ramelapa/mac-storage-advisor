# ADR-0004: SQLite for scan metadata

- Status: Accepted
- Date: 2026-09-27

## Context

Scan results have to survive the process, support later history and duplicate queries, and stay on the machine. The first schema only needs the tables the scanner fills today.

## Decision

Store metadata in SQLite through `rusqlite` with the `bundled` feature. Migrations start at version 1 and are applied on open. The file lives in the platform data directory from the `directories` crate, overridable with `--db` or `MAC_STORAGE_DB`.

Tables created now: `scans`, `files`, `directories`, `settings`, `exclusions`, and `scan_errors`. Indexes cover path, logical size, scan id, and modified time.

## Consequences

- CI does not need a system SQLite library.
- Schema changes are explicit SQL files, not ad hoc `CREATE TABLE IF NOT EXISTS` in the query path.
- The scan is written in one transaction after the walk. Very large trees will need streaming inserts later.
- Querying duplicates later can use SQL over size and inode without a second database.

## Alternatives considered

- **JSON files.** Easy to inspect and awkward to index by size or path as history grows.
- **sled or another embedded KV store.** Extra operational model, weaker ad hoc queries for a future history view.
- **Server database.** Rejected. The product is local-first and must run without a daemon.
