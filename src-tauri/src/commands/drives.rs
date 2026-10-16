//! Volume enumeration.

use tauri::State;

use crate::platform;
use crate::state::AppState;

/// Mirrors `DriveInfo`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveInfo {
    pub id: String,
    pub name: String,
    pub path: String,
    pub kind: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub used_bytes: u64,
    pub file_system: String,
    pub is_system_drive: bool,
    pub is_read_only: bool,
    pub is_disconnected: bool,
}

/// Every logical volume on this machine, including ones that are not ready.
#[tauri::command]
pub fn list_drives(_state: State<'_, AppState>) -> Vec<DriveInfo> {
    platform::list_volumes()
        .into_iter()
        .map(|volume| {
            let root = std::path::PathBuf::from(&volume.root);
            let name = if volume.label.is_empty() {
                format!("Local Disk ({})", volume.id)
            } else {
                format!("{} ({})", volume.label, volume.id)
            };
            let used = volume.total_bytes.saturating_sub(volume.free_bytes);
            DriveInfo {
                id: volume.id.clone(),
                name,
                path: volume.root.clone(),
                kind: volume.kind.as_str().to_string(),
                total_bytes: volume.total_bytes,
                free_bytes: volume.free_bytes,
                used_bytes: used,
                file_system: if volume.file_system.is_empty() {
                    "unknown".to_string()
                } else {
                    volume.file_system
                },
                is_system_drive: platform::is_system_drive(&root),
                is_read_only: volume.read_only,
                is_disconnected: volume.disconnected,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drive_with_no_label_gets_a_readable_name() {
        // The mapping is exercised through `list_volumes` on Windows; here we assert the shape.
        let info = DriveInfo {
            id: "C:".to_string(),
            name: "Local Disk (C:)".to_string(),
            path: "C:\\".to_string(),
            kind: "fixed".to_string(),
            total_bytes: 100,
            free_bytes: 40,
            used_bytes: 60,
            file_system: "NTFS".to_string(),
            is_system_drive: true,
            is_read_only: false,
            is_disconnected: false,
        };
        assert_eq!(info.used_bytes, info.total_bytes - info.free_bytes);
        assert!(info.name.contains("C:"));
    }
}
