//! Windows long-path (`\\\\?\\`) handling.
//!
//! The scanner keeps filesystem paths as `PathBuf`/`OsString` throughout. On Windows that is
//! important: converting a path through UTF-8 would replace an unpaired UTF-16 surrogate and make
//! the original filename impossible to open. Only paths crossing into the UI are rendered with
//! `to_string_lossy`, where the replacement character (U+FFFD) is the visible marker.

use std::path::{Path, PathBuf};

pub const MAX_PATH: usize = 260;

/// True when a path is at risk of hitting the legacy `MAX_PATH` limit.
pub fn is_long_path(path: &Path) -> bool {
    path.as_os_str().len() > MAX_PATH - 12
}

/// Converts a display path into the extended-length form accepted by Win32 APIs.
///
/// This implementation edits UTF-16 code units directly instead of round-tripping through a Rust
/// `String`, preserving unpaired surrogates in filenames. It is idempotent.
#[cfg(windows)]
pub fn to_unc_long_path(path: &Path) -> PathBuf {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    const PREFIX: &[u16] = &[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    const NAMESPACE: &[u16] = &[
        b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16,
    ];
    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.starts_with(PREFIX) || wide.starts_with(&[b'\\' as u16, b'\\' as u16, b'?' as u16, b'?' as u16, b'\\' as u16]) {
        return path.to_path_buf();
    }

    let is_drive_letter = wide.first().map(|unit| {
        (b'A' as u16..=b'Z' as u16).contains(unit)
            || (b'a' as u16..=b'z' as u16).contains(unit)
    }).unwrap_or(false);
    if wide.len() >= 2 && wide[1] == b':' as u16 && is_drive_letter {
        let mut extended = Vec::with_capacity(NAMESPACE.len() + wide.len());
        extended.extend_from_slice(NAMESPACE);
        extended.extend_from_slice(&wide);
        return PathBuf::from(OsString::from_wide(&extended));
    }

    if wide.starts_with(&[b'\\' as u16, b'\\' as u16]) {
        let mut extended = Vec::with_capacity(8 + wide.len());
        extended.extend_from_slice(&[
            b'\\' as u16,
            b'\\' as u16,
            b'?' as u16,
            b'\\' as u16,
            b'U' as u16,
            b'N' as u16,
            b'C' as u16,
            b'\\' as u16,
        ]);
        extended.extend_from_slice(&wide[2..]);
        return PathBuf::from(OsString::from_wide(&extended));
    }

    path.to_path_buf()
}

/// Removes the extended-length prefix while preserving every UTF-16 code unit.
#[cfg(windows)]
pub fn normalize_long_path(path: &Path) -> PathBuf {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    let unc_prefix: &[u16] = &[
        b'\\' as u16,
        b'\\' as u16,
        b'?' as u16,
        b'\\' as u16,
        b'U' as u16,
        b'N' as u16,
        b'C' as u16,
        b'\\' as u16,
    ];
    let regular_prefix: &[u16] = &[
        b'\\' as u16,
        b'\\' as u16,
        b'?' as u16,
        b'\\' as u16,
    ];
    if let Some(rest) = wide.strip_prefix(unc_prefix) {
        let mut display = Vec::with_capacity(rest.len() + 2);
        display.extend_from_slice(&[b'\\' as u16, b'\\' as u16]);
        display.extend_from_slice(rest);
        return PathBuf::from(OsString::from_wide(&display));
    }
    if let Some(rest) = wide.strip_prefix(regular_prefix) {
        return PathBuf::from(OsString::from_wide(rest));
    }
    path.to_path_buf()
}

/// Non-Windows builds use ordinary paths. The Windows-specific branches above are the only ones
/// used for filesystem access in the shipped application.
#[cfg(not(windows))]
pub fn to_unc_long_path(path: &Path) -> PathBuf {
    path.to_path_buf()
}

#[cfg(not(windows))]
pub fn normalize_long_path(path: &Path) -> PathBuf {
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_is_idempotent_for_ordinary_paths() {
        let path = Path::new("C:\\Users\\me\\file.txt");
        #[cfg(windows)]
        assert_eq!(normalize_long_path(&to_unc_long_path(path)), path);
        #[cfg(not(windows))]
        assert_eq!(normalize_long_path(&to_unc_long_path(path)), path);
    }

    #[cfg(windows)]
    #[test]
    fn round_trips_unpaired_surrogates_without_loss() {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt;
        let path = PathBuf::from(OsString::from_wide(&[
            b'C' as u16,
            b':' as u16,
            b'\\' as u16,
            0xD800,
            b'.' as u16,
            b't' as u16,
            b'x' as u16,
            b't' as u16,
        ]));
        assert_eq!(normalize_long_path(&to_unc_long_path(&path)), path);
    }
}
