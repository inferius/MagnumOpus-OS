//! `locale.h`: locale procesu a vlaken (`setlocale`, `newlocale`,
//! `uselocale`, ...), `localeconv`, `nl_langinfo` a varianty `*_l`.
//!
//! Locale ma dve kodovani znaku:
//!
//! - `C` / `POSIX`: jednobajtove (`MB_CUR_MAX` = 1), bajty 0x80-0xFF se
//!   v sirokych znacich vratne mapuji na U+DF80-U+DFFF (jako musl),
//! - `C.UTF-8` a kazde jine jmeno bez ciziho kodovani (`cs_CZ.UTF-8`,
//!   `en_US`, ...): UTF-8.
//!
//! Kategorie se zatim nelokalizuji (cisla, datum, razeni jsou vsude jako
//! v locale C); jmeno s jinym kodovanim nez UTF-8 (`cs_CZ.ISO-8859-2`) se
//! odmitne, aby program necekal jine kodovani, nez dostane. Kodovani
//! rozhoduje jen kategorie `LC_CTYPE`, ostatni si jmeno jen pamatuji
//! (`setlocale(..., NULL)`).
//!
//! Proces zacina v locale C (standard). `uselocale` nastavi locale vlakna
//! (jinak plati globalni ze `setlocale`).

use core::cell::Cell;
use core::ptr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use cstd_core::mb::{self, Enc};

use crate::{c_bytes, c_char, c_int, errno, size_t};

mod langinfo;
pub mod xl;

pub use langinfo::{langinfo, lconv, localeconv, nl, nl_langinfo, nl_langinfo_l};

pub const LC_CTYPE: c_int = 0;
pub const LC_NUMERIC: c_int = 1;
pub const LC_TIME: c_int = 2;
pub const LC_COLLATE: c_int = 3;
pub const LC_MONETARY: c_int = 4;
pub const LC_MESSAGES: c_int = 5;
pub const LC_ALL: c_int = 6;
/// Pocet kategorii (bez `LC_ALL`).
pub const CATS: usize = 6;
/// `LC_ALL_MASK` (vsechny kategorie; `LC_*_MASK` = `1 << LC_*`).
pub const LC_ALL_MASK: c_int = (1 << CATS) - 1;

/// Jmena promennych prostredi kategorii (poradi `LC_*`).
const ENV_NAMES: [&str; CATS] = ["LC_CTYPE", "LC_NUMERIC", "LC_TIME", "LC_COLLATE", "LC_MONETARY", "LC_MESSAGES"];

/// Locale: kodovani kazde kategorie (bit `i` = kategorie `i` je UTF-8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Locale {
    utf8: u8,
}

/// `locale_t` (v C nepruhledny ukazatel na `struct __locale_struct`).
pub type locale_t = *mut Locale;

/// `LC_GLOBAL_LOCALE`.
pub const LC_GLOBAL_LOCALE: locale_t = usize::MAX as locale_t;

/// Locale bez alokace: vsechno C, vsechno UTF-8. `freelocale` je nepusti.
static C_LOCALE: Locale = Locale { utf8: 0 };
static UTF8_LOCALE: Locale = Locale { utf8: LC_ALL_MASK as u8 };

/// Globalni locale (`setlocale`): bitova maska jako [`Locale::utf8`].
static GLOBAL: AtomicU8 = AtomicU8::new(0);

std::thread_local! {
    /// Locale vlakna z `uselocale` (nulovy = globalni).
    static CURRENT: Cell<*const Locale> = const { Cell::new(ptr::null()) };
}

impl Locale {
    fn enc(self, cat: usize) -> Enc {
        if self.utf8 & (1 << cat) != 0 { Enc::Utf8 } else { Enc::C }
    }

    /// Kodovani znaku (kategorie `LC_CTYPE`).
    pub fn ctype(self) -> Enc {
        self.enc(LC_CTYPE as usize)
    }

    /// Ukazatel na sdilene locale se stejnym obsahem (bez alokace), pokud je.
    fn shared(self) -> Option<locale_t> {
        match self.utf8 {
            0 => Some(&C_LOCALE as *const Locale as locale_t),
            m if m == LC_ALL_MASK as u8 => Some(&UTF8_LOCALE as *const Locale as locale_t),
            _ => None,
        }
    }
}

fn is_static(l: *const Locale) -> bool {
    ptr::eq(l, &C_LOCALE) || ptr::eq(l, &UTF8_LOCALE)
}

/// Locale `l` (vcetne `LC_GLOBAL_LOCALE`) jako hodnota.
///
/// # Safety
///
/// `l` je `LC_GLOBAL_LOCALE` nebo platne locale z `newlocale`/`duplocale`.
pub unsafe fn get(l: locale_t) -> Locale {
    if l.is_null() || l == LC_GLOBAL_LOCALE {
        return Locale { utf8: GLOBAL.load(Ordering::Acquire) };
    }
    // SAFETY: kontrakt funkce.
    unsafe { *l }
}

/// Aktualni locale vlakna (`uselocale`, jinak globalni).
pub fn current() -> Locale {
    let l = CURRENT.with(|c| c.get());
    if l.is_null() {
        Locale { utf8: GLOBAL.load(Ordering::Acquire) }
    } else {
        // SAFETY: `uselocale` uklada jen platna locale; uvolnit locale,
        // ktere nektere vlakno pouziva, program nesmi (POSIX).
        unsafe { *l }
    }
}

/// Kodovani znaku aktualniho locale vlakna.
pub fn ctype() -> Enc {
    current().ctype()
}

/// Od prvni zmeny locale se cstd-core (`printf`, `mbrtowc` v jadru
/// formatovani) pta na locale vlakna; do te doby plati C.
fn install_hook() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| mb::set_ctype_hook(ctype));
}

/// Kodovani podle jmena locale: `Some(true)` = UTF-8, `Some(false)` = C,
/// `None` = nezname kodovani (odmitnout). Prazdne jmeno = prostredi.
pub fn parse_name(name: &[u8]) -> Option<bool> {
    if name == b"C" || name == b"POSIX" {
        return Some(false);
    }
    // Kodovani za teckou (pred pripadnym `@modifikator`).
    let codeset = name.split(|&b| b == b'@').next().unwrap_or(b"");
    match codeset.iter().position(|&b| b == b'.') {
        None => Some(true),
        Some(dot) => {
            let cs: Vec<u8> = codeset[dot + 1..].iter().filter(|&&b| b != b'-' && b != b'_').map(u8::to_ascii_lowercase).collect();
            (cs == b"utf8").then_some(true)
        }
    }
}

/// Jmeno z prostredi pro kategorii `cat` (`LC_ALL`, `LC_<kat>`, `LANG`;
/// nic = `C.UTF-8`, system je UTF-8).
fn env_name(cat: usize) -> Vec<u8> {
    for var in ["LC_ALL", ENV_NAMES[cat], "LANG"] {
        if let Some(v) = std::env::var_os(var)
            && !v.is_empty()
        {
            return v.into_encoded_bytes();
        }
    }
    b"C.UTF-8".to_vec()
}

/// Kodovani kategorie `cat` podle jmena (`""` = prostredi).
fn resolve(name: &[u8], cat: usize) -> Option<bool> {
    if name.is_empty() { parse_name(&env_name(cat)) } else { parse_name(name) }
}

/// Jmeno kategorie pro `setlocale(..., NULL)`.
fn cat_name(utf8: bool) -> &'static [u8] {
    if utf8 { b"C.UTF-8\0" } else { b"C\0" }
}

/// Slozene jmeno `LC_ALL` pro masku (kategorie oddelene `;`, jako musl);
/// jednotne locale ma jednoduche jmeno.
fn all_name(mask: u8) -> &'static [u8] {
    static NAMES: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    let names = NAMES.get_or_init(|| {
        (0..=LC_ALL_MASK as u8)
            .map(|m| {
                if m == 0 || m == LC_ALL_MASK as u8 {
                    return cat_name(m != 0).to_vec();
                }
                let mut s = Vec::new();
                for c in 0..CATS {
                    if c > 0 {
                        s.push(b';');
                    }
                    let n = cat_name(m & (1 << c) != 0);
                    s.extend_from_slice(&n[..n.len() - 1]);
                }
                s.push(0);
                s
            })
            .collect()
    });
    &names[mask as usize]
}

/// Nova maska globalniho locale pro `setlocale(cat, name)`.
pub fn set_mask(old: u8, cat: c_int, name: &[u8]) -> Option<u8> {
    if cat == LC_ALL {
        // Slozene jmeno z `setlocale(LC_ALL, NULL)`: jmena kategorii.
        if name.contains(&b';') {
            let parts: Vec<&[u8]> = name.split(|&b| b == b';').collect();
            if parts.len() != CATS {
                return None;
            }
            let mut m = 0u8;
            for (c, p) in parts.iter().enumerate() {
                if resolve(p, c)? {
                    m |= 1 << c;
                }
            }
            return Some(m);
        }
        let mut m = 0u8;
        for c in 0..CATS {
            if resolve(name, c)? {
                m |= 1 << c;
            }
        }
        return Some(m);
    }
    let c = usize::try_from(cat).ok().filter(|&c| c < CATS)?;
    let utf8 = resolve(name, c)?;
    Some(if utf8 { old | (1 << c) } else { old & !(1 << c) })
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn setlocale(cat: c_int, name: *const c_char) -> *const c_char {
    if !(0..=LC_ALL).contains(&cat) {
        return ptr::null();
    }
    let mut mask = GLOBAL.load(Ordering::Acquire);
    if !name.is_null() {
        // SAFETY: retezec C.
        let n = unsafe { c_bytes(name) };
        let Some(m) = set_mask(mask, cat, n) else { return ptr::null() };
        install_hook();
        GLOBAL.store(m, Ordering::Release);
        mask = m;
    }
    if cat == LC_ALL {
        all_name(mask).as_ptr()
    } else {
        cat_name(mask & (1 << cat) != 0).as_ptr()
    }
}

/// Nove locale z masky kategorii a zakladu (bez prace s ukazateli).
pub fn new_mask(mask: c_int, name: &[u8], base: u8) -> Option<u8> {
    let mut m = base;
    for c in 0..CATS {
        if mask & (1 << c) != 0 {
            if resolve(name, c)? {
                m |= 1 << c;
            } else {
                m &= !(1 << c);
            }
        }
    }
    Some(m)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn newlocale(mask: c_int, name: *const c_char, base: locale_t) -> locale_t {
    if name.is_null() || mask & !LC_ALL_MASK != 0 || base == LC_GLOBAL_LOCALE {
        errno::set(errno::EINVAL);
        return ptr::null_mut();
    }
    // SAFETY: retezec C; `base` je nulovy nebo platne locale (kontrakt).
    let (n, b) = unsafe { (c_bytes(name), if base.is_null() { C_LOCALE } else { *base }) };
    let Some(m) = new_mask(mask, n, b.utf8) else {
        errno::set(errno::ENOENT);
        return ptr::null_mut();
    };
    install_hook();
    let l = Locale { utf8: m };
    // Zaklad z alokace se pouzije znovu (POSIX: `base` uz neplati).
    if !base.is_null() && !is_static(base) {
        // SAFETY: `base` je nase alokace z `newlocale`/`duplocale`.
        unsafe { *base = l };
        return base;
    }
    l.shared().unwrap_or_else(|| Box::into_raw(Box::new(l)))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn duplocale(l: locale_t) -> locale_t {
    if l.is_null() {
        errno::set(errno::EINVAL);
        return ptr::null_mut();
    }
    // SAFETY: kontrakt C.
    let v = unsafe { get(l) };
    v.shared().unwrap_or_else(|| Box::into_raw(Box::new(v)))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn freelocale(l: locale_t) {
    if l.is_null() || l == LC_GLOBAL_LOCALE || is_static(l) {
        return;
    }
    // SAFETY: alokace z `newlocale`/`duplocale` (kontrakt C).
    drop(unsafe { Box::from_raw(l) });
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn uselocale(new: locale_t) -> locale_t {
    let old = CURRENT.with(|c| c.get());
    let old = if old.is_null() { LC_GLOBAL_LOCALE } else { old as locale_t };
    if !new.is_null() {
        install_hook();
        let v = if new == LC_GLOBAL_LOCALE { ptr::null() } else { new as *const Locale };
        CURRENT.with(|c| c.set(v));
    }
    old
}

/// `MB_CUR_MAX` (makro v `stdlib.h`).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn __ctype_get_mb_cur_max() -> size_t {
    ctype().max_len()
}
