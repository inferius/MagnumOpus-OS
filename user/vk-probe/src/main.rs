//! vk-probe: testovaci program Vulkanu pres Venus (testy v `cargo xtask test`).
//!
//! Dostane kanal render uzlu ovladace GPU (startovni zprava, kind USER)
//! a pres knihovnu `venus` posle na bootstrap kanal (init) dva radky:
//! 1. zarizeni: instance, fyzicka zarizeni a jejich vlastnosti -
//!    `ok <pocet>|<jmeno>|<typ>|<api x.y.z>|<vendor:device>` prvniho,
//!    `skip <duvod>` (zarizeni nema Venus) nebo `err <popis>`,
//! 2. obrazek (`clear.rs`): vyplneni barvou a zpetne cteni pixelu -
//!    `ok <w>x<h> <rgba> ...` nebo `err <popis>` (po `skip` se neposila).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libos::{Channel, Startup, println};

mod clear;
use venus::client::{Error, Venus, check};
use venus::vk::{self, cmd};
use venus::{REPLY, api_version, cs, version_parts};

libos::entry!(main);

fn main(startup: Startup) -> i64 {
    let out = startup.bootstrap;
    let Some(ch) = startup.handles.into_iter().next().map(Channel::from_handle) else {
        let _ = out.write(b"err chybi kanal render uzlu", Vec::new());
        return 1;
    };
    let (line, rest) = match probe(ch) {
        Ok((s, v, pd, instance)) => (s, Some((v, pd, instance))),
        Err(Error::Os(libos::Error::NotSupported)) => (String::from("skip zarizeni nema Venus"), None),
        Err(e) => (format!("err {e:?}"), None),
    };
    println!("vk-probe: {line}");
    let _ = out.write(line.as_bytes(), Vec::new());
    let Some((mut v, pd, instance)) = rest else { return 0 };
    let line = match clear::run(&mut v, pd) {
        Ok(s) => s,
        Err(e) => format!("err {e:?}"),
    };
    println!("vk-probe: obrazek: {line}");
    let _ = out.write(line.as_bytes(), Vec::new());
    let _ = v.send(|e| cmd::destroy_instance(e, 0, instance));
    0
}

fn device_type(t: vk::VkPhysicalDeviceType) -> &'static str {
    match t {
        vk::VK_PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU => "integrovana GPU",
        vk::VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU => "samostatna GPU",
        vk::VK_PHYSICAL_DEVICE_TYPE_VIRTUAL_GPU => "virtualni GPU",
        vk::VK_PHYSICAL_DEVICE_TYPE_CPU => "CPU",
        _ => "jine",
    }
}

/// Zarizeni: radek vysledku, spojeni, prvni fyzicke zarizeni a instance.
fn probe(ch: Channel) -> Result<(String, Venus, vk::VkPhysicalDevice, vk::VkInstance), Error> {
    let mut v = Venus::connect(ch)?;
    let (maj, min, pat) = version_parts(v.capset.vk_xml_version);
    println!("vk-probe: renderer Venus, vk.xml {maj}.{min}.{pat}, format proudu {}", v.capset.wire_format_version);
    let ver = v.call(|e| cmd::enumerate_instance_version(e, REPLY), cmd::decode_enumerate_instance_version_reply)?;
    check(ver.ret)?;
    let instance = vk::VkInstance(v.new_id());
    let app = vk::VkApplicationInfo {
        p_application_name: Some("vk-probe"),
        application_version: 1,
        p_engine_name: Some("MagnumOpus-OS"),
        engine_version: 1,
        api_version: api_version(1, 1, 0),
    };
    let info = vk::VkInstanceCreateInfo { p_application_info: Some(&app), ..Default::default() };
    let r = v.call(|e| cmd::create_instance(e, REPLY, &info, instance), cmd::decode_create_instance_reply)?;
    check(r.ret)?;
    // Pocet zarizeni, pak id pro kazde (handle prideluje klient).
    let r = v.call(
        |e| cmd::enumerate_physical_devices(e, REPLY, instance, Some(0), None),
        cmd::decode_enumerate_physical_devices_reply,
    )?;
    check(r.ret)?;
    let n = r.p_physical_device_count.unwrap_or(0);
    if n == 0 {
        return Err(Error::Vk(vk::VK_ERROR_INITIALIZATION_FAILED));
    }
    let ids: Vec<vk::VkPhysicalDevice> = (0..n).map(|_| vk::VkPhysicalDevice(v.new_id())).collect();
    let r = v.call(
        |e| cmd::enumerate_physical_devices(e, REPLY, instance, Some(n), Some(&ids)),
        cmd::decode_enumerate_physical_devices_reply,
    )?;
    if r.ret != vk::VK_SUCCESS && r.ret != vk::VK_INCOMPLETE {
        return Err(Error::Vk(r.ret));
    }
    let devices = r.p_physical_devices.unwrap_or_default();
    let first = *devices.first().ok_or(Error::Cs(cs::CsError::Unexpected))?;
    let mut line = String::new();
    for (i, &pd) in devices.iter().enumerate() {
        let r = v.call(|e| cmd::get_physical_device_properties(e, REPLY, pd), cmd::decode_get_physical_device_properties_reply)?;
        let p = r.p_properties.ok_or(Error::Cs(cs::CsError::Unexpected))?;
        let (a, b, c) = version_parts(p.api_version);
        let name = cs::c_str(&p.device_name);
        println!(
            "vk-probe: zarizeni {i}: {name} ({}), Vulkan {a}.{b}.{c}, {:04x}:{:04x}",
            device_type(p.device_type),
            p.vendor_id,
            p.device_id
        );
        if pd == first {
            line = format!(
                "ok {}|{name}|{}|{a}.{b}.{c}|{:04x}:{:04x}",
                devices.len(),
                device_type(p.device_type),
                p.vendor_id,
                p.device_id
            );
        }
    }
    // Fronty a pamet prvniho zarizeni (podklad pro dalsi krok).
    let q = v.call(
        |e| cmd::get_physical_device_queue_family_properties(e, REPLY, first, Some(0), false),
        cmd::decode_get_physical_device_queue_family_properties_reply,
    )?;
    let m = v.call(
        |e| cmd::get_physical_device_memory_properties(e, REPLY, first),
        cmd::decode_get_physical_device_memory_properties_reply,
    )?;
    if let Some(m) = m.p_memory_properties {
        println!(
            "vk-probe: rodin front {}, typu pameti {}, hald {}",
            q.p_queue_family_property_count.unwrap_or(0),
            m.memory_type_count,
            m.memory_heap_count
        );
    }
    Ok((line, v, first, instance))
}
