//! Vlastni klient Venus: Vulkan pres virtio-gpu bez Mesy.
//!
//! Venus serializuje volani Vulkanu do proudu prikazu (format
//! venus-protocol); renderer na hostiteli (virglrenderer, render server)
//! je dekoduje a vola skutecny ovladac Vulkanu hostitele. Odpovedi pise do
//! sdilene pameti (blob zdroj namapovany do BARu virtio-gpu).
//!
//! - [`cs`] - zakladni kodovani proudu (rucne),
//! - [`vk`] - typy, struktury a prikazy Vulkanu (generovano
//!   `cargo xtask venus-gen` z `xml/vk.xml` a XML Venusu podle `subset.txt`),
//! - [`capset`] - capset Venusu (verze protokolu od rendereru),
//! - `client` (vlastnost `os`) - spojeni pres render uzel ovladace GPU
//!   (`gpuproto`): kontext, blob pro odpovedi, volani s cekanim na plot.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod capset;
pub mod cs;
pub mod vk;

#[cfg(feature = "os")]
pub mod client;

#[cfg(test)]
mod tests;

/// Bit `VK_COMMAND_GENERATE_REPLY_BIT_EXT`: renderer zapise odpoved.
pub const REPLY: vk::VkCommandFlagsEXT = vk::VK_COMMAND_GENERATE_REPLY_BIT_EXT;

/// `VK_MAKE_API_VERSION(0, major, minor, patch)`.
pub const fn api_version(major: u32, minor: u32, patch: u32) -> u32 {
    (major << 22) | (minor << 12) | patch
}

/// Rozlozi verzi API na (major, minor, patch).
pub const fn version_parts(v: u32) -> (u32, u32, u32) {
    ((v >> 22) & 0x7F, (v >> 12) & 0x3FF, v & 0xFFF)
}
