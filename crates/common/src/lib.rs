//! Shared domain types for the local storage advisor.
//!
//! The display name lives here so a later rename does not require hunting
//! through scanners, storage, and the CLI.

#![forbid(unsafe_code)]

mod error;
mod model;

pub use error::Error;
pub use model::{
    DirectoryRecord, FileKind, FileRecord, RecommendationCategory, RiskLevel, Scan,
    ScanErrorRecord, ScanReport, ScanSnapshot, ScanStatistics, ScanStatus, ScanTarget,
    SCAN_CONCURRENCY,
};

/// User-facing product name. Placeholder; change this constant to rename the product.
pub const PRODUCT_NAME: &str = "Mac Storage Advisor";

/// Crate version shared by reports and persisted scan rows.
pub const PRODUCT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Environment variable that overrides the SQLite database path.
pub const DB_ENV_VAR: &str = "MAC_STORAGE_DB";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_name_is_the_placeholder() {
        assert_eq!(PRODUCT_NAME, "Mac Storage Advisor");
        assert!(!PRODUCT_VERSION.is_empty());
        assert_eq!(SCAN_CONCURRENCY, 1);
    }

    #[test]
    fn file_kind_db_strings_round_trip() {
        for kind in [
            FileKind::File,
            FileKind::Directory,
            FileKind::Symlink,
            FileKind::Other,
        ] {
            assert_eq!(FileKind::from_db(kind.as_str()), Some(kind));
        }
        assert_eq!(FileKind::from_db("socket"), None);
    }
}
