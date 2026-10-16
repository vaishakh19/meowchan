//! Metadata snapshots.
//!
//! One `EntryMeta` per filesystem entry, taken with `symlink_metadata` so links are never followed
//! by default. Only the fields the product actually uses are stored — the brief is explicit that
//! Diskwise must not retain every metadata field for millions of entries.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::platform;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Folder,
}

/// Everything Diskwise keeps about one entry.
#[derive(Debug, Clone)]
pub struct EntryMeta {
    /// Display path (no `\\?\` prefix).
    pub path: PathBuf,
    pub name: String,
    pub kind: Kind,
    /// Logical size in bytes. For folders this is the entry's own contribution (0).
    pub size: u64,
    /// Allocated on-disk size when the platform exposes it.
    pub disk_size: Option<u64>,
    pub modified_at: Option<i64>,
    pub created_at: Option<i64>,
    pub extension: Option<String>,
    pub is_symlink: bool,
    pub is_hidden: bool,
    pub depth: usize,
}

/// A file in the flat index. Folders are aggregated separately.
#[derive(Debug, Clone)]
pub struct FileRecord {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub disk_size: Option<u64>,
    pub modified_at: Option<i64>,
    pub extension: Option<String>,
    /// Display path of the directory containing this file.
    pub parent_path: PathBuf,
    pub depth: usize,
}

pub fn millis(time: Option<SystemTime>) -> Option<i64> {
    time.and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
}

/// Reads metadata for one entry without following links.
///
/// `Err` is only returned for genuine I/O problems; permission denials are reported to the caller
/// through the error value so the scanner can record them honestly rather than guessing.
pub fn read_metadata(path: &Path, depth: usize) -> std::io::Result<EntryMeta> {
    // `\\?\` normalisation happens here so callers can pass display paths.
    let os_path = crate::filesystem::long_path::to_unc_long_path(path);
    let metadata = std::fs::symlink_metadata(&os_path)?;

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());

    let is_symlink = metadata.is_symlink();
    let kind = if metadata.is_dir() { Kind::Folder } else { Kind::File };

    let extension = if kind == Kind::File {
        path.extension().map(|e| e.to_string_lossy().to_lowercase())
    } else {
        None
    };

    #[cfg(unix)]
    let is_hidden = name.starts_with('.');
    #[cfg(windows)]
    let is_hidden = {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_HIDDEN = 0x2
        (metadata.file_attributes() & 0x2) != 0
    };
    #[cfg(not(any(unix, windows)))]
    let is_hidden = false;

    let disk_size = if kind == Kind::File {
        platform::disk_size(path)
    } else {
        None
    };

    Ok(EntryMeta {
        path: path.to_path_buf(),
        name,
        kind,
        size: if kind == Kind::File { metadata.len() } else { 0 },
        disk_size,
        modified_at: millis(metadata.modified().ok()),
        created_at: millis(metadata.created().ok()),
        extension,
        is_symlink,
        is_hidden,
        depth,
    })
}

/// Convenience for the details panel.
pub fn extension_of(path: &Path) -> Option<String> {
    path.extension().map(|e| e.to_string_lossy().to_lowercase())
}
