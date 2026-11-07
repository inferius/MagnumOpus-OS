//! DNS pro sluzbu `dnsd`: zpravy (RFC 1035, EDNS0) a overeni DNSSEC.
//!
//! - [`name`]: domenova jmena, komprese, kanonicke poradi.
//! - [`message`]: cteni a sestaveni zprav; RDATA bez komprese.
//! - [`dnssec`]: overeni podpisu RRSIG klici DNSKEY, retez DS od
//!   korenove kotvy, dukazy neexistence NSEC a NSEC3.
//!
//! Bez `std` a bez site: dotazy posila `dnsd`, knihovna jen cte a overuje
//! (testovatelne na hostiteli).

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod dnssec;
pub mod message;
pub mod name;
#[cfg(any(test, feature = "testzone"))]
pub mod testzone;

#[cfg(test)]
mod tests;

pub use message::{Message, Question, Record};
pub use name::Name;

/// Typy zaznamu.
pub mod rtype {
    pub const A: u16 = 1;
    pub const NS: u16 = 2;
    pub const CNAME: u16 = 5;
    pub const SOA: u16 = 6;
    pub const PTR: u16 = 12;
    pub const MX: u16 = 15;
    pub const TXT: u16 = 16;
    pub const AAAA: u16 = 28;
    pub const SRV: u16 = 33;
    pub const DNAME: u16 = 39;
    pub const OPT: u16 = 41;
    pub const DS: u16 = 43;
    pub const RRSIG: u16 = 46;
    pub const NSEC: u16 = 47;
    pub const DNSKEY: u16 = 48;
    pub const NSEC3: u16 = 50;
    pub const NSEC3PARAM: u16 = 51;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Poskozena zprava nebo zaznam.
    Malformed(&'static str),
    /// DNSSEC: podpis nebo dukaz neprosel (data jsou podvrzena nebo
    /// poskozena).
    Bogus(&'static str),
}

pub type Result<T> = core::result::Result<T, Error>;
