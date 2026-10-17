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

pub fn open_with_default_app(_path: &Path) -> Result<(), String> {
    Err("Opening files with the default application is only available on Windows.".to_string())
}

pub fn reveal_in_explorer(path: &Path) -> Result<(), String> {
    Err(format!(
        "File Explorer integration is only available on Windows ({}).",
        path.display()
    ))
}

/// The filesystem of the volume holding `path`. Unknown off Windows.
pub fn volume_filesystem(_path: &Path) -> Option<String> {
    None
}

/// Recycle Bin deletion is a Windows shell operation; other platforms get an
/// honest refusal rather than a silent permanent delete.
pub fn recycle_bin_delete(path: &Path) -> Result<(), String> {
    Err(format!(
        "Moving {} to the Recycle Bin is only available on Windows.",
        path.display()
    ))
}

/* ------------------------ storage breakdown ------------------------ */

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageBreakdownItem {
    pub id: String,
    pub name: String,
    pub category: String,
    pub size_bytes: Option<u64>,
    pub item_count: Option<u64>,
    pub status: String,
    pub path: Option<String>,
    pub description: String,
    pub actionable_explanation: String,
    pub is_system_protected: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageBreakdownReport {
    pub drive_id: String,
    pub drive_path: String,
    pub measured_at: i64,
    pub total_drive_bytes: u64,
    pub free_drive_bytes: u64,
    pub items: Vec<StorageBreakdownItem>,
    pub notes: Vec<String>,
}

pub fn inspect_storage_breakdown(drive_root_str: &str) -> StorageBreakdownReport {
    StorageBreakdownReport {
        drive_id: drive_root_str.trim_end_matches(['\\', '/']).to_string(),
        drive_path: drive_root_str.to_string(),
        measured_at: crate::time::now_millis(),
        total_drive_bytes: 0,
        free_drive_bytes: 0,
        items: vec![
            StorageBreakdownItem {
                id: "recycle-bin".to_string(),
                name: "Recycle Bin".to_string(),
                category: "other".to_string(),
                size_bytes: None,
                item_count: None,
                status: "unsupported".to_string(),
                path: None,
                description: "Windows Recycle Bin query is not supported on this platform.".to_string(),
                actionable_explanation: "Only available on Windows.".to_string(),
                is_system_protected: false,
            },
            StorageBreakdownItem {
                id: "system-restore-vss".to_string(),
                name: "System Restore & Shadow Copies".to_string(),
                category: "system".to_string(),
                size_bytes: None,
                item_count: None,
                status: "unsupported".to_string(),
                path: None,
                description: "Windows Volume Shadow Copies (VSS) are only supported on Windows.".to_string(),
                actionable_explanation: "Only available on Windows.".to_string(),
                is_system_protected: true,
            },
            StorageBreakdownItem {
                id: "hibernation-file".to_string(),
                name: "Hibernation File (hiberfil.sys)".to_string(),
                category: "system".to_string(),
                size_bytes: None,
                item_count: None,
                status: "unsupported".to_string(),
                path: None,
                description: "Windows Fast Startup / Hibernation file is only present on Windows.".to_string(),
                actionable_explanation: "Only available on Windows.".to_string(),
                is_system_protected: true,
            },
            StorageBreakdownItem {
                id: "virtual-memory".to_string(),
                name: "Virtual Memory (pagefile.sys)".to_string(),
                category: "system".to_string(),
                size_bytes: None,
                item_count: None,
                status: "unsupported".to_string(),
                path: None,
                description: "Windows paging and swap files are only present on Windows.".to_string(),
                actionable_explanation: "Only available on Windows.".to_string(),
                is_system_protected: true,
            },
        ],
        notes: vec!["Windows storage breakdown APIs are only available on Windows.".to_string()],
    }
}
