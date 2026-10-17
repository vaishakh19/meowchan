//! Metadata snapshots.
//!
//! Filesystem access uses `symlink_metadata`, which describes the entry itself and never follows a
//! symbolic link. Windows reparse points (junctions, mount points and cloud placeholders) are also
//! treated as links: their stored logical size is zero and traversal never enters them.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::platform;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Folder,
}

/// Everything Diskora keeps about one entry.
#[derive(Debug, Clone)]
pub struct EntryMeta {
    /// Filesystem path without a `\\?\` prefix. This remains an `OsString`-backed path and is
    /// never round-tripped through UTF-8 during filesystem operations.
    pub path: PathBuf,
    pub name: String,
    pub kind: Kind,
    /// Logical size in bytes. For folders and all reparse points this is zero.
    pub size: u64,
    /// Allocated on-disk size when the platform exposes it, and only for ordinary files.
    pub disk_size: Option<u64>,
    pub modified_at: Option<i64>,
    pub created_at: Option<i64>,
    pub extension: Option<String>,
    pub is_symlink: bool,
    pub is_junction: bool,
    pub is_reparse_point: bool,
    pub is_hidden: bool,
    pub is_cloud_placeholder: bool,
    pub cloud_status: Option<String>,
    pub depth: usize,
}

/// A file in the flat index. Folders are aggregated separately.
#[derive(Debug, Clone, Default)]
pub struct FileRecord {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub disk_size: Option<u64>,
    pub modified_at: Option<i64>,
    pub extension: Option<String>,
    pub depth: usize,
    pub is_symlink: bool,
    pub is_junction: bool,
    pub is_cloud_placeholder: bool,
    pub cloud_status: Option<String>,
}

pub fn millis(time: Option<SystemTime>) -> Option<i64> {
    time.and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
}

/// Reads metadata for one entry without following links or reparse points.
pub fn read_metadata(path: &Path, depth: usize) -> std::io::Result<EntryMeta> {
    let os_path = crate::filesystem::long_path::to_unc_long_path(path);
    let metadata = std::fs::symlink_metadata(&os_path)?;
    let file_type = metadata.file_type();

    #[cfg(windows)]
    let (is_reparse_point, is_hidden, is_cloud_placeholder, cloud_status) = {
        use std::os::windows::fs::MetadataExt;
        let attrs = metadata.file_attributes();
        // FILE_ATTRIBUTE_REPARSE_POINT = 0x400. All reparse points are non-traversable links.
        let reparse = (attrs & 0x400) != 0;
        // FILE_ATTRIBUTE_HIDDEN = 0x2
        let hidden = (attrs & 0x2) != 0;
        // Cloud Files placeholder flags. We retain the annotation for display, but never hydrate
        // or read a placeholder during the scan.
        let is_offline = (attrs & 0x1000) != 0;
        let is_recall_open = (attrs & 0x40000) != 0;
        let is_pinned = (attrs & 0x80000) != 0;
        let is_unpinned = (attrs & 0x100000) != 0;
        let is_recall_access = (attrs & 0x400000) != 0;
        let cloud = is_offline || is_recall_open || is_pinned || is_unpinned || is_recall_access;
        let status = if cloud {
            if is_pinned {
                Some("pinned".to_string())
            } else if is_recall_access || is_offline || is_unpinned {
                Some("online-only".to_string())
            } else {
                Some("locally-available".to_string())
            }
        } else {
            None
        };
        (reparse, hidden, cloud, status)
    };

    #[cfg(unix)]
    let (is_reparse_point, is_hidden, is_cloud_placeholder, cloud_status) =
        (file_type.is_symlink(), path.file_name().is_some_and(|name| name.to_string_lossy().starts_with('.')), false, None);

    #[cfg(not(any(unix, windows)))]
    let (is_reparse_point, is_hidden, is_cloud_placeholder, cloud_status) =
        (file_type.is_symlink(), false, false, None);

    let kind = if metadata.is_dir() { Kind::Folder } else { Kind::File };
    let is_symlink = file_type.is_symlink() || (is_reparse_point && kind == Kind::File);
    let is_junction = is_reparse_point && kind == Kind::Folder && !is_symlink;

    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());
    let extension = if kind == Kind::File && !is_reparse_point {
        path.extension().map(|extension| extension.to_string_lossy().to_lowercase())
    } else {
        None
    };
    let disk_size = if kind == Kind::File && !is_reparse_point {
        platform::disk_size(&os_path)
    } else {
        None
    };

    Ok(EntryMeta {
        path: path.to_path_buf(),
        name,
        kind,
        size: if kind == Kind::File && !is_reparse_point {
            metadata.len()
        } else {
            0
        },
        disk_size,
        modified_at: millis(metadata.modified().ok()),
        created_at: millis(metadata.created().ok()),
        extension,
        is_symlink,
        is_junction,
        is_reparse_point,
        is_hidden,
        is_cloud_placeholder,
        cloud_status,
        depth,
    })
}

/// Verifies each path component from its root toward the leaf before a caller opens or enumerates
/// the full path. This rejects a link in an ancestor as well as a link at the leaf.
pub fn ensure_no_reparse_components(path: &Path) -> std::io::Result<()> {
    use std::path::Component;

    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(crate::filesystem::long_path::to_unc_long_path(&current))?;
        #[cfg(windows)]
        let is_reparse_point = {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes() & 0x400 != 0
        };
        #[cfg(not(windows))]
        let is_reparse_point = metadata.file_type().is_symlink();
        if is_reparse_point {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("Refusing to follow link or reparse point: {}", current.to_string_lossy()),
            ));
        }
    }
    Ok(())
}

/// Convenience for the details panel.
pub fn extension_of(path: &Path) -> Option<String> {
    path.extension().map(|extension| extension.to_string_lossy().to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn regular_file_size_is_logical_size() {
        let directory = std::env::temp_dir().join(format!("diskora-metadata-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("ordinary.txt");
        fs::write(&path, b"logical").unwrap();
        let entry = read_metadata(&path, 1).unwrap();
        assert_eq!(entry.size, 7);
        assert!(!entry.is_reparse_point);
        let _ = fs::remove_dir_all(directory);
    }

    #[cfg(unix)]
    #[test]
    fn path_component_check_rejects_a_symlinked_ancestor() {
        use std::os::unix::fs::symlink;
        let directory = std::env::temp_dir().join(format!("diskora-component-link-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(directory.join("real")).unwrap();
        symlink(directory.join("real"), directory.join("link")).unwrap();
        let path = directory.join("link").join("file.bin");
        let error = ensure_no_reparse_components(&path).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        let _ = fs::remove_dir_all(directory);
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_links_are_zero_size_entries_and_are_never_followed() {
        use std::os::unix::fs::symlink;
        let directory = std::env::temp_dir().join(format!("diskora-link-meta-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let target = directory.join("target.bin");
        let link = directory.join("link.bin");
        fs::write(&target, b"not counted through link").unwrap();
        symlink(&target, &link).unwrap();
        let entry = read_metadata(&link, 1).unwrap();
        assert!(entry.is_symlink);
        assert!(entry.is_reparse_point);
        assert_eq!(entry.size, 0);
        assert_eq!(entry.disk_size, None);
        let _ = fs::remove_dir_all(directory);
    }
}
