//! Windows-specific helpers, written against the raw Win32 ABI.
//!
//! The bindings are declared by hand rather than pulled from a binding crate so the set of OS
//! entry points Diskwise uses stays explicit and reviewable: volume metadata, on-disk size,
//! Recycle Bin query, and "open with the default application". Nothing here can spawn a shell
//! command, and the webview has no permission to reach any of it directly — the frontend can only
//! ask for a narrowly-scoped command in `src/commands/`.
//!
//! Every function is `unsafe`-free at the boundary: raw pointers are created and dropped inside
//! these functions and never escape.

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::OnceLock;

/* ------------------------------- declarations ------------------------------- */

#[link(name = "kernel32")]
extern "system" {
    fn GetLogicalDrives() -> u32;
    fn GetDriveTypeW(lpRootPathName: *const u16) -> u32;
    fn GetDiskFreeSpaceExW(
        lpDirectoryName: *const u16,
        lpFreeBytesAvailableToCaller: *mut u64,
        lpTotalNumberOfBytes: *mut u64,
        lpTotalNumberOfFreeBytes: *mut u64,
    ) -> i32;
    fn GetVolumeInformationW(
        lpRootPathName: *const u16,
        lpVolumeNameBuffer: *mut u16,
        nVolumeNameSize: u32,
        lpVolumeSerialNumber: *mut u32,
        lpMaximumComponentLength: *mut u32,
        lpFileSystemFlags: *mut u32,
        lpFileSystemNameBuffer: *mut u16,
        nFileSystemNameSize: u32,
    ) -> i32;
    fn GetCompressedFileSizeW(lpFileName: *const u16, lpFileSizeHigh: *mut u32) -> u32;
    fn GetLastError() -> u32;
    fn SetLastError(dwErrCode: u32);
}

#[link(name = "ole32")]
extern "system" {
    fn CoInitializeEx(pvReserved: *mut c_void, dwCoInit: u32) -> i32;
    fn CoUninitialize();
    fn CoCreateInstance(
        rclsid: *const Guid,
        pUnkOuter: *mut c_void,
        dwClsContext: u32,
        riid: *const Guid,
        ppv: *mut *mut c_void,
    ) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteW(
        hwnd: *mut c_void,
        lpOperation: *const u16,
        lpFile: *const u16,
        lpParameters: *const u16,
        lpDirectory: *const u16,
        nShowCmd: i32,
    ) -> *mut c_void;
    fn SHCreateItemFromParsingName(
        pszPath: *const u16,
        pbc: *const c_void,
        riid: *const Guid,
        ppv: *mut *mut c_void,
    ) -> i32;
    fn SHQueryRecycleBinW(pszRootPath: *const u16, pSHQueryRBInfo: *mut SHQUERYRBINFO) -> i32;
}

#[repr(C)]
#[derive(Debug, Default)]
pub struct SHQUERYRBINFO {
    pub cbSize: u32,
    pub i64Size: i64,
    pub i64NumItems: i64,
}

/// A COM GUID, laid out exactly as the ABI expects.
#[repr(C)]
#[derive(Clone, Copy)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

const fn guid(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Guid {
    Guid { data1, data2, data3, data4 }
}

// CLSID_FileOperations {3AD05575-8857-4850-9277-11B85BDB8E09}
const CLSID_FILE_OPERATIONS: Guid = guid(
    0x3AD05575,
    0x8857,
    0x4850,
    [0x92, 0x77, 0x11, 0xB8, 0x5B, 0xDB, 0x8E, 0x09],
);
// IID_IFileOperation {947AAB5F-0A5C-4C13-B4D6-4BFB7836FC9F}
const IID_I_FILE_OPERATION: Guid = guid(
    0x947AAB5F,
    0x0A5C,
    0x4C13,
    [0xB4, 0xD6, 0x4B, 0xFB, 0x78, 0x36, 0xFC, 0x9F],
);
// IID_IShellItem {43826D1E-E718-42EE-BC55-A1E261C37BFE}
const IID_I_SHELL_ITEM: Guid = guid(
    0x43826D1E,
    0xE718,
    0x42EE,
    [0xBC, 0x55, 0xA1, 0xE2, 0x61, 0xC3, 0x7B, 0xFE],
);

// IFileOperation operation flags (FILEOP_FLAGS).
const FOF_SILENT: u32 = 0x4;
const FOF_NOCONFIRMATION: u32 = 0x10;
const FOF_ALLOWUNDO: u32 = 0x40;
const FOF_NOERRORUI: u32 = 0x400;

const COINIT_APARTMENTTHREADED: u32 = 2;
const CLSCTX_ALL: u32 = 23;

/// The `IFileOperation` vtable. Slots are declared in COM order; only the ones
/// Diskora uses are given precise signatures, but every slot exists so the
/// indices of `perform_operations` and friends are correct.
#[repr(C)]
struct IFileOperationVTable {
    query_interface: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    advise: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut u32) -> i32,
    unadvise: unsafe extern "system" fn(*mut c_void, u32) -> i32,
    set_operation_flags: unsafe extern "system" fn(*mut c_void, u32) -> i32,
    set_progress_message: unsafe extern "system" fn(*mut c_void, *const u16) -> i32,
    set_progress_dialog: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    set_properties: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    set_owner_window: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    apply_operations_to_item: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    delete_item: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    delete_items: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    copy_item: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *const u16, *mut c_void) -> i32,
    copy_items: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    move_item: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *const u16, *mut c_void) -> i32,
    move_items: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    rename_item: unsafe extern "system" fn(*mut c_void, *mut c_void, *const u16, *mut c_void) -> i32,
    rename_items: unsafe extern "system" fn(*mut c_void, *mut c_void, *const u16, *mut c_void) -> i32,
    new_item: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *const u16, *const u16, *mut c_void) -> i32,
    perform_operations: unsafe extern "system" fn(*mut c_void) -> i32,
    get_any_operations_aborted: unsafe extern "system" fn(*mut c_void, *mut i32) -> i32,
}

#[repr(C)]
struct IFileOperation {
    vtable: *const IFileOperationVTable,
}

/// The first three slots of every COM interface. `IShellItem` is only used as
/// an opaque token here, so this is all Diskora ever needs of it.
#[repr(C)]
struct IUnknownVTable {
    query_interface: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
struct IShellItem {
    vtable: *const IUnknownVTable,
}

const DRIVE_UNKNOWN: u32 = 0;
const DRIVE_NO_ROOT_DIR: u32 = 1;
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const DRIVE_REMOTE: u32 = 4;
const DRIVE_CDROM: u32 = 5;
const DRIVE_RAMDISK: u32 = 6;

const INVALID_FILE_SIZE: u32 = 0xFFFF_FFFF;
const SW_SHOWNORMAL: i32 = 1;
const SE_ERR_SUCCESS_THRESHOLD: usize = 32;

/* --------------------------------- helpers --------------------------------- */

/// Wide, NUL-terminated copy of a path. Windows APIs want this form.
pub fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
}

fn wide_str(value: &str) -> Vec<u16> {
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn last_error() -> u32 {
    // SAFETY: reading the thread's last-error value has no preconditions.
    unsafe { GetLastError() }
}

/* -------------------------------- drive info -------------------------------- */

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
    /// `C:`
    pub id: String,
    /// `C:\`
    pub root: String,
    pub kind: DriveKind,
    pub label: String,
    pub file_system: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    /// Volume is present but not ready (empty card reader, unformatted media).
    pub disconnected: bool,
    pub read_only: bool,
}

/// Enumerates every logical drive letter, including ones that are not ready.
pub fn list_volumes() -> Vec<VolumeInfo> {
    // SAFETY: `GetLogicalDrives` takes no arguments and cannot fail.
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for index in 0..26u32 {
        if mask & (1u32 << index) == 0 {
            continue;
        }
        let letter = (b'A' + index as u8) as char;
        let root = format!("{letter}:\\");
        out.push(volume_info(&root));
    }
    out
}

fn volume_info(root: &str) -> VolumeInfo {
    let root_path = Path::new(root);
    let wide_root = wide(root_path);

    // SAFETY: `wide_root` is a valid NUL-terminated wide string that outlives the call.
    let drive_type = unsafe { GetDriveTypeW(wide_root.as_ptr()) };
    let kind = match drive_type {
        DRIVE_UNKNOWN => DriveKind::Unknown,
        DRIVE_REMOVABLE => DriveKind::Removable,
        DRIVE_FIXED => DriveKind::Fixed,
        DRIVE_REMOTE => DriveKind::Network,
        DRIVE_CDROM => DriveKind::Optical,
        DRIVE_RAMDISK => DriveKind::RamDisk,
        DRIVE_NO_ROOT_DIR => DriveKind::NoRootDir,
        _ => DriveKind::Unknown,
    };

    let mut total: u64 = 0;
    let mut free: u64 = 0;
    // SAFETY: pointers are to live locals; the wide string outlives the call.
    let space_ok = unsafe {
        GetDiskFreeSpaceExW(
            wide_root.as_ptr(),
            std::ptr::null_mut(),
            &mut total as *mut u64,
            &mut free as *mut u64,
        )
    } != 0;

    let mut name_buf = [0u16; 260];
    let mut fs_buf = [0u16; 64];
    let mut serial: u32 = 0;
    let mut max_component: u32 = 0;
    let mut flags: u32 = 0;
    // SAFETY: buffers are sized correctly and live for the duration of the call.
    let info_ok = unsafe {
        GetVolumeInformationW(
            wide_root.as_ptr(),
            name_buf.as_mut_ptr(),
            name_buf.len() as u32,
            &mut serial as *mut u32,
            &mut max_component as *mut u32,
            &mut flags as *mut u32,
            fs_buf.as_mut_ptr(),
            fs_buf.len() as u32,
        )
    } != 0;

    let label = if info_ok {
        let end = name_buf.iter().position(|c| *c == 0).unwrap_or(name_buf.len());
        String::from_utf16_lossy(&name_buf[..end]).trim().to_string()
    } else {
        String::new()
    };

    let file_system = if info_ok {
        let end = fs_buf.iter().position(|c| *c == 0).unwrap_or(fs_buf.len());
        String::from_utf16_lossy(&fs_buf[..end]).trim().to_string()
    } else {
        String::new()
    };

    // FILE_READ_ONLY_VOLUME (0x0008_0000) / FILE_VOLUME_IS_COMPRESSED (0x0000_8000).
    let read_only = info_ok && (flags & 0x0008_0000) != 0;

    VolumeInfo {
        id: root.trim_end_matches('\\').to_string(),
        root: root.to_string(),
        kind,
        label,
        file_system,
        total_bytes: total,
        free_bytes: free,
        disconnected: !space_ok || !info_ok,
        read_only,
    }
}

/// Drive letter of a path, when the path is on a drive-rooted volume.
pub fn drive_letter(path: &Path) -> Option<String> {
    let text = path.to_string_lossy().to_string();
    let bytes = text.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return Some(text[..2].to_uppercase());
    }
    None
}

pub fn is_system_drive(path: &Path) -> bool {
    static SYSTEM_DRIVE: OnceLock<Option<String>> = OnceLock::new();
    let system = SYSTEM_DRIVE.get_or_init(|| {
        std::env::var("SystemDrive")
            .ok()
            .or_else(|| std::env::var("SYSTEMDRIVE").ok())
    });
    match (system.as_deref(), drive_letter(path)) {
        (Some(system), Some(letter)) => system.eq_ignore_ascii_case(&letter),
        _ => false,
    }
}

/* ------------------------------- on-disk size ------------------------------- */

/// Bytes actually allocated on disk, including sparse and compressed files.
/// Returns `None` when the platform cannot tell us (non-NTFS, network share, …).
pub fn disk_size(path: &Path) -> Option<u64> {
    let wide_path = wide(path);
    let mut high: u32 = 0;
    // SAFETY: valid wide string; `high` points at a live local.
    let low = unsafe {
        SetLastError(0);
        GetCompressedFileSizeW(wide_path.as_ptr(), &mut high as *mut u32)
    };
    if low == INVALID_FILE_SIZE && last_error() != 0 {
        return None;
    }
    Some((high as u64) << 32 | low as u64)
}

/* --------------------------------- open path -------------------------------- */

/// Opens a path with its default application through the shell.
/// Returns an error message when the shell refuses.
pub fn open_with_default_app(path: &Path) -> Result<(), String> {
    let wide_file = wide(path);
    let wide_verb = wide_str("open");
    // SAFETY: all wide strings outlive the call; no window handle is required.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            wide_verb.as_ptr(),
            wide_file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    let value = result as usize;
    if value <= SE_ERR_SUCCESS_THRESHOLD {
        return Err(format!(
            "Windows ShellExecute refused to open {} (code {value}).",
            path.display()
        ));
    }
    Ok(())
}

/// Reveals a path in File Explorer with the item selected.
pub fn reveal_in_explorer(path: &Path) -> Result<(), String> {
    let argument = format!("/select,{}", path.display());
    match std::process::Command::new("explorer.exe")
        .arg(&argument)
        .spawn()
    {
        Ok(_) => Ok(()),
        Err(error) => Err(format!("Could not start File Explorer: {error}")),
    }
}

/* --------------------------- recycle bin delete --------------------------- */

/// The filesystem of the volume holding `path` (`NTFS`, `FAT32`, …), or `None`
/// when it cannot be determined. Deletion only recycles on filesystems whose
/// volumes have a Recycle Bin.
pub fn volume_filesystem(path: &Path) -> Option<String> {
    let text = path.to_string_lossy().to_string();
    let bytes = text.as_bytes();
    if bytes.len() < 2 || bytes[1] != b':' || !bytes[0].is_ascii_alphabetic() {
        return None; // UNC and already-normalised paths have no simple root here
    }
    let root = format!("{}\\", &text[..2]);
    let wide_root = wide(Path::new(&root));

    let mut name_buf = [0u16; 64];
    let mut serial: u32 = 0;
    let mut max_component: u32 = 0;
    let mut flags: u32 = 0;
    // SAFETY: buffers are correctly sized locals; the wide root outlives the call.
    let ok = unsafe {
        SetLastError(0);
        GetVolumeInformationW(
            wide_root.as_ptr(),
            std::ptr::null_mut(),
            0,
            &mut serial as *mut u32,
            &mut max_component as *mut u32,
            &mut flags as *mut u32,
            name_buf.as_mut_ptr(),
            name_buf.len() as u32,
        )
    };
    if ok == 0 {
        return None;
    }
    let end = name_buf.iter().position(|c| *c == 0).unwrap_or(name_buf.len());
    let name = String::from_utf16_lossy(&name_buf[..end]).trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

/// Moves one file or folder to the Windows Recycle Bin through the shell's
/// `IFileOperation` with `FOF_ALLOWUNDO`. The operation is undoable by design:
/// if the volume has no Recycle Bin the call fails instead of deleting.
///
/// Callers must have validated the path first — this function does not re-check
/// protections; it is the last step of a guarded pipeline.
pub fn recycle_bin_delete(path: &Path) -> Result<(), String> {
    // SAFETY: COM is initialised for this thread only, and every interface
    // pointer obtained below is released before returning. No pointer escapes.
    unsafe {
        let co = CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED);
        // S_OK (0) or S_FALSE (1, already initialised) are both fine.
        if co != 0 && co != 1 {
            return Err(format!("Windows could not start a shell operation (code {co})."));
        }
        let result = recycle_bin_delete_com(path);
        CoUninitialize();
        result
    }
}

/// SAFETY: the caller must hold a COM apartment initialisation for this thread.
unsafe fn recycle_bin_delete_com(path: &Path) -> Result<(), String> {
    let mut operation: *mut c_void = std::ptr::null_mut();
    let created = CoCreateInstance(
        &CLSID_FILE_OPERATIONS as *const Guid,
        std::ptr::null_mut(),
        CLSCTX_ALL,
        &IID_I_FILE_OPERATION as *const Guid,
        &mut operation,
    );
    if created != 0 || operation.is_null() {
        return Err(format!("Windows could not create the recycle operation (code {created})."));
    }
    let operation = operation as *const IFileOperation;

    // Diskora shows its own confirmation dialog and reports errors itself, so
    // the shell stays quiet; ALLOWUNDO is what routes the delete to the
    // Recycle Bin instead of permanent removal.
    let flags = FOF_SILENT | FOF_NOCONFIRMATION | FOF_ALLOWUNDO | FOF_NOERRORUI;
    let flagged = ((*(*operation).vtable).set_operation_flags)(operation as *mut c_void, flags);
    if flagged != 0 {
        release(operation as *mut c_void);
        return Err(format!("Windows refused the recycle flags for {} (code {flagged}).", path.display()));
    }

    // Extended-length form keeps deep paths working, matching the scanner.
    let wide_path = crate::filesystem::long_path::to_unc_long_path(path);
    let mut item: *mut c_void = std::ptr::null_mut();
    let item_created = SHCreateItemFromParsingName(
        wide(&wide_path).as_ptr(),
        std::ptr::null(),
        &IID_I_SHELL_ITEM as *const Guid,
        &mut item,
    );
    if item_created != 0 || item.is_null() {
        release(operation as *mut c_void);
        return Err(format!("Windows could not read {} (code {item_created}).", path.display()));
    }

    let deleted = ((*(*operation).vtable).delete_item)(operation as *mut c_void, item, std::ptr::null_mut());
    release(item as *mut c_void);
    if deleted != 0 {
        release(operation as *mut c_void);
        return Err(format!("Windows refused to move {} to the Recycle Bin (code {deleted}).", path.display()));
    }

    let performed = ((*(*operation).vtable).perform_operations)(operation as *mut c_void);
    let mut aborted: i32 = 0;
    let _ = ((*(*operation).vtable).get_any_operations_aborted)(operation as *mut c_void, &mut aborted);
    release(operation as *mut c_void);

    if performed != 0 {
        return Err(format!("Moving {} to the Recycle Bin failed (Windows code {performed}).", path.display()));
    }
    if aborted != 0 {
        return Err(format!("Moving {} to the Recycle Bin was aborted by the system.", path.display()));
    }
    Ok(())
}

/// Releases a COM interface through its `IUnknown` vtable.
///
/// SAFETY: `interface` must be a valid, owned COM interface pointer.
unsafe fn release(interface: *mut c_void) -> u32 {
    let vtable = *(interface as *const *const IUnknownVTable);
    ((*vtable).release)(interface)
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

pub fn query_recycle_bin(drive_root: Option<&str>) -> Result<(u64, u64), String> {
    let mut info = SHQUERYRBINFO {
        cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
        i64Size: 0,
        i64NumItems: 0,
    };
    let wide_path = drive_root.map(|r| wide(Path::new(r)));
    let path_ptr = match &wide_path {
        Some(w) => w.as_ptr(),
        None => std::ptr::null(),
    };
    let res = unsafe { SHQueryRecycleBinW(path_ptr, &mut info) };
    if res == 0 {
        Ok((info.i64Size.max(0) as u64, info.i64NumItems.max(0) as u64))
    } else {
        Err(format!("SHQueryRecycleBinW failed with code {res}"))
    }
}

pub fn inspect_storage_breakdown(drive_root_str: &str) -> StorageBreakdownReport {
    let drive_path = Path::new(drive_root_str);
    let vol = volume_info(drive_root_str);
    let mut items = Vec::new();
    let mut notes = Vec::new();

    // 1. Recycle Bin
    match query_recycle_bin(Some(drive_root_str)) {
        Ok((size, count)) => {
            items.push(StorageBreakdownItem {
                id: "recycle-bin".to_string(),
                name: "Recycle Bin".to_string(),
                category: "other".to_string(),
                size_bytes: Some(size),
                item_count: Some(count),
                status: "measured".to_string(),
                path: Some(format!("{}\\$Recycle.Bin", drive_root_str.trim_end_matches('\\'))),
                description: "User files and folders moved to the Windows Recycle Bin.".to_string(),
                actionable_explanation: if size > 0 {
                    format!("Contains {} deleted item(s) using space. Can be emptied from the Desktop Recycle Bin or Windows Storage Settings.", count)
                } else {
                    "Recycle Bin is currently empty on this volume.".to_string()
                },
                is_system_protected: false,
            });
        }
        Err(e) => {
            items.push(StorageBreakdownItem {
                id: "recycle-bin".to_string(),
                name: "Recycle Bin".to_string(),
                category: "other".to_string(),
                size_bytes: None,
                item_count: None,
                status: "inaccessible".to_string(),
                path: Some(format!("{}\\$Recycle.Bin", drive_root_str.trim_end_matches('\\'))),
                description: "Windows Recycle Bin storage on this volume.".to_string(),
                actionable_explanation: format!("Could not query Recycle Bin API: {e}"),
                is_system_protected: false,
            });
        }
    }

    // 2. System Restore & Volume Shadow Copies (VSS)
    let svi_path = drive_path.join("System Volume Information");
    let svi_unc = crate::filesystem::long_path::to_unc_long_path(&svi_path);
    let svi_meta = std::fs::symlink_metadata(&svi_unc);
    match svi_meta {
        Ok(_) => {
            items.push(StorageBreakdownItem {
                id: "system-restore-vss".to_string(),
                name: "System Restore & Shadow Copies".to_string(),
                category: "system".to_string(),
                size_bytes: None,
                item_count: None,
                status: "inaccessible".to_string(),
                path: Some(svi_path.to_string_lossy().to_string()),
                description: "System Restore points, Volume Shadow Copies (VSS), and system catalog databases.".to_string(),
                actionable_explanation: "Windows restricts access to the SYSTEM account. Shadow copy allocation can be viewed and configured via System Properties > System Protection or elevated 'vssadmin list shadowstorage'.".to_string(),
                is_system_protected: true,
            });
        }
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            items.push(StorageBreakdownItem {
                id: "system-restore-vss".to_string(),
                name: "System Restore & Shadow Copies".to_string(),
                category: "system".to_string(),
                size_bytes: None,
                item_count: None,
                status: "inaccessible".to_string(),
                path: Some(svi_path.to_string_lossy().to_string()),
                description: "System Restore points, Volume Shadow Copies (VSS), and system catalog databases.".to_string(),
                actionable_explanation: "Access denied by Windows security permissions (requires elevated Administrator/SYSTEM). Managed in Windows System Protection.".to_string(),
                is_system_protected: true,
            });
        }
        Err(_) => {
            items.push(StorageBreakdownItem {
                id: "system-restore-vss".to_string(),
                name: "System Restore & Shadow Copies".to_string(),
                category: "system".to_string(),
                size_bytes: None,
                item_count: None,
                status: "not-configured".to_string(),
                path: Some(svi_path.to_string_lossy().to_string()),
                description: "System Volume Information directory is not present or configured on this volume.".to_string(),
                actionable_explanation: "System Restore is not enabled on this volume.".to_string(),
                is_system_protected: true,
            });
        }
    }

    // 3. Hibernation file (hiberfil.sys)
    let hiber_path = drive_path.join("hiberfil.sys");
    let hiber_unc = crate::filesystem::long_path::to_unc_long_path(&hiber_path);
    match std::fs::symlink_metadata(&hiber_unc) {
        Ok(m) => {
            let actual = disk_size(&hiber_unc).unwrap_or(m.len());
            items.push(StorageBreakdownItem {
                id: "hibernation-file".to_string(),
                name: "Hibernation File (hiberfil.sys)".to_string(),
                category: "system".to_string(),
                size_bytes: Some(actual),
                item_count: Some(1),
                status: "measured".to_string(),
                path: Some(hiber_path.to_string_lossy().to_string()),
                description: "Reserved memory dump file used by Windows Fast Startup and Hibernation.".to_string(),
                actionable_explanation: "Protected system file. Size and state can be managed via elevated command prompt using 'powercfg.exe /hibernate on|off' or 'powercfg /h /type reduced'.".to_string(),
                is_system_protected: true,
            });
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            items.push(StorageBreakdownItem {
                id: "hibernation-file".to_string(),
                name: "Hibernation File (hiberfil.sys)".to_string(),
                category: "system".to_string(),
                size_bytes: Some(0),
                item_count: Some(0),
                status: "not-configured".to_string(),
                path: Some(hiber_path.to_string_lossy().to_string()),
                description: "Windows Hibernation is disabled or not allocated on this volume.".to_string(),
                actionable_explanation: "No hibernation storage is currently allocated on this drive.".to_string(),
                is_system_protected: true,
            });
        }
        Err(e) => {
            items.push(StorageBreakdownItem {
                id: "hibernation-file".to_string(),
                name: "Hibernation File (hiberfil.sys)".to_string(),
                category: "system".to_string(),
                size_bytes: None,
                item_count: None,
                status: "inaccessible".to_string(),
                path: Some(hiber_path.to_string_lossy().to_string()),
                description: "Windows Hibernation file presence could not be determined.".to_string(),
                actionable_explanation: format!("File system check returned: {e}"),
                is_system_protected: true,
            });
        }
    }

    // 4. Pagefile & Swapfile (pagefile.sys, swapfile.sys)
    let page_path = drive_path.join("pagefile.sys");
    let swap_path = drive_path.join("swapfile.sys");
    let mut page_size = 0u64;
    let mut found_page = false;
    if let Ok(m) = std::fs::symlink_metadata(crate::filesystem::long_path::to_unc_long_path(&page_path)) {
        page_size += disk_size(&page_path).unwrap_or(m.len());
        found_page = true;
    }
    if let Ok(m) = std::fs::symlink_metadata(crate::filesystem::long_path::to_unc_long_path(&swap_path)) {
        page_size += disk_size(&swap_path).unwrap_or(m.len());
        found_page = true;
    }

    if found_page {
        items.push(StorageBreakdownItem {
            id: "virtual-memory".to_string(),
            name: "Virtual Memory (pagefile.sys / swapfile.sys)".to_string(),
            category: "system".to_string(),
            size_bytes: Some(page_size),
            item_count: Some(if page_size > 0 { 1 } else { 0 }),
            status: "measured".to_string(),
            path: Some(page_path.to_string_lossy().to_string()),
            description: "Windows virtual memory paging file and modern app swap space.".to_string(),
            actionable_explanation: "Managed automatically by Windows memory manager. Configurable in System Properties > Advanced > Performance Settings > Virtual Memory.".to_string(),
            is_system_protected: true,
        });
    } else {
        items.push(StorageBreakdownItem {
            id: "virtual-memory".to_string(),
            name: "Virtual Memory (pagefile.sys)".to_string(),
            category: "system".to_string(),
            size_bytes: Some(0),
            item_count: Some(0),
            status: "not-configured".to_string(),
            path: Some(page_path.to_string_lossy().to_string()),
            description: "No active paging file allocated on this specific drive.".to_string(),
            actionable_explanation: "Paging file is either disabled or allocated on another drive.".to_string(),
            is_system_protected: true,
        });
    }

    // 5. Windows Temp / Software Distribution Cache (if on system drive)
    if is_system_drive(drive_path) {
        let temp_dir = drive_path.join("Windows").join("Temp");
        let mut temp_bytes = 0u64;
        let mut temp_files = 0u64;
        let mut temp_accessible = false;
        if let Ok(entries) = std::fs::read_dir(crate::filesystem::long_path::to_unc_long_path(&temp_dir)) {
            temp_accessible = true;
            for entry in entries.flatten() {
                if let Ok(m) = entry.metadata() {
                    if m.is_file() {
                        temp_bytes += m.len();
                        temp_files += 1;
                    }
                }
            }
        }
        if temp_accessible {
            items.push(StorageBreakdownItem {
                id: "system-temp".to_string(),
                name: "System Temporary Files (C:\\Windows\\Temp)".to_string(),
                category: "cache".to_string(),
                size_bytes: Some(temp_bytes),
                item_count: Some(temp_files),
                status: "measured".to_string(),
                path: Some(temp_dir.to_string_lossy().to_string()),
                description: "Temporary files created by Windows system services and installers.".to_string(),
                actionable_explanation: "Safe to clean via Windows Disk Cleanup (cleanmgr.exe) or Windows Storage Sense.".to_string(),
                is_system_protected: false,
            });
        }
    }

    notes.push("All breakdown inspections are read-only. Diskora never deletes system-protected files or alters Windows configurations.".to_string());

    StorageBreakdownReport {
        drive_id: vol.id,
        drive_path: vol.root,
        measured_at: crate::time::now_millis(),
        total_drive_bytes: vol.total_bytes,
        free_drive_bytes: vol.free_bytes,
        items,
        notes,
    }
}
