//! Strana ovladace: spolecna obsluha klientu pro vsechny ovladace disku
//! (virtio-blk, NVMe, AHCI) vcetne ochrany cizich disku.
//!
//! Ovladac implementuje [`Disks`] (cteni, zapis, flush, DISCARD po
//! sektorech zarizeni) a zavola [`guard`] pro kazdy disk a pak [`serve`].
//! Kontrola rozsahu, preklad na nas oddil GPT a zakaz zapisu na cizi disk
//! jsou tady - na jednom miste pro vsechny ovladace (`diskguard`).
//!
//! Ovladac s vic disky (AHCI s vic porty, NVMe s vic jmennymi prostory)
//! vytvori kanal pro kazdy disk a posle ho spravci zarizeni
//! ([`report_disks`]): `block` + konec kanalu pro klienta, nakonec `done`.

use alloc::vec;
use alloc::vec::Vec;

use diskguard::Decision;
use libos::{Channel, Error, Result, Signals, println};

use crate::{MAX_SECTORS, Request, SECTOR, encode_reply, feature, op};

/// Disky jednoho ovladace. Indexy `d` jsou `0..count()`; `lba` jsou sektory
/// zarizeni po 512 B (preklad oddilu a kontrolu rozsahu dela server).
/// Server vola `read`, `write` a `discard` jen s nenulovym poctem sektoru.
pub trait Disks {
    fn count(&self) -> usize;
    /// Velikost disku v sektorech.
    fn sectors(&self, d: usize) -> u64;
    /// Vlastnosti zarizeni (bity [`feature`]).
    fn features(&self, d: usize) -> u64;
    /// Precte `out.len() / 512` sektoru (nejvys [`MAX_SECTORS`]).
    fn read(&mut self, d: usize, lba: u64, out: &mut [u8]) -> Result<()>;
    /// Zapise `data.len() / 512` sektoru (nejvys [`MAX_SECTORS`]).
    fn write(&mut self, d: usize, lba: u64, data: &[u8]) -> Result<()>;
    /// Zapise mezipamet zarizeni na medium; bez podpory `NOT_SUPPORTED`.
    fn flush(&mut self, d: usize) -> Result<()>;
    /// TRIM sektoru `lba..lba+count`; bez podpory `NOT_SUPPORTED`.
    fn discard(&mut self, d: usize, lba: u64, count: u64) -> Result<()>;
    /// Test ochrany DMA (`op::TEST_DMA_PROTECTION`).
    fn test_dma_protection(&mut self, d: usize) -> Result<u64>;
}

/// Rozebere disk `d` a zaloguje, co klient uvidi (`name` = jmeno ovladace).
pub fn guard(name: &str, disks: &mut impl Disks, d: usize) -> Decision {
    let sectors = disks.sectors(d);
    let decision = diskguard::decide(sectors, |lba, buf| {
        buf.chunks_mut(MAX_SECTORS as usize * SECTOR)
            .enumerate()
            .all(|(i, chunk)| {
                let at = lba + (i * MAX_SECTORS as usize) as u64;
                at.checked_add((chunk.len() / SECTOR) as u64)
                    .is_some_and(|end| end <= sectors)
                    && disks.read(d, at, chunk).is_ok()
            })
    });
    let decision = if disks.features(d) & feature::READ_ONLY != 0 {
        Decision {
            writable: false,
            ..decision
        }
    } else {
        decision
    };
    println!(
        "{name}: disk {d}: {}, {} MiB od sektoru {}, {}",
        decision.kind.describe(),
        decision.sectors * SECTOR as u64 / (1024 * 1024),
        decision.start,
        if decision.writable {
            "zapis povolen"
        } else {
            "JEN PRO CTENI"
        }
    );
    decision
}

/// Klient jednoho disku.
pub struct Client {
    pub disk: usize,
    pub view: Decision,
    pub channel: Channel,
}

/// Pro kazdy disk zjisti [`guard`], vytvori kanal, posle ho po `bootstrap`
/// spravci zarizeni (`block`) a nakonec `done`. Vraci klienty pro [`serve`].
pub fn report_disks(name: &str, disks: &mut impl Disks, bootstrap: &Channel) -> Result<Vec<Client>> {
    let mut clients = Vec::new();
    for d in 0..disks.count() {
        let view = guard(name, disks, d);
        let (mine, theirs) = Channel::create()?;
        bootstrap.write(b"block", vec![theirs.into_handle()])?;
        clients.push(Client {
            disk: d,
            view,
            channel: mine,
        });
    }
    bootstrap.write(b"done", Vec::new())?;
    Ok(clients)
}

/// Obsluhuje klienty, dokud vsichni nezavrou kanal. Pozadavky se
/// zpracovavaji po jednom (protokol verze 1 je synchronni).
pub fn serve(name: &str, disks: &mut impl Disks, mut clients: Vec<Client>) {
    while !clients.is_empty() {
        let items: Vec<_> = clients
            .iter()
            .map(|c| (c.channel.handle(), Signals::READABLE | Signals::PEER_CLOSED))
            .collect();
        let observed = match libos::wait_many(&items, None) {
            Ok((_, o)) => o,
            Err(e) => {
                println!("{name}: cekani na klienty selhalo: {e}");
                return;
            }
        };
        drop(items);
        let mut closed = Vec::new();
        for (i, s) in observed.iter().enumerate() {
            let c = &clients[i];
            if s.intersects(Signals::READABLE) {
                match c.channel.try_read() {
                    Ok(msg) => {
                        let (result, data) = match Request::decode(&msg.data) {
                            Some((req, payload)) => handle(disks, c.disk, &c.view, req, payload),
                            None => (Err(Error::InvalidArgs), Vec::new()),
                        };
                        if c.channel.write(&encode_reply(result, &data), Vec::new()).is_err() {
                            closed.push(i);
                        }
                    }
                    Err(Error::ShouldWait) => {}
                    Err(_) => closed.push(i),
                }
            } else if s.intersects(Signals::PEER_CLOSED) {
                closed.push(i);
            }
        }
        for &i in closed.iter().rev() {
            clients.remove(i);
        }
    }
}

/// Jeden pozadavek klienta disku `d` s pohledem `view`.
pub fn handle(
    disks: &mut impl Disks,
    d: usize,
    view: &Decision,
    req: Request,
    data: &[u8],
) -> (Result<u64>, Vec<u8>) {
    let none = Vec::new;
    let range = |count: u64| view.translate(req.lba, count).ok_or(Error::OutOfRange);
    match req.op {
        op::INFO => (Ok(view.sectors), none()),
        op::READ => {
            if req.count > MAX_SECTORS {
                return (Err(Error::OutOfRange), none());
            }
            let at = match range(req.count as u64) {
                Ok(a) => a,
                Err(e) => return (Err(e), none()),
            };
            // Pocet 0 k ovladaci nesmi dojit: u ATA znamena 65536 sektoru.
            if req.count == 0 {
                return (Ok(0), none());
            }
            let mut out = crate::zeroed_sectors(req.count);
            let r = disks.read(d, at, &mut out);
            if r.is_err() {
                out.fill(0);
            }
            (r.map(|_| 0), out)
        }
        op::WRITE => {
            if !view.writable {
                return (Err(Error::ReadOnly), none());
            }
            if req.count > MAX_SECTORS || data.len() != req.count as usize * SECTOR {
                return (Err(Error::InvalidArgs), none());
            }
            match range(req.count as u64) {
                Ok(_) if req.count == 0 => (Ok(0), none()),
                Ok(at) => (disks.write(d, at, data).map(|_| 0), none()),
                Err(e) => (Err(e), none()),
            }
        }
        op::FEATURES => {
            let mut f = disks.features(d);
            if !view.writable {
                f = (f | feature::READ_ONLY) & !feature::DISCARD;
            }
            (Ok(f), none())
        }
        op::SYNC => (Ok(req.lba), none()),
        op::FLUSH => (disks.flush(d).map(|_| 0), none()),
        op::DISCARD => {
            if !view.writable {
                return (Err(Error::ReadOnly), none());
            }
            if req.count == 0 {
                return (Ok(0), none());
            }
            match range(req.count as u64) {
                Ok(at) => (disks.discard(d, at, req.count as u64).map(|_| 0), none()),
                Err(e) => (Err(e), none()),
            }
        }
        op::TEST_DMA_PROTECTION => (disks.test_dma_protection(d), none()),
        _ => (Err(Error::NotSupported), none()),
    }
}
