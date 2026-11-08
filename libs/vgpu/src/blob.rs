//! Prikazy pro kontexty s capsetem (Venus) a blob zdroje.
//!
//! Rozlozeni podle linuxoveho `include/uapi/linux/virtio_gpu.h`:
//! `virtio_gpu_ctx_create` (s `context_init`), `virtio_gpu_get_capset`,
//! `virtio_gpu_resource_create_blob`, `virtio_gpu_resource_map_blob`,
//! `virtio_gpu_resource_unmap_blob`, `virtio_gpu_resp_map_info`.
//!
//! Blob zdroj v pameti hostitele (`BLOB_MEM_HOST3D`) s priznakem
//! `BLOB_FLAG_USE_MAPPABLE` jde namapovat do oblasti sdilene pameti
//! zarizeni (`VIRTIO_PCI_CAP_SHARED_MEMORY_CFG`, id
//! [`SHM_ID_HOST_VISIBLE`]) na zvoleny offset; host a hostitel pak sdili
//! stejne stranky.

use alloc::vec::Vec;

use crate::{Cmd, HEADER_SIZE, Header, cmd, le32, response_type, with_ctx};

/// Pamet blobu: stranky hosta (seznam kusu jako u `ATTACH_BACKING`).
pub const BLOB_MEM_GUEST: u32 = 1;
/// Pamet blobu: pamet hostitele vytvorena kontextem (u Venusu
/// `VkDeviceMemory` nebo sdilena pamet pro ring a odpovedi).
pub const BLOB_MEM_HOST3D: u32 = 2;
/// Pamet hostitele se zalohou ve strankach hosta.
pub const BLOB_MEM_HOST3D_GUEST: u32 = 3;

/// Blob pujde namapovat do sdilene pameti (`RESOURCE_MAP_BLOB`).
pub const BLOB_FLAG_USE_MAPPABLE: u32 = 1;
pub const BLOB_FLAG_USE_SHAREABLE: u32 = 2;
pub const BLOB_FLAG_USE_CROSS_DEVICE: u32 = 4;

/// Id oblasti sdilene pameti virtio-gpu pro mapovani blobu.
pub const SHM_ID_HOST_VISIBLE: u8 = 1;

/// Maska capsetu v `context_init` (`CTX_CREATE`).
pub const CONTEXT_INIT_CAPSET_ID_MASK: u32 = 0xFF;

/// Typ cache namapovaneho blobu (`map_info & MAP_CACHE_MASK`).
pub const MAP_CACHE_MASK: u32 = 0x0F;
pub const MAP_CACHE_NONE: u32 = 0;
pub const MAP_CACHE_CACHED: u32 = 1;
pub const MAP_CACHE_UNCACHED: u32 = 2;
pub const MAP_CACHE_WC: u32 = 3;

/// Velikost odpovedi `RESOURCE_MAP_BLOB`.
pub const MAP_INFO_SIZE: usize = HEADER_SIZE + 8;

/// Novy kontext `ctx` s capsetem `capset` (napr. [`crate::CAPSET_VENUS`]).
/// Vyzaduje vlastnost `F_CONTEXT_INIT`. Jmeno pro ladeni nejvys 63 B.
pub fn ctx_create_init(ctx: u32, capset: u32, name: &str) -> Vec<u8> {
    let n = name.len().min(63);
    let mut c = with_ctx(cmd::CTX_CREATE, ctx);
    c.u32(n as u32).u32(capset & CONTEXT_INIT_CAPSET_ID_MASK);
    let mut buf = [0u8; 64];
    buf[..n].copy_from_slice(&name.as_bytes()[..n]);
    c.bytes(&buf);
    c.finish()
}

pub fn ctx_destroy(ctx: u32) -> Vec<u8> {
    with_ctx(cmd::CTX_DESTROY, ctx).finish()
}

/// `GET_CAPSET`: obsah capsetu `id` ve verzi `version`.
pub fn get_capset(id: u32, version: u32) -> Vec<u8> {
    let mut c = Cmd::new(cmd::GET_CAPSET);
    c.u32(id).u32(version);
    c.finish()
}

/// Obsah capsetu z odpovedi `GET_CAPSET` (bez hlavicky).
pub fn parse_capset(b: &[u8]) -> Option<&[u8]> {
    if response_type(b)? != cmd::RESP_OK_CAPSET {
        return None;
    }
    b.get(HEADER_SIZE..)
}

/// Parametry `RESOURCE_CREATE_BLOB`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BlobInfo {
    pub resource: u32,
    pub mem: u32,
    pub flags: u32,
    /// Id objektu v kontextu (u Venusu id `VkDeviceMemory`; 0 = nova
    /// sdilena pamet).
    pub blob_id: u64,
    pub size: u64,
}

/// `RESOURCE_CREATE_BLOB` v kontextu `ctx`; `entries` jsou kusy pameti
/// hosta (adresa pro DMA, delka) pro `BLOB_MEM_GUEST` a
/// `BLOB_MEM_HOST3D_GUEST`, jinak prazdne. Zdroj je pak pripojeny ke
/// kontextu (bez `CTX_ATTACH_RESOURCE`).
pub fn resource_create_blob(ctx: u32, b: &BlobInfo, entries: &[(u64, u32)]) -> Vec<u8> {
    let mut c = with_ctx(cmd::RESOURCE_CREATE_BLOB, ctx);
    c.u32(b.resource).u32(b.mem).u32(b.flags).u32(entries.len() as u32);
    c.u64(b.blob_id).u64(b.size);
    for &(addr, len) in entries {
        c.u64(addr).u32(len).u32(0);
    }
    c.finish()
}

/// `RESOURCE_MAP_BLOB`: namapuje blob na `offset` v oblasti sdilene
/// pameti [`SHM_ID_HOST_VISIBLE`]. Odpoved `RESP_OK_MAP_INFO`.
pub fn resource_map_blob(ctx: u32, resource: u32, offset: u64) -> Vec<u8> {
    let mut c = with_ctx(cmd::RESOURCE_MAP_BLOB, ctx);
    c.u32(resource).u32(0).u64(offset);
    c.finish()
}

pub fn resource_unmap_blob(ctx: u32, resource: u32) -> Vec<u8> {
    let mut c = with_ctx(cmd::RESOURCE_UNMAP_BLOB, ctx);
    c.u32(resource).u32(0);
    c.finish()
}

/// `SET_SCANOUT_BLOB`: vystup `scanout` zobrazi blob `resource` (`w` x `h`,
/// format `format`, krok radku `stride`, zacatek `offset`). U virgl/Venus
/// musi byt blob podlozeny dmabuf (QEMU ho preda displeji jako dmabuf).
/// `resource` 0 vystup vypne. Rozlozeni `virtio_gpu_set_scanout_blob`.
pub fn set_scanout_blob(scanout: u32, resource: u32, w: u32, h: u32, format: u32, stride: u32, offset: u32) -> Vec<u8> {
    let mut c = Cmd::new(cmd::SET_SCANOUT_BLOB);
    c.rect(crate::Rect { x: 0, y: 0, w, h });
    c.u32(scanout).u32(resource).u32(w).u32(h).u32(format).u32(0);
    c.u32(stride).u32(0).u32(0).u32(0);
    c.u32(offset).u32(0).u32(0).u32(0);
    c.finish()
}

/// `map_info` z odpovedi `RESOURCE_MAP_BLOB` (typ cache v dolnich 4 bitech).
pub fn parse_map_info(b: &[u8]) -> Option<u32> {
    if response_type(b)? != cmd::RESP_OK_MAP_INFO {
        return None;
    }
    le32(b, HEADER_SIZE)
}

/// `SUBMIT_3D` s plotem `fence` na casove ose `ring_idx` kontextu `ctx`:
/// odpoved prijde az po splneni plotu. U Venusu je osa 0 osa CPU (plot
/// se splni po zpracovani prikazu proudu).
pub fn submit_3d_fenced(ctx: u32, ring_idx: u8, fence: u64, stream: &[u8]) -> Vec<u8> {
    let mut c = Cmd::with(Header {
        ty: cmd::SUBMIT_3D,
        flags: crate::FLAG_FENCE | crate::FLAG_INFO_RING_IDX,
        fence,
        ctx,
        ring_idx,
    });
    c.u32(stream.len() as u32).u32(0).bytes(stream);
    c.finish()
}
