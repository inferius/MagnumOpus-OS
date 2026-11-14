//! `math.h` nad crate `libm` (port musl libm do Rustu, MIT/Apache;
//! rozhodnuti 3A) a prevody `strtod`, `strtof`, `atof`.
//!
//! Funkce pro `long double` (`sinl`, ...) a `strtold` jsou v C
//! (`c/math_ld.c`): ABI `long double` (x86_64 80 bitu v x87, aarch64
//! 128 bitu) Rust neumi; pocitaji v presnosti `double` (rozhodnuti 9A).

use crate::{c_char, c_int, c_long, errno};

/// `double f(double)` a `float f(float)` z libm jako symboly C.
macro_rules! unary {
    ($($name:ident),* $(,)?) => {$(
        #[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
        pub extern "C" fn $name(x: f64) -> f64 {
            libm::$name(x)
        }
    )*};
}

macro_rules! unary_f {
    ($($name:ident),* $(,)?) => {$(
        #[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
        pub extern "C" fn $name(x: f32) -> f32 {
            libm::$name(x)
        }
    )*};
}

macro_rules! binary {
    ($($name:ident),* $(,)?) => {$(
        #[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
        pub extern "C" fn $name(x: f64, y: f64) -> f64 {
            libm::$name(x, y)
        }
    )*};
}

macro_rules! binary_f {
    ($($name:ident),* $(,)?) => {$(
        #[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
        pub extern "C" fn $name(x: f32, y: f32) -> f32 {
            libm::$name(x, y)
        }
    )*};
}

unary!(
    acos, asin, atan, cos, sin, tan, acosh, asinh, atanh, cosh, sinh, tanh, exp, exp2, exp10, expm1, log, log10,
    log1p, log2, cbrt, sqrt, ceil, floor, trunc, round, rint, fabs, erf, erfc, tgamma, lgamma, j0, j1, y0, y1,
);
unary_f!(
    acosf, asinf, atanf, cosf, sinf, tanf, acoshf, asinhf, atanhf, coshf, sinhf, tanhf, expf, exp2f, exp10f,
    expm1f, logf, log10f, log1pf, log2f, cbrtf, sqrtf, ceilf, floorf, truncf, roundf, rintf, fabsf, erff, erfcf,
    tgammaf, lgammaf, j0f, j1f, y0f, y1f,
);
binary!(atan2, pow, fmod, hypot, fmin, fmax, fdim, copysign, remainder, nextafter);
binary_f!(atan2f, powf, fmodf, hypotf, fminf, fmaxf, fdimf, copysignf, remainderf, nextafterf);

/// `nearbyint` = `rint` (vyjimky FPU se nehlasi).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn nearbyint(x: f64) -> f64 {
    libm::rint(x)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn nearbyintf(x: f32) -> f32 {
    libm::rintf(x)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn fma(x: f64, y: f64, z: f64) -> f64 {
    libm::fma(x, y, z)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn fmaf(x: f32, y: f32, z: f32) -> f32 {
    libm::fmaf(x, y, z)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn frexp(x: f64, e: *mut c_int) -> f64 {
    let (m, ex) = libm::frexp(x);
    if !e.is_null() {
        // SAFETY: kontrakt C.
        unsafe { *e = ex };
    }
    m
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn frexpf(x: f32, e: *mut c_int) -> f32 {
    let (m, ex) = libm::frexpf(x);
    if !e.is_null() {
        // SAFETY: kontrakt C.
        unsafe { *e = ex };
    }
    m
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn ldexp(x: f64, e: c_int) -> f64 {
    libm::ldexp(x, e)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn ldexpf(x: f32, e: c_int) -> f32 {
    libm::ldexpf(x, e)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn scalbn(x: f64, e: c_int) -> f64 {
    libm::scalbn(x, e)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn scalbnf(x: f32, e: c_int) -> f32 {
    libm::scalbnf(x, e)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn modf(x: f64, ip: *mut f64) -> f64 {
    let (frac, int) = libm::modf(x);
    if !ip.is_null() {
        // SAFETY: kontrakt C.
        unsafe { *ip = int };
    }
    frac
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn modff(x: f32, ip: *mut f32) -> f32 {
    let (frac, int) = libm::modff(x);
    if !ip.is_null() {
        // SAFETY: kontrakt C.
        unsafe { *ip = int };
    }
    frac
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn remquo(x: f64, y: f64, q: *mut c_int) -> f64 {
    let (r, quo) = libm::remquo(x, y);
    if !q.is_null() {
        // SAFETY: kontrakt C.
        unsafe { *q = quo };
    }
    r
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn ilogb(x: f64) -> c_int {
    libm::ilogb(x)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn logb(x: f64) -> f64 {
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    if !x.is_finite() {
        return x.abs();
    }
    f64::from(libm::ilogb(x))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn jn(n: c_int, x: f64) -> f64 {
    libm::jn(n, x)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn yn(n: c_int, x: f64) -> f64 {
    libm::yn(n, x)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn sincos(x: f64, s: *mut f64, c: *mut f64) {
    let (a, b) = libm::sincos(x);
    // SAFETY: kontrakt C (rozsireni GNU).
    unsafe {
        *s = a;
        *c = b;
    }
}

/// Zaokrouhleni na cele cislo; mimo rozsah `long` je vysledek neurcity
/// (C), tady nasycene.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn lround(x: f64) -> c_long {
    libm::round(x) as c_long
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn llround(x: f64) -> i64 {
    libm::round(x) as i64
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn lrint(x: f64) -> c_long {
    libm::rint(x) as c_long
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn llrint(x: f64) -> i64 {
    libm::rint(x) as i64
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn lroundf(x: f32) -> c_long {
    libm::roundf(x) as c_long
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn lrintf(x: f32) -> c_long {
    libm::rintf(x) as c_long
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn nan(_tag: *const c_char) -> f64 {
    f64::NAN
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn nanf(_tag: *const c_char) -> f32 {
    f32::NAN
}

// --- prevody textu ---------------------------------------------------------

/// `strtod`: `ERANGE` pri preteceni (`HUGE_VAL`) i podteceni.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn strtod(p: *const c_char, end: *mut *mut c_char) -> f64 {
    if p.is_null() {
        return 0.0;
    }
    // SAFETY: retezec C.
    let s = unsafe { crate::c_bytes(p) };
    let (v, used, range) = cstd_core::stdlib::parse_float(s);
    if range {
        errno::set(errno::ERANGE);
    }
    if !end.is_null() {
        // SAFETY: `used` <= delka retezce.
        unsafe { *end = p.add(used) as *mut c_char };
    }
    v
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn strtof(p: *const c_char, end: *mut *mut c_char) -> f32 {
    // SAFETY: kontrakt C.
    let v = unsafe { strtod(p, end) };
    let f = v as f32;
    if (f.is_infinite() && v.is_finite()) || (f == 0.0 && v != 0.0) {
        errno::set(errno::ERANGE);
    }
    f
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn atof(p: *const c_char) -> f64 {
    // SAFETY: kontrakt C.
    unsafe { strtod(p, core::ptr::null_mut()) }
}
