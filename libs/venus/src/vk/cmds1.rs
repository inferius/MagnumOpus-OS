// Vygenerovano `cargo xtask venus-gen` z libs/venus/xml (vk.xml a venus-protocol,
// licence v libs/venus/xml/README.md) a libs/venus/subset.txt. Neupravovat rucne.

use alloc::vec::Vec;

use super::super::*;
use crate::cs::{CsError, Decoder, Encoder, Result};

/// `vkSetReplyCommandStreamMESA`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn set_reply_command_stream_mesa(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, p_stream: &VkCommandStreamDescriptionMESA) {
    e.i32(VK_COMMAND_TYPE_vkSetReplyCommandStreamMESA_EXT);
    e.u32(cmd_flags);
    e.pointer(true);
    p_stream.encode(e);
}

/// Odpoved na `vkSetReplyCommandStreamMESA`.
#[derive(Debug, Clone, Default)]
pub struct SetReplyCommandStreamMESAReply {
}

/// Precte odpoved na `vkSetReplyCommandStreamMESA` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_set_reply_command_stream_mesa_reply(d: &mut Decoder) -> Result<SetReplyCommandStreamMESAReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkSetReplyCommandStreamMESA_EXT {
        return Err(CsError::Unexpected);
    }
    let r = SetReplyCommandStreamMESAReply::default();
    Ok(r)
}

/// `vkSeekReplyCommandStreamMESA`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn seek_reply_command_stream_mesa(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, position: u64) {
    e.i32(VK_COMMAND_TYPE_vkSeekReplyCommandStreamMESA_EXT);
    e.u32(cmd_flags);
    e.u64(position);
}

/// Odpoved na `vkSeekReplyCommandStreamMESA`.
#[derive(Debug, Clone, Default)]
pub struct SeekReplyCommandStreamMESAReply {
}

/// Precte odpoved na `vkSeekReplyCommandStreamMESA` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_seek_reply_command_stream_mesa_reply(d: &mut Decoder) -> Result<SeekReplyCommandStreamMESAReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkSeekReplyCommandStreamMESA_EXT {
        return Err(CsError::Unexpected);
    }
    let r = SeekReplyCommandStreamMESAReply::default();
    Ok(r)
}

/// `vkEnumerateInstanceVersion`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn enumerate_instance_version(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT) {
    e.i32(VK_COMMAND_TYPE_vkEnumerateInstanceVersion_EXT);
    e.u32(cmd_flags);
    e.pointer(true);
}

/// Odpoved na `vkEnumerateInstanceVersion`.
#[derive(Debug, Clone, Default)]
pub struct EnumerateInstanceVersionReply {
    pub ret: VkResult,
    pub p_api_version: Option<u32>,
}

/// Precte odpoved na `vkEnumerateInstanceVersion` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_enumerate_instance_version_reply(d: &mut Decoder) -> Result<EnumerateInstanceVersionReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkEnumerateInstanceVersion_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = EnumerateInstanceVersionReply::default();
    r.ret = d.i32()?;
    r.p_api_version = if d.pointer()? { Some(d.u32()?) } else { None };
    Ok(r)
}

/// `vkCreateInstance`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn create_instance(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, p_create_info: &VkInstanceCreateInfo<'_>, p_instance: VkInstance) {
    e.i32(VK_COMMAND_TYPE_vkCreateInstance_EXT);
    e.u32(cmd_flags);
    e.pointer(true);
    p_create_info.encode(e);
    e.pointer(false);
    e.pointer(true);
    e.u64(p_instance.0);
}

/// Odpoved na `vkCreateInstance`.
#[derive(Debug, Clone, Default)]
pub struct CreateInstanceReply {
    pub ret: VkResult,
    pub p_instance: Option<VkInstance>,
}

/// Precte odpoved na `vkCreateInstance` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_create_instance_reply(d: &mut Decoder) -> Result<CreateInstanceReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkCreateInstance_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = CreateInstanceReply::default();
    r.ret = d.i32()?;
    r.p_instance = if d.pointer()? { Some(VkInstance(d.u64()?)) } else { None };
    Ok(r)
}

/// `vkDestroyInstance`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn destroy_instance(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, instance: VkInstance) {
    e.i32(VK_COMMAND_TYPE_vkDestroyInstance_EXT);
    e.u32(cmd_flags);
    e.u64(instance.0);
    e.pointer(false);
}

/// Odpoved na `vkDestroyInstance`.
#[derive(Debug, Clone, Default)]
pub struct DestroyInstanceReply {
}

/// Precte odpoved na `vkDestroyInstance` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_destroy_instance_reply(d: &mut Decoder) -> Result<DestroyInstanceReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkDestroyInstance_EXT {
        return Err(CsError::Unexpected);
    }
    let r = DestroyInstanceReply::default();
    Ok(r)
}

/// `vkEnumeratePhysicalDevices`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn enumerate_physical_devices(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, instance: VkInstance, p_physical_device_count: Option<u32>, p_physical_devices: Option<&[VkPhysicalDevice]>) {
    e.i32(VK_COMMAND_TYPE_vkEnumeratePhysicalDevices_EXT);
    e.u32(cmd_flags);
    e.u64(instance.0);
    match p_physical_device_count {
        Some(x) => {
            e.pointer(true);
            e.u32(x);
        }
        None => e.pointer(false),
    }
    match p_physical_devices {
        Some(s) => {
            let s = e.dyn_array(s, p_physical_device_count.unwrap_or(0) as usize);
            for x in s { e.u64(x.0); }
        }
        None => e.array_size(0),
    }
}

/// Odpoved na `vkEnumeratePhysicalDevices`.
#[derive(Debug, Clone, Default)]
pub struct EnumeratePhysicalDevicesReply {
    pub ret: VkResult,
    pub p_physical_device_count: Option<u32>,
    pub p_physical_devices: Option<Vec<VkPhysicalDevice>>,
}

/// Precte odpoved na `vkEnumeratePhysicalDevices` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_enumerate_physical_devices_reply(d: &mut Decoder) -> Result<EnumeratePhysicalDevicesReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkEnumeratePhysicalDevices_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = EnumeratePhysicalDevicesReply::default();
    r.ret = d.i32()?;
    r.p_physical_device_count = if d.pointer()? { Some(d.u32()?) } else { None };
    r.p_physical_devices = {
        let n = d.array_size(8)?;
        if n == 0 {
            None
        } else {
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                v.push(VkPhysicalDevice(d.u64()?));
            }
            Some(v)
        }
    };
    Ok(r)
}

/// `vkGetPhysicalDeviceProperties`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn get_physical_device_properties(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, physical_device: VkPhysicalDevice) {
    e.i32(VK_COMMAND_TYPE_vkGetPhysicalDeviceProperties_EXT);
    e.u32(cmd_flags);
    e.u64(physical_device.0);
    e.pointer(true);
    VkPhysicalDeviceProperties::default().encode_partial(e);
}

/// Odpoved na `vkGetPhysicalDeviceProperties`.
#[derive(Debug, Clone, Default)]
pub struct GetPhysicalDevicePropertiesReply {
    pub p_properties: Option<VkPhysicalDeviceProperties>,
}

/// Precte odpoved na `vkGetPhysicalDeviceProperties` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_get_physical_device_properties_reply(d: &mut Decoder) -> Result<GetPhysicalDevicePropertiesReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkGetPhysicalDeviceProperties_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = GetPhysicalDevicePropertiesReply::default();
    r.p_properties = if d.pointer()? { Some(VkPhysicalDeviceProperties::decode(d)?) } else { None };
    Ok(r)
}

/// `vkGetPhysicalDeviceQueueFamilyProperties`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn get_physical_device_queue_family_properties(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, physical_device: VkPhysicalDevice, p_queue_family_property_count: Option<u32>, p_queue_family_properties: bool) {
    e.i32(VK_COMMAND_TYPE_vkGetPhysicalDeviceQueueFamilyProperties_EXT);
    e.u32(cmd_flags);
    e.u64(physical_device.0);
    match p_queue_family_property_count {
        Some(x) => {
            e.pointer(true);
            e.u32(x);
        }
        None => e.pointer(false),
    }
    {
        let n = if p_queue_family_properties { p_queue_family_property_count.unwrap_or(0) as usize } else { 0 };
        e.array_size(n);
        for _ in 0..n {
            VkQueueFamilyProperties::default().encode_partial(e);
        }
    }
}

/// Odpoved na `vkGetPhysicalDeviceQueueFamilyProperties`.
#[derive(Debug, Clone, Default)]
pub struct GetPhysicalDeviceQueueFamilyPropertiesReply {
    pub p_queue_family_property_count: Option<u32>,
    pub p_queue_family_properties: Option<Vec<VkQueueFamilyProperties>>,
}

/// Precte odpoved na `vkGetPhysicalDeviceQueueFamilyProperties` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_get_physical_device_queue_family_properties_reply(d: &mut Decoder) -> Result<GetPhysicalDeviceQueueFamilyPropertiesReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkGetPhysicalDeviceQueueFamilyProperties_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = GetPhysicalDeviceQueueFamilyPropertiesReply::default();
    r.p_queue_family_property_count = if d.pointer()? { Some(d.u32()?) } else { None };
    r.p_queue_family_properties = {
        let n = d.array_size(4)?;
        if n == 0 {
            None
        } else {
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                v.push(VkQueueFamilyProperties::decode(d)?);
            }
            Some(v)
        }
    };
    Ok(r)
}

/// `vkGetPhysicalDeviceMemoryProperties`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn get_physical_device_memory_properties(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, physical_device: VkPhysicalDevice) {
    e.i32(VK_COMMAND_TYPE_vkGetPhysicalDeviceMemoryProperties_EXT);
    e.u32(cmd_flags);
    e.u64(physical_device.0);
    e.pointer(true);
    VkPhysicalDeviceMemoryProperties::default().encode_partial(e);
}

/// Odpoved na `vkGetPhysicalDeviceMemoryProperties`.
#[derive(Debug, Clone, Default)]
pub struct GetPhysicalDeviceMemoryPropertiesReply {
    pub p_memory_properties: Option<VkPhysicalDeviceMemoryProperties>,
}

/// Precte odpoved na `vkGetPhysicalDeviceMemoryProperties` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_get_physical_device_memory_properties_reply(d: &mut Decoder) -> Result<GetPhysicalDeviceMemoryPropertiesReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkGetPhysicalDeviceMemoryProperties_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = GetPhysicalDeviceMemoryPropertiesReply::default();
    r.p_memory_properties = if d.pointer()? { Some(VkPhysicalDeviceMemoryProperties::decode(d)?) } else { None };
    Ok(r)
}

/// `vkGetPhysicalDeviceFormatProperties`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn get_physical_device_format_properties(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, physical_device: VkPhysicalDevice, format: VkFormat) {
    e.i32(VK_COMMAND_TYPE_vkGetPhysicalDeviceFormatProperties_EXT);
    e.u32(cmd_flags);
    e.u64(physical_device.0);
    e.i32(format);
    e.pointer(true);
    VkFormatProperties::default().encode_partial(e);
}

/// Odpoved na `vkGetPhysicalDeviceFormatProperties`.
#[derive(Debug, Clone, Default)]
pub struct GetPhysicalDeviceFormatPropertiesReply {
    pub p_format_properties: Option<VkFormatProperties>,
}

/// Precte odpoved na `vkGetPhysicalDeviceFormatProperties` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_get_physical_device_format_properties_reply(d: &mut Decoder) -> Result<GetPhysicalDeviceFormatPropertiesReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkGetPhysicalDeviceFormatProperties_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = GetPhysicalDeviceFormatPropertiesReply::default();
    r.p_format_properties = if d.pointer()? { Some(VkFormatProperties::decode(d)?) } else { None };
    Ok(r)
}

/// `vkCreateDevice`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn create_device(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, physical_device: VkPhysicalDevice, p_create_info: &VkDeviceCreateInfo<'_>, p_device: VkDevice) {
    e.i32(VK_COMMAND_TYPE_vkCreateDevice_EXT);
    e.u32(cmd_flags);
    e.u64(physical_device.0);
    e.pointer(true);
    p_create_info.encode(e);
    e.pointer(false);
    e.pointer(true);
    e.u64(p_device.0);
}

/// Odpoved na `vkCreateDevice`.
#[derive(Debug, Clone, Default)]
pub struct CreateDeviceReply {
    pub ret: VkResult,
    pub p_device: Option<VkDevice>,
}

/// Precte odpoved na `vkCreateDevice` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_create_device_reply(d: &mut Decoder) -> Result<CreateDeviceReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkCreateDevice_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = CreateDeviceReply::default();
    r.ret = d.i32()?;
    r.p_device = if d.pointer()? { Some(VkDevice(d.u64()?)) } else { None };
    Ok(r)
}

/// `vkDestroyDevice`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn destroy_device(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice) {
    e.i32(VK_COMMAND_TYPE_vkDestroyDevice_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.pointer(false);
}

/// Odpoved na `vkDestroyDevice`.
#[derive(Debug, Clone, Default)]
pub struct DestroyDeviceReply {
}

/// Precte odpoved na `vkDestroyDevice` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_destroy_device_reply(d: &mut Decoder) -> Result<DestroyDeviceReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkDestroyDevice_EXT {
        return Err(CsError::Unexpected);
    }
    let r = DestroyDeviceReply::default();
    Ok(r)
}

/// `vkGetDeviceQueue2`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn get_device_queue2(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, p_queue_info: &VkDeviceQueueInfo2<'_>, p_queue: VkQueue) {
    e.i32(VK_COMMAND_TYPE_vkGetDeviceQueue2_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.pointer(true);
    p_queue_info.encode(e);
    e.pointer(true);
    e.u64(p_queue.0);
}

/// Odpoved na `vkGetDeviceQueue2`.
#[derive(Debug, Clone, Default)]
pub struct GetDeviceQueue2Reply {
    pub p_queue: Option<VkQueue>,
}

/// Precte odpoved na `vkGetDeviceQueue2` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_get_device_queue2_reply(d: &mut Decoder) -> Result<GetDeviceQueue2Reply> {
    if d.i32()? != VK_COMMAND_TYPE_vkGetDeviceQueue2_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = GetDeviceQueue2Reply::default();
    r.p_queue = if d.pointer()? { Some(VkQueue(d.u64()?)) } else { None };
    Ok(r)
}

/// `vkDeviceWaitIdle`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn device_wait_idle(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice) {
    e.i32(VK_COMMAND_TYPE_vkDeviceWaitIdle_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
}

/// Odpoved na `vkDeviceWaitIdle`.
#[derive(Debug, Clone, Default)]
pub struct DeviceWaitIdleReply {
    pub ret: VkResult,
}

/// Precte odpoved na `vkDeviceWaitIdle` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_device_wait_idle_reply(d: &mut Decoder) -> Result<DeviceWaitIdleReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkDeviceWaitIdle_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = DeviceWaitIdleReply::default();
    r.ret = d.i32()?;
    Ok(r)
}

/// `vkAllocateMemory`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn allocate_memory(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, p_allocate_info: &VkMemoryAllocateInfo, p_memory: VkDeviceMemory) {
    e.i32(VK_COMMAND_TYPE_vkAllocateMemory_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.pointer(true);
    p_allocate_info.encode(e);
    e.pointer(false);
    e.pointer(true);
    e.u64(p_memory.0);
}

/// Odpoved na `vkAllocateMemory`.
#[derive(Debug, Clone, Default)]
pub struct AllocateMemoryReply {
    pub ret: VkResult,
    pub p_memory: Option<VkDeviceMemory>,
}

/// Precte odpoved na `vkAllocateMemory` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_allocate_memory_reply(d: &mut Decoder) -> Result<AllocateMemoryReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkAllocateMemory_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = AllocateMemoryReply::default();
    r.ret = d.i32()?;
    r.p_memory = if d.pointer()? { Some(VkDeviceMemory(d.u64()?)) } else { None };
    Ok(r)
}

/// `vkFreeMemory`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn free_memory(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, memory: VkDeviceMemory) {
    e.i32(VK_COMMAND_TYPE_vkFreeMemory_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.u64(memory.0);
    e.pointer(false);
}

/// Odpoved na `vkFreeMemory`.
#[derive(Debug, Clone, Default)]
pub struct FreeMemoryReply {
}

/// Precte odpoved na `vkFreeMemory` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_free_memory_reply(d: &mut Decoder) -> Result<FreeMemoryReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkFreeMemory_EXT {
        return Err(CsError::Unexpected);
    }
    let r = FreeMemoryReply::default();
    Ok(r)
}

/// `vkCreateBuffer`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn create_buffer(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, p_create_info: &VkBufferCreateInfo<'_>, p_buffer: VkBuffer) {
    e.i32(VK_COMMAND_TYPE_vkCreateBuffer_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.pointer(true);
    p_create_info.encode(e);
    e.pointer(false);
    e.pointer(true);
    e.u64(p_buffer.0);
}

/// Odpoved na `vkCreateBuffer`.
#[derive(Debug, Clone, Default)]
pub struct CreateBufferReply {
    pub ret: VkResult,
    pub p_buffer: Option<VkBuffer>,
}

/// Precte odpoved na `vkCreateBuffer` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_create_buffer_reply(d: &mut Decoder) -> Result<CreateBufferReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkCreateBuffer_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = CreateBufferReply::default();
    r.ret = d.i32()?;
    r.p_buffer = if d.pointer()? { Some(VkBuffer(d.u64()?)) } else { None };
    Ok(r)
}

/// `vkDestroyBuffer`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn destroy_buffer(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, buffer: VkBuffer) {
    e.i32(VK_COMMAND_TYPE_vkDestroyBuffer_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.u64(buffer.0);
    e.pointer(false);
}

/// Odpoved na `vkDestroyBuffer`.
#[derive(Debug, Clone, Default)]
pub struct DestroyBufferReply {
}

/// Precte odpoved na `vkDestroyBuffer` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_destroy_buffer_reply(d: &mut Decoder) -> Result<DestroyBufferReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkDestroyBuffer_EXT {
        return Err(CsError::Unexpected);
    }
    let r = DestroyBufferReply::default();
    Ok(r)
}

/// `vkGetBufferMemoryRequirements`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn get_buffer_memory_requirements(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, buffer: VkBuffer) {
    e.i32(VK_COMMAND_TYPE_vkGetBufferMemoryRequirements_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.u64(buffer.0);
    e.pointer(true);
    VkMemoryRequirements::default().encode_partial(e);
}

/// Odpoved na `vkGetBufferMemoryRequirements`.
#[derive(Debug, Clone, Default)]
pub struct GetBufferMemoryRequirementsReply {
    pub p_memory_requirements: Option<VkMemoryRequirements>,
}

/// Precte odpoved na `vkGetBufferMemoryRequirements` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_get_buffer_memory_requirements_reply(d: &mut Decoder) -> Result<GetBufferMemoryRequirementsReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkGetBufferMemoryRequirements_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = GetBufferMemoryRequirementsReply::default();
    r.p_memory_requirements = if d.pointer()? { Some(VkMemoryRequirements::decode(d)?) } else { None };
    Ok(r)
}

/// `vkBindBufferMemory`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn bind_buffer_memory(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, buffer: VkBuffer, memory: VkDeviceMemory, memory_offset: VkDeviceSize) {
    e.i32(VK_COMMAND_TYPE_vkBindBufferMemory_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.u64(buffer.0);
    e.u64(memory.0);
    e.u64(memory_offset);
}

/// Odpoved na `vkBindBufferMemory`.
#[derive(Debug, Clone, Default)]
pub struct BindBufferMemoryReply {
    pub ret: VkResult,
}

/// Precte odpoved na `vkBindBufferMemory` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_bind_buffer_memory_reply(d: &mut Decoder) -> Result<BindBufferMemoryReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkBindBufferMemory_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = BindBufferMemoryReply::default();
    r.ret = d.i32()?;
    Ok(r)
}

/// `vkCreateImage`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn create_image(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, p_create_info: &VkImageCreateInfo<'_>, p_image: VkImage) {
    e.i32(VK_COMMAND_TYPE_vkCreateImage_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.pointer(true);
    p_create_info.encode(e);
    e.pointer(false);
    e.pointer(true);
    e.u64(p_image.0);
}

/// Odpoved na `vkCreateImage`.
#[derive(Debug, Clone, Default)]
pub struct CreateImageReply {
    pub ret: VkResult,
    pub p_image: Option<VkImage>,
}

/// Precte odpoved na `vkCreateImage` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_create_image_reply(d: &mut Decoder) -> Result<CreateImageReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkCreateImage_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = CreateImageReply::default();
    r.ret = d.i32()?;
    r.p_image = if d.pointer()? { Some(VkImage(d.u64()?)) } else { None };
    Ok(r)
}

/// `vkDestroyImage`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn destroy_image(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, image: VkImage) {
    e.i32(VK_COMMAND_TYPE_vkDestroyImage_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.u64(image.0);
    e.pointer(false);
}

/// Odpoved na `vkDestroyImage`.
#[derive(Debug, Clone, Default)]
pub struct DestroyImageReply {
}

/// Precte odpoved na `vkDestroyImage` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_destroy_image_reply(d: &mut Decoder) -> Result<DestroyImageReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkDestroyImage_EXT {
        return Err(CsError::Unexpected);
    }
    let r = DestroyImageReply::default();
    Ok(r)
}

/// `vkGetImageMemoryRequirements`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn get_image_memory_requirements(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, image: VkImage) {
    e.i32(VK_COMMAND_TYPE_vkGetImageMemoryRequirements_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.u64(image.0);
    e.pointer(true);
    VkMemoryRequirements::default().encode_partial(e);
}

/// Odpoved na `vkGetImageMemoryRequirements`.
#[derive(Debug, Clone, Default)]
pub struct GetImageMemoryRequirementsReply {
    pub p_memory_requirements: Option<VkMemoryRequirements>,
}

/// Precte odpoved na `vkGetImageMemoryRequirements` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_get_image_memory_requirements_reply(d: &mut Decoder) -> Result<GetImageMemoryRequirementsReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkGetImageMemoryRequirements_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = GetImageMemoryRequirementsReply::default();
    r.p_memory_requirements = if d.pointer()? { Some(VkMemoryRequirements::decode(d)?) } else { None };
    Ok(r)
}

/// `vkBindImageMemory`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn bind_image_memory(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, image: VkImage, memory: VkDeviceMemory, memory_offset: VkDeviceSize) {
    e.i32(VK_COMMAND_TYPE_vkBindImageMemory_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.u64(image.0);
    e.u64(memory.0);
    e.u64(memory_offset);
}

/// Odpoved na `vkBindImageMemory`.
#[derive(Debug, Clone, Default)]
pub struct BindImageMemoryReply {
    pub ret: VkResult,
}

/// Precte odpoved na `vkBindImageMemory` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_bind_image_memory_reply(d: &mut Decoder) -> Result<BindImageMemoryReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkBindImageMemory_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = BindImageMemoryReply::default();
    r.ret = d.i32()?;
    Ok(r)
}

/// `vkCreateCommandPool`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn create_command_pool(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, p_create_info: &VkCommandPoolCreateInfo, p_command_pool: VkCommandPool) {
    e.i32(VK_COMMAND_TYPE_vkCreateCommandPool_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.pointer(true);
    p_create_info.encode(e);
    e.pointer(false);
    e.pointer(true);
    e.u64(p_command_pool.0);
}

/// Odpoved na `vkCreateCommandPool`.
#[derive(Debug, Clone, Default)]
pub struct CreateCommandPoolReply {
    pub ret: VkResult,
    pub p_command_pool: Option<VkCommandPool>,
}

/// Precte odpoved na `vkCreateCommandPool` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_create_command_pool_reply(d: &mut Decoder) -> Result<CreateCommandPoolReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkCreateCommandPool_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = CreateCommandPoolReply::default();
    r.ret = d.i32()?;
    r.p_command_pool = if d.pointer()? { Some(VkCommandPool(d.u64()?)) } else { None };
    Ok(r)
}

/// `vkDestroyCommandPool`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn destroy_command_pool(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, command_pool: VkCommandPool) {
    e.i32(VK_COMMAND_TYPE_vkDestroyCommandPool_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.u64(command_pool.0);
    e.pointer(false);
}

/// Odpoved na `vkDestroyCommandPool`.
#[derive(Debug, Clone, Default)]
pub struct DestroyCommandPoolReply {
}

/// Precte odpoved na `vkDestroyCommandPool` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_destroy_command_pool_reply(d: &mut Decoder) -> Result<DestroyCommandPoolReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkDestroyCommandPool_EXT {
        return Err(CsError::Unexpected);
    }
    let r = DestroyCommandPoolReply::default();
    Ok(r)
}

/// `vkAllocateCommandBuffers`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn allocate_command_buffers(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, p_allocate_info: &VkCommandBufferAllocateInfo, p_command_buffers: Option<&[VkCommandBuffer]>) {
    e.i32(VK_COMMAND_TYPE_vkAllocateCommandBuffers_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.pointer(true);
    p_allocate_info.encode(e);
    match p_command_buffers {
        Some(s) => {
            let s = e.dyn_array(s, p_allocate_info.command_buffer_count as usize);
            for x in s { e.u64(x.0); }
        }
        None => e.array_size(0),
    }
}

/// Odpoved na `vkAllocateCommandBuffers`.
#[derive(Debug, Clone, Default)]
pub struct AllocateCommandBuffersReply {
    pub ret: VkResult,
    pub p_command_buffers: Option<Vec<VkCommandBuffer>>,
}

/// Precte odpoved na `vkAllocateCommandBuffers` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_allocate_command_buffers_reply(d: &mut Decoder) -> Result<AllocateCommandBuffersReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkAllocateCommandBuffers_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = AllocateCommandBuffersReply::default();
    r.ret = d.i32()?;
    r.p_command_buffers = {
        let n = d.array_size(8)?;
        if n == 0 {
            None
        } else {
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                v.push(VkCommandBuffer(d.u64()?));
            }
            Some(v)
        }
    };
    Ok(r)
}

/// `vkFreeCommandBuffers`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn free_command_buffers(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, device: VkDevice, command_pool: VkCommandPool, command_buffer_count: u32, p_command_buffers: &[VkCommandBuffer]) {
    e.i32(VK_COMMAND_TYPE_vkFreeCommandBuffers_EXT);
    e.u32(cmd_flags);
    e.u64(device.0);
    e.u64(command_pool.0);
    e.u32(command_buffer_count);
    {
        let s = e.dyn_array(p_command_buffers, command_buffer_count as usize);
        for x in s { e.u64(x.0); }
    }
}

/// Odpoved na `vkFreeCommandBuffers`.
#[derive(Debug, Clone, Default)]
pub struct FreeCommandBuffersReply {
}

/// Precte odpoved na `vkFreeCommandBuffers` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_free_command_buffers_reply(d: &mut Decoder) -> Result<FreeCommandBuffersReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkFreeCommandBuffers_EXT {
        return Err(CsError::Unexpected);
    }
    let r = FreeCommandBuffersReply::default();
    Ok(r)
}

/// `vkBeginCommandBuffer`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn begin_command_buffer(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, command_buffer: VkCommandBuffer, p_begin_info: &VkCommandBufferBeginInfo<'_>) {
    e.i32(VK_COMMAND_TYPE_vkBeginCommandBuffer_EXT);
    e.u32(cmd_flags);
    e.u64(command_buffer.0);
    e.pointer(true);
    p_begin_info.encode(e);
}

/// Odpoved na `vkBeginCommandBuffer`.
#[derive(Debug, Clone, Default)]
pub struct BeginCommandBufferReply {
    pub ret: VkResult,
}

/// Precte odpoved na `vkBeginCommandBuffer` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_begin_command_buffer_reply(d: &mut Decoder) -> Result<BeginCommandBufferReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkBeginCommandBuffer_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = BeginCommandBufferReply::default();
    r.ret = d.i32()?;
    Ok(r)
}

/// `vkEndCommandBuffer`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn end_command_buffer(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, command_buffer: VkCommandBuffer) {
    e.i32(VK_COMMAND_TYPE_vkEndCommandBuffer_EXT);
    e.u32(cmd_flags);
    e.u64(command_buffer.0);
}

/// Odpoved na `vkEndCommandBuffer`.
#[derive(Debug, Clone, Default)]
pub struct EndCommandBufferReply {
    pub ret: VkResult,
}

/// Precte odpoved na `vkEndCommandBuffer` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_end_command_buffer_reply(d: &mut Decoder) -> Result<EndCommandBufferReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkEndCommandBuffer_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = EndCommandBufferReply::default();
    r.ret = d.i32()?;
    Ok(r)
}

/// `vkCmdPipelineBarrier`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn cmd_pipeline_barrier(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, command_buffer: VkCommandBuffer, src_stage_mask: VkPipelineStageFlags, dst_stage_mask: VkPipelineStageFlags, dependency_flags: VkDependencyFlags, memory_barrier_count: u32, p_memory_barriers: &[VkMemoryBarrier], buffer_memory_barrier_count: u32, p_buffer_memory_barriers: &[VkBufferMemoryBarrier], image_memory_barrier_count: u32, p_image_memory_barriers: &[VkImageMemoryBarrier]) {
    e.i32(VK_COMMAND_TYPE_vkCmdPipelineBarrier_EXT);
    e.u32(cmd_flags);
    e.u64(command_buffer.0);
    e.u32(src_stage_mask);
    e.u32(dst_stage_mask);
    e.u32(dependency_flags);
    e.u32(memory_barrier_count);
    {
        let s = e.dyn_array(p_memory_barriers, memory_barrier_count as usize);
        for x in s { x.encode(e); }
    }
    e.u32(buffer_memory_barrier_count);
    {
        let s = e.dyn_array(p_buffer_memory_barriers, buffer_memory_barrier_count as usize);
        for x in s { x.encode(e); }
    }
    e.u32(image_memory_barrier_count);
    {
        let s = e.dyn_array(p_image_memory_barriers, image_memory_barrier_count as usize);
        for x in s { x.encode(e); }
    }
}

/// Odpoved na `vkCmdPipelineBarrier`.
#[derive(Debug, Clone, Default)]
pub struct CmdPipelineBarrierReply {
}

/// Precte odpoved na `vkCmdPipelineBarrier` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_cmd_pipeline_barrier_reply(d: &mut Decoder) -> Result<CmdPipelineBarrierReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkCmdPipelineBarrier_EXT {
        return Err(CsError::Unexpected);
    }
    let r = CmdPipelineBarrierReply::default();
    Ok(r)
}

/// `vkCmdClearColorImage`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn cmd_clear_color_image(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, command_buffer: VkCommandBuffer, image: VkImage, image_layout: VkImageLayout, p_color: &VkClearColorValue, range_count: u32, p_ranges: &[VkImageSubresourceRange]) {
    e.i32(VK_COMMAND_TYPE_vkCmdClearColorImage_EXT);
    e.u32(cmd_flags);
    e.u64(command_buffer.0);
    e.u64(image.0);
    e.i32(image_layout);
    e.pointer(true);
    p_color.encode(e);
    e.u32(range_count);
    {
        let s = e.dyn_array(p_ranges, range_count as usize);
        for x in s { x.encode(e); }
    }
}

/// Odpoved na `vkCmdClearColorImage`.
#[derive(Debug, Clone, Default)]
pub struct CmdClearColorImageReply {
}

/// Precte odpoved na `vkCmdClearColorImage` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_cmd_clear_color_image_reply(d: &mut Decoder) -> Result<CmdClearColorImageReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkCmdClearColorImage_EXT {
        return Err(CsError::Unexpected);
    }
    let r = CmdClearColorImageReply::default();
    Ok(r)
}

/// `vkCmdCopyImageToBuffer`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn cmd_copy_image_to_buffer(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, command_buffer: VkCommandBuffer, src_image: VkImage, src_image_layout: VkImageLayout, dst_buffer: VkBuffer, region_count: u32, p_regions: &[VkBufferImageCopy]) {
    e.i32(VK_COMMAND_TYPE_vkCmdCopyImageToBuffer_EXT);
    e.u32(cmd_flags);
    e.u64(command_buffer.0);
    e.u64(src_image.0);
    e.i32(src_image_layout);
    e.u64(dst_buffer.0);
    e.u32(region_count);
    {
        let s = e.dyn_array(p_regions, region_count as usize);
        for x in s { x.encode(e); }
    }
}

/// Odpoved na `vkCmdCopyImageToBuffer`.
#[derive(Debug, Clone, Default)]
pub struct CmdCopyImageToBufferReply {
}

/// Precte odpoved na `vkCmdCopyImageToBuffer` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_cmd_copy_image_to_buffer_reply(d: &mut Decoder) -> Result<CmdCopyImageToBufferReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkCmdCopyImageToBuffer_EXT {
        return Err(CsError::Unexpected);
    }
    let r = CmdCopyImageToBufferReply::default();
    Ok(r)
}

/// `vkCmdCopyBufferToImage`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn cmd_copy_buffer_to_image(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, command_buffer: VkCommandBuffer, src_buffer: VkBuffer, dst_image: VkImage, dst_image_layout: VkImageLayout, region_count: u32, p_regions: &[VkBufferImageCopy]) {
    e.i32(VK_COMMAND_TYPE_vkCmdCopyBufferToImage_EXT);
    e.u32(cmd_flags);
    e.u64(command_buffer.0);
    e.u64(src_buffer.0);
    e.u64(dst_image.0);
    e.i32(dst_image_layout);
    e.u32(region_count);
    {
        let s = e.dyn_array(p_regions, region_count as usize);
        for x in s { x.encode(e); }
    }
}

/// Odpoved na `vkCmdCopyBufferToImage`.
#[derive(Debug, Clone, Default)]
pub struct CmdCopyBufferToImageReply {
}

/// Precte odpoved na `vkCmdCopyBufferToImage` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_cmd_copy_buffer_to_image_reply(d: &mut Decoder) -> Result<CmdCopyBufferToImageReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkCmdCopyBufferToImage_EXT {
        return Err(CsError::Unexpected);
    }
    let r = CmdCopyBufferToImageReply::default();
    Ok(r)
}

/// `vkQueueSubmit`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn queue_submit(e: &mut Encoder, cmd_flags: VkCommandFlagsEXT, queue: VkQueue, submit_count: u32, p_submits: &[VkSubmitInfo<'_>], fence: VkFence) {
    e.i32(VK_COMMAND_TYPE_vkQueueSubmit_EXT);
    e.u32(cmd_flags);
    e.u64(queue.0);
    e.u32(submit_count);
    {
        let s = e.dyn_array(p_submits, submit_count as usize);
        for x in s { x.encode(e); }
    }
    e.u64(fence.0);
}

/// Odpoved na `vkQueueSubmit`.
#[derive(Debug, Clone, Default)]
pub struct QueueSubmitReply {
    pub ret: VkResult,
}

/// Precte odpoved na `vkQueueSubmit` (typ prikazu, navratova hodnota, vystupy).
pub fn decode_queue_submit_reply(d: &mut Decoder) -> Result<QueueSubmitReply> {
    if d.i32()? != VK_COMMAND_TYPE_vkQueueSubmit_EXT {
        return Err(CsError::Unexpected);
    }
    let mut r = QueueSubmitReply::default();
    r.ret = d.i32()?;
    Ok(r)
}

