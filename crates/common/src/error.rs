use std::io;

/// Application-level error returned by the CLI.
///
/// Scanner and storage crates keep their own typed errors. The CLI maps those
/// into this type at the process boundary so `common` does not depend on them.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Usage(String),
    #[error("{0}")]
    Scan(String),
    #[error("{0}")]
    Storage(String),
    #[error("{0}")]
    Io(#[from] io::Error),
}
