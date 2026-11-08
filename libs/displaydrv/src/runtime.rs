//! Beh nativniho ovladace v OS (feature `os`): startovni zprava,
//! mapovani stinu kompozitoru a smycka udalosti nad [`Server`].
//!
//! Startovni zprava ovladace (kind USER, v tomto poradi):
//! 1. prostredek zarizeni (BARy, preruseni, DMA),
//! 2. konfiguracni prostor PCI,
//! 3. konec kanalu ke klientovi (kompozitor; konec drzi init, takze
//!    prezije pad ovladace),
//! 4. objekt pameti pro zaznam stavu firmwaru ([`crate::snapshot`]).
//!
//! Bootstrap zprava: `pci <sbernice> <zarizeni> <funkce> [restore]`;
//! `restore` = nova instance po padu predchozi: vratit stav firmwaru
//! a dal slouzit jen rezim firmwaru.

use alloc::boxed::Box;
use alloc::vec::Vec;

use fbcopy::Surface;
use guiproto::display::Request;
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::{Channel, Handle, Mapping, Signals, Startup, Vmo, println};

use crate::{Backend, Config, Server, Shadow, parse_config};

/// Velikost objektu pameti se zaznamem firmwaru.
pub const SNAPSHOT_SIZE: u64 = 64 * 1024;
/// Perioda kontroly konektoru (hotplug bez preruseni).
const HOTPLUG_POLL_NS: u64 = 500_000_000;
/// Perioda kontroly dokonceni flipu.
const FLIP_POLL_NS: u64 = 1_000_000;

/// Handly a konfigurace ze startovni zpravy.
pub struct Start {
    pub resource: libos::hw::Resource,
    pub config_vmo: Vmo,
    pub client: Channel,
    pub snapshot: Mapping,
    _snapshot_vmo: Vmo,
    pub config: Config,
    /// Handle vlastniho procesu: libos ho potrebuje k mapovani pameti,
    /// proto musi zit po celou dobu behu.
    _process: libos::Process,
}

impl Start {
    pub fn take(startup: Startup, name: &str) -> Option<Self> {
        let process = startup.process;
        let mut h = startup.handles.into_iter();
        let (Some(res), Some(cfg), Some(client), Some(snap)) = (h.next(), h.next(), h.next(), h.next()) else {
            println!("{name}: chybi handly ve startovni zprave");
            return None;
        };
        let text = startup.bootstrap.read(None).ok()?;
        let Some(config) = core::str::from_utf8(&text.data).ok().and_then(parse_config) else {
            println!("{name}: neplatna konfigurace");
            return None;
        };
        let snapshot_vmo = Vmo::from_handle(snap);
        let snapshot = match Mapping::new(&snapshot_vmo, 0, SNAPSHOT_SIZE, MAP_READ | MAP_WRITE) {
            Ok(m) => m,
            Err(e) => {
                println!("{name}: zaznam firmwaru nelze namapovat: {e}");
                return None;
            }
        };
        Some(Start {
            resource: libos::hw::Resource::from_handle(res),
            config_vmo: Vmo::from_handle(cfg),
            client: Channel::from_handle(client),
            snapshot,
            _snapshot_vmo: snapshot_vmo,
            config,
            _process: process,
        })
    }

    /// Pamet zaznamu firmwaru.
    pub fn snapshot_bytes(&mut self) -> &mut [u8] {
        // SAFETY: mapovani SNAPSHOT_SIZE bajtu zije se `Start`; pise do nej
        // jen tento proces (init ho drzi, ale necte za behu ovladace).
        unsafe { core::slice::from_raw_parts_mut(self.snapshot.addr() as *mut u8, SNAPSHOT_SIZE as usize) }
    }
}

/// Stin kompozitoru (VMO z `ATTACH`, jen pro cteni).
struct VmoShadow {
    map: Mapping,
    _vmo: Vmo,
    w: u32,
    h: u32,
    stride: u32,
}

impl Shadow for VmoShadow {
    fn surface(&self) -> Surface {
        Surface {
            base: self.map.addr() as *mut u8,
            width: self.w,
            height: self.h,
            stride: self.stride,
        }
    }
}

fn map_shadow(req: &Request, handles: Vec<Handle>) -> Option<Box<dyn Shadow>> {
    let Request::Attach { w, h, stride, .. } = *req else { return None };
    let vmo = Vmo::from_handle(handles.into_iter().next()?);
    let len = stride as u64 * h as u64;
    if stride % 4 != 0 || (stride as u64) < w as u64 * 4 || len == 0 || len > 256 << 20 {
        return None;
    }
    // Mapovani overi, ze je VMO dost velka.
    let map = Mapping::new(&vmo, 0, len, MAP_READ).ok()?;
    Some(Box::new(VmoShadow { map, _vmo: vmo, w, h, stride }))
}

/// Smycka ovladace: konci, az klient zavre kanal (init ho drzi, takze
/// prakticky jen pri vypnuti systemu).
pub fn run<B: Backend>(server: &mut Server<B>, client: &Channel) {
    let hotplug = server.backend.caps() & guiproto::display::caps::HOTPLUG != 0;
    let mut next_hotplug = libos::time::now() + HOTPLUG_POLL_NS;
    loop {
        let now = libos::time::now();
        let mut deadline = if hotplug { Some(next_hotplug) } else { None };
        if server.busy() {
            deadline = Some(now + FLIP_POLL_NS);
        }
        let items = [(client.handle(), Signals::READABLE | Signals::PEER_CLOSED)];
        let obs = match libos::wait_many(&items, deadline) {
            Ok((_, o)) => o,
            Err(libos::Error::TimedOut) => Vec::new(),
            Err(_) => break,
        };
        let s = obs.first().copied().unwrap_or(Signals(0));
        if s.intersects(Signals::READABLE) {
            while let Ok(m) = client.try_read() {
                let now = libos::time::now();
                let replies = match Request::decode(&m.data) {
                    Ok(req) => {
                        let shadow = map_shadow(&req, m.handles);
                        server.handle(req, shadow, now)
                    }
                    Err(code) => alloc::vec![guiproto::display::Reply::Error {
                        op: guiproto::window::ClientMsg::op_of(&m.data),
                        code
                    }],
                };
                for r in replies {
                    let _ = client.write(&r.encode(), Vec::new());
                }
            }
        } else if s.intersects(Signals::PEER_CLOSED) {
            break;
        }
        let now = libos::time::now();
        if hotplug && now >= next_hotplug {
            next_hotplug = now + HOTPLUG_POLL_NS;
        }
        for r in server.poll(now) {
            let _ = client.write(&r.encode(), Vec::new());
        }
    }
}
