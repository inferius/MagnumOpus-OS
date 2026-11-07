//! HPACK (RFC 7541): komprese hlavicek HTTP/2.
//!
//! - Dekoder: vsechny reprezentace, dynamicka tabulka, Huffman. Limity:
//!   velikost tabulky (nase `SETTINGS_HEADER_TABLE_SIZE`) a celkova
//!   velikost dekodovanych hlavicek (ochrana proti "HPACK bomb": kratky
//!   odkaz na velkou polozku tabulky opakovany mnohokrat).
//! - Koder: literaly bez indexovani a bez Huffmana (vzdy platne, server
//!   nemusi nic ukladat; pozadavek klienta je maly).

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;

use super::huffman;
use crate::{Error, Header, Result, protocol};

/// Staticka tabulka (RFC 7541, priloha A), index 1..=61.
pub const STATIC: [(&str, &str); 61] = [
    (":authority", ""),
    (":method", "GET"),
    (":method", "POST"),
    (":path", "/"),
    (":path", "/index.html"),
    (":scheme", "http"),
    (":scheme", "https"),
    (":status", "200"),
    (":status", "204"),
    (":status", "206"),
    (":status", "304"),
    (":status", "400"),
    (":status", "404"),
    (":status", "500"),
    ("accept-charset", ""),
    ("accept-encoding", "gzip, deflate"),
    ("accept-language", ""),
    ("accept-ranges", ""),
    ("accept", ""),
    ("access-control-allow-origin", ""),
    ("age", ""),
    ("allow", ""),
    ("authorization", ""),
    ("cache-control", ""),
    ("content-disposition", ""),
    ("content-encoding", ""),
    ("content-language", ""),
    ("content-length", ""),
    ("content-location", ""),
    ("content-range", ""),
    ("content-type", ""),
    ("cookie", ""),
    ("date", ""),
    ("etag", ""),
    ("expect", ""),
    ("expires", ""),
    ("from", ""),
    ("host", ""),
    ("if-match", ""),
    ("if-modified-since", ""),
    ("if-none-match", ""),
    ("if-range", ""),
    ("if-unmodified-since", ""),
    ("last-modified", ""),
    ("link", ""),
    ("location", ""),
    ("max-forwards", ""),
    ("proxy-authenticate", ""),
    ("proxy-authorization", ""),
    ("range", ""),
    ("referer", ""),
    ("refresh", ""),
    ("retry-after", ""),
    ("server", ""),
    ("set-cookie", ""),
    ("strict-transport-security", ""),
    ("transfer-encoding", ""),
    ("user-agent", ""),
    ("vary", ""),
    ("via", ""),
    ("www-authenticate", ""),
];

/// Velikost polozky v tabulce (RFC 7541 4.1).
fn entry_size(name: &str, value: &str) -> usize {
    name.len() + value.len() + 32
}

/// Dekodovani Huffmanova kodu; konec musi byt vyplnen jednickami (nejvys
/// 7 bitu), EOS uprostred = chyba.
pub fn huffman_decode(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len() * 8 / 5);
    let mut code: u32 = 0;
    let mut len: usize = 0;
    for &byte in data {
        for bit in (0..8).rev() {
            code = (code << 1) | u32::from((byte >> bit) & 1);
            len += 1;
            if len > 30 {
                return Err(protocol("HPACK: neplatny Huffmanuv kod"));
            }
            let count = huffman::COUNT[len] as u32;
            if count == 0 {
                continue;
            }
            let first = huffman::FIRST[len];
            if code >= first && code - first < count {
                let sym = huffman::SYMBOLS[huffman::OFFSET[len] as usize + (code - first) as usize];
                if sym == 256 {
                    return Err(protocol("HPACK: EOS v retezci"));
                }
                out.push(sym as u8);
                code = 0;
                len = 0;
            }
        }
    }
    // Zbytek: nejvys 7 bitu, same jednicky (zacatek EOS).
    if len > 7 || code != (1u32 << len) - 1 {
        return Err(protocol("HPACK: neplatne zakonceni Huffmanova kodu"));
    }
    Ok(out)
}

/// Huffmanovo kodovani (pro testy a pripadne pouziti koderem).
pub fn huffman_encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut acc: u64 = 0;
    let mut bits = 0u32;
    for &b in data {
        acc = (acc << huffman::LENGTHS[b as usize]) | u64::from(huffman::CODES[b as usize]);
        bits += u32::from(huffman::LENGTHS[b as usize]);
        while bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    if bits > 0 {
        out.push(((acc << (8 - bits)) as u8) | ((1u8 << (8 - bits)) - 1));
    }
    out
}

/// Cislo s prefixem `prefix` bitu (RFC 7541 5.1). Vraci hodnotu a pocet
/// prectenych bajtu.
pub fn decode_int(data: &[u8], prefix: u8) -> Result<(usize, usize)> {
    let max = (1usize << prefix) - 1;
    let first = *data.first().ok_or_else(|| protocol("HPACK: chybi cislo"))? as usize & max;
    if first < max {
        return Ok((first, 1));
    }
    let mut value = max;
    let mut shift = 0u32;
    for (i, &b) in data[1..].iter().enumerate() {
        // Nejvys 4 pokracovaci bajty (hodnoty do ~2^28): vic je utok.
        if shift > 21 {
            return Err(protocol("HPACK: prilis velke cislo"));
        }
        value += ((b & 0x7F) as usize) << shift;
        shift += 7;
        if b & 0x80 == 0 {
            return Ok((value, i + 2));
        }
    }
    Err(protocol("HPACK: neukoncene cislo"))
}

pub fn encode_int(out: &mut Vec<u8>, flags: u8, prefix: u8, mut value: usize) {
    let max = (1usize << prefix) - 1;
    if value < max {
        out.push(flags | value as u8);
        return;
    }
    out.push(flags | max as u8);
    value -= max;
    while value >= 0x80 {
        out.push((value as u8 & 0x7F) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn encode_str(out: &mut Vec<u8>, s: &str) {
    encode_int(out, 0, 7, s.len());
    out.extend_from_slice(s.as_bytes());
}

/// Zakoduje hlavicky (literal bez indexovani, jmeno jako literal).
pub fn encode(headers: &[(&str, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, value) in headers {
        out.push(0x00);
        encode_str(&mut out, name);
        encode_str(&mut out, value);
    }
    out
}

pub struct Decoder {
    table: VecDeque<(String, String)>,
    size: usize,
    /// Nejvetsi velikost, kterou jsme serveru povolili.
    max_allowed: usize,
    /// Aktualni nejvetsi velikost (zmena instrukci v bloku).
    max_size: usize,
}

impl Decoder {
    pub fn new(max_allowed: usize) -> Self {
        Self {
            table: VecDeque::new(),
            size: 0,
            max_allowed,
            max_size: max_allowed,
        }
    }

    fn evict(&mut self) {
        while self.size > self.max_size {
            let (n, v) = self.table.pop_back().expect("velikost odpovida polozkam");
            self.size -= entry_size(&n, &v);
        }
    }

    fn insert(&mut self, name: String, value: String) {
        let size = entry_size(&name, &value);
        if size > self.max_size {
            // Vetsi nez cela tabulka: tabulka se vyprazdni (RFC 7541 4.4).
            self.table.clear();
            self.size = 0;
            return;
        }
        self.size += size;
        self.table.push_front((name, value));
        self.evict();
    }

    fn get(&self, index: usize) -> Result<(&str, &str)> {
        match index {
            0 => Err(protocol("HPACK: index 0")),
            1..=61 => Ok(STATIC[index - 1]),
            _ => self
                .table
                .get(index - 62)
                .map(|(n, v)| (n.as_str(), v.as_str()))
                .ok_or_else(|| protocol("HPACK: index mimo tabulku")),
        }
    }

    fn string(data: &[u8]) -> Result<(String, usize)> {
        let huff = data.first().is_some_and(|b| b & 0x80 != 0);
        let (len, n) = decode_int(data, 7)?;
        let raw = data.get(n..n + len).ok_or_else(|| protocol("HPACK: retezec za koncem bloku"))?;
        let bytes = if huff { huffman_decode(raw)? } else { raw.to_vec() };
        let s = String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
        Ok((s, n + len))
    }

    /// Dekoduje blok hlavicek. `max_bytes` = limit dekodovaneho seznamu
    /// (velikost podle RFC 9113 6.5.2: jmeno + hodnota + 32 na polozku).
    pub fn decode(&mut self, block: &[u8], max_bytes: usize, max_headers: usize) -> Result<Vec<Header>> {
        let mut out = Vec::new();
        let mut total = 0usize;
        let mut pos = 0;
        let mut may_resize = true;
        while pos < block.len() {
            let b = block[pos];
            let data = &block[pos..];
            let (name, value) = if b & 0x80 != 0 {
                let (index, n) = decode_int(data, 7)?;
                pos += n;
                let (name, value) = self.get(index)?;
                (String::from(name), String::from(value))
            } else if b & 0xE0 == 0x20 {
                // Zmena velikosti tabulky: jen na zacatku bloku.
                if !may_resize {
                    return Err(protocol("HPACK: zmena velikosti tabulky uprostred bloku"));
                }
                let (size, n) = decode_int(data, 5)?;
                if size > self.max_allowed {
                    return Err(protocol("HPACK: tabulka vetsi nez povolena"));
                }
                self.max_size = size;
                self.evict();
                pos += n;
                continue;
            } else {
                // Literal: s indexovanim (01), bez (0000), nikdy (0001).
                let (indexing, prefix) = if b & 0xC0 == 0x40 { (true, 6) } else { (false, 4) };
                let (index, n) = decode_int(data, prefix)?;
                pos += n;
                let name = if index == 0 {
                    let (s, n) = Self::string(&block[pos..])?;
                    pos += n;
                    s
                } else {
                    String::from(self.get(index)?.0)
                };
                let (value, n) = Self::string(&block[pos..])?;
                pos += n;
                if indexing {
                    self.insert(name.clone(), value.clone());
                }
                (name, value)
            };
            may_resize = false;
            total += entry_size(&name, &value);
            if total > max_bytes {
                return Err(Error::TooLarge("hlavicky odpovedi"));
            }
            if out.len() >= max_headers {
                return Err(Error::TooLarge("pocet hlavicek"));
            }
            out.push((name, value));
        }
        Ok(out)
    }
}
