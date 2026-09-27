//! Local SQLite store for scan metadata.
//!
//! The database lives under the platform data directory from the `directories`
//! crate, unless `MAC_STORAGE_DB` or an explicit path is set. Migrations run
//! on open. This crate does not delete user files; it only writes the database.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mac_storage_common::{
    DirectoryRecord, FileKind, FileRecord, Scan, ScanErrorRecord, ScanSnapshot, ScanStatistics,
    ScanStatus, ScanTarget, DB_ENV_VAR, PRODUCT_NAME, PRODUCT_VERSION,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

const MIGRATIONS: &[(i64, &str)] = &[(1, include_str!("../migrations/001_init.sql"))];

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid database path")]
    InvalidPath,
    #[error("could not resolve a platform data directory")]
    NoDataDir,
    #[error("value {value} does not fit in a sqlite integer")]
    ValueOutOfRange { value: u64 },
    #[error("scan {0} was not found")]
    NotFound(i64),
    #[error("stored scan status is invalid: {0}")]
    InvalidStatus(String),
    #[error("stored file type is invalid: {0}")]
    InvalidFileType(String),
    #[error("refusing to open a SQLite file that is not a {product} database")]
    ForeignDatabase { product: &'static str },
}

pub struct Database {
    conn: Connection,
}

/// Header plus rows loaded back from SQLite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedScan {
    pub scan: Scan,
    pub files: Vec<FileRecord>,
    pub directories: Vec<DirectoryRecord>,
    pub errors: Vec<ScanErrorRecord>,
    pub exclusions: Vec<String>,
}

pub fn resolve_db_path(flag: Option<&Path>) -> Result<PathBuf, StorageError> {
    let env = std::env::var(DB_ENV_VAR).ok();
    resolve_db_path_from(flag, env.as_deref())
}

pub fn resolve_db_path_from(
    flag: Option<&Path>,
    env: Option<&str>,
) -> Result<PathBuf, StorageError> {
    if let Some(path) = flag {
        if path.as_os_str().is_empty() {
            return Err(StorageError::InvalidPath);
        }
        return Ok(path.to_path_buf());
    }
    if let Some(value) = env {
        if !value.is_empty() {
            return Ok(PathBuf::from(value));
        }
    }
    default_db_path()
}

pub fn default_db_path() -> Result<PathBuf, StorageError> {
    let dirs = directories::ProjectDirs::from("com", "mac-storage", "mac-storage-advisor")
        .ok_or(StorageError::NoDataDir)?;
    Ok(dirs.data_dir().join("mac-storage.sqlite"))
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(path)?;
        Self::from_connection(conn)
    }

    pub fn open_in_memory() -> Result<Self, StorageError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self, StorageError> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(Duration::from_secs(5))?;
        let db = Self { conn };
        db.assert_compatible()?;
        db.migrate()?;
        db.ensure_settings()?;
        Ok(db)
    }

    /// A new file is ours. An existing file must already be this product's
    /// database. Anything else is left untouched so a bad `--db` path cannot
    /// migrate another application's SQLite file.
    fn assert_compatible(&self) -> Result<(), StorageError> {
        let names = user_tables(&self.conn)?;
        if names.is_empty() {
            return Ok(());
        }
        if let Some(product) = setting_if_present(&self.conn, &names, "product_name")? {
            if product == PRODUCT_NAME {
                return Ok(());
            }
            return Err(StorageError::ForeignDatabase {
                product: PRODUCT_NAME,
            });
        }
        if names.iter().any(|name| name == "scans") && names.iter().any(|name| name == "files") {
            return Ok(());
        }
        Err(StorageError::ForeignDatabase {
            product: PRODUCT_NAME,
        })
    }

    pub fn migrate(&self) -> Result<(), StorageError> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at INTEGER NOT NULL
            );",
        )?;
        let current: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?;
        for (version, sql) in MIGRATIONS {
            if *version <= current {
                continue;
            }
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
            tx.execute(
                "INSERT INTO schema_migrations(version, applied_at) VALUES (?1, ?2)",
                params![version, now_millis()],
            )?;
            tx.commit()?;
        }
        Ok(())
    }

    fn ensure_settings(&self) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT INTO settings(key, value) VALUES ('product_name', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![PRODUCT_NAME],
        )?;
        self.conn.execute(
            "INSERT INTO settings(key, value) VALUES ('schema_version', '1')
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [],
        )?;
        Ok(())
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>, StorageError> {
        let value = self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value)
    }

    pub fn save_scan(&mut self, snap: &ScanSnapshot) -> Result<Scan, StorageError> {
        let tx = self.conn.transaction()?;
        let scan_id = insert_scan(&tx, snap)?;
        insert_files(&tx, scan_id, &snap.files)?;
        insert_directories(&tx, scan_id, &snap.directories)?;
        insert_errors(&tx, scan_id, &snap.errors)?;
        insert_exclusions(&tx, scan_id, &snap.exclusions)?;
        tx.commit()?;
        Ok(snap.to_scan(scan_id, PRODUCT_NAME, PRODUCT_VERSION))
    }

    pub fn load_scan(&self, id: i64) -> Result<LoadedScan, StorageError> {
        let scan = load_scan_row(&self.conn, id)?;
        let files = load_files(&self.conn, id)?;
        let directories = load_directories(&self.conn, id)?;
        let errors = load_errors(&self.conn, id)?;
        let exclusions = load_exclusions(&self.conn, id)?;
        Ok(LoadedScan {
            scan,
            files,
            directories,
            errors,
            exclusions,
        })
    }
}

fn insert_scan(tx: &Transaction<'_>, snap: &ScanSnapshot) -> Result<i64, StorageError> {
    let stats = &snap.statistics;
    tx.execute(
        "INSERT INTO scans (
            root_path, started_at, finished_at, status,
            directories_scanned, files_scanned, logical_bytes, allocated_bytes,
            allocated_bytes_complete, symlink_count, other_count, error_count,
            skipped_count, files_below_min_size, elapsed_ms, min_logical_size,
            threads_requested, product_name, product_version
        ) VALUES (
            ?1, ?2, ?3, ?4,
            ?5, ?6, ?7, ?8,
            ?9, ?10, ?11, ?12,
            ?13, ?14, ?15, ?16,
            ?17, ?18, ?19
        )",
        params![
            path_string(&snap.root),
            time_to_millis(Some(snap.started_at)).unwrap_or(0),
            time_to_millis(Some(snap.finished_at)).unwrap_or(0),
            snap.status.as_str(),
            req_i64(stats.directories_scanned)?,
            req_i64(stats.files_scanned)?,
            req_i64(stats.logical_bytes)?,
            req_i64(stats.allocated_bytes)?,
            i64::from(stats.allocated_bytes_complete),
            req_i64(stats.symlinks)?,
            req_i64(stats.other_entries)?,
            req_i64(stats.errors)?,
            req_i64(stats.skipped)?,
            req_i64(stats.files_below_min_size)?,
            req_i64(stats.elapsed_ms)?,
            req_i64(snap.min_logical_size)?,
            i64::from(snap.threads_requested),
            PRODUCT_NAME,
            PRODUCT_VERSION,
        ],
    )?;
    Ok(tx.last_insert_rowid())
}

fn insert_files(
    tx: &Transaction<'_>,
    scan_id: i64,
    files: &[FileRecord],
) -> Result<(), StorageError> {
    let mut stmt = tx.prepare(
        "INSERT INTO files (
            scan_id, path, filename, extension, logical_size, allocated_size,
            created_time, modified_time, accessed_time, inode, device_id,
            permissions, file_type, is_symlink, is_broken_symlink, link_target
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6,
            ?7, ?8, ?9, ?10, ?11,
            ?12, ?13, ?14, ?15, ?16
        )",
    )?;
    for file in files {
        if file.kind == FileKind::Directory {
            return Err(StorageError::InvalidFileType(
                "directory rows belong in the directories table".into(),
            ));
        }
        stmt.execute(params![
            scan_id,
            path_string(&file.path),
            file.filename,
            file.extension,
            req_i64(file.logical_size)?,
            opt_fit(file.allocated_size),
            time_to_millis(file.created),
            time_to_millis(file.modified),
            time_to_millis(file.accessed),
            opt_fit(file.inode),
            opt_fit(file.device_id),
            file.permissions.map(i64::from),
            file.kind.as_str(),
            i64::from(file.is_symlink),
            i64::from(file.is_broken_symlink),
            file.link_target.as_ref().map(|path| path_string(path)),
        ])?;
    }
    Ok(())
}

fn insert_directories(
    tx: &Transaction<'_>,
    scan_id: i64,
    directories: &[DirectoryRecord],
) -> Result<(), StorageError> {
    let mut stmt = tx.prepare(
        "INSERT INTO directories (
            scan_id, path, filename, logical_size, allocated_size,
            created_time, modified_time, accessed_time, inode, device_id, permissions
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
    )?;
    for directory in directories {
        stmt.execute(params![
            scan_id,
            path_string(&directory.path),
            directory.filename,
            req_i64(directory.logical_size)?,
            opt_fit(directory.allocated_size),
            time_to_millis(directory.created),
            time_to_millis(directory.modified),
            time_to_millis(directory.accessed),
            opt_fit(directory.inode),
            opt_fit(directory.device_id),
            directory.permissions.map(i64::from),
        ])?;
    }
    Ok(())
}

fn insert_errors(
    tx: &Transaction<'_>,
    scan_id: i64,
    errors: &[ScanErrorRecord],
) -> Result<(), StorageError> {
    let mut stmt =
        tx.prepare("INSERT INTO scan_errors(scan_id, path, message) VALUES (?1, ?2, ?3)")?;
    for err in errors {
        stmt.execute(params![
            scan_id,
            err.path.as_ref().map(|path| path_string(path)),
            err.message,
        ])?;
    }
    Ok(())
}

fn insert_exclusions(
    tx: &Transaction<'_>,
    scan_id: i64,
    exclusions: &[String],
) -> Result<(), StorageError> {
    let mut stmt = tx.prepare("INSERT INTO exclusions(scan_id, pattern) VALUES (?1, ?2)")?;
    for pattern in exclusions {
        stmt.execute(params![scan_id, pattern])?;
    }
    Ok(())
}

fn load_scan_row(conn: &Connection, id: i64) -> Result<Scan, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT root_path, started_at, finished_at, status,
                directories_scanned, files_scanned, logical_bytes, allocated_bytes,
                allocated_bytes_complete, symlink_count, other_count, error_count,
                skipped_count, files_below_min_size, elapsed_ms, min_logical_size,
                threads_requested, product_name, product_version
         FROM scans WHERE id = ?1",
    )?;
    let mut rows = stmt.query(params![id])?;
    let Some(row) = rows.next()? else {
        return Err(StorageError::NotFound(id));
    };
    let status_raw: String = row.get(3)?;
    let status =
        ScanStatus::from_db(&status_raw).ok_or_else(|| StorageError::InvalidStatus(status_raw))?;
    let threads: i64 = row.get(16)?;
    let stats = ScanStatistics {
        directories_scanned: row_u64(row, 4)?,
        files_scanned: row_u64(row, 5)?,
        logical_bytes: row_u64(row, 6)?,
        allocated_bytes: row_u64(row, 7)?,
        allocated_bytes_complete: row.get::<_, i64>(8)? != 0,
        symlinks: row_u64(row, 9)?,
        other_entries: row_u64(row, 10)?,
        errors: row_u64(row, 11)?,
        skipped: row_u64(row, 12)?,
        files_below_min_size: row_u64(row, 13)?,
        elapsed_ms: row_u64(row, 14)?,
    };
    let root: String = row.get(0)?;
    let min_logical_size = row_u64(row, 15)?;
    Ok(Scan {
        id,
        target: ScanTarget {
            root: PathBuf::from(root),
            exclusions: Vec::new(),
            min_logical_size,
            threads: u32::try_from(threads).unwrap_or(u32::MAX),
            allow_protected_roots: false,
            redact_paths: false,
        },
        started_at: millis_to_time(row.get(1)?).unwrap_or(UNIX_EPOCH),
        finished_at: millis_to_time(row.get(2)?).unwrap_or(UNIX_EPOCH),
        status,
        statistics: stats,
        product_name: row.get(17)?,
        product_version: row.get(18)?,
    })
}

fn load_files(conn: &Connection, scan_id: i64) -> Result<Vec<FileRecord>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT path, filename, extension, logical_size, allocated_size,
                created_time, modified_time, accessed_time, inode, device_id,
                permissions, file_type, is_symlink, is_broken_symlink, link_target
         FROM files WHERE scan_id = ?1 ORDER BY path",
    )?;
    let mut rows = stmt.query(params![scan_id])?;
    let mut files = Vec::new();
    while let Some(row) = rows.next()? {
        let kind_raw: String = row.get(11)?;
        let kind =
            FileKind::from_db(&kind_raw).ok_or_else(|| StorageError::InvalidFileType(kind_raw))?;
        let link: Option<String> = row.get(14)?;
        files.push(FileRecord {
            path: PathBuf::from(row.get::<_, String>(0)?),
            filename: row.get(1)?,
            extension: row.get(2)?,
            logical_size: row_u64(row, 3)?,
            allocated_size: opt_u64(row.get(4)?)?,
            created: millis_to_time(row.get(5)?),
            modified: millis_to_time(row.get(6)?),
            accessed: millis_to_time(row.get(7)?),
            inode: opt_u64(row.get(8)?)?,
            device_id: opt_u64(row.get(9)?)?,
            permissions: opt_u32(row.get(10)?)?,
            kind,
            is_symlink: row.get::<_, i64>(12)? != 0,
            is_broken_symlink: row.get::<_, i64>(13)? != 0,
            link_target: link.map(PathBuf::from),
        });
    }
    Ok(files)
}

fn load_directories(conn: &Connection, scan_id: i64) -> Result<Vec<DirectoryRecord>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT path, filename, logical_size, allocated_size, created_time,
                modified_time, accessed_time, inode, device_id, permissions
         FROM directories WHERE scan_id = ?1 ORDER BY path",
    )?;
    let mut rows = stmt.query(params![scan_id])?;
    let mut directories = Vec::new();
    while let Some(row) = rows.next()? {
        directories.push(DirectoryRecord {
            path: PathBuf::from(row.get::<_, String>(0)?),
            filename: row.get(1)?,
            logical_size: row_u64(row, 2)?,
            allocated_size: opt_u64(row.get(3)?)?,
            created: millis_to_time(row.get(4)?),
            modified: millis_to_time(row.get(5)?),
            accessed: millis_to_time(row.get(6)?),
            inode: opt_u64(row.get(7)?)?,
            device_id: opt_u64(row.get(8)?)?,
            permissions: opt_u32(row.get(9)?)?,
        });
    }
    Ok(directories)
}

fn load_errors(conn: &Connection, scan_id: i64) -> Result<Vec<ScanErrorRecord>, StorageError> {
    let mut stmt =
        conn.prepare("SELECT path, message FROM scan_errors WHERE scan_id = ?1 ORDER BY id")?;
    let rows = stmt.query_map(params![scan_id], |row| {
        let path: Option<String> = row.get(0)?;
        Ok(ScanErrorRecord {
            path: path.map(PathBuf::from),
            message: row.get(1)?,
        })
    })?;
    let mut errors = Vec::new();
    for row in rows {
        errors.push(row?);
    }
    Ok(errors)
}

fn load_exclusions(conn: &Connection, scan_id: i64) -> Result<Vec<String>, StorageError> {
    let mut stmt = conn.prepare("SELECT pattern FROM exclusions WHERE scan_id = ?1 ORDER BY id")?;
    let rows = stmt.query_map(params![scan_id], |row| row.get(0))?;
    let mut patterns = Vec::new();
    for row in rows {
        patterns.push(row?);
    }
    Ok(patterns)
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn req_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::ValueOutOfRange { value })
}

fn opt_fit(value: Option<u64>) -> Option<i64> {
    value.and_then(|n| i64::try_from(n).ok())
}

fn row_u64(row: &rusqlite::Row<'_>, idx: usize) -> Result<u64, StorageError> {
    let value: i64 = row.get(idx)?;
    u64::try_from(value).map_err(|_| StorageError::ValueOutOfRange { value: u64::MAX })
}

fn opt_u64(value: Option<i64>) -> Result<Option<u64>, StorageError> {
    match value {
        Some(n) if n < 0 => Err(StorageError::ValueOutOfRange { value: u64::MAX }),
        Some(n) => Ok(Some(n as u64)),
        None => Ok(None),
    }
}

fn opt_u32(value: Option<i64>) -> Result<Option<u32>, StorageError> {
    match value {
        Some(n) => u32::try_from(n)
            .map(Some)
            .map_err(|_| StorageError::ValueOutOfRange {
                value: u64::try_from(n).unwrap_or(u64::MAX),
            }),
        None => Ok(None),
    }
}

fn time_to_millis(time: Option<SystemTime>) -> Option<i64> {
    let time = time?;
    let duration = time.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(duration.as_millis()).ok()
}

fn millis_to_time(millis: Option<i64>) -> Option<SystemTime> {
    let millis = millis?;
    if millis < 0 {
        return None;
    }
    Some(UNIX_EPOCH + Duration::from_millis(millis as u64))
}

fn now_millis() -> i64 {
    time_to_millis(Some(SystemTime::now())).unwrap_or(0)
}

fn user_tables(conn: &Connection) -> Result<Vec<String>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
         ORDER BY name",
    )?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    let mut names = Vec::new();
    for row in rows {
        names.push(row?);
    }
    Ok(names)
}

fn setting_if_present(
    conn: &Connection,
    tables: &[String],
    key: &str,
) -> Result<Option<String>, StorageError> {
    if !tables.iter().any(|name| name == "settings") {
        return Ok(None);
    }
    match conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |row| row.get(0),
    ) {
        Ok(value) => Ok(Some(value)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_db() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "mac-storage-db-{}-{}-{}.sqlite",
            std::process::id(),
            n,
            nanos
        ))
    }

    fn sample_snapshot() -> ScanSnapshot {
        let modified = UNIX_EPOCH + Duration::from_millis(1_700_000_000_123);
        ScanSnapshot {
            root: PathBuf::from("/tmp/fixture"),
            started_at: UNIX_EPOCH + Duration::from_millis(1_700_000_000_000),
            finished_at: UNIX_EPOCH + Duration::from_millis(1_700_000_000_050),
            status: ScanStatus::CompletedWithErrors,
            statistics: ScanStatistics {
                directories_scanned: 1,
                files_scanned: 2,
                logical_bytes: 11,
                allocated_bytes: 4096,
                allocated_bytes_complete: true,
                symlinks: 1,
                other_entries: 0,
                errors: 1,
                skipped: 0,
                files_below_min_size: 1,
                elapsed_ms: 50,
            },
            files: vec![
                FileRecord {
                    path: PathBuf::from("/tmp/fixture/notes.txt"),
                    filename: "notes.txt".into(),
                    extension: Some("txt".into()),
                    logical_size: 11,
                    allocated_size: Some(4096),
                    created: None,
                    modified: Some(modified),
                    accessed: Some(modified),
                    inode: Some(42),
                    device_id: Some(7),
                    permissions: Some(0o644),
                    kind: FileKind::File,
                    is_symlink: false,
                    is_broken_symlink: false,
                    link_target: None,
                },
                FileRecord {
                    path: PathBuf::from("/tmp/fixture/broken"),
                    filename: "broken".into(),
                    extension: None,
                    logical_size: 8,
                    allocated_size: Some(0),
                    created: None,
                    modified: Some(modified),
                    accessed: None,
                    inode: Some(43),
                    device_id: Some(7),
                    permissions: Some(0o777),
                    kind: FileKind::Symlink,
                    is_symlink: true,
                    is_broken_symlink: true,
                    link_target: Some(PathBuf::from("missing")),
                },
            ],
            directories: vec![DirectoryRecord {
                path: PathBuf::from("/tmp/fixture"),
                filename: "fixture".into(),
                logical_size: 128,
                allocated_size: Some(4096),
                created: None,
                modified: Some(modified),
                accessed: Some(modified),
                inode: Some(10),
                device_id: Some(7),
                permissions: Some(0o755),
            }],
            errors: vec![ScanErrorRecord {
                path: Some(PathBuf::from("/tmp/fixture/locked")),
                message: "permission denied".into(),
            }],
            exclusions: vec!["skip".into()],
            threads_requested: 4,
            min_logical_size: 5,
            allow_protected_roots: false,
            redact_paths: false,
        }
    }

    #[test]
    fn scan_and_file_rows_round_trip() {
        let mut db = Database::open_in_memory().unwrap();
        assert_eq!(
            db.setting("product_name").unwrap().as_deref(),
            Some(PRODUCT_NAME)
        );
        assert_eq!(db.setting("schema_version").unwrap().as_deref(), Some("1"));

        let snap = sample_snapshot();
        let scan = db.save_scan(&snap).unwrap();
        assert!(scan.id > 0);
        assert_eq!(scan.product_name, PRODUCT_NAME);

        let loaded = db.load_scan(scan.id).unwrap();
        assert_eq!(loaded.scan.statistics, snap.statistics);
        assert_eq!(loaded.scan.status, ScanStatus::CompletedWithErrors);
        assert_eq!(loaded.scan.target.root, snap.root);
        assert_eq!(loaded.scan.target.min_logical_size, 5);
        assert_eq!(loaded.scan.target.threads, 4);
        assert_eq!(loaded.files.len(), 2);
        assert_eq!(loaded.directories.len(), 1);
        assert_eq!(loaded.errors.len(), 1);
        assert_eq!(loaded.exclusions, vec!["skip".to_owned()]);

        let notes = loaded
            .files
            .iter()
            .find(|file| file.filename == "notes.txt")
            .unwrap();
        assert_eq!(notes.logical_size, 11);
        assert_eq!(notes.extension.as_deref(), Some("txt"));
        assert_eq!(notes.allocated_size, Some(4096));
        assert_eq!(notes.inode, Some(42));
        assert_eq!(notes.device_id, Some(7));
        assert_eq!(notes.permissions, Some(0o644));
        assert_eq!(notes.kind, FileKind::File);
        assert_eq!(notes.modified, snap.files[0].modified);
        assert!(notes.created.is_none());

        let link = loaded
            .files
            .iter()
            .find(|file| file.filename == "broken")
            .unwrap();
        assert!(link.is_symlink);
        assert!(link.is_broken_symlink);
        assert_eq!(link.link_target.as_deref(), Some(Path::new("missing")));
        assert_eq!(link.kind, FileKind::Symlink);

        let directory = &loaded.directories[0];
        assert_eq!(directory.filename, "fixture");
        assert_eq!(directory.logical_size, 128);
        assert_eq!(loaded.errors[0].message, "permission denied");
        assert_eq!(
            loaded.errors[0].path.as_deref(),
            Some(Path::new("/tmp/fixture/locked"))
        );
    }

    #[test]
    fn file_database_survives_reopen() {
        let path = temp_db();
        let snap = sample_snapshot();
        let id = {
            let mut db = Database::open(&path).unwrap();
            db.save_scan(&snap).unwrap().id
        };
        let db = Database::open(&path).unwrap();
        let loaded = db.load_scan(id).unwrap();
        assert_eq!(loaded.files.len(), 2);
        assert_eq!(loaded.scan.statistics.files_scanned, 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn migration_is_idempotent_and_foreign_keys_hold() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();
        let err = db
            .conn
            .execute(
                "INSERT INTO files(scan_id, path, filename, logical_size, file_type, is_symlink, is_broken_symlink)
                 VALUES (999, '/nope', 'nope', 0, 'file', 0, 0)",
                [],
            )
            .unwrap_err();
        let message = err.to_string();
        assert!(
            message.to_ascii_lowercase().contains("foreign")
                || message.to_ascii_lowercase().contains("constraint"),
            "{message}"
        );
    }

    #[test]
    fn refuses_to_migrate_an_unrelated_sqlite_file() {
        let path = temp_db();
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE notes (id INTEGER PRIMARY KEY);")
                .unwrap();
        }
        let opened = Database::open(&path);
        assert!(matches!(opened, Err(StorageError::ForeignDatabase { .. })));
        let conn = rusqlite::Connection::open(&path).unwrap();
        let names = super::user_tables(&conn).unwrap();
        assert_eq!(names, vec!["notes".to_owned()]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_scan_is_not_found() {
        let db = Database::open_in_memory().unwrap();
        let err = db.load_scan(42).unwrap_err();
        assert!(matches!(err, StorageError::NotFound(42)));
    }

    #[test]
    fn db_path_resolution_prefers_flag_then_env() {
        let flag = Path::new("/tmp/explicit.sqlite");
        let resolved = resolve_db_path_from(Some(flag), Some("/tmp/env.sqlite")).unwrap();
        assert_eq!(resolved, flag);
        let from_env = resolve_db_path_from(None, Some("/tmp/env.sqlite")).unwrap();
        assert_eq!(from_env, PathBuf::from("/tmp/env.sqlite"));
        let default_path = resolve_db_path_from(None, Some("")).unwrap();
        assert!(default_path.ends_with("mac-storage.sqlite"));
        assert!(resolve_db_path_from(Some(Path::new("")), None).is_err());
    }
}
