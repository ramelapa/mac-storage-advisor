# Roadmap

This commit is the scan foundation (v0.1). The product MVP is larger and is split across later releases. Duplicate detection is next and is not started.

## v0.1 — Foundation (this version)

Done:

- Recursive metadata scan
- Exclusions and protected macOS prefix policy
- Non-fatal error collection
- SQLite migrations and persistence
- `mac-storage scan`, including JSON output

Not done in v0.1: hashing, duplicate groups, analyzers, recommendations, trends, history browsing, Trash, and the desktop UI.

## MVP

The MVP is a local macOS app that explains where space went and can suggest moving items to Trash. It stays non-destructive. The MVP does not include AI, cloud sync, cross-device inventory, automatic deletion, photo or video similarity, semantic duplicates, or Windows/Linux product builds.

### v0.2

- Duplicate detection stage 1: group persisted files by logical size
- Stage 2: collapse hard links using inode and device
- Stage 3: sample BLAKE3, then full BLAKE3, then optional byte-for-byte confirmation
- Large-file listing from a stored scan
- `history` command over existing scan rows
- Decide whether `--threads` becomes a bounded pool or stays at one thread

### v0.3

- Stale files, Downloads, and developer-artifact analyzers
- Recommendation engine that only emits suggestions
- Trends across scan totals
- Tauri UI on the same Rust core
- Move to Trash as the only remediation

## Future / Deferred

- AI-assisted suggestions
- Cloud sync and cross-device inventory
- Automatic deletion (will not be implemented)
- Permanent delete (will not exist)
- Photo/video similarity and semantic duplicates
- Windows and Linux as packaged products
- Live filesystem watching

See [features.md](features.md) for the authoritative status of each item.
