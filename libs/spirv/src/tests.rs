//! Testy assembleru a shaderu skladani (`cargo test -p spirv`). Vystup se
//! uklada do `target/spirv-blend.spv` pro kontrolu `spirv-val` (ve WSL).

use alloc::vec::Vec;

use crate::blend::{self, Push};
use crate::*;

#[test]
fn header_and_strings() {
    let b = Builder::new();
    let m = b.finish();
    assert_eq!(&m[..5], &[MAGIC, VERSION_1_0, 0, 1, 0]);
    // OpCapability Shader, OpMemoryModel Logical GLSL450.
    assert_eq!(&m[5..7], &[2 << 16 | op::CAPABILITY as u32, op::CAPABILITY_SHADER]);
    assert_eq!(&m[7..10], &[3 << 16 | op::MEMORY_MODEL as u32, 0, 1]);
    assert_eq!(string_words("main"), [u32::from_le_bytes(*b"main"), 0]);
    assert_eq!(string_words("abc"), [u32::from_le_bytes(*b"abc\0")]);
}

#[test]
fn types_are_deduplicated() {
    let mut b = Builder::new();
    let u1 = b.type_uint();
    let u2 = b.type_uint();
    let c1 = b.const_u32(7);
    let c2 = b.const_u32(7);
    let c3 = b.const_u32(8);
    assert_eq!((u1, c1), (u2, c2));
    assert_ne!(c1, c3);
}

/// Instrukce modulu (opcode, operandy) a kontrola delek.
fn instructions(m: &[u32]) -> Vec<(u16, &[u32])> {
    let mut out = Vec::new();
    let mut i = 5;
    while i < m.len() {
        let n = (m[i] >> 16) as usize;
        assert!(n >= 1 && i + n <= m.len(), "spatna delka instrukce na slove {i}");
        out.push(((m[i] & 0xFFFF) as u16, &m[i + 1..i + n]));
        i += n;
    }
    out
}

#[test]
fn blend_shader_is_well_formed() {
    let m = blend::shader();
    let ins = instructions(&m);
    let bound = m[3];
    // Vsechna id vysledku jsou pod mezi a kazde je definovane jednou.
    let mut defined = alloc::collections::BTreeSet::new();
    for (opc, o) in &ins {
        let rid = match *opc {
            op::TYPE_VOID | op::TYPE_BOOL | op::TYPE_INT | op::TYPE_VECTOR | op::TYPE_RUNTIME_ARRAY
            | op::TYPE_STRUCT | op::TYPE_POINTER | op::TYPE_FUNCTION | op::LABEL => Some(o[0]),
            op::CONSTANT | op::VARIABLE | op::FUNCTION | op::LOAD | op::ACCESS_CHAIN | op::COMPOSITE_EXTRACT
            | op::I_ADD | op::I_SUB | op::I_MUL | op::U_DIV | op::SELECT | op::I_EQUAL | op::U_LESS_THAN
            | op::U_GREATER_THAN | op::U_GREATER_THAN_EQUAL | op::SHIFT_LEFT_LOGICAL | op::SHIFT_RIGHT_LOGICAL
            | op::BITWISE_AND | op::BITWISE_OR | op::LOGICAL_AND => Some(o[1]),
            _ => None,
        };
        if let Some(r) = rid {
            assert!(r < bound, "id {r} >= mez {bound}");
            assert!(defined.insert(r), "id {r} definovane dvakrat");
        }
    }
    // Poradi casti: capability, model pameti, vstupni bod, rezim, ... funkce.
    let pos = |o: u16| ins.iter().position(|(x, _)| *x == o).unwrap();
    assert!(pos(op::CAPABILITY) < pos(op::MEMORY_MODEL));
    assert!(pos(op::MEMORY_MODEL) < pos(op::ENTRY_POINT));
    assert!(pos(op::ENTRY_POINT) < pos(op::EXECUTION_MODE));
    assert!(pos(op::EXECUTION_MODE) < pos(op::DECORATE));
    assert!(pos(op::DECORATE) < pos(op::TYPE_VOID));
    assert!(ins.iter().rposition(|(x, _)| *x == op::CONSTANT).unwrap() < pos(op::FUNCTION));
    assert_eq!(ins.last().unwrap().0, op::FUNCTION_END);
    // Pro rucni kontrolu: spirv-val target/spirv-blend.spv
    let bytes: Vec<u8> = m.iter().flat_map(|w| w.to_le_bytes()).collect();
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/spirv-blend.spv");
    let _ = std::fs::write(path, bytes);
}

#[test]
fn reference_matches_gfx() {
    // Referencni pixel shaderu = gfx::Canvas::draw_image (scale + over).
    let mut x: u32 = 0x1234_5678;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    };
    for _ in 0..20000 {
        let d = next();
        // Premultiplikovany zdroj: slozky <= alfa (gfx vadny vstup snese,
        // ale aplikace posilaji premultiplied).
        let a = next() & 255;
        let s = a << 24 | (next() % (a + 1)) << 16 | (next() % (a + 1)) << 8 | next() % (a + 1);
        let k = match next() % 4 {
            0 => 255,
            1 => 0,
            _ => next() & 255,
        };
        let expect = gfx::over(d, gfx::scale(s, k));
        assert_eq!(blend::reference(d, s, k, false), expect, "d={d:#x} s={s:#x} k={k}");
    }
    assert_eq!(blend::reference(0x1111_1111, 0x8040_2010, 255, true), 0x8040_2010);
    let p = Push { dw: 4, ..Default::default() };
    assert_eq!(p.words().len(), blend::PUSH_WORDS);
}
