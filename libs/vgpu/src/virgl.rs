//! Proud prikazu virgl (3D) pro `SUBMIT_3D`.
//!
//! Vlastni kodovani podle `virgl_protocol.h` a `virgl_hw.h` z virglrenderer
//! (MIT): kazdy prikaz je hlavicka `cmd | obj << 8 | delka << 16` (delka =
//! pocet nasledujicich dwordu) a jeho pole. Kodujeme jen to, co potrebuje
//! kompozitor: textury jako obdelniky s pruhlednosti, orezem (scissor)
//! a jednobarevne plochy.

use alloc::vec::Vec;

/// Prikazy kontextu (`enum virgl_context_cmd`).
pub mod ccmd {
    pub const CREATE_OBJECT: u32 = 1;
    pub const BIND_OBJECT: u32 = 2;
    pub const DESTROY_OBJECT: u32 = 3;
    pub const SET_VIEWPORT_STATE: u32 = 4;
    pub const SET_FRAMEBUFFER_STATE: u32 = 5;
    pub const SET_VERTEX_BUFFERS: u32 = 6;
    pub const CLEAR: u32 = 7;
    pub const DRAW_VBO: u32 = 8;
    pub const RESOURCE_INLINE_WRITE: u32 = 9;
    pub const SET_SAMPLER_VIEWS: u32 = 10;
    pub const SET_SCISSOR_STATE: u32 = 15;
    pub const BIND_SAMPLER_STATES: u32 = 18;
    pub const SET_SUB_CTX: u32 = 28;
    pub const CREATE_SUB_CTX: u32 = 29;
    pub const BIND_SHADER: u32 = 31;
}

/// Typy objektu (`enum virgl_object_type`).
pub mod obj {
    pub const BLEND: u32 = 1;
    pub const RASTERIZER: u32 = 2;
    pub const DSA: u32 = 3;
    pub const SHADER: u32 = 4;
    pub const VERTEX_ELEMENTS: u32 = 5;
    pub const SAMPLER_VIEW: u32 = 6;
    pub const SAMPLER_STATE: u32 = 7;
    pub const SURFACE: u32 = 8;
}

/// Formaty (`enum virgl_formats`).
pub mod format {
    pub const B8G8R8A8_UNORM: u32 = 1;
    pub const B8G8R8X8_UNORM: u32 = 2;
    pub const R32G32B32A32_FLOAT: u32 = 31;
    pub const R8_UNORM: u32 = 64;
}

/// Pouziti zdroje (`VIRGL_BIND_*`).
pub mod bind {
    pub const RENDER_TARGET: u32 = 1 << 1;
    pub const SAMPLER_VIEW: u32 = 1 << 3;
    pub const VERTEX_BUFFER: u32 = 1 << 4;
    pub const SCANOUT: u32 = 1 << 18;
}

/// Cil zdroje (`enum pipe_texture_target`).
pub mod target {
    pub const BUFFER: u32 = 0;
    pub const TEXTURE_2D: u32 = 2;
}

/// Typ shaderu (`enum pipe_shader_type`).
pub const SHADER_VERTEX: u32 = 0;
pub const SHADER_FRAGMENT: u32 = 1;
/// `PIPE_PRIM_TRIANGLE_STRIP`.
pub const PRIM_TRIANGLE_STRIP: u32 = 5;
/// `PIPE_CLEAR_COLOR0`.
pub const CLEAR_COLOR0: u32 = 1 << 2;

// Prolinani a vzorkovani (`p_defines.h`).
const BLENDFACTOR_ONE: u32 = 1;
const BLENDFACTOR_INV_SRC_ALPHA: u32 = 0x13;
const BLEND_ADD: u32 = 0;
const MASK_RGBA: u32 = 0xF;
const TEX_WRAP_CLAMP_TO_EDGE: u32 = 2;
const TEX_FILTER_LINEAR: u32 = 1;
const TEX_MIPFILTER_NONE: u32 = 2;

/// Vertex shader: poloha, souradnice textury a barva (nasobitel) projdou.
pub const VERTEX_SHADER: &str = "VERT\n\
DCL IN[0]\n\
DCL IN[1]\n\
DCL IN[2]\n\
DCL OUT[0], POSITION\n\
DCL OUT[1], GENERIC[0]\n\
DCL OUT[2], GENERIC[1]\n\
  0: MOV OUT[0], IN[0]\n\
  1: MOV OUT[1], IN[1]\n\
  2: MOV OUT[2], IN[2]\n\
  3: END\n";

/// Fragment shader: texel krat barva (premultiplied: barva = pruhlednost
/// ve vsech slozkach; jednobarevna plocha = bila textura 1x1 krat barva).
pub const FRAGMENT_SHADER: &str = "FRAG\n\
PROPERTY FS_COLOR0_WRITES_ALL_CBUFS 1\n\
DCL IN[0], GENERIC[0], PERSPECTIVE\n\
DCL IN[1], GENERIC[1], PERSPECTIVE\n\
DCL OUT[0], COLOR\n\
DCL SAMP[0]\n\
DCL SVIEW[0], 2D, FLOAT\n\
DCL TEMP[0]\n\
  0: TEX TEMP[0], IN[0], SAMP[0], 2D\n\
  1: MUL OUT[0], TEMP[0], IN[1]\n\
  2: END\n";

/// Velikost jednoho vrcholu: 3 x vec4 f32.
pub const VERTEX_BYTES: u32 = 48;

/// Sestavovani proudu prikazu.
#[derive(Default)]
pub struct Stream {
    d: Vec<u32>,
}

fn f(v: f32) -> u32 {
    v.to_bits()
}

impl Stream {
    pub fn new() -> Self {
        Self::default()
    }

    fn cmd(&mut self, cmd: u32, obj: u32, len: u32) -> &mut Self {
        self.d.push(cmd | obj << 8 | len << 16);
        self
    }

    fn w(&mut self, v: u32) -> &mut Self {
        self.d.push(v);
        self
    }

    pub fn words(&self) -> &[u32] {
        &self.d
    }

    pub fn len_bytes(&self) -> usize {
        self.d.len() * 4
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.d.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    pub fn reset(&mut self) {
        self.d.clear();
    }

    pub fn create_sub_ctx(&mut self, id: u32) -> &mut Self {
        self.cmd(ccmd::CREATE_SUB_CTX, 0, 1).w(id)
    }

    pub fn set_sub_ctx(&mut self, id: u32) -> &mut Self {
        self.cmd(ccmd::SET_SUB_CTX, 0, 1).w(id)
    }

    /// Prolinani pro premultiplied alfu: `src + dst * (1 - src_a)`.
    pub fn create_blend_premultiplied(&mut self, handle: u32) -> &mut Self {
        let s2 = 1
            | BLEND_ADD << 1
            | BLENDFACTOR_ONE << 4
            | BLENDFACTOR_INV_SRC_ALPHA << 9
            | BLEND_ADD << 14
            | BLENDFACTOR_ONE << 17
            | BLENDFACTOR_INV_SRC_ALPHA << 22
            | MASK_RGBA << 27;
        self.cmd(ccmd::CREATE_OBJECT, obj::BLEND, 11).w(handle).w(0).w(0).w(s2);
        for _ in 1..8 {
            self.w(0);
        }
        self
    }

    /// Hloubka a stencil vypnute.
    pub fn create_dsa_off(&mut self, handle: u32) -> &mut Self {
        self.cmd(ccmd::CREATE_OBJECT, obj::DSA, 5).w(handle).w(0).w(0).w(0).w(0)
    }

    /// Rasterizer s orezem (scissor), bez orezu ploch, stred pixelu v pulce.
    pub fn create_rasterizer(&mut self, handle: u32) -> &mut Self {
        let s0 = 1 << 1 /* depth_clip */ | 1 << 14 /* scissor */ | 1 << 29 /* half_pixel_center */;
        self.cmd(ccmd::CREATE_OBJECT, obj::RASTERIZER, 9)
            .w(handle)
            .w(s0)
            .w(f(1.0))
            .w(0)
            .w(0)
            .w(f(1.0))
            .w(0)
            .w(0)
            .w(0)
    }

    /// Shader v textove podobe TGSI (vcetne koncove nuly, zarovnane na dword).
    pub fn create_shader(&mut self, handle: u32, ty: u32, text: &str) -> &mut Self {
        let mut bytes: Vec<u8> = text.as_bytes().to_vec();
        bytes.push(0);
        let offlen = bytes.len() as u32;
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
        let words = bytes.len() as u32 / 4;
        // Hlavicka: handle, typ, delka textu, pocet tokenu (horni odhad pro
        // alokaci prekladace), pocet vystupu streamout = 0.
        self.cmd(ccmd::CREATE_OBJECT, obj::SHADER, 5 + words).w(handle).w(ty).w(offlen).w(300).w(0);
        for c in bytes.chunks(4) {
            self.w(u32::from_le_bytes([c[0], c[1], c[2], c[3]]));
        }
        self
    }

    pub fn bind_shader(&mut self, handle: u32, ty: u32) -> &mut Self {
        self.cmd(ccmd::BIND_SHADER, 0, 2).w(handle).w(ty)
    }

    /// Tri prvky vrcholu po vec4 f32 (poloha, souradnice, barva).
    pub fn create_vertex_elements(&mut self, handle: u32) -> &mut Self {
        self.cmd(ccmd::CREATE_OBJECT, obj::VERTEX_ELEMENTS, 13).w(handle);
        for i in 0..3 {
            self.w(i * 16).w(0).w(0).w(format::R32G32B32A32_FLOAT);
        }
        self
    }

    pub fn create_sampler_state(&mut self, handle: u32) -> &mut Self {
        let s0 = TEX_WRAP_CLAMP_TO_EDGE
            | TEX_WRAP_CLAMP_TO_EDGE << 3
            | TEX_WRAP_CLAMP_TO_EDGE << 6
            | TEX_FILTER_LINEAR << 9
            | TEX_MIPFILTER_NONE << 11
            | TEX_FILTER_LINEAR << 13;
        self.cmd(ccmd::CREATE_OBJECT, obj::SAMPLER_STATE, 9).w(handle).w(s0).w(0).w(0).w(0).w(0).w(0).w(0).w(0)
    }

    pub fn create_sampler_view(&mut self, handle: u32, resource: u32, fmt: u32) -> &mut Self {
        let swizzle = 0 | 1 << 3 | 2 << 6 | 3 << 9;
        self.cmd(ccmd::CREATE_OBJECT, obj::SAMPLER_VIEW, 6).w(handle).w(resource).w(fmt).w(0).w(0).w(swizzle)
    }

    pub fn create_surface(&mut self, handle: u32, resource: u32, fmt: u32) -> &mut Self {
        self.cmd(ccmd::CREATE_OBJECT, obj::SURFACE, 5).w(handle).w(resource).w(fmt).w(0).w(0)
    }

    pub fn destroy_object(&mut self, handle: u32, o: u32) -> &mut Self {
        self.cmd(ccmd::DESTROY_OBJECT, o, 1).w(handle)
    }

    pub fn bind_object(&mut self, handle: u32, o: u32) -> &mut Self {
        self.cmd(ccmd::BIND_OBJECT, o, 1).w(handle)
    }

    pub fn set_framebuffer(&mut self, surface: u32) -> &mut Self {
        self.cmd(ccmd::SET_FRAMEBUFFER_STATE, 0, 3).w(1).w(0).w(surface)
    }

    /// Viewport na cely cil `w` x `h` (NDC -> pixely).
    pub fn set_viewport(&mut self, w: u32, h: u32) -> &mut Self {
        let (hw, hh) = (w as f32 / 2.0, h as f32 / 2.0);
        self.cmd(ccmd::SET_VIEWPORT_STATE, 0, 7).w(0).w(f(hw)).w(f(hh)).w(f(0.5)).w(f(hw)).w(f(hh)).w(f(0.5))
    }

    pub fn set_scissor(&mut self, x0: u32, y0: u32, x1: u32, y1: u32) -> &mut Self {
        let m = |v: u32| v.min(0xFFFF);
        self.cmd(ccmd::SET_SCISSOR_STATE, 0, 3).w(0).w(m(x0) | m(y0) << 16).w(m(x1) | m(y1) << 16)
    }

    /// Vymaze barvu (premultiplied, slozky 0..1).
    pub fn clear_color(&mut self, rgba: [f32; 4]) -> &mut Self {
        self.cmd(ccmd::CLEAR, 0, 8).w(CLEAR_COLOR0).w(f(rgba[0])).w(f(rgba[1])).w(f(rgba[2])).w(f(rgba[3])).w(0).w(0).w(0)
    }

    pub fn set_vertex_buffer(&mut self, resource: u32, offset: u32) -> &mut Self {
        self.cmd(ccmd::SET_VERTEX_BUFFERS, 0, 3).w(VERTEX_BYTES).w(offset).w(resource)
    }

    pub fn set_sampler_view(&mut self, view: u32) -> &mut Self {
        self.cmd(ccmd::SET_SAMPLER_VIEWS, 0, 3).w(SHADER_FRAGMENT).w(0).w(view)
    }

    pub fn bind_sampler_state(&mut self, state: u32) -> &mut Self {
        self.cmd(ccmd::BIND_SAMPLER_STATES, 0, 3).w(SHADER_FRAGMENT).w(0).w(state)
    }

    /// Zapis dat do bufferu (vrcholy): `data.len()` musi byt nasobek 4.
    pub fn inline_write(&mut self, resource: u32, offset: u32, data: &[u8]) -> &mut Self {
        let words = data.len().div_ceil(4) as u32;
        self.cmd(ccmd::RESOURCE_INLINE_WRITE, 0, 11 + words)
            .w(resource)
            .w(0)
            .w(0)
            .w(0)
            .w(0)
            .w(offset)
            .w(0)
            .w(0)
            .w(data.len() as u32)
            .w(1)
            .w(1);
        for c in data.chunks(4) {
            let mut b = [0u8; 4];
            b[..c.len()].copy_from_slice(c);
            self.w(u32::from_le_bytes(b));
        }
        self
    }

    /// Kresleni `count` vrcholu od `start` jako trojuhelnikovy pas.
    pub fn draw_strip(&mut self, start: u32, count: u32) -> &mut Self {
        self.cmd(ccmd::DRAW_VBO, 0, 12)
            .w(start)
            .w(count)
            .w(PRIM_TRIANGLE_STRIP)
            .w(0)
            .w(1)
            .w(0)
            .w(0)
            .w(0)
            .w(0)
            .w(0)
            .w(count.saturating_sub(1))
            .w(0)
    }
}

/// Ctyri vrcholy obdelniku: cil v pixelech (cil `w` x `h`), zdroj
/// v souradnicich textury 0..1, nasobitel barvy (premultiplied).
pub fn quad_vertices(dst: (f32, f32, f32, f32), uv: (f32, f32, f32, f32), mul: [f32; 4], w: u32, h: u32) -> Vec<u8> {
    let (x0, y0, x1, y1) = dst;
    let nx = |x: f32| x / w as f32 * 2.0 - 1.0;
    // Y dolu v pixelech -> NDC nahoru.
    let ny = |y: f32| 1.0 - y / h as f32 * 2.0;
    let (u0, v0, u1, v1) = uv;
    let mut out = Vec::with_capacity(4 * VERTEX_BYTES as usize);
    for (x, y, u, v) in [(x0, y0, u0, v0), (x0, y1, u0, v1), (x1, y0, u1, v0), (x1, y1, u1, v1)] {
        for val in [nx(x), ny(y), 0.0, 1.0, u, v, 0.0, 0.0, mul[0], mul[1], mul[2], mul[3]] {
            out.extend_from_slice(&val.to_le_bytes());
        }
    }
    out
}
