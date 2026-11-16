//! Spojeni se serverem SSH: TCP, `ssh::client::ClientConn` a rozhodnuti
//! klienta (`Back`): klic serveru podle `~/.ssh/known_hosts`, prihlaseni
//! klicem pres sluzbu uctu (kanal podpisu, soukromy klic program nema),
//! heslo k serveru do skryte vyzvy (ECHO vypnute pres `ttyproto`).

use std::collections::BTreeMap;
use std::io::{BufRead, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::os::magnumopus::runtime as rt;
use std::time::Duration;

use accountproto::agent::{AgentReq, decode_identities};
use ssh::client::{CState, ClientBackend, ClientConn, Failure, Request};
use ssh::keys::PublicKey;

/// Cil `[uzivatel@]server`, port.
#[derive(Debug, Clone)]
pub struct Target {
    pub user: String,
    pub host: String,
    pub port: u16,
}

impl Target {
    /// `[uzivatel@]server`; bez uzivatele `$USER`.
    pub fn parse(s: &str, port: u16) -> Result<Self, String> {
        let (user, host) = match s.rsplit_once('@') {
            Some((u, h)) => (u.to_string(), h.to_string()),
            None => (std::env::var("USER").unwrap_or_default(), s.to_string()),
        };
        if host.is_empty() || user.is_empty() {
            return Err(format!("{s}: čekám [uživatel@]server"));
        }
        Ok(Target { user, host, port })
    }

    /// Jmeno serveru v known_hosts (`server` nebo `[server]:port`).
    pub fn known_name(&self) -> String {
        if self.port == 22 { self.host.to_lowercase() } else { format!("[{}]:{}", self.host.to_lowercase(), self.port) }
    }
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| String::from("/"))
}

/// Radek ze stdin (bez konce radku); `None` = konec vstupu.
pub fn read_line() -> Option<String> {
    let mut s = String::new();
    match std::io::stdin().lock().read_line(&mut s) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(s.trim_end_matches(['\r', '\n']).to_string()),
    }
}

/// ECHO terminalu (skryty vstup hesla). Bez terminalu nic.
fn set_echo(on: bool) {
    use ttyproto::{Body, Request};
    let get = Request::GetMode;
    let Ok((d, h)) = rt::tty::call(&get.encode()) else { return };
    rt::tty::close_handles(h);
    let Ok(Body::Mode(mut m)) = ttyproto::decode_reply(&get, &d) else { return };
    if on {
        m.lflag |= ttyproto::lflag::ECHO;
    } else {
        m.lflag &= !ttyproto::lflag::ECHO;
    }
    let set = Request::SetMode { when: ttyproto::when::NOW, mode: m };
    if let Ok((_, h)) = rt::tty::call(&set.encode()) {
        rt::tty::close_handles(h);
    }
}

/// Rozhodnuti klienta.
pub struct Back {
    target: Target,
    /// Identity ze sluzby uctu: blob -> cislo klice.
    ids: BTreeMap<Vec<u8>, u32>,
    pub quiet: bool,
}

impl Back {
    /// Pozadavek na kanal podpisu (chyby jako text: std a knihovny maji
    /// kazdy svou kopii `abi`).
    fn agent(req: &AgentReq) -> Result<Vec<u8>, String> {
        let b = rt::agent::call(&req.encode()).map_err(|e| format!("{e}"))?;
        accountproto::parse_reply(&b).map(<[u8]>::to_vec).map_err(|e| format!("{e}"))
    }

    fn known_hosts() -> std::path::PathBuf {
        std::path::PathBuf::from(format!("{}/.ssh/known_hosts", home()))
    }
}

impl ClientBackend for Back {
    fn hostkey(&mut self, key: &PublicKey) -> bool {
        let name = self.target.known_name();
        let want = key.line();
        let mut want_parts = want.split_whitespace();
        let (wt, wk) = (want_parts.next().unwrap_or(""), want_parts.next().unwrap_or(""));
        let text = std::fs::read_to_string(Self::known_hosts()).unwrap_or_default();
        let mut mismatch = false;
        for line in text.lines() {
            let mut f = line.split_whitespace();
            let (Some(hosts), Some(t), Some(k)) = (f.next(), f.next(), f.next()) else { continue };
            if hosts.starts_with('#') || !hosts.split(',').any(|h| h.eq_ignore_ascii_case(&name)) {
                continue;
            }
            if t == wt && k == wk {
                return true;
            }
            mismatch = true;
        }
        if mismatch {
            eprintln!("VAROVÁNÍ: klíč serveru {name} se ZMĚNIL!");
            eprintln!("Může jít o útok (někdo se vydává za server), nebo server dostal nový klíč.");
            eprintln!("Nový klíč: {}", key.fingerprint());
            eprintln!("Spojení odmítnuto. Pokud změnu znáte, odeberte řádek {name} z ~/.ssh/known_hosts.");
            return false;
        }
        eprintln!("Server {name} není známý. Otisk klíče: {}", key.fingerprint());
        eprintln!("Ověřte otisk u správce serveru. Připojit a zapamatovat? (yes/no)");
        let _ = std::io::stderr().flush();
        let Some(answer) = read_line() else { return false };
        if !matches!(answer.trim(), "yes" | "ano") {
            eprintln!("Spojení zrušeno.");
            return false;
        }
        let path = Self::known_hosts();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path);
        match f.as_mut().map(|f| writeln!(f, "{name} {wt} {wk}")) {
            Ok(Ok(())) => eprintln!("Klíč serveru {name} uložen do ~/.ssh/known_hosts."),
            _ => eprintln!("Varování: klíč serveru nejde uložit do ~/.ssh/known_hosts."),
        }
        true
    }

    fn identities(&mut self) -> Vec<PublicKey> {
        if !rt::agent::available() {
            return Vec::new();
        }
        let Ok(b) = Self::agent(&AgentReq::Identities) else { return Vec::new() };
        let Ok(list) = decode_identities(&b) else { return Vec::new() };
        let mut out = Vec::new();
        for i in list {
            if let Ok(pk) = PublicKey::from_blob(&i.blob) {
                self.ids.insert(i.blob, i.id);
                out.push(pk);
            }
        }
        out
    }

    fn sign(&mut self, key: &PublicKey, data: &[u8]) -> Option<[u8; 64]> {
        let id = *self.ids.get(&key.blob())?;
        let req = AgentReq::Sign { id, host: self.target.known_name(), data: data.to_vec() };
        match Self::agent(&req) {
            Ok(sig) => sig.try_into().ok(),
            Err(e) => {
                if !self.quiet {
                    eprintln!("Klíč {} nepodepsán ({e}).", key.fingerprint());
                }
                None
            }
        }
    }

    fn password(&mut self) -> Option<String> {
        eprint!("Heslo pro {}@{}: ", self.target.user, self.target.host);
        let _ = std::io::stderr().flush();
        set_echo(false);
        let pw = read_line();
        set_echo(true);
        eprintln!();
        pw
    }
}

/// Otevrene spojeni s bezicim kanalem.
pub struct Sess {
    tcp: TcpStream,
    pub conn: ClientConn,
    back: Back,
    out: Vec<u8>,
    tcp_closed: bool,
    /// Prectena data kanalu (stderr?, bajty): klient nepreda dalsi vstup,
    /// dokud se data kanalu nevyberou.
    inbox: std::collections::VecDeque<(bool, Vec<u8>)>,
}

impl Sess {
    /// Spojeni, overeni serveru, prihlaseni a otevreni kanalu `req`.
    pub fn open(t: &Target, req: Request) -> Result<Sess, String> {
        let addrs: Vec<SocketAddr> = (t.host.as_str(), t.port).to_socket_addrs().map_err(|e| format!("{}: jméno nejde přeložit ({e})", t.host))?.collect();
        let addr = *addrs.iter().find(|a| a.is_ipv4()).or(addrs.first()).ok_or_else(|| format!("{}: bez adresy", t.host))?;
        let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(15)).map_err(|e| format!("{addr}: spojení selhalo ({e})"))?;
        let _ = tcp.set_read_timeout(Some(Duration::from_millis(20)));
        let back = Back { target: t.clone(), ids: BTreeMap::new(), quiet: false };
        let mut s = Sess { tcp, conn: ClientConn::new(&t.user, req), back, out: vec![0u8; 40_000], tcp_closed: false, inbox: Default::default() };
        loop {
            s.pump()?;
            match s.conn.state() {
                CState::Running => return Ok(s),
                CState::Connecting => {}
                CState::Closed(_) => return Err(String::from("server spojení ukončil")),
                CState::Failed(Failure::Hostkey) => return Err(String::from("klíč serveru nebyl přijat")),
                CState::Failed(Failure::Auth) => return Err(format!("přihlášení {}@{} odmítnuto (klíč ani heslo neprošly)", t.user, t.host)),
                CState::Failed(Failure::Closed) => return Err(String::from("server spojení ukončil před přihlášením")),
            }
        }
    }

    /// Data kanalu z klienta do fronty (uvolni klienta pro dalsi vstup).
    fn drain(&mut self) -> Result<(), String> {
        let mut b = [0u8; 16 * 1024];
        // Pojistka: data, ktera `read` nevrati (jiny kanal), smycku neudrzi.
        let mut idle = 0;
        loop {
            self.conn.progress(&mut self.back).map_err(|e| format!("protokol SSH: {e:?}"))?;
            match self.conn.read(&mut b).map_err(|e| format!("protokol SSH: {e:?}"))? {
                Some((err, n)) => {
                    idle = 0;
                    self.inbox.push_back((err, b[..n].to_vec()));
                }
                None if self.conn.has_channel_data() && idle < 8 => idle += 1,
                None => return self.flush(),
            }
        }
    }

    /// Dalsi prectena data kanalu.
    pub fn take(&mut self) -> Option<(bool, Vec<u8>)> {
        self.inbox.pop_front()
    }

    /// Ceka neco ve fronte nebo v klientu?
    pub fn pending(&self) -> bool {
        !self.inbox.is_empty() || self.conn.has_channel_data()
    }

    /// Jedno kolo: data ze site do klienta, udalosti, data na sit.
    pub fn pump(&mut self) -> Result<(), String> {
        self.drain()?;
        if self.tcp_closed {
            return Ok(());
        }
        let mut b = [0u8; 16 * 1024];
        match (&self.tcp).read(&mut b) {
            Ok(0) => {
                self.tcp_closed = true;
                self.conn.close();
            }
            Ok(n) => {
                let mut off = 0;
                let mut stuck = 0;
                while off < n {
                    let k = self.conn.input(&b[off..n]).map_err(|e| format!("protokol SSH: {e:?}"))?;
                    off += k;
                    // Klient prijme dalsi bajty, az se vyberou data kanalu.
                    self.drain()?;
                    stuck = if k == 0 { stuck + 1 } else { 0 };
                    if stuck > 100 {
                        return Err(String::from("protokol SSH: klient neprijima data"));
                    }
                    if !matches!(self.conn.state(), CState::Connecting | CState::Running) {
                        break;
                    }
                }
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => return Err(format!("síť: {e}")),
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), String> {
        loop {
            let n = self.conn.output(&mut self.out);
            if n == 0 {
                return Ok(());
            }
            (&self.tcp).write_all(&self.out[..n]).map_err(|e| format!("síť: {e}"))?;
        }
    }

    /// Zapise vsechna data do kanalu (ceka na okno).
    pub fn write_all(&mut self, mut data: &[u8]) -> Result<(), String> {
        while !data.is_empty() {
            let n = self.conn.write(data).map_err(|e| format!("protokol SSH: {e:?}"))?;
            data = &data[n..];
            if n == 0 {
                if !matches!(self.conn.state(), CState::Running) {
                    return Err(String::from("kanál je zavřený"));
                }
                self.pump()?;
            }
        }
        self.flush()
    }
}
