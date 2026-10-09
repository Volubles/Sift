pub mod catalog;
pub mod config;
pub mod library;
pub mod search;
pub mod worker;

#[cfg(feature = "desktop")]
pub mod desktop;

mod path_codec;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("File operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Catalog operation failed: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("Unsupported catalog schema {0}; this application supports schema 1")]
    UnsupportedSchema(i64),
    #[error("Invalid catalog path encoding")]
    InvalidPath,
    #[error("The selected library root is not a directory")]
    InvalidRoot,
    #[error("{0}")]
    InvalidQuery(String),
    #[error("A value exceeds the catalog's supported range")]
    OutOfRange,
    #[error("Desktop initialization failed: {0}")]
    Desktop(String),
    #[error("Configuration error: {0}")]
    Configuration(String),
}

pub type Result<T> = std::result::Result<T, Error>;
