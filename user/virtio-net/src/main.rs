//! virtio-net: ovladac virtualni sitove karty (virtio 1.x pres PCI).
//!
//! Spousti ho spravce zarizeni. Startovni zprava (kind USER, v tomto poradi):
//! 1. omezeny Resource (jen BARy a konfigurace tohoto zarizeni, kvoty
//!    preruseni a DMA; s IOMMU vlastni domena DMA),
//! 2. stranka konfiguracniho prostoru PCI teto funkce,
//! 3. kanal ke sluzbe site (protokol `netproto::link`).
//!
//! Prvni zprava na bootstrap kanalu od spravce: `pci <sbernice> <zarizeni>
//! <funkce>`. Ovladac posle sluzbe `INFO` (MAC, MTU) a pak predava ramce
//! obema smery, dokud sluzba kanal nezavre.
//!
//! Fronty: 0 = prijem (ovladac do ni dava prazdne buffery), 1 = odesilani.
//! Kazdy buffer ma 2 KiB: hlavicka virtio-net (12 B) + ramec. Bez
//! `VIRTIO_NET_F_MRG_RXBUF` se ramec vzdy vejde do jednoho bufferu.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::hw::{Interrupt, Resource};
use libos::{Channel, Error, Mapping, Result, Signals, Startup, Vmo, println, wait_many};
use netproto::link;
use pci::msix::MsixTable;
use pci::{Address, Ecam};
use virtio::{Buffer, Device, F_VERSION_1, Virtqueue};

/// Vlastnost: zarizeni zna svou MAC (v konfiguraci).
const F_MAC: u64 = 1 << 5;
/// Vlastnost: zarizeni hlasi stav linky.
const F_STATUS: u64 = 1 << 16;
/// Vlastnost: DMA jde pres preklad platformy (IOMMU). Kdyz ji zarizeni
/// nabidne, ovladac ji musi prijmout.
const F_ACCESS_PLATFORM: u64 = 1 << 33;

/// Hlavicka virtio-net s `VIRTIO_F_VERSION_1` (vcetne `num_buffers`).
const HDR: usize = 12;
/// Velikost jednoho bufferu (hlavicka + ramec 1514 B, zaokrouhleno).
const BUF: u64 = 2048;
/// Nejvic bufferu v kazde fronte.
const QUEUE_MAX: u16 = 64;

const RX_QUEUE: u16 = 0;
const TX_QUEUE: u16 = 1;

/// Offset stavu linky v konfiguraci zarizeni (u16, bit 0 = pripojeno).
const CFG_STATUS: u64 = 6;

libos::entry!(main);

struct Nic {
    device: Device,
    rx: Virtqueue,
    tx: Virtqueue,
    irq: Interrupt,
    _msix: MsixTable,
    _dma: Vmo,
    buf: Mapping,
    buf_phys: u64,
    /// Volne buffery odesilani (cisla deskriptoru fronty 1).
    tx_free: Vec<u16>,
    info: link::Info,
    /// Zahozene ramce: sluzba nestihala prijimat / ramec neplatny.
    dropped_rx: u64,
}

fn main(startup: Startup) -> i64 {
    let mut handles = startup.handles.into_iter();
    let (Some(res), Some(ecam_vmo), Some(client)) =
        (handles.next(), handles.next(), handles.next())
    else {
        println!("virtio-net: chybi handly ve startovni zprave");
        return 1;
    };
    let resource = Resource::from_handle(res);
    let ecam_vmo = Vmo::from_handle(ecam_vmo);
    let client = Channel::from_handle(client);

    let Some(address) = virtio::driver_address(&startup.bootstrap) else {
        println!("virtio-net: neplatna konfigurace od spravce zarizeni");
        return 2;
    };
    let nic =
        Ecam::single(&ecam_vmo, address).and_then(|ecam| Nic::open(&ecam, address, &resource));
    let mut nic = match nic {
        Ok(n) => n,
        Err(e) => {
            println!("virtio-net {address}: inicializace selhala: {e}");
            return 3;
        }
    };
    let m = nic.info.mac;
    println!(
        "virtio-net {address}: MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}, linka {}",
        m[0],
        m[1],
        m[2],
        m[3],
        m[4],
        m[5],
        if nic.info.flags & link::FLAG_UP != 0 {
            "pripojena"
        } else {
            "odpojena"
        }
    );
    if client.write(&nic.info.encode(), Vec::new()).is_err() {
        return 0;
    }
    let result = nic.serve(&client);
    // Zarizeni zastavit v kazdem pripade, aby po konci ovladace uz
    // nepouzivalo DMA buffery (pamet se s procesem uvolni).
    let _ = nic.device.reset();
    let code = match result {
        Ok(()) => 0,
        Err(e) => {
            println!("virtio-net {address}: zarizeni neodpovida spravne ({e}), koncim");
            4
        }
    };
    if nic.dropped_rx > 0 {
        println!(
            "virtio-net {address}: zahozeno {} prijatych ramcu",
            nic.dropped_rx
        );
    }
    code
}

/// Mistne spravovana MAC, kdyz ji zarizeni nehlasi (bit 1 prvniho bajtu =
/// mistne spravovana, bit 0 = unicast).
fn local_mac() -> [u8; 6] {
    let t = libos::time::now().to_le_bytes();
    [0x02, t[0], t[1], t[2], t[3], t[4]]
}

impl Nic {
    fn open(ecam: &Ecam, a: Address, resource: &Resource) -> Result<Self> {
        let device = Device::open(ecam, a, resource)?;
        device.begin_init()?;
        let features = device.negotiate(F_VERSION_1 | F_MAC | F_STATUS | F_ACCESS_PLATFORM)?;

        let irq = resource.interrupt()?;
        let msix = MsixTable::open(ecam, a, resource)?;
        msix.set(0, irq.msi())?;
        msix.enable(ecam);
        // Obe fronty na jednom vektoru: ovladac pri preruseni projde obe.
        let mut rx = device.setup_queue(RX_QUEUE, QUEUE_MAX, 0, resource)?;
        let tx = device.setup_queue(TX_QUEUE, QUEUE_MAX, 0, resource)?;

        let total = (rx.size() as u64 + tx.size() as u64) * BUF;
        let dma = resource.dma_memory(total)?;
        let buf_phys = resource.phys_addr(&dma, 0)?;
        let buf = Mapping::new(&dma, 0, total, MAP_READ | MAP_WRITE)?;
        for id in 0..rx.size() {
            rx.post(
                id,
                Buffer {
                    phys: buf_phys + id as u64 * BUF,
                    len: BUF as u32,
                    device_writes: true,
                },
            )?;
        }
        let tx_free = (0..tx.size()).rev().collect();

        let mac = if features & F_MAC != 0 {
            let mut m = [0u8; 6];
            for (i, b) in m.iter_mut().enumerate() {
                *b = device.config::<u8>(i as u64);
            }
            m
        } else {
            local_mac()
        };
        let up = features & F_STATUS == 0 || device.config::<u16>(CFG_STATUS) & 1 != 0;
        device.driver_ok();
        device.notify(&rx);
        Ok(Self {
            device,
            rx,
            tx,
            irq,
            _msix: msix,
            _dma: dma,
            buf,
            buf_phys,
            tx_free,
            info: link::Info {
                mac,
                mtu: 1500,
                flags: if up { link::FLAG_UP } else { 0 },
            },
            dropped_rx: 0,
        })
    }

    /// Offset bufferu odesilani `id` v DMA pameti (za buffery prijmu).
    fn tx_offset(&self, id: u16) -> u64 {
        (self.rx.size() as u64 + id as u64) * BUF
    }

    /// Smycka ovladace: preruseni zarizeni a zpravy od sluzby. Chyba =
    /// vadne zarizeni.
    fn serve(&mut self, client: &Channel) -> Result<()> {
        loop {
            // Bez volneho bufferu odesilani nove ramce necist (protitlak);
            // konec sluzby ale hlidat porad.
            let client_mask = if self.tx_free.is_empty() {
                Signals::PEER_CLOSED
            } else {
                Signals::READABLE | Signals::PEER_CLOSED
            };
            let (_, observed) = wait_many(
                &[
                    (self.irq.handle(), Signals::SIGNALED),
                    (client.handle(), client_mask),
                ],
                None,
            )?;
            // Sluzba skoncila: zbyle ramce od ni uz nikoho nezajimaji. Bez
            // teto kontroly by se pri plne fronte odesilani (kanal se necte)
            // trvale nastaveny PEER_CLOSED tocil dokola.
            if observed[1].contains(Signals::PEER_CLOSED) {
                return Ok(());
            }
            // Potvrdit pred obsluhou: preruseni, ktere prijde behem ni,
            // znovu probudi.
            let _ = self.irq.ack();
            self.reclaim_tx()?;
            if !self.receive(client)? {
                return Ok(());
            }
            if !self.transmit(client)? {
                return Ok(());
            }
        }
    }

    /// Prijate ramce predat sluzbe a buffery vratit zarizeni. `false` =
    /// sluzba zavrela kanal.
    fn receive(&mut self, client: &Channel) -> Result<bool> {
        let mut reposted = false;
        let mut frame = [0u8; 1 + link::MAX_FRAME];
        frame[0] = link::FRAME;
        while let Some((id, len)) = self.rx.take_used_any()? {
            let len = len as usize;
            let off = id as u64 * BUF;
            if (HDR + 14..=HDR + link::MAX_FRAME).contains(&len) {
                let n = len - HDR;
                self.buf.read_bytes(off + HDR as u64, &mut frame[1..1 + n]);
                match client.write(&frame[..1 + n], Vec::new()) {
                    Ok(()) => {}
                    Err(Error::PeerClosed) => return Ok(false),
                    // Plna fronta sluzby: ramec zahodit.
                    Err(_) => self.dropped_rx += 1,
                }
            } else {
                self.dropped_rx += 1;
            }
            self.rx.post(
                id,
                Buffer {
                    phys: self.buf_phys + off,
                    len: BUF as u32,
                    device_writes: true,
                },
            )?;
            reposted = true;
        }
        if reposted {
            self.device.notify(&self.rx);
        }
        Ok(true)
    }

    /// Odeslane buffery vratit mezi volne.
    fn reclaim_tx(&mut self) -> Result<()> {
        while let Some((id, _)) = self.tx.take_used_any()? {
            if self.tx_free.contains(&id) {
                // Zarizeni vratilo buffer, ktery nemelo.
                return Err(Error::Corrupted);
            }
            self.tx_free.push(id);
        }
        Ok(())
    }

    /// Ramce od sluzby poslat, dokud jsou volne buffery. `false` = sluzba
    /// zavrela kanal.
    fn transmit(&mut self, client: &Channel) -> Result<bool> {
        let mut sent = false;
        while let Some(&id) = self.tx_free.last() {
            let msg = match client.try_read() {
                Ok(m) => m,
                Err(Error::ShouldWait) => break,
                Err(_) => return Ok(false),
            };
            let Some((&kind, frame)) = msg.data.split_first() else {
                continue;
            };
            if kind != link::FRAME || frame.len() < 14 || frame.len() > link::MAX_FRAME {
                continue;
            }
            let off = self.tx_offset(id);
            // Hlavicka virtio-net: bez offloadu kontrolnich souctu a GSO.
            self.buf.write_bytes(off, &[0u8; HDR]);
            self.buf.write_bytes(off + HDR as u64, frame);
            self.tx.post(
                id,
                Buffer {
                    phys: self.buf_phys + off,
                    len: (HDR + frame.len()) as u32,
                    device_writes: false,
                },
            )?;
            self.tx_free.pop();
            sent = true;
        }
        if sent {
            self.device.notify(&self.tx);
        }
        Ok(true)
    }
}
