// Vygenerovano `cargo xtask venus-gen` z libs/venus/xml (vk.xml a venus-protocol,
// licence v libs/venus/xml/README.md) a libs/venus/subset.txt. Neupravovat rucne.

use alloc::vec::Vec;

use super::*;
use crate::cs::{CsError, Decoder, Encoder, Result};

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

