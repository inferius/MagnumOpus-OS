//! Rozbor klicu (blob, radek authorized_keys) a overeni podpisu.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ssh::keys::PublicKey;

fuzz_target!(|data: &[u8]| {
    let (a, b) = data.split_at(data.len() / 2);
    if let Ok(k) = PublicKey::from_blob(a) {
        let _ = k.verify(b"data", b, true);
        let _ = k.verify(b"data", b, false);
    }
    if let Ok(s) = core::str::from_utf8(data) {
        let _ = PublicKey::parse_line(s);
    }
});
