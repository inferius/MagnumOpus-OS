//! Stahovani do souboru (`save <adresa> [jmeno]`, docs/cs/stahovani.md):
//! telo jde po castech (`httpc::Sink`) do docasneho souboru ve slozce
//! Stazene, na konci se prejmenuje a dostane puvod (`sys.origin`:
//! adresa a cas). Prubeh je uloha v prehledu uloh (zrusit jde ze Spravce
//! uloh), konec oznameni. Chyby jsou srozumitelne (TLS, 404, malo mista).
//!
//! Relace FS vidi jen `/Stazene` (s priznakem `ORIGIN`, jinak by puvod
//! nesla zapsat); nic jineho z domova fetch nevidi.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use fsproto::FsClient;
use httpc::{Client, Header, Method, Request, Sink, Url};
use libos::{Channel, Error, time};
use notifyproto::{Priority, Send};
use taskproto::{Control, Origin, Produce, State};
use web::Web;

/// Slozka stazenych souboru v relaci FS fetch.
pub const DIR: &str = "/Stazene";
/// Zapis na disk po castech teto velikosti.
const WRITE: usize = 256 * 1024;
/// Prubeh do prehledu uloh nejvys tak casto.
const REPORT: u64 = 500_000_000;
/// Rezerva volneho mista, ktera po stazeni musi zustat.
const RESERVE: u64 = 16 * 1024 * 1024;

pub struct Downloads {
    pub fs: FsClient,
    pub tasks: Option<Channel>,
    pub notify: Option<Channel>,
    next: u32,
}

impl Downloads {
    pub fn new(fs: FsClient, tasks: Option<Channel>, notify: Option<Channel>) -> Self {
        Self { fs, tasks, notify, next: 0 }
    }

    fn produce(&mut self, m: &Produce) {
        if let Some(c) = &self.tasks
            && c.write(&m.encode(), Vec::new()).is_err()
        {
            self.tasks = None;
        }
    }

    /// Uloha v prehledu uloh (cislo od taskd, nebo `None` bez taskd).
    fn register(&mut self, name: &str) -> Option<u64> {
        self.next = self.next.wrapping_add(1).max(1);
        let req = self.next;
        self.produce(&Produce::Register {
            req,
            parent: 0,
            name: format!("Stahování {name}"),
            origin: Origin::App,
            who: String::from("Stahování z webu"),
            agent: false,
            job: None,
            attach: None,
            actions: taskproto::actions::CANCEL,
        });
        let c = self.tasks.as_ref()?;
        let until = time::deadline_after(core::time::Duration::from_secs(2));
        loop {
            let m = c.read(Some(until)).ok()?;
            if let Ok((r, id)) = taskproto::decode_registered(&m.data)
                && r == req
            {
                return Some(id);
            }
        }
    }

    fn notify(&mut self, title: &str, body: &str, priority: Priority) {
        let key = self.next as u64;
        if let Some(c) = &self.notify {
            let m = Send::Notify { key, title: title.into(), body: body.into(), priority, actions: Vec::new() };
            if c.write(&m.encode(), Vec::new()).is_err() {
                self.notify = None;
            }
        }
    }
}

/// Zapis tela do docasneho souboru.
struct FileSink<'a> {
    d: &'a mut Downloads,
    tmp: String,
    status: u16,
    total: Option<u64>,
    written: u64,
    buf: Vec<u8>,
    task: Option<u64>,
    last: u64,
}

impl FileSink<'_> {
    fn flush(&mut self) -> httpc::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        self.d.fs.write(&self.tmp, self.written, &self.buf).map_err(httpc::Error::Io)?;
        self.written += self.buf.len() as u64;
        self.buf.clear();
        Ok(())
    }

    /// Prubeh do prehledu uloh a pozadavek na zruseni.
    fn report(&mut self) -> httpc::Result<()> {
        let now = time::now();
        if now < self.last + REPORT {
            return Ok(());
        }
        self.last = now;
        let Some(id) = self.task else { return Ok(()) };
        let done = self.written + self.buf.len() as u64;
        let progress = self.total.filter(|&t| t > 0).map(|t| (done * 100 / t).min(100) as u8);
        let state = State::Running;
        self.d.produce(&Produce::Update { id, state, progress, eta_ms: None });
        // Zruseni ze Spravce uloh.
        if let Some(c) = &self.d.tasks {
            while let Ok(m) = c.try_read() {
                if let Ok(Control::Cancel(x)) = Control::decode(&m.data)
                    && x == id
                {
                    return Err(httpc::Error::Io(Error::Canceled));
                }
            }
        }
        Ok(())
    }
}

impl Sink for FileSink<'_> {
    fn head(&mut self, status: u16, _h: &[Header], len: Option<u64>) -> httpc::Result<()> {
        self.status = status;
        if !(200..300).contains(&status) {
            return Err(httpc::Error::Request("stav odpovedi"));
        }
        self.total = len;
        // Malo mista: hned, ne az v pulce stahovani.
        if let Some(n) = len
            && let Ok(i) = self.d.fs.info()
            && n.saturating_add(RESERVE) > i.free_blocks.saturating_mul(i.block_size as u64)
        {
            return Err(httpc::Error::Io(Error::NoSpace));
        }
        Ok(())
    }

    fn data(&mut self, chunk: &[u8]) -> httpc::Result<()> {
        self.buf.extend_from_slice(chunk);
        if self.buf.len() >= WRITE {
            self.flush()?;
        }
        self.report()
    }
}

/// Jmeno souboru z adresy (posledni cast cesty), bez zakazanych znaku.
pub fn name_from(url: &Url, given: Option<&str>) -> String {
    let raw = given.map(String::from).unwrap_or_else(|| {
        let p = url.path.split('?').next().unwrap_or("");
        String::from(p.rsplit('/').next().unwrap_or(""))
    });
    let decoded = percent_decode(&raw);
    let clean: String = decoded.chars().filter(|c| !c.is_control() && !"/\\:*?\"<>|".contains(*c)).collect();
    let clean = String::from(clean.trim().trim_start_matches('.'));
    if clean.is_empty() { String::from("stazeny-soubor") } else { clean }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(core::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Volne jmeno ve slozce (`soubor (2).zip`).
fn unique(fs: &FsClient, name: &str) -> String {
    if fs.stat(&format!("{DIR}/{name}")).is_err() {
        return String::from(name);
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2..1000).map(|n| format!("{stem} ({n}){ext}")).find(|c| fs.stat(&format!("{DIR}/{c}")).is_err()).unwrap_or_else(|| format!("{stem} ({}){ext}", time::now()))
}

/// Srozumitelny popis chyby pro uzivatele.
pub fn explain(e: &httpc::Error, status: u16) -> String {
    use httpc::Error as H;
    match e {
        H::Request(_) if status == 404 => String::from("Soubor na serveru neexistuje (404)."),
        H::Request(_) if status == 403 || status == 401 => format!("Server stažení nedovolil ({status})."),
        H::Request(_) if status >= 500 => format!("Chyba serveru ({status}), zkuste to později."),
        H::Request(_) if status != 0 => format!("Server odpověděl stavem {status}."),
        H::Transport(Error::AccessDenied, m) | H::Transport(Error::Corrupted, m) => format!("Zabezpečené spojení nejde ověřit: {m}"),
        H::Transport(_, m) => format!("Spojení se nepodařilo: {m}"),
        H::Io(Error::NoSpace) => String::from("Na disku není dost místa."),
        H::Io(Error::Canceled) => String::from("Stahování zrušeno."),
        H::Io(Error::NotFound) => String::from("Adresa serveru neexistuje (DNS)."),
        H::Io(Error::TimedOut) => String::from("Server neodpovídá."),
        H::Io(Error::ConnectionRefused | Error::Unreachable | Error::ConnectionReset) => String::from("Server je nedostupný."),
        H::Url(_) => String::from("Neplatná adresa."),
        H::InsecureRedirect => String::from("Server přesměroval ze zabezpečené adresy na nezabezpečenou."),
        H::TooManyRedirects => String::from("Příliš mnoho přesměrování."),
        e => format!("{e}"),
    }
}

/// Stahne `url` do slozky Stazene; vraci jmeno souboru a pocet bajtu,
/// nebo (kod, popis).
pub fn save(client: &mut Client<Web<'_>>, d: &mut Downloads, url: &str, given: Option<&str>) -> Result<(String, u64), (Error, String)> {
    let u = Url::parse(url).map_err(|e| (e.code(), explain(&e, 0)))?;
    let name = unique(&d.fs, &name_from(&u, given));
    let tmp = format!("{DIR}/.{name}.stahuje");
    let task = d.register(&name);
    let r = (|| {
        d.fs.create(&tmp, fsproto::kind::FILE).map_err(|e| (e, format!("Soubor nejde vytvořit: {e}")))?;
        let mut sink = FileSink { d: &mut *d, tmp: tmp.clone(), status: 0, total: None, written: 0, buf: Vec::new(), task, last: 0 };
        let res = client.request_to(Request::new(Method::Get, u.clone()), &mut sink).and_then(|_| sink.flush());
        let (status, written) = (sink.status, sink.written);
        res.map_err(|e| (e.code(), explain(&e, status)))?;
        let dst = format!("{DIR}/{name}");
        d.fs.rename(&tmp, &dst).map_err(|e| (e, format!("Soubor nejde uložit: {e}")))?;
        // Puvod: adresa a cas (sys.origin smi zapsat jen fetch).
        let when = libos::time::wall().map_or(0, |t| (t.secs.max(0) as u64).saturating_mul(1_000_000_000) + t.nanos as u64);
        if let Err(e) = d.fs.set_xattr(&dst, fsproto::perm::ORIGIN_XATTR, format!("{url}\n{when}").as_bytes()) {
            libos::println!("fetch: puvod {name}: {e}");
        }
        Ok(written)
    })();
    match r {
        Ok(n) => {
            if let Some(id) = task {
                d.produce(&Produce::Finish { id, state: State::Done(format!("Staženo {}", size(n))) });
            }
            d.notify(&format!("Staženo: {name}"), &format!("Ve složce Stažené, {}", size(n)), Priority::Normal);
            Ok((name, n))
        }
        Err((code, msg)) => {
            let _ = d.fs.remove(&tmp);
            if let Some(id) = task {
                let state = if code == Error::Canceled { State::Cancelled } else { State::Failed { code: code.code(), message: msg.clone() } };
                d.produce(&Produce::Finish { id, state });
            }
            if code != Error::Canceled {
                d.notify("Stahování selhalo", &format!("{name}: {msg}"), Priority::High);
            }
            Err((code, msg))
        }
    }
}

/// Velikost pro lidi (B, KB, MB, GB po 1000).
pub fn size(n: u64) -> String {
    match n {
        0..1_000 => format!("{n} B"),
        1_000..1_000_000 => format!("{} kB", n / 1_000),
        1_000_000..1_000_000_000 => format!("{},{} MB", n / 1_000_000, n / 100_000 % 10),
        _ => format!("{},{} GB", n / 1_000_000_000, n / 100_000_000 % 10),
    }
}
