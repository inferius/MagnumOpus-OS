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

/// `VkBufferCopy` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkBufferCopy {
    pub src_offset: VkDeviceSize,
    pub dst_offset: VkDeviceSize,
    pub size: VkDeviceSize,
}

impl Default for VkBufferCopy {
    fn default() -> Self {
        Self {
            src_offset: 0,
            dst_offset: 0,
            size: 0,
        }
    }
}

impl VkBufferCopy {
    pub fn encode(&self, e: &mut Encoder) {
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u64(self.src_offset);
        e.u64(self.dst_offset);
        e.u64(self.size);
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

/// `VkComputePipelineCreateInfo` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkComputePipelineCreateInfo<'a> {
    pub flags: VkPipelineCreateFlags,
    pub stage: VkPipelineShaderStageCreateInfo<'a>,
    pub layout: VkPipelineLayout,
    pub base_pipeline_handle: VkPipeline,
    pub base_pipeline_index: i32,
}

impl Default for VkComputePipelineCreateInfo<'_> {
    fn default() -> Self {
        Self {
            flags: 0,
            stage: VkPipelineShaderStageCreateInfo::default(),
            layout: VkPipelineLayout(0),
            base_pipeline_handle: VkPipeline(0),
            base_pipeline_index: 0,
        }
    }
}

impl<'a> VkComputePipelineCreateInfo<'a> {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.flags);
        self.stage.encode(e);
        e.u64(self.layout.0);
        e.u64(self.base_pipeline_handle.0);
        e.i32(self.base_pipeline_index);
    }

}

/// `VkCopyDescriptorSet` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkCopyDescriptorSet {
    pub src_set: VkDescriptorSet,
    pub src_binding: u32,
    pub src_array_element: u32,
    pub dst_set: VkDescriptorSet,
    pub dst_binding: u32,
    pub dst_array_element: u32,
    pub descriptor_count: u32,
}

impl Default for VkCopyDescriptorSet {
    fn default() -> Self {
        Self {
            src_set: VkDescriptorSet(0),
            src_binding: 0,
            src_array_element: 0,
            dst_set: VkDescriptorSet(0),
            dst_binding: 0,
            dst_array_element: 0,
            descriptor_count: 0,
        }
    }
}

impl VkCopyDescriptorSet {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_COPY_DESCRIPTOR_SET;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u64(self.src_set.0);
        e.u32(self.src_binding);
        e.u32(self.src_array_element);
        e.u64(self.dst_set.0);
        e.u32(self.dst_binding);
        e.u32(self.dst_array_element);
        e.u32(self.descriptor_count);
    }

}

/// `VkDescriptorBufferInfo` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkDescriptorBufferInfo {
    pub buffer: VkBuffer,
    pub offset: VkDeviceSize,
    pub range: VkDeviceSize,
}

impl Default for VkDescriptorBufferInfo {
    fn default() -> Self {
        Self {
            buffer: VkBuffer(0),
            offset: 0,
            range: 0,
        }
    }
}

impl VkDescriptorBufferInfo {
    pub fn encode(&self, e: &mut Encoder) {
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u64(self.buffer.0);
        e.u64(self.offset);
        e.u64(self.range);
    }

}

/// `VkDescriptorImageInfo` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkDescriptorImageInfo {
    pub sampler: VkSampler,
    pub image_view: VkImageView,
    pub image_layout: VkImageLayout,
}

impl Default for VkDescriptorImageInfo {
    fn default() -> Self {
        Self {
            sampler: VkSampler(0),
            image_view: VkImageView(0),
            image_layout: 0,
        }
    }
}

impl VkDescriptorImageInfo {
    pub fn encode(&self, e: &mut Encoder) {
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u64(self.sampler.0);
        e.u64(self.image_view.0);
        e.i32(self.image_layout);
    }

}

/// `VkDescriptorPoolCreateInfo` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkDescriptorPoolCreateInfo<'a> {
    pub flags: VkDescriptorPoolCreateFlags,
    pub max_sets: u32,
    pub pool_size_count: u32,
    pub p_pool_sizes: &'a [VkDescriptorPoolSize],
}

impl Default for VkDescriptorPoolCreateInfo<'_> {
    fn default() -> Self {
        Self {
            flags: 0,
            max_sets: 0,
            pool_size_count: 0,
            p_pool_sizes: &[],
        }
    }
}

impl<'a> VkDescriptorPoolCreateInfo<'a> {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.flags);
        e.u32(self.max_sets);
        e.u32(self.pool_size_count);
        {
            let s = e.dyn_array(self.p_pool_sizes, self.pool_size_count as usize);
            for x in s { x.encode(e); }
        }
    }

}

/// `VkDescriptorPoolSize` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkDescriptorPoolSize {
    pub r#type: VkDescriptorType,
    pub descriptor_count: u32,
}

impl Default for VkDescriptorPoolSize {
    fn default() -> Self {
        Self {
            r#type: 0,
            descriptor_count: 0,
        }
    }
}

impl VkDescriptorPoolSize {
    pub fn encode(&self, e: &mut Encoder) {
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.i32(self.r#type);
        e.u32(self.descriptor_count);
    }

}

/// `VkDescriptorSetAllocateInfo` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkDescriptorSetAllocateInfo<'a> {
    pub descriptor_pool: VkDescriptorPool,
    pub descriptor_set_count: u32,
    pub p_set_layouts: &'a [VkDescriptorSetLayout],
}

impl Default for VkDescriptorSetAllocateInfo<'_> {
    fn default() -> Self {
        Self {
            descriptor_pool: VkDescriptorPool(0),
            descriptor_set_count: 0,
            p_set_layouts: &[],
        }
    }
}

impl<'a> VkDescriptorSetAllocateInfo<'a> {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u64(self.descriptor_pool.0);
        e.u32(self.descriptor_set_count);
        {
            let s = e.dyn_array(self.p_set_layouts, self.descriptor_set_count as usize);
            for x in s { e.u64(x.0); }
        }
    }

}

/// `VkDescriptorSetLayoutBinding` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkDescriptorSetLayoutBinding<'a> {
    pub binding: u32,
    pub descriptor_type: VkDescriptorType,
    pub descriptor_count: u32,
    pub stage_flags: VkShaderStageFlags,
    pub p_immutable_samplers: &'a [VkSampler],
}

impl Default for VkDescriptorSetLayoutBinding<'_> {
    fn default() -> Self {
        Self {
            binding: 0,
            descriptor_type: 0,
            descriptor_count: 0,
            stage_flags: 0,
            p_immutable_samplers: &[],
        }
    }
}

impl<'a> VkDescriptorSetLayoutBinding<'a> {
    pub fn encode(&self, e: &mut Encoder) {
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.binding);
        e.i32(self.descriptor_type);
        e.u32(self.descriptor_count);
        e.u32(self.stage_flags);
        {
            let s = e.dyn_array(self.p_immutable_samplers, self.descriptor_count as usize);
            for x in s { e.u64(x.0); }
        }
    }

}

/// `VkDescriptorSetLayoutCreateInfo` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkDescriptorSetLayoutCreateInfo<'a> {
    pub flags: VkDescriptorSetLayoutCreateFlags,
    pub binding_count: u32,
    pub p_bindings: &'a [VkDescriptorSetLayoutBinding<'a>],
}

impl Default for VkDescriptorSetLayoutCreateInfo<'_> {
    fn default() -> Self {
        Self {
            flags: 0,
            binding_count: 0,
            p_bindings: &[],
        }
    }
}

impl<'a> VkDescriptorSetLayoutCreateInfo<'a> {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.flags);
        e.u32(self.binding_count);
        {
            let s = e.dyn_array(self.p_bindings, self.binding_count as usize);
            for x in s { x.encode(e); }
        }
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

