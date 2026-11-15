//! `libc.a` pro sysroot C (`target/c-sysroot/<arch>`, docs/cs/cstd.md):
//! staticka knihovna s knihovnou C `cstd`, `std` a behovym prostredim
//! `osrt`. Program v C ji prilinkuje (`-lc`) a `_start` z osrt zavola jeho
//! `main`. Kod je v `cstd` (a rozhrani SDL `mosdl`); tady jen vynuceni
//! prilinkovani.

#![no_std]

extern crate cstd;
extern crate mosdl;
extern crate mosys;
