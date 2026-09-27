-- Duplicate pass. Recommendations and trend tables stay absent.

CREATE TABLE content_hashes (
    scan_id INTEGER NOT NULL REFERENCES scans(id) ON DELETE CASCADE,
    file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    algorithm TEXT NOT NULL,
    sample_hash TEXT,
    full_hash TEXT,
    hashed_bytes INTEGER NOT NULL,
    PRIMARY KEY (scan_id, file_id, algorithm)
);

CREATE TABLE duplicate_groups (
    id INTEGER PRIMARY KEY,
    scan_id INTEGER NOT NULL REFERENCES scans(id) ON DELETE CASCADE,
    logical_size INTEGER NOT NULL,
    full_hash TEXT NOT NULL,
    redundant_bytes INTEGER NOT NULL
);

CREATE TABLE duplicate_members (
    group_id INTEGER NOT NULL REFERENCES duplicate_groups(id) ON DELETE CASCADE,
    file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    hard_link_leader INTEGER NOT NULL CHECK (hard_link_leader IN (0, 1)),
    PRIMARY KEY (group_id, file_id)
);

CREATE INDEX idx_content_hashes_scan_id ON content_hashes(scan_id);
CREATE INDEX idx_duplicate_groups_scan_id ON duplicate_groups(scan_id);
CREATE INDEX idx_duplicate_members_group_id ON duplicate_members(group_id);
