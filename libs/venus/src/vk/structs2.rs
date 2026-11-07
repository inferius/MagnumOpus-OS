// Vygenerovano `cargo xtask venus-gen` z libs/venus/xml (vk.xml a venus-protocol,
// licence v libs/venus/xml/README.md) a libs/venus/subset.txt. Neupravovat rucne.

use alloc::vec::Vec;

use super::*;
use crate::cs::{CsError, Decoder, Encoder, Result};

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

/// `VkPhysicalDeviceLimits` (vystup).
#[derive(Debug, Clone, Copy)]
pub struct VkPhysicalDeviceLimits {
    pub max_image_dimension1_d: u32,
    pub max_image_dimension2_d: u32,
    pub max_image_dimension3_d: u32,
    pub max_image_dimension_cube: u32,
    pub max_image_array_layers: u32,
    pub max_texel_buffer_elements: u32,
    pub max_uniform_buffer_range: u32,
    pub max_storage_buffer_range: u32,
    pub max_push_constants_size: u32,
    pub max_memory_allocation_count: u32,
    pub max_sampler_allocation_count: u32,
    pub buffer_image_granularity: VkDeviceSize,
    pub sparse_address_space_size: VkDeviceSize,
    pub max_bound_descriptor_sets: u32,
    pub max_per_stage_descriptor_samplers: u32,
    pub max_per_stage_descriptor_uniform_buffers: u32,
    pub max_per_stage_descriptor_storage_buffers: u32,
    pub max_per_stage_descriptor_sampled_images: u32,
    pub max_per_stage_descriptor_storage_images: u32,
    pub max_per_stage_descriptor_input_attachments: u32,
    pub max_per_stage_resources: u32,
    pub max_descriptor_set_samplers: u32,
    pub max_descriptor_set_uniform_buffers: u32,
    pub max_descriptor_set_uniform_buffers_dynamic: u32,
    pub max_descriptor_set_storage_buffers: u32,
    pub max_descriptor_set_storage_buffers_dynamic: u32,
    pub max_descriptor_set_sampled_images: u32,
    pub max_descriptor_set_storage_images: u32,
    pub max_descriptor_set_input_attachments: u32,
    pub max_vertex_input_attributes: u32,
    pub max_vertex_input_bindings: u32,
    pub max_vertex_input_attribute_offset: u32,
    pub max_vertex_input_binding_stride: u32,
    pub max_vertex_output_components: u32,
    pub max_tessellation_generation_level: u32,
    pub max_tessellation_patch_size: u32,
    pub max_tessellation_control_per_vertex_input_components: u32,
    pub max_tessellation_control_per_vertex_output_components: u32,
    pub max_tessellation_control_per_patch_output_components: u32,
    pub max_tessellation_control_total_output_components: u32,
    pub max_tessellation_evaluation_input_components: u32,
    pub max_tessellation_evaluation_output_components: u32,
    pub max_geometry_shader_invocations: u32,
    pub max_geometry_input_components: u32,
    pub max_geometry_output_components: u32,
    pub max_geometry_output_vertices: u32,
    pub max_geometry_total_output_components: u32,
    pub max_fragment_input_components: u32,
    pub max_fragment_output_attachments: u32,
    pub max_fragment_dual_src_attachments: u32,
    pub max_fragment_combined_output_resources: u32,
    pub max_compute_shared_memory_size: u32,
    pub max_compute_work_group_count: [u32; 3],
    pub max_compute_work_group_invocations: u32,
    pub max_compute_work_group_size: [u32; 3],
    pub sub_pixel_precision_bits: u32,
    pub sub_texel_precision_bits: u32,
    pub mipmap_precision_bits: u32,
    pub max_draw_indexed_index_value: u32,
    pub max_draw_indirect_count: u32,
    pub max_sampler_lod_bias: f32,
    pub max_sampler_anisotropy: f32,
    pub max_viewports: u32,
    pub max_viewport_dimensions: [u32; 2],
    pub viewport_bounds_range: [f32; 2],
    pub viewport_sub_pixel_bits: u32,
    pub min_memory_map_alignment: u64,
    pub min_texel_buffer_offset_alignment: VkDeviceSize,
    pub min_uniform_buffer_offset_alignment: VkDeviceSize,
    pub min_storage_buffer_offset_alignment: VkDeviceSize,
    pub min_texel_offset: i32,
    pub max_texel_offset: u32,
    pub min_texel_gather_offset: i32,
    pub max_texel_gather_offset: u32,
    pub min_interpolation_offset: f32,
    pub max_interpolation_offset: f32,
    pub sub_pixel_interpolation_offset_bits: u32,
    pub max_framebuffer_width: u32,
    pub max_framebuffer_height: u32,
    pub max_framebuffer_layers: u32,
    pub framebuffer_color_sample_counts: VkSampleCountFlags,
    pub framebuffer_depth_sample_counts: VkSampleCountFlags,
    pub framebuffer_stencil_sample_counts: VkSampleCountFlags,
    pub framebuffer_no_attachments_sample_counts: VkSampleCountFlags,
    pub max_color_attachments: u32,
    pub sampled_image_color_sample_counts: VkSampleCountFlags,
    pub sampled_image_integer_sample_counts: VkSampleCountFlags,
    pub sampled_image_depth_sample_counts: VkSampleCountFlags,
    pub sampled_image_stencil_sample_counts: VkSampleCountFlags,
    pub storage_image_sample_counts: VkSampleCountFlags,
    pub max_sample_mask_words: u32,
    pub timestamp_compute_and_graphics: VkBool32,
    pub timestamp_period: f32,
    pub max_clip_distances: u32,
    pub max_cull_distances: u32,
    pub max_combined_clip_and_cull_distances: u32,
    pub discrete_queue_priorities: u32,
    pub point_size_range: [f32; 2],
    pub line_width_range: [f32; 2],
    pub point_size_granularity: f32,
    pub line_width_granularity: f32,
    pub strict_lines: VkBool32,
    pub standard_sample_locations: VkBool32,
    pub optimal_buffer_copy_offset_alignment: VkDeviceSize,
    pub optimal_buffer_copy_row_pitch_alignment: VkDeviceSize,
    pub non_coherent_atom_size: VkDeviceSize,
}

impl Default for VkPhysicalDeviceLimits {
    fn default() -> Self {
        Self {
            max_image_dimension1_d: 0,
            max_image_dimension2_d: 0,
            max_image_dimension3_d: 0,
            max_image_dimension_cube: 0,
            max_image_array_layers: 0,
            max_texel_buffer_elements: 0,
            max_uniform_buffer_range: 0,
            max_storage_buffer_range: 0,
            max_push_constants_size: 0,
            max_memory_allocation_count: 0,
            max_sampler_allocation_count: 0,
            buffer_image_granularity: 0,
            sparse_address_space_size: 0,
            max_bound_descriptor_sets: 0,
            max_per_stage_descriptor_samplers: 0,
            max_per_stage_descriptor_uniform_buffers: 0,
            max_per_stage_descriptor_storage_buffers: 0,
            max_per_stage_descriptor_sampled_images: 0,
            max_per_stage_descriptor_storage_images: 0,
            max_per_stage_descriptor_input_attachments: 0,
            max_per_stage_resources: 0,
            max_descriptor_set_samplers: 0,
            max_descriptor_set_uniform_buffers: 0,
            max_descriptor_set_uniform_buffers_dynamic: 0,
            max_descriptor_set_storage_buffers: 0,
            max_descriptor_set_storage_buffers_dynamic: 0,
            max_descriptor_set_sampled_images: 0,
            max_descriptor_set_storage_images: 0,
            max_descriptor_set_input_attachments: 0,
            max_vertex_input_attributes: 0,
            max_vertex_input_bindings: 0,
            max_vertex_input_attribute_offset: 0,
            max_vertex_input_binding_stride: 0,
            max_vertex_output_components: 0,
            max_tessellation_generation_level: 0,
            max_tessellation_patch_size: 0,
            max_tessellation_control_per_vertex_input_components: 0,
            max_tessellation_control_per_vertex_output_components: 0,
            max_tessellation_control_per_patch_output_components: 0,
            max_tessellation_control_total_output_components: 0,
            max_tessellation_evaluation_input_components: 0,
            max_tessellation_evaluation_output_components: 0,
            max_geometry_shader_invocations: 0,
            max_geometry_input_components: 0,
            max_geometry_output_components: 0,
            max_geometry_output_vertices: 0,
            max_geometry_total_output_components: 0,
            max_fragment_input_components: 0,
            max_fragment_output_attachments: 0,
            max_fragment_dual_src_attachments: 0,
            max_fragment_combined_output_resources: 0,
            max_compute_shared_memory_size: 0,
            max_compute_work_group_count: [0; 3],
            max_compute_work_group_invocations: 0,
            max_compute_work_group_size: [0; 3],
            sub_pixel_precision_bits: 0,
            sub_texel_precision_bits: 0,
            mipmap_precision_bits: 0,
            max_draw_indexed_index_value: 0,
            max_draw_indirect_count: 0,
            max_sampler_lod_bias: 0.0,
            max_sampler_anisotropy: 0.0,
            max_viewports: 0,
            max_viewport_dimensions: [0; 2],
            viewport_bounds_range: [0.0; 2],
            viewport_sub_pixel_bits: 0,
            min_memory_map_alignment: 0,
            min_texel_buffer_offset_alignment: 0,
            min_uniform_buffer_offset_alignment: 0,
            min_storage_buffer_offset_alignment: 0,
            min_texel_offset: 0,
            max_texel_offset: 0,
            min_texel_gather_offset: 0,
            max_texel_gather_offset: 0,
            min_interpolation_offset: 0.0,
            max_interpolation_offset: 0.0,
            sub_pixel_interpolation_offset_bits: 0,
            max_framebuffer_width: 0,
            max_framebuffer_height: 0,
            max_framebuffer_layers: 0,
            framebuffer_color_sample_counts: 0,
            framebuffer_depth_sample_counts: 0,
            framebuffer_stencil_sample_counts: 0,
            framebuffer_no_attachments_sample_counts: 0,
            max_color_attachments: 0,
            sampled_image_color_sample_counts: 0,
            sampled_image_integer_sample_counts: 0,
            sampled_image_depth_sample_counts: 0,
            sampled_image_stencil_sample_counts: 0,
            storage_image_sample_counts: 0,
            max_sample_mask_words: 0,
            timestamp_compute_and_graphics: 0,
            timestamp_period: 0.0,
            max_clip_distances: 0,
            max_cull_distances: 0,
            max_combined_clip_and_cull_distances: 0,
            discrete_queue_priorities: 0,
            point_size_range: [0.0; 2],
            line_width_range: [0.0; 2],
            point_size_granularity: 0.0,
            line_width_granularity: 0.0,
            strict_lines: 0,
            standard_sample_locations: 0,
            optimal_buffer_copy_offset_alignment: 0,
            optimal_buffer_copy_row_pitch_alignment: 0,
            non_coherent_atom_size: 0,
        }
    }
}

impl VkPhysicalDeviceLimits {
    #[cfg(test)]
    pub fn encode(&self, e: &mut Encoder) {
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    #[cfg(test)]
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.max_image_dimension1_d);
        e.u32(self.max_image_dimension2_d);
        e.u32(self.max_image_dimension3_d);
        e.u32(self.max_image_dimension_cube);
        e.u32(self.max_image_array_layers);
        e.u32(self.max_texel_buffer_elements);
        e.u32(self.max_uniform_buffer_range);
        e.u32(self.max_storage_buffer_range);
        e.u32(self.max_push_constants_size);
        e.u32(self.max_memory_allocation_count);
        e.u32(self.max_sampler_allocation_count);
        e.u64(self.buffer_image_granularity);
        e.u64(self.sparse_address_space_size);
        e.u32(self.max_bound_descriptor_sets);
        e.u32(self.max_per_stage_descriptor_samplers);
        e.u32(self.max_per_stage_descriptor_uniform_buffers);
        e.u32(self.max_per_stage_descriptor_storage_buffers);
        e.u32(self.max_per_stage_descriptor_sampled_images);
        e.u32(self.max_per_stage_descriptor_storage_images);
        e.u32(self.max_per_stage_descriptor_input_attachments);
        e.u32(self.max_per_stage_resources);
        e.u32(self.max_descriptor_set_samplers);
        e.u32(self.max_descriptor_set_uniform_buffers);
        e.u32(self.max_descriptor_set_uniform_buffers_dynamic);
        e.u32(self.max_descriptor_set_storage_buffers);
        e.u32(self.max_descriptor_set_storage_buffers_dynamic);
        e.u32(self.max_descriptor_set_sampled_images);
        e.u32(self.max_descriptor_set_storage_images);
        e.u32(self.max_descriptor_set_input_attachments);
        e.u32(self.max_vertex_input_attributes);
        e.u32(self.max_vertex_input_bindings);
        e.u32(self.max_vertex_input_attribute_offset);
        e.u32(self.max_vertex_input_binding_stride);
        e.u32(self.max_vertex_output_components);
        e.u32(self.max_tessellation_generation_level);
        e.u32(self.max_tessellation_patch_size);
        e.u32(self.max_tessellation_control_per_vertex_input_components);
        e.u32(self.max_tessellation_control_per_vertex_output_components);
        e.u32(self.max_tessellation_control_per_patch_output_components);
        e.u32(self.max_tessellation_control_total_output_components);
        e.u32(self.max_tessellation_evaluation_input_components);
        e.u32(self.max_tessellation_evaluation_output_components);
        e.u32(self.max_geometry_shader_invocations);
        e.u32(self.max_geometry_input_components);
        e.u32(self.max_geometry_output_components);
        e.u32(self.max_geometry_output_vertices);
        e.u32(self.max_geometry_total_output_components);
        e.u32(self.max_fragment_input_components);
        e.u32(self.max_fragment_output_attachments);
        e.u32(self.max_fragment_dual_src_attachments);
        e.u32(self.max_fragment_combined_output_resources);
        e.u32(self.max_compute_shared_memory_size);
        e.array_size(3);
        e.u32_array(&self.max_compute_work_group_count);
        e.u32(self.max_compute_work_group_invocations);
        e.array_size(3);
        e.u32_array(&self.max_compute_work_group_size);
        e.u32(self.sub_pixel_precision_bits);
        e.u32(self.sub_texel_precision_bits);
        e.u32(self.mipmap_precision_bits);
        e.u32(self.max_draw_indexed_index_value);
        e.u32(self.max_draw_indirect_count);
        e.f32(self.max_sampler_lod_bias);
        e.f32(self.max_sampler_anisotropy);
        e.u32(self.max_viewports);
        e.array_size(2);
        e.u32_array(&self.max_viewport_dimensions);
        e.array_size(2);
        e.f32_array(&self.viewport_bounds_range);
        e.u32(self.viewport_sub_pixel_bits);
        e.u64(self.min_memory_map_alignment);
        e.u64(self.min_texel_buffer_offset_alignment);
        e.u64(self.min_uniform_buffer_offset_alignment);
        e.u64(self.min_storage_buffer_offset_alignment);
        e.i32(self.min_texel_offset);
        e.u32(self.max_texel_offset);
        e.i32(self.min_texel_gather_offset);
        e.u32(self.max_texel_gather_offset);
        e.f32(self.min_interpolation_offset);
        e.f32(self.max_interpolation_offset);
        e.u32(self.sub_pixel_interpolation_offset_bits);
        e.u32(self.max_framebuffer_width);
        e.u32(self.max_framebuffer_height);
        e.u32(self.max_framebuffer_layers);
        e.u32(self.framebuffer_color_sample_counts);
        e.u32(self.framebuffer_depth_sample_counts);
        e.u32(self.framebuffer_stencil_sample_counts);
        e.u32(self.framebuffer_no_attachments_sample_counts);
        e.u32(self.max_color_attachments);
        e.u32(self.sampled_image_color_sample_counts);
        e.u32(self.sampled_image_integer_sample_counts);
        e.u32(self.sampled_image_depth_sample_counts);
        e.u32(self.sampled_image_stencil_sample_counts);
        e.u32(self.storage_image_sample_counts);
        e.u32(self.max_sample_mask_words);
        e.u32(self.timestamp_compute_and_graphics);
        e.f32(self.timestamp_period);
        e.u32(self.max_clip_distances);
        e.u32(self.max_cull_distances);
        e.u32(self.max_combined_clip_and_cull_distances);
        e.u32(self.discrete_queue_priorities);
        e.array_size(2);
        e.f32_array(&self.point_size_range);
        e.array_size(2);
        e.f32_array(&self.line_width_range);
        e.f32(self.point_size_granularity);
        e.f32(self.line_width_granularity);
        e.u32(self.strict_lines);
        e.u32(self.standard_sample_locations);
        e.u64(self.optimal_buffer_copy_offset_alignment);
        e.u64(self.optimal_buffer_copy_row_pitch_alignment);
        e.u64(self.non_coherent_atom_size);
    }

    /// Castecne kodovani vystupu (sType, pNext, id handlu).
    pub fn encode_partial(&self, e: &mut Encoder) {
        let _ = e;
    }

    pub fn decode(d: &mut Decoder) -> Result<Self> {
        let mut v = Self::default();
        v.max_image_dimension1_d = d.u32()?;
        v.max_image_dimension2_d = d.u32()?;
        v.max_image_dimension3_d = d.u32()?;
        v.max_image_dimension_cube = d.u32()?;
        v.max_image_array_layers = d.u32()?;
        v.max_texel_buffer_elements = d.u32()?;
        v.max_uniform_buffer_range = d.u32()?;
        v.max_storage_buffer_range = d.u32()?;
        v.max_push_constants_size = d.u32()?;
        v.max_memory_allocation_count = d.u32()?;
        v.max_sampler_allocation_count = d.u32()?;
        v.buffer_image_granularity = d.u64()?;
        v.sparse_address_space_size = d.u64()?;
        v.max_bound_descriptor_sets = d.u32()?;
        v.max_per_stage_descriptor_samplers = d.u32()?;
        v.max_per_stage_descriptor_uniform_buffers = d.u32()?;
        v.max_per_stage_descriptor_storage_buffers = d.u32()?;
        v.max_per_stage_descriptor_sampled_images = d.u32()?;
        v.max_per_stage_descriptor_storage_images = d.u32()?;
        v.max_per_stage_descriptor_input_attachments = d.u32()?;
        v.max_per_stage_resources = d.u32()?;
        v.max_descriptor_set_samplers = d.u32()?;
        v.max_descriptor_set_uniform_buffers = d.u32()?;
        v.max_descriptor_set_uniform_buffers_dynamic = d.u32()?;
        v.max_descriptor_set_storage_buffers = d.u32()?;
        v.max_descriptor_set_storage_buffers_dynamic = d.u32()?;
        v.max_descriptor_set_sampled_images = d.u32()?;
        v.max_descriptor_set_storage_images = d.u32()?;
        v.max_descriptor_set_input_attachments = d.u32()?;
        v.max_vertex_input_attributes = d.u32()?;
        v.max_vertex_input_bindings = d.u32()?;
        v.max_vertex_input_attribute_offset = d.u32()?;
        v.max_vertex_input_binding_stride = d.u32()?;
        v.max_vertex_output_components = d.u32()?;
        v.max_tessellation_generation_level = d.u32()?;
        v.max_tessellation_patch_size = d.u32()?;
        v.max_tessellation_control_per_vertex_input_components = d.u32()?;
        v.max_tessellation_control_per_vertex_output_components = d.u32()?;
        v.max_tessellation_control_per_patch_output_components = d.u32()?;
        v.max_tessellation_control_total_output_components = d.u32()?;
        v.max_tessellation_evaluation_input_components = d.u32()?;
        v.max_tessellation_evaluation_output_components = d.u32()?;
        v.max_geometry_shader_invocations = d.u32()?;
        v.max_geometry_input_components = d.u32()?;
        v.max_geometry_output_components = d.u32()?;
        v.max_geometry_output_vertices = d.u32()?;
        v.max_geometry_total_output_components = d.u32()?;
        v.max_fragment_input_components = d.u32()?;
        v.max_fragment_output_attachments = d.u32()?;
        v.max_fragment_dual_src_attachments = d.u32()?;
        v.max_fragment_combined_output_resources = d.u32()?;
        v.max_compute_shared_memory_size = d.u32()?;
        d.fixed_array(3)?;
        d.u32_array(&mut v.max_compute_work_group_count)?;
        v.max_compute_work_group_invocations = d.u32()?;
        d.fixed_array(3)?;
        d.u32_array(&mut v.max_compute_work_group_size)?;
        v.sub_pixel_precision_bits = d.u32()?;
        v.sub_texel_precision_bits = d.u32()?;
        v.mipmap_precision_bits = d.u32()?;
        v.max_draw_indexed_index_value = d.u32()?;
        v.max_draw_indirect_count = d.u32()?;
        v.max_sampler_lod_bias = d.f32()?;
        v.max_sampler_anisotropy = d.f32()?;
        v.max_viewports = d.u32()?;
        d.fixed_array(2)?;
        d.u32_array(&mut v.max_viewport_dimensions)?;
        d.fixed_array(2)?;
        d.f32_array(&mut v.viewport_bounds_range)?;
        v.viewport_sub_pixel_bits = d.u32()?;
        v.min_memory_map_alignment = d.u64()?;
        v.min_texel_buffer_offset_alignment = d.u64()?;
        v.min_uniform_buffer_offset_alignment = d.u64()?;
        v.min_storage_buffer_offset_alignment = d.u64()?;
        v.min_texel_offset = d.i32()?;
        v.max_texel_offset = d.u32()?;
        v.min_texel_gather_offset = d.i32()?;
        v.max_texel_gather_offset = d.u32()?;
        v.min_interpolation_offset = d.f32()?;
        v.max_interpolation_offset = d.f32()?;
        v.sub_pixel_interpolation_offset_bits = d.u32()?;
        v.max_framebuffer_width = d.u32()?;
        v.max_framebuffer_height = d.u32()?;
        v.max_framebuffer_layers = d.u32()?;
        v.framebuffer_color_sample_counts = d.u32()?;
        v.framebuffer_depth_sample_counts = d.u32()?;
        v.framebuffer_stencil_sample_counts = d.u32()?;
        v.framebuffer_no_attachments_sample_counts = d.u32()?;
        v.max_color_attachments = d.u32()?;
        v.sampled_image_color_sample_counts = d.u32()?;
        v.sampled_image_integer_sample_counts = d.u32()?;
        v.sampled_image_depth_sample_counts = d.u32()?;
        v.sampled_image_stencil_sample_counts = d.u32()?;
        v.storage_image_sample_counts = d.u32()?;
        v.max_sample_mask_words = d.u32()?;
        v.timestamp_compute_and_graphics = d.u32()?;
        v.timestamp_period = d.f32()?;
        v.max_clip_distances = d.u32()?;
        v.max_cull_distances = d.u32()?;
        v.max_combined_clip_and_cull_distances = d.u32()?;
        v.discrete_queue_priorities = d.u32()?;
        d.fixed_array(2)?;
        d.f32_array(&mut v.point_size_range)?;
        d.fixed_array(2)?;
        d.f32_array(&mut v.line_width_range)?;
        v.point_size_granularity = d.f32()?;
        v.line_width_granularity = d.f32()?;
        v.strict_lines = d.u32()?;
        v.standard_sample_locations = d.u32()?;
        v.optimal_buffer_copy_offset_alignment = d.u64()?;
        v.optimal_buffer_copy_row_pitch_alignment = d.u64()?;
        v.non_coherent_atom_size = d.u64()?;
        Ok(v)
    }
}

/// `VkPhysicalDeviceMemoryProperties` (vystup).
#[derive(Debug, Clone, Copy)]
pub struct VkPhysicalDeviceMemoryProperties {
    pub memory_type_count: u32,
    pub memory_types: [VkMemoryType; 32],
    pub memory_heap_count: u32,
    pub memory_heaps: [VkMemoryHeap; 16],
}

impl Default for VkPhysicalDeviceMemoryProperties {
    fn default() -> Self {
        Self {
            memory_type_count: 0,
            memory_types: [VkMemoryType::default(); 32],
            memory_heap_count: 0,
            memory_heaps: [VkMemoryHeap::default(); 16],
        }
    }
}

impl VkPhysicalDeviceMemoryProperties {
    #[cfg(test)]
    pub fn encode(&self, e: &mut Encoder) {
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    #[cfg(test)]
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.memory_type_count);
        e.array_size(32);
        for x in self.memory_types.iter() { x.encode(e); }
        e.u32(self.memory_heap_count);
        e.array_size(16);
        for x in self.memory_heaps.iter() { x.encode(e); }
    }

    /// Castecne kodovani vystupu (sType, pNext, id handlu).
    pub fn encode_partial(&self, e: &mut Encoder) {
        e.array_size(32);
        for x in self.memory_types.iter() { x.encode_partial(e); }
        e.array_size(16);
        for x in self.memory_heaps.iter() { x.encode_partial(e); }
    }

    pub fn decode(d: &mut Decoder) -> Result<Self> {
        let mut v = Self::default();
        v.memory_type_count = d.u32()?;
        d.fixed_array(32)?;
        for x in v.memory_types.iter_mut() { *x = VkMemoryType::decode(d)?; }
        v.memory_heap_count = d.u32()?;
        d.fixed_array(16)?;
        for x in v.memory_heaps.iter_mut() { *x = VkMemoryHeap::decode(d)?; }
        Ok(v)
    }
}

/// `VkPhysicalDeviceProperties` (vystup).
#[derive(Debug, Clone, Copy)]
pub struct VkPhysicalDeviceProperties {
    pub api_version: u32,
    pub driver_version: u32,
    pub vendor_id: u32,
    pub device_id: u32,
    pub device_type: VkPhysicalDeviceType,
    pub device_name: [u8; 256],
    pub pipeline_cache_uuid: [u8; 16],
    pub limits: VkPhysicalDeviceLimits,
    pub sparse_properties: VkPhysicalDeviceSparseProperties,
}

impl Default for VkPhysicalDeviceProperties {
    fn default() -> Self {
        Self {
            api_version: 0,
            driver_version: 0,
            vendor_id: 0,
            device_id: 0,
            device_type: 0,
            device_name: [0; 256],
            pipeline_cache_uuid: [0; 16],
            limits: VkPhysicalDeviceLimits::default(),
            sparse_properties: VkPhysicalDeviceSparseProperties::default(),
        }
    }
}

impl VkPhysicalDeviceProperties {
    #[cfg(test)]
    pub fn encode(&self, e: &mut Encoder) {
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    #[cfg(test)]
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.api_version);
        e.u32(self.driver_version);
        e.u32(self.vendor_id);
        e.u32(self.device_id);
        e.i32(self.device_type);
        e.array_size(256);
        e.u8_array(&self.device_name);
        e.array_size(16);
        e.u8_array(&self.pipeline_cache_uuid);
        self.limits.encode(e);
        self.sparse_properties.encode(e);
    }

    /// Castecne kodovani vystupu (sType, pNext, id handlu).
    pub fn encode_partial(&self, e: &mut Encoder) {
        self.limits.encode_partial(e);
        self.sparse_properties.encode_partial(e);
    }

    pub fn decode(d: &mut Decoder) -> Result<Self> {
        let mut v = Self::default();
        v.api_version = d.u32()?;
        v.driver_version = d.u32()?;
        v.vendor_id = d.u32()?;
        v.device_id = d.u32()?;
        v.device_type = d.i32()?;
        d.fixed_array(256)?;
        d.char_array(&mut v.device_name)?;
        d.fixed_array(16)?;
        d.u8_array(&mut v.pipeline_cache_uuid)?;
        v.limits = VkPhysicalDeviceLimits::decode(d)?;
        v.sparse_properties = VkPhysicalDeviceSparseProperties::decode(d)?;
        Ok(v)
    }
}

