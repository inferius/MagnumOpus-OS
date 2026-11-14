//! `ctype.h`: tridy znaku (jen ASCII, locale "C"). Hodnoty mimo 0-255
//! (vcetne EOF = -1) do zadne tridy nepatri.

use crate::ctypes::c_int;

fn byte(c: c_int) -> Option<u8> {
    u8::try_from(c).ok()
}

macro_rules! class {
    ($($name:ident => $test:expr;)*) => {$(
        #[cfg_attr(not(test), unsafe(no_mangle))]
        pub extern "C" fn $name(c: c_int) -> c_int {
            byte(c).is_some_and($test) as c_int
        }
    )*};
}

class! {
    isdigit => |b: u8| b.is_ascii_digit();
    isxdigit => |b: u8| b.is_ascii_hexdigit();
    isalpha => |b: u8| b.is_ascii_alphabetic();
    isalnum => |b: u8| b.is_ascii_alphanumeric();
    isupper => |b: u8| b.is_ascii_uppercase();
    islower => |b: u8| b.is_ascii_lowercase();
    isspace => |b: u8| matches!(b, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r');
    isprint => |b: u8| (0x20..0x7F).contains(&b);
    isgraph => |b: u8| (0x21..0x7F).contains(&b);
    iscntrl => |b: u8| b < 0x20 || b == 0x7F;
    ispunct => |b: u8| b.is_ascii_punctuation();
    isblank => |b: u8| b == b' ' || b == 0x09;
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn toupper(c: c_int) -> c_int {
    byte(c).map_or(c, |b| b.to_ascii_uppercase() as c_int)
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn tolower(c: c_int) -> c_int {
    byte(c).map_or(c, |b| b.to_ascii_lowercase() as c_int)
}
