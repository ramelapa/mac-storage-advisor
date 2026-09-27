# ADR-0005: Non-destructive remediation

- Status: Accepted
- Date: 2026-09-27

## Context

A storage advisor that can delete files will eventually delete the wrong one. The useful product still needs a way to act on a suggestion. v0.1 has no action other than writing the scan database.

## Decision

- This version does not delete, move, or trash files.
- Permanent delete will not be added.
- The only future removal mechanism is moving a user-confirmed path to the OS Trash.
- Automatic deletion will not be implemented.
- Recommendations, when they exist, are suggestions. They do not apply themselves.

## Consequences

- The scanner can be run on a home directory without a "dry run" flag. The dry run is the product.
- Trash support waits until suggestions are trustworthy and the UI can show exactly which paths would move.
- Tests do not need a destructive fixture mode.

## Alternatives considered

- **Ship `rm` behind a flag.** Rejected. Flags get scripted.
- **Shred or overwrite.** Rejected. Out of scope and hostile to SSDs and APFS clones.
- **Trash in v0.1.** Rejected. There is nothing to recommend yet, so there is nothing safe to move.
