// Vygenerovano `cargo xtask venus-gen` z libs/venus/xml (vk.xml a venus-protocol,
// licence v libs/venus/xml/README.md) a libs/venus/subset.txt. Neupravovat rucne.

use alloc::vec::Vec;

use super::*;
use crate::cs::{CsError, Decoder, Encoder, Result};

/// `VkSubmitInfo` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkSubmitInfo<'a> {
    pub wait_semaphore_count: u32,
    pub p_wait_semaphores: &'a [VkSemaphore],
    pub p_wait_dst_stage_mask: &'a [VkPipelineStageFlags],
    pub command_buffer_count: u32,
    pub p_command_buffers: &'a [VkCommandBuffer],
    pub signal_semaphore_count: u32,
    pub p_signal_semaphores: &'a [VkSemaphore],
}

impl Default for VkSubmitInfo<'_> {
    fn default() -> Self {
        Self {
            wait_semaphore_count: 0,
            p_wait_semaphores: &[],
            p_wait_dst_stage_mask: &[],
            command_buffer_count: 0,
            p_command_buffers: &[],
            signal_semaphore_count: 0,
            p_signal_semaphores: &[],
        }
    }
}

impl<'a> VkSubmitInfo<'a> {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_SUBMIT_INFO;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u32(self.wait_semaphore_count);
        {
            let s = e.dyn_array(self.p_wait_semaphores, self.wait_semaphore_count as usize);
            for x in s { e.u64(x.0); }
        }
        {
            let s = e.dyn_array(self.p_wait_dst_stage_mask, self.wait_semaphore_count as usize);
            e.u32_array(s);
        }
        e.u32(self.command_buffer_count);
        {
            let s = e.dyn_array(self.p_command_buffers, self.command_buffer_count as usize);
            for x in s { e.u64(x.0); }
        }
        e.u32(self.signal_semaphore_count);
        {
            let s = e.dyn_array(self.p_signal_semaphores, self.signal_semaphore_count as usize);
            for x in s { e.u64(x.0); }
        }
    }

}

/// `VkWriteDescriptorSet` (vstup).
#[derive(Debug, Clone, Copy)]
pub struct VkWriteDescriptorSet<'a> {
    pub dst_set: VkDescriptorSet,
    pub dst_binding: u32,
    pub dst_array_element: u32,
    pub descriptor_count: u32,
    pub descriptor_type: VkDescriptorType,
    pub p_image_info: &'a [VkDescriptorImageInfo],
    pub p_buffer_info: &'a [VkDescriptorBufferInfo],
    pub p_texel_buffer_view: &'a [VkBufferView],
}

impl Default for VkWriteDescriptorSet<'_> {
    fn default() -> Self {
        Self {
            dst_set: VkDescriptorSet(0),
            dst_binding: 0,
            dst_array_element: 0,
            descriptor_count: 0,
            descriptor_type: 0,
            p_image_info: &[],
            p_buffer_info: &[],
            p_texel_buffer_view: &[],
        }
    }
}

impl<'a> VkWriteDescriptorSet<'a> {
    pub const S_TYPE: VkStructureType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET;

    pub fn encode(&self, e: &mut Encoder) {
        e.i32(Self::S_TYPE);
        e.pointer(false);
        self.encode_self(e);
    }

    /// Cleny bez sType a pNext.
    pub fn encode_self(&self, e: &mut Encoder) {
        e.u64(self.dst_set.0);
        e.u32(self.dst_binding);
        e.u32(self.dst_array_element);
        e.u32(self.descriptor_count);
        e.i32(self.descriptor_type);
        {
            let s = e.dyn_array(self.p_image_info, self.descriptor_count as usize);
            for x in s { x.encode(e); }
        }
        {
            let s = e.dyn_array(self.p_buffer_info, self.descriptor_count as usize);
            for x in s { x.encode(e); }
        }
        {
            let s = e.dyn_array(self.p_texel_buffer_view, self.descriptor_count as usize);
            for x in s { e.u64(x.0); }
        }
    }

}

/// Unie `VkClearColorValue` (vstup): varianta = clen.
#[derive(Debug, Clone, Copy)]
pub enum VkClearColorValue {
    Float32([f32; 4]),
    Int32([i32; 4]),
    Uint32([u32; 4]),
}

impl Default for VkClearColorValue {
    fn default() -> Self {
        Self::Float32([0.0; 4])
    }
}

impl VkClearColorValue {
    pub fn encode(&self, e: &mut Encoder) {
        match self {
            Self::Float32(x) => {
                e.u32(0);
                e.array_size(4);
                e.f32_array(&x[..]);
            }
            Self::Int32(x) => {
                e.u32(1);
                e.array_size(4);
                e.i32_array(&x[..]);
            }
            Self::Uint32(x) => {
                e.u32(2);
                e.array_size(4);
                e.u32_array(&x[..]);
            }
        }
    }
}

