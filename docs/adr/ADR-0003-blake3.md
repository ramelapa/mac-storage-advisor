# ADR-0003: BLAKE3 for future content identity

- Status: Accepted (implemented in v0.2)
- Date: 2026-09-27

## Context

Duplicate detection needs a content hash after size and inode checks. v0.1 must not read file contents, so the hash crate should not appear until that pass is built.

## Decision

Use BLAKE3 for sample and full-file hashes in the duplicate pipeline described in `docs/duplicate-detection.md`. Do not add the `blake3` crate in the scan-foundation release.

## Consequences

- Hashing can stream, which matters for large files.
- The algorithm is fixed before two implementations choose different hashes.
- The scanner still has no read path for contents. `crates/duplicates` streams BLAKE3 during `mac-storage duplicates`. Migration 002 stores the hashes.

## Alternatives considered

- **SHA-256.** Ubiquitous and slower for bulk hashing. Fine for integrity, not the best default for grouping terabytes of local files.
- **xxHash or similar.** Fast and not a cryptographic identity. A collision would merge unrelated files. Rejected for the full-file step.
- **Hash in v0.1.** Rejected. The foundation's tests prove contents are not persisted. Adding a hash now would blur that boundary.
