//! Prikazy zarizeni virtio-gpu (specifikace virtio 1.2, kapitola 5.7):
//! kodovani pozadavku a cteni odpovedi. Bez zavislosti na jadre, testuje
//! se na hostiteli (`cargo test -p vgpu`).
//!
//! Vsechna cisla jsou little-endian. Kazdy prikaz zacina hlavickou
//! [`Header`] (24 B); odpoved take.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod virgl;

#[cfg(test)]
mod tests;

use alloc::vec::Vec;

/// Vlastnosti zarizeni.
pub const F_VIRGL: u64 = 1 << 0;
pub const F_EDID: u64 = 1 << 1;
pub const F_RESOURCE_BLOB: u64 = 1 << 3;
pub const F_CONTEXT_INIT: u64 = 1 << 4;

/// Fronty.
pub const CONTROL_QUEUE: u16 = 0;
pub const CURSOR_QUEUE: u16 = 1;

/// Konfigurace zarizeni (offsety).
pub mod config {
    pub const EVENTS_READ: u64 = 0;
    pub const EVENTS_CLEAR: u64 = 4;
    pub const NUM_SCANOUTS: u64 = 8;
    pub const NUM_CAPSETS: u64 = 12;
    /// Udalost: zmenila se konfigurace displeje.
    pub const EVENT_DISPLAY: u32 = 1;
}

/// Typy prikazu a odpovedi.
pub mod cmd {
    pub const GET_DISPLAY_INFO: u32 = 0x0100;
    pub const RESOURCE_CREATE_2D: u32 = 0x0101;
    pub const RESOURCE_UNREF: u32 = 0x0102;
    pub const SET_SCANOUT: u32 = 0x0103;
    pub const RESOURCE_FLUSH: u32 = 0x0104;
    pub const TRANSFER_TO_HOST_2D: u32 = 0x0105;
    pub const RESOURCE_ATTACH_BACKING: u32 = 0x0106;
    pub const RESOURCE_DETACH_BACKING: u32 = 0x0107;
    pub const GET_CAPSET_INFO: u32 = 0x0108;
    pub const GET_CAPSET: u32 = 0x0109;
    pub const CTX_CREATE: u32 = 0x0200;
    pub const CTX_DESTROY: u32 = 0x0201;
    pub const CTX_ATTACH_RESOURCE: u32 = 0x0202;
    pub const CTX_DETACH_RESOURCE: u32 = 0x0203;
    pub const RESOURCE_CREATE_3D: u32 = 0x0204;
    pub const TRANSFER_TO_HOST_3D: u32 = 0x0205;
    pub const TRANSFER_FROM_HOST_3D: u32 = 0x0206;
    pub const SUBMIT_3D: u32 = 0x0207;
    pub const UPDATE_CURSOR: u32 = 0x0300;
    pub const MOVE_CURSOR: u32 = 0x0301;

    pub const RESP_OK_NODATA: u32 = 0x1100;
    pub const RESP_OK_DISPLAY_INFO: u32 = 0x1101;
    pub const RESP_OK_CAPSET_INFO: u32 = 0x1102;
    pub const RESP_OK_CAPSET: u32 = 0x1103;
    pub const RESP_ERR_UNSPEC: u32 = 0x1200;
    pub const RESP_ERR_OUT_OF_MEMORY: u32 = 0x1201;
    pub const RESP_ERR_INVALID_SCANOUT_ID: u32 = 0x1202;
    pub const RESP_ERR_INVALID_RESOURCE_ID: u32 = 0x1203;
    pub const RESP_ERR_INVALID_CONTEXT_ID: u32 = 0x1204;
    pub const RESP_ERR_INVALID_PARAMETER: u32 = 0x1205;
}

/// Format pixelu: v pameti bajty B, G, R, A (`u32` 0xAARRGGBB).
pub const FORMAT_B8G8R8A8_UNORM: u32 = 1;
pub const FORMAT_B8G8R8X8_UNORM: u32 = 2;

/// Priznak hlavicky: pozadavek nese plot (`fence_id`), odpoved prijde
/// az po dokonceni prace GPU.
pub const FLAG_FENCE: u32 = 1 << 0;

/// Nejvic vystupu.
pub const MAX_SCANOUTS: usize = 16;

/// Velikost hlavicky.
pub const HEADER_SIZE: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Header {
    pub ty: u32,
    pub flags: u32,
    pub fence: u64,
    pub ctx: u32,
}

/// Obdelnik zarizeni (bez znamenka).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Sestavovani prikazu.
pub struct Cmd(Vec<u8>);

impl Cmd {
    pub fn new(ty: u32) -> Self {
        Self::with(Header { ty, ..Default::default() })
    }

    pub fn with(h: Header) -> Self {
        let mut c = Cmd(Vec::with_capacity(64));
        c.u32(h.ty).u32(h.flags).u64(h.fence).u32(h.ctx).u32(0);
        c
    }

    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn rect(&mut self, r: Rect) -> &mut Self {
        self.u32(r.x).u32(r.y).u32(r.w).u32(r.h)
    }

    pub fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.0.extend_from_slice(b);
        self
    }

    pub fn finish(self) -> Vec<u8> {
        self.0
    }
}

pub fn get_display_info() -> Vec<u8> {
    Cmd::new(cmd::GET_DISPLAY_INFO).finish()
}

pub fn resource_create_2d(resource: u32, format: u32, w: u32, h: u32) -> Vec<u8> {
    let mut c = Cmd::new(cmd::RESOURCE_CREATE_2D);
    c.u32(resource).u32(format).u32(w).u32(h);
    c.finish()
}

pub fn resource_unref(resource: u32) -> Vec<u8> {
    let mut c = Cmd::new(cmd::RESOURCE_UNREF);
    c.u32(resource).u32(0);
    c.finish()
}

/// Kus pameti pro `RESOURCE_ATTACH_BACKING` (adresa pro DMA, delka).
pub fn resource_attach_backing(resource: u32, entries: &[(u64, u32)]) -> Vec<u8> {
    let mut c = Cmd::new(cmd::RESOURCE_ATTACH_BACKING);
    c.u32(resource).u32(entries.len() as u32);
    for &(addr, len) in entries {
        c.u64(addr).u32(len).u32(0);
    }
    c.finish()
}

pub fn resource_detach_backing(resource: u32) -> Vec<u8> {
    let mut c = Cmd::new(cmd::RESOURCE_DETACH_BACKING);
    c.u32(resource).u32(0);
    c.finish()
}

pub fn set_scanout(scanout: u32, resource: u32, r: Rect) -> Vec<u8> {
    let mut c = Cmd::new(cmd::SET_SCANOUT);
    c.rect(r).u32(scanout).u32(resource);
    c.finish()
}

pub fn transfer_to_host_2d(resource: u32, r: Rect, offset: u64) -> Vec<u8> {
    let mut c = Cmd::new(cmd::TRANSFER_TO_HOST_2D);
    c.rect(r).u64(offset).u32(resource).u32(0);
    c.finish()
}

pub fn resource_flush(resource: u32, r: Rect) -> Vec<u8> {
    let mut c = Cmd::new(cmd::RESOURCE_FLUSH);
    c.rect(r).u32(resource).u32(0);
    c.finish()
}

pub fn get_capset_info(index: u32) -> Vec<u8> {
    let mut c = Cmd::new(cmd::GET_CAPSET_INFO);
    c.u32(index).u32(0);
    c.finish()
}

/// Velikost odpovedi `GET_DISPLAY_INFO`.
pub const DISPLAY_INFO_SIZE: usize = HEADER_SIZE + MAX_SCANOUTS * 24;

/// Vystup z `GET_DISPLAY_INFO`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scanout {
    pub rect: Rect,
    pub enabled: bool,
}

fn le32(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(off..off + 4)?.try_into().ok()?))
}

/// Typ odpovedi (z hlavicky).
pub fn response_type(b: &[u8]) -> Option<u32> {
    le32(b, 0)
}

/// Je odpoved uspech (`RESP_OK_*`)?
pub fn is_ok(b: &[u8]) -> bool {
    response_type(b).is_some_and(|t| (cmd::RESP_OK_NODATA..cmd::RESP_ERR_UNSPEC).contains(&t))
}

/// Precte vystupy z odpovedi `GET_DISPLAY_INFO` (jen ty s nenulovou
/// velikosti, nejvys `num_scanouts`).
pub fn parse_display_info(b: &[u8], num_scanouts: u32) -> Option<Vec<Scanout>> {
    if response_type(b)? != cmd::RESP_OK_DISPLAY_INFO {
        return None;
    }
    let n = (num_scanouts as usize).min(MAX_SCANOUTS);
    let mut out = Vec::new();
    for i in 0..n {
        let o = HEADER_SIZE + i * 24;
        let rect = Rect {
            x: le32(b, o)?,
            y: le32(b, o + 4)?,
            w: le32(b, o + 8)?,
            h: le32(b, o + 12)?,
        };
        out.push(Scanout {
            rect,
            enabled: le32(b, o + 16)? != 0,
        });
    }
    Some(out)
}

/// Odpoved `GET_CAPSET_INFO`: (id, nejvyssi verze, velikost).
pub fn parse_capset_info(b: &[u8]) -> Option<(u32, u32, u32)> {
    if response_type(b)? != cmd::RESP_OK_CAPSET_INFO {
        return None;
    }
    Some((le32(b, HEADER_SIZE)?, le32(b, HEADER_SIZE + 4)?, le32(b, HEADER_SIZE + 8)?))
}

/// Rozdeli buffer `h` radku po `stride` bajtech na kusy pameti pro DMA:
/// kazdy nejvys `chunk_max` bajtu a vzdy z celych radku (kopie radku
/// pak nikdy nepresahuje hranici kusu). Vraci pocet radku v kusu.
pub fn rows_per_chunk(stride: u32, chunk_max: u64) -> u32 {
    if stride == 0 {
        return 0;
    }
    (chunk_max / stride as u64).min(u32::MAX as u64) as u32
}

/// Kapabilita (capset) virgl.
pub const CAPSET_VIRGL: u32 = 1;
pub const CAPSET_VIRGL2: u32 = 2;
pub const CAPSET_VENUS: u32 = 4;

/// Jmeno capsetu (log).
pub fn capset_name(id: u32) -> &'static str {
    match id {
        CAPSET_VIRGL => "virgl",
        CAPSET_VIRGL2 => "virgl2",
        3 => "gfxstream-vulkan",
        CAPSET_VENUS => "venus",
        5 => "cross-domain",
        6 => "drm",
        _ => "?",
    }
}

fn with_ctx(ty: u32, ctx: u32) -> Cmd {
    Cmd::with(Header { ty, ctx, ..Default::default() })
}

/// Novy 3D kontext `ctx` (jmeno pro ladeni nejvys 64 B).
pub fn ctx_create(ctx: u32, name: &str) -> Vec<u8> {
    let n = name.len().min(63);
    let mut c = with_ctx(cmd::CTX_CREATE, ctx);
    c.u32(n as u32).u32(0);
    let mut buf = [0u8; 64];
    buf[..n].copy_from_slice(&name.as_bytes()[..n]);
    c.bytes(&buf);
    c.finish()
}

pub fn ctx_attach_resource(ctx: u32, resource: u32) -> Vec<u8> {
    let mut c = with_ctx(cmd::CTX_ATTACH_RESOURCE, ctx);
    c.u32(resource).u32(0);
    c.finish()
}

/// Zdroj 3D (textura, cil vykresleni, buffer): cil, format a pouziti jsou
/// hodnoty virgl (`virgl::target`, `virgl::format`, `virgl::bind`).
#[allow(clippy::too_many_arguments)]
pub fn resource_create_3d(resource: u32, target: u32, format: u32, bind: u32, w: u32, h: u32) -> Vec<u8> {
    let mut c = Cmd::new(cmd::RESOURCE_CREATE_3D);
    c.u32(resource).u32(target).u32(format).u32(bind).u32(w).u32(h).u32(1).u32(1).u32(0).u32(0).u32(0).u32(0);
    c.finish()
}

/// Prenos 3D (`TRANSFER_TO_HOST_3D` / `TRANSFER_FROM_HOST_3D`): obdelnik,
/// offset v backingu a krok radku.
pub fn transfer_3d(to_host: bool, ctx: u32, resource: u32, r: Rect, offset: u64, stride: u32) -> Vec<u8> {
    let ty = if to_host { cmd::TRANSFER_TO_HOST_3D } else { cmd::TRANSFER_FROM_HOST_3D };
    let mut c = with_ctx(ty, ctx);
    c.u32(r.x).u32(r.y).u32(0).u32(r.w).u32(r.h).u32(1);
    c.u64(offset).u32(resource).u32(0).u32(stride).u32(0);
    c.finish()
}

/// `SUBMIT_3D`: hlavicka a za ni proud prikazu virgl.
pub fn submit_3d(ctx: u32, stream: &[u8]) -> Vec<u8> {
    let mut c = with_ctx(cmd::SUBMIT_3D, ctx);
    c.u32(stream.len() as u32).u32(0).bytes(stream);
    c.finish()
}
