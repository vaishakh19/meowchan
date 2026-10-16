//! Portable fallbacks for the platform layer.
//!
//! Used when compiling for non-Windows targets (development, CI). Nothing here pretends to know
//! Windows volume metadata: the functions return honest "unknown" values so the UI can show
//! "not available on this platform" instead of inventing numbers.

use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriveKind {
    Unknown,
    NoRootDir,
    Removable,
    Fixed,
    Network,
    Optical,
    RamDisk,
}

impl DriveKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            DriveKind::Unknown => "unknown",
            DriveKind::NoRootDir => "unknown",
            DriveKind::Removable => "removable",
            DriveKind::Fixed => "fixed",
            DriveKind::Network => "network",
            DriveKind::Optical => "optical",
            DriveKind::RamDisk => "ramdisk",
        }
    }
}

#[derive(Debug, Clone)]
pub struct VolumeInfo {
    pub id: String,
    pub root: String,
    pub kind: DriveKind,
    pub label: String,
    pub file_system: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub disconnected: bool,
    pub read_only: bool,
}

#[derive(Debug, Clone)]
pub struct RecycleBinInfo {
    pub item_count: u64,
    pub size_bytes: u64,
    pub supported: bool,
    pub detail: String,
}

pub fn list_volumes() -> Vec<VolumeInfo> {
    Vec::new()
}

pub fn drive_letter(path: &Path) -> Option<String> {
    let text = path.to_string_lossy().to_string();
    let bytes = text.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return Some(text[..2].to_uppercase());
    }
    None
}

pub fn is_system_drive(_path: &Path) -> bool {
    false
}

pub fn disk_size(_path: &Path) -> Option<u64> {
    None
}

pub fn recycle_bin(_drive_root: Option<&str>) -> RecycleBinInfo {
    RecycleBinInfo {
        item_count: 0,
        size_bytes: 0,
        supported: false,
        detail: "The Recycle Bin is only available on Windows.".to_string(),
    }
}

pub fn open_with_default_app(_path: &Path) -> Result<(), String> {
    Err("Opening files with the default application is only available on Windows.".to_string())
}

pub fn reveal_in_explorer(path: &Path) -> Result<(), String> {
    Err(format!(
        "File Explorer integration is only available on Windows ({}).",
        path.display()
    ))
}
