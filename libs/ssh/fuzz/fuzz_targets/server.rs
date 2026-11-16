//! Server SSH pred prihlasenim: libovolne bajty od klienta po kouscich.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ssh::server::{Auth, Backend, ServerConn, Start};

struct Deny;

impl Backend for Deny {
    fn probe(&mut self, _: &str, _: &[u8]) -> bool {
        false
    }
    fn auth(&mut self, _: &str, _: &[u8], _: &[u8], _: &[u8]) -> Auth {
        Auth::Denied
    }
    fn start(&mut self, _: u32, _: Start) -> bool {
        false
    }
    fn winch(&mut self, _: u32, _: u32, _: u32) {}
}

fuzz_target!(|data: &[u8]| {
    let Some((&split, data)) = data.split_first() else { return };
    let mut s = ServerConn::new(&[3; 32]).unwrap();
    let mut out = vec![0u8; 40_000];
    for c in data.chunks(1 + split as usize * 8) {
        let mut off = 0;
        for _ in 0..64 {
            if off >= c.len() {
                break;
            }
            match s.input(&c[off..]) {
                Ok(n) => off += n,
                Err(_) => return,
            }
            if s.progress(&mut Deny).is_err() {
                return;
            }
            while s.output(&mut out) > 0 {}
        }
    }
});
