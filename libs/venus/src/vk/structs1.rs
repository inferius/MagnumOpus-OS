// Vygenerovano `cargo xtask venus-gen` z libs/venus/xml (vk.xml a venus-protocol,
// licence v libs/venus/xml/README.md) a libs/venus/subset.txt. Neupravovat rucne.

use alloc::vec::Vec;

use super::*;
use crate::cs::{CsError, Decoder, Encoder, Result};

/// Struktura v retezci pNext.
#[derive(Debug, Clone, Copy)]
pub enum Next<'a> {
    VkDeviceQueueTimelineInfoMESA(&'a VkDeviceQueueTimelineInfoMESA),
}

impl Next<'_> {
    pub fn s_type(&self) -> VkStructureType {
        match self {
            Next::VkDeviceQueueTimelineInfoMESA(_) => VkDeviceQueueTimelineInfoMESA::S_TYPE,
        }
    }

    pub fn encode_self(&self, e: &mut Encoder) {
        match self {
            Next::VkDeviceQueueTimelineInfoMESA(x) => x.encode_self(e),
        }
    }
}

/// Retezec pNext: ukazatel, sType, zbytek retezce, cleny (jako
/// `vn_encode_*_pnext`). Struktura mimo `allowed` = chyba.
pub fn encode_chain(e: &mut Encoder, next: &[Next<'_>], allowed: &[VkStructureType]) {
    match next.split_first() {
        None => e.pointer(false),
        Some((n, rest)) => {
            if !allowed.contains(&n.s_type()) {
                e.fail(CsError::BadChain);
            }
            e.pointer(true);
            e.i32(n.s_type());
            encode_chain(e, rest, allowed);
            n.encode_self(e);
        }
    }
}

/// `VkApplicationInfo` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkApplicationInfo<'a> {
    pub p_application_name: Option<&'a str>,
    pub application_version: u32,
    pub p_engine_name: Option<&'a str>,
    pub engine_version: u32,
    pub api_version: u32,
}

impl Default for VkApplicationInfo<'_> {
    fn default() -> Self {
        Self {
            p_application_name: None,
            application_version: 0,
            p_engine_name: None,
            engine_version: 0,
            api_version: 0,
        }
    }
}

impl<'a> VkApplicationInfo<'a> {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_APPLICATION_INFO;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.opt_string(self.p_application_name);
        e.u32(self.application_version);
        e.opt_string(self.p_engine_name);
        e.u32(self.engine_version);
        e.u32(self.api_version);
    }

}

/// `VkBufferCreateInfo` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkBufferCreateInfo<'a> {
    pub flags: VkBufferCreateFlags,
    pub size: VkDeviceSize,
    pub usage: VkBufferUsageFlags,
    pub sharing_mode: VkSharingMode,
    pub queue_family_index_count: u32,
    pub p_queue_family_indices: &'a [u32],
}

impl Default for VkBufferCreateInfo<'_> {
    fn default() -> Self {
        Self {
            flags: 0,
            size: 0,
            usage: 0,
            sharing_mode: 0,
            queue_family_index_count: 0,
            p_queue_family_indices: &[],
        }
    }
}

impl<'a> VkBufferCreateInfo<'a> {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.flags);
        e.u64(self.size);
        e.u32(self.usage);
        e.i32(self.sharing_mode);
        e.u32(self.queue_family_index_count);
        {
            let s = e.dyn_array(self.p_queue_family_indices, self.queue_family_index_count as usize);
            e.u32_array(s);
        }
    }

}

/// `VkBufferImageCopy` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkBufferImageCopy {
    pub buffer_offset: VkDeviceSize,
    pub buffer_row_length: u32,
    pub buffer_image_height: u32,
    pub image_subresource: VkImageSubresourceLayers,
    pub image_offset: VkOffset3D,
    pub image_extent: VkExtent3D,
}

impl Default for VkBufferImageCopy {
    fn default() -> Self {
        Self {
            buffer_offset: 0,
            buffer_row_length: 0,
            buffer_image_height: 0,
            image_subresource: VkImageSubresourceLayers::default(),
            image_offset: VkOffset3D::default(),
            image_extent: VkExtent3D::default(),
        }
    }
}

impl VkBufferImageCopy {
    pub fn encode(&self, e: &mut Encoder) {
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u64(self.buffer_offset);
        e.u32(self.buffer_row_length);
        e.u32(self.buffer_image_height);
        self.image_subresource.encode(e);
        self.image_offset.encode(e);
        self.image_extent.encode(e);
    }

}

/// `VkBufferMemoryBarrier` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkBufferMemoryBarrier {
    pub src_access_mask: VkAccessFlags,
    pub dst_access_mask: VkAccessFlags,
    pub src_queue_family_index: u32,
    pub dst_queue_family_index: u32,
    pub buffer: VkBuffer,
    pub offset: VkDeviceSize,
    pub size: VkDeviceSize,
}

impl Default for VkBufferMemoryBarrier {
    fn default() -> Self {
        Self {
            src_access_mask: 0,
            dst_access_mask: 0,
            src_queue_family_index: 0,
            dst_queue_family_index: 0,
            buffer: VkBuffer(0),
            offset: 0,
            size: 0,
        }
    }
}

impl VkBufferMemoryBarrier {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_BUFFER_MEMORY_BARRIER;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.src_access_mask);
        e.u32(self.dst_access_mask);
        e.u32(self.src_queue_family_index);
        e.u32(self.dst_queue_family_index);
        e.u64(self.buffer.0);
        e.u64(self.offset);
        e.u64(self.size);
    }

}

/// `VkCommandBufferAllocateInfo` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkCommandBufferAllocateInfo {
    pub command_pool: VkCommandPool,
    pub level: VkCommandBufferLevel,
    pub command_buffer_count: u32,
}

impl Default for VkCommandBufferAllocateInfo {
    fn default() -> Self {
        Self {
            command_pool: VkCommandPool(0),
            level: 0,
            command_buffer_count: 0,
        }
    }
}

impl VkCommandBufferAllocateInfo {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u64(self.command_pool.0);
        e.i32(self.level);
        e.u32(self.command_buffer_count);
    }

}

/// `VkCommandBufferBeginInfo` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkCommandBufferBeginInfo<'a> {
    pub flags: VkCommandBufferUsageFlags,
    pub p_inheritance_info: Option<&'a VkCommandBufferInheritanceInfo>,
}

impl Default for VkCommandBufferBeginInfo<'_> {
    fn default() -> Self {
        Self {
            flags: 0,
            p_inheritance_info: None,
        }
    }
}

impl<'a> VkCommandBufferBeginInfo<'a> {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.flags);
        match self.p_inheritance_info {
            Some(x) => {
                e.pointer(true);
                x.encode(e);
            }
            None => e.pointer(false),
        }
    }

}

/// `VkCommandBufferInheritanceInfo` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkCommandBufferInheritanceInfo {
    pub render_pass: VkRenderPass,
    pub subpass: u32,
    pub framebuffer: VkFramebuffer,
    pub occlusion_query_enable: VkBool32,
    pub query_flags: VkQueryControlFlags,
    pub pipeline_statistics: VkQueryPipelineStatisticFlags,
}

impl Default for VkCommandBufferInheritanceInfo {
    fn default() -> Self {
        Self {
            render_pass: VkRenderPass(0),
            subpass: 0,
            framebuffer: VkFramebuffer(0),
            occlusion_query_enable: 0,
            query_flags: 0,
            pipeline_statistics: 0,
        }
    }
}

impl VkCommandBufferInheritanceInfo {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_INHERITANCE_INFO;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u64(self.render_pass.0);
        e.u32(self.subpass);
        e.u64(self.framebuffer.0);
        e.u32(self.occlusion_query_enable);
        e.u32(self.query_flags);
        e.u32(self.pipeline_statistics);
    }

}

/// `VkCommandPoolCreateInfo` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkCommandPoolCreateInfo {
    pub flags: VkCommandPoolCreateFlags,
    pub queue_family_index: u32,
}

impl Default for VkCommandPoolCreateInfo {
    fn default() -> Self {
        Self {
            flags: 0,
            queue_family_index: 0,
        }
    }
}

impl VkCommandPoolCreateInfo {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.flags);
        e.u32(self.queue_family_index);
    }

}

/// `VkCommandStreamDescriptionMESA` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkCommandStreamDescriptionMESA {
    pub resource_id: u32,
    pub offset: u64,
    pub size: u64,
}

impl Default for VkCommandStreamDescriptionMESA {
    fn default() -> Self {
        Self {
            resource_id: 0,
            offset: 0,
            size: 0,
        }
    }
}

impl VkCommandStreamDescriptionMESA {
    pub fn encode(&self, e: &mut Encoder) {
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.resource_id);
        e.u64(self.offset);
        e.u64(self.size);
    }

}

/// `VkDeviceCreateInfo` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkDeviceCreateInfo<'a> {
    pub flags: VkDeviceCreateFlags,
    pub queue_create_info_count: u32,
    pub p_queue_create_infos: &'a [VkDeviceQueueCreateInfo<'a>],
    pub enabled_layer_count: u32,
    pub pp_enabled_layer_names: &'a [&'a str],
    pub enabled_extension_count: u32,
    pub pp_enabled_extension_names: &'a [&'a str],
    pub p_enabled_features: Option<&'a VkPhysicalDeviceFeatures>,
}

impl Default for VkDeviceCreateInfo<'_> {
    fn default() -> Self {
        Self {
            flags: 0,
            queue_create_info_count: 0,
            p_queue_create_infos: &[],
            enabled_layer_count: 0,
            pp_enabled_layer_names: &[],
            enabled_extension_count: 0,
            pp_enabled_extension_names: &[],
            p_enabled_features: None,
        }
    }
}

impl<'a> VkDeviceCreateInfo<'a> {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.flags);
        e.u32(self.queue_create_info_count);
        {
            let s = e.dyn_array(self.p_queue_create_infos, self.queue_create_info_count as usize);
            for x in s { x.encode(e); }
        }
        e.u32(self.enabled_layer_count);
        {
            let s = e.dyn_array(self.pp_enabled_layer_names, self.enabled_layer_count as usize);
            for x in s {
                e.string(x);
            }
        }
        e.u32(self.enabled_extension_count);
        {
            let s = e.dyn_array(self.pp_enabled_extension_names, self.enabled_extension_count as usize);
            for x in s {
                e.string(x);
            }
        }
        match self.p_enabled_features {
            Some(x) => {
                e.pointer(true);
                x.encode(e);
            }
            None => e.pointer(false),
        }
    }

}

/// `VkDeviceQueueCreateInfo` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkDeviceQueueCreateInfo<'a> {
    pub flags: VkDeviceQueueCreateFlags,
    pub queue_family_index: u32,
    pub queue_count: u32,
    pub p_queue_priorities: &'a [f32],
}

impl Default for VkDeviceQueueCreateInfo<'_> {
    fn default() -> Self {
        Self {
            flags: 0,
            queue_family_index: 0,
            queue_count: 0,
            p_queue_priorities: &[],
        }
    }
}

impl<'a> VkDeviceQueueCreateInfo<'a> {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.flags);
        e.u32(self.queue_family_index);
        e.u32(self.queue_count);
        {
            let s = e.dyn_array(self.p_queue_priorities, self.queue_count as usize);
            e.f32_array(s);
        }
    }

}

/// `VkDeviceQueueInfo2` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkDeviceQueueInfo2<'a> {
    /// Retezec pNext (jen struktury, ktere `VkDeviceQueueInfo2` povoluje).
    pub p_next: &'a [Next<'a>],
    pub flags: VkDeviceQueueCreateFlags,
    pub queue_family_index: u32,
    pub queue_index: u32,
}

impl Default for VkDeviceQueueInfo2<'_> {
    fn default() -> Self {
        Self {
            p_next: &[],
            flags: 0,
            queue_family_index: 0,
            queue_index: 0,
        }
    }
}

impl<'a> VkDeviceQueueInfo2<'a> {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_INFO_2;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        encode_chain(e, self.p_next, &[VkDeviceQueueTimelineInfoMESA::S_TYPE]);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.flags);
        e.u32(self.queue_family_index);
        e.u32(self.queue_index);
    }

}

/// `VkDeviceQueueTimelineInfoMESA` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkDeviceQueueTimelineInfoMESA {
    pub ring_idx: u32,
}

impl Default for VkDeviceQueueTimelineInfoMESA {
    fn default() -> Self {
        Self {
            ring_idx: 0,
        }
    }
}

impl VkDeviceQueueTimelineInfoMESA {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_TIMELINE_INFO_MESA;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.ring_idx);
    }

}

/// `VkExtent3D` (vstup i vystup).
#[derive(Debug, Clone, Copy)]
pub struct VkExtent3D {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
}

impl Default for VkExtent3D {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            depth: 0,
        }
    }
}

impl VkExtent3D {
    pub fn encode(&self, e: &mut Encoder) {
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.width);
        e.u32(self.height);
        e.u32(self.depth);
    }

    /// Castecne kodovani vystupu (sType, pNext, id handlu).
    pub fn encode_partial(&self, e: &mut Encoder) {
        let _ = e;
    }

    pub fn decode(d: &mut Decoder) -> Result<Self> {
        let mut v = Self::default();
        v.width = d.u32()?;
        v.height = d.u32()?;
        v.depth = d.u32()?;
        Ok(v)
    }
}

/// `VkFenceCreateInfo` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkFenceCreateInfo {
    pub flags: VkFenceCreateFlags,
}

impl Default for VkFenceCreateInfo {
    fn default() -> Self {
        Self {
            flags: 0,
        }
    }
}

impl VkFenceCreateInfo {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_FENCE_CREATE_INFO;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.flags);
    }

}

/// `VkFormatProperties` (vystup).
#[derive(Debug, Clone, Copy)]
pub struct VkFormatProperties {
    pub linear_tiling_features: VkFormatFeatureFlags,
    pub optimal_tiling_features: VkFormatFeatureFlags,
    pub buffer_features: VkFormatFeatureFlags,
}

impl Default for VkFormatProperties {
    fn default() -> Self {
        Self {
            linear_tiling_features: 0,
            optimal_tiling_features: 0,
            buffer_features: 0,
        }
    }
}

impl VkFormatProperties {
    #[cfg(test)]
    pub fn encode(&self, e: &mut Encoder) {
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    #[cfg(test)]
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.linear_tiling_features);
        e.u32(self.optimal_tiling_features);
        e.u32(self.buffer_features);
    }

    /// Castecne kodovani vystupu (sType, pNext, id handlu).
    pub fn encode_partial(&self, e: &mut Encoder) {
        let _ = e;
    }

    pub fn decode(d: &mut Decoder) -> Result<Self> {
        let mut v = Self::default();
        v.linear_tiling_features = d.u32()?;
        v.optimal_tiling_features = d.u32()?;
        v.buffer_features = d.u32()?;
        Ok(v)
    }
}

/// `VkImageCreateInfo` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkImageCreateInfo<'a> {
    pub flags: VkImageCreateFlags,
    pub image_type: VkImageType,
    pub format: VkFormat,
    pub extent: VkExtent3D,
    pub mip_levels: u32,
    pub array_layers: u32,
    pub samples: VkSampleCountFlagBits,
    pub tiling: VkImageTiling,
    pub usage: VkImageUsageFlags,
    pub sharing_mode: VkSharingMode,
    pub queue_family_index_count: u32,
    pub p_queue_family_indices: &'a [u32],
    pub initial_layout: VkImageLayout,
}

impl Default for VkImageCreateInfo<'_> {
    fn default() -> Self {
        Self {
            flags: 0,
            image_type: 0,
            format: 0,
            extent: VkExtent3D::default(),
            mip_levels: 0,
            array_layers: 0,
            samples: 0,
            tiling: 0,
            usage: 0,
            sharing_mode: 0,
            queue_family_index_count: 0,
            p_queue_family_indices: &[],
            initial_layout: 0,
        }
    }
}

impl<'a> VkImageCreateInfo<'a> {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.flags);
        e.i32(self.image_type);
        e.i32(self.format);
        self.extent.encode(e);
        e.u32(self.mip_levels);
        e.u32(self.array_layers);
        e.u32(self.samples);
        e.i32(self.tiling);
        e.u32(self.usage);
        e.i32(self.sharing_mode);
        e.u32(self.queue_family_index_count);
        {
            let s = e.dyn_array(self.p_queue_family_indices, self.queue_family_index_count as usize);
            e.u32_array(s);
        }
        e.i32(self.initial_layout);
    }

}

/// `VkImageMemoryBarrier` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkImageMemoryBarrier {
    pub src_access_mask: VkAccessFlags,
    pub dst_access_mask: VkAccessFlags,
    pub old_layout: VkImageLayout,
    pub new_layout: VkImageLayout,
    pub src_queue_family_index: u32,
    pub dst_queue_family_index: u32,
    pub image: VkImage,
    pub subresource_range: VkImageSubresourceRange,
}

impl Default for VkImageMemoryBarrier {
    fn default() -> Self {
        Self {
            src_access_mask: 0,
            dst_access_mask: 0,
            old_layout: 0,
            new_layout: 0,
            src_queue_family_index: 0,
            dst_queue_family_index: 0,
            image: VkImage(0),
            subresource_range: VkImageSubresourceRange::default(),
        }
    }
}

impl VkImageMemoryBarrier {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.src_access_mask);
        e.u32(self.dst_access_mask);
        e.i32(self.old_layout);
        e.i32(self.new_layout);
        e.u32(self.src_queue_family_index);
        e.u32(self.dst_queue_family_index);
        e.u64(self.image.0);
        self.subresource_range.encode(e);
    }

}

/// `VkImageSubresourceLayers` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkImageSubresourceLayers {
    pub aspect_mask: VkImageAspectFlags,
    pub mip_level: u32,
    pub base_array_layer: u32,
    pub layer_count: u32,
}

impl Default for VkImageSubresourceLayers {
    fn default() -> Self {
        Self {
            aspect_mask: 0,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 0,
        }
    }
}

impl VkImageSubresourceLayers {
    pub fn encode(&self, e: &mut Encoder) {
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.aspect_mask);
        e.u32(self.mip_level);
        e.u32(self.base_array_layer);
        e.u32(self.layer_count);
    }

}

/// `VkImageSubresourceRange` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkImageSubresourceRange {
    pub aspect_mask: VkImageAspectFlags,
    pub base_mip_level: u32,
    pub level_count: u32,
    pub base_array_layer: u32,
    pub layer_count: u32,
}

impl Default for VkImageSubresourceRange {
    fn default() -> Self {
        Self {
            aspect_mask: 0,
            base_mip_level: 0,
            level_count: 0,
            base_array_layer: 0,
            layer_count: 0,
        }
    }
}

impl VkImageSubresourceRange {
    pub fn encode(&self, e: &mut Encoder) {
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.aspect_mask);
        e.u32(self.base_mip_level);
        e.u32(self.level_count);
        e.u32(self.base_array_layer);
        e.u32(self.layer_count);
    }

}

/// `VkInstanceCreateInfo` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkInstanceCreateInfo<'a> {
    pub flags: VkInstanceCreateFlags,
    pub p_application_info: Option<&'a VkApplicationInfo<'a>>,
    pub enabled_layer_count: u32,
    pub pp_enabled_layer_names: &'a [&'a str],
    pub enabled_extension_count: u32,
    pub pp_enabled_extension_names: &'a [&'a str],
}

impl Default for VkInstanceCreateInfo<'_> {
    fn default() -> Self {
        Self {
            flags: 0,
            p_application_info: None,
            enabled_layer_count: 0,
            pp_enabled_layer_names: &[],
            enabled_extension_count: 0,
            pp_enabled_extension_names: &[],
        }
    }
}

impl<'a> VkInstanceCreateInfo<'a> {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.flags);
        match self.p_application_info {
            Some(x) => {
                e.pointer(true);
                x.encode(e);
            }
            None => e.pointer(false),
        }
        e.u32(self.enabled_layer_count);
        {
            let s = e.dyn_array(self.pp_enabled_layer_names, self.enabled_layer_count as usize);
            for x in s {
                e.string(x);
            }
        }
        e.u32(self.enabled_extension_count);
        {
            let s = e.dyn_array(self.pp_enabled_extension_names, self.enabled_extension_count as usize);
            for x in s {
                e.string(x);
            }
        }
    }

}

/// `VkMemoryAllocateInfo` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkMemoryAllocateInfo {
    pub allocation_size: VkDeviceSize,
    pub memory_type_index: u32,
}

impl Default for VkMemoryAllocateInfo {
    fn default() -> Self {
        Self {
            allocation_size: 0,
            memory_type_index: 0,
        }
    }
}

impl VkMemoryAllocateInfo {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u64(self.allocation_size);
        e.u32(self.memory_type_index);
    }

}

