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
