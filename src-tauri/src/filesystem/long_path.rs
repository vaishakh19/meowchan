//! Windows long-path (`\\?\`) handling.
//!
//! `std::fs` on Windows silently fails on paths longer than `MAX_PATH` (260) unless the process
//! opts into long paths. Diskwise normalises to the `\\?\` form before touching the filesystem so
//! deep trees work, and keeps the *display* path (the original, human-readable one) for the UI.

use std::path::{Path, PathBuf};

pub const MAX_PATH: usize = 260;

/// True when a path is at risk of hitting the legacy `MAX_PATH` limit.
pub fn is_long_path(path: &Path) -> bool {
    path.as_os_str().len() > MAX_PATH - 12
}

/// Converts a display path into the `\\?\` form the Win32 APIs accept.
/// Idempotent: an already-normalised path is returned unchanged.
pub fn to_unc_long_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy().to_string();
    if text.starts_with("\\\\?\\") || text.starts_with("\\\\??\\") || text.starts_with("//?/") {
        return path.to_path_buf();
    }
    // Drive-letter paths (`C:\...`) become `\\?\C:\...`.
    let bytes = text.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return PathBuf::from(format!("\\\\?\\{text}"));
    }
    // UNC paths (`\\server\share\...`) become `\\?\UNC\server\share\...`.
    if text.starts_with("\\\\") && !text.starts_with("\\\\?") {
        return PathBuf::from(format!("\\\\?\\UNC\\{}", &text[2..]));
    }
    path.to_path_buf()
}

/// Removes the `\\?\` prefix so the path can be shown to a human.
pub fn normalize_long_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy().to_string();
    if let Some(rest) = text.strip_prefix("\\\\?\\UNC\\") {
        return PathBuf::from(format!("\\\\{rest}"));
    }
    if let Some(rest) = text.strip_prefix("\\\\?\\") {
        return PathBuf::from(rest);
    }
    path.to_path_buf()
}
