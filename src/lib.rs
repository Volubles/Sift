pub mod audio;
pub mod catalog;
pub mod collections;
pub mod config;
pub mod libraries;
pub mod library;
pub mod search;
pub mod watch;
pub mod waveform;
pub mod worker;

#[cfg(feature = "native-drag")]
pub mod platform;

#[cfg(feature = "desktop")]
pub mod desktop;

mod file_state;
mod names;
mod path_codec;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Selection(#[from] search::SelectionError),
    #[error("File operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Catalog operation failed: {0}")]
    Database(#[from] rusqlite::Error),
    #[error(
        "Unsupported catalog schema {0}; this application supports schema {supported}",
        supported = catalog::SCHEMA_VERSION
    )]
    UnsupportedSchema(i64),
    #[error("Invalid catalog path encoding")]
    InvalidPath,
    #[error("Choose an ordinary directory without a symbolic link or reparse point")]
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
