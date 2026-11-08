// Vygenerovano `cargo xtask venus-gen` z libs/venus/xml (vk.xml a venus-protocol,
// licence v libs/venus/xml/README.md) a libs/venus/subset.txt. Neupravovat rucne.

use alloc::vec::Vec;

use super::*;
use crate::cs::{CsError, Decoder, Encoder, Result};

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

/// `VkMemoryBarrier` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkMemoryBarrier {
    pub src_access_mask: VkAccessFlags,
    pub dst_access_mask: VkAccessFlags,
}

impl Default for VkMemoryBarrier {
    fn default() -> Self {
        Self {
            src_access_mask: 0,
            dst_access_mask: 0,
        }
    }
}

impl VkMemoryBarrier {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_MEMORY_BARRIER;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.src_access_mask);
        e.u32(self.dst_access_mask);
    }

}

/// `VkMemoryHeap` (vystup).
#[derive(Debug, Clone, Copy)]
pub struct VkMemoryHeap {
    pub size: VkDeviceSize,
    pub flags: VkMemoryHeapFlags,
}

impl Default for VkMemoryHeap {
    fn default() -> Self {
        Self {
            size: 0,
            flags: 0,
        }
    }
}

impl VkMemoryHeap {
    #[cfg(test)]
    pub fn encode(&self, e: &mut Encoder) {
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    #[cfg(test)]
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u64(self.size);
        e.u32(self.flags);
    }

    /// Castecne kodovani vystupu (sType, pNext, id handlu).
    pub fn encode_partial(&self, e: &mut Encoder) {
        let _ = e;
    }

    pub fn decode(d: &mut Decoder) -> Result<Self> {
        let mut v = Self::default();
        v.size = d.u64()?;
        v.flags = d.u32()?;
        Ok(v)
    }
}

/// `VkMemoryRequirements` (vystup).
#[derive(Debug, Clone, Copy)]
pub struct VkMemoryRequirements {
    pub size: VkDeviceSize,
    pub alignment: VkDeviceSize,
    pub memory_type_bits: u32,
}

impl Default for VkMemoryRequirements {
    fn default() -> Self {
        Self {
            size: 0,
            alignment: 0,
            memory_type_bits: 0,
        }
    }
}

impl VkMemoryRequirements {
    #[cfg(test)]
    pub fn encode(&self, e: &mut Encoder) {
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    #[cfg(test)]
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u64(self.size);
        e.u64(self.alignment);
        e.u32(self.memory_type_bits);
    }

    /// Castecne kodovani vystupu (sType, pNext, id handlu).
    pub fn encode_partial(&self, e: &mut Encoder) {
        let _ = e;
    }

    pub fn decode(d: &mut Decoder) -> Result<Self> {
        let mut v = Self::default();
        v.size = d.u64()?;
        v.alignment = d.u64()?;
        v.memory_type_bits = d.u32()?;
        Ok(v)
    }
}

/// `VkMemoryType` (vystup).
#[derive(Debug, Clone, Copy)]
pub struct VkMemoryType {
    pub property_flags: VkMemoryPropertyFlags,
    pub heap_index: u32,
}

impl Default for VkMemoryType {
    fn default() -> Self {
        Self {
            property_flags: 0,
            heap_index: 0,
        }
    }
}

impl VkMemoryType {
    #[cfg(test)]
    pub fn encode(&self, e: &mut Encoder) {
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    #[cfg(test)]
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.property_flags);
        e.u32(self.heap_index);
    }

    /// Castecne kodovani vystupu (sType, pNext, id handlu).
    pub fn encode_partial(&self, e: &mut Encoder) {
        let _ = e;
    }

    pub fn decode(d: &mut Decoder) -> Result<Self> {
        let mut v = Self::default();
        v.property_flags = d.u32()?;
        v.heap_index = d.u32()?;
        Ok(v)
    }
}

/// `VkOffset3D` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkOffset3D {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl Default for VkOffset3D {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            z: 0,
        }
    }
}

impl VkOffset3D {
    pub fn encode(&self, e: &mut Encoder) {
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.i32(self.x);
        e.i32(self.y);
        e.i32(self.z);
    }

}

/// `VkPhysicalDeviceFeatures` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkPhysicalDeviceFeatures {
    pub robust_buffer_access: VkBool32,
    pub full_draw_index_uint32: VkBool32,
    pub image_cube_array: VkBool32,
    pub independent_blend: VkBool32,
    pub geometry_shader: VkBool32,
    pub tessellation_shader: VkBool32,
    pub sample_rate_shading: VkBool32,
    pub dual_src_blend: VkBool32,
    pub logic_op: VkBool32,
    pub multi_draw_indirect: VkBool32,
    pub draw_indirect_first_instance: VkBool32,
    pub depth_clamp: VkBool32,
    pub depth_bias_clamp: VkBool32,
    pub fill_mode_non_solid: VkBool32,
    pub depth_bounds: VkBool32,
    pub wide_lines: VkBool32,
    pub large_points: VkBool32,
    pub alpha_to_one: VkBool32,
    pub multi_viewport: VkBool32,
    pub sampler_anisotropy: VkBool32,
    pub texture_compression_etc2: VkBool32,
    pub texture_compression_astc_ldr: VkBool32,
    pub texture_compression_bc: VkBool32,
    pub occlusion_query_precise: VkBool32,
    pub pipeline_statistics_query: VkBool32,
    pub vertex_pipeline_stores_and_atomics: VkBool32,
    pub fragment_stores_and_atomics: VkBool32,
    pub shader_tessellation_and_geometry_point_size: VkBool32,
    pub shader_image_gather_extended: VkBool32,
    pub shader_storage_image_extended_formats: VkBool32,
    pub shader_storage_image_multisample: VkBool32,
    pub shader_storage_image_read_without_format: VkBool32,
    pub shader_storage_image_write_without_format: VkBool32,
    pub shader_uniform_buffer_array_dynamic_indexing: VkBool32,
    pub shader_sampled_image_array_dynamic_indexing: VkBool32,
    pub shader_storage_buffer_array_dynamic_indexing: VkBool32,
    pub shader_storage_image_array_dynamic_indexing: VkBool32,
    pub shader_clip_distance: VkBool32,
    pub shader_cull_distance: VkBool32,
    pub shader_float64: VkBool32,
    pub shader_int64: VkBool32,
    pub shader_int16: VkBool32,
    pub shader_resource_residency: VkBool32,
    pub shader_resource_min_lod: VkBool32,
    pub sparse_binding: VkBool32,
    pub sparse_residency_buffer: VkBool32,
    pub sparse_residency_image2_d: VkBool32,
    pub sparse_residency_image3_d: VkBool32,
    pub sparse_residency2_samples: VkBool32,
    pub sparse_residency4_samples: VkBool32,
    pub sparse_residency8_samples: VkBool32,
    pub sparse_residency16_samples: VkBool32,
    pub sparse_residency_aliased: VkBool32,
    pub variable_multisample_rate: VkBool32,
    pub inherited_queries: VkBool32,
}

impl Default for VkPhysicalDeviceFeatures {
    fn default() -> Self {
        Self {
            robust_buffer_access: 0,
            full_draw_index_uint32: 0,
            image_cube_array: 0,
            independent_blend: 0,
            geometry_shader: 0,
            tessellation_shader: 0,
            sample_rate_shading: 0,
            dual_src_blend: 0,
            logic_op: 0,
            multi_draw_indirect: 0,
            draw_indirect_first_instance: 0,
            depth_clamp: 0,
            depth_bias_clamp: 0,
            fill_mode_non_solid: 0,
            depth_bounds: 0,
            wide_lines: 0,
            large_points: 0,
            alpha_to_one: 0,
            multi_viewport: 0,
            sampler_anisotropy: 0,
            texture_compression_etc2: 0,
            texture_compression_astc_ldr: 0,
            texture_compression_bc: 0,
            occlusion_query_precise: 0,
            pipeline_statistics_query: 0,
            vertex_pipeline_stores_and_atomics: 0,
            fragment_stores_and_atomics: 0,
            shader_tessellation_and_geometry_point_size: 0,
            shader_image_gather_extended: 0,
            shader_storage_image_extended_formats: 0,
            shader_storage_image_multisample: 0,
            shader_storage_image_read_without_format: 0,
            shader_storage_image_write_without_format: 0,
            shader_uniform_buffer_array_dynamic_indexing: 0,
            shader_sampled_image_array_dynamic_indexing: 0,
            shader_storage_buffer_array_dynamic_indexing: 0,
            shader_storage_image_array_dynamic_indexing: 0,
            shader_clip_distance: 0,
            shader_cull_distance: 0,
            shader_float64: 0,
            shader_int64: 0,
            shader_int16: 0,
            shader_resource_residency: 0,
            shader_resource_min_lod: 0,
            sparse_binding: 0,
            sparse_residency_buffer: 0,
            sparse_residency_image2_d: 0,
            sparse_residency_image3_d: 0,
            sparse_residency2_samples: 0,
            sparse_residency4_samples: 0,
            sparse_residency8_samples: 0,
            sparse_residency16_samples: 0,
            sparse_residency_aliased: 0,
            variable_multisample_rate: 0,
            inherited_queries: 0,
        }
    }
}

impl VkPhysicalDeviceFeatures {
    pub fn encode(&self, e: &mut Encoder) {
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.robust_buffer_access);
        e.u32(self.full_draw_index_uint32);
        e.u32(self.image_cube_array);
        e.u32(self.independent_blend);
        e.u32(self.geometry_shader);
        e.u32(self.tessellation_shader);
        e.u32(self.sample_rate_shading);
        e.u32(self.dual_src_blend);
        e.u32(self.logic_op);
        e.u32(self.multi_draw_indirect);
        e.u32(self.draw_indirect_first_instance);
        e.u32(self.depth_clamp);
        e.u32(self.depth_bias_clamp);
        e.u32(self.fill_mode_non_solid);
        e.u32(self.depth_bounds);
        e.u32(self.wide_lines);
        e.u32(self.large_points);
        e.u32(self.alpha_to_one);
        e.u32(self.multi_viewport);
        e.u32(self.sampler_anisotropy);
        e.u32(self.texture_compression_etc2);
        e.u32(self.texture_compression_astc_ldr);
        e.u32(self.texture_compression_bc);
        e.u32(self.occlusion_query_precise);
        e.u32(self.pipeline_statistics_query);
        e.u32(self.vertex_pipeline_stores_and_atomics);
        e.u32(self.fragment_stores_and_atomics);
        e.u32(self.shader_tessellation_and_geometry_point_size);
        e.u32(self.shader_image_gather_extended);
        e.u32(self.shader_storage_image_extended_formats);
        e.u32(self.shader_storage_image_multisample);
        e.u32(self.shader_storage_image_read_without_format);
        e.u32(self.shader_storage_image_write_without_format);
        e.u32(self.shader_uniform_buffer_array_dynamic_indexing);
        e.u32(self.shader_sampled_image_array_dynamic_indexing);
        e.u32(self.shader_storage_buffer_array_dynamic_indexing);
        e.u32(self.shader_storage_image_array_dynamic_indexing);
        e.u32(self.shader_clip_distance);
        e.u32(self.shader_cull_distance);
        e.u32(self.shader_float64);
        e.u32(self.shader_int64);
        e.u32(self.shader_int16);
        e.u32(self.shader_resource_residency);
        e.u32(self.shader_resource_min_lod);
        e.u32(self.sparse_binding);
        e.u32(self.sparse_residency_buffer);
        e.u32(self.sparse_residency_image2_d);
        e.u32(self.sparse_residency_image3_d);
        e.u32(self.sparse_residency2_samples);
        e.u32(self.sparse_residency4_samples);
        e.u32(self.sparse_residency8_samples);
        e.u32(self.sparse_residency16_samples);
        e.u32(self.sparse_residency_aliased);
        e.u32(self.variable_multisample_rate);
        e.u32(self.inherited_queries);
    }

}

