//! usb-hid: ovladac klavesnice a mysi USB v boot protokolu (HID 1.11).
//!
//! Startovni zprava (kind USER, v tomto poradi): kanal rozhrani
//! (`usbproto::msg`) a kanal ke sluzbe vstupu (`inputproto::device`).
//! Ovladac prepne rozhrani do boot protokolu, vypne opakovani reportu
//! (`SET_IDLE(0)`, opakuje sluzba vstupu) a pak cte reporty z interrupt
//! endpointu. Rozhrani bez boot protokolu (multimedialni klavesy, tablety)
//! zatim neobsluhuje - potrebuji parser deskriptoru reportu (faze 4).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;

use inputproto::device::{self, RawEvent, caps};
use libos::{Channel, Error, Signals, Startup, println};
use usbclient::{UsbClient, is_stall};
use usbproto::desc::{Configuration, Transfer, class};
use usbproto::hid::{self, BootKeyboard, BootMouse};
use usbproto::msg::op;

libos::entry!(main);

enum Kind {
    Keyboard(BootKeyboard),
    Mouse(BootMouse),
}

fn main(startup: Startup) -> i64 {
    let mut handles = startup.handles.into_iter();
    let (Some(usb), Some(input)) = (handles.next(), handles.next()) else {
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
    let boot = iface.class == class::HID && iface.subclass == hid::SUBCLASS_BOOT;
    let (Some(ep), true) = (ep, boot) else {
        println!(
            "usb-hid: rozhrani {:02x}/{:02x}/{:02x} bez boot protokolu zatim nepodporovane",
            iface.class, iface.subclass, iface.protocol
        );
        return 0;
    };
    let (mut kind, caps, what) = match iface.protocol {
        hid::PROTOCOL_KEYBOARD => (Kind::Keyboard(BootKeyboard::new()), caps::KEYBOARD, "klavesnice"),
        hid::PROTOCOL_MOUSE => (Kind::Mouse(BootMouse::new()), caps::POINTER, "mys"),
        p => {
            println!("usb-hid: boot protokol {p} nepodporovany");
            return 0;
        }
    };
    // Boot protokol a bez opakovani reportu; zarizeni, ktere pozadavek
    // nezna (STALL), uz boot protokol pouziva.
    for setup in [hid::set_boot_protocol(iface.number), hid::set_idle_zero(iface.number)] {
        if let Err(e) = usb.control(setup, &[])
            && !is_stall(&e)
        {
            println!("usb-hid: nastaveni rozhrani selhalo: {e}");
            return 3;
        }
    }
    let name = format!(
        "USB {what} {:04x}:{:04x}",
        info.device.vendor, info.device.product
    );
    println!("usb-hid: {name} (port {:?}, {})", info.path, info.speed.name());
    let max = (ep.max_packet as u32).clamp(8, 64);
    let mut hello = Some(device::encode_hello(caps, &name));
    loop {
        let id = match usb.send(op::INTERRUPT_IN, ep.address as u32, max, &[]) {
            Ok(id) => id,
            Err(_) => return 0,
        };
        // Predstavit se sluzbe az s prvnim dotazem na data: zarizeni je pak
        // pripravene (QEMU napr. smeruje udalosti mysi az po prvnim dotazu).
        if let Some(h) = hello.take()
            && input.write(&h, Vec::new()).is_err()
        {
            return 0;
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
            },
            Err(e) if e.error == Error::PeerClosed => {
                // Zarizeni odpojeno: drzene klavesy a tlacitka uvolnit.
                match &mut kind {
                    Kind::Keyboard(k) => k.release_all(&mut events),
                    Kind::Mouse(m) => m.release_all(&mut events),
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
