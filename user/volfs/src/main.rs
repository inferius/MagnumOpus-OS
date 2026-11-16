//! volfs: proces jednoho vymenneho media (docs/cs/media.md).
//!
//! Spousti ho init pro kazdy oddil media (`block-media` od ovladace).
//! Argumenty: `volfs <fs> <ro|rw>` (`fat`, `ntfs`). Netagovane handly
//! startovni zpravy v tomto poradi:
//!
//! 1. kanal oddilu media (`blockproto`; ochranu rozsahu a zapisu dela
//!    ovladac, `diskguard::media`),
//! 2. kanal k fsd (`volproto`): pozadavky s cislem, odpovedi v poradi.
//!
//! Zadna jina capability (sit, FS, okna). Konec: `UNMOUNT`, zavreny kanal
//! fsd, nebo zmizele medium (chyba cteni = chyba operace; ovladac skonci
//! a kanal se zavre).
//!
//! Kod 0 = odpojeno, 2 = chybi handly nebo argumenty, 3 = svazek nejde
//! pripojit (poskozeny nebo neznamy FS).

#![cfg_attr(target_os = "magnumopus", feature(rustc_private))]

#[cfg(target_os = "magnumopus")]
fn main() {
    std::process::exit(os::run());
}

#[cfg(not(target_os = "magnumopus"))]
fn main() {
    eprintln!("volfs bezi jen v MagnumOpus (testy: cargo test -p volfs)");
}

#[cfg(target_os = "magnumopus")]
mod os {
    use std::io;
    use std::os::magnumopus::runtime as rt;

    use blockproto::BlockClient;
    use libos::{Channel, Handle};
    use volfs::Volume;
    use volfs::dev::Sectors;

    /// Kanal oddilu jako zarizeni po sektorech (512 B, jednotka protokolu).
    struct Block {
        c: BlockClient,
        sectors: u64,
    }

    fn io(e: libos::Error) -> io::Error {
        match e {
            libos::Error::ReadOnly => io::Error::new(io::ErrorKind::PermissionDenied, "medium jen pro cteni"),
            e => io::Error::other(e.name()),
        }
    }

    impl Sectors for Block {
        fn sector_size(&self) -> usize {
            blockproto::SECTOR
        }
        fn sectors(&self) -> u64 {
            self.sectors
        }
        fn read(&mut self, lba: u64, buf: &mut [u8]) -> io::Result<()> {
            self.c.read(lba, buf).map_err(io)
        }
        fn write(&mut self, lba: u64, data: &[u8]) -> io::Result<()> {
            self.c.write(lba, data).map_err(io)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.c.flush().map_err(io)
        }
    }

    pub fn run() -> i32 {
        libos::rt::set_self_process(rt::start::self_process());
        let args: Vec<String> = std::env::args().collect();
        let mut hs = rt::start::take_handles().into_iter().map(Handle::from_raw);
        let (Some(block), Some(fsd), Some(fs)) = (hs.next(), hs.next(), args.get(1)) else {
            eprintln!("volfs: chybi handly nebo argumenty");
            return 2;
        };
        let read_only = args.get(2).is_none_or(|a| a != "rw");
        let fsd = Channel::from_handle(fsd);
        let c = match BlockClient::new(Channel::from_handle(block)) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("volfs: oddil neodpovida: {e}");
                return 3;
            }
        };
        let dev = Block { sectors: c.sectors(), c };
        let mut vol: Box<dyn Volume> = match fs.as_str() {
            "fat" => match volfs::fat::FatVolume::new(dev, read_only) {
                Ok(v) => Box::new(v),
                Err(e) => {
                    eprintln!("volfs: FAT nejde pripojit: {e}");
                    return 3;
                }
            },
            "ntfs" => match volfs::ntfs::NtfsVolume::new(dev) {
                Ok(v) => Box::new(v),
                Err(e) => {
                    eprintln!("volfs: NTFS nejde pripojit: {e}");
                    return 3;
                }
            },
            other => {
                eprintln!("volfs: nepodporovany souborovy system {other}");
                return 3;
            }
        };
        // Pripraveno: fsd ceka na prvni odpoved (INFO) az po pripojeni.
        loop {
            let msg = match fsd.read(None) {
                Ok(m) => m,
                Err(_) => {
                    // fsd skoncil nebo medium odpojil: data na medium.
                    let _ = vol.unmount();
                    return 0;
                }
            };
            let out = volfs::serve::handle(vol.as_mut(), &msg.data);
            if fsd.write(&out.reply, Vec::new()).is_err() {
                let _ = vol.unmount();
                return 0;
            }
            if out.stop {
                return 0;
            }
        }
    }
}
