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

use diskguard::{Decision, Kind};
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
    /// Vymenne medium (USB mass storage s bitem RMB): jen na nem smi byt
    /// oddil media zapisovatelny ([`diskguard::media`]). Vychozi ne.
    fn removable(&self, _d: usize) -> bool {
        false
    }
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
    /// Model disku pro instalator (z identifikace zarizeni).
    /// Handle, jehoz zavreni (`PEER_CLOSED`) znamena odpojene zarizeni
    /// (kanal USB): [`serve`] pak skonci a zavre kanaly klientu, proces
    /// media vi, ze medium zmizelo.
    fn gone(&self) -> Option<&libos::Handle> {
        None
    }

    fn model(&self, _d: usize) -> alloc::string::String {
        alloc::string::String::new()
    }
    /// Nejvic bajtu v jednom volani `read` (nasobek sektoru, aspon
    /// [`IO_MAX`]). Ovladac s pomalym prikazem (USB: CBW, data, CSW) vrati
    /// vic: server pak spoji po sobe jdouci cteni klienta do jednoho
    /// prikazu ([`serve`]). `write` dostane vzdy nejvys [`IO_MAX`].
    fn max_read(&self, _d: usize) -> usize {
        IO_MAX
    }
}

/// Nejvic bajtu, ktere server spoji z po sobe jdoucich cteni klienta.
pub const BATCH_MAX: usize = 1024 * 1024;

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

/// Precte souvisly usek sektoru zarizeni po kusech nejvys
/// [`Disks::max_read`].
fn read_dev(disks: &mut impl Disks, d: usize, lba: u64, buf: &mut [u8]) -> Result<()> {
    let ss = disks.sector_size(d);
    let max = (disks.max_read(d).clamp(IO_MAX, BATCH_MAX) / ss * ss).max(ss);
    for (i, chunk) in buf.chunks_mut(max).enumerate() {
        disks.read(d, lba + (i * max / ss) as u64, chunk)?;
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

/// Oddil pro logy na disku `d` (`diskguard::log_partition`).
fn log_view(disks: &mut impl Disks, d: usize) -> Option<Decision> {
    let sectors = disks.sectors(d);
    let ss = disks.sector_size(d);
    if ss > IO_MAX || !diskguard::valid_sector_size(ss) || disks.features(d) & feature::READ_ONLY != 0 {
        return None;
    }
    diskguard::log_partition(sectors, ss, |lba, buf| {
        lba.checked_add((buf.len() / ss) as u64).is_some_and(|end| end <= sectors) && read_dev(disks, d, lba, buf).is_ok()
    })
}

/// Oddily media na disku `d` (`diskguard::media_partitions`); zapis jen
/// na vymennem zarizeni bez ochrany proti zapisu.
fn media_views(disks: &mut impl Disks, d: usize) -> Vec<Decision> {
    let sectors = disks.sectors(d);
    let ss = disks.sector_size(d);
    if ss > IO_MAX || !diskguard::valid_sector_size(ss) {
        return Vec::new();
    }
    let removable = disks.removable(d) && disks.features(d) & feature::READ_ONLY == 0;
    diskguard::media_partitions(sectors, ss, removable, |lba, buf| {
        lba.checked_add((buf.len() / ss) as u64).is_some_and(|end| end <= sectors) && read_dev(disks, d, lba, buf).is_ok()
    })
}

/// Klient jednoho disku.
pub struct Client {
    pub disk: usize,
    pub view: Decision,
    pub channel: Channel,
    /// Raw kanal instalatoru (`None` = bezny kanal): (tabulka je platna,
    /// posledni prectena GPT disku). Zapis do oblasti GPT ji zneplatni.
    pub raw_table: Option<(bool, Option<diskguard::gpt::Table>)>,
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
            raw_table: None,
        });
        // Zive USB: oddil pro logy (FAT32 MO-LOGY) jako dalsi klient.
        if let Some(log) = log_view(disks, d) {
            println!("{name}: disk {d}: oddil pro logy MO-LOGY, {} MiB", log.sectors * log.sector_size as u64 >> 20);
            let (mine, theirs) = Channel::create()?;
            bootstrap.write(b"block-logy", vec![theirs.into_handle()])?;
            clients.push(Client {
                disk: d,
                view: log,
                channel: mine,
                raw_table: None,
            });
        }
        // Oddily vymenneho media (FAT, exFAT, NTFS): kanal na kazdy oddil,
        // init pro nej spusti proces media (docs/cs/media.md). Zprava:
        // `block-media <fs> <index> <model>`.
        for v in media_views(disks, d) {
            let Kind::Media { index, fs } = v.kind else { continue };
            let part = if index == diskguard::media::WHOLE { alloc::string::String::from("cely disk") } else { alloc::format!("oddil {index}") };
            println!(
                "{name}: disk {d}: medium {} {part}, {} MiB, {}",
                fs.name(),
                v.sectors * v.sector_size as u64 >> 20,
                if v.writable { "zapis povolen" } else { "JEN PRO CTENI" }
            );
            let (mine, theirs) = Channel::create()?;
            let text = alloc::format!("block-media {} {} {}", fs.name(), index, disks.model(d));
            bootstrap.write(text.as_bytes(), vec![theirs.into_handle()])?;
            clients.push(Client {
                disk: d,
                view: v,
                channel: mine,
                raw_table: None,
            });
        }
        // Raw kanal instalatoru (cely disk, zapis jen do GPT a nasich
        // oddilu). Init ho drzi a preda jen instalatoru; zprava nese model.
        let ss = disks.sector_size(d);
        if ss <= IO_MAX && diskguard::valid_sector_size(ss) {
            let raw = Decision {
                kind: diskguard::Kind::Installer,
                start: 0,
                sectors: disks.sectors(d),
                sector_size: ss,
                writable: disks.features(d) & feature::READ_ONLY == 0,
            };
            let (mine, theirs) = Channel::create()?;
            let text = alloc::format!("block-raw {} {}", ss, disks.model(d));
            bootstrap.write(text.as_bytes(), vec![theirs.into_handle()])?;
            clients.push(Client {
                disk: d,
                view: raw,
                channel: mine,
                raw_table: Some((false, None)),
            });
        }
    }
    bootstrap.write(b"done", Vec::new())?;
    Ok(clients)
}

/// Obsluhuje klienty, dokud vsichni nezavrou kanal. Pozadavky se
/// zpracovavaji v poradi prichodu a odpovedi jdou ve stejnem poradi.
/// Cteni, ktera klient poslal bez cekani (`BlockClient::read`) a ktera na
/// sebe navazuji, server u ovladace s vetsim [`Disks::max_read`] precte
/// jednim volanim (`read_batch`).
///
/// Vraci `true`, kdyz skoncil kvuli odpojenemu zarizeni ([`Disks::gone`]):
/// pak uz nema smysl zapisovat mezipamet.
pub fn serve(name: &str, disks: &mut impl Disks, mut clients: Vec<Client>) -> bool {
    while !clients.is_empty() {
        let mut items: Vec<_> = clients
            .iter()
            .map(|c| (c.channel.handle(), Signals::READABLE | Signals::PEER_CLOSED))
            .collect();
        if let Some(g) = disks.gone() {
            items.push((g, Signals::PEER_CLOSED));
        }
        let mut observed = match libos::wait_many(&items, None) {
            Ok((_, o)) => o,
            Err(e) => {
                println!("{name}: cekani na klienty selhalo: {e}");
                return false;
            }
        };
        drop(items);
        if observed.len() > clients.len() {
            if observed[clients.len()].intersects(Signals::PEER_CLOSED) {
                println!("{name}: zarizeni odpojeno, koncim");
                return true;
            }
            observed.truncate(clients.len());
        }
        let mut closed = Vec::new();
        for (i, s) in observed.iter().enumerate() {
            let c = &mut clients[i];
            if s.intersects(Signals::READABLE) {
                match c.channel.try_read() {
                    Ok(msg) => {
                        let mut msg = Some(msg);
                        // Navazujici cteni ve fronte: jednim volanim ovladace.
                        if disks.max_read(c.disk) > IO_MAX {
                            match read_batch(disks, c, msg.take().unwrap()) {
                                Ok(rest) => msg = rest,
                                Err(()) => {
                                    closed.push(i);
                                    continue;
                                }
                            }
                        }
                        if let Some(msg) = msg
                            && !reply_one(name, disks, c, &msg.data)
                        {
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
    false
}

/// Jeden pozadavek klienta a odpoved; `false` = kanal je zavreny.
fn reply_one(name: &str, disks: &mut impl Disks, c: &mut Client, raw: &[u8]) -> bool {
    let (result, data) = match Request::decode(raw) {
        Some((req, payload)) => match raw_check(disks, c, &req) {
            Ok(()) => handle(disks, c.disk, &c.view, req, payload),
            Err(e) => {
                println!("{name}: disk {}: raw zapis LBA {} ({} s.) odmitnut", c.disk, req.lba, req.count);
                (Err(e), Vec::new())
            }
        },
        None => (Err(Error::InvalidArgs), Vec::new()),
    };
    c.channel.write(&encode_reply(result, &data), Vec::new()).is_ok()
}

/// Platne cteni pro spojeni: (lba, pocet sektoru po 512 B).
fn batch_read(view: &Decision, raw: &[u8]) -> Option<(u64, u32)> {
    let (req, _) = Request::decode(raw)?;
    (req.op == op::READ && req.count > 0 && req.count <= MAX_SECTORS && span(view, req.lba, req.count as u64).is_ok())
        .then_some((req.lba, req.count))
}

/// Spoji cteni `first` s navazujicimi ctenimi ve fronte klienta (celkem
/// nejvys [`Disks::max_read`]), precte je jednim volanim a odpovi na kazde
/// zvlast. Vraci prvni zpravu, ktera nenavazuje (zpracuje ji volajici),
/// `Err` = kanal je zavreny. Chyba spojeneho cteni: kazde cteni znovu samo,
/// at chybu dostane to, ktereho se tyka.
fn read_batch(disks: &mut impl Disks, c: &mut Client, first: libos::Message) -> core::result::Result<Option<libos::Message>, ()> {
    let Some((lba, count)) = batch_read(&c.view, &first.data) else { return Ok(Some(first)) };
    // Nezarovnany usek (sektor zarizeni > 512 B) cte o sektor navic.
    let extra = if c.view.sector_size > SECTOR { c.view.sector_size } else { 0 };
    let limit = disks.max_read(c.disk).min(BATCH_MAX).saturating_sub(extra);
    let mut parts: Vec<u32> = alloc::vec![count];
    let mut total = count as u64;
    let mut rest = None;
    while (total as usize + MAX_SECTORS as usize) * SECTOR <= limit {
        let m = match c.channel.try_read() {
            Ok(m) => m,
            Err(_) => break,
        };
        match batch_read(&c.view, &m.data) {
            Some((l, n)) if l == lba + total && span(&c.view, lba, total + n as u64).is_ok() => {
                parts.push(n);
                total += n as u64;
            }
            _ => {
                rest = Some(m);
                break;
            }
        }
    }
    let send = |c: &Client, r: Result<u64>, data: &[u8]| c.channel.write(&encode_reply(r, data), Vec::new()).map_err(|_| ());
    if parts.len() == 1 {
        let (r, data) = handle(disks, c.disk, &c.view, Request { op: op::READ, count, lba }, &[]);
        send(c, r, &data)?;
        return Ok(rest);
    }
    match read_client(disks, c.disk, &c.view, lba, total as u32) {
        Ok(buf) => {
            let mut at = 0;
            for n in parts {
                let len = n as usize * SECTOR;
                send(c, Ok(0), &buf[at..at + len])?;
                at += len;
            }
        }
        Err(_) => {
            let mut l = lba;
            for n in parts {
                let (r, data) = handle(disks, c.disk, &c.view, Request { op: op::READ, count: n, lba: l }, &[]);
                send(c, r, &data)?;
                l += n as u64;
            }
        }
    }
    Ok(rest)
}

/// Raw kanal instalatoru: zapis a TRIM jen do struktur GPT a nasich
/// oddilu podle aktualni tabulky na disku. Ostatni kanaly beze zmeny.
fn raw_check(disks: &mut impl Disks, c: &mut Client, req: &Request) -> Result<()> {
    let Some(cache) = c.raw_table.as_mut() else { return Ok(()) };
    if req.op != op::WRITE && req.op != op::DISCARD {
        return Ok(());
    }
    let ss = c.view.sector_size;
    let r = (ss / SECTOR) as u64;
    let lba = req.lba / r;
    let count = (req.lba + req.count as u64).div_ceil(r).saturating_sub(lba).max(1);
    let sectors = c.view.sectors;
    if !cache.0 {
        let d = c.disk;
        let t = match diskguard::gpt::read_table(sectors, ss, &mut |lba, buf| read_dev(disks, d, lba, buf).is_ok()) {
            diskguard::gpt::Read::Table(t) => Some(t),
            _ => None,
        };
        *cache = (true, t);
    }
    let table = cache.1.as_ref();
    if !diskguard::install::raw_write_allowed(table, sectors, ss, lba, count) {
        return Err(Error::AccessDenied);
    }
    // Zapis do oblasti GPT zmeni tabulku: pri dalsim zapisu ji precist znovu.
    let tsec = diskguard::gpt::table_sectors(ss);
    let gpt_area = lba < diskguard::gpt::first_usable(ss) || lba + count > sectors.saturating_sub(1 + tsec);
    if gpt_area {
        cache.0 = false;
    }
    Ok(())
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

/// Cteni useku klienta (`count` sektoru po 512 B; z `handle` nejvys
/// `MAX_SECTORS`, ze spojeneho cteni nejvys [`Disks::max_read`]).
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
