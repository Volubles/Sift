use std::path::PathBuf;

use crate::{Error, Result};

pub fn default_database() -> Result<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute());

    #[cfg(unix)]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|path| PathBuf::from(path).join(".local/share")));

    let base = base.ok_or_else(|| {
        Error::Configuration("User data directory is unavailable; specify --database PATH".into())
    })?;
    Ok(base.join("Sift").join("catalog.sqlite"))
}
