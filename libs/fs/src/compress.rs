//! Komprese kusu dat (faze 3).
//!
//! Kus = nejvys `COMP_CHUNK` logickych bloku jednoho souboru (128 KiB).
//! Ulozi se komprimovany jen tehdy, kdyz usetri aspon 1/8 bloku (aspon
//! jeden blok) - jinak (fotky, video, archivy) zustane nekomprimovany
//! a cteni neplati za rozbaleni.
//!
//! - `Lz4`: blokovy format LZ4 (lz4_flex), bez ramce a vlastnich souctu -
//!   integritu hlida soucet extentu nad daty na disku.
//! - `Zstd`: ramec Zstd (ruzstd, uroven "Fastest" ~ zstd -1).

use alloc::vec;
use alloc::vec::Vec;

use abi::{Error, Result};

use crate::layout::{BLOCK_SIZE, Compress};

/// Kolik fyzickych bloku smi komprimovany kus `blocks` bloku zabrat, aby se
/// vyplatil (usetrit aspon 1/8, nejmene jeden blok).
pub fn max_packed_blocks(blocks: usize) -> usize {
    blocks - (blocks / 8).max(1)
}

/// Zkomprimuje `data` (cele bloky). Vraci data doplnena nulami na cele
/// bloky a skutecnou delku, nebo `None`, kdyz se komprese nevyplati.
pub fn pack(algo: Compress, data: &[u8]) -> Option<(Vec<u8>, usize)> {
    let blocks = data.len() / BLOCK_SIZE;
    if blocks < 2 {
        return None;
    }
    let limit = max_packed_blocks(blocks) * BLOCK_SIZE;
    let mut out = match algo {
        Compress::Lz4 => {
            let mut out = vec![0u8; lz4_flex::block::get_maximum_output_size(data.len())];
            let n = lz4_flex::block::compress_into(data, &mut out).ok()?;
            out.truncate(n);
            out
        }
        Compress::Zstd => ruzstd::encoding::compress_to_vec(data, ruzstd::encoding::CompressionLevel::Fastest),
    };
    let bytes = out.len();
    if bytes == 0 || bytes > limit {
        return None;
    }
    out.resize(bytes.next_multiple_of(BLOCK_SIZE), 0);
    Some((out, bytes))
}

/// Rozbali `packed[..bytes]` na presne `out_len` bajtu. Cokoli jineho
/// (poskozeny proud, jina delka) = `Corrupted`.
pub fn unpack(algo: Compress, packed: &[u8], bytes: usize, out_len: usize) -> Result<Vec<u8>> {
    let src = packed.get(..bytes).ok_or(Error::Corrupted)?;
    let mut out = vec![0u8; out_len];
    let n = match algo {
        Compress::Lz4 => lz4_flex::block::decompress_into(src, &mut out).map_err(|_| Error::Corrupted)?,
        Compress::Zstd => {
            let mut d = ruzstd::decoding::FrameDecoder::new();
            d.decode_all(src, &mut out).map_err(|_| Error::Corrupted)?
        }
    };
    if n != out_len {
        return Err(Error::Corrupted);
    }
    Ok(out)
}
