// Vygenerovano `cargo xtask venus-gen` z libs/venus/xml (vk.xml a venus-protocol,
// licence v libs/venus/xml/README.md) a libs/venus/subset.txt. Neupravovat rucne.

//! Vygenerovana serializace Vulkanu pro Venus: typy a konstanty, struktury
//! (`encode`, `encode_partial`, `decode`) a prikazy (funkce v `cmd`,
//! odpovedi `*Reply`).

#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]
#![allow(dead_code, unused_imports, unused_mut, clippy::all)]

mod types1;
pub use types1::*;
mod types2;
pub use types2::*;
mod structs1;
pub use structs1::*;
mod structs2;
pub use structs2::*;
mod structs3;
pub use structs3::*;

/// Prikazy: kodovani a cteni odpovedi.
pub mod cmd {
    use super::*;
    #[path = "../cmds1.rs"]
    mod cmds1;
    pub use cmds1::*;
    #[path = "../cmds2.rs"]
    mod cmds2;
    pub use cmds2::*;
}
