//! Testy na hostiteli (`cargo test -p display`).

mod data;
mod dp;
mod edid;
mod hdmi;
mod timing;

/// Pomocnik: blok EDID se spravnym kontrolnim souctem.
pub fn fix_checksum(block: &mut [u8]) {
    let n = block.len();
    let sum = block[..n - 1].iter().fold(0u8, |a, &b| a.wrapping_add(b));
    block[n - 1] = 0u8.wrapping_sub(sum);
}
