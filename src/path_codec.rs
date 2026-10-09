use std::path::{Path, PathBuf};

use crate::Result;

#[cfg(windows)]
pub fn encode(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[cfg(windows)]
pub fn decode(bytes: Vec<u8>) -> Result<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    if !bytes.len().is_multiple_of(2) {
        return Err(crate::Error::InvalidPath);
    }
    let units: Vec<_> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    Ok(PathBuf::from(OsString::from_wide(&units)))
}

#[cfg(unix)]
pub fn encode(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(unix)]
pub fn decode(bytes: Vec<u8>) -> Result<PathBuf> {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    Ok(PathBuf::from(OsString::from_vec(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_round_trip_without_using_display_strings() {
        let path = PathBuf::from("sounds/Forest_雨.wav");
        assert_eq!(decode(encode(&path)).unwrap(), path);
    }

    #[cfg(windows)]
    #[test]
    fn unpaired_utf16_surrogate_round_trips() {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt;
        let path = PathBuf::from(OsString::from_wide(&[0x61, 0xd800, 0x62]));
        assert_eq!(decode(encode(&path)).unwrap(), path);
        assert!(decode(vec![1]).is_err());
    }
}
