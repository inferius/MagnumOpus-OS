//! Capset Venusu (`struct virgl_renderer_capset_venus`, virglrenderer
//! `venus_hw.h`): verze formatu a protokolu, ktere renderer umi.

/// Verze formatu proudu, kterou tento klient pouziva
/// (`VN_WIRE_FORMAT_VERSION` ve venus-protocol).
pub const WIRE_FORMAT_VERSION: u32 = 1;

/// Velikost capsetu verze 0 (40 x u32).
pub const SIZE: usize = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VenusCapset {
    pub wire_format_version: u32,
    /// Verze vk.xml rendereru (`VK_MAKE_API_VERSION`).
    pub vk_xml_version: u32,
    pub vk_ext_command_serialization_spec_version: u32,
    pub vk_mesa_venus_protocol_spec_version: u32,
    /// Renderer umi blob s `blob_id` 0 (nova sdilena pamet).
    pub supports_blob_id_0: bool,
    pub allow_vk_wait_syncs: bool,
    /// Vic casovych os plotu (ring_idx); 0 je vzdy osa CPU.
    pub supports_multiple_timelines: bool,
    pub use_guest_vram: bool,
}

fn word(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i * 4..i * 4 + 4)?.try_into().ok()?))
}

impl VenusCapset {
    /// Rozebere obsah capsetu (kratsi data = `None`).
    pub fn parse(b: &[u8]) -> Option<Self> {
        if b.len() < SIZE {
            return None;
        }
        Some(Self {
            wire_format_version: word(b, 0)?,
            vk_xml_version: word(b, 1)?,
            vk_ext_command_serialization_spec_version: word(b, 2)?,
            vk_mesa_venus_protocol_spec_version: word(b, 3)?,
            supports_blob_id_0: word(b, 4)? != 0,
            allow_vk_wait_syncs: word(b, 37)? != 0,
            supports_multiple_timelines: word(b, 38)? != 0,
            use_guest_vram: word(b, 39)? != 0,
        })
    }

    /// Muze tento klient s rendererem mluvit? (stejny format proudu
    /// a blob s id 0 pro odpovedi).
    pub fn compatible(&self) -> bool {
        self.wire_format_version == WIRE_FORMAT_VERSION && self.supports_blob_id_0
    }
}
