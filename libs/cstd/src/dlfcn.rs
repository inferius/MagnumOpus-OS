//! `dlfcn.h`: `dlopen`, `dlsym`, `dlclose`, `dlerror`, `dladdr`,
//! `dl_iterate_phdr` nad tabulkou zavadece ld.so (DL-4b, knihovny z
//! `/system/lib` pres sluzbu libd). Staticky program zavadec nema:
//! `dlopen` vrati NULL a `dlerror` duvod; `dl_iterate_phdr` popise jediny
//! modul (program) hlavickami slozenymi ze symbolu linker skriptu
//! (`user/user-std.ld`): `PT_LOAD` pres cely obraz a `PT_GNU_EH_FRAME`
//! nad `.eh_frame_hdr`, aby libunwind nasel tabulky rozbalovani (C++).

use core::ffi::c_void;

use crate::{c_char, c_int};

/// Tabulka zavadece (jen program PIE pod ld.so).
#[cfg(target_os = "magnumopus")]
fn table() -> Option<&'static std::os::magnumopus::runtime::dl::Interface> {
    std::os::magnumopus::runtime::dl::interface()
}

static NO_LOADER: &[u8] = b"dlopen: program je staticky (bez zavadece ld.so)\0";

std::thread_local! {
    /// Chyba bez zavadece (`dlerror` ji vrati jednou).
    static ERR: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void {
    #[cfg(target_os = "magnumopus")]
    if let Some(t) = table() {
        // SAFETY: funkce zavadece, retezec C od volajiciho.
        return unsafe { (t.dlopen)(path.cast(), flags) }.cast();
    }
    let _ = (path, flags);
    ERR.with(|e| e.set(true));
    core::ptr::null_mut()
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void {
    #[cfg(target_os = "magnumopus")]
    if let Some(t) = table() {
        // SAFETY: funkce zavadece.
        return unsafe { (t.dlsym)(handle.cast(), name.cast()) }.cast();
    }
    let _ = (handle, name);
    ERR.with(|e| e.set(true));
    core::ptr::null_mut()
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn dlclose(handle: *mut c_void) -> c_int {
    #[cfg(target_os = "magnumopus")]
    if let Some(t) = table() {
        // SAFETY: funkce zavadece.
        return unsafe { (t.dlclose)(handle.cast()) };
    }
    let _ = handle;
    -1
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn dlerror() -> *const c_char {
    #[cfg(target_os = "magnumopus")]
    if let Some(t) = table() {
        // SAFETY: funkce zavadece.
        return unsafe { (t.dlerror)() }.cast();
    }
    if ERR.with(|e| e.replace(false)) { NO_LOADER.as_ptr() } else { core::ptr::null() }
}

/// `Dl_info` (jako glibc).
#[repr(C)]
pub struct Dl_info {
    pub dli_fname: *const c_char,
    pub dli_fbase: *mut c_void,
    pub dli_sname: *const c_char,
    pub dli_saddr: *mut c_void,
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn dladdr(addr: *const c_void, info: *mut Dl_info) -> c_int {
    #[cfg(target_os = "magnumopus")]
    if let Some(t) = table() {
        // Rozlozeni Dl_info = abi::loader::DlInfo (4 slova).
        // SAFETY: funkce zavadece, `info` od volajiciho.
        return unsafe { (t.dladdr)(addr.cast(), info.cast()) };
    }
    let _ = (addr, info);
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn dl_iterate_phdr(
    cb: unsafe extern "C" fn(*mut c_void, usize, *mut c_void) -> c_int,
    data: *mut c_void,
) -> c_int {
    #[cfg(target_os = "magnumopus")]
    if let Some(t) = table() {
        // SAFETY: funkce zavadece; zpetne volani ma rozlozeni PhdrCallback.
        return unsafe { (t.dl_iterate_phdr)(core::mem::transmute(cb), data.cast()) };
    }
    #[cfg(all(target_os = "magnumopus", not(target_env = "dyn")))]
    {
        // SAFETY: zpetne volani od volajiciho; popis programu ze symbolu.
        unsafe { static_phdr::iterate(cb, data) }
    }
    #[cfg(not(all(target_os = "magnumopus", not(target_env = "dyn"))))]
    {
        let _ = (cb, data);
        0
    }
}

/// Programove hlavicky ELF64 (`Elf64_Phdr`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Phdr {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

pub const PT_LOAD: u32 = 1;
pub const PT_GNU_EH_FRAME: u32 = 0x6474_e550;
pub const PF_X: u32 = 1;
pub const PF_R: u32 = 4;

/// Hlavicky staticky linkovaneho programu (baze 0, ET_EXEC): obraz od
/// `image_start` do `image_end` a `.eh_frame_hdr` (prazdne = bez nej).
pub fn static_phdrs(image: (u64, u64), eh_hdr: (u64, u64)) -> ([Phdr; 2], u16) {
    let load = Phdr {
        p_type: PT_LOAD,
        p_flags: PF_R | PF_X,
        p_offset: 0,
        p_vaddr: image.0,
        p_paddr: image.0,
        p_filesz: image.1 - image.0,
        p_memsz: image.1 - image.0,
        p_align: 0x10000,
    };
    let eh = Phdr {
        p_type: PT_GNU_EH_FRAME,
        p_flags: PF_R,
        p_offset: 0,
        p_vaddr: eh_hdr.0,
        p_paddr: eh_hdr.0,
        p_filesz: eh_hdr.1 - eh_hdr.0,
        p_memsz: eh_hdr.1 - eh_hdr.0,
        p_align: 4,
    };
    ([load, eh], if eh_hdr.1 > eh_hdr.0 { 2 } else { 1 })
}

#[cfg(all(target_os = "magnumopus", not(target_env = "dyn")))]
mod static_phdr {
    use core::ffi::c_void;

    use abi::loader::PhdrInfo;

    use crate::c_int;

    unsafe extern "C" {
        // Symboly linker skriptu user/user-std.ld (jen staticky cil).
        static __mos_image_start: u8;
        static __mos_image_end: u8;
        static __eh_frame_hdr_start: u8;
        static __eh_frame_hdr_end: u8;
    }

    pub unsafe fn iterate(
        cb: unsafe extern "C" fn(*mut c_void, usize, *mut c_void) -> c_int,
        data: *mut c_void,
    ) -> c_int {
        let image = (&raw const __mos_image_start as u64, &raw const __mos_image_end as u64);
        let eh = (&raw const __eh_frame_hdr_start as u64, &raw const __eh_frame_hdr_end as u64);
        let (phdrs, phnum) = super::static_phdrs(image, eh);
        let mut info = PhdrInfo {
            addr: 0,
            name: b"\0".as_ptr(),
            phdr: phdrs.as_ptr().cast(),
            phnum,
            adds: 1,
            subs: 0,
            // TLS programu je blok osrt; libunwind ho nepotrebuje.
            tls_modid: 0,
            tls_data: core::ptr::null_mut(),
        };
        // SAFETY: rozlozeni PhdrInfo = struct dl_phdr_info (link.h);
        // hlavicky ziji po celou dobu volani.
        unsafe { cb((&raw mut info).cast(), core::mem::size_of::<PhdrInfo>(), data) }
    }
}
