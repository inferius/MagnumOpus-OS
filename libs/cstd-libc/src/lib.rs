//! `libc.a` pro sysroot C (`target/c-sysroot/<arch>`, docs/cs/cstd.md):
//! staticka knihovna s knihovnou C `cstd`, `std` a behovym prostredim
//! `osrt`. Program v C ji prilinkuje (`-lc`) a `_start` z osrt zavola jeho
//! `main`. Kod je cely v `cstd`; tady jen vynuceni prilinkovani.

#![no_std]

extern crate cstd;
