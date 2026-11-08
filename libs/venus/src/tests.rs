//! Testy kodovani Venusu na hostiteli (`cargo test -p venus`). Ocekavane
//! bajty odpovidaji referencnimu kodovani venus-protocol
//! (`vn_protocol_driver_*.h`, vygenerovane z tehoz vk.xml).

use alloc::vec::Vec;

use crate::cs::{CsError, Decoder, Encoder};
use crate::vk::{self, *};
use crate::{REPLY, api_version, capset, version_parts};

fn words(b: &[u8]) -> Vec<u32> {
    b.chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect()
}

/// Zapisovac odpovedi (jako renderer).
struct W(Vec<u8>);

impl W {
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
}

#[test]
fn create_instance_matches_reference() {
    let app = VkApplicationInfo { p_application_name: Some("ab"), api_version: api_version(1, 1, 0), ..Default::default() };
    let ci = VkInstanceCreateInfo { p_application_info: Some(&app), ..Default::default() };
    let mut e = Encoder::new();
    cmd::create_instance(&mut e, REPLY, &ci, VkInstance(7));
    let w = words(&e.finish().unwrap());
    let expect: Vec<u32> = [
        &[VK_COMMAND_TYPE_vkCreateInstance_EXT as u32, 1][..],
        // pCreateInfo: ukazatel 1, sType, pNext NULL, flags, pApplicationInfo 1
        &[1, 0, VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO as u32, 0, 0, 0, 1, 0],
        // VkApplicationInfo: sType, pNext NULL, jmeno "ab" (delka 3, "ab\0" + 1 B), verze
        &[VK_STRUCTURE_TYPE_APPLICATION_INFO as u32, 0, 0, 3, 0, u32::from_le_bytes(*b"ab\0\0"), 0],
        // pEngineName NULL, engineVersion, apiVersion
        &[0, 0, 0, api_version(1, 1, 0)],
        // vrstvy: pocet 0, pole 0; rozsireni: pocet 0, pole 0
        &[0, 0, 0, 0, 0, 0],
        // pAllocator NULL, pInstance: ukazatel 1, id 7
        &[0, 0, 1, 0, 7, 0],
    ]
    .concat();
    assert_eq!(w, expect);
}

#[test]
fn enumerate_physical_devices_out_ids() {
    let mut e = Encoder::new();
    cmd::enumerate_physical_devices(&mut e, REPLY, VkInstance(1), Some(2), Some(&[VkPhysicalDevice(5), VkPhysicalDevice(6)]));
    let w = words(&e.finish().unwrap());
    assert_eq!(w, [VK_COMMAND_TYPE_vkEnumeratePhysicalDevices_EXT as u32, 1, 1, 0, 1, 0, 2, 2, 0, 5, 0, 6, 0]);
    // Jen dotaz na pocet: pole NULL.
    let mut e = Encoder::new();
    cmd::enumerate_physical_devices(&mut e, REPLY, VkInstance(1), Some(0), None);
    assert_eq!(words(&e.finish().unwrap())[6..], [0, 0, 0]);
    // Kratsi pole nez pocet = chyba.
    let mut e = Encoder::new();
    cmd::enumerate_physical_devices(&mut e, REPLY, VkInstance(1), Some(3), Some(&[VkPhysicalDevice(5)]));
    assert_eq!(e.finish(), Err(CsError::ShortArray));
}

#[test]
fn memory_properties_partial_has_array_sizes() {
    // Castecne kodovani VkPhysicalDeviceMemoryProperties nese velikosti
    // pevnych poli (32 typu, 16 hald) - renderer je cte.
    let mut e = Encoder::new();
    cmd::get_physical_device_memory_properties(&mut e, REPLY, VkPhysicalDevice(9));
    assert_eq!(words(&e.finish().unwrap()), [VK_COMMAND_TYPE_vkGetPhysicalDeviceMemoryProperties_EXT as u32, 1, 9, 0, 1, 0, 32, 0, 16, 0]);
}

#[test]
fn decode_physical_device_properties() {
    let mut w = W(Vec::new());
    w.u32(VK_COMMAND_TYPE_vkGetPhysicalDeviceProperties_EXT as u32).u64(1);
    w.u32(api_version(1, 3, 255)).u32(5).u32(0x10005).u32(0).u32(VK_PHYSICAL_DEVICE_TYPE_CPU as u32);
    let mut name = [0u8; 256];
    name[..8].copy_from_slice(b"llvmpipe");
    w.u64(256);
    w.0.extend_from_slice(&name);
    w.u64(16);
    w.0.extend_from_slice(&[0xAB; 16]);
    // Limity a ridke vlastnosti: vychozi hodnoty (plne kodovani vystupnich
    // struktur je jen v testech).
    let mut lim = Encoder::new();
    VkPhysicalDeviceLimits::default().encode(&mut lim);
    VkPhysicalDeviceSparseProperties::default().encode(&mut lim);
    w.0.extend_from_slice(&lim.finish().unwrap());
    let mut d = Decoder::new(&w.0);
    let r = cmd::decode_get_physical_device_properties_reply(&mut d).unwrap();
    let p = r.p_properties.unwrap();
    assert_eq!(crate::cs::c_str(&p.device_name), "llvmpipe");
    assert_eq!(version_parts(p.api_version), (1, 3, 255));
    assert_eq!(p.vendor_id, 0x10005);
    assert_eq!(p.pipeline_cache_uuid, [0xAB; 16]);
    assert_eq!(d.remaining(), 0);
    // Useknuta odpoved = chyba, ne panika.
    for n in 0..w.0.len() {
        let mut d = Decoder::new(&w.0[..n]);
        assert!(cmd::decode_get_physical_device_properties_reply(&mut d).is_err());
    }
    // Jiny typ prikazu.
    let mut d = Decoder::new(&w.0);
    assert_eq!(cmd::decode_create_instance_reply(&mut d).unwrap_err(), CsError::Unexpected);
}

#[test]
fn strings_and_padding() {
    let mut e = Encoder::new();
    e.string("abcd");
    // delka 5, "abcd\0" + 3 B doplneni
    assert_eq!(e.finish().unwrap(), [5, 0, 0, 0, 0, 0, 0, 0, b'a', b'b', b'c', b'd', 0, 0, 0, 0]);
    let mut e = Encoder::new();
    e.string("a\0b");
    assert_eq!(e.finish(), Err(CsError::BadString));
    let mut e = Encoder::new();
    e.u16_array(&[1, 2, 3]);
    e.u8(9);
    assert_eq!(e.finish().unwrap(), [1, 0, 2, 0, 3, 0, 0, 0, 9, 0, 0, 0]);
}

#[test]
fn decoder_rejects_huge_arrays() {
    let mut w = W(Vec::new());
    w.u32(VK_COMMAND_TYPE_vkEnumeratePhysicalDevices_EXT as u32).u32(0).u64(1).u32(1).u64(u64::MAX);
    let mut d = Decoder::new(&w.0);
    assert!(cmd::decode_enumerate_physical_devices_reply(&mut d).is_err());
}

#[test]
fn capset_parse() {
    let mut b = [0u8; capset::SIZE];
    b[0..4].copy_from_slice(&1u32.to_le_bytes());
    b[16..20].copy_from_slice(&1u32.to_le_bytes());
    b[38 * 4..38 * 4 + 4].copy_from_slice(&1u32.to_le_bytes());
    let c = capset::VenusCapset::parse(&b).unwrap();
    assert!(c.compatible() && c.supports_multiple_timelines && !c.use_guest_vram);
    assert!(capset::VenusCapset::parse(&b[..100]).is_none());
    b[0] = 2;
    assert!(!capset::VenusCapset::parse(&b).unwrap().compatible());
    let _ = vk::VK_SUCCESS;
}
