# Roadmap

v0.1 is the scan foundation. v0.2 adds duplicate grouping, large files, and history. v0.3 adds review suggestions. The desktop UI and Trash are still later.

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

Done:

- Duplicate detection: size group, then inode/hard link, then sample BLAKE3, then full BLAKE3, then optional byte comparison
- Large-file listing from a stored scan
- `history` command over existing scan rows
- `--threads` stays a recorded value. Execution remains one thread.

### v0.3

Done:

- Stale files, Downloads, and developer-artifact analyzers
- Recommendation engine that only emits suggestions
- Trends across scan totals of the same folder

Still planned in this release line:

- Tauri UI on the same Rust core
- Move to Trash as the only remediation
- iCloud placeholder handling and APFS clone awareness

## Future / Deferred

- AI-assisted suggestions
- Cloud sync and cross-device inventory
- Automatic deletion (will not be implemented)
- Permanent delete (will not exist)
- Photo/video similarity and semantic duplicates
- Windows and Linux as packaged products
- Live filesystem watching

See [features.md](features.md) for the authoritative status of each item.
