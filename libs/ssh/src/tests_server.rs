//! Server SSH proti klientovi sunset v pameti: vymena klicu, prihlaseni
//! klicem (podpis overuje backend pres `keys`, jako sluzba uctu), exec
//! s kodem ukonceni, PTY a zmena velikosti, odmitnuty cizi klic.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use sunset::event::CliEvent;
use sunset::{ChanData, ChanHandle, Client, Event, Runner, SignKey};

use crate::keys::{Algo, PublicKey};
use crate::server::{Auth, Backend, ServerConn, Start, State};

const HOST: [u8; 32] = [9; 32];

struct TestBackend {
    /// Klic uctu alice.
    alice: PublicKey,
    probes: u32,
    started: Vec<(u32, Start)>,
    winch: Vec<(u32, u32, u32)>,
    /// Data od klienta (kanal, bajty).
    data: Vec<(u32, u8)>,
}

impl Backend for TestBackend {
    fn probe(&mut self, user: &str, key: &[u8]) -> bool {
        self.probes += 1;
        user == "alice" && key == self.alice.blob().as_slice()
    }
    fn auth(&mut self, user: &str, key: &[u8], data: &[u8], sig: &[u8]) -> Auth {
        if user != "alice" || key != self.alice.blob().as_slice() {
            return Auth::Denied;
        }
        if self.alice.verify(data, sig, false).is_ok() { Auth::Ok } else { Auth::Denied }
    }
    fn start(&mut self, chan: u32, start: Start) -> bool {
        self.started.push((chan, start));
        true
    }
    fn winch(&mut self, chan: u32, cols: u32, rows: u32) {
        self.winch.push((chan, cols, rows));
    }
}

fn pubkey(seed: u8) -> PublicKey {
    let k = ed25519_dalek::SigningKey::from_bytes(&[seed; 32]);
    PublicKey { algo: Algo::Ed25519, key: k.verifying_key().to_bytes(), comment: String::new() }
}

/// Co ma klient udelat po otevreni relace.
#[derive(Clone, Copy)]
enum Plan {
    Exec,
    Shell,
}

struct State0 {
    key: Option<SignKey>,
    hostkey_seen: bool,
    authed: bool,
    plan: Plan,
    exit: Option<u32>,
}

struct Cli {
    runner: Runner<'static, Client>,
    st: State0,
    chan: Option<ChanHandle>,
    got: Vec<u8>,
}

impl Cli {
    fn new(seed: u8, plan: Plan) -> Self {
        let inb = alloc::boxed::Box::leak(vec![0u8; 40_000].into_boxed_slice());
        let outb = alloc::boxed::Box::leak(vec![0u8; 40_000].into_boxed_slice());
        Cli {
            runner: Runner::new_client(inb, outb),
            st: State0 {
                key: Some(SignKey::Ed25519(ed25519_dalek::SigningKey::from_bytes(&[seed; 32]))),
                hostkey_seen: false,
                authed: false,
                plan,
                exit: None,
            },
            chan: None,
            got: Vec::new(),
        }
    }

    fn progress(&mut self) {
        loop {
            let stop = {
                let Ok(ev) = self.runner.progress() else { break };
                matches!(ev, Event::None) || event(&mut self.st, ev)
            };
            if self.st.authed && self.chan.is_none() {
                self.chan = Some(self.runner.open_client_session().unwrap());
            }
            if stop {
                break;
            }
        }
        self.read_all();
    }

    fn read_all(&mut self) {
        if let Some(c) = &self.chan {
            let mut b = [0u8; 1024];
            while let Ok(n) = self.runner.read_channel(c, ChanData::Normal, &mut b) {
                if n == 0 {
                    break;
                }
                self.got.extend_from_slice(&b[..n]);
            }
        }
    }
}

/// Udalost klienta; `true` = konec smycky.
fn event(st: &mut State0, ev: Event<'_, '_>) -> bool {
    {
        match ev {
            Event::Cli(CliEvent::Hostkey(h)) => {
                st.hostkey_seen = true;
                h.accept().unwrap();
            }
            Event::Cli(CliEvent::Username(u)) => u.username("alice").unwrap(),
            // Bez dalsich metod klient skonci s NoAuthMethods (cizi klic).
            Event::Cli(CliEvent::Password(p)) => drop(p.skip()),
            Event::Cli(CliEvent::Pubkey(p)) => match st.key.take() {
                Some(k) => p.pubkey(k).unwrap(),
                None => drop(p.skip()),
            },
            Event::Cli(CliEvent::Authenticated) => st.authed = true,
            Event::Cli(CliEvent::SessionOpened(mut o)) => match st.plan {
                Plan::Exec => o.exec("echo ahoj").unwrap(),
                Plan::Shell => {
                    o.pty(sunset::Pty { term: "xterm".try_into().unwrap(), cols: 100, rows: 30, width: 0, height: 0, modes: Default::default() })
                        .unwrap();
                    o.shell().unwrap();
                }
            },
            Event::Cli(CliEvent::SessionExit(e)) => {
                if let sunset::CliSessionExit::Status(s) = e {
                    st.exit = Some(s);
                }
            }
            Event::None => return true,
            _ => {}
        }
    }
    false
}

/// Udalosti serveru a data od klienta (jako smycka procesu spojeni).
fn serve(s: &mut ServerConn, b: &mut TestBackend) {
    let mut d = [0u8; 256];
    loop {
        s.progress(b).unwrap();
        let mut any = false;
        while let Some((ch, n)) = s.read(&mut d).unwrap() {
            b.data.extend(d[..n].iter().map(|&x| (ch, x)));
            any = true;
        }
        if !any {
            break;
        }
    }
}

/// Prenese bajty mezi klientem a serverem a necha oba zpracovat udalosti.
fn pump(c: &mut Cli, s: &mut ServerConn, b: &mut TestBackend, rounds: usize) {
    let mut buf = vec![0u8; 40_000];
    for _ in 0..rounds {
        c.progress();
        let n = c.runner.output(&mut buf);
        let mut off = 0;
        while off < n {
            off += s.input(&buf[off..n]).unwrap();
            serve(s, b);
        }
        serve(s, b);
        let n = s.output(&mut buf);
        let mut off = 0;
        while off < n {
            off += c.runner.input(&buf[off..n]).unwrap();
            c.progress();
        }
        if s.state() != State::Running {
            break;
        }
    }
}

#[test]
fn prihlaseni_klicem_exec_a_kod_ukonceni() {
    let mut b = TestBackend { alice: pubkey(1), probes: 0, started: Vec::new(), winch: Vec::new(), data: Vec::new() };
    let mut s = ServerConn::new(&HOST).unwrap();
    let mut c = Cli::new(1, Plan::Exec);
    pump(&mut c, &mut s, &mut b, 20);
    assert!(c.st.hostkey_seen && c.st.authed, "prihlaseni");
    assert_eq!(s.user(), Some("alice"));
    assert_eq!(b.started.len(), 1);
    let (chan, start) = b.started[0].clone();
    assert_eq!(start, Start::Exec { command: String::from("echo ahoj"), pty: None });
    // Odpoved prikazu a kod ukonceni.
    assert_eq!(s.write(chan, b"ahoj\n", false).unwrap(), 5);
    s.finish(chan, 3).unwrap();
    pump(&mut c, &mut s, &mut b, 5);
    assert_eq!(c.got, b"ahoj\n");
    assert_eq!(c.st.exit, Some(3));
}

#[test]
fn pty_shell_vstup_a_zmena_velikosti() {
    let mut b = TestBackend { alice: pubkey(2), probes: 0, started: Vec::new(), winch: Vec::new(), data: Vec::new() };
    let mut s = ServerConn::new(&HOST).unwrap();
    let mut c = Cli::new(2, Plan::Shell);
    pump(&mut c, &mut s, &mut b, 20);
    let (chan, start) = b.started[0].clone();
    match start {
        Start::Shell { pty: Some(p) } => assert_eq!((p.cols, p.rows, p.term.as_str()), (100, 30, "xterm")),
        other => panic!("{other:?}"),
    }
    // Klavesy od klienta prijdou na kanale.
    let h = c.chan.as_ref().unwrap();
    c.runner.write_channel(h, ChanData::Normal, b"ls\r").unwrap();
    c.runner
        .term_window_change(h, &sunset::packets::WinChange { cols: 120, rows: 40, width: 0, height: 0 })
        .unwrap();
    pump(&mut c, &mut s, &mut b, 5);
    let got: Vec<u8> = b.data.iter().filter(|d| d.0 == chan).map(|d| d.1).collect();
    assert_eq!(got, b"ls\r");
    assert_eq!(b.winch, vec![(chan, 120, 40)]);
}

#[test]
fn cizi_klic_neprojde() {
    let mut b = TestBackend { alice: pubkey(3), probes: 0, started: Vec::new(), winch: Vec::new(), data: Vec::new() };
    let mut s = ServerConn::new(&HOST).unwrap();
    let mut c = Cli::new(4, Plan::Exec);
    pump(&mut c, &mut s, &mut b, 20);
    assert!(!c.st.authed);
    assert!(b.probes >= 1, "klient se zeptal na klic");
    assert!(b.started.is_empty());
    assert_eq!(s.user(), None);
}

#[test]
fn otisk_klice_hostitele() {
    let (blob, fp) = ServerConn::host_public(&HOST);
    assert_eq!(crate::keys::fingerprint(&blob), fp);
    assert!(PublicKey::from_blob(&blob).is_ok());
}

#[test]
fn velky_prenos_dat_v_kanalu() {
    let mut b = TestBackend { alice: pubkey(5), probes: 0, started: Vec::new(), winch: Vec::new(), data: Vec::new() };
    let mut s = ServerConn::new(&HOST).unwrap();
    let mut c = Cli::new(5, Plan::Exec);
    pump(&mut c, &mut s, &mut b, 20);
    let (chan, _) = b.started[0].clone();
    // 100 KB od klienta (vic nez okno i paket kanalu); klient zapisuje,
    // kolik okno pusti.
    let total = 100 * 1024;
    let data: Vec<u8> = (0..total).map(|i| (i % 251) as u8).collect();
    let mut sent = 0;
    for _ in 0..2000 {
        if sent < total {
            let h = c.chan.as_ref().unwrap();
            if let Ok(n) = c.runner.write_channel(h, ChanData::Normal, &data[sent..]) {
                sent += n;
            }
        }
        pump(&mut c, &mut s, &mut b, 1);
        if b.data.len() == total {
            break;
        }
    }
    let got: Vec<u8> = b.data.iter().filter(|d| d.0 == chan).map(|d| d.1).collect();
    assert_eq!(got.len(), total, "odeslano {sent}");
    assert!(got == data);
}

/// Cely proud bajtu klienta pri prihlaseni a exec (pro fuzz serveru):
/// uvod, vymena klicu, overeni, kanal.
pub(crate) fn client_hello() -> Vec<u8> {
    let mut b = TestBackend { alice: pubkey(1), probes: 0, started: Vec::new(), winch: Vec::new(), data: Vec::new() };
    let mut s = ServerConn::new(&HOST).unwrap();
    let mut c = Cli::new(1, Plan::Exec);
    let mut rec = Vec::new();
    let mut buf = vec![0u8; 40_000];
    for _ in 0..20 {
        c.progress();
        let n = c.runner.output(&mut buf);
        rec.extend_from_slice(&buf[..n]);
        let mut off = 0;
        while off < n {
            off += s.input(&buf[off..n]).unwrap();
            serve(&mut s, &mut b);
        }
        serve(&mut s, &mut b);
        let n = s.output(&mut buf);
        let mut off = 0;
        while off < n {
            off += c.runner.input(&buf[off..n]).unwrap();
            c.progress();
        }
    }
    rec
}
