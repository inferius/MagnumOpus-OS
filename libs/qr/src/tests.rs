//! Testy kodovani QR: funkcni vzory, format, kapacita, platnost kodu
//! Reed-Solomon a zpetne cteni dat (zrcadlovy postup ke kodovani).

use super::*;

#[test]
fn capacities_match_standard() {
    // ISO 18004 tab. 7, bajtovy rezim, uroven M.
    let expected = [14, 26, 42, 62, 84, 106, 122, 152, 180];
    for (v, &c) in expected.iter().enumerate() {
        assert_eq!(capacity(v + 1), c, "verze {}", v + 1);
    }
    assert!(encode(&[0u8; 181]).is_none());
}

#[test]
fn format_bits_known() {
    // Tabulka C.1: M, maska 0 = 101010000010010; M, maska 5 = 100000011001110.
    assert_eq!(format_bits(0), 0b101010000010010);
    assert_eq!(format_bits(5), 0b100000011001110);
}

#[test]
fn alignment_positions_known() {
    assert_eq!(alignment_positions(2).0[..2], [6, 18]);
    assert_eq!(alignment_positions(7).0[..3], [6, 22, 38]);
    assert_eq!(alignment_positions(9).0[..3], [6, 26, 46]);
}

/// Precte format z obou kopii a vrati masku (obe musi sedet).
fn read_mask(q: &Qr) -> u32 {
    let size = q.size;
    let mut a = 0u32;
    let mut b = 0u32;
    let mut a_bits = [(8, 0); 15];
    for i in 0..=5 {
        a_bits[i] = (8, i);
    }
    a_bits[6] = (8, 7);
    a_bits[7] = (8, 8);
    a_bits[8] = (7, 8);
    for i in 9..15 {
        a_bits[i] = (14 - i, 8);
    }
    for (i, &(x, y)) in a_bits.iter().enumerate() {
        a |= (q.get(x, y) as u32) << i;
    }
    for i in 0..8 {
        b |= (q.get(size - 1 - i, 8) as u32) << i;
    }
    for i in 8..15 {
        b |= (q.get(8, size - 15 + i) as u32) << i;
    }
    assert_eq!(a, b, "obe kopie formatu");
    let mask = (0..8).find(|&m| format_bits(m) == a).expect("platny format");
    assert!(q.get(8, size - 8), "tmavy modul");
    mask
}

/// Hodnota polynomu (kodove slovo bloku) v alpha^k.
fn eval(block: &[u8], k: usize) -> u8 {
    let mut x = 1u8;
    for _ in 0..k {
        x = gf_mul(x, 2);
    }
    block.iter().fold(0u8, |acc, &c| gf_mul(acc, x) ^ c)
}

#[test]
fn encodes_and_reads_back() {
    for len in [1usize, 14, 15, 40, 84, 100, 150, 180] {
        let data: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
        let q = encode(&data).unwrap();
        assert_eq!(q.size, 17 + 4 * q.version);
        assert!(capacity(q.version) >= len && (q.version == 1 || capacity(q.version - 1) < len));
        // Vyhledavaci vzory a casovani.
        for (cx, cy) in [(3, 3), (q.size - 4, 3), (3, q.size - 4)] {
            assert!(q.get(cx, cy) && !q.get(cx + 2, cy) && q.get(cx + 3, cy));
        }
        for i in 8..q.size - 8 {
            assert_eq!(q.get(i, 6), i % 2 == 0);
            assert_eq!(q.get(6, i), i % 2 == 0);
        }
        // Zpetne cteni: odmaskovat, cik-cak, rozlozit bloky.
        let mask = read_mask(&q);
        let mut u = q.clone();
        apply_mask(&mut u, mask);
        let raw = raw_modules(q.version) / 8;
        let mut cw = vec![0u8; raw];
        let mut i = 0usize;
        let mut right = q.size as i32 - 1;
        while right >= 1 {
            if right == 6 {
                right = 5;
            }
            for vert in 0..q.size {
                for j in 0..2 {
                    let x = (right - j) as usize;
                    let y = if (right + 1) & 2 == 0 { q.size - 1 - vert } else { vert };
                    if !u.is_func(x, y) && i < raw * 8 {
                        if u.get(x, y) {
                            cw[i >> 3] |= 0x80 >> (i & 7);
                        }
                        i += 1;
                    }
                }
            }
            right -= 2;
        }
        let blocks = BLOCKS[q.version - 1] as usize;
        let ecc = ECC_PER_BLOCK[q.version - 1] as usize;
        let short_blocks = blocks - raw % blocks;
        let short_len = raw / blocks;
        let mut blk: Vec<Vec<u8>> = vec![Vec::new(); blocks];
        let mut k = 0;
        for i in 0..=short_len {
            for (j, b) in blk.iter_mut().enumerate() {
                if i != short_len - ecc || j >= short_blocks {
                    b.push(cw[k]);
                    k += 1;
                }
            }
        }
        assert_eq!(k, raw);
        let mut stream = Vec::new();
        for b in &blk {
            // Kazdy blok je platne kodove slovo RS: nulove v alpha^0..ecc-1.
            for r in 0..ecc {
                assert_eq!(eval(b, r), 0, "RS blok, verze {}", q.version);
            }
            stream.extend_from_slice(&b[..b.len() - ecc]);
        }
        assert_eq!(stream[0] >> 4, 0b0100, "bajtovy rezim");
        let n = ((stream[0] as usize & 0xF) << 4) | (stream[1] as usize >> 4);
        assert_eq!(n, len);
        let got: Vec<u8> = (0..len)
            .map(|i| (stream[1 + i] << 4) | (stream[2 + i] >> 4))
            .collect();
        assert_eq!(got, data);
    }
}
