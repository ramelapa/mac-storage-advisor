-- Initial schema. Later tables (content hashes, duplicate groups, recommendations,
-- trends) are intentionally absent until those features exist.

CREATE TABLE scans (
    id INTEGER PRIMARY KEY,
    root_path TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    finished_at INTEGER NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('completed', 'completed_with_errors')),
    directories_scanned INTEGER NOT NULL,
    files_scanned INTEGER NOT NULL,
    logical_bytes INTEGER NOT NULL,
    allocated_bytes INTEGER NOT NULL,
    allocated_bytes_complete INTEGER NOT NULL CHECK (allocated_bytes_complete IN (0, 1)),
    symlink_count INTEGER NOT NULL,
    other_count INTEGER NOT NULL,
    error_count INTEGER NOT NULL,
    skipped_count INTEGER NOT NULL,
    files_below_min_size INTEGER NOT NULL,
    elapsed_ms INTEGER NOT NULL,
    min_logical_size INTEGER NOT NULL,
    threads_requested INTEGER NOT NULL,
    product_name TEXT NOT NULL,
    product_version TEXT NOT NULL
);

CREATE TABLE files (
    id INTEGER PRIMARY KEY,
    scan_id INTEGER NOT NULL REFERENCES scans(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    filename TEXT NOT NULL,
    extension TEXT,
    logical_size INTEGER NOT NULL,
    allocated_size INTEGER,
    created_time INTEGER,
    modified_time INTEGER,
    accessed_time INTEGER,
    inode INTEGER,
    device_id INTEGER,
    permissions INTEGER,
    file_type TEXT NOT NULL CHECK (file_type IN ('file', 'symlink', 'other')),
    is_symlink INTEGER NOT NULL CHECK (is_symlink IN (0, 1)),
    is_broken_symlink INTEGER NOT NULL CHECK (is_broken_symlink IN (0, 1)),
    link_target TEXT
);

CREATE TABLE directories (
    id INTEGER PRIMARY KEY,
    scan_id INTEGER NOT NULL REFERENCES scans(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    filename TEXT NOT NULL,
    logical_size INTEGER NOT NULL,
    allocated_size INTEGER,
    created_time INTEGER,
    modified_time INTEGER,
    accessed_time INTEGER,
    inode INTEGER,
    device_id INTEGER,
    permissions INTEGER
);

CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE exclusions (
    id INTEGER PRIMARY KEY,
    scan_id INTEGER NOT NULL REFERENCES scans(id) ON DELETE CASCADE,
    pattern TEXT NOT NULL
);

CREATE TABLE scan_errors (
    id INTEGER PRIMARY KEY,
    scan_id INTEGER NOT NULL REFERENCES scans(id) ON DELETE CASCADE,
    path TEXT,
    message TEXT NOT NULL
);

CREATE INDEX idx_files_path ON files(path);
CREATE INDEX idx_files_logical_size ON files(logical_size);
CREATE INDEX idx_files_scan_id ON files(scan_id);
CREATE INDEX idx_files_modified_time ON files(modified_time);

CREATE INDEX idx_directories_path ON directories(path);
CREATE INDEX idx_directories_scan_id ON directories(scan_id);
CREATE INDEX idx_directories_modified_time ON directories(modified_time);

CREATE INDEX idx_exclusions_scan_id ON exclusions(scan_id);
CREATE INDEX idx_scan_errors_scan_id ON scan_errors(scan_id);
