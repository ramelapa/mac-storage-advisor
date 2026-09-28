-- iCloud placeholder bit, and a local record of paths moved to Trash.
ALTER TABLE files ADD COLUMN is_dataless INTEGER NOT NULL DEFAULT 0
    CHECK (is_dataless IN (0, 1));

CREATE TABLE trash_events (
    id INTEGER PRIMARY KEY,
    scan_id INTEGER NOT NULL REFERENCES scans(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    moved_at INTEGER NOT NULL
);

CREATE INDEX trash_events_scan_id ON trash_events(scan_id);
