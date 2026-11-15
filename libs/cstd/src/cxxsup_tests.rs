//! Testy funkci pro beh C++ (`cxxsup.rs`) na hostiteli.

use core::ffi::c_void;

use crate::cxxsup::*;
use crate::errno;
use crate::stat::timespec;

#[test]
fn getentropy_limits() {
    let mut b = [0u8; 300];
    unsafe {
        assert_eq!(getentropy(b.as_mut_ptr().cast(), 0), 0);
        assert_eq!(getentropy(b.as_mut_ptr().cast(), 256), 0);
        assert!(b[..256].iter().any(|&x| x != 0), "256 nul je nepravdepodobnych");
        assert_eq!(getentropy(b.as_mut_ptr().cast(), 257), -1);
        assert_eq!(errno::get(), errno::EINVAL);
    }
}

#[test]
fn aligned_alloc_and_posix_memalign() {
    unsafe {
        let p = aligned_alloc(256, 1000);
        assert!(!p.is_null());
        assert_eq!(p as usize % 256, 0);
        cstd_core::stdlib::free(p.cast());
        assert!(aligned_alloc(3, 8).is_null());
        assert_eq!(errno::get(), errno::EINVAL);

        let mut out: *mut c_void = core::ptr::null_mut();
        assert_eq!(posix_memalign(&mut out, 64, 10), 0);
        assert_eq!(out as usize % 64, 0);
        cstd_core::stdlib::free(out.cast());
        // zarovnani mensi nez sizeof(void *) nebo ne mocnina 2
        assert_eq!(posix_memalign(&mut out, 4, 10), errno::EINVAL);
        assert_eq!(posix_memalign(&mut out, 24, 10), errno::EINVAL);
        assert_eq!(posix_memalign(&mut out, 64, usize::MAX), errno::ENOMEM);
    }
}

#[test]
fn timespec_get_bases() {
    let mut t = timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe {
        assert_eq!(timespec_get(&mut t, TIME_UTC), TIME_UTC);
        assert!(t.tv_sec > 1_600_000_000);
        assert!((0..1_000_000_000).contains(&t.tv_nsec));
        assert_eq!(timespec_get(&mut t, TIME_MONOTONIC), TIME_MONOTONIC);
        assert_eq!(timespec_get(&mut t, 99), 0);
    }
}

#[test]
fn headers_match() {
    let time = include_str!("../include/time.h");
    assert!(time.contains(&format!("#define TIME_UTC {TIME_UTC}\n")));
    assert!(time.contains(&format!("#define TIME_MONOTONIC {TIME_MONOTONIC}\n")));
    let unistd = include_str!("../include/unistd.h");
    assert!(unistd.contains(&format!("#define GETENTROPY_MAX {GETENTROPY_MAX}\n")));
    let elf = include_str!("../include/elf.h");
    use crate::dlfcn::{PF_R, PF_X, PT_GNU_EH_FRAME, PT_LOAD};
    assert!(elf.contains(&format!("#define PT_GNU_EH_FRAME {PT_GNU_EH_FRAME:#x}\n")));
    assert!(elf.contains(&format!("#define PT_LOAD {PT_LOAD}\n")));
    assert!(elf.contains(&format!("#define PF_X {PF_X}\n")) && elf.contains(&format!("#define PF_R {PF_R}\n")));
}

#[test]
fn lldiv_truncates_toward_zero() {
    assert_eq!(lldiv(-7, 2), lldiv_t { quot: -3, rem: -1 });
    assert_eq!(lldiv(i64::MAX, 10).rem, 7);
}

static ORDER: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());

unsafe extern "C" fn record(arg: *mut c_void) {
    ORDER.lock().unwrap().push(arg as usize);
}

unsafe extern "C" fn record_and_register(arg: *mut c_void) {
    ORDER.lock().unwrap().push(arg as usize);
    // Registrace behem behu seznamu probehne take.
    unsafe { __cxa_atexit(record, 99 as *mut c_void, 0x2000 as *mut c_void) };
}

#[test]
fn cxa_atexit_reverse_order_and_finalize_by_module() {
    let (a, b) = (0x1000 as *mut c_void, 0x2000 as *mut c_void);
    unsafe {
        __cxa_atexit(record, 1 as *mut c_void, a);
        __cxa_atexit(record, 2 as *mut c_void, b);
        __cxa_atexit(record_and_register, 3 as *mut c_void, b);
        __cxa_atexit(record, 4 as *mut c_void, a);
        // jen modul b (dlclose knihovny), od posledni registrace
        __cxa_finalize(b);
    }
    assert_eq!(*ORDER.lock().unwrap(), [3, 99, 2]);
    ORDER.lock().unwrap().clear();
    // modul a zustal; vse ostatni (NULL) v obracenem poradi
    unsafe { __cxa_finalize(core::ptr::null_mut()) };
    assert_eq!(*ORDER.lock().unwrap(), [4, 1]);
}

static THREAD_ORDER: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());

unsafe extern "C" fn thread_dtor(arg: *mut c_void) {
    THREAD_ORDER.lock().unwrap().push(arg as usize);
}

#[test]
fn thread_dtors_run_per_thread_in_reverse() {
    std::thread::spawn(|| unsafe {
        __cxa_thread_atexit_impl(thread_dtor, 1 as *mut c_void, core::ptr::null_mut());
        __cxa_thread_atexit_impl(thread_dtor, 2 as *mut c_void, core::ptr::null_mut());
        run_thread_dtors();
        // podruhe uz nic
        run_thread_dtors();
    })
    .join()
    .unwrap();
    assert_eq!(*THREAD_ORDER.lock().unwrap(), [2, 1]);
}

#[test]
fn statvfs_layout_and_values() {
    use crate::statvfs::*;
    assert_eq!(core::mem::size_of::<statvfs>(), 11 * 8);
    let v = from_space((4096, 100, 150));
    assert_eq!((v.f_bsize, v.f_frsize, v.f_blocks, v.f_bfree, v.f_bavail), (4096, 4096, 100, 100, 100));
    let h = include_str!("../include/sys/statvfs.h");
    assert!(h.contains("unsigned long f_namemax;\n};"));
    let mut out = statvfs::default();
    let dir = std::env::temp_dir();
    let p = std::ffi::CString::new(dir.to_str().unwrap()).unwrap();
    unsafe {
        assert_eq!(crate::statvfs::statvfs(p.as_ptr().cast(), &mut out), 0);
        assert!(out.f_blocks > 0);
        let missing = std::ffi::CString::new("/neexistuje/nikde").unwrap();
        assert_eq!(crate::statvfs::statvfs(missing.as_ptr().cast(), &mut out), -1);
    }
}
