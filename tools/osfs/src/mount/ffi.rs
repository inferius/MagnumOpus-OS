//! Minimalni rucne psane napojeni na WinFsp (`winfsp-x64.dll`).
//!
//! Struktury a poradi polozek odpovidaji hlavickam z instalace WinFsp
//! (`inc\winfsp\fsctl.h`, `inc\winfsp\winfsp.h`, verze 2.x). Velikosti
//! hlida `const` assert stejne jako `FSP_FSCTL_STATIC_ASSERT` v hlavickach.
//! Bez bindgen a clang: DLL se nacte za behu (`LoadLibraryExW`) a funkce
//! se najdou pres `GetProcAddress`, takze nastroj jde prelozit i bez
//! nainstalovaneho WinFsp (selze az `osfs mount`).

#![allow(non_snake_case, clippy::upper_case_acronyms)]

use std::ffi::c_void;

pub type NTSTATUS = i32;
pub type BOOLEAN = u8;
pub type PWSTR = *mut u16;

// --- NTSTATUS kody, ktere pouzivame ---
pub const STATUS_SUCCESS: NTSTATUS = 0;
pub const STATUS_BUFFER_OVERFLOW: NTSTATUS = 0x8000_0005u32 as i32;
pub const STATUS_UNSUCCESSFUL: NTSTATUS = 0xC000_0001u32 as i32;
pub const STATUS_INVALID_PARAMETER: NTSTATUS = 0xC000_000Du32 as i32;
pub const STATUS_END_OF_FILE: NTSTATUS = 0xC000_0011u32 as i32;
pub const STATUS_NO_MEMORY: NTSTATUS = 0xC000_0017u32 as i32;
pub const STATUS_ACCESS_DENIED: NTSTATUS = 0xC000_0022u32 as i32;
pub const STATUS_OBJECT_NAME_INVALID: NTSTATUS = 0xC000_0033u32 as i32;
pub const STATUS_OBJECT_NAME_NOT_FOUND: NTSTATUS = 0xC000_0034u32 as i32;
pub const STATUS_OBJECT_NAME_COLLISION: NTSTATUS = 0xC000_0035u32 as i32;
pub const STATUS_OBJECT_PATH_NOT_FOUND: NTSTATUS = 0xC000_003Au32 as i32;
pub const STATUS_DISK_FULL: NTSTATUS = 0xC000_007Fu32 as i32;
pub const STATUS_INSUFFICIENT_RESOURCES: NTSTATUS = 0xC000_009Au32 as i32;
pub const STATUS_MEDIA_WRITE_PROTECTED: NTSTATUS = 0xC000_00A2u32 as i32;
pub const STATUS_FILE_IS_A_DIRECTORY: NTSTATUS = 0xC000_00BAu32 as i32;
pub const STATUS_NOT_SUPPORTED: NTSTATUS = 0xC000_00BBu32 as i32;
pub const STATUS_DIRECTORY_NOT_EMPTY: NTSTATUS = 0xC000_0101u32 as i32;
pub const STATUS_FILE_CORRUPT_ERROR: NTSTATUS = 0xC000_0102u32 as i32;
pub const STATUS_NOT_A_DIRECTORY: NTSTATUS = 0xC000_0103u32 as i32;
pub const STATUS_IO_DEVICE_ERROR: NTSTATUS = 0xC000_0185u32 as i32;

// --- atributy a volby (winnt.h) ---
pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
pub const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x20;
pub const FILE_DIRECTORY_FILE: u32 = 0x01;
pub const FILE_NON_DIRECTORY_FILE: u32 = 0x40;

/// `FspCleanupDelete` (winfsp.h, `FSP_FILE_SYSTEM_CLEANUP_FLAGS`).
pub const FSP_CLEANUP_DELETE: u32 = 0x01;

/// `FSP_FSCTL_DISK_DEVICE_NAME` = "WinFsp.Disk".
pub const DISK_DEVICE_NAME: &str = "WinFsp.Disk";

/// `FSP_FSCTL_VOLUME_PARAMS` (fsctl.h, verze s polozkami V0 + V1).
#[repr(C)]
pub struct VolumeParams {
    /// `sizeof(FSP_FSCTL_VOLUME_PARAMS)` = 504.
    pub Version: u16,
    pub SectorSize: u16,
    pub SectorsPerAllocationUnit: u16,
    pub MaxComponentLength: u16,
    pub VolumeCreationTime: u64,
    pub VolumeSerialNumber: u32,
    pub TransactTimeout: u32,
    pub IrpTimeout: u32,
    pub IrpCapacity: u32,
    pub FileInfoTimeout: u32,
    /// Bitova pole `CaseSensitiveSearch:1 ... KmReservedFlags:1` - viz
    /// konstanty `VP_*`.
    pub Flags: u32,
    pub Prefix: [u16; 192],
    pub FileSystemName: [u16; 16],
    // V1
    pub Flags2: u32,
    pub VolumeInfoTimeout: u32,
    pub DirInfoTimeout: u32,
    pub SecurityTimeout: u32,
    pub StreamInfoTimeout: u32,
    pub EaTimeout: u32,
    pub FsextControlCode: u32,
    pub Reserved32: [u32; 1],
    pub Reserved64: [u64; 2],
}
const _: () = assert!(size_of::<VolumeParams>() == 504);

// Bity v `VolumeParams::Flags` (poradi bitovych poli v fsctl.h od bitu 0).
pub const VP_CASE_SENSITIVE_SEARCH: u32 = 1 << 0;
pub const VP_CASE_PRESERVED_NAMES: u32 = 1 << 1;
pub const VP_UNICODE_ON_DISK: u32 = 1 << 2;
pub const VP_PERSISTENT_ACLS: u32 = 1 << 3;
pub const VP_READ_ONLY_VOLUME: u32 = 1 << 9;
pub const VP_POST_CLEANUP_WHEN_MODIFIED_ONLY: u32 = 1 << 10;
pub const VP_UM_FILE_CONTEXT_IS_USER_CONTEXT2: u32 = 1 << 16;

/// `FSP_FSCTL_VOLUME_INFO`.
#[repr(C)]
pub struct VolumeInfo {
    pub TotalSize: u64,
    pub FreeSize: u64,
    /// V bajtech.
    pub VolumeLabelLength: u16,
    pub VolumeLabel: [u16; 32],
}
const _: () = assert!(size_of::<VolumeInfo>() == 88);

/// `FSP_FSCTL_FILE_INFO`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FileInfo {
    pub FileAttributes: u32,
    pub ReparseTag: u32,
    pub AllocationSize: u64,
    pub FileSize: u64,
    pub CreationTime: u64,
    pub LastAccessTime: u64,
    pub LastWriteTime: u64,
    pub ChangeTime: u64,
    pub IndexNumber: u64,
    pub HardLinks: u32,
    pub EaSize: u32,
}
const _: () = assert!(size_of::<FileInfo>() == 72);

/// `FSP_FSCTL_OPEN_FILE_INFO`: u Create a Open ukazuje vystupni `FileInfo`
/// ve skutecnosti na tuto strukturu (`FspFileSystemGetOpenFileInfo`).
#[repr(C)]
pub struct OpenFileInfo {
    pub FileInfo: FileInfo,
    /// Buffer pro normalizovane jmeno (cela cesta s `\`), bez nuly na konci.
    pub NormalizedName: PWSTR,
    /// Na vstupu kapacita bufferu, na vystupu delka jmena (bajty).
    pub NormalizedNameSize: u16,
}

/// `FSP_FSCTL_DIR_INFO` bez jmena (jmeno UTF-16 nasleduje hned za ni).
#[repr(C)]
pub struct DirInfo {
    /// Velikost cele polozky vcetne jmena (bajty).
    pub Size: u16,
    pub FileInfo: FileInfo,
    /// Union `NextOffset` / `Padding[24]`.
    pub Padding: [u64; 3],
}
const _: () = assert!(size_of::<DirInfo>() == 104);

/// Nepruhledny `FSP_FILE_SYSTEM`.
#[repr(C)]
pub struct FileSystem {
    _private: [u8; 0],
}

type Fsp = *mut FileSystem;
type Ctx = *mut c_void;

/// `FSP_FILE_SYSTEM_INTERFACE`: 64 ukazatelu na funkce, poradi presne
/// podle winfsp.h. Nepouzite polozky = `None` (WinFsp pak vrati
/// `STATUS_INVALID_DEVICE_REQUEST`).
#[repr(C)]
pub struct Interface {
    pub GetVolumeInfo: Option<unsafe extern "system" fn(Fsp, *mut VolumeInfo) -> NTSTATUS>,
    pub SetVolumeLabel: Option<unsafe extern "system" fn()>,
    pub GetSecurityByName: Option<
        unsafe extern "system" fn(Fsp, PWSTR, *mut u32, *mut c_void, *mut usize) -> NTSTATUS,
    >,
    pub Create: Option<
        unsafe extern "system" fn(
            Fsp,
            PWSTR,
            u32,
            u32,
            u32,
            *mut c_void,
            u64,
            *mut Ctx,
            *mut FileInfo,
        ) -> NTSTATUS,
    >,
    pub Open: Option<
        unsafe extern "system" fn(Fsp, PWSTR, u32, u32, *mut Ctx, *mut FileInfo) -> NTSTATUS,
    >,
    pub Overwrite:
        Option<unsafe extern "system" fn(Fsp, Ctx, u32, BOOLEAN, u64, *mut FileInfo) -> NTSTATUS>,
    pub Cleanup: Option<unsafe extern "system" fn(Fsp, Ctx, PWSTR, u32)>,
    pub Close: Option<unsafe extern "system" fn(Fsp, Ctx)>,
    pub Read:
        Option<unsafe extern "system" fn(Fsp, Ctx, *mut c_void, u64, u32, *mut u32) -> NTSTATUS>,
    pub Write: Option<
        unsafe extern "system" fn(
            Fsp,
            Ctx,
            *mut c_void,
            u64,
            u32,
            BOOLEAN,
            BOOLEAN,
            *mut u32,
            *mut FileInfo,
        ) -> NTSTATUS,
    >,
    pub Flush: Option<unsafe extern "system" fn(Fsp, Ctx, *mut FileInfo) -> NTSTATUS>,
    pub GetFileInfo: Option<unsafe extern "system" fn(Fsp, Ctx, *mut FileInfo) -> NTSTATUS>,
    pub SetBasicInfo: Option<
        unsafe extern "system" fn(Fsp, Ctx, u32, u64, u64, u64, u64, *mut FileInfo) -> NTSTATUS,
    >,
    pub SetFileSize:
        Option<unsafe extern "system" fn(Fsp, Ctx, u64, BOOLEAN, *mut FileInfo) -> NTSTATUS>,
    pub CanDelete: Option<unsafe extern "system" fn(Fsp, Ctx, PWSTR) -> NTSTATUS>,
    pub Rename: Option<unsafe extern "system" fn(Fsp, Ctx, PWSTR, PWSTR, BOOLEAN) -> NTSTATUS>,
    pub GetSecurity:
        Option<unsafe extern "system" fn(Fsp, Ctx, *mut c_void, *mut usize) -> NTSTATUS>,
    pub SetSecurity: Option<unsafe extern "system" fn()>,
    pub ReadDirectory: Option<
        unsafe extern "system" fn(Fsp, Ctx, PWSTR, PWSTR, *mut c_void, u32, *mut u32) -> NTSTATUS,
    >,
    pub ResolveReparsePoints: Option<unsafe extern "system" fn()>,
    pub GetReparsePoint: Option<unsafe extern "system" fn()>,
    pub SetReparsePoint: Option<unsafe extern "system" fn()>,
    pub DeleteReparsePoint: Option<unsafe extern "system" fn()>,
    pub GetStreamInfo: Option<unsafe extern "system" fn()>,
    pub GetDirInfoByName: Option<unsafe extern "system" fn()>,
    pub Control: Option<unsafe extern "system" fn()>,
    pub SetDelete: Option<unsafe extern "system" fn()>,
    pub CreateEx: Option<unsafe extern "system" fn()>,
    pub OverwriteEx: Option<unsafe extern "system" fn()>,
    pub GetEa: Option<unsafe extern "system" fn()>,
    pub SetEa: Option<unsafe extern "system" fn()>,
    pub Obsolete0: Option<unsafe extern "system" fn()>,
    pub DispatcherStopped: Option<unsafe extern "system" fn()>,
    pub Reserved: [Option<unsafe extern "system" fn()>; 31],
}
const _: () = assert!(size_of::<Interface>() == 64 * size_of::<usize>());

// --- funkce z DLL ---
pub type FspFileSystemCreateFn =
    unsafe extern "system" fn(PWSTR, *const VolumeParams, *const Interface, *mut Fsp) -> NTSTATUS;
pub type FspFileSystemDeleteFn = unsafe extern "system" fn(Fsp);
pub type FspFileSystemSetMountPointFn = unsafe extern "system" fn(Fsp, PWSTR) -> NTSTATUS;
pub type FspFileSystemStartDispatcherFn = unsafe extern "system" fn(Fsp, u32) -> NTSTATUS;
pub type FspFileSystemStopDispatcherFn = unsafe extern "system" fn(Fsp);
pub type FspFileSystemAddDirInfoFn =
    unsafe extern "system" fn(*mut DirInfo, *mut c_void, u32, *mut u32) -> BOOLEAN;

/// Nactene funkce WinFsp.
pub struct Api {
    pub create: FspFileSystemCreateFn,
    pub delete: FspFileSystemDeleteFn,
    pub set_mount_point: FspFileSystemSetMountPointFn,
    pub start_dispatcher: FspFileSystemStartDispatcherFn,
    pub stop_dispatcher: FspFileSystemStopDispatcherFn,
    pub add_dir_info: FspFileSystemAddDirInfoFn,
}

// --- Win32 (kernel32, advapi32) ---
type HMODULE = *mut c_void;
pub type CtrlHandler = unsafe extern "system" fn(u32) -> i32;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> HMODULE;
    fn GetProcAddress(module: HMODULE, name: *const u8) -> *mut c_void;
    pub fn SetConsoleCtrlHandler(handler: Option<CtrlHandler>, add: i32) -> i32;
    pub fn LocalFree(mem: *mut c_void) -> *mut c_void;
    pub fn CreateEventW(
        attrs: *mut c_void,
        manual: i32,
        initial: i32,
        name: *const u16,
    ) -> *mut c_void;
    pub fn OpenEventW(access: u32, inherit: i32, name: *const u16) -> *mut c_void;
    pub fn SetEvent(event: *mut c_void) -> i32;
    pub fn WaitForSingleObject(handle: *mut c_void, ms: u32) -> u32;
    pub fn CloseHandle(handle: *mut c_void) -> i32;
    pub fn GetLastError() -> u32;
}

/// `EVENT_MODIFY_STATE` (pravo pro `SetEvent`).
pub const EVENT_MODIFY_STATE: u32 = 0x0002;
/// `ERROR_ALREADY_EXISTS` z `CreateEventW`.
pub const ERROR_ALREADY_EXISTS: u32 = 183;
pub const WAIT_OBJECT_0: u32 = 0;

#[link(name = "advapi32")]
unsafe extern "system" {
    fn RegGetValueW(
        key: isize,
        subkey: *const u16,
        value: *const u16,
        flags: u32,
        typ: *mut u32,
        data: *mut c_void,
        size: *mut u32,
    ) -> i32;
    pub fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        sddl: *const u16,
        revision: u32,
        sd: *mut *mut c_void,
        size: *mut u32,
    ) -> i32;
}

const HKEY_LOCAL_MACHINE: isize = 0x8000_0002u32 as i32 as isize;
const RRF_RT_REG_SZ: u32 = 0x0000_0002;
const RRF_SUBKEY_WOW6432KEY: u32 = 0x0002_0000;
const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x0000_0008;

/// Retezec -> UTF-16 zakonceny nulou.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// Delka retezce UTF-16 zakonceneho nulou.
///
/// # Safety
/// `p` ukazuje na platny retezec zakonceny nulou (nebo je null).
pub unsafe fn wcslen(p: *const u16) -> usize {
    if p.is_null() {
        return 0;
    }
    let mut n = 0;
    // SAFETY: volajici zarucuje retezec zakonceny nulou.
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    n
}

/// Adresar instalace WinFsp z registru (stejne jako `FspLoad` v winfsp.h).
fn install_dir() -> Option<String> {
    let subkey = wide("Software\\WinFsp");
    let value = wide("InstallDir");
    let mut buf = [0u16; 512];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: buffer a velikost odpovidaji, retezce zakoncene nulou.
    let r = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ | RRF_SUBKEY_WOW6432KEY,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if r != 0 {
        return None;
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16(&buf[..n]).ok()
}

/// Najde a nacte `winfsp-x64.dll`.
pub fn load() -> Result<Api, String> {
    let dir = install_dir().unwrap_or_else(|| "C:\\Program Files (x86)\\WinFsp\\".into());
    let mut path = dir.clone();
    if !path.ends_with('\\') {
        path.push('\\');
    }
    path.push_str("bin\\winfsp-x64.dll");
    let wpath = wide(&path);
    // SAFETY: cesta je platny retezec zakonceny nulou.
    let module = unsafe {
        LoadLibraryExW(
            wpath.as_ptr(),
            std::ptr::null_mut(),
            LOAD_WITH_ALTERED_SEARCH_PATH,
        )
    };
    if module.is_null() {
        return Err(format!(
            "nejde nacist {path} - je WinFsp nainstalovany? (https://winfsp.dev, potrebuje prava spravce)"
        ));
    }
    let get = |name: &str| -> Result<*mut c_void, String> {
        let cname: Vec<u8> = name.bytes().chain(Some(0)).collect();
        // SAFETY: modul je nacteny, jmeno zakoncene nulou.
        let p = unsafe { GetProcAddress(module, cname.as_ptr()) };
        if p.is_null() {
            Err(format!("{path}: chybi funkce {name} (stara verze WinFsp?)"))
        } else {
            Ok(p)
        }
    };
    // SAFETY: signatury odpovidaji deklaracim ve winfsp.h.
    unsafe {
        Ok(Api {
            create: std::mem::transmute::<*mut c_void, FspFileSystemCreateFn>(get(
                "FspFileSystemCreate",
            )?),
            delete: std::mem::transmute::<*mut c_void, FspFileSystemDeleteFn>(get(
                "FspFileSystemDelete",
            )?),
            set_mount_point: std::mem::transmute::<*mut c_void, FspFileSystemSetMountPointFn>(get(
                "FspFileSystemSetMountPoint",
            )?),
            start_dispatcher: std::mem::transmute::<*mut c_void, FspFileSystemStartDispatcherFn>(
                get("FspFileSystemStartDispatcher")?,
            ),
            stop_dispatcher: std::mem::transmute::<*mut c_void, FspFileSystemStopDispatcherFn>(
                get("FspFileSystemStopDispatcher")?,
            ),
            add_dir_info: std::mem::transmute::<*mut c_void, FspFileSystemAddDirInfoFn>(get(
                "FspFileSystemAddDirInfo",
            )?),
        })
    }
}

/// Popisovac zabezpeceni ve tvaru pro WinFsp (samorelativni).
pub fn security_descriptor(sddl: &str) -> Result<Vec<u8>, String> {
    let w = wide(sddl);
    let mut sd: *mut c_void = std::ptr::null_mut();
    let mut size: u32 = 0;
    // SAFETY: platne ukazatele; SD uvolnime pres LocalFree.
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(w.as_ptr(), 1, &mut sd, &mut size)
    };
    if ok == 0 || sd.is_null() {
        return Err("nejde vytvorit popisovac zabezpeceni".into());
    }
    // SAFETY: funkce vratila `size` bajtu na `sd`.
    let out = unsafe { std::slice::from_raw_parts(sd as *const u8, size as usize) }.to_vec();
    // SAFETY: alokovano systemem pres LocalAlloc.
    unsafe { LocalFree(sd) };
    Ok(out)
}
