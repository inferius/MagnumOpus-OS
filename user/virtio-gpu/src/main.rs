//! virtio-gpu: ovladac grafiky (virtio 1.x pres PCI): displej 2D a render
//! uzel pro 3D (Venus, protokol `gpuproto`, viz `render.rs`).
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
//! `OPEN_RENDER` vrati novy kanal render uzlu (kontext s capsetem, bloby
//! ve sdilene pameti, proudy prikazu). Render klientu muze byt vic.
//!
//! RIZIKO: kompozitor muze VMO menit i behem kopie - roztrhne to jen
//! jeden snimek (kompozitor je jediny klient). Bez IOMMU muze zarizeni
//! pres DMA zapsat kamkoli (jako u ostatnich ovladacu).

#![no_std]
#![no_main]

extern crate alloc;

mod device;
mod render;

use alloc::vec;
use alloc::vec::Vec;

use guiproto::Rect;
use guiproto::display::{Output, Reply, Request};
use libos::hw::Resource;
use libos::{Channel, Error, Signals, Startup, Vmo, println};
use pci::Ecam;

use device::Gpu;
use render::Client;

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
            "virtio-gpu {address}: 3D: virgl={} blob={} context_init={} sdilena pamet={} MiB capsety:{s}",
            gpu.virgl as u8,
            gpu.blob as u8,
            gpu.context_init as u8,
            gpu.shm.map_or(0, |m| m.len >> 20)
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
    let mut renders: Vec<Client> = Vec::new();
    loop {
        // Ovladac konci, az klient displeje (init, pak kompozitor) zavre
        // kanal. Render klienti se obsluhuji vedle nej.
        let mut items = alloc::vec![(client.handle(), Signals::READABLE | Signals::PEER_CLOSED)];
        for r in &renders {
            items.push((r.ch.handle(), Signals::READABLE | Signals::PEER_CLOSED));
        }
        let Ok((_, obs)) = libos::wait_many(&items, None) else {
            break;
        };
        drop(items);
        // Render klienti (odzadu, zavrene se odebiraji).
        for i in (0..renders.len()).rev() {
            let o = obs[i + 1];
            if o.intersects(Signals::READABLE) {
                while let Ok(m) = renders[i].ch.try_read() {
                    let (reply, handles) = renders[i].handle(&mut gpu, &m.data);
                    let _ = renders[i].ch.write(&reply.encode(), handles);
                    if gpu.broken {
                        break;
                    }
                }
            } else if o.intersects(Signals::PEER_CLOSED) {
                let mut c = renders.remove(i);
                c.close(&mut gpu);
            }
        }
        if obs[0].intersects(Signals::READABLE) {
            while let Ok(m) = client.try_read() {
                let (reply, handles) = handle(&mut gpu, &m.data, m.handles, &mut renders);
                if client.write(&reply.encode(), handles).is_err() {
                    break;
                }
            }
        } else if obs[0].intersects(Signals::PEER_CLOSED) {
            break;
        }
        if gpu.broken {
            println!("virtio-gpu {address}: zarizeni neodpovida, koncim");
            let _ = gpu.reset();
            return 4;
        }
    }
    // Zarizeni zastavit (zadne DMA po konci ovladace).
    let _ = gpu.reset();
    0
}

/// Zprava displeje; odpoved a handly k ni.
fn handle(gpu: &mut Gpu, data: &[u8], handles: Vec<libos::Handle>, renders: &mut Vec<Client>) -> (Reply, Vec<libos::Handle>) {
    let op = guiproto::window::ClientMsg::op_of(data);
    let req = match Request::decode(data) {
        Ok(r) => r,
        Err(code) => return (Reply::Error { op, code }, Vec::new()),
    };
    if req == Request::OpenRender {
        return match open_render(gpu, renders) {
            Ok(h) => (Reply::Render, alloc::vec![h]),
            Err(code) => (Reply::Error { op, code }, Vec::new()),
        };
    }
    (handle_display(gpu, req, handles, op), Vec::new())
}

/// Ma zarizeni vse pro Venus (capset, bloby, context_init, sdilena pamet)?
fn has_venus(gpu: &Gpu) -> bool {
    gpu.virgl && gpu.blob && gpu.context_init && gpu.shm.is_some() && gpu.capsets.iter().any(|c| c.0 == vgpu::CAPSET_VENUS)
}

/// Novy kanal render uzlu (jen se zarizenim s kontexty 3D).
fn open_render(gpu: &Gpu, renders: &mut Vec<Client>) -> Result<libos::Handle, Error> {
    if !gpu.virgl || !gpu.context_init {
        return Err(Error::NotSupported);
    }
    if renders.len() >= MAX_RENDER_CLIENTS {
        return Err(Error::NoResources);
    }
    let (mine, theirs) = Channel::create()?;
    renders.push(Client::new(mine));
    Ok(theirs.into_handle())
}

/// Nejvic soucasnych render klientu.
const MAX_RENDER_CLIENTS: usize = 16;

fn handle_display(gpu: &mut Gpu, req: Request, handles: Vec<libos::Handle>, op: u32) -> Reply {
    let err = |code: Error| Reply::Error { op, code };
    match req {
        Request::OpenRender => err(Error::BadState),
        Request::Info => Reply::Modes {
            caps: (if gpu.virgl { guiproto::display::caps::VIRGL } else { 0 })
                | (if has_venus(gpu) { guiproto::display::caps::VENUS } else { 0 }),
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
