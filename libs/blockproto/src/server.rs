//! Strana ovladace: spolecna obsluha klientu pro vsechny ovladace disku
//! (virtio-blk, NVMe, AHCI) vcetne ochrany cizich disku.
//!
//! Ovladac implementuje [`Disks`] (cteni, zapis, flush, DISCARD po
//! sektorech zarizeni) a zavola [`guard`] pro kazdy disk a pak [`serve`].
//! Kontrola rozsahu, preklad na nas oddil GPT a zakaz zapisu na cizi disk
//! jsou tady - na jednom miste pro vsechny ovladace (`diskguard`).
//!
//! Klient adresuje vzdy po 512 B ([`SECTOR`]); disk muze mit vetsi logicky
//! sektor (NVMe naformatovane na 4 KiB, SATA 4Kn). Server adresy prepocita:
//! zarovnany pristup jde primo, nezarovnane cteni precte cele sektory
//! a vrati jen pozadovanou cast, nezarovnany zapis precte okrajove sektory,
//! zmeni v nich pozadovanou cast a zapise cele sektory. TRIM se omezi na
//! cele sektory uvnitr rozsahu (je to jen napoveda).
//!
//! Ovladac s vic disky (AHCI s vic porty, NVMe s vic jmennymi prostory)
//! vytvori kanal pro kazdy disk a posle ho spravci zarizeni
//! ([`report_disks`]): `block` + konec kanalu pro klienta, nakonec `done`.

use alloc::vec;
use alloc::vec::Vec;

use diskguard::Decision;
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::hw::Resource;
use libos::{Channel, Error, Mapping, Result, Signals, Vmo, println};

use crate::{MAX_SECTORS, Request, SECTOR, encode_reply, feature, op};

/// Nejvic bajtu v jednom volani `Disks::read` / `Disks::write` (velikost
/// datoveho bufferu ovladacu).
pub const IO_MAX: usize = MAX_SECTORS as usize * SECTOR;

/// Disky jednoho ovladace. Indexy `d` jsou `0..count()`; `lba` jsou
/// sektory zarizeni (velikost [`Disks::sector_size`]). Preklad oddilu,
/// prepocet z 512 B a kontrolu rozsahu dela server. Server vola `read`,
/// `write` a `discard` jen s nenulovym poctem celych sektoru a nejvys
/// [`IO_MAX`] bajty.
pub trait Disks {
    fn count(&self) -> usize;
    /// Velikost disku v sektorech zarizeni.
    fn sectors(&self, d: usize) -> u64;
    /// Logicky sektor zarizeni v bajtech: mocnina dvou 512..=[`IO_MAX`].
    fn sector_size(&self, d: usize) -> usize;
    /// Vlastnosti zarizeni (bity [`feature`]).
    fn features(&self, d: usize) -> u64;
    /// Precte `out.len() / sector_size` sektoru.
    fn read(&mut self, d: usize, lba: u64, out: &mut [u8]) -> Result<()>;
    /// Zapise `data.len() / sector_size` sektoru.
    fn write(&mut self, d: usize, lba: u64, data: &[u8]) -> Result<()>;
    /// Zapise mezipamet zarizeni na medium; bez podpory `NOT_SUPPORTED`.
    fn flush(&mut self, d: usize) -> Result<()>;
    /// TRIM sektoru `lba..lba+count`; bez podpory `NOT_SUPPORTED`.
    fn discard(&mut self, d: usize, lba: u64, count: u64) -> Result<()>;
    /// Test ochrany DMA (`op::TEST_DMA_PROTECTION`).
    fn test_dma_protection(&mut self, d: usize) -> Result<u64>;
}

/// Stranka pro test ochrany DMA (`op::TEST_DMA_PROTECTION`): bezna pamet
/// ovladace (ne DMA pamet, s IOMMU neni v domene zarizeni) naplnena vzorem.
/// Ovladac do ni necha zarizeni precist jeden sektor; zapis jde jen do
/// vlastni pameti ovladace, takze test nic neohrozi.
pub struct DmaProbe {
    _vmo: Vmo,
    map: Mapping,
    /// Adresa stranky pro zarizeni.
    pub phys: u64,
    len: usize,
}

impl DmaProbe {
    const PATTERN: u8 = 0xAA;
    const PAGE: usize = 4096;

    /// Stranka pro prenos `len` bajtu (nejvys 4 KiB: jedna stranka, jeden
    /// popis prenosu).
    pub fn new(resource: &Resource, len: usize) -> Result<Self> {
        if len == 0 || len > Self::PAGE {
            return Err(Error::NotSupported);
        }
        let vmo = Vmo::create(Self::PAGE as u64)?;
        let map = Mapping::new(&vmo, 0, Self::PAGE as u64, MAP_READ | MAP_WRITE)?;
        map.write_bytes(0, &vec![Self::PATTERN; len]);
        let phys = resource.phys_addr(&vmo, 0)?;
        Ok(Self {
            _vmo: vmo,
            map,
            phys,
            len,
        })
    }

    /// Vysledek testu: 1 = stranka nezmenena (zapis zablokovan), 0 =
    /// zarizeni ji prepsalo.
    pub fn untouched(&self) -> u64 {
        let mut back = vec![0u8; self.len];
        self.map.read_bytes(0, &mut back);
        back.iter().all(|&b| b == Self::PATTERN) as u64
    }
}

/// Precte souvisly usek sektoru zarizeni po kusech nejvys [`IO_MAX`].
fn read_dev(disks: &mut impl Disks, d: usize, lba: u64, buf: &mut [u8]) -> Result<()> {
    let ss = disks.sector_size(d);
    for (i, chunk) in buf.chunks_mut(IO_MAX).enumerate() {
        disks.read(d, lba + (i * IO_MAX / ss) as u64, chunk)?;
    }
    Ok(())
}

/// Zapise souvisly usek sektoru zarizeni po kusech nejvys [`IO_MAX`].
fn write_dev(disks: &mut impl Disks, d: usize, lba: u64, buf: &[u8]) -> Result<()> {
    let ss = disks.sector_size(d);
    for (i, chunk) in buf.chunks(IO_MAX).enumerate() {
        disks.write(d, lba + (i * IO_MAX / ss) as u64, chunk)?;
    }
    Ok(())
}

/// Rozebere disk `d` a zaloguje, co klient uvidi (`name` = jmeno ovladace).
pub fn guard(name: &str, disks: &mut impl Disks, d: usize) -> Decision {
    let sectors = disks.sectors(d);
    let ss = disks.sector_size(d);
    let decision = if ss > IO_MAX || !diskguard::valid_sector_size(ss) {
        // Ovladac by takovy disk nemel nabidnout; radeji jen cteni nicim.
        diskguard::decide(sectors, ss, |_, _| false)
    } else {
        diskguard::decide(sectors, ss, |lba, buf| {
            lba.checked_add((buf.len() / ss) as u64)
                .is_some_and(|end| end <= sectors)
                && read_dev(disks, d, lba, buf).is_ok()
        })
    };
    let decision = if disks.features(d) & feature::READ_ONLY != 0 {
        Decision {
            writable: false,
            ..decision
        }
    } else {
        decision
    };
    println!(
        "{name}: disk {d}: {}, {} MiB od sektoru {}, sektor {} B, {}",
        decision.kind.describe(),
        decision.sectors * ss as u64 / (1024 * 1024),
        decision.start,
        ss,
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

/// Usek klienta (po 512 B) prelozeny na sektory zarizeni.
struct Span {
    /// Prvni sektor zarizeni (absolutne).
    lba: u64,
    /// Pocet sektoru zarizeni, ktere usek zasahuje.
    count: u64,
    /// Bajt zacatku useku v prvnim sektoru.
    head: usize,
    /// Usek zacina i konci na hranici sektoru zarizeni.
    aligned: bool,
}

/// Velikost videneho useku v sektorech klienta (512 B).
fn client_sectors(view: &Decision) -> u64 {
    view.sectors * (view.sector_size / SECTOR) as u64
}

/// Preklad useku klienta `lba..lba+count` (512 B) na sektory zarizeni.
fn span(view: &Decision, lba: u64, count: u64) -> Result<Span> {
    let end = lba.checked_add(count).ok_or(Error::OutOfRange)?;
    if end > client_sectors(view) {
        return Err(Error::OutOfRange);
    }
    let r = (view.sector_size / SECTOR) as u64;
    let first = lba / r;
    let last = end.div_ceil(r);
    let at = view.translate(first, last - first).ok_or(Error::OutOfRange)?;
    Ok(Span {
        lba: at,
        count: last - first,
        head: ((lba % r) as usize) * SECTOR,
        aligned: lba.is_multiple_of(r) && end.is_multiple_of(r),
    })
}

/// Cteni useku klienta (`count` sektoru po 512 B, nejvys `MAX_SECTORS`).
fn read_client(disks: &mut impl Disks, d: usize, view: &Decision, lba: u64, count: u32) -> Result<Vec<u8>> {
    let s = span(view, lba, count as u64)?;
    let len = count as usize * SECTOR;
    let mut buf = vec![0u8; s.count as usize * view.sector_size];
    read_dev(disks, d, s.lba, &mut buf)?;
    if s.aligned {
        return Ok(buf);
    }
    Ok(buf[s.head..s.head + len].to_vec())
}

/// Zapis useku klienta; nezarovnany okraj = cteni, zmena, zapis sektoru.
fn write_client(disks: &mut impl Disks, d: usize, view: &Decision, lba: u64, data: &[u8]) -> Result<()> {
    let s = span(view, lba, (data.len() / SECTOR) as u64)?;
    if s.aligned {
        return write_dev(disks, d, s.lba, data);
    }
    let ss = view.sector_size;
    let mut buf = vec![0u8; s.count as usize * ss];
    // Okrajove sektory, ktere zapis zasahuje jen zcasti (muze to byt jeden).
    if s.head != 0 {
        read_dev(disks, d, s.lba, &mut buf[..ss])?;
    }
    let tail_partial = !(s.head + data.len()).is_multiple_of(ss);
    if tail_partial && (s.count > 1 || s.head == 0) {
        let at = buf.len() - ss;
        read_dev(disks, d, s.lba + s.count - 1, &mut buf[at..])?;
    }
    buf[s.head..s.head + data.len()].copy_from_slice(data);
    write_dev(disks, d, s.lba, &buf)
}

/// TRIM jen celych sektoru zarizeni uvnitr useku klienta.
fn discard_client(disks: &mut impl Disks, d: usize, view: &Decision, lba: u64, count: u64) -> Result<()> {
    span(view, lba, count)?;
    let r = (view.sector_size / SECTOR) as u64;
    let first = lba.div_ceil(r);
    let end = (lba + count) / r;
    if end <= first {
        return Ok(());
    }
    disks.discard(d, view.start + first, end - first)
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
    match req.op {
        op::INFO => (Ok(client_sectors(view)), none()),
        op::READ => {
            if req.count > MAX_SECTORS {
                return (Err(Error::OutOfRange), none());
            }
            if let Err(e) = span(view, req.lba, req.count as u64) {
                return (Err(e), none());
            }
            // Pocet 0 k ovladaci nesmi dojit: u ATA znamena 65536 sektoru.
            if req.count == 0 {
                return (Ok(0), none());
            }
            match read_client(disks, d, view, req.lba, req.count) {
                Ok(out) => (Ok(0), out),
                Err(e) => (Err(e), crate::zeroed_sectors(req.count)),
            }
        }
        op::WRITE => {
            if !view.writable {
                return (Err(Error::ReadOnly), none());
            }
            if req.count > MAX_SECTORS || data.len() != req.count as usize * SECTOR {
                return (Err(Error::InvalidArgs), none());
            }
            match span(view, req.lba, req.count as u64) {
                Ok(_) if req.count == 0 => (Ok(0), none()),
                Ok(_) => (write_client(disks, d, view, req.lba, data).map(|_| 0), none()),
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
            (discard_client(disks, d, view, req.lba, req.count as u64).map(|_| 0), none())
        }
        op::TEST_DMA_PROTECTION => (disks.test_dma_protection(d), none()),
        _ => (Err(Error::NotSupported), none()),
    }
}
