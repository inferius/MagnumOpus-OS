//! usb-hid: ovladac zarizeni USB HID (klavesnice, mys, tablet,
//! multimedialni klavesy; HID 1.11).
//!
//! Startovni zprava (kind USER, v tomto poradi): kanal rozhrani
//! (`usbproto::msg`), kanal ke sluzbe vstupu (`inputproto::device`)
//! a volitelne kanal bezpecnostniho klice (`fido`).
//!
//! Bezpecnostni klic FIDO (usage page 0xF1D0 v deskriptoru reportu) neni
//! vstupni zarizeni: ovladac jen predava hlaseni CTAPHID po 64 B mezi
//! klicem a kanalem `fido` (jedna zprava = jedno hlaseni; druhy konec ma
//! sluzba uctu). U ostatnich zarizeni kanal `fido` zavre.
//!
//! Ovladac precte deskriptor reportu a kdyz mu rozumi
//! (`usbproto::report`, `usbproto::hid_report`), cte reporty v report
//! protokolu - umi tak multimedialni klavesy, tablety (absolutni osy) a mysi
//! s vic tlacitky. Jinak prepne rozhrani do boot protokolu (klavesnice
//! a mys bez deskriptoru). Opakovani reportu vypne (`SET_IDLE(0)`, klavesy
//! opakuje sluzba vstupu) a pak cte reporty z interrupt endpointu.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;

use inputproto::device::{self, RawEvent, caps};
use libos::{Channel, Error, Signals, Startup, println};
use usbclient::{UsbClient, is_stall};
use usbproto::desc::{Configuration, Interface, Setup, Transfer, class, kind, request, rt};
use usbproto::hid::{self, BootKeyboard, BootMouse};
use usbproto::hid_report::Decoder;
use usbproto::msg::op;

libos::entry!(main);

enum Kind {
    Keyboard(BootKeyboard),
    Mouse(BootMouse),
    Report(Decoder),
}

/// Nejdelsi deskriptor reportu, ktery ovladac cte.
const MAX_REPORT_DESC: u16 = 4096;

/// Surovy deskriptor reportu rozhrani (`None` = neni nebo nejde precist).
fn report_descriptor(usb: &mut UsbClient, iface: &Interface) -> Option<Vec<u8>> {
    if iface.hid_report_len == 0 {
        return None;
    }
    let setup = Setup {
        request_type: rt::DEVICE_TO_HOST | rt::STANDARD | rt::TO_INTERFACE,
        request: request::GET_DESCRIPTOR,
        value: (kind::HID_REPORT as u16) << 8,
        index: iface.number as u16,
        length: iface.hid_report_len.min(MAX_REPORT_DESC),
    };
    usb.control(setup, &[]).ok()
}

/// Rozebere deskriptor reportu; `None` = nema nic, cemu dekoder rozumi.
fn report_decoder(raw: &[u8]) -> Option<Decoder> {
    let dec = Decoder::new(usbproto::report::parse(raw)?);
    (dec.caps() != 0).then_some(dec)
}

/// Bezpecnostni klic: hlaseni IN -> kanal `fido`, zpravy kanalu -> OUT.
fn fido_loop(usb: &mut UsbClient, iface: &Interface, ep_in: u8, fido: Channel) -> i64 {
    let Some(ep_out) = iface.endpoints.iter().find(|e| !e.is_in() && e.transfer == Transfer::Interrupt).copied() else {
        println!("usb-hid: klic FIDO bez interrupt endpointu OUT");
        return 0;
    };
    let Ok(mut in_id) = usb.send(op::INTERRUPT_IN, ep_in as u32, 64, &[]) else { return 0 };
    loop {
        let items = [
            (usb.channel.handle(), Signals::READABLE | Signals::PEER_CLOSED),
            (fido.handle(), Signals::READABLE | Signals::PEER_CLOSED),
        ];
        let s = match libos::wait_many(&items, None) {
            Ok((_, s)) => s,
            Err(_) => return 0,
        };
        // Od sluzby uctu ke klici.
        loop {
            match fido.try_read() {
                Ok(m) if m.data.len() == 64 => {
                    if usb.send(op::INTERRUPT_OUT, ep_out.address as u32, 64, &m.data).is_err() {
                        return 0;
                    }
                }
                Ok(_) => {}
                Err(Error::ShouldWait) => break,
                Err(_) => {
                    // Sluzba uctu kanal zavrela (napr. restart): klic ne.
                    println!("usb-hid: kanal klice FIDO zavren");
                    return 0;
                }
            }
        }
        // Od klice ke sluzbe uctu (odpovedi OUT se jen zahodi).
        loop {
            let m = match usb.channel.try_read() {
                Ok(m) => m,
                Err(Error::ShouldWait) => break,
                Err(_) => return 0,
            };
            let Some(r) = usbproto::msg::Reply::decode(&m.data) else { continue };
            if r.id != in_id {
                continue;
            }
            if r.status == 0 && r.data.len() == 64 {
                let _ = fido.write(r.data, Vec::new());
            } else if r.status == Error::PeerClosed.code() {
                return 0;
            }
            match usb.send(op::INTERRUPT_IN, ep_in as u32, 64, &[]) {
                Ok(id) => in_id = id,
                Err(_) => return 0,
            }
        }
        if s[0].contains(Signals::PEER_CLOSED) && !s[0].contains(Signals::READABLE) {
            return 0;
        }
    }
}

fn main(startup: Startup) -> i64 {
    let mut handles = startup.handles.into_iter();
    let (Some(usb), Some(input), fido) = (handles.next(), handles.next(), handles.next()) else {
        println!("usb-hid: chybi handly ve startovni zprave");
        return 1;
    };
    let mut usb = UsbClient::new(Channel::from_handle(usb));
    let input = Channel::from_handle(input);
    let info = match usb.info() {
        Ok(i) => i,
        Err(e) => {
            println!("usb-hid: INFO selhalo: {e}");
            return 2;
        }
    };
    let Some(iface) = Configuration::parse(&info.config)
        .and_then(|c| c.interfaces.into_iter().find(|i| i.number == info.interface))
    else {
        println!("usb-hid: rozhrani {} v konfiguraci chybi", info.interface);
        return 2;
    };
    let ep = iface
        .endpoints
        .iter()
        .find(|e| e.is_in() && e.transfer == Transfer::Interrupt)
        .copied();
    let Some(ep) = ep else {
        println!("usb-hid: rozhrani {} nema interrupt endpoint IN", iface.number);
        return 0;
    };
    let boot = iface.class == class::HID && iface.subclass == hid::SUBCLASS_BOOT;
    let raw_desc = report_descriptor(&mut usb, &iface);
    if raw_desc.as_deref().is_some_and(hid::is_fido) {
        println!(
            "usb-hid: bezpecnostni klic FIDO {:04x}:{:04x} (port {:?})",
            info.device.vendor, info.device.product, info.path
        );
        // Vstup klic nedostane (zavrenim kanalu ho sluzba vstupu zapomene).
        drop(input);
        return match fido {
            Some(f) => fido_loop(&mut usb, &iface, ep.address, Channel::from_handle(f)),
            None => 0,
        };
    }
    // Neni klic: kanal klice zavrit (sluzba uctu ho zahodi).
    drop(fido);
    let mut axes = Vec::new();
    let (mut kind, caps, mut setups) = if let Some(dec) = raw_desc.as_deref().and_then(report_decoder) {
        let c = dec.caps();
        axes = dec.axes();
        // Report protokol je vychozi po resetu zarizeni.
        (Kind::Report(dec), c, alloc::vec![hid::set_idle_zero(iface.number)])
    } else if boot && iface.protocol == hid::PROTOCOL_KEYBOARD {
        (Kind::Keyboard(BootKeyboard::new()), caps::KEYBOARD, alloc::vec![])
    } else if boot && iface.protocol == hid::PROTOCOL_MOUSE {
        (Kind::Mouse(BootMouse::new()), caps::POINTER, alloc::vec![])
    } else {
        println!(
            "usb-hid: rozhrani {:02x}/{:02x}/{:02x}: deskriptoru reportu nerozumim",
            iface.class, iface.subclass, iface.protocol
        );
        return 0;
    };
    if !matches!(kind, Kind::Report(_)) {
        // Boot protokol a bez opakovani reportu; zarizeni, ktere
        // pozadavek nezna (STALL), uz boot protokol pouziva.
        setups = alloc::vec![hid::set_boot_protocol(iface.number), hid::set_idle_zero(iface.number)];
    }
    for setup in setups {
        if let Err(e) = usb.control(setup, &[])
            && !is_stall(&e)
        {
            println!("usb-hid: nastaveni rozhrani selhalo: {e}");
            return 3;
        }
    }
    let what = if caps & caps::KEYBOARD != 0 {
        "klavesnice"
    } else if caps & caps::ABSOLUTE != 0 {
        "tablet"
    } else if caps & caps::POINTER != 0 {
        "mys"
    } else if caps & caps::MEDIA != 0 {
        "multimedialni klavesy"
    } else {
        "HID"
    };
    let name = format!(
        "USB {what} {:04x}:{:04x}",
        info.device.vendor, info.device.product
    );
    println!("usb-hid: {name} (port {:?}, {})", info.path, info.speed.name());
    let report_len = match &kind {
        Kind::Report(d) => d.max_report_len() as u32,
        _ => 8,
    };
    let max = (ep.max_packet as u32).max(report_len).clamp(8, 1024);
    let mut hello = Some(device::encode_hello(caps, &name));
    loop {
        let id = match usb.send(op::INTERRUPT_IN, ep.address as u32, max, &[]) {
            Ok(id) => id,
            Err(_) => return 0,
        };
        // Predstavit se sluzbe az s prvnim dotazem na data: zarizeni je pak
        // pripravene (QEMU napr. smeruje udalosti mysi az po prvnim dotazu).
        if let Some(h) = hello.take() {
            if input.write(&h, Vec::new()).is_err() {
                return 0;
            }
            if !axes.is_empty() {
                let _ = input.write(&device::encode_axes(&axes), Vec::new());
            }
        }
        // Cekat na report, nebo na zavreni sluzby vstupu.
        let items = [
            (usb.channel.handle(), Signals::READABLE | Signals::PEER_CLOSED),
            (input.handle(), Signals::PEER_CLOSED),
        ];
        match libos::wait_many(&items, None) {
            Ok((_, s)) if s[1].intersects(Signals::PEER_CLOSED) => return 0,
            Ok(_) => {}
            Err(_) => return 0,
        }
        let mut events = Vec::new();
        match usb.wait(id, None) {
            Ok(t) => match &mut kind {
                Kind::Keyboard(k) => k.report(&t.data, &mut events),
                Kind::Mouse(m) => m.report(&t.data, &mut events),
                Kind::Report(d) => d.report(&t.data, &mut events),
            },
            Err(e) if e.error == Error::PeerClosed => {
                // Zarizeni odpojeno: drzene klavesy a tlacitka uvolnit.
                match &mut kind {
                    Kind::Keyboard(k) => k.release_all(&mut events),
                    Kind::Mouse(m) => m.release_all(&mut events),
                    Kind::Report(d) => d.release_all(&mut events),
                }
                send(&input, &events);
                return 0;
            }
            Err(e) => {
                println!("usb-hid: {name}: prenos selhal: {e}");
                libos::time::sleep(core::time::Duration::from_millis(100));
                continue;
            }
        }
        send(&input, &events);
    }
}

fn send(input: &Channel, events: &[RawEvent]) {
    if !events.is_empty() {
        let _ = input.write(&device::encode_events(events), Vec::new());
    }
}
