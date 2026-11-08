//! Kopie logu a zaznamu padu na FAT32 oddil flashky (`MO-LOGY`), ktery
//! Windows precte bez dalsich nastroju. Jen slozka `MAGNUMOS` v korenu
//! oddilu (`libs/fatw`): soubory se vytvari a prepisuji v pevnych slotech,
//! nic se nemaze, cizi soubory zustanou netknute.
//!
//! | soubor | obsah |
//! |---|---|
//! | `LOGxx.TXT` | log bootu (slot = cislo bootu mod 16), nejvys posledni 2 MiB |
//! | `PADxx.TXT` | zaznam padu jadra nebo procesu (slot dokola 0-31) |
//! | `REPORT.TXT` | posledni balicek pro vyvojare |
//! | `CTI-MNE.TXT` | co ve slozce je |
//!
//! Prvni radek kazdeho souboru rika boot a puvod (soubory se v slotech
//! prepisuji dokola, Windows je seradi podle data zmeny).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use blockproto::BlockClient;
use libos::println;

const DIR: &str = "MAGNUMOS";
const LOG_SLOTS: u64 = 16;
const CRASH_SLOTS: u64 = 32;
/// Nejvic logu jednoho bootu na FAT (konec).
pub const LOG_TAIL: usize = 2 << 20;

struct Dev(BlockClient);

impl fatw::Device for Dev {
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

pub struct FatLog {
    vol: Option<fatw::Volume<Dev>>,
    crash_slot: u64,
}

const README: &str = "MagnumOpus - logy a zaznamy padu\r\n\
\r\n\
LOGxx.TXT   log jednoho startu systemu (prvni radek: cislo startu)\r\n\
PADxx.TXT   zaznam padu jadra nebo sluzby\r\n\
REPORT.TXT  posledni balicek pro vyvojare (prikaz report)\r\n\
\r\n\
Posilejte vyvojari REPORT.TXT, pripadne celou slozku.\r\n";

impl FatLog {
    /// Otevre oddil; `None` = neni (beh z interniho disku) nebo neni FAT32.
    pub fn open(client: BlockClient, boot: u64) -> Option<Self> {
        let vol = match fatw::Volume::open(Dev(client)) {
            Ok(v) => v,
            Err(e) => {
                println!("logd: oddil MO-LOGY neni FAT32 ({e:?}), kopie logu na nej nebudou");
                return None;
            }
        };
        let mut f = FatLog { vol: Some(vol), crash_slot: boot * 4 % CRASH_SLOTS };
        f.write("CTI-MNE.TXT", README.as_bytes());
        println!("logd: kopie logu na oddil MO-LOGY (slozka {DIR})");
        Some(f)
    }

    fn write(&mut self, name: &str, data: &[u8]) {
        let Some(v) = self.vol.as_mut() else { return };
        if let Err(e) = v.write_file(DIR, name, data, fatw::Stamp::default()) {
            println!("logd: zapis {name} na MO-LOGY selhal: {e:?}");
            if e == fatw::Error::Io || e == fatw::Error::Corrupted {
                // Flashka vytazena nebo poskozena: dal nezkouset.
                self.vol = None;
            }
        }
    }

    /// Log bootu `boot` (konec nejvys [`LOG_TAIL`]).
    pub fn boot_log(&mut self, boot: u64, log: &[u8]) {
        let tail = &log[log.len().saturating_sub(LOG_TAIL)..];
        let mut data: Vec<u8> = format!("MagnumOpus: log startu {boot}\r\n").into_bytes();
        data.extend_from_slice(tail);
        self.write(&format!("LOG{:02}.TXT", boot % LOG_SLOTS), &data);
    }

    /// Zaznam padu (jadra nebo procesu).
    pub fn crash(&mut self, what: &str, text: &str) {
        let slot = self.crash_slot;
        self.crash_slot = (self.crash_slot + 1) % CRASH_SLOTS;
        let data = format!("{what}\r\n{text}");
        self.write(&format!("PAD{slot:02}.TXT"), data.as_bytes());
    }

    pub fn report(&mut self, text: &[u8]) {
        self.write("REPORT.TXT", text);
    }

    pub fn active(&self) -> bool {
        self.vol.is_some()
    }
}

/// Text pro hlaseni (prazdny = bez kopie).
pub fn note(f: &Option<FatLog>) -> String {
    match f {
        Some(f) if f.active() => String::from("  kopie na flashce: MO-LOGY\\MAGNUMOS\\REPORT.TXT (cte i Windows)\n"),
        _ => String::new(),
    }
}
