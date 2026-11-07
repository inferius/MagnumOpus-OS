// Vygenerovano `cargo xtask venus-gen` z libs/venus/xml (vk.xml a venus-protocol,
// licence v libs/venus/xml/README.md) a libs/venus/subset.txt. Neupravovat rucne.

use alloc::vec::Vec;

use super::*;
use crate::cs::{CsError, Decoder, Encoder, Result};

// Skalarni typy (enum = i32, bitmaska a FlagBits = u32/u64).
pub type VkBool32 = u32;
pub type VkCommandFlagBitsEXT = u32;
pub type VkCommandFlagsEXT = u32;
pub type VkCommandTypeEXT = i32;
pub type VkDeviceSize = u64;
pub type VkInstanceCreateFlagBits = u32;
pub type VkInstanceCreateFlags = u32;
pub type VkMemoryHeapFlagBits = u32;
pub type VkMemoryHeapFlags = u32;
pub type VkMemoryPropertyFlagBits = u32;
pub type VkMemoryPropertyFlags = u32;
pub type VkPhysicalDeviceType = i32;
pub type VkQueueFlagBits = u32;
pub type VkQueueFlags = u32;
pub type VkResult = i32;
pub type VkSampleCountFlagBits = u32;
pub type VkSampleCountFlags = u32;
pub type VkStructureType = i32;

// Handly: id objektu, ktere prideluje klient (u64).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VkInstance(pub u64);
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VkPhysicalDevice(pub u64);

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

// VkCommandFlagBitsEXT
pub const VK_COMMAND_GENERATE_REPLY_BIT_EXT: VkCommandFlagBitsEXT = 1;

// VkCommandTypeEXT
pub const VK_COMMAND_TYPE_vkCreateInstance_EXT: VkCommandTypeEXT = 0;
pub const VK_COMMAND_TYPE_vkDestroyInstance_EXT: VkCommandTypeEXT = 1;
pub const VK_COMMAND_TYPE_vkEnumeratePhysicalDevices_EXT: VkCommandTypeEXT = 2;
pub const VK_COMMAND_TYPE_vkGetPhysicalDeviceProperties_EXT: VkCommandTypeEXT = 6;
pub const VK_COMMAND_TYPE_vkGetPhysicalDeviceQueueFamilyProperties_EXT: VkCommandTypeEXT = 7;
pub const VK_COMMAND_TYPE_vkGetPhysicalDeviceMemoryProperties_EXT: VkCommandTypeEXT = 8;
pub const VK_COMMAND_TYPE_vkEnumerateInstanceVersion_EXT: VkCommandTypeEXT = 137;
pub const VK_COMMAND_TYPE_vkSetReplyCommandStreamMESA_EXT: VkCommandTypeEXT = 178;
pub const VK_COMMAND_TYPE_vkSeekReplyCommandStreamMESA_EXT: VkCommandTypeEXT = 179;

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

// VkQueueFlagBits
pub const VK_QUEUE_GRAPHICS_BIT: VkQueueFlagBits = 1;
pub const VK_QUEUE_COMPUTE_BIT: VkQueueFlagBits = 2;
pub const VK_QUEUE_TRANSFER_BIT: VkQueueFlagBits = 4;
pub const VK_QUEUE_SPARSE_BINDING_BIT: VkQueueFlagBits = 8;
pub const VK_QUEUE_PROTECTED_BIT: VkQueueFlagBits = 0x10;
pub const VK_QUEUE_VIDEO_DECODE_BIT_KHR: VkQueueFlagBits = 0x20;
pub const VK_QUEUE_VIDEO_ENCODE_BIT_KHR: VkQueueFlagBits = 0x40;
pub const VK_QUEUE_OPTICAL_FLOW_BIT_NV: VkQueueFlagBits = 0x100;
pub const VK_QUEUE_DATA_GRAPH_BIT_ARM: VkQueueFlagBits = 0x400;

// VkResult
pub const VK_ERROR_NOT_ENOUGH_SPACE_KHR: VkResult = -1000483000;
pub const VK_ERROR_COMPRESSION_EXHAUSTED_EXT: VkResult = -1000338000;
pub const VK_ERROR_INVALID_VIDEO_STD_PARAMETERS_KHR: VkResult = -1000299000;
pub const VK_ERROR_INVALID_DEVICE_ADDRESS_EXT: VkResult = -1000257000;
pub const VK_ERROR_INVALID_OPAQUE_CAPTURE_ADDRESS: VkResult = -1000257000;
pub const VK_ERROR_INVALID_OPAQUE_CAPTURE_ADDRESS_KHR: VkResult = -1000257000;
pub const VK_ERROR_FULL_SCREEN_EXCLUSIVE_MODE_LOST_EXT: VkResult = -1000255000;
pub const VK_ERROR_PRESENT_TIMING_QUEUE_FULL_EXT: VkResult = -1000208000;
pub const VK_ERROR_NOT_PERMITTED: VkResult = -1000174001;
pub const VK_ERROR_NOT_PERMITTED_EXT: VkResult = -1000174001;
pub const VK_ERROR_NOT_PERMITTED_KHR: VkResult = -1000174001;
pub const VK_ERROR_FRAGMENTATION: VkResult = -1000161000;
pub const VK_ERROR_FRAGMENTATION_EXT: VkResult = -1000161000;
pub const VK_ERROR_INVALID_DRM_FORMAT_MODIFIER_PLANE_LAYOUT_EXT: VkResult = -1000158000;
pub const VK_ERROR_INVALID_EXTERNAL_HANDLE: VkResult = -1000072003;
pub const VK_ERROR_INVALID_EXTERNAL_HANDLE_KHR: VkResult = -1000072003;
pub const VK_ERROR_OUT_OF_POOL_MEMORY: VkResult = -1000069000;
pub const VK_ERROR_OUT_OF_POOL_MEMORY_KHR: VkResult = -1000069000;
pub const VK_ERROR_VIDEO_STD_VERSION_NOT_SUPPORTED_KHR: VkResult = -1000023005;
pub const VK_ERROR_VIDEO_PROFILE_CODEC_NOT_SUPPORTED_KHR: VkResult = -1000023004;
pub const VK_ERROR_VIDEO_PROFILE_FORMAT_NOT_SUPPORTED_KHR: VkResult = -1000023003;
pub const VK_ERROR_VIDEO_PROFILE_OPERATION_NOT_SUPPORTED_KHR: VkResult = -1000023002;
pub const VK_ERROR_VIDEO_PICTURE_LAYOUT_NOT_SUPPORTED_KHR: VkResult = -1000023001;
pub const VK_ERROR_IMAGE_USAGE_NOT_SUPPORTED_KHR: VkResult = -1000023000;
pub const VK_ERROR_INVALID_SHADER_NV: VkResult = -1000012000;
pub const VK_ERROR_VALIDATION_FAILED: VkResult = -1000011001;
pub const VK_ERROR_VALIDATION_FAILED_EXT: VkResult = -1000011001;
pub const VK_ERROR_INCOMPATIBLE_DISPLAY_KHR: VkResult = -1000003001;
pub const VK_ERROR_OUT_OF_DATE_KHR: VkResult = -1000001004;
pub const VK_ERROR_NATIVE_WINDOW_IN_USE_KHR: VkResult = -1000000001;
pub const VK_ERROR_SURFACE_LOST_KHR: VkResult = -1000000000;
pub const VK_ERROR_UNKNOWN: VkResult = -13;
pub const VK_ERROR_FRAGMENTED_POOL: VkResult = -12;
pub const VK_ERROR_FORMAT_NOT_SUPPORTED: VkResult = -11;
pub const VK_ERROR_TOO_MANY_OBJECTS: VkResult = -10;
pub const VK_ERROR_INCOMPATIBLE_DRIVER: VkResult = -9;
pub const VK_ERROR_FEATURE_NOT_PRESENT: VkResult = -8;
pub const VK_ERROR_EXTENSION_NOT_PRESENT: VkResult = -7;
pub const VK_ERROR_LAYER_NOT_PRESENT: VkResult = -6;
pub const VK_ERROR_MEMORY_MAP_FAILED: VkResult = -5;
pub const VK_ERROR_DEVICE_LOST: VkResult = -4;
pub const VK_ERROR_INITIALIZATION_FAILED: VkResult = -3;
pub const VK_ERROR_OUT_OF_DEVICE_MEMORY: VkResult = -2;
pub const VK_ERROR_OUT_OF_HOST_MEMORY: VkResult = -1;
pub const VK_SUCCESS: VkResult = 0;
pub const VK_NOT_READY: VkResult = 1;
pub const VK_TIMEOUT: VkResult = 2;
pub const VK_EVENT_SET: VkResult = 3;
pub const VK_EVENT_RESET: VkResult = 4;
pub const VK_INCOMPLETE: VkResult = 5;
pub const VK_SUBOPTIMAL_KHR: VkResult = 1000001003;
pub const VK_THREAD_IDLE_KHR: VkResult = 1000268000;
pub const VK_THREAD_DONE_KHR: VkResult = 1000268001;
pub const VK_OPERATION_DEFERRED_KHR: VkResult = 1000268002;
pub const VK_OPERATION_NOT_DEFERRED_KHR: VkResult = 1000268003;
pub const VK_ERROR_PIPELINE_COMPILE_REQUIRED_EXT: VkResult = 1000297000;
pub const VK_PIPELINE_COMPILE_REQUIRED: VkResult = 1000297000;
pub const VK_PIPELINE_COMPILE_REQUIRED_EXT: VkResult = 1000297000;
pub const VK_ERROR_INCOMPATIBLE_SHADER_BINARY_EXT: VkResult = 1000482000;
pub const VK_INCOMPATIBLE_SHADER_BINARY_EXT: VkResult = 1000482000;
pub const VK_PIPELINE_BINARY_MISSING_KHR: VkResult = 1000483000;

// VkSampleCountFlagBits
pub const VK_SAMPLE_COUNT_1_BIT: VkSampleCountFlagBits = 1;
pub const VK_SAMPLE_COUNT_2_BIT: VkSampleCountFlagBits = 2;
pub const VK_SAMPLE_COUNT_4_BIT: VkSampleCountFlagBits = 4;
pub const VK_SAMPLE_COUNT_8_BIT: VkSampleCountFlagBits = 8;
pub const VK_SAMPLE_COUNT_16_BIT: VkSampleCountFlagBits = 0x10;
pub const VK_SAMPLE_COUNT_32_BIT: VkSampleCountFlagBits = 0x20;
pub const VK_SAMPLE_COUNT_64_BIT: VkSampleCountFlagBits = 0x40;

// VkStructureType
pub const VK_STRUCTURE_TYPE_APPLICATION_INFO: VkStructureType = 0;
pub const VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO: VkStructureType = 1;

