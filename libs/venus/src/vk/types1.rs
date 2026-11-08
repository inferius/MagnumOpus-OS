// Vygenerovano `cargo xtask venus-gen` z libs/venus/xml (vk.xml a venus-protocol,
// licence v libs/venus/xml/README.md) a libs/venus/subset.txt. Neupravovat rucne.

use alloc::vec::Vec;

use super::*;
use crate::cs::{CsError, Decoder, Encoder, Result};

// Skalarni typy (enum = i32, bitmaska a FlagBits = u32/u64).
pub type VkAccessFlagBits = u32;
pub type VkAccessFlags = u32;
pub type VkBool32 = u32;
pub type VkBufferCreateFlagBits = u32;
pub type VkBufferCreateFlags = u32;
pub type VkBufferUsageFlagBits = u32;
pub type VkBufferUsageFlags = u32;
pub type VkCommandBufferLevel = i32;
pub type VkCommandBufferResetFlagBits = u32;
pub type VkCommandBufferResetFlags = u32;
pub type VkCommandBufferUsageFlagBits = u32;
pub type VkCommandBufferUsageFlags = u32;
pub type VkCommandFlagBitsEXT = u32;
pub type VkCommandFlagsEXT = u32;
pub type VkCommandPoolCreateFlagBits = u32;
pub type VkCommandPoolCreateFlags = u32;
pub type VkCommandTypeEXT = i32;
pub type VkDependencyFlagBits = u32;
pub type VkDependencyFlags = u32;
pub type VkDescriptorPoolCreateFlagBits = u32;
pub type VkDescriptorPoolCreateFlags = u32;
pub type VkDescriptorSetLayoutCreateFlagBits = u32;
pub type VkDescriptorSetLayoutCreateFlags = u32;
pub type VkDescriptorType = i32;
pub type VkDeviceCreateFlags = u32;
pub type VkDeviceQueueCreateFlagBits = u32;
pub type VkDeviceQueueCreateFlags = u32;
pub type VkDeviceSize = u64;
pub type VkFenceCreateFlagBits = u32;
pub type VkFenceCreateFlags = u32;
pub type VkFormat = i32;
pub type VkFormatFeatureFlagBits = u32;
pub type VkFormatFeatureFlags = u32;
pub type VkImageAspectFlagBits = u32;
pub type VkImageAspectFlags = u32;
pub type VkImageCreateFlagBits = u32;
pub type VkImageCreateFlags = u32;
pub type VkImageLayout = i32;
pub type VkImageTiling = i32;
pub type VkImageType = i32;
pub type VkImageUsageFlagBits = u32;
pub type VkImageUsageFlags = u32;
pub type VkInstanceCreateFlagBits = u32;
pub type VkInstanceCreateFlags = u32;
pub type VkMemoryHeapFlagBits = u32;
pub type VkMemoryHeapFlags = u32;
pub type VkMemoryPropertyFlagBits = u32;
pub type VkMemoryPropertyFlags = u32;
pub type VkPhysicalDeviceType = i32;
pub type VkPipelineBindPoint = i32;
pub type VkPipelineCreateFlagBits = u32;
pub type VkPipelineCreateFlags = u32;
pub type VkPipelineLayoutCreateFlagBits = u32;
pub type VkPipelineLayoutCreateFlags = u32;
pub type VkPipelineShaderStageCreateFlagBits = u32;
pub type VkPipelineShaderStageCreateFlags = u32;
pub type VkPipelineStageFlagBits = u32;
pub type VkPipelineStageFlags = u32;
pub type VkQueryControlFlagBits = u32;
pub type VkQueryControlFlags = u32;
pub type VkQueryPipelineStatisticFlagBits = u32;
pub type VkQueryPipelineStatisticFlags = u32;
pub type VkQueueFlagBits = u32;
pub type VkQueueFlags = u32;
pub type VkResult = i32;
pub type VkSampleCountFlagBits = u32;
pub type VkSampleCountFlags = u32;
pub type VkShaderModuleCreateFlags = u32;
pub type VkShaderStageFlagBits = u32;
pub type VkShaderStageFlags = u32;
pub type VkSharingMode = i32;
pub type VkStructureType = i32;

// Handly: id objektu, ktere prideluje klient (u64).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VkBuffer(pub u64);
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VkBufferView(pub u64);
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VkCommandBuffer(pub u64);
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VkCommandPool(pub u64);
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VkDescriptorPool(pub u64);
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VkDescriptorSet(pub u64);
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VkDescriptorSetLayout(pub u64);
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VkDevice(pub u64);
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VkDeviceMemory(pub u64);
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VkFence(pub u64);
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VkFramebuffer(pub u64);
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VkImage(pub u64);
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VkImageView(pub u64);
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VkInstance(pub u64);
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VkPhysicalDevice(pub u64);
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VkPipeline(pub u64);
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VkPipelineCache(pub u64);
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VkPipelineLayout(pub u64);
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VkQueue(pub u64);
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VkRenderPass(pub u64);
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VkSampler(pub u64);
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VkSemaphore(pub u64);
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VkShaderModule(pub u64);

// Konstanty API.
pub const VK_ATTACHMENT_UNUSED: u32 = u32::MAX;
pub const VK_COMPRESSED_TRIANGLE_FORMAT_DGF1_BYTE_ALIGNMENT_AMDX: u32 = 128;
pub const VK_COMPRESSED_TRIANGLE_FORMAT_DGF1_BYTE_STRIDE_AMDX: u32 = 128;
pub const VK_COMPUTE_OCCUPANCY_PRIORITY_HIGH_NV: f32 = 0.75_f32;
pub const VK_COMPUTE_OCCUPANCY_PRIORITY_LOW_NV: f32 = 0.25_f32;
pub const VK_COMPUTE_OCCUPANCY_PRIORITY_NORMAL_NV: f32 = 0.50_f32;
pub const VK_DATA_GRAPH_MODEL_TOOLCHAIN_VERSION_LENGTH_QCOM: u32 = 3;
pub const VK_FALSE: u32 = 0;
pub const VK_LOD_CLAMP_NONE: f32 = 1000.0_f32;
pub const VK_LUID_SIZE: u32 = 8;
pub const VK_MAX_DATA_GRAPH_TOSA_NAME_SIZE_ARM: u32 = 128;
pub const VK_MAX_DESCRIPTION_SIZE: u32 = 256;
pub const VK_MAX_DEVICE_GROUP_SIZE: u32 = 32;
pub const VK_MAX_DRIVER_INFO_SIZE: u32 = 256;
pub const VK_MAX_DRIVER_NAME_SIZE: u32 = 256;
pub const VK_MAX_EXTENSION_NAME_SIZE: u32 = 256;
pub const VK_MAX_GLOBAL_PRIORITY_SIZE: u32 = 16;
pub const VK_MAX_MEMORY_HEAPS: u32 = 16;
pub const VK_MAX_MEMORY_TYPES: u32 = 32;
pub const VK_MAX_PHYSICAL_DEVICE_DATA_GRAPH_OPERATION_SET_NAME_SIZE_ARM: u32 = 128;
pub const VK_MAX_PHYSICAL_DEVICE_NAME_SIZE: u32 = 256;
pub const VK_MAX_PIPELINE_BINARY_KEY_SIZE_KHR: u32 = 32;
pub const VK_MAX_SHADER_MODULE_IDENTIFIER_SIZE_EXT: u32 = 32;
pub const VK_MAX_TENSOR_CREATE_INFO_ROLLING_BACKING_WRAP_COUNT_ARM: u32 = 4;
pub const VK_MAX_VIDEO_AV1_REFERENCES_PER_FRAME_KHR: u32 = 7;
pub const VK_MAX_VIDEO_VP9_REFERENCES_PER_FRAME_KHR: u32 = 3;
pub const VK_PARTITIONED_ACCELERATION_STRUCTURE_PARTITION_INDEX_GLOBAL_NV: u32 = u32::MAX;
pub const VK_QUEUE_FAMILY_EXTERNAL: u32 = u32::MAX - 1;
pub const VK_QUEUE_FAMILY_FOREIGN_EXT: u32 = u32::MAX - 2;
pub const VK_QUEUE_FAMILY_IGNORED: u32 = u32::MAX;
pub const VK_REMAINING_3D_SLICES_EXT: u32 = u32::MAX;
pub const VK_REMAINING_ARRAY_LAYERS: u32 = u32::MAX;
pub const VK_REMAINING_MIP_LEVELS: u32 = u32::MAX;
pub const VK_SHADER_INDEX_UNUSED_AMDX: u32 = u32::MAX;
pub const VK_SHADER_UNUSED_KHR: u32 = u32::MAX;
pub const VK_SUBPASS_EXTERNAL: u32 = u32::MAX;
pub const VK_TRUE: u32 = 1;
pub const VK_UUID_SIZE: u32 = 16;
pub const VK_WHOLE_SIZE: u64 = u64::MAX;

// VkAccessFlagBits
pub const VK_ACCESS_NONE: VkAccessFlagBits = 0;
pub const VK_ACCESS_NONE_KHR: VkAccessFlagBits = 0;
pub const VK_ACCESS_INDIRECT_COMMAND_READ_BIT: VkAccessFlagBits = 1;
pub const VK_ACCESS_INDEX_READ_BIT: VkAccessFlagBits = 2;
pub const VK_ACCESS_VERTEX_ATTRIBUTE_READ_BIT: VkAccessFlagBits = 4;
pub const VK_ACCESS_UNIFORM_READ_BIT: VkAccessFlagBits = 8;
pub const VK_ACCESS_INPUT_ATTACHMENT_READ_BIT: VkAccessFlagBits = 0x10;
pub const VK_ACCESS_SHADER_READ_BIT: VkAccessFlagBits = 0x20;
pub const VK_ACCESS_SHADER_WRITE_BIT: VkAccessFlagBits = 0x40;
pub const VK_ACCESS_COLOR_ATTACHMENT_READ_BIT: VkAccessFlagBits = 0x80;
pub const VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT: VkAccessFlagBits = 0x100;
pub const VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT: VkAccessFlagBits = 0x200;
pub const VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT: VkAccessFlagBits = 0x400;
pub const VK_ACCESS_TRANSFER_READ_BIT: VkAccessFlagBits = 0x800;
pub const VK_ACCESS_TRANSFER_WRITE_BIT: VkAccessFlagBits = 0x1000;
pub const VK_ACCESS_HOST_READ_BIT: VkAccessFlagBits = 0x2000;
pub const VK_ACCESS_HOST_WRITE_BIT: VkAccessFlagBits = 0x4000;
pub const VK_ACCESS_MEMORY_READ_BIT: VkAccessFlagBits = 0x8000;
pub const VK_ACCESS_MEMORY_WRITE_BIT: VkAccessFlagBits = 0x10000;
pub const VK_ACCESS_COMMAND_PREPROCESS_READ_BIT_EXT: VkAccessFlagBits = 0x20000;
pub const VK_ACCESS_COMMAND_PREPROCESS_READ_BIT_NV: VkAccessFlagBits = 0x20000;
pub const VK_ACCESS_COMMAND_PREPROCESS_WRITE_BIT_EXT: VkAccessFlagBits = 0x40000;
pub const VK_ACCESS_COMMAND_PREPROCESS_WRITE_BIT_NV: VkAccessFlagBits = 0x40000;
pub const VK_ACCESS_COLOR_ATTACHMENT_READ_NONCOHERENT_BIT_EXT: VkAccessFlagBits = 0x80000;
pub const VK_ACCESS_CONDITIONAL_RENDERING_READ_BIT_EXT: VkAccessFlagBits = 0x100000;
pub const VK_ACCESS_ACCELERATION_STRUCTURE_READ_BIT_KHR: VkAccessFlagBits = 0x200000;
pub const VK_ACCESS_ACCELERATION_STRUCTURE_READ_BIT_NV: VkAccessFlagBits = 0x200000;
pub const VK_ACCESS_ACCELERATION_STRUCTURE_WRITE_BIT_KHR: VkAccessFlagBits = 0x400000;
pub const VK_ACCESS_ACCELERATION_STRUCTURE_WRITE_BIT_NV: VkAccessFlagBits = 0x400000;
pub const VK_ACCESS_FRAGMENT_SHADING_RATE_ATTACHMENT_READ_BIT_KHR: VkAccessFlagBits = 0x800000;
pub const VK_ACCESS_SHADING_RATE_IMAGE_READ_BIT_NV: VkAccessFlagBits = 0x800000;
pub const VK_ACCESS_FRAGMENT_DENSITY_MAP_READ_BIT_EXT: VkAccessFlagBits = 0x1000000;
pub const VK_ACCESS_TRANSFORM_FEEDBACK_WRITE_BIT_EXT: VkAccessFlagBits = 0x2000000;
pub const VK_ACCESS_TRANSFORM_FEEDBACK_COUNTER_READ_BIT_EXT: VkAccessFlagBits = 0x4000000;
pub const VK_ACCESS_TRANSFORM_FEEDBACK_COUNTER_WRITE_BIT_EXT: VkAccessFlagBits = 0x8000000;

// VkBufferCreateFlagBits
pub const VK_BUFFER_CREATE_SPARSE_BINDING_BIT: VkBufferCreateFlagBits = 1;
pub const VK_BUFFER_CREATE_SPARSE_RESIDENCY_BIT: VkBufferCreateFlagBits = 2;
pub const VK_BUFFER_CREATE_SPARSE_ALIASED_BIT: VkBufferCreateFlagBits = 4;
pub const VK_BUFFER_CREATE_PROTECTED_BIT: VkBufferCreateFlagBits = 8;
pub const VK_BUFFER_CREATE_DEVICE_ADDRESS_CAPTURE_REPLAY_BIT: VkBufferCreateFlagBits = 0x10;
pub const VK_BUFFER_CREATE_DEVICE_ADDRESS_CAPTURE_REPLAY_BIT_EXT: VkBufferCreateFlagBits = 0x10;
pub const VK_BUFFER_CREATE_DEVICE_ADDRESS_CAPTURE_REPLAY_BIT_KHR: VkBufferCreateFlagBits = 0x10;
pub const VK_BUFFER_CREATE_DESCRIPTOR_BUFFER_CAPTURE_REPLAY_BIT_EXT: VkBufferCreateFlagBits = 0x20;
pub const VK_BUFFER_CREATE_VIDEO_PROFILE_INDEPENDENT_BIT_KHR: VkBufferCreateFlagBits = 0x40;

// VkBufferUsageFlagBits
pub const VK_BUFFER_USAGE_TRANSFER_SRC_BIT: VkBufferUsageFlagBits = 1;
pub const VK_BUFFER_USAGE_TRANSFER_DST_BIT: VkBufferUsageFlagBits = 2;
pub const VK_BUFFER_USAGE_UNIFORM_TEXEL_BUFFER_BIT: VkBufferUsageFlagBits = 4;
pub const VK_BUFFER_USAGE_STORAGE_TEXEL_BUFFER_BIT: VkBufferUsageFlagBits = 8;
pub const VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT: VkBufferUsageFlagBits = 0x10;
pub const VK_BUFFER_USAGE_STORAGE_BUFFER_BIT: VkBufferUsageFlagBits = 0x20;
pub const VK_BUFFER_USAGE_INDEX_BUFFER_BIT: VkBufferUsageFlagBits = 0x40;
pub const VK_BUFFER_USAGE_VERTEX_BUFFER_BIT: VkBufferUsageFlagBits = 0x80;
pub const VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT: VkBufferUsageFlagBits = 0x100;
pub const VK_BUFFER_USAGE_CONDITIONAL_RENDERING_BIT_EXT: VkBufferUsageFlagBits = 0x200;
pub const VK_BUFFER_USAGE_RAY_TRACING_BIT_NV: VkBufferUsageFlagBits = 0x400;
pub const VK_BUFFER_USAGE_SHADER_BINDING_TABLE_BIT_KHR: VkBufferUsageFlagBits = 0x400;
pub const VK_BUFFER_USAGE_TRANSFORM_FEEDBACK_BUFFER_BIT_EXT: VkBufferUsageFlagBits = 0x800;
pub const VK_BUFFER_USAGE_TRANSFORM_FEEDBACK_COUNTER_BUFFER_BIT_EXT: VkBufferUsageFlagBits = 0x1000;
pub const VK_BUFFER_USAGE_VIDEO_DECODE_SRC_BIT_KHR: VkBufferUsageFlagBits = 0x2000;
pub const VK_BUFFER_USAGE_VIDEO_DECODE_DST_BIT_KHR: VkBufferUsageFlagBits = 0x4000;
pub const VK_BUFFER_USAGE_VIDEO_ENCODE_DST_BIT_KHR: VkBufferUsageFlagBits = 0x8000;
pub const VK_BUFFER_USAGE_VIDEO_ENCODE_SRC_BIT_KHR: VkBufferUsageFlagBits = 0x10000;
pub const VK_BUFFER_USAGE_SHADER_DEVICE_ADDRESS_BIT: VkBufferUsageFlagBits = 0x20000;
pub const VK_BUFFER_USAGE_SHADER_DEVICE_ADDRESS_BIT_EXT: VkBufferUsageFlagBits = 0x20000;
pub const VK_BUFFER_USAGE_SHADER_DEVICE_ADDRESS_BIT_KHR: VkBufferUsageFlagBits = 0x20000;
pub const VK_BUFFER_USAGE_ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_BIT_KHR: VkBufferUsageFlagBits = 0x80000;
pub const VK_BUFFER_USAGE_ACCELERATION_STRUCTURE_STORAGE_BIT_KHR: VkBufferUsageFlagBits = 0x100000;
pub const VK_BUFFER_USAGE_SAMPLER_DESCRIPTOR_BUFFER_BIT_EXT: VkBufferUsageFlagBits = 0x200000;
pub const VK_BUFFER_USAGE_RESOURCE_DESCRIPTOR_BUFFER_BIT_EXT: VkBufferUsageFlagBits = 0x400000;
pub const VK_BUFFER_USAGE_MICROMAP_BUILD_INPUT_READ_ONLY_BIT_EXT: VkBufferUsageFlagBits = 0x800000;
pub const VK_BUFFER_USAGE_MICROMAP_STORAGE_BIT_EXT: VkBufferUsageFlagBits = 0x1000000;
pub const VK_BUFFER_USAGE_EXECUTION_GRAPH_SCRATCH_BIT_AMDX: VkBufferUsageFlagBits = 0x2000000;
pub const VK_BUFFER_USAGE_PUSH_DESCRIPTORS_DESCRIPTOR_BUFFER_BIT_EXT: VkBufferUsageFlagBits = 0x4000000;
pub const VK_BUFFER_USAGE_TILE_MEMORY_BIT_QCOM: VkBufferUsageFlagBits = 0x8000000;
pub const VK_BUFFER_USAGE_DESCRIPTOR_HEAP_BIT_EXT: VkBufferUsageFlagBits = 0x10000000;

// VkCommandBufferLevel
pub const VK_COMMAND_BUFFER_LEVEL_PRIMARY: VkCommandBufferLevel = 0;
pub const VK_COMMAND_BUFFER_LEVEL_SECONDARY: VkCommandBufferLevel = 1;

// VkCommandBufferResetFlagBits
pub const VK_COMMAND_BUFFER_RESET_RELEASE_RESOURCES_BIT: VkCommandBufferResetFlagBits = 1;

// VkCommandBufferUsageFlagBits
pub const VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT: VkCommandBufferUsageFlagBits = 1;
pub const VK_COMMAND_BUFFER_USAGE_RENDER_PASS_CONTINUE_BIT: VkCommandBufferUsageFlagBits = 2;
pub const VK_COMMAND_BUFFER_USAGE_SIMULTANEOUS_USE_BIT: VkCommandBufferUsageFlagBits = 4;

// VkCommandFlagBitsEXT
pub const VK_COMMAND_GENERATE_REPLY_BIT_EXT: VkCommandFlagBitsEXT = 1;

// VkCommandPoolCreateFlagBits
pub const VK_COMMAND_POOL_CREATE_TRANSIENT_BIT: VkCommandPoolCreateFlagBits = 1;
pub const VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT: VkCommandPoolCreateFlagBits = 2;
pub const VK_COMMAND_POOL_CREATE_PROTECTED_BIT: VkCommandPoolCreateFlagBits = 4;

// VkCommandTypeEXT
pub const VK_COMMAND_TYPE_vkCreateInstance_EXT: VkCommandTypeEXT = 0;
pub const VK_COMMAND_TYPE_vkDestroyInstance_EXT: VkCommandTypeEXT = 1;
pub const VK_COMMAND_TYPE_vkEnumeratePhysicalDevices_EXT: VkCommandTypeEXT = 2;
pub const VK_COMMAND_TYPE_vkGetPhysicalDeviceFormatProperties_EXT: VkCommandTypeEXT = 4;
pub const VK_COMMAND_TYPE_vkGetPhysicalDeviceProperties_EXT: VkCommandTypeEXT = 6;
pub const VK_COMMAND_TYPE_vkGetPhysicalDeviceQueueFamilyProperties_EXT: VkCommandTypeEXT = 7;
pub const VK_COMMAND_TYPE_vkGetPhysicalDeviceMemoryProperties_EXT: VkCommandTypeEXT = 8;
pub const VK_COMMAND_TYPE_vkCreateDevice_EXT: VkCommandTypeEXT = 11;
pub const VK_COMMAND_TYPE_vkDestroyDevice_EXT: VkCommandTypeEXT = 12;
pub const VK_COMMAND_TYPE_vkQueueSubmit_EXT: VkCommandTypeEXT = 18;
pub const VK_COMMAND_TYPE_vkDeviceWaitIdle_EXT: VkCommandTypeEXT = 20;
pub const VK_COMMAND_TYPE_vkAllocateMemory_EXT: VkCommandTypeEXT = 21;
pub const VK_COMMAND_TYPE_vkFreeMemory_EXT: VkCommandTypeEXT = 22;
pub const VK_COMMAND_TYPE_vkBindBufferMemory_EXT: VkCommandTypeEXT = 28;
pub const VK_COMMAND_TYPE_vkBindImageMemory_EXT: VkCommandTypeEXT = 29;
pub const VK_COMMAND_TYPE_vkGetBufferMemoryRequirements_EXT: VkCommandTypeEXT = 30;
pub const VK_COMMAND_TYPE_vkGetImageMemoryRequirements_EXT: VkCommandTypeEXT = 31;
pub const VK_COMMAND_TYPE_vkCreateFence_EXT: VkCommandTypeEXT = 35;
pub const VK_COMMAND_TYPE_vkDestroyFence_EXT: VkCommandTypeEXT = 36;
pub const VK_COMMAND_TYPE_vkResetFences_EXT: VkCommandTypeEXT = 37;
pub const VK_COMMAND_TYPE_vkGetFenceStatus_EXT: VkCommandTypeEXT = 38;
pub const VK_COMMAND_TYPE_vkWaitForFences_EXT: VkCommandTypeEXT = 39;
pub const VK_COMMAND_TYPE_vkCreateBuffer_EXT: VkCommandTypeEXT = 50;
pub const VK_COMMAND_TYPE_vkDestroyBuffer_EXT: VkCommandTypeEXT = 51;
pub const VK_COMMAND_TYPE_vkCreateImage_EXT: VkCommandTypeEXT = 54;
pub const VK_COMMAND_TYPE_vkDestroyImage_EXT: VkCommandTypeEXT = 55;
pub const VK_COMMAND_TYPE_vkCreateShaderModule_EXT: VkCommandTypeEXT = 59;
pub const VK_COMMAND_TYPE_vkDestroyShaderModule_EXT: VkCommandTypeEXT = 60;
pub const VK_COMMAND_TYPE_vkCreateComputePipelines_EXT: VkCommandTypeEXT = 66;
pub const VK_COMMAND_TYPE_vkDestroyPipeline_EXT: VkCommandTypeEXT = 67;
pub const VK_COMMAND_TYPE_vkCreatePipelineLayout_EXT: VkCommandTypeEXT = 68;
pub const VK_COMMAND_TYPE_vkDestroyPipelineLayout_EXT: VkCommandTypeEXT = 69;
pub const VK_COMMAND_TYPE_vkCreateDescriptorSetLayout_EXT: VkCommandTypeEXT = 72;
pub const VK_COMMAND_TYPE_vkDestroyDescriptorSetLayout_EXT: VkCommandTypeEXT = 73;
pub const VK_COMMAND_TYPE_vkCreateDescriptorPool_EXT: VkCommandTypeEXT = 74;
pub const VK_COMMAND_TYPE_vkDestroyDescriptorPool_EXT: VkCommandTypeEXT = 75;
pub const VK_COMMAND_TYPE_vkAllocateDescriptorSets_EXT: VkCommandTypeEXT = 77;
pub const VK_COMMAND_TYPE_vkFreeDescriptorSets_EXT: VkCommandTypeEXT = 78;
pub const VK_COMMAND_TYPE_vkUpdateDescriptorSets_EXT: VkCommandTypeEXT = 79;
pub const VK_COMMAND_TYPE_vkCreateCommandPool_EXT: VkCommandTypeEXT = 85;
pub const VK_COMMAND_TYPE_vkDestroyCommandPool_EXT: VkCommandTypeEXT = 86;
pub const VK_COMMAND_TYPE_vkAllocateCommandBuffers_EXT: VkCommandTypeEXT = 88;
pub const VK_COMMAND_TYPE_vkFreeCommandBuffers_EXT: VkCommandTypeEXT = 89;
pub const VK_COMMAND_TYPE_vkBeginCommandBuffer_EXT: VkCommandTypeEXT = 90;
pub const VK_COMMAND_TYPE_vkEndCommandBuffer_EXT: VkCommandTypeEXT = 91;
pub const VK_COMMAND_TYPE_vkResetCommandBuffer_EXT: VkCommandTypeEXT = 92;
pub const VK_COMMAND_TYPE_vkCmdBindPipeline_EXT: VkCommandTypeEXT = 93;
pub const VK_COMMAND_TYPE_vkCmdBindDescriptorSets_EXT: VkCommandTypeEXT = 103;
pub const VK_COMMAND_TYPE_vkCmdDispatch_EXT: VkCommandTypeEXT = 110;
pub const VK_COMMAND_TYPE_vkCmdCopyBuffer_EXT: VkCommandTypeEXT = 112;
pub const VK_COMMAND_TYPE_vkCmdCopyBufferToImage_EXT: VkCommandTypeEXT = 115;
pub const VK_COMMAND_TYPE_vkCmdCopyImageToBuffer_EXT: VkCommandTypeEXT = 116;
pub const VK_COMMAND_TYPE_vkCmdClearColorImage_EXT: VkCommandTypeEXT = 119;
pub const VK_COMMAND_TYPE_vkCmdPipelineBarrier_EXT: VkCommandTypeEXT = 126;
pub const VK_COMMAND_TYPE_vkCmdPushConstants_EXT: VkCommandTypeEXT = 132;
pub const VK_COMMAND_TYPE_vkEnumerateInstanceVersion_EXT: VkCommandTypeEXT = 137;
pub const VK_COMMAND_TYPE_vkGetDeviceQueue2_EXT: VkCommandTypeEXT = 155;
pub const VK_COMMAND_TYPE_vkSetReplyCommandStreamMESA_EXT: VkCommandTypeEXT = 178;
pub const VK_COMMAND_TYPE_vkSeekReplyCommandStreamMESA_EXT: VkCommandTypeEXT = 179;

// VkDependencyFlagBits
pub const VK_DEPENDENCY_BY_REGION_BIT: VkDependencyFlagBits = 1;
pub const VK_DEPENDENCY_VIEW_LOCAL_BIT: VkDependencyFlagBits = 2;
pub const VK_DEPENDENCY_VIEW_LOCAL_BIT_KHR: VkDependencyFlagBits = 2;
pub const VK_DEPENDENCY_DEVICE_GROUP_BIT: VkDependencyFlagBits = 4;
pub const VK_DEPENDENCY_DEVICE_GROUP_BIT_KHR: VkDependencyFlagBits = 4;
pub const VK_DEPENDENCY_FEEDBACK_LOOP_BIT_EXT: VkDependencyFlagBits = 8;
pub const VK_DEPENDENCY_QUEUE_FAMILY_OWNERSHIP_TRANSFER_USE_ALL_STAGES_BIT_KHR: VkDependencyFlagBits = 0x20;
pub const VK_DEPENDENCY_ASYMMETRIC_EVENT_BIT_KHR: VkDependencyFlagBits = 0x40;

// VkDescriptorPoolCreateFlagBits
pub const VK_DESCRIPTOR_POOL_CREATE_FREE_DESCRIPTOR_SET_BIT: VkDescriptorPoolCreateFlagBits = 1;
pub const VK_DESCRIPTOR_POOL_CREATE_UPDATE_AFTER_BIND_BIT: VkDescriptorPoolCreateFlagBits = 2;
pub const VK_DESCRIPTOR_POOL_CREATE_UPDATE_AFTER_BIND_BIT_EXT: VkDescriptorPoolCreateFlagBits = 2;
pub const VK_DESCRIPTOR_POOL_CREATE_HOST_ONLY_BIT_EXT: VkDescriptorPoolCreateFlagBits = 4;
pub const VK_DESCRIPTOR_POOL_CREATE_HOST_ONLY_BIT_VALVE: VkDescriptorPoolCreateFlagBits = 4;
pub const VK_DESCRIPTOR_POOL_CREATE_ALLOW_OVERALLOCATION_SETS_BIT_NV: VkDescriptorPoolCreateFlagBits = 8;
pub const VK_DESCRIPTOR_POOL_CREATE_ALLOW_OVERALLOCATION_POOLS_BIT_NV: VkDescriptorPoolCreateFlagBits = 0x10;

// VkDescriptorSetLayoutCreateFlagBits
pub const VK_DESCRIPTOR_SET_LAYOUT_CREATE_PUSH_DESCRIPTOR_BIT: VkDescriptorSetLayoutCreateFlagBits = 1;
pub const VK_DESCRIPTOR_SET_LAYOUT_CREATE_PUSH_DESCRIPTOR_BIT_KHR: VkDescriptorSetLayoutCreateFlagBits = 1;
pub const VK_DESCRIPTOR_SET_LAYOUT_CREATE_UPDATE_AFTER_BIND_POOL_BIT: VkDescriptorSetLayoutCreateFlagBits = 2;
pub const VK_DESCRIPTOR_SET_LAYOUT_CREATE_UPDATE_AFTER_BIND_POOL_BIT_EXT: VkDescriptorSetLayoutCreateFlagBits = 2;
pub const VK_DESCRIPTOR_SET_LAYOUT_CREATE_HOST_ONLY_POOL_BIT_EXT: VkDescriptorSetLayoutCreateFlagBits = 4;
pub const VK_DESCRIPTOR_SET_LAYOUT_CREATE_HOST_ONLY_POOL_BIT_VALVE: VkDescriptorSetLayoutCreateFlagBits = 4;
pub const VK_DESCRIPTOR_SET_LAYOUT_CREATE_DESCRIPTOR_BUFFER_BIT_EXT: VkDescriptorSetLayoutCreateFlagBits = 0x10;
pub const VK_DESCRIPTOR_SET_LAYOUT_CREATE_EMBEDDED_IMMUTABLE_SAMPLERS_BIT_EXT: VkDescriptorSetLayoutCreateFlagBits = 0x20;
pub const VK_DESCRIPTOR_SET_LAYOUT_CREATE_PER_STAGE_BIT_NV: VkDescriptorSetLayoutCreateFlagBits = 0x40;
pub const VK_DESCRIPTOR_SET_LAYOUT_CREATE_INDIRECT_BINDABLE_BIT_NV: VkDescriptorSetLayoutCreateFlagBits = 0x80;

// VkDescriptorType
pub const VK_DESCRIPTOR_TYPE_SAMPLER: VkDescriptorType = 0;
pub const VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER: VkDescriptorType = 1;
pub const VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE: VkDescriptorType = 2;
pub const VK_DESCRIPTOR_TYPE_STORAGE_IMAGE: VkDescriptorType = 3;
pub const VK_DESCRIPTOR_TYPE_UNIFORM_TEXEL_BUFFER: VkDescriptorType = 4;
pub const VK_DESCRIPTOR_TYPE_STORAGE_TEXEL_BUFFER: VkDescriptorType = 5;
pub const VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER: VkDescriptorType = 6;
pub const VK_DESCRIPTOR_TYPE_STORAGE_BUFFER: VkDescriptorType = 7;
pub const VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER_DYNAMIC: VkDescriptorType = 8;
pub const VK_DESCRIPTOR_TYPE_STORAGE_BUFFER_DYNAMIC: VkDescriptorType = 9;
pub const VK_DESCRIPTOR_TYPE_INPUT_ATTACHMENT: VkDescriptorType = 10;
pub const VK_DESCRIPTOR_TYPE_INLINE_UNIFORM_BLOCK: VkDescriptorType = 1000138000;
pub const VK_DESCRIPTOR_TYPE_INLINE_UNIFORM_BLOCK_EXT: VkDescriptorType = 1000138000;
pub const VK_DESCRIPTOR_TYPE_ACCELERATION_STRUCTURE_KHR: VkDescriptorType = 1000150000;
pub const VK_DESCRIPTOR_TYPE_ACCELERATION_STRUCTURE_NV: VkDescriptorType = 1000165000;
pub const VK_DESCRIPTOR_TYPE_MUTABLE_EXT: VkDescriptorType = 1000351000;
pub const VK_DESCRIPTOR_TYPE_MUTABLE_VALVE: VkDescriptorType = 1000351000;
pub const VK_DESCRIPTOR_TYPE_SAMPLE_WEIGHT_IMAGE_QCOM: VkDescriptorType = 1000440000;
pub const VK_DESCRIPTOR_TYPE_BLOCK_MATCH_IMAGE_QCOM: VkDescriptorType = 1000440001;
pub const VK_DESCRIPTOR_TYPE_TENSOR_ARM: VkDescriptorType = 1000460000;
pub const VK_DESCRIPTOR_TYPE_PARTITIONED_ACCELERATION_STRUCTURE_NV: VkDescriptorType = 1000570000;

// VkDeviceQueueCreateFlagBits
pub const VK_DEVICE_QUEUE_CREATE_PROTECTED_BIT: VkDeviceQueueCreateFlagBits = 1;
pub const VK_DEVICE_QUEUE_CREATE_INTERNALLY_SYNCHRONIZED_BIT_KHR: VkDeviceQueueCreateFlagBits = 4;

// VkFenceCreateFlagBits
pub const VK_FENCE_CREATE_SIGNALED_BIT: VkFenceCreateFlagBits = 1;

// VkFormat
pub const VK_FORMAT_UNDEFINED: VkFormat = 0;
pub const VK_FORMAT_R4G4_UNORM_PACK8: VkFormat = 1;
pub const VK_FORMAT_R4G4B4A4_UNORM_PACK16: VkFormat = 2;
pub const VK_FORMAT_B4G4R4A4_UNORM_PACK16: VkFormat = 3;
pub const VK_FORMAT_R5G6B5_UNORM_PACK16: VkFormat = 4;
pub const VK_FORMAT_B5G6R5_UNORM_PACK16: VkFormat = 5;
pub const VK_FORMAT_R5G5B5A1_UNORM_PACK16: VkFormat = 6;
pub const VK_FORMAT_B5G5R5A1_UNORM_PACK16: VkFormat = 7;
pub const VK_FORMAT_A1R5G5B5_UNORM_PACK16: VkFormat = 8;
pub const VK_FORMAT_R8_UNORM: VkFormat = 9;
pub const VK_FORMAT_R8_SNORM: VkFormat = 10;
pub const VK_FORMAT_R8_USCALED: VkFormat = 11;
pub const VK_FORMAT_R8_SSCALED: VkFormat = 12;
pub const VK_FORMAT_R8_UINT: VkFormat = 13;
pub const VK_FORMAT_R8_SINT: VkFormat = 14;
pub const VK_FORMAT_R8_SRGB: VkFormat = 15;
pub const VK_FORMAT_R8G8_UNORM: VkFormat = 16;
pub const VK_FORMAT_R8G8_SNORM: VkFormat = 17;
pub const VK_FORMAT_R8G8_USCALED: VkFormat = 18;
pub const VK_FORMAT_R8G8_SSCALED: VkFormat = 19;
pub const VK_FORMAT_R8G8_UINT: VkFormat = 20;
pub const VK_FORMAT_R8G8_SINT: VkFormat = 21;
pub const VK_FORMAT_R8G8_SRGB: VkFormat = 22;
pub const VK_FORMAT_R8G8B8_UNORM: VkFormat = 23;
pub const VK_FORMAT_R8G8B8_SNORM: VkFormat = 24;
pub const VK_FORMAT_R8G8B8_USCALED: VkFormat = 25;
pub const VK_FORMAT_R8G8B8_SSCALED: VkFormat = 26;
pub const VK_FORMAT_R8G8B8_UINT: VkFormat = 27;
pub const VK_FORMAT_R8G8B8_SINT: VkFormat = 28;
pub const VK_FORMAT_R8G8B8_SRGB: VkFormat = 29;
pub const VK_FORMAT_B8G8R8_UNORM: VkFormat = 30;
pub const VK_FORMAT_B8G8R8_SNORM: VkFormat = 31;
pub const VK_FORMAT_B8G8R8_USCALED: VkFormat = 32;
pub const VK_FORMAT_B8G8R8_SSCALED: VkFormat = 33;
pub const VK_FORMAT_B8G8R8_UINT: VkFormat = 34;
pub const VK_FORMAT_B8G8R8_SINT: VkFormat = 35;
pub const VK_FORMAT_B8G8R8_SRGB: VkFormat = 36;
pub const VK_FORMAT_R8G8B8A8_UNORM: VkFormat = 37;
pub const VK_FORMAT_R8G8B8A8_SNORM: VkFormat = 38;
pub const VK_FORMAT_R8G8B8A8_USCALED: VkFormat = 39;
pub const VK_FORMAT_R8G8B8A8_SSCALED: VkFormat = 40;
pub const VK_FORMAT_R8G8B8A8_UINT: VkFormat = 41;
pub const VK_FORMAT_R8G8B8A8_SINT: VkFormat = 42;
pub const VK_FORMAT_R8G8B8A8_SRGB: VkFormat = 43;
pub const VK_FORMAT_B8G8R8A8_UNORM: VkFormat = 44;
pub const VK_FORMAT_B8G8R8A8_SNORM: VkFormat = 45;
pub const VK_FORMAT_B8G8R8A8_USCALED: VkFormat = 46;
pub const VK_FORMAT_B8G8R8A8_SSCALED: VkFormat = 47;
pub const VK_FORMAT_B8G8R8A8_UINT: VkFormat = 48;
pub const VK_FORMAT_B8G8R8A8_SINT: VkFormat = 49;
pub const VK_FORMAT_B8G8R8A8_SRGB: VkFormat = 50;
pub const VK_FORMAT_A8B8G8R8_UNORM_PACK32: VkFormat = 51;
pub const VK_FORMAT_A8B8G8R8_SNORM_PACK32: VkFormat = 52;
pub const VK_FORMAT_A8B8G8R8_USCALED_PACK32: VkFormat = 53;
pub const VK_FORMAT_A8B8G8R8_SSCALED_PACK32: VkFormat = 54;
pub const VK_FORMAT_A8B8G8R8_UINT_PACK32: VkFormat = 55;
pub const VK_FORMAT_A8B8G8R8_SINT_PACK32: VkFormat = 56;
pub const VK_FORMAT_A8B8G8R8_SRGB_PACK32: VkFormat = 57;
pub const VK_FORMAT_A2R10G10B10_UNORM_PACK32: VkFormat = 58;
pub const VK_FORMAT_A2R10G10B10_SNORM_PACK32: VkFormat = 59;
pub const VK_FORMAT_A2R10G10B10_USCALED_PACK32: VkFormat = 60;
pub const VK_FORMAT_A2R10G10B10_SSCALED_PACK32: VkFormat = 61;
pub const VK_FORMAT_A2R10G10B10_UINT_PACK32: VkFormat = 62;
pub const VK_FORMAT_A2R10G10B10_SINT_PACK32: VkFormat = 63;
pub const VK_FORMAT_A2B10G10R10_UNORM_PACK32: VkFormat = 64;
pub const VK_FORMAT_A2B10G10R10_SNORM_PACK32: VkFormat = 65;
pub const VK_FORMAT_A2B10G10R10_USCALED_PACK32: VkFormat = 66;
pub const VK_FORMAT_A2B10G10R10_SSCALED_PACK32: VkFormat = 67;
pub const VK_FORMAT_A2B10G10R10_UINT_PACK32: VkFormat = 68;
pub const VK_FORMAT_A2B10G10R10_SINT_PACK32: VkFormat = 69;
pub const VK_FORMAT_R16_UNORM: VkFormat = 70;
pub const VK_FORMAT_R16_SNORM: VkFormat = 71;
pub const VK_FORMAT_R16_USCALED: VkFormat = 72;
pub const VK_FORMAT_R16_SSCALED: VkFormat = 73;
pub const VK_FORMAT_R16_UINT: VkFormat = 74;
pub const VK_FORMAT_R16_SINT: VkFormat = 75;
pub const VK_FORMAT_R16_SFLOAT: VkFormat = 76;
pub const VK_FORMAT_R16G16_UNORM: VkFormat = 77;
pub const VK_FORMAT_R16G16_SNORM: VkFormat = 78;
pub const VK_FORMAT_R16G16_USCALED: VkFormat = 79;
pub const VK_FORMAT_R16G16_SSCALED: VkFormat = 80;
pub const VK_FORMAT_R16G16_UINT: VkFormat = 81;
pub const VK_FORMAT_R16G16_SINT: VkFormat = 82;
pub const VK_FORMAT_R16G16_SFLOAT: VkFormat = 83;
pub const VK_FORMAT_R16G16B16_UNORM: VkFormat = 84;
pub const VK_FORMAT_R16G16B16_SNORM: VkFormat = 85;
pub const VK_FORMAT_R16G16B16_USCALED: VkFormat = 86;
pub const VK_FORMAT_R16G16B16_SSCALED: VkFormat = 87;
pub const VK_FORMAT_R16G16B16_UINT: VkFormat = 88;
pub const VK_FORMAT_R16G16B16_SINT: VkFormat = 89;
pub const VK_FORMAT_R16G16B16_SFLOAT: VkFormat = 90;
pub const VK_FORMAT_R16G16B16A16_UNORM: VkFormat = 91;
pub const VK_FORMAT_R16G16B16A16_SNORM: VkFormat = 92;
pub const VK_FORMAT_R16G16B16A16_USCALED: VkFormat = 93;
pub const VK_FORMAT_R16G16B16A16_SSCALED: VkFormat = 94;
pub const VK_FORMAT_R16G16B16A16_UINT: VkFormat = 95;
pub const VK_FORMAT_R16G16B16A16_SINT: VkFormat = 96;
pub const VK_FORMAT_R16G16B16A16_SFLOAT: VkFormat = 97;
pub const VK_FORMAT_R32_UINT: VkFormat = 98;
pub const VK_FORMAT_R32_SINT: VkFormat = 99;
pub const VK_FORMAT_R32_SFLOAT: VkFormat = 100;
pub const VK_FORMAT_R32G32_UINT: VkFormat = 101;
pub const VK_FORMAT_R32G32_SINT: VkFormat = 102;
pub const VK_FORMAT_R32G32_SFLOAT: VkFormat = 103;
pub const VK_FORMAT_R32G32B32_UINT: VkFormat = 104;
pub const VK_FORMAT_R32G32B32_SINT: VkFormat = 105;
pub const VK_FORMAT_R32G32B32_SFLOAT: VkFormat = 106;
pub const VK_FORMAT_R32G32B32A32_UINT: VkFormat = 107;
pub const VK_FORMAT_R32G32B32A32_SINT: VkFormat = 108;
pub const VK_FORMAT_R32G32B32A32_SFLOAT: VkFormat = 109;
pub const VK_FORMAT_R64_UINT: VkFormat = 110;
pub const VK_FORMAT_R64_SINT: VkFormat = 111;
pub const VK_FORMAT_R64_SFLOAT: VkFormat = 112;
pub const VK_FORMAT_R64G64_UINT: VkFormat = 113;
pub const VK_FORMAT_R64G64_SINT: VkFormat = 114;
pub const VK_FORMAT_R64G64_SFLOAT: VkFormat = 115;
pub const VK_FORMAT_R64G64B64_UINT: VkFormat = 116;
pub const VK_FORMAT_R64G64B64_SINT: VkFormat = 117;
pub const VK_FORMAT_R64G64B64_SFLOAT: VkFormat = 118;
pub const VK_FORMAT_R64G64B64A64_UINT: VkFormat = 119;
pub const VK_FORMAT_R64G64B64A64_SINT: VkFormat = 120;
pub const VK_FORMAT_R64G64B64A64_SFLOAT: VkFormat = 121;
pub const VK_FORMAT_B10G11R11_UFLOAT_PACK32: VkFormat = 122;
pub const VK_FORMAT_E5B9G9R9_UFLOAT_PACK32: VkFormat = 123;
pub const VK_FORMAT_D16_UNORM: VkFormat = 124;
pub const VK_FORMAT_X8_D24_UNORM_PACK32: VkFormat = 125;
pub const VK_FORMAT_D32_SFLOAT: VkFormat = 126;
pub const VK_FORMAT_S8_UINT: VkFormat = 127;
pub const VK_FORMAT_D16_UNORM_S8_UINT: VkFormat = 128;
pub const VK_FORMAT_D24_UNORM_S8_UINT: VkFormat = 129;
pub const VK_FORMAT_D32_SFLOAT_S8_UINT: VkFormat = 130;
pub const VK_FORMAT_BC1_RGB_UNORM_BLOCK: VkFormat = 131;
pub const VK_FORMAT_BC1_RGB_SRGB_BLOCK: VkFormat = 132;
pub const VK_FORMAT_BC1_RGBA_UNORM_BLOCK: VkFormat = 133;
pub const VK_FORMAT_BC1_RGBA_SRGB_BLOCK: VkFormat = 134;
pub const VK_FORMAT_BC2_UNORM_BLOCK: VkFormat = 135;
pub const VK_FORMAT_BC2_SRGB_BLOCK: VkFormat = 136;
pub const VK_FORMAT_BC3_UNORM_BLOCK: VkFormat = 137;
pub const VK_FORMAT_BC3_SRGB_BLOCK: VkFormat = 138;
pub const VK_FORMAT_BC4_UNORM_BLOCK: VkFormat = 139;
pub const VK_FORMAT_BC4_SNORM_BLOCK: VkFormat = 140;
pub const VK_FORMAT_BC5_UNORM_BLOCK: VkFormat = 141;
pub const VK_FORMAT_BC5_SNORM_BLOCK: VkFormat = 142;
pub const VK_FORMAT_BC6H_UFLOAT_BLOCK: VkFormat = 143;
pub const VK_FORMAT_BC6H_SFLOAT_BLOCK: VkFormat = 144;
pub const VK_FORMAT_BC7_UNORM_BLOCK: VkFormat = 145;
pub const VK_FORMAT_BC7_SRGB_BLOCK: VkFormat = 146;
pub const VK_FORMAT_ETC2_R8G8B8_UNORM_BLOCK: VkFormat = 147;
pub const VK_FORMAT_ETC2_R8G8B8_SRGB_BLOCK: VkFormat = 148;
pub const VK_FORMAT_ETC2_R8G8B8A1_UNORM_BLOCK: VkFormat = 149;
pub const VK_FORMAT_ETC2_R8G8B8A1_SRGB_BLOCK: VkFormat = 150;
pub const VK_FORMAT_ETC2_R8G8B8A8_UNORM_BLOCK: VkFormat = 151;
pub const VK_FORMAT_ETC2_R8G8B8A8_SRGB_BLOCK: VkFormat = 152;
pub const VK_FORMAT_EAC_R11_UNORM_BLOCK: VkFormat = 153;
pub const VK_FORMAT_EAC_R11_SNORM_BLOCK: VkFormat = 154;
pub const VK_FORMAT_EAC_R11G11_UNORM_BLOCK: VkFormat = 155;
pub const VK_FORMAT_EAC_R11G11_SNORM_BLOCK: VkFormat = 156;
pub const VK_FORMAT_ASTC_4x4_UNORM_BLOCK: VkFormat = 157;
pub const VK_FORMAT_ASTC_4x4_SRGB_BLOCK: VkFormat = 158;
pub const VK_FORMAT_ASTC_5x4_UNORM_BLOCK: VkFormat = 159;
pub const VK_FORMAT_ASTC_5x4_SRGB_BLOCK: VkFormat = 160;
pub const VK_FORMAT_ASTC_5x5_UNORM_BLOCK: VkFormat = 161;
pub const VK_FORMAT_ASTC_5x5_SRGB_BLOCK: VkFormat = 162;
pub const VK_FORMAT_ASTC_6x5_UNORM_BLOCK: VkFormat = 163;
pub const VK_FORMAT_ASTC_6x5_SRGB_BLOCK: VkFormat = 164;
pub const VK_FORMAT_ASTC_6x6_UNORM_BLOCK: VkFormat = 165;
pub const VK_FORMAT_ASTC_6x6_SRGB_BLOCK: VkFormat = 166;
pub const VK_FORMAT_ASTC_8x5_UNORM_BLOCK: VkFormat = 167;
pub const VK_FORMAT_ASTC_8x5_SRGB_BLOCK: VkFormat = 168;
pub const VK_FORMAT_ASTC_8x6_UNORM_BLOCK: VkFormat = 169;
pub const VK_FORMAT_ASTC_8x6_SRGB_BLOCK: VkFormat = 170;
pub const VK_FORMAT_ASTC_8x8_UNORM_BLOCK: VkFormat = 171;
pub const VK_FORMAT_ASTC_8x8_SRGB_BLOCK: VkFormat = 172;
pub const VK_FORMAT_ASTC_10x5_UNORM_BLOCK: VkFormat = 173;
pub const VK_FORMAT_ASTC_10x5_SRGB_BLOCK: VkFormat = 174;
pub const VK_FORMAT_ASTC_10x6_UNORM_BLOCK: VkFormat = 175;
pub const VK_FORMAT_ASTC_10x6_SRGB_BLOCK: VkFormat = 176;
pub const VK_FORMAT_ASTC_10x8_UNORM_BLOCK: VkFormat = 177;
pub const VK_FORMAT_ASTC_10x8_SRGB_BLOCK: VkFormat = 178;
pub const VK_FORMAT_ASTC_10x10_UNORM_BLOCK: VkFormat = 179;
pub const VK_FORMAT_ASTC_10x10_SRGB_BLOCK: VkFormat = 180;
pub const VK_FORMAT_ASTC_12x10_UNORM_BLOCK: VkFormat = 181;
pub const VK_FORMAT_ASTC_12x10_SRGB_BLOCK: VkFormat = 182;
pub const VK_FORMAT_ASTC_12x12_UNORM_BLOCK: VkFormat = 183;
pub const VK_FORMAT_ASTC_12x12_SRGB_BLOCK: VkFormat = 184;
pub const VK_FORMAT_PVRTC1_2BPP_UNORM_BLOCK_IMG: VkFormat = 1000054000;
pub const VK_FORMAT_PVRTC1_4BPP_UNORM_BLOCK_IMG: VkFormat = 1000054001;
pub const VK_FORMAT_PVRTC2_2BPP_UNORM_BLOCK_IMG: VkFormat = 1000054002;
pub const VK_FORMAT_PVRTC2_4BPP_UNORM_BLOCK_IMG: VkFormat = 1000054003;
pub const VK_FORMAT_PVRTC1_2BPP_SRGB_BLOCK_IMG: VkFormat = 1000054004;
pub const VK_FORMAT_PVRTC1_4BPP_SRGB_BLOCK_IMG: VkFormat = 1000054005;
pub const VK_FORMAT_PVRTC2_2BPP_SRGB_BLOCK_IMG: VkFormat = 1000054006;
pub const VK_FORMAT_PVRTC2_4BPP_SRGB_BLOCK_IMG: VkFormat = 1000054007;
pub const VK_FORMAT_ASTC_4x4_SFLOAT_BLOCK: VkFormat = 1000066000;
pub const VK_FORMAT_ASTC_4x4_SFLOAT_BLOCK_EXT: VkFormat = 1000066000;
pub const VK_FORMAT_ASTC_5x4_SFLOAT_BLOCK: VkFormat = 1000066001;
pub const VK_FORMAT_ASTC_5x4_SFLOAT_BLOCK_EXT: VkFormat = 1000066001;
pub const VK_FORMAT_ASTC_5x5_SFLOAT_BLOCK: VkFormat = 1000066002;
pub const VK_FORMAT_ASTC_5x5_SFLOAT_BLOCK_EXT: VkFormat = 1000066002;
pub const VK_FORMAT_ASTC_6x5_SFLOAT_BLOCK: VkFormat = 1000066003;
pub const VK_FORMAT_ASTC_6x5_SFLOAT_BLOCK_EXT: VkFormat = 1000066003;
pub const VK_FORMAT_ASTC_6x6_SFLOAT_BLOCK: VkFormat = 1000066004;
pub const VK_FORMAT_ASTC_6x6_SFLOAT_BLOCK_EXT: VkFormat = 1000066004;
pub const VK_FORMAT_ASTC_8x5_SFLOAT_BLOCK: VkFormat = 1000066005;
pub const VK_FORMAT_ASTC_8x5_SFLOAT_BLOCK_EXT: VkFormat = 1000066005;
pub const VK_FORMAT_ASTC_8x6_SFLOAT_BLOCK: VkFormat = 1000066006;
pub const VK_FORMAT_ASTC_8x6_SFLOAT_BLOCK_EXT: VkFormat = 1000066006;
pub const VK_FORMAT_ASTC_8x8_SFLOAT_BLOCK: VkFormat = 1000066007;
pub const VK_FORMAT_ASTC_8x8_SFLOAT_BLOCK_EXT: VkFormat = 1000066007;
pub const VK_FORMAT_ASTC_10x5_SFLOAT_BLOCK: VkFormat = 1000066008;
pub const VK_FORMAT_ASTC_10x5_SFLOAT_BLOCK_EXT: VkFormat = 1000066008;
pub const VK_FORMAT_ASTC_10x6_SFLOAT_BLOCK: VkFormat = 1000066009;
pub const VK_FORMAT_ASTC_10x6_SFLOAT_BLOCK_EXT: VkFormat = 1000066009;
pub const VK_FORMAT_ASTC_10x8_SFLOAT_BLOCK: VkFormat = 1000066010;
pub const VK_FORMAT_ASTC_10x8_SFLOAT_BLOCK_EXT: VkFormat = 1000066010;
pub const VK_FORMAT_ASTC_10x10_SFLOAT_BLOCK: VkFormat = 1000066011;
pub const VK_FORMAT_ASTC_10x10_SFLOAT_BLOCK_EXT: VkFormat = 1000066011;
pub const VK_FORMAT_ASTC_12x10_SFLOAT_BLOCK: VkFormat = 1000066012;
pub const VK_FORMAT_ASTC_12x10_SFLOAT_BLOCK_EXT: VkFormat = 1000066012;
pub const VK_FORMAT_ASTC_12x12_SFLOAT_BLOCK: VkFormat = 1000066013;
pub const VK_FORMAT_ASTC_12x12_SFLOAT_BLOCK_EXT: VkFormat = 1000066013;
pub const VK_FORMAT_G8B8G8R8_422_UNORM: VkFormat = 1000156000;
pub const VK_FORMAT_G8B8G8R8_422_UNORM_KHR: VkFormat = 1000156000;
pub const VK_FORMAT_B8G8R8G8_422_UNORM: VkFormat = 1000156001;
pub const VK_FORMAT_B8G8R8G8_422_UNORM_KHR: VkFormat = 1000156001;
pub const VK_FORMAT_G8_B8_R8_3PLANE_420_UNORM: VkFormat = 1000156002;
pub const VK_FORMAT_G8_B8_R8_3PLANE_420_UNORM_KHR: VkFormat = 1000156002;
pub const VK_FORMAT_G8_B8R8_2PLANE_420_UNORM: VkFormat = 1000156003;
pub const VK_FORMAT_G8_B8R8_2PLANE_420_UNORM_KHR: VkFormat = 1000156003;
pub const VK_FORMAT_G8_B8_R8_3PLANE_422_UNORM: VkFormat = 1000156004;
pub const VK_FORMAT_G8_B8_R8_3PLANE_422_UNORM_KHR: VkFormat = 1000156004;
pub const VK_FORMAT_G8_B8R8_2PLANE_422_UNORM: VkFormat = 1000156005;
pub const VK_FORMAT_G8_B8R8_2PLANE_422_UNORM_KHR: VkFormat = 1000156005;
pub const VK_FORMAT_G8_B8_R8_3PLANE_444_UNORM: VkFormat = 1000156006;
pub const VK_FORMAT_G8_B8_R8_3PLANE_444_UNORM_KHR: VkFormat = 1000156006;
pub const VK_FORMAT_R10X6_UNORM_PACK16: VkFormat = 1000156007;
pub const VK_FORMAT_R10X6_UNORM_PACK16_KHR: VkFormat = 1000156007;
pub const VK_FORMAT_R10X6G10X6_UNORM_2PACK16: VkFormat = 1000156008;
pub const VK_FORMAT_R10X6G10X6_UNORM_2PACK16_KHR: VkFormat = 1000156008;
pub const VK_FORMAT_R10X6G10X6B10X6A10X6_UNORM_4PACK16: VkFormat = 1000156009;
pub const VK_FORMAT_R10X6G10X6B10X6A10X6_UNORM_4PACK16_KHR: VkFormat = 1000156009;
pub const VK_FORMAT_G10X6B10X6G10X6R10X6_422_UNORM_4PACK16: VkFormat = 1000156010;
pub const VK_FORMAT_G10X6B10X6G10X6R10X6_422_UNORM_4PACK16_KHR: VkFormat = 1000156010;
pub const VK_FORMAT_B10X6G10X6R10X6G10X6_422_UNORM_4PACK16: VkFormat = 1000156011;
pub const VK_FORMAT_B10X6G10X6R10X6G10X6_422_UNORM_4PACK16_KHR: VkFormat = 1000156011;
pub const VK_FORMAT_G10X6_B10X6_R10X6_3PLANE_420_UNORM_3PACK16: VkFormat = 1000156012;
pub const VK_FORMAT_G10X6_B10X6_R10X6_3PLANE_420_UNORM_3PACK16_KHR: VkFormat = 1000156012;
pub const VK_FORMAT_G10X6_B10X6R10X6_2PLANE_420_UNORM_3PACK16: VkFormat = 1000156013;
pub const VK_FORMAT_G10X6_B10X6R10X6_2PLANE_420_UNORM_3PACK16_KHR: VkFormat = 1000156013;
pub const VK_FORMAT_G10X6_B10X6_R10X6_3PLANE_422_UNORM_3PACK16: VkFormat = 1000156014;
pub const VK_FORMAT_G10X6_B10X6_R10X6_3PLANE_422_UNORM_3PACK16_KHR: VkFormat = 1000156014;
pub const VK_FORMAT_G10X6_B10X6R10X6_2PLANE_422_UNORM_3PACK16: VkFormat = 1000156015;
pub const VK_FORMAT_G10X6_B10X6R10X6_2PLANE_422_UNORM_3PACK16_KHR: VkFormat = 1000156015;
pub const VK_FORMAT_G10X6_B10X6_R10X6_3PLANE_444_UNORM_3PACK16: VkFormat = 1000156016;
pub const VK_FORMAT_G10X6_B10X6_R10X6_3PLANE_444_UNORM_3PACK16_KHR: VkFormat = 1000156016;
pub const VK_FORMAT_R12X4_UNORM_PACK16: VkFormat = 1000156017;
pub const VK_FORMAT_R12X4_UNORM_PACK16_KHR: VkFormat = 1000156017;
pub const VK_FORMAT_R12X4G12X4_UNORM_2PACK16: VkFormat = 1000156018;
pub const VK_FORMAT_R12X4G12X4_UNORM_2PACK16_KHR: VkFormat = 1000156018;
pub const VK_FORMAT_R12X4G12X4B12X4A12X4_UNORM_4PACK16: VkFormat = 1000156019;
pub const VK_FORMAT_R12X4G12X4B12X4A12X4_UNORM_4PACK16_KHR: VkFormat = 1000156019;
pub const VK_FORMAT_G12X4B12X4G12X4R12X4_422_UNORM_4PACK16: VkFormat = 1000156020;
pub const VK_FORMAT_G12X4B12X4G12X4R12X4_422_UNORM_4PACK16_KHR: VkFormat = 1000156020;
pub const VK_FORMAT_B12X4G12X4R12X4G12X4_422_UNORM_4PACK16: VkFormat = 1000156021;
pub const VK_FORMAT_B12X4G12X4R12X4G12X4_422_UNORM_4PACK16_KHR: VkFormat = 1000156021;
pub const VK_FORMAT_G12X4_B12X4_R12X4_3PLANE_420_UNORM_3PACK16: VkFormat = 1000156022;
pub const VK_FORMAT_G12X4_B12X4_R12X4_3PLANE_420_UNORM_3PACK16_KHR: VkFormat = 1000156022;
pub const VK_FORMAT_G12X4_B12X4R12X4_2PLANE_420_UNORM_3PACK16: VkFormat = 1000156023;
pub const VK_FORMAT_G12X4_B12X4R12X4_2PLANE_420_UNORM_3PACK16_KHR: VkFormat = 1000156023;
pub const VK_FORMAT_G12X4_B12X4_R12X4_3PLANE_422_UNORM_3PACK16: VkFormat = 1000156024;
pub const VK_FORMAT_G12X4_B12X4_R12X4_3PLANE_422_UNORM_3PACK16_KHR: VkFormat = 1000156024;
pub const VK_FORMAT_G12X4_B12X4R12X4_2PLANE_422_UNORM_3PACK16: VkFormat = 1000156025;
pub const VK_FORMAT_G12X4_B12X4R12X4_2PLANE_422_UNORM_3PACK16_KHR: VkFormat = 1000156025;
pub const VK_FORMAT_G12X4_B12X4_R12X4_3PLANE_444_UNORM_3PACK16: VkFormat = 1000156026;
pub const VK_FORMAT_G12X4_B12X4_R12X4_3PLANE_444_UNORM_3PACK16_KHR: VkFormat = 1000156026;
pub const VK_FORMAT_G16B16G16R16_422_UNORM: VkFormat = 1000156027;
pub const VK_FORMAT_G16B16G16R16_422_UNORM_KHR: VkFormat = 1000156027;
pub const VK_FORMAT_B16G16R16G16_422_UNORM: VkFormat = 1000156028;
pub const VK_FORMAT_B16G16R16G16_422_UNORM_KHR: VkFormat = 1000156028;
pub const VK_FORMAT_G16_B16_R16_3PLANE_420_UNORM: VkFormat = 1000156029;
pub const VK_FORMAT_G16_B16_R16_3PLANE_420_UNORM_KHR: VkFormat = 1000156029;
pub const VK_FORMAT_G16_B16R16_2PLANE_420_UNORM: VkFormat = 1000156030;
pub const VK_FORMAT_G16_B16R16_2PLANE_420_UNORM_KHR: VkFormat = 1000156030;
pub const VK_FORMAT_G16_B16_R16_3PLANE_422_UNORM: VkFormat = 1000156031;
pub const VK_FORMAT_G16_B16_R16_3PLANE_422_UNORM_KHR: VkFormat = 1000156031;
pub const VK_FORMAT_G16_B16R16_2PLANE_422_UNORM: VkFormat = 1000156032;
pub const VK_FORMAT_G16_B16R16_2PLANE_422_UNORM_KHR: VkFormat = 1000156032;
pub const VK_FORMAT_G16_B16_R16_3PLANE_444_UNORM: VkFormat = 1000156033;
pub const VK_FORMAT_G16_B16_R16_3PLANE_444_UNORM_KHR: VkFormat = 1000156033;
pub const VK_FORMAT_ASTC_3x3x3_UNORM_BLOCK_EXT: VkFormat = 1000288000;
pub const VK_FORMAT_ASTC_3x3x3_SRGB_BLOCK_EXT: VkFormat = 1000288001;
pub const VK_FORMAT_ASTC_3x3x3_SFLOAT_BLOCK_EXT: VkFormat = 1000288002;
pub const VK_FORMAT_ASTC_4x3x3_UNORM_BLOCK_EXT: VkFormat = 1000288003;
pub const VK_FORMAT_ASTC_4x3x3_SRGB_BLOCK_EXT: VkFormat = 1000288004;
pub const VK_FORMAT_ASTC_4x3x3_SFLOAT_BLOCK_EXT: VkFormat = 1000288005;
pub const VK_FORMAT_ASTC_4x4x3_UNORM_BLOCK_EXT: VkFormat = 1000288006;
pub const VK_FORMAT_ASTC_4x4x3_SRGB_BLOCK_EXT: VkFormat = 1000288007;
pub const VK_FORMAT_ASTC_4x4x3_SFLOAT_BLOCK_EXT: VkFormat = 1000288008;
pub const VK_FORMAT_ASTC_4x4x4_UNORM_BLOCK_EXT: VkFormat = 1000288009;
pub const VK_FORMAT_ASTC_4x4x4_SRGB_BLOCK_EXT: VkFormat = 1000288010;
pub const VK_FORMAT_ASTC_4x4x4_SFLOAT_BLOCK_EXT: VkFormat = 1000288011;
pub const VK_FORMAT_ASTC_5x4x4_UNORM_BLOCK_EXT: VkFormat = 1000288012;
pub const VK_FORMAT_ASTC_5x4x4_SRGB_BLOCK_EXT: VkFormat = 1000288013;
pub const VK_FORMAT_ASTC_5x4x4_SFLOAT_BLOCK_EXT: VkFormat = 1000288014;
pub const VK_FORMAT_ASTC_5x5x4_UNORM_BLOCK_EXT: VkFormat = 1000288015;
pub const VK_FORMAT_ASTC_5x5x4_SRGB_BLOCK_EXT: VkFormat = 1000288016;
pub const VK_FORMAT_ASTC_5x5x4_SFLOAT_BLOCK_EXT: VkFormat = 1000288017;
pub const VK_FORMAT_ASTC_5x5x5_UNORM_BLOCK_EXT: VkFormat = 1000288018;
pub const VK_FORMAT_ASTC_5x5x5_SRGB_BLOCK_EXT: VkFormat = 1000288019;
pub const VK_FORMAT_ASTC_5x5x5_SFLOAT_BLOCK_EXT: VkFormat = 1000288020;
pub const VK_FORMAT_ASTC_6x5x5_UNORM_BLOCK_EXT: VkFormat = 1000288021;
pub const VK_FORMAT_ASTC_6x5x5_SRGB_BLOCK_EXT: VkFormat = 1000288022;
pub const VK_FORMAT_ASTC_6x5x5_SFLOAT_BLOCK_EXT: VkFormat = 1000288023;
pub const VK_FORMAT_ASTC_6x6x5_UNORM_BLOCK_EXT: VkFormat = 1000288024;
pub const VK_FORMAT_ASTC_6x6x5_SRGB_BLOCK_EXT: VkFormat = 1000288025;
pub const VK_FORMAT_ASTC_6x6x5_SFLOAT_BLOCK_EXT: VkFormat = 1000288026;
pub const VK_FORMAT_ASTC_6x6x6_UNORM_BLOCK_EXT: VkFormat = 1000288027;
pub const VK_FORMAT_ASTC_6x6x6_SRGB_BLOCK_EXT: VkFormat = 1000288028;
pub const VK_FORMAT_ASTC_6x6x6_SFLOAT_BLOCK_EXT: VkFormat = 1000288029;
pub const VK_FORMAT_G8_B8R8_2PLANE_444_UNORM: VkFormat = 1000330000;
pub const VK_FORMAT_G8_B8R8_2PLANE_444_UNORM_EXT: VkFormat = 1000330000;
pub const VK_FORMAT_G10X6_B10X6R10X6_2PLANE_444_UNORM_3PACK16: VkFormat = 1000330001;
pub const VK_FORMAT_G10X6_B10X6R10X6_2PLANE_444_UNORM_3PACK16_EXT: VkFormat = 1000330001;
pub const VK_FORMAT_G12X4_B12X4R12X4_2PLANE_444_UNORM_3PACK16: VkFormat = 1000330002;
pub const VK_FORMAT_G12X4_B12X4R12X4_2PLANE_444_UNORM_3PACK16_EXT: VkFormat = 1000330002;
pub const VK_FORMAT_G16_B16R16_2PLANE_444_UNORM: VkFormat = 1000330003;
pub const VK_FORMAT_G16_B16R16_2PLANE_444_UNORM_EXT: VkFormat = 1000330003;
pub const VK_FORMAT_A4R4G4B4_UNORM_PACK16: VkFormat = 1000340000;
pub const VK_FORMAT_A4R4G4B4_UNORM_PACK16_EXT: VkFormat = 1000340000;
pub const VK_FORMAT_A4B4G4R4_UNORM_PACK16: VkFormat = 1000340001;
pub const VK_FORMAT_A4B4G4R4_UNORM_PACK16_EXT: VkFormat = 1000340001;
pub const VK_FORMAT_R8_BOOL_ARM: VkFormat = 1000460000;
pub const VK_FORMAT_R16_SFLOAT_FPENCODING_BFLOAT16_ARM: VkFormat = 1000460001;
pub const VK_FORMAT_R8_SFLOAT_FPENCODING_FLOAT8E4M3_ARM: VkFormat = 1000460002;
pub const VK_FORMAT_R8_SFLOAT_FPENCODING_FLOAT8E5M2_ARM: VkFormat = 1000460003;
pub const VK_FORMAT_R16G16_S10_5_NV: VkFormat = 1000464000;
pub const VK_FORMAT_R16G16_SFIXED5_NV: VkFormat = 1000464000;
pub const VK_FORMAT_A1B5G5R5_UNORM_PACK16: VkFormat = 1000470000;
pub const VK_FORMAT_A1B5G5R5_UNORM_PACK16_KHR: VkFormat = 1000470000;
pub const VK_FORMAT_A8_UNORM: VkFormat = 1000470001;
pub const VK_FORMAT_A8_UNORM_KHR: VkFormat = 1000470001;
pub const VK_FORMAT_R10X6_UINT_PACK16_ARM: VkFormat = 1000609000;
pub const VK_FORMAT_R10X6G10X6_UINT_2PACK16_ARM: VkFormat = 1000609001;
pub const VK_FORMAT_R10X6G10X6B10X6A10X6_UINT_4PACK16_ARM: VkFormat = 1000609002;
pub const VK_FORMAT_R12X4_UINT_PACK16_ARM: VkFormat = 1000609003;
pub const VK_FORMAT_R12X4G12X4_UINT_2PACK16_ARM: VkFormat = 1000609004;
pub const VK_FORMAT_R12X4G12X4B12X4A12X4_UINT_4PACK16_ARM: VkFormat = 1000609005;
pub const VK_FORMAT_R14X2_UINT_PACK16_ARM: VkFormat = 1000609006;
pub const VK_FORMAT_R14X2G14X2_UINT_2PACK16_ARM: VkFormat = 1000609007;
pub const VK_FORMAT_R14X2G14X2B14X2A14X2_UINT_4PACK16_ARM: VkFormat = 1000609008;
pub const VK_FORMAT_R14X2_UNORM_PACK16_ARM: VkFormat = 1000609009;
pub const VK_FORMAT_R14X2G14X2_UNORM_2PACK16_ARM: VkFormat = 1000609010;
pub const VK_FORMAT_R14X2G14X2B14X2A14X2_UNORM_4PACK16_ARM: VkFormat = 1000609011;
pub const VK_FORMAT_G14X2_B14X2R14X2_2PLANE_420_UNORM_3PACK16_ARM: VkFormat = 1000609012;
pub const VK_FORMAT_G14X2_B14X2R14X2_2PLANE_422_UNORM_3PACK16_ARM: VkFormat = 1000609013;

// VkFormatFeatureFlagBits
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_BIT: VkFormatFeatureFlagBits = 1;
pub const VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT: VkFormatFeatureFlagBits = 2;
pub const VK_FORMAT_FEATURE_STORAGE_IMAGE_ATOMIC_BIT: VkFormatFeatureFlagBits = 4;
pub const VK_FORMAT_FEATURE_UNIFORM_TEXEL_BUFFER_BIT: VkFormatFeatureFlagBits = 8;
pub const VK_FORMAT_FEATURE_STORAGE_TEXEL_BUFFER_BIT: VkFormatFeatureFlagBits = 0x10;
pub const VK_FORMAT_FEATURE_STORAGE_TEXEL_BUFFER_ATOMIC_BIT: VkFormatFeatureFlagBits = 0x20;
pub const VK_FORMAT_FEATURE_VERTEX_BUFFER_BIT: VkFormatFeatureFlagBits = 0x40;
pub const VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BIT: VkFormatFeatureFlagBits = 0x80;
pub const VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BLEND_BIT: VkFormatFeatureFlagBits = 0x100;
pub const VK_FORMAT_FEATURE_DEPTH_STENCIL_ATTACHMENT_BIT: VkFormatFeatureFlagBits = 0x200;
pub const VK_FORMAT_FEATURE_BLIT_SRC_BIT: VkFormatFeatureFlagBits = 0x400;
pub const VK_FORMAT_FEATURE_BLIT_DST_BIT: VkFormatFeatureFlagBits = 0x800;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_LINEAR_BIT: VkFormatFeatureFlagBits = 0x1000;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_CUBIC_BIT_EXT: VkFormatFeatureFlagBits = 0x2000;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_CUBIC_BIT_IMG: VkFormatFeatureFlagBits = 0x2000;
pub const VK_FORMAT_FEATURE_TRANSFER_SRC_BIT: VkFormatFeatureFlagBits = 0x4000;
pub const VK_FORMAT_FEATURE_TRANSFER_SRC_BIT_KHR: VkFormatFeatureFlagBits = 0x4000;
pub const VK_FORMAT_FEATURE_TRANSFER_DST_BIT: VkFormatFeatureFlagBits = 0x8000;
pub const VK_FORMAT_FEATURE_TRANSFER_DST_BIT_KHR: VkFormatFeatureFlagBits = 0x8000;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_MINMAX_BIT: VkFormatFeatureFlagBits = 0x10000;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_MINMAX_BIT_EXT: VkFormatFeatureFlagBits = 0x10000;
pub const VK_FORMAT_FEATURE_MIDPOINT_CHROMA_SAMPLES_BIT: VkFormatFeatureFlagBits = 0x20000;
pub const VK_FORMAT_FEATURE_MIDPOINT_CHROMA_SAMPLES_BIT_KHR: VkFormatFeatureFlagBits = 0x20000;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_YCBCR_CONVERSION_LINEAR_FILTER_BIT: VkFormatFeatureFlagBits = 0x40000;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_YCBCR_CONVERSION_LINEAR_FILTER_BIT_KHR: VkFormatFeatureFlagBits = 0x40000;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_YCBCR_CONVERSION_SEPARATE_RECONSTRUCTION_FILTER_BIT: VkFormatFeatureFlagBits = 0x80000;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_YCBCR_CONVERSION_SEPARATE_RECONSTRUCTION_FILTER_BIT_KHR: VkFormatFeatureFlagBits = 0x80000;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_YCBCR_CONVERSION_CHROMA_RECONSTRUCTION_EXPLICIT_BIT: VkFormatFeatureFlagBits = 0x100000;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_YCBCR_CONVERSION_CHROMA_RECONSTRUCTION_EXPLICIT_BIT_KHR: VkFormatFeatureFlagBits = 0x100000;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_YCBCR_CONVERSION_CHROMA_RECONSTRUCTION_EXPLICIT_FORCEABLE_BIT: VkFormatFeatureFlagBits = 0x200000;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_YCBCR_CONVERSION_CHROMA_RECONSTRUCTION_EXPLICIT_FORCEABLE_BIT_KHR: VkFormatFeatureFlagBits = 0x200000;
pub const VK_FORMAT_FEATURE_DISJOINT_BIT: VkFormatFeatureFlagBits = 0x400000;
pub const VK_FORMAT_FEATURE_DISJOINT_BIT_KHR: VkFormatFeatureFlagBits = 0x400000;
pub const VK_FORMAT_FEATURE_COSITED_CHROMA_SAMPLES_BIT: VkFormatFeatureFlagBits = 0x800000;
pub const VK_FORMAT_FEATURE_COSITED_CHROMA_SAMPLES_BIT_KHR: VkFormatFeatureFlagBits = 0x800000;
pub const VK_FORMAT_FEATURE_FRAGMENT_DENSITY_MAP_BIT_EXT: VkFormatFeatureFlagBits = 0x1000000;
pub const VK_FORMAT_FEATURE_VIDEO_DECODE_OUTPUT_BIT_KHR: VkFormatFeatureFlagBits = 0x2000000;
pub const VK_FORMAT_FEATURE_VIDEO_DECODE_DPB_BIT_KHR: VkFormatFeatureFlagBits = 0x4000000;
pub const VK_FORMAT_FEATURE_VIDEO_ENCODE_INPUT_BIT_KHR: VkFormatFeatureFlagBits = 0x8000000;
pub const VK_FORMAT_FEATURE_VIDEO_ENCODE_DPB_BIT_KHR: VkFormatFeatureFlagBits = 0x10000000;
pub const VK_FORMAT_FEATURE_ACCELERATION_STRUCTURE_VERTEX_BUFFER_BIT_KHR: VkFormatFeatureFlagBits = 0x20000000;
pub const VK_FORMAT_FEATURE_FRAGMENT_SHADING_RATE_ATTACHMENT_BIT_KHR: VkFormatFeatureFlagBits = 0x40000000;

// VkImageAspectFlagBits
pub const VK_IMAGE_ASPECT_NONE: VkImageAspectFlagBits = 0;
pub const VK_IMAGE_ASPECT_NONE_KHR: VkImageAspectFlagBits = 0;
pub const VK_IMAGE_ASPECT_COLOR_BIT: VkImageAspectFlagBits = 1;
pub const VK_IMAGE_ASPECT_DEPTH_BIT: VkImageAspectFlagBits = 2;
pub const VK_IMAGE_ASPECT_STENCIL_BIT: VkImageAspectFlagBits = 4;
pub const VK_IMAGE_ASPECT_METADATA_BIT: VkImageAspectFlagBits = 8;
pub const VK_IMAGE_ASPECT_PLANE_0_BIT: VkImageAspectFlagBits = 0x10;
pub const VK_IMAGE_ASPECT_PLANE_0_BIT_KHR: VkImageAspectFlagBits = 0x10;
pub const VK_IMAGE_ASPECT_PLANE_1_BIT: VkImageAspectFlagBits = 0x20;
pub const VK_IMAGE_ASPECT_PLANE_1_BIT_KHR: VkImageAspectFlagBits = 0x20;
pub const VK_IMAGE_ASPECT_PLANE_2_BIT: VkImageAspectFlagBits = 0x40;
pub const VK_IMAGE_ASPECT_PLANE_2_BIT_KHR: VkImageAspectFlagBits = 0x40;
pub const VK_IMAGE_ASPECT_MEMORY_PLANE_0_BIT_EXT: VkImageAspectFlagBits = 0x80;
pub const VK_IMAGE_ASPECT_MEMORY_PLANE_1_BIT_EXT: VkImageAspectFlagBits = 0x100;
pub const VK_IMAGE_ASPECT_MEMORY_PLANE_2_BIT_EXT: VkImageAspectFlagBits = 0x200;
pub const VK_IMAGE_ASPECT_MEMORY_PLANE_3_BIT_EXT: VkImageAspectFlagBits = 0x400;

// VkImageCreateFlagBits
pub const VK_IMAGE_CREATE_SPARSE_BINDING_BIT: VkImageCreateFlagBits = 1;
pub const VK_IMAGE_CREATE_SPARSE_RESIDENCY_BIT: VkImageCreateFlagBits = 2;
pub const VK_IMAGE_CREATE_SPARSE_ALIASED_BIT: VkImageCreateFlagBits = 4;
pub const VK_IMAGE_CREATE_MUTABLE_FORMAT_BIT: VkImageCreateFlagBits = 8;
pub const VK_IMAGE_CREATE_CUBE_COMPATIBLE_BIT: VkImageCreateFlagBits = 0x10;
pub const VK_IMAGE_CREATE_2D_ARRAY_COMPATIBLE_BIT: VkImageCreateFlagBits = 0x20;
pub const VK_IMAGE_CREATE_2D_ARRAY_COMPATIBLE_BIT_KHR: VkImageCreateFlagBits = 0x20;
pub const VK_IMAGE_CREATE_SPLIT_INSTANCE_BIND_REGIONS_BIT: VkImageCreateFlagBits = 0x40;
pub const VK_IMAGE_CREATE_SPLIT_INSTANCE_BIND_REGIONS_BIT_KHR: VkImageCreateFlagBits = 0x40;
pub const VK_IMAGE_CREATE_BLOCK_TEXEL_VIEW_COMPATIBLE_BIT: VkImageCreateFlagBits = 0x80;
pub const VK_IMAGE_CREATE_BLOCK_TEXEL_VIEW_COMPATIBLE_BIT_KHR: VkImageCreateFlagBits = 0x80;
pub const VK_IMAGE_CREATE_EXTENDED_USAGE_BIT: VkImageCreateFlagBits = 0x100;
pub const VK_IMAGE_CREATE_EXTENDED_USAGE_BIT_KHR: VkImageCreateFlagBits = 0x100;
pub const VK_IMAGE_CREATE_DISJOINT_BIT: VkImageCreateFlagBits = 0x200;
pub const VK_IMAGE_CREATE_DISJOINT_BIT_KHR: VkImageCreateFlagBits = 0x200;
pub const VK_IMAGE_CREATE_ALIAS_BIT: VkImageCreateFlagBits = 0x400;
pub const VK_IMAGE_CREATE_ALIAS_BIT_KHR: VkImageCreateFlagBits = 0x400;
pub const VK_IMAGE_CREATE_PROTECTED_BIT: VkImageCreateFlagBits = 0x800;
pub const VK_IMAGE_CREATE_SAMPLE_LOCATIONS_COMPATIBLE_DEPTH_BIT_EXT: VkImageCreateFlagBits = 0x1000;
pub const VK_IMAGE_CREATE_CORNER_SAMPLED_BIT_NV: VkImageCreateFlagBits = 0x2000;
pub const VK_IMAGE_CREATE_SUBSAMPLED_BIT_EXT: VkImageCreateFlagBits = 0x4000;
pub const VK_IMAGE_CREATE_FRAGMENT_DENSITY_MAP_OFFSET_BIT_EXT: VkImageCreateFlagBits = 0x8000;
pub const VK_IMAGE_CREATE_FRAGMENT_DENSITY_MAP_OFFSET_BIT_QCOM: VkImageCreateFlagBits = 0x8000;
pub const VK_IMAGE_CREATE_DESCRIPTOR_BUFFER_CAPTURE_REPLAY_BIT_EXT: VkImageCreateFlagBits = 0x10000;
pub const VK_IMAGE_CREATE_DESCRIPTOR_HEAP_CAPTURE_REPLAY_BIT_EXT: VkImageCreateFlagBits = 0x10000;
pub const VK_IMAGE_CREATE_2D_VIEW_COMPATIBLE_BIT_EXT: VkImageCreateFlagBits = 0x20000;
pub const VK_IMAGE_CREATE_MULTISAMPLED_RENDER_TO_SINGLE_SAMPLED_BIT_EXT: VkImageCreateFlagBits = 0x40000;
pub const VK_IMAGE_CREATE_VIDEO_PROFILE_INDEPENDENT_BIT_KHR: VkImageCreateFlagBits = 0x100000;
pub const VK_IMAGE_CREATE_ALIAS_SINGLE_LAYER_DESCRIPTOR_BIT_KHR: VkImageCreateFlagBits = 0x400000;

// VkImageLayout
pub const VK_IMAGE_LAYOUT_UNDEFINED: VkImageLayout = 0;
pub const VK_IMAGE_LAYOUT_GENERAL: VkImageLayout = 1;
pub const VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL: VkImageLayout = 2;
pub const VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL: VkImageLayout = 3;
pub const VK_IMAGE_LAYOUT_DEPTH_STENCIL_READ_ONLY_OPTIMAL: VkImageLayout = 4;
pub const VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL: VkImageLayout = 5;
pub const VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL: VkImageLayout = 6;
pub const VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL: VkImageLayout = 7;
pub const VK_IMAGE_LAYOUT_PREINITIALIZED: VkImageLayout = 8;
pub const VK_IMAGE_LAYOUT_PRESENT_SRC_KHR: VkImageLayout = 1000001002;
pub const VK_IMAGE_LAYOUT_VIDEO_DECODE_DST_KHR: VkImageLayout = 1000024000;
pub const VK_IMAGE_LAYOUT_VIDEO_DECODE_SRC_KHR: VkImageLayout = 1000024001;
pub const VK_IMAGE_LAYOUT_VIDEO_DECODE_DPB_KHR: VkImageLayout = 1000024002;
pub const VK_IMAGE_LAYOUT_SHARED_PRESENT_KHR: VkImageLayout = 1000111000;
pub const VK_IMAGE_LAYOUT_DEPTH_READ_ONLY_STENCIL_ATTACHMENT_OPTIMAL: VkImageLayout = 1000117000;
pub const VK_IMAGE_LAYOUT_DEPTH_READ_ONLY_STENCIL_ATTACHMENT_OPTIMAL_KHR: VkImageLayout = 1000117000;
pub const VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_STENCIL_READ_ONLY_OPTIMAL: VkImageLayout = 1000117001;
pub const VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_STENCIL_READ_ONLY_OPTIMAL_KHR: VkImageLayout = 1000117001;
pub const VK_IMAGE_LAYOUT_FRAGMENT_SHADING_RATE_ATTACHMENT_OPTIMAL_KHR: VkImageLayout = 1000164003;
pub const VK_IMAGE_LAYOUT_SHADING_RATE_OPTIMAL_NV: VkImageLayout = 1000164003;
pub const VK_IMAGE_LAYOUT_FRAGMENT_DENSITY_MAP_OPTIMAL_EXT: VkImageLayout = 1000218000;
pub const VK_IMAGE_LAYOUT_RENDERING_LOCAL_READ: VkImageLayout = 1000232000;
pub const VK_IMAGE_LAYOUT_RENDERING_LOCAL_READ_KHR: VkImageLayout = 1000232000;
pub const VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL: VkImageLayout = 1000241000;
pub const VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL_KHR: VkImageLayout = 1000241000;
pub const VK_IMAGE_LAYOUT_DEPTH_READ_ONLY_OPTIMAL: VkImageLayout = 1000241001;
pub const VK_IMAGE_LAYOUT_DEPTH_READ_ONLY_OPTIMAL_KHR: VkImageLayout = 1000241001;
pub const VK_IMAGE_LAYOUT_STENCIL_ATTACHMENT_OPTIMAL: VkImageLayout = 1000241002;
pub const VK_IMAGE_LAYOUT_STENCIL_ATTACHMENT_OPTIMAL_KHR: VkImageLayout = 1000241002;
pub const VK_IMAGE_LAYOUT_STENCIL_READ_ONLY_OPTIMAL: VkImageLayout = 1000241003;
pub const VK_IMAGE_LAYOUT_STENCIL_READ_ONLY_OPTIMAL_KHR: VkImageLayout = 1000241003;
pub const VK_IMAGE_LAYOUT_VIDEO_ENCODE_DST_KHR: VkImageLayout = 1000299000;
pub const VK_IMAGE_LAYOUT_VIDEO_ENCODE_SRC_KHR: VkImageLayout = 1000299001;
pub const VK_IMAGE_LAYOUT_VIDEO_ENCODE_DPB_KHR: VkImageLayout = 1000299002;
pub const VK_IMAGE_LAYOUT_READ_ONLY_OPTIMAL: VkImageLayout = 1000314000;
pub const VK_IMAGE_LAYOUT_READ_ONLY_OPTIMAL_KHR: VkImageLayout = 1000314000;
pub const VK_IMAGE_LAYOUT_ATTACHMENT_OPTIMAL: VkImageLayout = 1000314001;
pub const VK_IMAGE_LAYOUT_ATTACHMENT_OPTIMAL_KHR: VkImageLayout = 1000314001;
pub const VK_IMAGE_LAYOUT_ATTACHMENT_FEEDBACK_LOOP_OPTIMAL_EXT: VkImageLayout = 1000339000;
pub const VK_IMAGE_LAYOUT_TENSOR_ALIASING_ARM: VkImageLayout = 1000460000;
pub const VK_IMAGE_LAYOUT_VIDEO_ENCODE_QUANTIZATION_MAP_KHR: VkImageLayout = 1000553000;
pub const VK_IMAGE_LAYOUT_ZERO_INITIALIZED_EXT: VkImageLayout = 1000620000;

// VkImageTiling
pub const VK_IMAGE_TILING_OPTIMAL: VkImageTiling = 0;
pub const VK_IMAGE_TILING_LINEAR: VkImageTiling = 1;
pub const VK_IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT: VkImageTiling = 1000158000;

// VkImageType
pub const VK_IMAGE_TYPE_1D: VkImageType = 0;
pub const VK_IMAGE_TYPE_2D: VkImageType = 1;
pub const VK_IMAGE_TYPE_3D: VkImageType = 2;

// VkImageUsageFlagBits
pub const VK_IMAGE_USAGE_TRANSFER_SRC_BIT: VkImageUsageFlagBits = 1;
pub const VK_IMAGE_USAGE_TRANSFER_DST_BIT: VkImageUsageFlagBits = 2;
pub const VK_IMAGE_USAGE_SAMPLED_BIT: VkImageUsageFlagBits = 4;
pub const VK_IMAGE_USAGE_STORAGE_BIT: VkImageUsageFlagBits = 8;
pub const VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT: VkImageUsageFlagBits = 0x10;
pub const VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT: VkImageUsageFlagBits = 0x20;
pub const VK_IMAGE_USAGE_TRANSIENT_ATTACHMENT_BIT: VkImageUsageFlagBits = 0x40;
pub const VK_IMAGE_USAGE_INPUT_ATTACHMENT_BIT: VkImageUsageFlagBits = 0x80;
pub const VK_IMAGE_USAGE_FRAGMENT_SHADING_RATE_ATTACHMENT_BIT_KHR: VkImageUsageFlagBits = 0x100;
pub const VK_IMAGE_USAGE_SHADING_RATE_IMAGE_BIT_NV: VkImageUsageFlagBits = 0x100;
pub const VK_IMAGE_USAGE_FRAGMENT_DENSITY_MAP_BIT_EXT: VkImageUsageFlagBits = 0x200;
pub const VK_IMAGE_USAGE_VIDEO_DECODE_DST_BIT_KHR: VkImageUsageFlagBits = 0x400;
pub const VK_IMAGE_USAGE_VIDEO_DECODE_SRC_BIT_KHR: VkImageUsageFlagBits = 0x800;
pub const VK_IMAGE_USAGE_VIDEO_DECODE_DPB_BIT_KHR: VkImageUsageFlagBits = 0x1000;
pub const VK_IMAGE_USAGE_VIDEO_ENCODE_DST_BIT_KHR: VkImageUsageFlagBits = 0x2000;
pub const VK_IMAGE_USAGE_VIDEO_ENCODE_SRC_BIT_KHR: VkImageUsageFlagBits = 0x4000;
pub const VK_IMAGE_USAGE_VIDEO_ENCODE_DPB_BIT_KHR: VkImageUsageFlagBits = 0x8000;
pub const VK_IMAGE_USAGE_INVOCATION_MASK_BIT_HUAWEI: VkImageUsageFlagBits = 0x40000;
pub const VK_IMAGE_USAGE_ATTACHMENT_FEEDBACK_LOOP_BIT_EXT: VkImageUsageFlagBits = 0x80000;
pub const VK_IMAGE_USAGE_SAMPLE_WEIGHT_BIT_QCOM: VkImageUsageFlagBits = 0x100000;
pub const VK_IMAGE_USAGE_SAMPLE_BLOCK_MATCH_BIT_QCOM: VkImageUsageFlagBits = 0x200000;
pub const VK_IMAGE_USAGE_HOST_TRANSFER_BIT: VkImageUsageFlagBits = 0x400000;
pub const VK_IMAGE_USAGE_HOST_TRANSFER_BIT_EXT: VkImageUsageFlagBits = 0x400000;
pub const VK_IMAGE_USAGE_TENSOR_ALIASING_BIT_ARM: VkImageUsageFlagBits = 0x800000;
pub const VK_IMAGE_USAGE_VIDEO_ENCODE_QUANTIZATION_DELTA_MAP_BIT_KHR: VkImageUsageFlagBits = 0x2000000;
pub const VK_IMAGE_USAGE_VIDEO_ENCODE_EMPHASIS_MAP_BIT_KHR: VkImageUsageFlagBits = 0x4000000;
pub const VK_IMAGE_USAGE_TILE_MEMORY_BIT_QCOM: VkImageUsageFlagBits = 0x8000000;

// VkInstanceCreateFlagBits
pub const VK_INSTANCE_CREATE_ENUMERATE_PORTABILITY_BIT_KHR: VkInstanceCreateFlagBits = 1;

// VkMemoryHeapFlagBits
pub const VK_MEMORY_HEAP_DEVICE_LOCAL_BIT: VkMemoryHeapFlagBits = 1;
pub const VK_MEMORY_HEAP_MULTI_INSTANCE_BIT: VkMemoryHeapFlagBits = 2;
pub const VK_MEMORY_HEAP_MULTI_INSTANCE_BIT_KHR: VkMemoryHeapFlagBits = 2;
pub const VK_MEMORY_HEAP_TILE_MEMORY_BIT_QCOM: VkMemoryHeapFlagBits = 8;

// VkMemoryPropertyFlagBits
pub const VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT: VkMemoryPropertyFlagBits = 1;
pub const VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT: VkMemoryPropertyFlagBits = 2;
pub const VK_MEMORY_PROPERTY_HOST_COHERENT_BIT: VkMemoryPropertyFlagBits = 4;
pub const VK_MEMORY_PROPERTY_HOST_CACHED_BIT: VkMemoryPropertyFlagBits = 8;
pub const VK_MEMORY_PROPERTY_LAZILY_ALLOCATED_BIT: VkMemoryPropertyFlagBits = 0x10;
pub const VK_MEMORY_PROPERTY_PROTECTED_BIT: VkMemoryPropertyFlagBits = 0x20;
pub const VK_MEMORY_PROPERTY_DEVICE_COHERENT_BIT_AMD: VkMemoryPropertyFlagBits = 0x40;
pub const VK_MEMORY_PROPERTY_DEVICE_UNCACHED_BIT_AMD: VkMemoryPropertyFlagBits = 0x80;
pub const VK_MEMORY_PROPERTY_RDMA_CAPABLE_BIT_NV: VkMemoryPropertyFlagBits = 0x100;

// VkPhysicalDeviceType
pub const VK_PHYSICAL_DEVICE_TYPE_OTHER: VkPhysicalDeviceType = 0;
pub const VK_PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU: VkPhysicalDeviceType = 1;
pub const VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU: VkPhysicalDeviceType = 2;
pub const VK_PHYSICAL_DEVICE_TYPE_VIRTUAL_GPU: VkPhysicalDeviceType = 3;
pub const VK_PHYSICAL_DEVICE_TYPE_CPU: VkPhysicalDeviceType = 4;

// VkPipelineBindPoint
pub const VK_PIPELINE_BIND_POINT_GRAPHICS: VkPipelineBindPoint = 0;
pub const VK_PIPELINE_BIND_POINT_COMPUTE: VkPipelineBindPoint = 1;
pub const VK_PIPELINE_BIND_POINT_EXECUTION_GRAPH_AMDX: VkPipelineBindPoint = 1000134000;
pub const VK_PIPELINE_BIND_POINT_RAY_TRACING_KHR: VkPipelineBindPoint = 1000165000;
pub const VK_PIPELINE_BIND_POINT_RAY_TRACING_NV: VkPipelineBindPoint = 1000165000;
pub const VK_PIPELINE_BIND_POINT_SUBPASS_SHADING_HUAWEI: VkPipelineBindPoint = 1000369003;
pub const VK_PIPELINE_BIND_POINT_DATA_GRAPH_ARM: VkPipelineBindPoint = 1000507000;

