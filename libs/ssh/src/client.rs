//! Spojeni klienta SSH nad sunset (sans-IO): bajty z TCP a do TCP dela
//! volajici, rozhodnuti [`ClientBackend`].
//!
//! - Klic serveru overuje `ClientBackend::hostkey` (v systemu
//!   `~/.ssh/known_hosts`, prvni spojeni = dotaz s otiskem, zmena klice =
//!   odmitnuti).
//! - Prihlaseni klicem bez soukromeho klice v procesu: sunset dostane jen
//!   verejny klic (`SignKey::AgentEd25519`) a data k podpisu preda
//!   `ClientBackend::sign` (v systemu sluzba uctu).
//! - Heslo k serveru jen kdyz ho server nabidne a backend ho zada.
//! - Jeden kanal relace: prikaz (`exec`), subsystem (`sftp`) nebo shell.

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use sunset::event::CliEvent;
use sunset::{ChanData, ChanHandle, Client, Event, OwnedSig, Runner, SignKey};

use crate::keys::PublicKey;

/// Nejvic pokusu o heslo na spojeni.
pub const MAX_PASSWORD_TRIES: u32 = 3;
/// Buffer pro data k podpisu (zprava RFC 4252) a klic serveru.
const PARTS: usize = 4096;

/// Rozhodnuti klienta (v systemu program `ssh`).
pub trait ClientBackend {
    /// Prijmout klic serveru? (known_hosts)
    fn hostkey(&mut self, key: &PublicKey) -> bool;
    /// Verejne klice k nabidnuti serveru (v poradi zkouseni).
    fn identities(&mut self) -> Vec<PublicKey>;
    /// Podpis dat prihlaseni (RFC 4252) klicem `key`: 64 B ed25519;
    /// `None` = nepodepsat (zkusi se dalsi metoda).
    fn sign(&mut self, key: &PublicKey, data: &[u8]) -> Option<[u8; 64]>;
    /// Heslo k serveru (`None` = nezadavat).
    fn password(&mut self) -> Option<String>;
}

/// Terminal pro shell (`pty-req`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pty {
    pub cols: u32,
    pub rows: u32,
    pub term: String,
}

/// Co se na kanale relace spusti.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Exec(String),
    Subsystem(String),
    Shell { pty: Option<Pty> },
}

/// Stav spojeni.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CState {
    /// Vymena klicu, prihlaseni, otevreni kanalu.
    Connecting,
    /// Kanal bezi.
    Running,
    /// Konec: kod ukonceni prikazu, kdyz ho server poslal.
    Closed(Option<u32>),
    /// Spojeni odmitnuto (klic serveru, prihlaseni, kanal).
    Failed(Failure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// Klic serveru odmitnut (known_hosts).
    Hostkey,
    /// Zadna metoda prihlaseni neprosla.
    Auth,
    /// Server spojeni ukoncil drive, nez kanal bezel.
    Closed,
}

pub struct ClientConn {
    runner: Runner<'static, Client>,
    user: String,
    req: Request,
    /// Klice k nabidnuti (`None` = jeste se nenacetly).
    idents: Option<VecDeque<PublicKey>>,
    /// Posledni nabidnuty klic (podpis jde s nim).
    offered: Option<PublicKey>,
    passwords: u32,
    chan: Option<ChanHandle>,
    authed: bool,
    eof_sent: bool,
    state: CState,
}

impl ClientConn {
    pub fn new(user: &str, req: Request) -> Self {
        // Buffery jako u serveru (paket 32 KiB s rezervou na sifrovani).
        let inb = alloc::boxed::Box::leak(vec![0u8; 40_000].into_boxed_slice());
        let outb = alloc::boxed::Box::leak(vec![0u8; 40_000].into_boxed_slice());
        Self {
            runner: Runner::new_client(inb, outb),
            user: String::from(user),
            req,
            idents: None,
            offered: None,
            passwords: 0,
            chan: None,
            authed: false,
            eof_sent: false,
            state: CState::Connecting,
        }
    }

    pub fn state(&self) -> &CState {
        &self.state
    }

    /// Bajty od serveru (TCP); vraci, kolik se prijalo.
    pub fn input(&mut self, data: &[u8]) -> Result<usize, sunset::Error> {
        self.runner.input(data)
    }

    pub fn wants_input(&self) -> bool {
        self.runner.is_input_ready()
    }

    /// Bajty pro server (TCP).
    pub fn output(&mut self, buf: &mut [u8]) -> usize {
        self.runner.output(buf)
    }

    /// Server zavrel TCP.
    pub fn close(&mut self) {
        self.runner.close_input();
        self.runner.close_output();
        if matches!(self.state, CState::Connecting) {
            self.state = CState::Failed(Failure::Closed);
        } else if matches!(self.state, CState::Running) {
            self.state = CState::Closed(None);
        }
    }

    /// Zpracuje udalosti sunset, dokud nejake jsou.
    pub fn progress(&mut self, b: &mut dyn ClientBackend) -> Result<(), sunset::Error> {
        if matches!(self.state, CState::Failed(_) | CState::Closed(_)) {
            return Ok(());
        }
        match self.events(b) {
            // Vsechny metody prihlaseni vycerpany (i z `skip` v udalosti).
            Err(sunset::Error::NoAuthMethods) => {
                self.state = CState::Failed(Failure::Auth);
                Ok(())
            }
            r => r,
        }
    }

    fn events(&mut self, b: &mut dyn ClientBackend) -> Result<(), sunset::Error> {
        loop {
            {
                let ev = self.runner.progress()?;
                match ev {
                    Event::Cli(CliEvent::Hostkey(h)) => {
                        let mut buf = vec![0u8; PARTS];
                        let ok = h
                            .hostkey()
                            .ok()
                            .and_then(|k| sunset::sshwire::write_ssh(&mut buf, &k).ok())
                            .and_then(|n| PublicKey::from_blob(&buf[..n]).ok())
                            .is_some_and(|k| b.hostkey(&k));
                        if ok {
                            h.accept()?;
                        } else {
                            // `reject` konci spojeni chybou (zamerne).
                            let _ = h.reject();
                            self.state = CState::Failed(Failure::Hostkey);
                            return Ok(());
                        }
                    }
                    Event::Cli(CliEvent::Banner(_)) => {}
                    Event::Cli(CliEvent::Username(u)) => u.username(&self.user)?,
                    Event::Cli(CliEvent::Pubkey(p)) => {
                        let ids = self.idents.get_or_insert_with(|| b.identities().into_iter().collect());
                        let next = ids.pop_front();
                        match next.as_ref().and_then(|k| ed25519_dalek::VerifyingKey::from_bytes(&k.key).ok()) {
                            Some(vk) => {
                                self.offered = next;
                                p.pubkey(SignKey::AgentEd25519(vk))?;
                            }
                            None => p.skip()?,
                        }
                    }
                    Event::Cli(CliEvent::AgentSign(r)) => {
                        let mut buf = vec![0u8; PARTS];
                        let data = r.message().ok().and_then(|m| sunset::sshwire::write_ssh(&mut buf, &m).ok());
                        let sig = match (data, self.offered.as_ref()) {
                            (Some(n), Some(k)) => b.sign(k, &buf[..n]),
                            _ => None,
                        };
                        match sig {
                            Some(s) => r.signed(&OwnedSig::Ed25519(s))?,
                            None => r.skip()?,
                        }
                    }
                    Event::Cli(CliEvent::Password(p)) => {
                        let pw = if self.passwords < MAX_PASSWORD_TRIES { b.password() } else { None };
                        match pw {
                            Some(pw) => {
                                self.passwords += 1;
                                p.password(pw)?;
                            }
                            None => p.skip()?,
                        }
                    }
                    Event::Cli(CliEvent::Authenticated) => self.authed = true,
                    Event::Cli(CliEvent::SessionOpened(mut o)) => {
                        match &self.req {
                            Request::Exec(c) => o.exec(c)?,
                            Request::Subsystem(s) => o.subsystem(s)?,
                            Request::Shell { pty } => {
                                if let Some(p) = pty {
                                    let term = p.term.as_str().try_into().map_err(|_| sunset::Error::msg("TERM"))?;
                                    o.pty(sunset::Pty { term, cols: p.cols, rows: p.rows, width: 0, height: 0, modes: Default::default() })?;
                                }
                                o.shell()?;
                            }
                        }
                        self.state = CState::Running;
                    }
                    Event::Cli(CliEvent::SessionExit(e)) => {
                        let code = match e {
                            sunset::CliSessionExit::Status(s) => Some(s),
                            _ => Some(255),
                        };
                        self.state = CState::Closed(code);
                    }
                    Event::Cli(CliEvent::Defunct) => {
                        self.state = match &self.state {
                            CState::Connecting if !self.authed => CState::Failed(Failure::Auth),
                            CState::Connecting => CState::Failed(Failure::Closed),
                            CState::Running => CState::Closed(None),
                            s => s.clone(),
                        };
                        return Ok(());
                    }
                    Event::Cli(CliEvent::PollAgain) | Event::Progressed => {}
                    Event::None => return Ok(()),
                    _ => {}
                }
            }
            if self.authed && self.chan.is_none() {
                self.chan = Some(self.runner.open_client_session()?);
            }
            // Data kanalu cekaji na `read`.
            if self.runner.read_channel_ready().is_some() {
                return Ok(());
            }
        }
    }

    /// Data od serveru: `(stderr, delka)` v `buf`.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<Option<(bool, usize)>, sunset::Error> {
        let Some(c) = &self.chan else { return Ok(None) };
        let Some((num, _, _)) = self.runner.read_channel_ready() else { return Ok(None) };
        if num != c.num() {
            return Ok(None);
        }
        // `read_channel` by po EOF uz necetl data, ktera prisla pred nim.
        let (n, dt) = self.runner.read_channel_either(c, buf)?;
        Ok((n > 0).then_some((dt == ChanData::Stderr, n)))
    }

    /// Ceka neco na `read`?
    pub fn has_channel_data(&self) -> bool {
        self.runner.read_channel_ready().is_some()
    }

    /// Server poslal EOF (konec vystupu)?
    pub fn remote_eof(&self) -> bool {
        self.chan.as_ref().is_some_and(|c| self.runner.is_channel_eof(c))
    }

    /// Data serveru (stdin prikazu); vraci, kolik se veslo do okna.
    pub fn write(&mut self, data: &[u8]) -> Result<usize, sunset::Error> {
        if !matches!(self.state, CState::Running) || self.eof_sent {
            return Ok(0);
        }
        let Some(c) = &self.chan else { return Ok(0) };
        self.runner.write_channel(c, ChanData::Normal, data)
    }

    /// Konec vstupu (EOF na kanale; odpoved serveru jeste prijde).
    pub fn send_eof(&mut self) -> Result<(), sunset::Error> {
        if self.eof_sent {
            return Ok(());
        }
        let Some(c) = &self.chan else { return Ok(()) };
        self.eof_sent = true;
        self.runner.send_eof(c)
    }

    /// Zmena velikosti terminalu (shell s PTY).
    pub fn winch(&mut self, cols: u32, rows: u32) -> Result<(), sunset::Error> {
        let Some(c) = &self.chan else { return Ok(()) };
        let w = sunset::packets::WinChange { cols, rows, width: 0, height: 0 };
        self.runner.term_window_change(c, &w)
    }
}
