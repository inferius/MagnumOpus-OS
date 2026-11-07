// Vygenerovano `cargo xtask venus-gen` z libs/venus/xml (vk.xml a venus-protocol,
// licence v libs/venus/xml/README.md) a libs/venus/subset.txt. Neupravovat rucne.

use alloc::vec::Vec;

use super::super::*;
use crate::cs::{CsError, Decoder, Encoder, Result};

/// `vkCreateFence`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn create_fence(e: &mut Encoder, flags: VkCommandFlagsEXT, device: VkDevice, p_create_info: &VkFenceCreateInfo, p_fence: VkFence) {
    e.i32(VK_COMMAND_TYPE_vkCreateFence_EXT);
    e.u32(flags);
    e.u64(device.0);
    e.pointer(true);
    p_create_info.encode(e);
    e.pointer(false);
    e.pointer(true);
    e.u64(p_fence.0);
}

/// Odpoved na `vkCreateFence`.
#[derive(Debug, Clone, Default)]
pub struct CreateFenceReply {
    pub ret: VkResult,
    pub p_fence: Option<VkFence>,
}

/// Precte odpoved na `vkCreateFence` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_create_fence_reply(d: &mut Decoder) -> Result<CreateFenceReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkCreateFence_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = CreateFenceReply::default();
    r.ret = d.i32()?;
    r.p_fence = if d.pointer()? { Some(VkFence(d.u64()?)) } else { None };
    Ok(r)
}

/// `vkDestroyFence`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn destroy_fence(e: &mut Encoder, flags: VkCommandFlagsEXT, device: VkDevice, fence: VkFence) {
    e.i32(VK_COMMAND_TYPE_vkDestroyFence_EXT);
    e.u32(flags);
    e.u64(device.0);
    e.u64(fence.0);
    e.pointer(false);
}

/// Odpoved na `vkDestroyFence`.
#[derive(Debug, Clone, Default)]
pub struct DestroyFenceReply {
}

/// Precte odpoved na `vkDestroyFence` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_destroy_fence_reply(d: &mut Decoder) -> Result<DestroyFenceReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkDestroyFence_EXT {
        return Err(CsError::Unexpected);
    }
    let r = DestroyFenceReply::default();
    Ok(r)
}

/// `vkResetFences`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn reset_fences(e: &mut Encoder, flags: VkCommandFlagsEXT, device: VkDevice, fence_count: u32, p_fences: &[VkFence]) {
    e.i32(VK_COMMAND_TYPE_vkResetFences_EXT);
    e.u32(flags);
    e.u64(device.0);
    e.u32(fence_count);
    {
        let s = e.dyn_array(p_fences, fence_count as usize);
        for x in s { e.u64(x.0); }
    }
}

/// Odpoved na `vkResetFences`.
#[derive(Debug, Clone, Default)]
pub struct ResetFencesReply {
    pub ret: VkResult,
}

/// Precte odpoved na `vkResetFences` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_reset_fences_reply(d: &mut Decoder) -> Result<ResetFencesReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkResetFences_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = ResetFencesReply::default();
    r.ret = d.i32()?;
    Ok(r)
}

/// `vkGetFenceStatus`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn get_fence_status(e: &mut Encoder, flags: VkCommandFlagsEXT, device: VkDevice, fence: VkFence) {
    e.i32(VK_COMMAND_TYPE_vkGetFenceStatus_EXT);
    e.u32(flags);
    e.u64(device.0);
    e.u64(fence.0);
}

/// Odpoved na `vkGetFenceStatus`.
#[derive(Debug, Clone, Default)]
pub struct GetFenceStatusReply {
    pub ret: VkResult,
}

/// Precte odpoved na `vkGetFenceStatus` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_get_fence_status_reply(d: &mut Decoder) -> Result<GetFenceStatusReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkGetFenceStatus_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = GetFenceStatusReply::default();
    r.ret = d.i32()?;
    Ok(r)
}

/// `vkWaitForFences`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn wait_for_fences(e: &mut Encoder, flags: VkCommandFlagsEXT, device: VkDevice, fence_count: u32, p_fences: &[VkFence], wait_all: VkBool32, timeout: u64) {
    e.i32(VK_COMMAND_TYPE_vkWaitForFences_EXT);
    e.u32(flags);
    e.u64(device.0);
    e.u32(fence_count);
    {
        let s = e.dyn_array(p_fences, fence_count as usize);
        for x in s { e.u64(x.0); }
    }
    e.u32(wait_all);
    e.u64(timeout);
}

/// Odpoved na `vkWaitForFences`.
#[derive(Debug, Clone, Default)]
pub struct WaitForFencesReply {
    pub ret: VkResult,
}

/// Precte odpoved na `vkWaitForFences` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_wait_for_fences_reply(d: &mut Decoder) -> Result<WaitForFencesReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkWaitForFences_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = WaitForFencesReply::default();
    r.ret = d.i32()?;
    Ok(r)
}

