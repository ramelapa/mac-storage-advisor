# Roadmap

v0.1 is the scan foundation. v0.2 adds duplicate grouping, large files, and history. v0.3 adds review suggestions. v0.4 adds a localhost page. v0.5 adds placeholder and extent notes, plus a confirmed move to Trash. v0.6 checks the database and opens that page in a window. v0.7 groups stored files into folder totals. v0.8 packages that window as a Mac app. v0.8.1 makes a long duplicate list easier to review. v0.9 shows a home-folder scan in progress and where that space sits.

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

### v0.4

Done:

- `mac-storage ui` on `127.0.0.1`, default port 47231
- Interactive results and a command-line view in the same page, using the same database

### v0.5

Done:

- macOS `SF_DATALESS` is stored and excluded from local stale and Downloads totals
- Allocated-below-logical gaps are reported and are not called reclaimable space
- `mac-storage trash` moves named paths to the OS Trash only after the phrase `move to trash`

### v0.6

Done:

- `mac-storage doctor` checks product identity, schema version, and SQLite integrity, and lists recent Trash moves
- `mac-storage window` opens the existing local page in a Tauri window on `127.0.0.1`

### v0.7

Done:

- `mac-storage folders` rolls stored regular files into folder totals
- Placeholder bytes stay separate from local bytes
- Directory inode sizes stay on the directory row and are not added to the rollup

### v0.8

Done:

- `scripts/build-mac-app.sh` builds `Mac Storage Advisor.app` and a disk image on macOS
- Opening the app starts the existing local window. A subcommand on that binary stays a command
- The app is ad-hoc signed, not sandboxed, and uses the same database as the command line

### v0.8.1

Done:

- Interactive results are split into Overview, Folders, Largest, Duplicates, Suggestions, History, Trash, and Database
- Duplicate groups can be filtered, sorted, grouped by folder or type, and paged. A group stays collapsed until it is opened
- Listing extra copies adds them to the Trash list and does not move them

### v0.9

Done:

- Scan this Mac walks the home folder. System folders such as `/System` are not part of that walk
- The overview shows a moving progress bar plus the current folder, file count, and logical bytes while the scan runs
- After the scan, bars compare the largest top-level folders and mark unchanged files, developer folders, Library, and Downloads as worth a look

## Future / Deferred

- AI-assisted suggestions
- Cloud sync and cross-device inventory
- Automatic deletion (will not be implemented)
- Permanent delete (will not exist)
- Photo/video similarity and semantic duplicates
- Windows and Linux as packaged products
- Live filesystem watching

See [features.md](features.md) for the authoritative status of each item.
