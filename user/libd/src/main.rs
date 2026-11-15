//! Sluzba knihoven `libd` (`docs/cs/dynamicke-linkovani.md`, DL-4).
//!
//! Drzi zapecetene sablony segmentu sdilenych knihoven z `/system/lib`
//! (initfs `lib/<jmeno>`) a rozdava je zavadeci `ld.so` (a `spawn` pro
//! samotny zavadec): kod a konstanty jako zapecetene VMO (vsechny procesy
//! mapuji tytez stranky), data jako sablonu pro klon s kopii pri zapisu.
//! Sablona se hleda podle jmena a obsahu (BLAKE3): stejna knihovna = tytez
//! stranky.
//!
//! Spojeni tvori strom: korenove je bootstrap kanal (drzi ho adapter sluzby
//! v initu); `Connect` z kterekoli relace vytvori relaci pro dite (bez
//! odpovedi). Vse od klientu je **neduveryhodne**: `libdproto` hlida zpravy
//! a jmena (jen soubor v `/system/lib`), tady meze souboru, segmentu
//! a poctu spojeni (na relaci = na proces, a celkem), ELF pres `libs/elf`
//! (TEXTREL a REL odmitnuty).
//!
//! Knihovny z disku (DL-4c, `OpenImage`): libd disk necte a relaci FS nema.
//! Obsah posle klient (zavadec ho precetl relaci FS procesu, tedy s pravy
//! aplikace) jako VMO; libd ho zkopiruje k sobe jednou, spocita BLAKE3
//! a dal pracuje jen s kopii. Jmeno z `OpenImage` se do mapy jmen nedava
//! (hledani podle jmena = jen initfs). Sablon z disku je nejvys
//! `MAX_DISK_TEMPLATES` a `MAX_DISK_BYTES`, nad mez se zahodi nejdele
//! nepouzita (procesy maji sve handly, stranky zustanou jim).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use abi::object::Rights;
use abi::vm::{MAP_EXECUTE, MAP_READ, MAP_WRITE};
use libdproto::{
    Library, MAX_CHILD_CONNECTIONS, MAX_CONNECTIONS, MAX_DISK_BYTES, MAX_DISK_TEMPLATES, MAX_FILE, MAX_SEGMENTS, Reply, Request, Segment, Stats,
    kind,
};
use libos::initfs::Initfs;
use libos::{Channel, Error, Handle, Port, Result, Signals, Startup, Vmo, println};

libos::entry!(main);

/// Sablona knihovny: rozlozeni a VMO segmentu (kod zapeceteny).
struct Template {
    lib: Library,
    vmos: Vec<Vmo>,
}

impl Template {
    /// Pamet sablony (soucet delek VMO segmentu).
    fn bytes(&self) -> u64 {
        self.lib.segments.iter().map(|s| s.len).sum()
    }
}

/// Sablona z disku: posledni pouziti (pocitadlo `Libd::clock`) a velikost.
struct DiskUse {
    last: u64,
    bytes: u64,
}

/// Spojeni klienta.
struct Conn {
    ch: Channel,
    /// Relace, ze ktere vzniklo (`Connect`).
    parent: Option<u64>,
    /// Zijici relace vytvorene z teto.
    children: u32,
}

/// Cislo korenoveho spojeni (bootstrap kanal od initu).
const ROOT: u64 = 0;

struct Libd {
    fs: Initfs,
    page: u64,
    /// Jmeno -> hash obsahu, hash -> sablona.
    names: BTreeMap<String, [u8; 32]>,
    templates: BTreeMap<[u8; 32], Arc<Template>>,
    /// Sablony jen z disku (`OpenImage`) a jejich pouziti; sablona, kterou
    /// pouziva i initfs (`names`), tu neni a nezahazuje se.
    disk: BTreeMap<[u8; 32], DiskUse>,
    disk_bytes: u64,
    clock: u64,
    evicted: u64,
    conns: BTreeMap<u64, Conn>,
    next_id: u64,
    port: Port,
    opens: u64,
    hits: u64,
}

const WATCH: Signals = Signals(Signals::READABLE.0 | Signals::PEER_CLOSED.0);

fn main(mut s: Startup) -> i64 {
    if s.handles.is_empty() {
        println!("libd: chybi initfs");
        return 1;
    }
    let initfs = Vmo::from_handle(s.handles.remove(0));
    let fs = match Initfs::from_vmo(&initfs) {
        Ok(f) => f,
        Err(e) => {
            println!("libd: initfs nejde namapovat: {e}");
            return 1;
        }
    };
    let Ok(port) = Port::create() else { return 1 };
    let mut d = Libd {
        fs,
        page: libos::vm::page_size(),
        names: BTreeMap::new(),
        templates: BTreeMap::new(),
        disk: BTreeMap::new(),
        disk_bytes: 0,
        clock: 0,
        evicted: 0,
        conns: BTreeMap::new(),
        next_id: 0,
        port,
        opens: 0,
        hits: 0,
    };
    // Prvni spojeni: bootstrap kanal od initu.
    let root = Channel::from_handle(match s.bootstrap.handle().duplicate(None) {
        Ok(h) => h,
        Err(_) => return 1,
    });
    if d.add(root, None).is_err() {
        return 1;
    }
    println!("libd: bezim");
    loop {
        let Ok(p) = d.port.wait(None) else { continue };
        d.event(p.key, Signals(p.signals));
    }
}

impl Libd {
    /// Nove spojeni (hlidane portem).
    fn add(&mut self, ch: Channel, parent: Option<u64>) -> Result<u64> {
        let id = self.next_id;
        self.port.wait_async(ch.handle(), id, WATCH)?;
        self.next_id += 1;
        self.conns.insert(id, Conn { ch, parent, children: 0 });
        if let Some(p) = parent.and_then(|p| self.conns.get_mut(&p)) {
            p.children += 1;
        }
        Ok(id)
    }

    fn remove(&mut self, id: u64) {
        if let Some(c) = self.conns.remove(&id)
            && let Some(p) = c.parent.and_then(|p| self.conns.get_mut(&p))
        {
            p.children = p.children.saturating_sub(1);
        }
    }

    fn event(&mut self, id: u64, signals: Signals) {
        if signals.contains(Signals::READABLE) {
            // Vsechny cekajici zpravy, pak znovu hlidat.
            loop {
                let Some(c) = self.conns.get(&id) else { return };
                let Ok(msg) = c.ch.try_read() else { break };
                let Some((reply, handles)) = self.handle(id, &msg.data, msg.handles) else { continue };
                let Some(c) = self.conns.get(&id) else { return };
                if c.ch.write(&reply.encode(), handles).is_err() {
                    break;
                }
            }
            if let Some(c) = self.conns.get(&id)
                && self.port.wait_async(c.ch.handle(), id, WATCH).is_ok()
            {
                return;
            }
        }
        // Druha strana zavrela (nebo kanal nejde hlidat): spojeni pryc.
        self.remove(id);
    }

    /// Odpoved na zpravu (`None` = bez odpovedi: `Connect`).
    fn handle(&mut self, id: u64, data: &[u8], mut handles: Vec<Handle>) -> Option<(Reply, Vec<Handle>)> {
        let err = |e: Error| Some((Reply::Error(e.code()), Vec::new()));
        Some(match Request::decode(data) {
            Some(Request::Open { name }) if handles.is_empty() => match self.open(&name) {
                Ok(t) => match share(&t) {
                    Ok(h) => (Reply::Library(t.lib.clone()), h),
                    Err(e) => return err(e),
                },
                Err(e) => return err(e),
            },
            Some(Request::OpenImage { name, len }) if handles.len() == 1 => {
                let vmo = Vmo::from_handle(handles.remove(0));
                match self.open_image(&name, &vmo, len) {
                    Ok(t) => match share(&t) {
                        Ok(h) => (Reply::Library(t.lib.clone()), h),
                        Err(e) => return err(e),
                    },
                    Err(e) => return err(e),
                }
            }
            // Relace pro dite, bez odpovedi: nad mez se konec zahodi (dite
            // uvidi PEER_CLOSED a listek mu rekne, ze libd bezi dal).
            Some(Request::Connect) if handles.len() == 1 => {
                let ch = Channel::from_handle(handles.remove(0));
                let per_process = id != ROOT && self.conns.get(&id).is_some_and(|c| c.children >= MAX_CHILD_CONNECTIONS);
                if per_process {
                    println!("libd: relace {id}: prilis mnoho relaci pro deti ({MAX_CHILD_CONNECTIONS})");
                } else if self.conns.len() as u32 >= MAX_CONNECTIONS {
                    println!("libd: prilis mnoho spojeni ({MAX_CONNECTIONS})");
                } else if let Err(e) = self.add(ch, Some(id)) {
                    println!("libd: spojeni nejde hlidat: {e}");
                }
                return None;
            }
            Some(Request::Stats) if handles.is_empty() => (
                Reply::Stats(Stats {
                    templates: self.templates.len() as u64,
                    opens: self.opens,
                    hits: self.hits,
                    connections: self.conns.len() as u64,
                    disk_templates: self.disk.len() as u64,
                    evicted: self.evicted,
                }),
                Vec::new(),
            ),
            // Vadna zprava; prilozene handly se zahodi (zavrou).
            _ => return err(Error::InvalidArgs),
        })
    }

    /// Sablona knihovny `name` (vytvori ji pri prvnim pouziti).
    fn open(&mut self, name: &str) -> Result<Arc<Template>> {
        self.opens += 1;
        if let Some(t) = self.names.get(name).and_then(|h| self.templates.get(h)) {
            self.hits += 1;
            return Ok(t.clone());
        }
        let mut path = String::from("lib/");
        path.push_str(name);
        let image = self.fs.find(&path).ok_or(Error::NotFound)?;
        if image.len() as u64 > MAX_FILE {
            return Err(Error::Corrupted);
        }
        let hash = *blake3::hash(image).as_bytes();
        self.names.insert(String::from(name), hash);
        // Stejny obsah uz prisel z disku: od ted ho drzi initfs, nezahazovat.
        if let Some(u) = self.disk.remove(&hash) {
            self.disk_bytes -= u.bytes;
        }
        if let Some(t) = self.templates.get(&hash) {
            self.hits += 1;
            return Ok(t.clone());
        }
        let t = Arc::new(build(image, self.page).inspect_err(|e| println!("libd: {name}: {e}"))?);
        self.templates.insert(hash, t.clone());
        Ok(t)
    }

    /// Sablona knihovny z disku: obsah od klienta (prvnich `len` bajtu
    /// `vmo`). Kopie k sobe hned - klient muze VMO dal menit, libd pracuje
    /// jen s kopii, nad kterou spocitala hash.
    fn open_image(&mut self, name: &str, vmo: &Vmo, len: u64) -> Result<Arc<Template>> {
        self.opens += 1;
        if len > MAX_FILE {
            return Err(Error::Corrupted);
        }
        let mut image = Vec::new();
        image.try_reserve_exact(len as usize).map_err(|_| Error::NoMemory)?;
        image.resize(len as usize, 0);
        vmo.read(0, &mut image)?;
        let hash = *blake3::hash(&image).as_bytes();
        self.clock += 1;
        if let Some(t) = self.templates.get(&hash) {
            self.hits += 1;
            if let Some(u) = self.disk.get_mut(&hash) {
                u.last = self.clock;
            }
            return Ok(t.clone());
        }
        let t = Arc::new(build(&image, self.page).inspect_err(|e| println!("libd: {name} (z disku): {e}"))?);
        drop(image);
        let bytes = t.bytes();
        self.templates.insert(hash, t.clone());
        self.disk.insert(hash, DiskUse { last: self.clock, bytes });
        self.disk_bytes += bytes;
        self.evict(&hash);
        Ok(t)
    }

    /// Nad mezi sablon z disku zahodi nejdele nepouzite (krome `keep`,
    /// prave vracene). Procesy, ktere je mapuji, maji vlastni handly.
    fn evict(&mut self, keep: &[u8; 32]) {
        while self.disk.len() > MAX_DISK_TEMPLATES || self.disk_bytes > MAX_DISK_BYTES {
            let Some(old) = self.disk.iter().filter(|(h, _)| *h != keep).min_by_key(|(_, u)| u.last).map(|(h, _)| *h) else {
                return;
            };
            if let Some(u) = self.disk.remove(&old) {
                self.disk_bytes -= u.bytes;
            }
            self.templates.remove(&old);
            self.evicted += 1;
        }
    }
}

/// Sablona z obrazu knihovny: overeni ELF (ET_DYN, dynamicka cast bez
/// TEXTREL), segmenty do VMO, kod zapecetit.
fn build(image: &[u8], page: u64) -> Result<Template> {
    let elf = elf::ElfFile::parse_object(image, elf::Machine::NATIVE).map_err(|_| Error::Corrupted)?;
    if elf.elf_type != elf::ElfType::Dyn {
        return Err(Error::Corrupted);
    }
    elf.dynamic().map_err(|_| Error::Corrupted)?.ok_or(Error::Corrupted)?;
    let mut segments = Vec::new();
    let mut vmos = Vec::new();
    let mut span = 0;
    for seg in elf.load_segments() {
        let seg = seg.map_err(|_| Error::Corrupted)?;
        if segments.len() == MAX_SEGMENTS {
            return Err(Error::Corrupted);
        }
        let start = seg.vaddr & !(page - 1);
        let end = seg.vaddr.checked_add(seg.mem_size).ok_or(Error::Corrupted)?.next_multiple_of(page);
        if end - start > 1 << 30 {
            return Err(Error::Corrupted);
        }
        let vmo = Vmo::create(end - start)?;
        vmo.write(seg.vaddr - start, elf.segment_data(&seg))?;
        let mut prot = MAP_READ;
        if seg.flags & elf::PF_W != 0 {
            prot |= MAP_WRITE;
        }
        if seg.flags & elf::PF_X != 0 {
            prot |= MAP_EXECUTE;
        }
        let k = if prot & MAP_WRITE == 0 {
            vmo.seal()?;
            kind::SHARED
        } else {
            kind::DATA
        };
        segments.push(Segment { start, len: end - start, prot, kind: k });
        vmos.push(vmo);
        span = span.max(end);
    }
    if segments.is_empty() || segments[0].start != 0 {
        // Hlavicky ELF musi lezet v prvnim segmentu (zavadec je cte z pameti).
        return Err(Error::Corrupted);
    }
    Ok(Template { lib: Library { span: span.next_multiple_of(abi::vm::MAX_PAGE_SIZE), segments }, vmos })
}

/// Handly segmentu pro klienta: kod jen cteni, mapovani a spusteni (bez
/// zapisu), data jen cteni a mapovani (klient si udela klon).
fn share(t: &Template) -> Result<Vec<Handle>> {
    t.lib
        .segments
        .iter()
        .zip(&t.vmos)
        .map(|(s, v)| {
            let mut r = Rights::READ.0 | Rights::MAP.0 | Rights::BASIC.0;
            if s.kind == kind::SHARED {
                r |= Rights::EXECUTE.0;
            }
            v.handle().duplicate(Some(Rights(r)))
        })
        .collect()
}
