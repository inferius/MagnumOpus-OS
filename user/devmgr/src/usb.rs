//! USB: spusteni ovladace radice xHCI a tridnich ovladacu podle zprav
//! `usb-interface` od nej.
//!
//! Radic hlasi kazde rozhrani (a svuj virtualni root hub) zpravou
//! `usb-interface <trida> <podtrida> <protokol> <vid> <pid>` (sestnactkove)
//! s kanalem rozhrani. Spravce podle tridy spusti `usb-hub` (09) nebo
//! `usb-hid` (03); tridni ovladace nedostanou zadny Resource, jen kanal
//! rozhrani (a `usb-hid` kanal ke sluzbe vstupu, jehoz druhy konec jde
//! initu zpravou `input`, a kanal bezpecnostniho klice - zprava `fido`).

use alloc::vec;
use alloc::vec::Vec;

use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Handle, Message, Process, Result, println};

/// Trida PCI radice xHCI: serial bus / USB / xHCI.
pub const XHCI_CLASS: (u8, u8, u8) = (0x0C, 0x03, 0x30);

/// Bezici radic.
pub struct Controller {
    pub _process: Process,
    pub channel: Channel,
    pub ready: bool,
    /// Radic skoncil (kanal zavreny): uz na nej necekat.
    pub closed: bool,
}

/// Zpracuje zpravu od radice. Vraci `true` pro `usb-ready`.
pub fn handle(
    fs: &Initfs,
    msg: Message,
    parent: &Channel,
    procs: &mut Vec<Process>,
) -> bool {
    let text = core::str::from_utf8(&msg.data).unwrap_or("");
    let mut w = text.split_whitespace();
    match w.next() {
        Some("usb-ready") => true,
        Some("usb-interface") => {
            let mut num = || w.next().and_then(|s| u16::from_str_radix(s, 16).ok());
            let (Some(class), Some(sub), Some(proto), Some(vid), Some(pid)) =
                (num(), num(), num(), num(), num())
            else {
                println!("devmgr: neplatna zprava od xhci: {text}");
                return false;
            };
            let Some(ch) = msg.handles.into_iter().next() else {
                return false;
            };
            match start_class(fs, class as u8, ch, parent) {
                Ok(Some(p)) => procs.push(p),
                Ok(None) => println!(
                    "devmgr: usb {vid:04x}:{pid:04x} rozhrani {class:02x}/{sub:02x}/{proto:02x}: ovladac neni"
                ),
                Err(e) => println!("devmgr: tridni ovladac USB nelze spustit: {e}"),
            }
            false
        }
        _ => {
            println!("devmgr: neznama zprava od xhci: {text}");
            false
        }
    }
}

/// Spusti tridni ovladac pro rozhrani; `Ok(None)` = trida bez ovladace
/// (kanal se zavre).
fn start_class(fs: &Initfs, class: u8, iface: Handle, parent: &Channel) -> Result<Option<Process>> {
    match class {
        0x09 => {
            let image = fs.find("usb-hub").ok_or(libos::Error::NotFound)?;
            let (p, _control) = libos::spawn("usb-hub", image, vec![StartHandle::user(iface)])?;
            Ok(Some(p))
        }
        0x03 => {
            let image = fs.find("usb-hid").ok_or(libos::Error::NotFound)?;
            let (client, driver) = Channel::create()?;
            // Kanal bezpecnostniho klice: ovladac ho pouzije jen u klice
            // FIDO (usage page 0xF1D0), jinak ho zavre. Druhy konec jde
            // initu (`fido`) a od nej sluzbe uctu.
            let (key_client, key_driver) = Channel::create()?;
            let handles = vec![
                StartHandle::user(iface),
                StartHandle::user(driver.into_handle()),
                StartHandle::user(key_driver.into_handle()),
            ];
            let (p, _control) = libos::spawn::service("usb-hid", image, handles)?;
            parent.write(b"input", vec![client.into_handle()])?;
            parent.write(b"fido", vec![key_client.into_handle()])?;
            Ok(Some(p))
        }
        _ => Ok(None),
    }
}
