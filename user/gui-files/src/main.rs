//! gui-files: spravce souboru v GUI (milnik 6c).
//!
//! Model a rozhrani jsou v knihovne `filesview` (testuje se na
//! hostiteli). Tady je jen backend nad relaci FS uzivatele a kanalem
//! k pomocnemu procesu `files-copy`.
//!
//! Startovni zprava (kind USER): 1. kanal okna ke kompozitoru, 2. listek
//! k supervisoru, 3. relace FS uzivatele (`CONNECT_AS` s celym stromem,
//! prava rozhoduji), 4. kanal k `files-copy` (bez procesu zavreny: jde
//! vse krome kopirovani a presunu mezi slozkami), 5. volitelne kanal
//! sluzby "otevri cestu" (init, `libs/openwith`). Pracovni adresar
//! v bloku parametru = domov uzivatele.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use fileops::proto::ClientMsg;
use filesview::{Backend, FileManager, Result};
use fsproto::FsClient;
use fsproto::datasets::DatasetEntry;
use fsproto::hist::{FileVersion, TrashEntry};
use fsproto::{Stat, StatEntry, kind};
use guiproto::window::Mode;
use libos::time::WallTime;
use libos::time::zone::Zone;
use libos::{Channel, Startup, println};

libos::entry!(main);

struct OsBackend {
    fs: FsClient,
    home: String,
    copier: Option<Channel>,
    /// Kanal sluzby "otevri cestu" (init).
    opener: Option<Channel>,
    zone: Option<Zone>,
}

impl Backend for OsBackend {
    fn home(&self) -> String {
        self.home.clone()
    }

    fn list(&mut self, dir: &str) -> Result<Vec<StatEntry>> {
        let t = libos::time::now();
        let r = self.fs.list_stat(dir);
        let ms = (libos::time::now() - t) / 1_000_000;
        if ms >= 200 {
            println!("gui-files: vypis {dir}: {} polozek za {ms} ms", r.as_ref().map_or(0, |v| v.len()));
        }
        r
    }

    fn stat(&mut self, path: &str) -> Result<Stat> {
        self.fs.stat(path)
    }

    fn datasets(&mut self) -> Result<Vec<DatasetEntry>> {
        self.fs.datasets()
    }

    fn dataset_of(&mut self, path: &str) -> Result<String> {
        self.fs.dataset_of(path)
    }

    fn mkdir(&mut self, path: &str) -> Result<()> {
        self.fs.create(path, kind::DIR)
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        self.fs.rename(from, to)
    }

    fn trash(&mut self, path: &str) -> Result<u64> {
        self.fs.trash(path)
    }

    fn read(&mut self, path: &str, offset: u64, len: usize) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        while out.len() < len {
            let part = self.fs.read(path, offset + out.len() as u64, (len - out.len()).min(fsproto::MAX_IO))?;
            if part.is_empty() {
                break;
            }
            out.extend_from_slice(&part);
        }
        Ok(out)
    }

    fn trash_list(&mut self, dataset: &str) -> Result<Vec<TrashEntry>> {
        self.fs.trash_list(dataset)
    }

    fn trash_restore(&mut self, dataset: &str, id: u64) -> Result<String> {
        self.fs.trash_restore(dataset, id, None)
    }

    fn trash_purge(&mut self, dataset: &str, id: u64) -> Result<()> {
        self.fs.trash_purge(dataset, id)
    }

    fn versions(&mut self, path: &str) -> Result<Vec<FileVersion>> {
        self.fs.file_versions(path)
    }

    fn restore_version(&mut self, path: &str, snapshot: &str, dest: Option<&str>) -> Result<String> {
        self.fs.restore_version(path, snapshot, dest)
    }

    fn copier(&mut self, m: ClientMsg) {
        // Trasovani vstupu: pozadavek vyvolany klikem.
        let id = ui::client::input_id();
        if id != 0 {
            println!("vstup: id={id} gui-files -> files-copy {m:?}");
        }
        if let Some(c) = &self.copier
            && c.write(&m.encode(), Vec::new()).is_err()
        {
            self.copier = None;
        }
    }

    fn open(&mut self, path: &str) -> bool {
        let Some(o) = &self.opener else { return false };
        // Odpoved se necte (sluzba muze volat akci tohoto okna); kanal
        // pro ni se hned zahodi.
        let Ok((_mine, theirs)) = Channel::create() else { return false };
        o.write(&openwith::msg::request(path), alloc::vec![theirs.into_handle()]).is_ok()
    }

    fn now(&self) -> Option<WallTime> {
        libos::time::wall()
    }

    fn zone(&self) -> Option<&Zone> {
        self.zone.as_ref()
    }
}

/// Aplikace s merenim: pomale zpracovani (nad 50 ms) jde do logu.
struct Timed(FileManager<OsBackend>);

fn timed<R>(what: &str, f: impl FnOnce() -> R) -> R {
    let t = libos::time::now();
    let r = f();
    let ms = (libos::time::now() - t) / 1_000_000;
    if ms >= 50 {
        println!("gui-files: pomale {what}: {ms} ms");
    }
    r
}

impl ui::App for Timed {
    type Msg = filesview::Msg;

    fn title(&self) -> String {
        self.0.title()
    }

    fn view(&self, cx: &ui::Context) -> ui::Node<Self::Msg> {
        timed("view", || self.0.view(cx))
    }

    fn update(&mut self, m: Self::Msg) {
        let what = alloc::format!("update {m:?}");
        timed(&what, || self.0.update(m))
    }

    fn action(&mut self, id: &str, args: &[guiproto::action::Arg]) -> core::result::Result<Option<guiproto::action::Value>, String> {
        timed(id, || self.0.action(id, args))
    }

    fn menu(&self) -> Vec<guiproto::menu::MenuItem> {
        self.0.menu()
    }

    fn external(&mut self, data: &[u8]) -> bool {
        timed("zprava files-copy", || self.0.external(data))
    }
}

/// Pasmo z nastaveni systemu (zapsal instalator); bez nej UTC.
fn zone(fs: &FsClient) -> Option<Zone> {
    let path = installplan::config::PATH;
    let st = fs.stat(path).ok()?;
    let data = fs.read(path, 0, st.size.min(16 << 10) as usize).ok()?;
    let s = installplan::config::Settings::parse(&String::from_utf8_lossy(&data));
    Zone::by_name(&s.zone).or_else(|| Zone::parse(&s.zone))
}

fn main(startup: Startup) -> i64 {
    let home = startup.params.as_ref().and_then(|v| libos::params::Params::read(v).ok()).map(|p| String::from_utf8_lossy(&p.cwd).into_owned()).unwrap_or_default();
    let mut handles = startup.handles.into_iter();
    let Some(link) = ui::client::Link::from_handles(&mut handles) else {
        println!("gui-files: chybi kanal okna");
        return 1;
    };
    let Some(fs) = handles.next() else {
        println!("gui-files: chybi relace FS");
        return 1;
    };
    let fs = FsClient::new(Channel::from_handle(fs));
    let copier = handles.next().map(Channel::from_handle);
    let opener = handles.next().map(Channel::from_handle);
    // Zapis pres druhy handle stejneho kanalu (cteni zustane run_with).
    let writer = match copier.as_ref().map(|c| c.handle().duplicate(None)) {
        Some(Ok(d)) => Some(Channel::from_handle(d)),
        Some(Err(e)) => {
            println!("gui-files: kanal files-copy: {e}");
            None
        }
        None => {
            println!("gui-files: bez files-copy (kopirovani nepujde)");
            None
        }
    };
    let zone = zone(&fs);
    let home = if home.is_empty() { String::from("/") } else { home };
    let app = Timed(FileManager::new(OsBackend { fs, home, copier: writer, opener, zone }));
    match ui::client::run_with(link, app, Mode::Standard, 1120, 660, copier) {
        Ok(()) => 0,
        Err(e) => {
            println!("gui-files: {e}");
            2
        }
    }
}
