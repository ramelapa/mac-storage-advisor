# Safety model

This version scans, hashes, and can move a user-confirmed path to the OS Trash. It does not permanently delete files.

## What this version does

- Reads directory entries and metadata (`lstat`, and `readlink` for symlinks).
- Optionally `stat`s a symlink target only to see whether the link is broken. That does not list the target directory.
- Writes a SQLite database of metadata under the platform data directory, or under `MAC_STORAGE_DB` / `--db`.
- Prints a summary. `--json` includes error paths and messages.
- `duplicates` opens regular files that a scan already stored, hashes them with BLAKE3, and can re-read them when `--verify` is set. The hash is stored. The bytes are not.
- `analyze` and `recommendations` read stored metadata and duplicate groups. They print suggestions. They do not move files.
- `mac-storage ui` serves that same behavior on `127.0.0.1`. The page's command box only accepts advisor verbs. It does not invoke a shell.
- `trash` moves a path to the operating-system Trash when the confirmation phrase is exactly `move to trash`. Without that phrase, nothing is moved.

## What this version does not do

- `scan` does not open files to read contents.
- It does not upload paths, names, hashes, or contents.
- It does not follow symlinks during the walk, and it does not hash a symlink as if it were the target.
- It does not permanently delete or rename files, and it has no `rm` integration.
- It does not move a path that the scan did not record, the scan folder itself, a protected macOS path, or an iCloud placeholder.
- The local page does not listen on any address other than `127.0.0.1`.

## Permanent delete

Permanent delete will not exist. `trash` moves a user-confirmed path to the operating-system Trash. Trash is reversible by the OS until the user empties it. Automatic deletion will not be implemented.

There is no boolean force flag. The confirmation text has to be `move to trash`. `--allow-protected-roots` only allows a scan of an exact protected prefix. It is a test override, not a deletion bypass, and Trash still refuses protected paths unless the scan was explicitly rooted inside that prefix as a project.

## Errors

Permission denied, a path that disappears during the walk, an unreadable directory, a special file, or a broken symlink does not abort the scan. The problem is stored as a path plus a message. The message is the I/O error string, not file bytes.

## Tests

Tests build fixtures in a temporary directory. They do not walk `/System`, `/usr`, `/Library`, or other protected prefixes. The refusal rule for those prefixes is a pure function of the path.
