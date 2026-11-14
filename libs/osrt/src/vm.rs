//! Sprava vlastniho adresniho prostoru pro `mmap` knihovny C (`cstd`):
//! obalka `libos::vm` a priznaku `abi::vm` (cstd nema libos primo).

pub use abi::vm::region;
pub use abi::vm::{MAP_EXECUTE, MAP_READ, MAP_WRITE, RegionInfo};
pub use libos::vm::{alloc, commit, decommit, page_size, protect, query, release, reserve, reserve_at, seal};
