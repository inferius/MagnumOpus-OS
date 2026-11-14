//! `fenv.h`: prostredi plovouci carky (priznaky vyjimek, zaokrouhleni,
//! ulozeni a obnova). Hodnoty a rozlozeni `fenv_t` jako glibc / musl na
//! dane architekture (`include/fenv.h`):
//!
//! - x86_64: x87 (`fnstenv`/`fldenv`, 28 B) + SSE (`MXCSR`, 4 B). Priznaky
//!   a zaokrouhleni se nastavuji v obou (C i Rust pocitaji v SSE,
//!   `long double` v x87); priznaky se ctou jako OR obou.
//! - aarch64: `FPCR` (zaokrouhleni, bity 22-23) a `FPSR` (priznaky, bity
//!   0-4).
//!
//! Pasti vyjimek (FE_* v masce) se nezapinaji: `feenableexcept` neni
//! (rozsireni GNU) a `fesetenv` z cizich hodnot pasti nezapne.

use crate::c_int;

#[cfg(target_arch = "x86_64")]
mod arch {
    use core::arch::asm;

    pub const FE_INVALID: u32 = 1;
    pub const FE_DIVBYZERO: u32 = 4;
    pub const FE_OVERFLOW: u32 = 8;
    pub const FE_UNDERFLOW: u32 = 16;
    pub const FE_INEXACT: u32 = 32;
    /// Denormalni operand (x86 navic, soucast `FE_ALL_EXCEPT` jako v musl).
    pub const FE_DENORMAL: u32 = 2;
    pub const FE_ALL_EXCEPT: u32 = 63;
    pub const FE_TONEAREST: u32 = 0;
    pub const FE_DOWNWARD: u32 = 0x400;
    pub const FE_UPWARD: u32 = 0x800;
    pub const FE_TOWARDZERO: u32 = 0xc00;
    pub const ROUND_MASK: u32 = 0xc00;

    /// `fenv_t` (glibc x86_64): prostredi x87 + MXCSR.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct Env {
        pub x87: [u32; 7],
        pub mxcsr: u32,
    }

    fn mxcsr() -> u32 {
        let mut v: u32 = 0;
        // SAFETY: cteni MXCSR do lokalni promenne.
        unsafe { asm!("stmxcsr [{}]", in(reg) &raw mut v, options(nostack)) };
        v
    }

    fn set_mxcsr(v: u32) {
        // SAFETY: zapis MXCSR; pasti (bity 7-12) volajici nezapina.
        unsafe { asm!("ldmxcsr [{}]", in(reg) &raw const v, options(nostack)) };
    }

    fn x87_env() -> [u32; 7] {
        let mut e = [0u32; 7];
        // SAFETY: fnstenv zapise 28 B; fnstenv maskuje vyjimky x87, fldenv
        // je hned vrati.
        unsafe { asm!("fnstenv [{0}]", "fldenv [{0}]", in(reg) e.as_mut_ptr(), options(nostack)) };
        e
    }

    fn set_x87_env(e: &[u32; 7]) {
        // SAFETY: 28 B platneho prostredi x87.
        unsafe { asm!("fldenv [{}]", in(reg) e.as_ptr(), options(nostack)) };
    }

    pub fn test(mask: u32) -> u32 {
        let sw: u16;
        // SAFETY: cteni stavoveho slova x87.
        unsafe { asm!("fnstsw ax", out("ax") sw, options(nostack, nomem)) };
        (u32::from(sw) | mxcsr()) & mask & FE_ALL_EXCEPT
    }

    pub fn clear(mask: u32) {
        let mask = mask & FE_ALL_EXCEPT;
        let mut e = x87_env();
        e[1] &= !mask;
        set_x87_env(&e);
        set_mxcsr(mxcsr() & !mask);
    }

    pub fn raise(mask: u32) {
        // Priznaky v SSE (tam se pocita); vyjimky jsou maskovane, takze
        // jen priznak, ne past.
        set_mxcsr(mxcsr() | (mask & FE_ALL_EXCEPT));
    }

    pub fn round() -> u32 {
        // Zaokrouhleni MXCSR je v bitech 13-14, ve fenv_t jako x87 (10-11).
        (mxcsr() >> 3) & ROUND_MASK
    }

    pub fn set_round(r: u32) {
        let mut e = x87_env();
        e[0] = (e[0] & !ROUND_MASK) | r;
        set_x87_env(&e);
        set_mxcsr((mxcsr() & !(ROUND_MASK << 3)) | r << 3);
    }

    pub fn get() -> Env {
        Env { x87: x87_env(), mxcsr: mxcsr() }
    }

    pub fn set(e: &Env) {
        let mut x = e.x87;
        // Masky vyjimek vzdy zapnute (bez pasti).
        x[0] |= FE_ALL_EXCEPT;
        set_x87_env(&x);
        set_mxcsr(e.mxcsr | FE_ALL_EXCEPT << 7);
    }

    pub fn default_env() -> Env {
        // Kontrolni slovo 0x37f (vse maskovane, nejblizsi, plna presnost),
        // tagy prazdne; MXCSR 0x1f80.
        Env { x87: [0x37f, 0, 0xffff, 0, 0, 0, 0], mxcsr: 0x1f80 }
    }
}

#[cfg(target_arch = "aarch64")]
mod arch {
    use core::arch::asm;

    pub const FE_INVALID: u32 = 1;
    pub const FE_DIVBYZERO: u32 = 2;
    pub const FE_OVERFLOW: u32 = 4;
    pub const FE_UNDERFLOW: u32 = 8;
    pub const FE_INEXACT: u32 = 16;
    pub const FE_ALL_EXCEPT: u32 = 31;
    pub const FE_TONEAREST: u32 = 0;
    pub const FE_UPWARD: u32 = 0x40_0000;
    pub const FE_DOWNWARD: u32 = 0x80_0000;
    pub const FE_TOWARDZERO: u32 = 0xc0_0000;
    pub const ROUND_MASK: u32 = 0xc0_0000;

    /// `fenv_t` (musl / glibc aarch64): FPCR a FPSR.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct Env {
        pub fpcr: u32,
        pub fpsr: u32,
    }

    fn fpcr() -> u32 {
        let v: u64;
        // SAFETY: cteni systemoveho registru FP.
        unsafe { asm!("mrs {}, fpcr", out(reg) v, options(nostack, nomem)) };
        v as u32
    }

    fn set_fpcr(v: u32) {
        // SAFETY: zapis FPCR; bity pasti (8-12) volajici nezapina.
        unsafe { asm!("msr fpcr, {}", in(reg) u64::from(v), options(nostack, nomem)) };
    }

    fn fpsr() -> u32 {
        let v: u64;
        // SAFETY: cteni priznaku FP.
        unsafe { asm!("mrs {}, fpsr", out(reg) v, options(nostack, nomem)) };
        v as u32
    }

    fn set_fpsr(v: u32) {
        // SAFETY: zapis priznaku FP.
        unsafe { asm!("msr fpsr, {}", in(reg) u64::from(v), options(nostack, nomem)) };
    }

    pub fn test(mask: u32) -> u32 {
        fpsr() & mask & FE_ALL_EXCEPT
    }

    pub fn clear(mask: u32) {
        set_fpsr(fpsr() & !(mask & FE_ALL_EXCEPT));
    }

    pub fn raise(mask: u32) {
        set_fpsr(fpsr() | (mask & FE_ALL_EXCEPT));
    }

    pub fn round() -> u32 {
        fpcr() & ROUND_MASK
    }

    pub fn set_round(r: u32) {
        set_fpcr((fpcr() & !ROUND_MASK) | r);
    }

    pub fn get() -> Env {
        Env { fpcr: fpcr(), fpsr: fpsr() }
    }

    pub fn set(e: &Env) {
        // Bez pasti (bity 8-12 a 15 FPCR).
        set_fpcr(e.fpcr & !0x9f00);
        set_fpsr(e.fpsr);
    }

    pub fn default_env() -> Env {
        Env::default()
    }
}

pub use arch::*;

/// `FE_DFL_ENV` v hlavicce: `(const fenv_t *)-1`.
const DFL_ENV: usize = usize::MAX;

fn valid_round(r: u32) -> bool {
    matches!(r, FE_TONEAREST | FE_DOWNWARD | FE_UPWARD | FE_TOWARDZERO)
}

/// Prostredi z ukazatele (`FE_DFL_ENV` = vychozi).
///
/// # Safety
///
/// `env` je `FE_DFL_ENV` nebo platny ukazatel na `fenv_t`.
unsafe fn env_from(env: *const Env) -> Env {
    if env as usize == DFL_ENV {
        default_env()
    } else {
        // SAFETY: platny podle smlouvy.
        unsafe { *env }
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn feclearexcept(e: c_int) -> c_int {
    clear(e as u32);
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn feraiseexcept(e: c_int) -> c_int {
    raise(e as u32);
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn fetestexcept(e: c_int) -> c_int {
    test(e as u32) as c_int
}

/// # Safety
///
/// `flag` je platny ukazatel na `fexcept_t` (unsigned short / unsigned int).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fegetexceptflag(flag: *mut FexceptT, e: c_int) -> c_int {
    // SAFETY: platny podle smlouvy.
    unsafe { *flag = test(e as u32) as FexceptT };
    0
}

/// # Safety
///
/// `flag` je platny ukazatel na `fexcept_t`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fesetexceptflag(flag: *const FexceptT, e: c_int) -> c_int {
    let mask = e as u32 & FE_ALL_EXCEPT;
    // SAFETY: platny podle smlouvy.
    let want = u32::from(unsafe { *flag }) & mask;
    clear(mask & !want);
    raise(want);
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn fegetround() -> c_int {
    round() as c_int
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn fesetround(r: c_int) -> c_int {
    let r = r as u32;
    if !valid_round(r) {
        return -1;
    }
    set_round(r);
    0
}

/// # Safety
///
/// `env` je platny ukazatel na `fenv_t`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fegetenv(env: *mut Env) -> c_int {
    // SAFETY: platny podle smlouvy.
    unsafe { *env = get() };
    0
}

/// # Safety
///
/// `env` je platny ukazatel na `fenv_t`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn feholdexcept(env: *mut Env) -> c_int {
    // SAFETY: platny podle smlouvy.
    unsafe { *env = get() };
    clear(FE_ALL_EXCEPT);
    0
}

/// # Safety
///
/// `env` je `FE_DFL_ENV` nebo platny ukazatel na `fenv_t`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fesetenv(env: *const Env) -> c_int {
    // SAFETY: predano volajicim.
    set(&unsafe { env_from(env) });
    0
}

/// Obnovi prostredi a znovu vyvola priznaky nastale mezitim.
///
/// # Safety
///
/// `env` je `FE_DFL_ENV` nebo platny ukazatel na `fenv_t`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn feupdateenv(env: *const Env) -> c_int {
    let raised = test(FE_ALL_EXCEPT);
    // SAFETY: predano volajicim.
    set(&unsafe { env_from(env) });
    raise(raised);
    0
}

/// `fexcept_t` (x86_64 unsigned short, aarch64 unsigned int).
#[cfg(target_arch = "x86_64")]
pub type FexceptT = u16;
#[cfg(target_arch = "aarch64")]
pub type FexceptT = u32;
