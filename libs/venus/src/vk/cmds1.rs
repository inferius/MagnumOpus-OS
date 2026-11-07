// Vygenerovano `cargo xtask venus-gen` z libs/venus/xml (vk.xml a venus-protocol,
// licence v libs/venus/xml/README.md) a libs/venus/subset.txt. Neupravovat rucne.

use alloc::vec::Vec;

use super::super::*;
use crate::cs::{CsError, Decoder, Encoder, Result};

/// `vkSetReplyCommandStreamMESA`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).
pub fn set_reply_command_stream_mesa(e: &mut Encoder, flags: VkCommandFlagsEXT, p_stream: &VkCommandStreamDescriptionMESA) {
    e.i32(VK_COMMAND_TYPE_vkSetReplyCommandStreamMESA_EXT);
    e.u32(flags);
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
pub fn seek_reply_command_stream_mesa(e: &mut Encoder, flags: VkCommandFlagsEXT, position: u64) {
    e.i32(VK_COMMAND_TYPE_vkSeekReplyCommandStreamMESA_EXT);
    e.u32(flags);
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
pub fn enumerate_instance_version(e: &mut Encoder, flags: VkCommandFlagsEXT) {
    e.i32(VK_COMMAND_TYPE_vkEnumerateInstanceVersion_EXT);
    e.u32(flags);
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
pub fn create_instance(e: &mut Encoder, flags: VkCommandFlagsEXT, p_create_info: &VkInstanceCreateInfo<'_>, p_instance: VkInstance) {
    e.i32(VK_COMMAND_TYPE_vkCreateInstance_EXT);
    e.u32(flags);
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
pub fn destroy_instance(e: &mut Encoder, flags: VkCommandFlagsEXT, instance: VkInstance) {
    e.i32(VK_COMMAND_TYPE_vkDestroyInstance_EXT);
    e.u32(flags);
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
pub fn enumerate_physical_devices(e: &mut Encoder, flags: VkCommandFlagsEXT, instance: VkInstance, p_physical_device_count: Option<u32>, p_physical_devices: Option<&[VkPhysicalDevice]>) {
    e.i32(VK_COMMAND_TYPE_vkEnumeratePhysicalDevices_EXT);
    e.u32(flags);
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
pub fn get_physical_device_properties(e: &mut Encoder, flags: VkCommandFlagsEXT, physical_device: VkPhysicalDevice) {
    e.i32(VK_COMMAND_TYPE_vkGetPhysicalDeviceProperties_EXT);
    e.u32(flags);
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
pub fn get_physical_device_queue_family_properties(e: &mut Encoder, flags: VkCommandFlagsEXT, physical_device: VkPhysicalDevice, p_queue_family_property_count: Option<u32>, p_queue_family_properties: bool) {
    e.i32(VK_COMMAND_TYPE_vkGetPhysicalDeviceQueueFamilyProperties_EXT);
    e.u32(flags);
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
pub fn get_physical_device_memory_properties(e: &mut Encoder, flags: VkCommandFlagsEXT, physical_device: VkPhysicalDevice) {
    e.i32(VK_COMMAND_TYPE_vkGetPhysicalDeviceMemoryProperties_EXT);
    e.u32(flags);
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

