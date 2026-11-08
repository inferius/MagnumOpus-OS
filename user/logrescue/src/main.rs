//! logrescue: rezim "vytahni logy" zive flashky.
//!
//! Spousti ho init (priznak startu `RESCUE_LOGS`, klavesa L v zavadeci).
//! Startovni zprava (kind USER): 1. kanal k oddilu MO-LOGY flashky,
//! 2..N. kanaly k diskum s nasim FS mimo flashku (nainstalovany system).
//!
//! Kazdy disk pripoji **jen pro cteni** (dvakrat: `MountOptions::read_only`
//! a blokove zarizeni, ktere zapis odmitne), zkopiruje `/system/log`
//! a `/system/pady` do slozky `ZACHRANA` na FAT32 oddilu flashky (cte ho
//! Windows) a zapise `SEZNAM.TXT` (puvodni jmena). Funguje i tehdy, kdyz
//! nainstalovany system nenastartuje - staci, ze jde precist jeho FS.
//!
//! Odpoved initu na bootstrap: `ok <pocet souboru>` nebo `chyba <text>`.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use blockproto::{BlockClient, SECTOR};
use fs::{BLOCK_SIZE, BlockDevice, Durability, Error, Fs, Kind, MountOptions};
use libos::{Channel, Startup, println};

libos::entry!(main);

const DIR: &str = "ZACHRANA";
/// Nejvic bajtu jednoho souboru (konec) na FAT.
const FILE_MAX: u64 = 8 << 20;
/// Nejvic souboru celkem.
const MAX_FILES: usize = 400;

/// Disk jen pro cteni (zapis = `READ_ONLY`, nikdy nedojde k ovladaci).
struct ReadOnlyDisk {
    client: BlockClient,
    blocks: u64,
}

impl BlockDevice for ReadOnlyDisk {
    fn block_count(&self) -> u64 {
        self.blocks
    }
    fn read_block(&mut self, block: u64, buf: &mut [u8]) -> fs::Result<()> {
        self.read_blocks(block, buf)
    }
    fn write_block(&mut self, _block: u64, _data: &[u8]) -> fs::Result<()> {
        Err(Error::ReadOnly)
    }
    fn read_blocks(&mut self, start: u64, buf: &mut [u8]) -> fs::Result<()> {
        self.client.read(start * (BLOCK_SIZE / SECTOR) as u64, buf)
    }
    fn write_blocks(&mut self, _start: u64, _data: &[u8]) -> fs::Result<()> {
        Err(Error::ReadOnly)
    }
    fn flush(&mut self) -> fs::Result<()> {
        Ok(())
    }
    fn durability(&self) -> Durability {
        Durability::None
    }
}

struct Fat(BlockClient);

impl fatw::Device for Fat {
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> fatw::Result<()> {
        self.0.read(lba, buf).map_err(|_| fatw::Error::Io)
    }
    fn write(&mut self, lba: u64, data: &[u8]) -> fatw::Result<()> {
        self.0.write(lba, data).map_err(|_| fatw::Error::Io)
    }
    fn flush(&mut self) -> fatw::Result<()> {
        self.0.flush().map_err(|_| fatw::Error::Io)
    }
}

fn main(startup: Startup) -> i64 {
    let mut handles = startup.handles.into_iter();
    let reply = |t: String| {
        println!("logrescue: {t}");
        let _ = startup.bootstrap.write(t.as_bytes(), Vec::new());
    };
    let Some(fat) = handles.next() else {
        reply(String::from("chyba chybi oddil MO-LOGY"));
        return 1;
    };
    let mut vol = match BlockClient::new(Channel::from_handle(fat)).map_err(|_| fatw::Error::Io).and_then(|c| fatw::Volume::open(Fat(c))) {
        Ok(v) => v,
        Err(e) => {
            reply(format!("chyba oddil MO-LOGY nejde otevrit: {e:?}"));
            return 2;
        }
    };
    let mut list = String::from("MagnumOpus - logy vytazene z nainstalovaneho systemu\r\n\r\n");
    let mut n = 0usize;
    let mut disks = 0;
    for (d, h) in handles.enumerate() {
        let Ok(client) = BlockClient::new(Channel::from_handle(h)) else { continue };
        let blocks = client.sectors() / (BLOCK_SIZE / SECTOR) as u64;
        let opts = MountOptions { read_only: true, ..MountOptions::default() };
        let mut fs = match Fs::mount_with(ReadOnlyDisk { client, blocks }, opts) {
            Ok(f) => f,
            Err(e) => {
                list.push_str(&format!("disk {d}: FS nejde pripojit ({e})\r\n"));
                continue;
            }
        };
        disks += 1;
        let info = fs.info();
        println!("logrescue: disk {d}: FS '{}' (generace {}), jen pro cteni", info.label, info.generation);
        list.push_str(&format!("disk {d}: FS '{}', generace {}\r\n", info.label, info.generation));
        for dir in ["/system/log", "/system/pady"] {
            let Ok(entries) = fs.list(dir) else {
                list.push_str(&format!("  {dir}: neni\r\n"));
                continue;
            };
            for e in entries.into_iter().filter(|e| e.kind == Kind::File) {
                if n >= MAX_FILES {
                    break;
                }
                let path = format!("{dir}/{}", e.name);
                let Ok(st) = fs.stat(&path) else { continue };
                let from = st.size.saturating_sub(FILE_MAX);
                let mut data = Vec::new();
                let mut at = from;
                while at < st.size {
                    match fs.read(&path, at, ((st.size - at) as usize).min(64 << 10)) {
                        Ok(c) if !c.is_empty() => {
                            at += c.len() as u64;
                            data.extend_from_slice(&c);
                        }
                        _ => break,
                    }
                }
                let short = format!("F{:03}{:04}.TXT", d % 1000, n % 10000);
                match vol.write_file(DIR, &short, &data, fatw::Stamp::default()) {
                    Ok(()) => {
                        n += 1;
                        list.push_str(&format!("  {short}  {path}  ({} B{})\r\n", data.len(), if from > 0 { ", konec" } else { "" }));
                    }
                    Err(e) => {
                        list.push_str(&format!("  {path}: zapis na flashku selhal ({e:?})\r\n"));
                        if e == fatw::Error::Full {
                            break;
                        }
                    }
                }
            }
        }
    }
    if disks == 0 {
        list.push_str("nenalezen zadny disk s nainstalovanym systemem\r\n");
    }
    let _ = vol.write_file(DIR, "SEZNAM.TXT", list.as_bytes(), fatw::Stamp::default());
    if disks == 0 {
        reply(String::from("chyba nenalezen nainstalovany system (disk s nasim FS mimo flashku)"));
        return 3;
    }
    reply(format!("ok {n}"));
    0
}
