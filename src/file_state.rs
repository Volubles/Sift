use std::fs::Metadata;
use std::time::{SystemTime, UNIX_EPOCH};

/// Inspect already-read metadata; callers own the file/directory operation and
/// its error policy. Windows junctions and other reparse points are links here.
pub(crate) fn is_link(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

fn signed_nanoseconds(time: SystemTime) -> Option<i128> {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => i128::try_from(duration.as_nanos()).ok(),
        Err(error) => i128::try_from(error.duration().as_nanos())
            .ok()?
            .checked_neg(),
    }
}

pub(crate) fn timestamp(time: SystemTime) -> Option<i64> {
    i64::try_from(signed_nanoseconds(time)?).ok()
}

/// Cache keys are full signed nanoseconds in decimal, independent of Debug's
/// presentation. SQL metadata uses the narrower optional integer above.
pub(crate) fn timestamp_key(time: SystemTime) -> std::io::Result<String> {
    signed_nanoseconds(time)
        .map(|nanoseconds| nanoseconds.to_string())
        .ok_or_else(|| std::io::Error::other("File timestamp exceeds the cache key range"))
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/file_state.rs"
    ));
}
