//! Program v C nad knihovnou cstd (zdroje v `c/`, preklad v build.rs).
//! Rust tu jen pripoji knihovnu C; `main` je v C a vola ho osrt.

#![no_main]

extern crate cstd;
