//! virtio-gpu: ovladac grafiky (virtio 1.x pres PCI, rezim 2D).
//!
//! Spousti ho spravce zarizeni. Startovni zprava (kind USER, v tomto poradi):
//! 1. omezeny Resource tohoto zarizeni,
//! 2. stranka konfiguracniho prostoru PCI,
//! 3. kanal ke klientovi (kompozitor, protokol `guiproto::display`);
//!    jeho zavreni = konec ovladace.
//!
//! Prvni zprava na bootstrap kanalu: `pci <sbernice> <zarizeni> <funkce>`.
//!
//! Kompozitor kresli do sve VMO (`ATTACH`, ovladac ji mapuje jen pro
//! cteni). Na `PRESENT` ovladac zkopiruje poskozene obdelniky do svych
//! bufferu DMA (s IOMMU jedina pamet, kterou zarizeni vidi), posle
//! `TRANSFER_TO_HOST_2D` a `RESOURCE_FLUSH` a odpovi `PRESENTED` s casem.
//!
//! RIZIKO: kompozitor muze VMO menit i behem kopie - roztrhne to jen
//! jeden snimek (kompozitor je jediny klient). Bez IOMMU muze zarizeni
//! pres DMA zapsat kamkoli (jako u ostatnich ovladacu).

#![no_std]
#![no_main]

extern crate alloc;

mod device;

use alloc::vec;
use alloc::vec::Vec;

use guiproto::Rect;
use guiproto::display::{Output, Reply, Request};
use libos::hw::Resource;
use libos::{Channel, Error, Signals, Startup, Vmo, println};
use pci::Ecam;

use device::Gpu;

libos::entry!(main);

fn main(startup: Startup) -> i64 {
    let mut handles = startup.handles.into_iter();
    let (Some(res), Some(ecam_vmo), Some(client)) = (handles.next(), handles.next(), handles.next()) else {
        println!("virtio-gpu: chybi handly ve startovni zprave");
        return 1;
    };
    let resource = Resource::from_handle(res);
    let ecam_vmo = Vmo::from_handle(ecam_vmo);
    let client = Channel::from_handle(client);
    let Some(address) = virtio::read_driver_config(&startup.bootstrap) else {
        println!("virtio-gpu: neplatna konfigurace od spravce zarizeni");
        return 2;
    };
    let gpu = Ecam::single(&ecam_vmo, address).and_then(|e| Gpu::open(&e, address, resource));
    let mut gpu = match gpu {
        Ok(g) => g,
        Err(e) => {
            println!("virtio-gpu {address}: inicializace selhala: {e}");
            return 3;
        }
    };
    if !gpu.capsets.is_empty() || gpu.virgl {
        let mut s = alloc::string::String::new();
        for (id, ver, size) in &gpu.capsets {
            use core::fmt::Write;
            let _ = write!(s, " {}(v{ver}, {size} B)", vgpu::capset_name(*id));
        }
        println!(
            "virtio-gpu {address}: 3D: virgl={} blob={} context_init={} capsety:{s}",
            gpu.virgl as u8, gpu.blob as u8, gpu.context_init as u8
        );
    }
    let outs = gpu.outputs();
    match outs.first() {
        Some(o) => println!(
            "virtio-gpu {address}: {} vystup(u), prvni {}x{}{}",
            outs.len(),
            o.w,
            o.h,
            if gpu.virgl { ", virgl" } else { "" }
        ),
        None => println!("virtio-gpu {address}: zadny vystup"),
    }
    loop {
        // Ovladac konci, az klient (init, pak kompozitor) zavre kanal. Po
        // padu kompozitoru kanal nezanikne (konec drzi supervisor), nova
        // instance se synchronizuje (`SYNC`) a pripoji znovu (`ATTACH`).
        let items = [(client.handle(), Signals::READABLE | Signals::PEER_CLOSED)];
        let Ok((_, obs)) = libos::wait_many(&items, None) else {
            break;
        };
        if obs[0].intersects(Signals::READABLE) {
            while let Ok(m) = client.try_read() {
                let reply = handle(&mut gpu, &m.data, m.handles);
                if client.write(&reply.encode(), Vec::new()).is_err() {
                    break;
                }
                if gpu.broken {
                    println!("virtio-gpu {address}: zarizeni neodpovida, koncim");
                    let _ = gpu.reset();
                    return 4;
                }
            }
        } else if obs[0].intersects(Signals::PEER_CLOSED) {
            break;
        }
    }
    // Zarizeni zastavit (zadne DMA po konci ovladace).
    let _ = gpu.reset();
    0
}

fn handle(gpu: &mut Gpu, data: &[u8], handles: Vec<libos::Handle>) -> Reply {
    let op = guiproto::window::ClientMsg::op_of(data);
    let err = |code: Error| Reply::Error { op, code };
    let req = match Request::decode(data) {
        Ok(r) => r,
        Err(e) => return err(e),
    };
    match req {
        Request::Info => Reply::Modes {
            caps: if gpu.virgl { guiproto::display::caps::VIRGL } else { 0 },
            outputs: gpu
                .outputs()
                .iter()
                .map(|o| Output {
                    w: o.w,
                    h: o.h,
                    enabled: true,
                })
                .collect(),
        },
        Request::Attach { output, w, h, stride } => {
            let Some(vmo) = handles.into_iter().next().map(Vmo::from_handle) else {
                return err(Error::InvalidArgs);
            };
            match gpu.attach(output, w, h, stride, vmo) {
                Ok(()) => Reply::Attached { output },
                Err(e) => err(e),
            }
        }
        // Znacka pro novou instanci kompozitoru (zahodi odpovedi stare).
        Request::Sync { token } => Reply::Synced { token },
        // Rezim urcuje hostitel (xres/yres); zmena za behu zatim ne.
        ref r @ (Request::Connectors | Request::SetMode { .. }) => {
            let sizes: Vec<(u32, u32)> = gpu.outputs().iter().map(|o| (o.w, o.h)).collect();
            guiproto::modes::fixed_reply(r, &sizes, guiproto::modes::kind::VIRTUAL).unwrap_or_else(|| err(Error::NotSupported))
        }
        Request::Present { seq, output, rects } => {
            let rects: Vec<Rect> = if rects.is_empty() { vec![Rect::new(0, 0, u32::MAX, u32::MAX)] } else { rects };
            match gpu.present(output, &rects) {
                Ok(()) => Reply::Presented {
                    seq,
                    time: libos::time::now(),
                },
                Err(e) => err(e),
            }
        }
    }
}
