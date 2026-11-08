//! Vypocetni shader skladani: jedna vrstva (vyrez zdrojoveho bufferu) do
//! ciloveho bufferu snimku, presne jako `gfx::Canvas::draw_image`.
//!
//! Pixely jsou `u32` 0xAARRGGBB s premultiplikovanou alfou. Na pixel:
//! `s = scale(src, opacity)`, pak `copy` = `s`, jinak `over(dst, s)`:
//! - `scale(c, k)` = kazda slozka `(c * k + 128) >> 8`, pro `k >= 255` `c`,
//! - `over(d, s)` = `s` pro `sa == 255`, `d` pro `s == 0`, jinak
//!   `s + scale(d, 255 - sa)` po slozkach s nasycenim na 255.
//!
//! Zdroj se skaluje nejblizsim sousedem (`sx = lx * sw / dw`). Jedno
//! vlakno = jeden pixel orezu; skupina 8 x 8.
//!
//! Vazby (sada 0): 0 = cilovy buffer, 1 = zdrojovy buffer (oba pole `u32`,
//! SPIR-V 1.0 `Uniform` + `BufferBlock`). Push constants: [`Push`].

use alloc::vec::Vec;

use crate::op::*;
use crate::{Builder, Id};

/// Velikost skupiny (pixely).
pub const LOCAL: u32 = 8;

/// Push constants shaderu (vse `u32`, offsety po 4 B).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Push {
    /// Cilovy obdelnik vrstvy (cele, i mimo orez).
    pub dx: u32,
    pub dy: u32,
    pub dw: u32,
    pub dh: u32,
    /// Orez (cast ciloveho obdelniku, ktera se kresli): jedno vlakno na pixel.
    pub cx: u32,
    pub cy: u32,
    pub cw: u32,
    pub ch: u32,
    /// Krok radku cile (pixely).
    pub dst_stride: u32,
    /// Index pixelu leveho horniho rohu vyrezu ve zdroji.
    pub src_off: u32,
    /// Krok radku zdroje (pixely).
    pub src_stride: u32,
    /// Velikost vyrezu zdroje (skalovani).
    pub sw: u32,
    pub sh: u32,
    /// Pruhlednost 0..=255.
    pub opacity: u32,
    /// 1 = kopie (nepruhledny zdroj, `opacity` 255), 0 = prolinani.
    pub copy: u32,
}

pub const PUSH_WORDS: usize = 15;
pub const PUSH_SIZE: u32 = (PUSH_WORDS * 4) as u32;

impl Push {
    pub fn words(&self) -> [u32; PUSH_WORDS] {
        [
            self.dx,
            self.dy,
            self.dw,
            self.dh,
            self.cx,
            self.cy,
            self.cw,
            self.ch,
            self.dst_stride,
            self.src_off,
            self.src_stride,
            self.sw,
            self.sh,
            self.opacity,
            self.copy,
        ]
    }
}

/// Pomocnik pro telo shaderu (typy pri ruce).
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

    fn cmp(&mut self, opcode: u16, a: Id, x: Id) -> Id {
        let t = self.bool_t;
        self.b.op(opcode, t, &[a, x])
    }

    fn select(&mut self, cond: Id, a: Id, x: Id) -> Id {
        let u = self.u;
        self.b.op(SELECT, u, &[cond, a, x])
    }

    /// Slozka `v` na bitech `sh..sh+8`.
    fn channel(&mut self, v: Id, sh: u32) -> Id {
        let s = self.c(sh);
        let m = self.c(255);
        let x = self.bin(SHIFT_RIGHT_LOGICAL, v, s);
        self.bin(BITWISE_AND, x, m)
    }

    /// `scale(v, k)` po slozkach.
    fn scale(&mut self, v: Id, k: Id) -> Id {
        let mut acc = self.c(0);
        for sh in [0, 8, 16, 24] {
            let ch = self.channel(v, sh);
            let m = self.bin(I_MUL, ch, k);
            let r = self.c(128);
            let m = self.bin(I_ADD, m, r);
            let e = self.c(8);
            let m = self.bin(SHIFT_RIGHT_LOGICAL, m, e);
            let f = self.c(255);
            let m = self.bin(BITWISE_AND, m, f);
            let s = self.c(sh);
            let m = self.bin(SHIFT_LEFT_LOGICAL, m, s);
            acc = self.bin(BITWISE_OR, acc, m);
        }
        let full = self.c(255);
        let big = self.cmp(U_GREATER_THAN_EQUAL, k, full);
        self.select(big, v, acc)
    }

    /// Soucet po slozkach s nasycenim na 255.
    fn add_sat(&mut self, a: Id, x: Id) -> Id {
        let mut acc = self.c(0);
        for sh in [0, 8, 16, 24] {
            let ca = self.channel(a, sh);
            let cx = self.channel(x, sh);
            let sum = self.bin(I_ADD, ca, cx);
            let max = self.c(255);
            let over = self.cmp(U_GREATER_THAN, sum, max);
            let v = self.select(over, max, sum);
            let s = self.c(sh);
            let v = self.bin(SHIFT_LEFT_LOGICAL, v, s);
            acc = self.bin(BITWISE_OR, acc, v);
        }
        acc
    }
}

/// SPIR-V shaderu skladani (vstupni bod `main`).
pub fn shader() -> Vec<u32> {
    let mut b = Builder::new();
    let void = b.type_void();
    let u = b.type_uint();
    let bool_t = b.type_bool();
    let uvec3 = b.type_vector(u, 3);

    // Push constants: struktura PUSH_WORDS x u32.
    let members = [u; PUSH_WORDS];
    let push_t = b.type_struct(&members);
    b.decorate(push_t, DECORATION_BLOCK, &[]);
    for i in 0..PUSH_WORDS as u32 {
        b.member_decorate(push_t, i, DECORATION_OFFSET, &[i * 4]);
    }
    let push_ptr = b.type_pointer(STORAGE_PUSH_CONSTANT, push_t);
    let push = b.variable(push_ptr, STORAGE_PUSH_CONSTANT);
    let push_u = b.type_pointer(STORAGE_PUSH_CONSTANT, u);

    // Buffery: struct { u32 px[]; } BufferBlock, vazby 0 a 1.
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

    // gl_GlobalInvocationID.
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
    let cw = pc(&mut f, 6);
    let ch = pc(&mut f, 7);
    let in_x = f.cmp(U_LESS_THAN, gx, cw);
    let in_y = f.cmp(U_LESS_THAN, gy, ch);
    let inside = f.b.op(LOGICAL_AND, bool_t, &[in_x, in_y]);
    f.b.op_void(SELECTION_MERGE, &[merge, SELECTION_CONTROL_NONE]);
    f.b.op_void(BRANCH_CONDITIONAL, &[inside, body, merge]);

    f.b.label(body);
    let dx = pc(&mut f, 0);
    let dy = pc(&mut f, 1);
    let dw = pc(&mut f, 2);
    let dh = pc(&mut f, 3);
    let cx = pc(&mut f, 4);
    let cy = pc(&mut f, 5);
    let dst_stride = pc(&mut f, 8);
    let src_off = pc(&mut f, 9);
    let src_stride = pc(&mut f, 10);
    let sw = pc(&mut f, 11);
    let sh = pc(&mut f, 12);
    let opacity = pc(&mut f, 13);
    let copy = pc(&mut f, 14);
    // Cilovy pixel a jeho poloha ve vrstve.
    let px = f.bin(I_ADD, cx, gx);
    let py = f.bin(I_ADD, cy, gy);
    let lx = f.bin(I_SUB, px, dx);
    let ly = f.bin(I_SUB, py, dy);
    // Zdroj (nejblizsi soused): sx = lx * sw / dw.
    let t = f.bin(I_MUL, lx, sw);
    let sx = f.bin(U_DIV, t, dw);
    let t = f.bin(I_MUL, ly, sh);
    let sy = f.bin(U_DIV, t, dh);
    let t = f.bin(I_MUL, sy, src_stride);
    let t = f.bin(I_ADD, t, sx);
    let si = f.bin(I_ADD, t, src_off);
    let t = f.bin(I_MUL, py, dst_stride);
    let di = f.bin(I_ADD, t, px);
    let zero = f.c(0);
    let sp = f.b.op(ACCESS_CHAIN, elem_ptr, &[src, zero, si]);
    let s = f.b.op(LOAD, u, &[sp]);
    let dp = f.b.op(ACCESS_CHAIN, elem_ptr, &[dst, zero, di]);
    let d = f.b.op(LOAD, u, &[dp]);
    let sk = f.scale(s, opacity);
    // over(d, sk).
    let e = f.c(24);
    let sa = f.bin(SHIFT_RIGHT_LOGICAL, sk, e);
    let full = f.c(255);
    let inv = f.bin(I_SUB, full, sa);
    let sd = f.scale(d, inv);
    let sum = f.add_sat(sk, sd);
    let opaque = f.cmp(I_EQUAL, sa, full);
    let empty = f.cmp(I_EQUAL, sk, zero);
    let t = f.select(empty, d, sum);
    let over = f.select(opaque, sk, t);
    let one = f.c(1);
    let is_copy = f.cmp(I_EQUAL, copy, one);
    let out = f.select(is_copy, sk, over);
    f.b.op_void(STORE, &[dp, out]);
    f.b.op_void(BRANCH, &[merge]);

    f.b.label(merge);
    f.b.op_void(RETURN, &[]);
    b.end_function();
    b.finish()
}

/// Referencni vypocet jednoho pixelu (stejny jako shader a `gfx`), pro
/// testy na hostiteli.
pub fn reference(d: u32, s: u32, opacity: u32, copy: bool) -> u32 {
    let scale = |c: u32, k: u32| -> u32 {
        if k >= 255 {
            return c;
        }
        let mut out = 0;
        for sh in [0, 8, 16, 24] {
            out |= ((((c >> sh) & 255) * k + 128) >> 8 & 255) << sh;
        }
        out
    };
    let sk = scale(s, opacity);
    if copy {
        return sk;
    }
    let sa = sk >> 24;
    if sa == 255 {
        return sk;
    }
    if sk == 0 {
        return d;
    }
    let sd = scale(d, 255 - sa);
    let mut out = 0;
    for sh in [0, 8, 16, 24] {
        out |= (((sk >> sh) & 255) + ((sd >> sh) & 255)).min(255) << sh;
    }
    out
}
