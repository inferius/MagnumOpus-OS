//! vk-probe: testovaci program Vulkanu pres Venus (testy v `cargo xtask test`).
//!
//! Dostane kanal render uzlu ovladace GPU (startovni zprava, kind USER),
//! pres knihovnu `venus` vytvori instanci Vulkanu, vyjmenuje fyzicka
//! zarizeni a precte jejich vlastnosti. Vysledek posle na bootstrap kanal
//! (init) jednim radkem a skonci:
//! - `ok <pocet>|<jmeno>|<typ>|<api x.y.z>|<vendor:device>` - prvni zarizeni,
//! - `skip <duvod>` - zarizeni nema Venus (capset 4),
//! - `err <popis>`.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libos::{Channel, Startup, println};
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
    let line = match probe(ch) {
        Ok(s) => s,
        Err(Error::Os(libos::Error::NotSupported)) => String::from("skip zarizeni nema Venus"),
        Err(e) => format!("err {e:?}"),
    };
    println!("vk-probe: {line}");
    let _ = out.write(line.as_bytes(), Vec::new());
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

fn probe(ch: Channel) -> Result<String, Error> {
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
    v.send(|e| cmd::destroy_instance(e, 0, instance))?;
    Ok(line)
}
