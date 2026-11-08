// Vygenerovano `cargo xtask venus-gen` z libs/venus/xml (vk.xml a venus-protocol,
// licence v libs/venus/xml/README.md) a libs/venus/subset.txt. Neupravovat rucne.

use alloc::vec::Vec;

use super::*;
use crate::cs::{CsError, Decoder, Encoder, Result};

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

/// `VkPhysicalDeviceSparseProperties` (vystup).
#[derive(Debug, Clone, Copy)]
pub struct VkPhysicalDeviceSparseProperties {
    pub residency_standard2_d_block_shape: VkBool32,
    pub residency_standard2_d_multisample_block_shape: VkBool32,
    pub residency_standard3_d_block_shape: VkBool32,
    pub residency_aligned_mip_size: VkBool32,
    pub residency_non_resident_strict: VkBool32,
}

impl Default for VkPhysicalDeviceSparseProperties {
    fn default() -> Self {
        Self {
            residency_standard2_d_block_shape: 0,
            residency_standard2_d_multisample_block_shape: 0,
            residency_standard3_d_block_shape: 0,
            residency_aligned_mip_size: 0,
            residency_non_resident_strict: 0,
        }
    }
}

impl VkPhysicalDeviceSparseProperties {
    #[cfg(test)]
    pub fn encode(&self, e: &mut Encoder) {
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    #[cfg(test)]
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.residency_standard2_d_block_shape);
        e.u32(self.residency_standard2_d_multisample_block_shape);
        e.u32(self.residency_standard3_d_block_shape);
        e.u32(self.residency_aligned_mip_size);
        e.u32(self.residency_non_resident_strict);
    }

    /// Castecne kodovani vystupu (sType, pNext, id handlu).
    pub fn encode_partial(&self, e: &mut Encoder) {
        let _ = e;
    }

    pub fn decode(d: &mut Decoder) -> Result<Self> {
        let mut v = Self::default();
        v.residency_standard2_d_block_shape = d.u32()?;
        v.residency_standard2_d_multisample_block_shape = d.u32()?;
        v.residency_standard3_d_block_shape = d.u32()?;
        v.residency_aligned_mip_size = d.u32()?;
        v.residency_non_resident_strict = d.u32()?;
        Ok(v)
    }
}

/// `VkPipelineLayoutCreateInfo` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkPipelineLayoutCreateInfo<'a> {
    pub flags: VkPipelineLayoutCreateFlags,
    pub set_layout_count: u32,
    pub p_set_layouts: &'a [VkDescriptorSetLayout],
    pub push_constant_range_count: u32,
    pub p_push_constant_ranges: &'a [VkPushConstantRange],
}

impl Default for VkPipelineLayoutCreateInfo<'_> {
    fn default() -> Self {
        Self {
            flags: 0,
            set_layout_count: 0,
            p_set_layouts: &[],
            push_constant_range_count: 0,
            p_push_constant_ranges: &[],
        }
    }
}

impl<'a> VkPipelineLayoutCreateInfo<'a> {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.flags);
        e.u32(self.set_layout_count);
        {
            let s = e.dyn_array(self.p_set_layouts, self.set_layout_count as usize);
            for x in s { e.u64(x.0); }
        }
        e.u32(self.push_constant_range_count);
        {
            let s = e.dyn_array(self.p_push_constant_ranges, self.push_constant_range_count as usize);
            for x in s { x.encode(e); }
        }
    }

}

/// `VkPipelineShaderStageCreateInfo` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkPipelineShaderStageCreateInfo<'a> {
    pub flags: VkPipelineShaderStageCreateFlags,
    pub stage: VkShaderStageFlagBits,
    pub module: VkShaderModule,
    pub p_name: Option<&'a str>,
    pub p_specialization_info: Option<&'a VkSpecializationInfo<'a>>,
}

impl Default for VkPipelineShaderStageCreateInfo<'_> {
    fn default() -> Self {
        Self {
            flags: 0,
            stage: 0,
            module: VkShaderModule(0),
            p_name: None,
            p_specialization_info: None,
        }
    }
}

impl<'a> VkPipelineShaderStageCreateInfo<'a> {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.flags);
        e.u32(self.stage);
        e.u64(self.module.0);
        e.opt_string(self.p_name);
        match self.p_specialization_info {
            Some(x) => {
                e.pointer(true);
                x.encode(e);
            }
            None => e.pointer(false),
        }
    }

}

/// `VkPushConstantRange` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkPushConstantRange {
    pub stage_flags: VkShaderStageFlags,
    pub offset: u32,
    pub size: u32,
}

impl Default for VkPushConstantRange {
    fn default() -> Self {
        Self {
            stage_flags: 0,
            offset: 0,
            size: 0,
        }
    }
}

impl VkPushConstantRange {
    pub fn encode(&self, e: &mut Encoder) {
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.stage_flags);
        e.u32(self.offset);
        e.u32(self.size);
    }

}

/// `VkQueueFamilyProperties` (vystup).
#[derive(Debug, Clone, Copy)]
pub struct VkQueueFamilyProperties {
    pub queue_flags: VkQueueFlags,
    pub queue_count: u32,
    pub timestamp_valid_bits: u32,
    pub min_image_transfer_granularity: VkExtent3D,
}

impl Default for VkQueueFamilyProperties {
    fn default() -> Self {
        Self {
            queue_flags: 0,
            queue_count: 0,
            timestamp_valid_bits: 0,
            min_image_transfer_granularity: VkExtent3D::default(),
        }
    }
}

impl VkQueueFamilyProperties {
    #[cfg(test)]
    pub fn encode(&self, e: &mut Encoder) {
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    #[cfg(test)]
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.queue_flags);
        e.u32(self.queue_count);
        e.u32(self.timestamp_valid_bits);
        self.min_image_transfer_granularity.encode(e);
    }

    /// Castecne kodovani vystupu (sType, pNext, id handlu).
    pub fn encode_partial(&self, e: &mut Encoder) {
        self.min_image_transfer_granularity.encode_partial(e);
    }

    pub fn decode(d: &mut Decoder) -> Result<Self> {
        let mut v = Self::default();
        v.queue_flags = d.u32()?;
        v.queue_count = d.u32()?;
        v.timestamp_valid_bits = d.u32()?;
        v.min_image_transfer_granularity = VkExtent3D::decode(d)?;
        Ok(v)
    }
}

/// `VkShaderModuleCreateInfo` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkShaderModuleCreateInfo<'a> {
    pub flags: VkShaderModuleCreateFlags,
    pub code_size: u64,
    pub p_code: &'a [u32],
}

impl Default for VkShaderModuleCreateInfo<'_> {
    fn default() -> Self {
        Self {
            flags: 0,
            code_size: 0,
            p_code: &[],
        }
    }
}

impl<'a> VkShaderModuleCreateInfo<'a> {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.flags);
        e.u64(self.code_size);
        {
            let s = e.dyn_array(self.p_code, (self.code_size as usize)/4);
            e.u32_array(s);
        }
    }

}

/// `VkSpecializationInfo` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkSpecializationInfo<'a> {
    pub map_entry_count: u32,
    pub p_map_entries: &'a [VkSpecializationMapEntry],
    pub data_size: u64,
    pub p_data: &'a [u8],
}

impl Default for VkSpecializationInfo<'_> {
    fn default() -> Self {
        Self {
            map_entry_count: 0,
            p_map_entries: &[],
            data_size: 0,
            p_data: &[],
        }
    }
}

impl<'a> VkSpecializationInfo<'a> {
    pub fn encode(&self, e: &mut Encoder) {
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.map_entry_count);
        {
            let s = e.dyn_array(self.p_map_entries, self.map_entry_count as usize);
            for x in s { x.encode(e); }
        }
        e.u64(self.data_size);
        {
            let s = e.dyn_array(self.p_data, self.data_size as usize);
            e.u8_array(s);
        }
    }

}

/// `VkSpecializationMapEntry` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkSpecializationMapEntry {
    pub constant_id: u32,
    pub offset: u32,
    pub size: u64,
}

impl Default for VkSpecializationMapEntry {
    fn default() -> Self {
        Self {
            constant_id: 0,
            offset: 0,
            size: 0,
        }
    }
}

impl VkSpecializationMapEntry {
    pub fn encode(&self, e: &mut Encoder) {
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.constant_id);
        e.u32(self.offset);
        e.u64(self.size);
    }

}

