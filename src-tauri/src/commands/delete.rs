//! Safe file deletion — a Diskora Pro capability.
//!
//! Everything destructive happens here, in the native backend, behind a guarded
//! pipeline. The webview can only *ask*; it never touches the filesystem.
//!
//! The pipeline for every requested path:
//!   1. Structural guards (lexical, pure, unit-tested): absolute path, strictly
//!      inside the scanned tree, not a drive/scan root, not a protected Windows
//!      location, not a UNC path.
//!   2. Live filesystem checks: the entry still exists, it is not a symbolic
//!      link / junction / any reparse point (links are never followed), and its
//!      volume has a Recycle Bin (NTFS/ReFS).
//!   3. The shell moves the item to the Recycle Bin (`IFileOperation` with
//!      `FOF_ALLOWUNDO`). There is no permanent-delete code path at all.
//!   4. The filesystem is re-checked: success is only reported when the item is
//!      genuinely gone.
//!   5. The in-memory scan index drops the subtree so the UI's numbers stay
//!      truthful without a rescan.

use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::files::validate_path;
use crate::error::{AppError, AppResult};
use crate::filesystem::long_path::to_unc_long_path;
use crate::licensing;
use crate::platform;
use crate::scanner::aggregate::ScanIndex;
use crate::state::AppState;

/// At most this many items per request, so a runaway caller cannot queue a
/// million shell operations behind one confirmation.
const MAX_DELETE_BATCH: usize = 100;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteItemsRequest {
    pub scan_id: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletedItem {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteFailure {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteOutcome {
    pub deleted: Vec<DeletedItem>,
    pub failed: Vec<DeleteFailure>,
}

/// Moves user-selected items to the Windows Recycle Bin. Pro-gated in the
/// backend as well as the UI: an IPC caller cannot bypass the licence.
#[tauri::command]
pub async fn delete_items(
    request: DeleteItemsRequest,
    state: State<'_, AppState>,
) -> AppResult<DeleteOutcome> {
    // Destructive action: re-verify the purchase with Dodo now (the offline
    // grace window still covers a temporarily disconnected machine).
    // `require_pro` refuses both the network failure and the genuinely
    // unlicensed case — a Locked decision must never reach the shell call.
    licensing::require_pro(&state).await?;

    let mut paths = request.paths;
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        return Err(AppError::InvalidArgument("No item was selected.".to_string()));
    }
    if paths.len() > MAX_DELETE_BATCH {
        return Err(AppError::InvalidArgument(format!(
            "Select at most {MAX_DELETE_BATCH} items at once."
        )));
    }

    // Deletion is only meaningful against a completed scan: the scan root is
    // the security boundary that keeps every target inside the user's tree.
    let (root_path, index) = {
        let scans = state.scans()?;
        let entry = scans
            .get(&request.scan_id)
            .ok_or_else(|| AppError::NotFound(format!("Unknown scan {}.", request.scan_id)))?;
        let index = entry.index.clone().ok_or_else(|| {
            AppError::InvalidArgument("The scan is still running; wait for it to finish.".to_string())
        })?;
        (entry.root_path.clone(), index)
    };
    // `index` is an `Arc<RwLock<ScanIndex>>`: the same tables every live
    // reader uses, with deletions applied as a lock-protected overlay.

    let mut outcome = DeleteOutcome {
        deleted: Vec::new(),
        failed: Vec::new(),
    };
    for path in &paths {
        match delete_one(&root_path, &index, path).await {
            Ok(item) => outcome.deleted.push(item),
            Err((path, reason)) => outcome.failed.push(DeleteFailure { path, reason }),
        }
    }
    Ok(outcome)
}

/// Guards plus live checks plus the shell operation for one path.
async fn delete_one(
    root_path: &str,
    index: &Arc<RwLock<ScanIndex>>,
    path: &str,
) -> Result<DeletedItem, (String, String)> {
    let candidate = PathBuf::from(path);

    // 0. The shared entry check: absolute, well-formed, real-path only.
    validate_path(path).map_err(|error| (path.to_string(), error.to_string()))?;

    // 1. Structural guards — lexical, no filesystem access.
    if let Err(guard) = guard_structure(&candidate, Path::new(root_path)) {
        return Err((path.to_string(), guard.explanation()));
    }

    // 2. Live filesystem checks. `symlink_metadata` never follows links.
    let metadata = std::fs::symlink_metadata(to_unc_long_path(&candidate))
        .map_err(|error| (path.to_string(), AppError::io(path, &error).to_string()))?;

    // FILE_ATTRIBUTE_REPARSE_POINT = 0x400: symlinks, junctions, mount points.
    // Diskora never follows one to delete an unrelated target — it refuses.
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if (metadata.file_attributes() & 0x400) != 0 {
            return Err((
                path.to_string(),
                "This item is a link or junction. Diskora never deletes through links.".to_string(),
            ));
        }
    }
    #[cfg(not(windows))]
    if metadata.file_type().is_symlink() {
        return Err((
            path.to_string(),
            "This item is a link. Diskora never deletes through links.".to_string(),
        ));
    }
    let kind = if metadata.is_dir() { "folder" } else { "file" };

    // The Recycle Bin only exists on NTFS/ReFS volumes. If the shell cannot
    // guarantee an undoable delete, Diskora refuses rather than deleting.
    let volume = platform::volume_filesystem(&candidate)
        .ok_or_else(|| {
            (
                path.to_string(),
                "The volume of this item could not be identified, so deletion was refused.".to_string(),
            )
        })?;
    if !volume.eq_ignore_ascii_case("NTFS") && !volume.eq_ignore_ascii_case("ReFS") {
        return Err((
            path.to_string(),
            format!(
                "This item is on a {volume} volume, which has no Recycle Bin. Diskora only deletes through the Recycle Bin."
            ),
        ));
    }

    // 3. The shell operation itself.
    platform::recycle_bin_delete(&candidate)
        .map_err(|message| (path.to_string(), message))?;

    // 4. Proof, not promises: the item must actually be gone.
    if to_unc_long_path(&candidate).try_exists().unwrap_or(true) {
        return Err((
            path.to_string(),
            "Windows reported success but the item still exists; nothing was deleted.".to_string(),
        ));
    }

    // 5. Keep the scan results truthful. Capture the scanned size first:
    // once the overlay marks the path removed, `describe` can no longer
    // see the item's own subtree totals.
    let described = {
        let guard = index.read().expect("the scan index lock was poisoned");
        describe(&guard, &candidate)
    };
    {
        let mut guard = index.write().expect("the scan index lock was poisoned");
        guard.remove(&candidate);
    }

    let (name, size_bytes) = described.unwrap_or_else(|| {
        (
            candidate
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string()),
            metadata.len(),
        )
    });

    Ok(DeletedItem {
        path: path.to_string(),
        name,
        kind: kind.to_string(),
        size_bytes,
    })
}

/// Name and scanned size of an item, taken from the scan index.
fn describe(index: &ScanIndex, path: &Path) -> Option<(String, u64)> {
    if let Some(position) = index.position_of(path) {
        let directory = &index.directories[position];
        return Some((
            directory.name.clone(),
            index.aggregates[position].size_bytes,
        ));
    }
    let parent = path.parent()?;
    let position = index.position_of(parent)?;
    index
        .files_of(position)
        .into_iter()
        .find(|file| file.path == path)
        .map(|file| (file.name.clone(), file.size))
}

/* ------------------------------ structural guards ------------------------------ */

/// Why a path was refused. `explanation()` turns it into the sentence the user
/// sees; the variants themselves are unit-tested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DeletionGuard {
    DriveRoot,
    ScanRoot,
    OutsideScan,
    UncPath,
    Protected(&'static str),
}

impl DeletionGuard {
    fn explanation(&self) -> String {
        match self {
            DeletionGuard::DriveRoot => {
                "A whole drive cannot be deleted. Select individual files or folders inside it.".to_string()
            }
            DeletionGuard::ScanRoot => {
                "The scanned folder itself cannot be deleted. Select items inside it.".to_string()
            }
            DeletionGuard::OutsideScan => {
                "This item is outside the scanned folder, so it cannot be deleted from here.".to_string()
            }
            DeletionGuard::UncPath => {
                "Network locations have no Recycle Bin, so Diskora cannot delete them safely.".to_string()
            }
            DeletionGuard::Protected(location) => {
                format!("{location} is a protected Windows system location and cannot be deleted from Diskora.")
            }
        }
    }
}

/// Pure, lexical guard pipeline. Every check runs on the normalised path so
/// `..`, `.` and duplicated separators cannot smuggle a target past them.
pub(crate) fn guard_structure(target: &Path, scan_root: &Path) -> Result<(), DeletionGuard> {
    let target = normalize_lexically(target);
    let root = normalize_lexically(scan_root);

    if is_drive_root(&target) {
        return Err(DeletionGuard::DriveRoot);
    }
    if target == root {
        return Err(DeletionGuard::ScanRoot);
    }
    if is_unc(&target) {
        return Err(DeletionGuard::UncPath);
    }
    // Named before the boundary check so an escape attempt that lands
    // in a system location is reported as what it is.
    if let Some(location) = protected_location(&target) {
        return Err(DeletionGuard::Protected(location));
    }
    if !is_strict_descendant(&target, &root) {
        return Err(DeletionGuard::OutsideScan);
    }
    Ok(())
}

/// Resolves `.` and `..` lexically. Unlike `fs::canonicalize` this never
/// touches the disk, so it is safe on items that may already be gone.
pub(crate) fn normalize_lexically(path: &Path) -> PathBuf {
    use std::ffi::OsString;
    let mut normalised: Vec<OsString> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // Pop one level, but never past a prefix (`C:`) or the root.
                match normalised.last() {
                    Some(last)
                        if last != OsString::from("\\")
                            && !last.to_string_lossy().ends_with(':') =>
                    {
                        normalised.pop();
                    }
                    _ => {}
                }
            }
            other => normalised.push(other.as_os_str().to_os_string()),
        }
    }
    normalised.into_iter().collect()
}

fn is_drive_root(path: &Path) -> bool {
    let mut components = path.components();
    matches!(
        (components.next(), components.next(), components.next()),
        (Some(Component::Prefix(_)), Some(Component::RootDir), None)
    )
}

fn is_unc(path: &Path) -> bool {
    path.to_string_lossy().starts_with("\\\\")
}

/// Component-wise, case-insensitive descendant test — Windows paths are
/// case-insensitive, and the scan tree and the UI agree on casing by
/// construction, but a hand-crafted IPC call must not slip through.
fn is_strict_descendant(target: &Path, root: &Path) -> bool {
    let target_components: Vec<String> = target
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_lowercase())
        .collect();
    let root_components: Vec<String> = root
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_lowercase())
        .collect();
    target_components.len() > root_components.len()
        && target_components.starts_with(&root_components)
}

/// System locations Diskora will never delete from, even with Pro unlocked.
///
/// * On the system drive: the operating system's own trees, and the `Users`
///   folder itself (every profile lives there). A user's *own* profile folder
///   is their data and stays deletable.
/// * On every volume: the per-volume system locations.
const SYSTEM_LOCATIONS: &[&str] = &[
    "windows",
    "program files",
    "program files (x86)",
    "programdata",
    "documents and settings",
];
const VOLUME_LOCATIONS: &[&str] = &[
    "$recycle.bin",
    "system volume information",
    "recovery",
];

fn protected_location(target: &Path) -> Option<&'static str> {
    let components: Vec<String> = target
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect();
    if components.is_empty() {
        return Some("the volume root");
    }
    // Per-volume system locations are protected on every drive.
    if components.len() == 1 && VOLUME_LOCATIONS.contains(&components[0].as_str()) {
        return Some(match components[0].as_str() {
            "$recycle.bin" => "$Recycle.Bin",
            "system volume information" => "System Volume Information",
            _ => "Recovery",
        });
    }
    if platform::is_system_drive(target) && SYSTEM_LOCATIONS.contains(&components[0].as_str()) {
        return Some(match components[0].as_str() {
            "windows" => "C:\\Windows",
            "program files" => "Program Files",
            "program files (x86)" => "Program Files (x86)",
            "programdata" => "ProgramData",
            _ => "Users",
        });
    }
    // `C:\Users` itself holds every profile on the machine.
    if platform::is_system_drive(target)
        && components.len() == 1
        && components[0] == "users"
    {
        return Some("C:\\Users");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_normalisation_resolves_dots_without_touching_the_disk() {
        assert_eq!(
            normalize_lexically(Path::new("C:\\Users\\me\\..\\me\\file.txt")),
            PathBuf::from("C:\\Users\\me\\file.txt")
        );
        assert_eq!(
            normalize_lexically(Path::new("C:\\Users\\.\\me")),
            PathBuf::from("C:\\Users\\me")
        );
        // `..` at the drive root stays at the drive root.
        assert_eq!(
            normalize_lexically(Path::new("C:\\..\\Windows")),
            PathBuf::from("C:\\Windows")
        );
        assert_eq!(
            normalize_lexically(Path::new("C:\\Users\\me\\")),
            PathBuf::from("C:\\Users\\me")
        );
    }

    #[test]
    fn drive_roots_and_scan_roots_are_refused() {
        assert_eq!(
            guard_structure(Path::new("C:\\"), Path::new("C:\\Users\\me")),
            Err(DeletionGuard::DriveRoot)
        );
        assert_eq!(
            guard_structure(Path::new("C:\\Users\\me"), Path::new("C:\\Users\\me")),
            Err(DeletionGuard::ScanRoot)
        );
        // A trailing separator is still the scan root.
        assert_eq!(
            guard_structure(Path::new("C:\\Users\\me\\"), Path::new("C:\\Users\\me")),
            Err(DeletionGuard::ScanRoot)
        );
    }

    #[test]
    fn paths_outside_the_scan_tree_are_refused() {
        assert_eq!(
            guard_structure(Path::new("D:\\other\\file.txt"), Path::new("C:\\Users\\me")),
            Err(DeletionGuard::OutsideScan)
        );
        // `..` cannot escape the scan root, even when nested.
        assert_eq!(
            guard_structure(
                Path::new("C:\\Users\\me\\..\\..\\Windows\\evil.dll"),
                Path::new("C:\\Users\\me")
            ),
            Err(DeletionGuard::Protected("C:\\Windows"))
        );
        assert_eq!(
            guard_structure(Path::new("C:\\Users\\other\\file.txt"), Path::new("C:\\Users\\me")),
            Err(DeletionGuard::OutsideScan)
        );
    }

    #[test]
    fn descendants_of_the_scan_root_pass_the_structural_guards() {
        assert_eq!(
            guard_structure(Path::new("C:\\Users\\me\\Downloads\\big.bin"), Path::new("C:\\Users\\me")),
            Ok(())
        );
        // Case differences do not matter on Windows.
        assert_eq!(
            guard_structure(Path::new("c:\\users\\me\\downloads"), Path::new("C:\\Users\\ME")),
            Ok(())
        );
    }

    #[test]
    fn unc_paths_have_no_recycle_bin_and_are_refused() {
        assert_eq!(
            guard_structure(Path::new("\\\\server\\share\\file.txt"), Path::new("\\\\server\\share")),
            Err(DeletionGuard::UncPath)
        );
    }

    #[test]
    fn critical_system_locations_are_protected() {
        let root = Path::new("C:\\");
        for path in [
            "C:\\Windows",
            "C:\\Windows\\System32\\cmd.exe",
            "C:\\Program Files\\App\\app.exe",
            "C:\\Program Files (x86)\\App\\app.exe",
            "C:\\ProgramData\\Microsoft\\Windows\\Start Menu",
            "C:\\Users",
            "C:\\Documents and Settings",
            "C:\\System Volume Information",
            "C:\\$Recycle.Bin",
            "C:\\Recovery\\ReCustomization.dat",
        ] {
            assert_eq!(
                guard_structure(Path::new(path), root),
                Err(DeletionGuard::Protected(_)),
                "{path} must stay protected"
            );
        }
        // A user's own profile and ordinary user data stay deletable.
        assert_eq!(guard_structure(Path::new("C:\\Users\\me\\file.txt"), root), Ok(()));
        assert_eq!(
            guard_structure(Path::new("C:\\Users\\me\\AppData\\Local\\Temp\\junk.tmp"), root),
            Ok(())
        );
    }

    #[test]
    fn volume_system_locations_are_protected_on_any_drive() {
        assert_eq!(
            guard_structure(Path::new("D:\\System Volume Information"), Path::new("D:\\")),
            Err(DeletionGuard::Protected("System Volume Information"))
        );
        assert_eq!(
            guard_structure(Path::new("D:\\$Recycle.Bin\\S-1-5-21\\file.txt"), Path::new("D:\\")),
            Ok(())
        );
        // A data drive's ordinary folders are the user's business.
        assert_eq!(
            guard_structure(Path::new("D:\\Windows\\old_backup.zip"), Path::new("D:\\")),
            Ok(())
        );
    }

    #[test]
    fn the_guard_pipeline_rejects_relative_and_malformed_paths_first() {
        // `validate_path` (the shared entry check) runs before the guards.
        assert!(validate_path("..\\..\\Windows").is_err());
        assert!(validate_path("C:\\Users\\me").is_ok());
    }

    #[test]
    fn describe_reads_names_and_sizes_from_the_index() {
        // Built from a synthetic run: one folder with one file.
        let run = crate::scanner::walker::ScanRun {
            files: vec![crate::filesystem::FileRecord {
                path: PathBuf::from("C:\\fixture\\a.bin"),
                name: "a.bin".to_string(),
                size: 10,
                disk_size: Some(4096),
                modified_at: None,
                extension: Some("bin".to_string()),
                parent_path: PathBuf::from("C:\\fixture"),
                depth: 1,
            }],
            directories: vec![crate::scanner::walker::DirectoryRecord {
                name: "fixture".to_string(),
                path: PathBuf::from("C:\\fixture"),
                parent: None,
                depth: 0,
                modified_at: None,
                created_at: None,
                access_denied: false,
            }],
            errors: Vec::new(),
            hit_limit: false,
            cancelled: false,
            elapsed_ms: 0,
        };
        let index = ScanIndex::from_run(&run);
        assert_eq!(
            describe(&index, Path::new("C:\\fixture\\a.bin")),
            Some(("a.bin".to_string(), 10))
        );
        assert_eq!(
            describe(&index, Path::new("C:\\fixture")),
            Some(("fixture".to_string(), 10))
        );
        assert_eq!(describe(&index, Path::new("C:\\fixture\\missing.bin")), None);
    }
}
