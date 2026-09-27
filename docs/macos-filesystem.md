# macOS filesystem notes

The product target is macOS and APFS. The scanner is portable enough to test on Linux. Linux is not a packaged product.

## What this version handles

| Object | Behavior |
| --- | --- |
| Regular file | Metadata recorded. Contents not read. |
| Directory | Entered unless excluded. Its own inode size is stored, not a rollup. |
| Symbolic link | Recorded, not followed. Relative `readlink` text is stored as returned. Broken if `stat` of the target returns not-found. A loop returns a kernel error and is not treated as a hang. |
| Symlink as the scan root | Refused. Pass the real directory. |
| Hard link | Each path is a row. Inode and device are stored so a later pass can group them. |
| Sparse file | Logical size and allocated size are stored separately when allocated size exists. |
| FIFO, socket, device | Classified as `other` when `lstat` succeeds. Contents are not read. |
| Protected prefixes | `/System`, `/private`, `/bin`, `/sbin`, `/usr`, `/Library`. See the architecture doc for when they are refused versus skipped. |

## What this version does not interpret

- **APFS clones.** `clonefile` shares extents. `st_blocks` does not tell you how many bytes are unique to one file. This version does not claim reclaimable space.
- **APFS snapshots.** Space can be held by a snapshot after a file is gone. Not visible to this scan.
- **Firmlinks and firmlinked system volumes.** Not specially detected. A path is walked only if the user rooted the scan there and it is not an excluded prefix.
- **Aliases.** Finder aliases are data files, not symlinks. They are recorded as normal files. They are not resolved.
- **iCloud dataless files.** A placeholder can have a logical size and little or no local allocation. This version stores both numbers and does not decide whether the file is evicted.
- **Resource forks and extended attributes.** Not read.
- **Case folding.** APFS default volumes are case-insensitive. Exclusion matching is case-sensitive. `Node_Modules` does not match an exclusion of `node_modules`.
- **Privacy (TCC).** macOS can deny access to Desktop, Documents, Downloads, or Mail even when Unix permissions look open. That denial is a recorded scan error if the OS returns one. This tool does not request Full Disk Access.
- **Birth time on Linux CI.** `Metadata::created` may be `None`. The scanner keeps that `None`. It does not backfill from `mtime`.

## Allocated size

On Unix, allocated size is `st_blocks * 512` (POSIX block units), including macOS. It is omitted on platforms without that field. A small file can have an allocated size of one filesystem block, or zero when the bytes are inline or the file is empty. Those outcomes are stored as reported.
