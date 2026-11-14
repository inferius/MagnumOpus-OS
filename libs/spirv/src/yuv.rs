//! Vypocetni shader prevodu YUV 4:2:0 (NV12, I420) na BGRA, bit po bitu
//! stejny jako `libs/yuv` (BT.709, omezeny rozsah, 10 bitu zlomku):
//!
//! ```text
//! Yc = Y - 16, Uc = U - 128, Vc = V - 128
//! R = (1192 Yc + 1836 Vc + 512) >> 10      (orez 0..255)
//! G = (1192 Yc - 218 Uc - 546 Vc + 512) >> 10
//! B = (1192 Yc + 2163 Uc + 512) >> 10
//! ```
//!
//! Shader pocita jen bez znamenka: k mezivysledku pricte `BIAS` (nasobek
//! 1024, vetsi nez nejzapornejsi hodnota), posune logicky o 10 bitu
//! a porovna s `BIAS >> 10`. To dava presne `floor(x / 1024)` jako
//! aritmeticky posun v `libs/yuv`.
//!
//! Vazby (sada 0, stejne rozlozeni jako `blend`): 0 = cil (pole `u32`,
//! pixely `0xFFRRGGBB`), 1 = zdroj (bajty YUV zabalene po 4 do `u32`,
//! little endian). Jedno vlakno = jeden pixel; skupina 8 x 8.

use alloc::vec::Vec;

use crate::op::*;
use crate::{Builder, Id};

/// Velikost skupiny (pixely).
pub const LOCAL: u32 = 8;

/// Posun do kladnych cisel (nasobek 1024; |mezivysledek| < 2^21).
pub const BIAS: u32 = 1 << 24;

/// Push constants (vse `u32`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Push {
    pub w: u32,
    pub h: u32,
    /// Offset a krok roviny Y (bajty).
    pub y_off: u32,
    pub y_pitch: u32,
    /// Rovina U (NV12: prokladana UV).
    pub u_off: u32,
    pub u_pitch: u32,
    /// Rovina V (u NV12 nepouzita).
    pub v_off: u32,
    pub v_pitch: u32,
    /// 1 = NV12 (UV prokladane), 0 = I420.
    pub nv12: u32,
    /// Krok radku cile (pixely).
    pub dst_stride: u32,
}

pub const PUSH_WORDS: usize = 10;
pub const PUSH_SIZE: u32 = (PUSH_WORDS * 4) as u32;

impl Push {
    pub fn words(&self) -> [u32; PUSH_WORDS] {
        [self.w, self.h, self.y_off, self.y_pitch, self.u_off, self.u_pitch, self.v_off, self.v_pitch, self.nv12, self.dst_stride]
    }
}

struct F<'a> {
    b: &'a mut Builder,
    u: Id,
    bool_t: Id,
}

impl F<'_> {
    fn c(&mut self, v: u32) -> Id {
        self.b.const_u32(v)
    }

    fn bin(&mut self, opcode: u16, a: Id, x: Id) -> Id {
        let u = self.u;
        self.b.op(opcode, u, &[a, x])
    }

    fn bin_c(&mut self, opcode: u16, a: Id, v: u32) -> Id {
        let x = self.c(v);
        self.bin(opcode, a, x)
    }

    fn cmp(&mut self, opcode: u16, a: Id, x: Id) -> Id {
        let t = self.bool_t;
        self.b.op(opcode, t, &[a, x])
    }

    fn select(&mut self, cond: Id, a: Id, x: Id) -> Id {
        let u = self.u;
        self.b.op(SELECT, u, &[cond, a, x])
    }

    /// Slozka z `x + BIAS` (uz s pripoctenym zaokrouhlenim): posun o 10,
    /// orez na 0..255.
    fn channel(&mut self, biased: Id) -> Id {
        let s = self.bin_c(SHIFT_RIGHT_LOGICAL, biased, 10);
        let lo = self.c(BIAS >> 10);
        let hi = self.c((BIAS >> 10) + 255);
        let under = self.cmp(U_LESS_THAN, s, lo);
        let over = self.cmp(U_GREATER_THAN, s, hi);
        let v = self.bin(I_SUB, s, lo);
        let zero = self.c(0);
        let max = self.c(255);
        let v = self.select(over, max, v);
        self.select(under, zero, v)
    }
}

/// SPIR-V shaderu prevodu (vstupni bod `main`).
pub fn shader() -> Vec<u32> {
    let mut b = Builder::new();
    let void = b.type_void();
    let u = b.type_uint();
    let bool_t = b.type_bool();
    let uvec3 = b.type_vector(u, 3);

    let members = [u; PUSH_WORDS];
    let push_t = b.type_struct(&members);
    b.decorate(push_t, DECORATION_BLOCK, &[]);
    for i in 0..PUSH_WORDS as u32 {
        b.member_decorate(push_t, i, DECORATION_OFFSET, &[i * 4]);
    }
    let push_ptr = b.type_pointer(STORAGE_PUSH_CONSTANT, push_t);
    let push = b.variable(push_ptr, STORAGE_PUSH_CONSTANT);
    let push_u = b.type_pointer(STORAGE_PUSH_CONSTANT, u);

    let arr = b.type_runtime_array(u);
    b.decorate(arr, DECORATION_ARRAY_STRIDE, &[4]);
    let buf_t = b.type_struct(&[arr]);
    b.decorate(buf_t, DECORATION_BUFFER_BLOCK, &[]);
    b.member_decorate(buf_t, 0, DECORATION_OFFSET, &[0]);
    let buf_ptr = b.type_pointer(STORAGE_UNIFORM, buf_t);
    let dst = b.variable(buf_ptr, STORAGE_UNIFORM);
    let src = b.variable(buf_ptr, STORAGE_UNIFORM);
    for (v, binding) in [(dst, 0), (src, 1)] {
        b.decorate(v, DECORATION_DESCRIPTOR_SET, &[0]);
        b.decorate(v, DECORATION_BINDING, &[binding]);
    }
    let elem_ptr = b.type_pointer(STORAGE_UNIFORM, u);

    let in_ptr = b.type_pointer(STORAGE_INPUT, uvec3);
    let gid_var = b.variable(in_ptr, STORAGE_INPUT);
    b.decorate(gid_var, DECORATION_BUILT_IN, &[BUILT_IN_GLOBAL_INVOCATION_ID]);

    let fn_t = b.type_function(void, &[]);
    let main = b.begin_function(void, fn_t);
    b.entry_point(EXECUTION_MODEL_GL_COMPUTE, main, "main", &[gid_var]);
    b.execution_mode(main, EXECUTION_MODE_LOCAL_SIZE, &[LOCAL, LOCAL, 1]);

    let entry = b.id();
    let body = b.id();
    let merge = b.id();
    b.label(entry);
    let mut f = F { b: &mut b, u, bool_t };
    let gid = f.b.op(LOAD, uvec3, &[gid_var]);
    let gx = f.b.op(COMPOSITE_EXTRACT, u, &[gid, 0]);
    let gy = f.b.op(COMPOSITE_EXTRACT, u, &[gid, 1]);
    let pc = |f: &mut F, i: u32| {
        let idx = f.c(i);
        let p = f.b.op(ACCESS_CHAIN, push_u, &[push, idx]);
        f.b.op(LOAD, u, &[p])
    };
    let w = pc(&mut f, 0);
    let h = pc(&mut f, 1);
    let in_x = f.cmp(U_LESS_THAN, gx, w);
    let in_y = f.cmp(U_LESS_THAN, gy, h);
    let inside = f.b.op(LOGICAL_AND, bool_t, &[in_x, in_y]);
    f.b.op_void(SELECTION_MERGE, &[merge, SELECTION_CONTROL_NONE]);
    f.b.op_void(BRANCH_CONDITIONAL, &[inside, body, merge]);

    f.b.label(body);
    let y_off = pc(&mut f, 2);
    let y_pitch = pc(&mut f, 3);
    let u_off = pc(&mut f, 4);
    let u_pitch = pc(&mut f, 5);
    let v_off = pc(&mut f, 6);
    let v_pitch = pc(&mut f, 7);
    let nv12 = pc(&mut f, 8);
    let dst_stride = pc(&mut f, 9);
    let zero = f.c(0);
    // Bajt zdroje na indexu `i`.
    let byte = |f: &mut F, i: Id| {
        let wi = f.bin_c(SHIFT_RIGHT_LOGICAL, i, 2);
        let p = f.b.op(ACCESS_CHAIN, elem_ptr, &[src, zero, wi]);
        let word = f.b.op(LOAD, u, &[p]);
        let lane = f.bin_c(BITWISE_AND, i, 3);
        let sh = f.bin_c(SHIFT_LEFT_LOGICAL, lane, 3);
        let x = f.bin(SHIFT_RIGHT_LOGICAL, word, sh);
        f.bin_c(BITWISE_AND, x, 255)
    };
    // Y.
    let t = f.bin(I_MUL, gy, y_pitch);
    let t = f.bin(I_ADD, t, y_off);
    let yi = f.bin(I_ADD, t, gx);
    let yv = byte(&mut f, yi);
    // Radek a sloupec barvy.
    let cy = f.bin_c(SHIFT_RIGHT_LOGICAL, gy, 1);
    let cx = f.bin_c(SHIFT_RIGHT_LOGICAL, gx, 1);
    // NV12: u = u_off + cy * u_pitch + 2 cx, v = u + 1.
    let ur = f.bin(I_MUL, cy, u_pitch);
    let ur = f.bin(I_ADD, ur, u_off);
    let cx2 = f.bin_c(SHIFT_LEFT_LOGICAL, cx, 1);
    let ui_nv = f.bin(I_ADD, ur, cx2);
    let one = f.c(1);
    let vi_nv = f.bin(I_ADD, ui_nv, one);
    // I420: u = u_off + cy * u_pitch + cx, v = v_off + cy * v_pitch + cx.
    let ui_i = f.bin(I_ADD, ur, cx);
    let vr = f.bin(I_MUL, cy, v_pitch);
    let vr = f.bin(I_ADD, vr, v_off);
    let vi_i = f.bin(I_ADD, vr, cx);
    let is_nv = f.cmp(I_EQUAL, nv12, one);
    let ui = f.select(is_nv, ui_nv, ui_i);
    let vi = f.select(is_nv, vi_nv, vi_i);
    let uv = byte(&mut f, ui);
    let vv = byte(&mut f, vi);
    // Mezivysledky s BIAS (vse modulo 2^32, vysledek kladny):
    // base = 1192 (Y - 16) + 512 + BIAS = 1192 Y + (BIAS + 512 - 19072).
    let ym = f.bin_c(I_MUL, yv, 1192);
    let base = f.bin_c(I_ADD, ym, BIAS + 512 - 1192 * 16);
    // Uc a Vc jako (U - 128): k = coef * U - coef * 128.
    let rv = f.bin_c(I_MUL, vv, 1836);
    let r = f.bin(I_ADD, base, rv);
    let r = f.bin_c(I_SUB, r, 1836 * 128);
    let gu = f.bin_c(I_MUL, uv, 218);
    let gv = f.bin_c(I_MUL, vv, 546);
    let g = f.bin_c(I_ADD, base, 218 * 128 + 546 * 128);
    let g = f.bin(I_SUB, g, gu);
    let g = f.bin(I_SUB, g, gv);
    let bu = f.bin_c(I_MUL, uv, 2163);
    let bl = f.bin(I_ADD, base, bu);
    let bl = f.bin_c(I_SUB, bl, 2163 * 128);
    let r = f.channel(r);
    let g = f.channel(g);
    let bl = f.channel(bl);
    let r = f.bin_c(SHIFT_LEFT_LOGICAL, r, 16);
    let g = f.bin_c(SHIFT_LEFT_LOGICAL, g, 8);
    let px = f.bin(BITWISE_OR, r, g);
    let px = f.bin(BITWISE_OR, px, bl);
    let px = f.bin_c(BITWISE_OR, px, 0xFF00_0000);
    let t = f.bin(I_MUL, gy, dst_stride);
    let di = f.bin(I_ADD, t, gx);
    let dp = f.b.op(ACCESS_CHAIN, elem_ptr, &[dst, zero, di]);
    f.b.op_void(STORE, &[dp, px]);
    f.b.op_void(BRANCH, &[merge]);

    f.b.label(merge);
    f.b.op_void(RETURN, &[]);
    b.end_function();
    b.finish()
}

/// Stejny vypocet jako shader (bez znamenka, s `BIAS`) pro testy na
/// hostiteli: musi se shodovat s `libs/yuv::pixel`.
pub fn reference(y: u8, u: u8, v: u8) -> u32 {
    let (y, u, v) = (y as u32, u as u32, v as u32);
    let base = (y * 1192).wrapping_add(BIAS + 512 - 1192 * 16);
    let ch = |x: u32| {
        let s = x >> 10;
        let lo = BIAS >> 10;
        if s < lo {
            0
        } else if s > lo + 255 {
            255
        } else {
            s - lo
        }
    };
    let r = ch(base.wrapping_add(v * 1836).wrapping_sub(1836 * 128));
    let g = ch(base.wrapping_add(218 * 128 + 546 * 128).wrapping_sub(u * 218).wrapping_sub(v * 546));
    let b = ch(base.wrapping_add(u * 2163).wrapping_sub(2163 * 128));
    0xFF00_0000 | r << 16 | g << 8 | b
}
