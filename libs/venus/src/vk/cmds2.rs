// Vygenerovano `cargo xtask venus-gen` z libs/venus/xml (vk.xml a venus-protocol,
// licence v libs/venus/xml/README.md) a libs/venus/subset.txt. Neupravovat rucne.

use alloc::vec::Vec;

use super::super::*;
use crate::cs::{CsError, Decoder, Encoder, Result};

/// `vkCreateFence`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn create_fence(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, p_create_info: &VkFenceCreateInfo, p_fence: VkFence) {
    e.i32(VK_COMMAND_TYPE_vkCreateFence_EXT);
    e.u32(cmd_flags);
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
pub fn destroy_fence(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, fence: VkFence) {
    e.i32(VK_COMMAND_TYPE_vkDestroyFence_EXT);
    e.u32(cmd_flags);
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
pub fn reset_fences(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, fence_count: u32, p_fences: &[VkFence]) {
    e.i32(VK_COMMAND_TYPE_vkResetFences_EXT);
    e.u32(cmd_flags);
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
pub fn get_fence_status(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, fence: VkFence) {
    e.i32(VK_COMMAND_TYPE_vkGetFenceStatus_EXT);
    e.u32(cmd_flags);
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
pub fn wait_for_fences(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, fence_count: u32, p_fences: &[VkFence], wait_all: VkBool32, timeout: u64) {
    e.i32(VK_COMMAND_TYPE_vkWaitForFences_EXT);
    e.u32(cmd_flags);
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

/// `vkCreateShaderModule`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn create_shader_module(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, p_create_info: &VkShaderModuleCreateInfo<'_>, p_shader_module: VkShaderModule) {
    e.i32(VK_COMMAND_TYPE_vkCreateShaderModule_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.pointer(true);
    p_create_info.encode(e);
    e.pointer(false);
    e.pointer(true);
    e.u64(p_shader_module.0);
}

/// Odpoved na `vkCreateShaderModule`.
#[derive(Debug, Clone, Default)]
pub struct CreateShaderModuleReply {
    pub ret: VkResult,
    pub p_shader_module: Option<VkShaderModule>,
}

/// Precte odpoved na `vkCreateShaderModule` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_create_shader_module_reply(d: &mut Decoder) -> Result<CreateShaderModuleReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkCreateShaderModule_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = CreateShaderModuleReply::default();
    r.ret = d.i32()?;
    r.p_shader_module = if d.pointer()? { Some(VkShaderModule(d.u64()?)) } else { None };
    Ok(r)
}

/// `vkDestroyShaderModule`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn destroy_shader_module(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, shader_module: VkShaderModule) {
    e.i32(VK_COMMAND_TYPE_vkDestroyShaderModule_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.u64(shader_module.0);
    e.pointer(false);
}

/// Odpoved na `vkDestroyShaderModule`.
#[derive(Debug, Clone, Default)]
pub struct DestroyShaderModuleReply {
}

/// Precte odpoved na `vkDestroyShaderModule` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_destroy_shader_module_reply(d: &mut Decoder) -> Result<DestroyShaderModuleReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkDestroyShaderModule_EXT {
        return Err(CsError::Unexpected);
    }
    let r = DestroyShaderModuleReply::default();
    Ok(r)
}

/// `vkCreateDescriptorSetLayout`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn create_descriptor_set_layout(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, p_create_info: &VkDescriptorSetLayoutCreateInfo<'_>, p_set_layout: VkDescriptorSetLayout) {
    e.i32(VK_COMMAND_TYPE_vkCreateDescriptorSetLayout_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.pointer(true);
    p_create_info.encode(e);
    e.pointer(false);
    e.pointer(true);
    e.u64(p_set_layout.0);
}

/// Odpoved na `vkCreateDescriptorSetLayout`.
#[derive(Debug, Clone, Default)]
pub struct CreateDescriptorSetLayoutReply {
    pub ret: VkResult,
    pub p_set_layout: Option<VkDescriptorSetLayout>,
}

/// Precte odpoved na `vkCreateDescriptorSetLayout` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_create_descriptor_set_layout_reply(d: &mut Decoder) -> Result<CreateDescriptorSetLayoutReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkCreateDescriptorSetLayout_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = CreateDescriptorSetLayoutReply::default();
    r.ret = d.i32()?;
    r.p_set_layout = if d.pointer()? { Some(VkDescriptorSetLayout(d.u64()?)) } else { None };
    Ok(r)
}

/// `vkDestroyDescriptorSetLayout`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn destroy_descriptor_set_layout(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, descriptor_set_layout: VkDescriptorSetLayout) {
    e.i32(VK_COMMAND_TYPE_vkDestroyDescriptorSetLayout_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.u64(descriptor_set_layout.0);
    e.pointer(false);
}

/// Odpoved na `vkDestroyDescriptorSetLayout`.
#[derive(Debug, Clone, Default)]
pub struct DestroyDescriptorSetLayoutReply {
}

/// Precte odpoved na `vkDestroyDescriptorSetLayout` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_destroy_descriptor_set_layout_reply(d: &mut Decoder) -> Result<DestroyDescriptorSetLayoutReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkDestroyDescriptorSetLayout_EXT {
        return Err(CsError::Unexpected);
    }
    let r = DestroyDescriptorSetLayoutReply::default();
    Ok(r)
}

/// `vkCreatePipelineLayout`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn create_pipeline_layout(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, p_create_info: &VkPipelineLayoutCreateInfo<'_>, p_pipeline_layout: VkPipelineLayout) {
    e.i32(VK_COMMAND_TYPE_vkCreatePipelineLayout_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.pointer(true);
    p_create_info.encode(e);
    e.pointer(false);
    e.pointer(true);
    e.u64(p_pipeline_layout.0);
}

/// Odpoved na `vkCreatePipelineLayout`.
#[derive(Debug, Clone, Default)]
pub struct CreatePipelineLayoutReply {
    pub ret: VkResult,
    pub p_pipeline_layout: Option<VkPipelineLayout>,
}

/// Precte odpoved na `vkCreatePipelineLayout` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_create_pipeline_layout_reply(d: &mut Decoder) -> Result<CreatePipelineLayoutReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkCreatePipelineLayout_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = CreatePipelineLayoutReply::default();
    r.ret = d.i32()?;
    r.p_pipeline_layout = if d.pointer()? { Some(VkPipelineLayout(d.u64()?)) } else { None };
    Ok(r)
}

/// `vkDestroyPipelineLayout`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn destroy_pipeline_layout(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, pipeline_layout: VkPipelineLayout) {
    e.i32(VK_COMMAND_TYPE_vkDestroyPipelineLayout_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.u64(pipeline_layout.0);
    e.pointer(false);
}

/// Odpoved na `vkDestroyPipelineLayout`.
#[derive(Debug, Clone, Default)]
pub struct DestroyPipelineLayoutReply {
}

/// Precte odpoved na `vkDestroyPipelineLayout` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_destroy_pipeline_layout_reply(d: &mut Decoder) -> Result<DestroyPipelineLayoutReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkDestroyPipelineLayout_EXT {
        return Err(CsError::Unexpected);
    }
    let r = DestroyPipelineLayoutReply::default();
    Ok(r)
}

/// `vkCreateComputePipelines`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn create_compute_pipelines(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, pipeline_cache: VkPipelineCache, create_info_count: u32, p_create_infos: &[VkComputePipelineCreateInfo<'_>], p_pipelines: Option<&[VkPipeline]>) {
    e.i32(VK_COMMAND_TYPE_vkCreateComputePipelines_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.u64(pipeline_cache.0);
    e.u32(create_info_count);
    {
        let s = e.dyn_array(p_create_infos, create_info_count as usize);
        for x in s { x.encode(e); }
    }
    e.pointer(false);
    match p_pipelines {
        Some(s) => {
            let s = e.dyn_array(s, create_info_count as usize);
            for x in s { e.u64(x.0); }
        }
        None => e.array_size(0),
    }
}

/// Odpoved na `vkCreateComputePipelines`.
#[derive(Debug, Clone, Default)]
pub struct CreateComputePipelinesReply {
    pub ret: VkResult,
    pub p_pipelines: Option<Vec<VkPipeline>>,
}

/// Precte odpoved na `vkCreateComputePipelines` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_create_compute_pipelines_reply(d: &mut Decoder) -> Result<CreateComputePipelinesReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkCreateComputePipelines_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = CreateComputePipelinesReply::default();
    r.ret = d.i32()?;
    r.p_pipelines = {
        let n = d.array_size(8)?;
        if n == 0 {
            None
        } else {
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                v.push(VkPipeline(d.u64()?));
            }
            Some(v)
        }
    };
    Ok(r)
}

/// `vkDestroyPipeline`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn destroy_pipeline(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, pipeline: VkPipeline) {
    e.i32(VK_COMMAND_TYPE_vkDestroyPipeline_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.u64(pipeline.0);
    e.pointer(false);
}

/// Odpoved na `vkDestroyPipeline`.
#[derive(Debug, Clone, Default)]
pub struct DestroyPipelineReply {
}

/// Precte odpoved na `vkDestroyPipeline` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_destroy_pipeline_reply(d: &mut Decoder) -> Result<DestroyPipelineReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkDestroyPipeline_EXT {
        return Err(CsError::Unexpected);
    }
    let r = DestroyPipelineReply::default();
    Ok(r)
}

/// `vkCreateDescriptorPool`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn create_descriptor_pool(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, p_create_info: &VkDescriptorPoolCreateInfo<'_>, p_descriptor_pool: VkDescriptorPool) {
    e.i32(VK_COMMAND_TYPE_vkCreateDescriptorPool_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.pointer(true);
    p_create_info.encode(e);
    e.pointer(false);
    e.pointer(true);
    e.u64(p_descriptor_pool.0);
}

/// Odpoved na `vkCreateDescriptorPool`.
#[derive(Debug, Clone, Default)]
pub struct CreateDescriptorPoolReply {
    pub ret: VkResult,
    pub p_descriptor_pool: Option<VkDescriptorPool>,
}

/// Precte odpoved na `vkCreateDescriptorPool` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_create_descriptor_pool_reply(d: &mut Decoder) -> Result<CreateDescriptorPoolReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkCreateDescriptorPool_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = CreateDescriptorPoolReply::default();
    r.ret = d.i32()?;
    r.p_descriptor_pool = if d.pointer()? { Some(VkDescriptorPool(d.u64()?)) } else { None };
    Ok(r)
}

/// `vkDestroyDescriptorPool`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn destroy_descriptor_pool(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, descriptor_pool: VkDescriptorPool) {
    e.i32(VK_COMMAND_TYPE_vkDestroyDescriptorPool_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.u64(descriptor_pool.0);
    e.pointer(false);
}

/// Odpoved na `vkDestroyDescriptorPool`.
#[derive(Debug, Clone, Default)]
pub struct DestroyDescriptorPoolReply {
}

/// Precte odpoved na `vkDestroyDescriptorPool` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_destroy_descriptor_pool_reply(d: &mut Decoder) -> Result<DestroyDescriptorPoolReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkDestroyDescriptorPool_EXT {
        return Err(CsError::Unexpected);
    }
    let r = DestroyDescriptorPoolReply::default();
    Ok(r)
}

/// `vkAllocateDescriptorSets`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn allocate_descriptor_sets(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, p_allocate_info: &VkDescriptorSetAllocateInfo<'_>, p_descriptor_sets: Option<&[VkDescriptorSet]>) {
    e.i32(VK_COMMAND_TYPE_vkAllocateDescriptorSets_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.pointer(true);
    p_allocate_info.encode(e);
    match p_descriptor_sets {
        Some(s) => {
            let s = e.dyn_array(s, p_allocate_info.descriptor_set_count as usize);
            for x in s { e.u64(x.0); }
        }
        None => e.array_size(0),
    }
}

/// Odpoved na `vkAllocateDescriptorSets`.
#[derive(Debug, Clone, Default)]
pub struct AllocateDescriptorSetsReply {
    pub ret: VkResult,
    pub p_descriptor_sets: Option<Vec<VkDescriptorSet>>,
}

/// Precte odpoved na `vkAllocateDescriptorSets` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_allocate_descriptor_sets_reply(d: &mut Decoder) -> Result<AllocateDescriptorSetsReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkAllocateDescriptorSets_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = AllocateDescriptorSetsReply::default();
    r.ret = d.i32()?;
    r.p_descriptor_sets = {
        let n = d.array_size(8)?;
        if n == 0 {
            None
        } else {
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                v.push(VkDescriptorSet(d.u64()?));
            }
            Some(v)
        }
    };
    Ok(r)
}

/// `vkUpdateDescriptorSets`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn update_descriptor_sets(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, descriptor_write_count: u32, p_descriptor_writes: &[VkWriteDescriptorSet<'_>], descriptor_copy_count: u32, p_descriptor_copies: &[VkCopyDescriptorSet]) {
    e.i32(VK_COMMAND_TYPE_vkUpdateDescriptorSets_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.u32(descriptor_write_count);
    {
        let s = e.dyn_array(p_descriptor_writes, descriptor_write_count as usize);
        for x in s { x.encode(e); }
    }
    e.u32(descriptor_copy_count);
    {
        let s = e.dyn_array(p_descriptor_copies, descriptor_copy_count as usize);
        for x in s { x.encode(e); }
    }
}

/// Odpoved na `vkUpdateDescriptorSets`.
#[derive(Debug, Clone, Default)]
pub struct UpdateDescriptorSetsReply {
}

/// Precte odpoved na `vkUpdateDescriptorSets` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_update_descriptor_sets_reply(d: &mut Decoder) -> Result<UpdateDescriptorSetsReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkUpdateDescriptorSets_EXT {
        return Err(CsError::Unexpected);
    }
    let r = UpdateDescriptorSetsReply::default();
    Ok(r)
}

/// `vkCmdBindPipeline`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn cmd_bind_pipeline(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, command_buffer: VkCommandBuffer, pipeline_bind_point: VkPipelineBindPoint, pipeline: VkPipeline) {
    e.i32(VK_COMMAND_TYPE_vkCmdBindPipeline_EXT);
    e.u32(cmd_flags);
    e.u64(command_buffer.0);
    e.i32(pipeline_bind_point);
    e.u64(pipeline.0);
}

/// Odpoved na `vkCmdBindPipeline`.
#[derive(Debug, Clone, Default)]
pub struct CmdBindPipelineReply {
}

/// Precte odpoved na `vkCmdBindPipeline` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_cmd_bind_pipeline_reply(d: &mut Decoder) -> Result<CmdBindPipelineReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkCmdBindPipeline_EXT {
        return Err(CsError::Unexpected);
    }
    let r = CmdBindPipelineReply::default();
    Ok(r)
}

/// `vkCmdBindDescriptorSets`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn cmd_bind_descriptor_sets(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, command_buffer: VkCommandBuffer, pipeline_bind_point: VkPipelineBindPoint, layout: VkPipelineLayout, first_set: u32, descriptor_set_count: u32, p_descriptor_sets: &[VkDescriptorSet], dynamic_offset_count: u32, p_dynamic_offsets: &[u32]) {
    e.i32(VK_COMMAND_TYPE_vkCmdBindDescriptorSets_EXT);
    e.u32(cmd_flags);
    e.u64(command_buffer.0);
    e.i32(pipeline_bind_point);
    e.u64(layout.0);
    e.u32(first_set);
    e.u32(descriptor_set_count);
    {
        let s = e.dyn_array(p_descriptor_sets, descriptor_set_count as usize);
        for x in s { e.u64(x.0); }
    }
    e.u32(dynamic_offset_count);
    {
        let s = e.dyn_array(p_dynamic_offsets, dynamic_offset_count as usize);
        e.u32_array(s);
    }
}

/// Odpoved na `vkCmdBindDescriptorSets`.
#[derive(Debug, Clone, Default)]
pub struct CmdBindDescriptorSetsReply {
}

/// Precte odpoved na `vkCmdBindDescriptorSets` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_cmd_bind_descriptor_sets_reply(d: &mut Decoder) -> Result<CmdBindDescriptorSetsReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkCmdBindDescriptorSets_EXT {
        return Err(CsError::Unexpected);
    }
    let r = CmdBindDescriptorSetsReply::default();
    Ok(r)
}

/// `vkCmdPushConstants`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn cmd_push_constants(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, command_buffer: VkCommandBuffer, layout: VkPipelineLayout, stage_flags: VkShaderStageFlags, offset: u32, size: u32, p_values: &[u8]) {
    e.i32(VK_COMMAND_TYPE_vkCmdPushConstants_EXT);
    e.u32(cmd_flags);
    e.u64(command_buffer.0);
    e.u64(layout.0);
    e.u32(stage_flags);
    e.u32(offset);
    e.u32(size);
    {
        let s = e.dyn_array(p_values, size as usize);
        e.u8_array(s);
    }
}

/// Odpoved na `vkCmdPushConstants`.
#[derive(Debug, Clone, Default)]
pub struct CmdPushConstantsReply {
}

/// Precte odpoved na `vkCmdPushConstants` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_cmd_push_constants_reply(d: &mut Decoder) -> Result<CmdPushConstantsReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkCmdPushConstants_EXT {
        return Err(CsError::Unexpected);
    }
    let r = CmdPushConstantsReply::default();
    Ok(r)
}

/// `vkCmdDispatch`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn cmd_dispatch(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, command_buffer: VkCommandBuffer, group_count_x: u32, group_count_y: u32, group_count_z: u32) {
    e.i32(VK_COMMAND_TYPE_vkCmdDispatch_EXT);
    e.u32(cmd_flags);
    e.u64(command_buffer.0);
    e.u32(group_count_x);
    e.u32(group_count_y);
    e.u32(group_count_z);
}

/// Odpoved na `vkCmdDispatch`.
#[derive(Debug, Clone, Default)]
pub struct CmdDispatchReply {
}

/// Precte odpoved na `vkCmdDispatch` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_cmd_dispatch_reply(d: &mut Decoder) -> Result<CmdDispatchReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkCmdDispatch_EXT {
        return Err(CsError::Unexpected);
    }
    let r = CmdDispatchReply::default();
    Ok(r)
}

/// `vkCmdCopyBuffer`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn cmd_copy_buffer(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, command_buffer: VkCommandBuffer, src_buffer: VkBuffer, dst_buffer: VkBuffer, region_count: u32, p_regions: &[VkBufferCopy]) {
    e.i32(VK_COMMAND_TYPE_vkCmdCopyBuffer_EXT);
    e.u32(cmd_flags);
    e.u64(command_buffer.0);
    e.u64(src_buffer.0);
    e.u64(dst_buffer.0);
    e.u32(region_count);
    {
        let s = e.dyn_array(p_regions, region_count as usize);
        for x in s { x.encode(e); }
    }
}

/// Odpoved na `vkCmdCopyBuffer`.
#[derive(Debug, Clone, Default)]
pub struct CmdCopyBufferReply {
}

/// Precte odpoved na `vkCmdCopyBuffer` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_cmd_copy_buffer_reply(d: &mut Decoder) -> Result<CmdCopyBufferReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkCmdCopyBuffer_EXT {
        return Err(CsError::Unexpected);
    }
    let r = CmdCopyBufferReply::default();
    Ok(r)
}

/// `vkResetCommandBuffer`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn reset_command_buffer(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, command_buffer: VkCommandBuffer, flags: VkCommandBufferResetFlags) {
    e.i32(VK_COMMAND_TYPE_vkResetCommandBuffer_EXT);
    e.u32(cmd_flags);
    e.u64(command_buffer.0);
    e.u32(flags);
}

/// Odpoved na `vkResetCommandBuffer`.
#[derive(Debug, Clone, Default)]
pub struct ResetCommandBufferReply {
    pub ret: VkResult,
}

/// Precte odpoved na `vkResetCommandBuffer` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_reset_command_buffer_reply(d: &mut Decoder) -> Result<ResetCommandBufferReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkResetCommandBuffer_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = ResetCommandBufferReply::default();
    r.ret = d.i32()?;
    Ok(r)
}

/// `vkFreeDescriptorSets`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn free_descriptor_sets(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, descriptor_pool: VkDescriptorPool, descriptor_set_count: u32, p_descriptor_sets: &[VkDescriptorSet]) {
    e.i32(VK_COMMAND_TYPE_vkFreeDescriptorSets_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.u64(descriptor_pool.0);
    e.u32(descriptor_set_count);
    {
        let s = e.dyn_array(p_descriptor_sets, descriptor_set_count as usize);
        for x in s { e.u64(x.0); }
    }
}

/// Odpoved na `vkFreeDescriptorSets`.
#[derive(Debug, Clone, Default)]
pub struct FreeDescriptorSetsReply {
    pub ret: VkResult,
}

/// Precte odpoved na `vkFreeDescriptorSets` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_free_descriptor_sets_reply(d: &mut Decoder) -> Result<FreeDescriptorSetsReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkFreeDescriptorSets_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = FreeDescriptorSetsReply::default();
    r.ret = d.i32()?;
    Ok(r)
}

