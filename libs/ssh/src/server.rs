//! Spojeni serveru SSH nad sunset (sans-IO): bajty z TCP a do TCP dela
//! volajici, rozhodnuti (prihlaseni, relace) [`Backend`].
//!
//! - Prihlaseni jen klicem: heslo se neinzeruje ani neprijme. Podpis
//!   overuje `Backend::auth` (v systemu sluzba uctu mimo proces spojeni),
//!   sunset ho sam neoveruje (`set_external_pubkey_verify`).
//! - Po [`MAX_AUTH_TRIES`] neuspesnych pokusech o podpis se spojeni
//!   ukonci.
//! - Kanaly relace: shell (s PTY nebo bez), exec, subsystem; zmena
//!   velikosti okna jde do `Backend::winch`.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use sunset::event::ServEvent;
use sunset::{ChanData, ChanHandle, Event, Runner, Server, SignKey};

/// Kolik neuspesnych podpisu na spojeni (OpenSSH `MaxAuthTries` = 6).
pub const MAX_AUTH_TRIES: u32 = 6;
/// Nejvic kanalu relace na spojeni.
pub const MAX_CHANNELS: usize = 4;
/// Buffer pro casti prihlaseni (klic, zprava, podpis).
const PARTS: usize = 4096;

/// Vysledek overeni klicem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Auth {
    Ok,
    Denied,
    /// Klic sedi, ale ucet se ted prihlasit nemuze (zamceny sifrovany
    /// domov); spojeni se ukonci s hlasenim.
    Locked,
}

/// Terminal klienta (`pty-req`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pty {
    pub cols: u32,
    pub rows: u32,
    pub term: String,
}

/// Co klient na kanale spousti.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    Shell { pty: Option<Pty> },
    Exec { command: String, pty: Option<Pty> },
    Subsystem { name: String },
}

/// Rozhodnuti a relace (v systemu proces spojeni sshd).
pub trait Backend {
    /// Prijme server klic (dotaz klienta pred podpisem)?
    fn probe(&mut self, user: &str, key: &[u8]) -> bool;
    /// Overi podpis prihlaseni (`data` = podepsana data RFC 4252).
    fn auth(&mut self, user: &str, key: &[u8], data: &[u8], sig: &[u8]) -> Auth;
    /// Kanal `chan` spousti `start`; `false` = odmitnout.
    fn start(&mut self, chan: u32, start: Start) -> bool;
    /// Klient zmenil velikost okna.
    fn winch(&mut self, chan: u32, cols: u32, rows: u32);
}

/// Stav spojeni.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Running,
    /// Overeni vycerpalo pokusy nebo ucet je zamceny: zavrit spojeni.
    Refused,
    /// Klient spojeni ukoncil.
    Closed,
}

fn take_pty(ptys: &mut Vec<(u32, Pty)>, num: u32) -> Option<Pty> {
    let i = ptys.iter().position(|p| p.0 == num)?;
    Some(ptys.remove(i).1)
}

pub struct ServerConn {
    runner: Runner<'static, Server>,
    hostkey: SignKey,
    chans: Vec<ChanHandle>,
    /// PTY pozadana na kanalu (pred shell / exec).
    ptys: Vec<(u32, Pty)>,
    failures: u32,
    user: Option<String>,
    state: State,
}

impl ServerConn {
    /// Spojeni s klicem hostitele ed25519 (`seed` = 32 B soukromeho
    /// klice).
    pub fn new(seed: &[u8; 32]) -> Result<Self, sunset::Error> {
        let mut runner = Runner::new_server_owned();
        runner.set_external_pubkey_verify(true)?;
        let hostkey = SignKey::Ed25519(ed25519_dalek::SigningKey::from_bytes(seed));
        Ok(Self { runner, hostkey, chans: Vec::new(), ptys: Vec::new(), failures: 0, user: None, state: State::Running })
    }

    /// Verejny klic hostitele (blob protokolu) a jeho otisk.
    pub fn host_public(seed: &[u8; 32]) -> (Vec<u8>, String) {
        let pk = ed25519_dalek::SigningKey::from_bytes(seed).verifying_key().to_bytes();
        let k = crate::keys::PublicKey { algo: crate::keys::Algo::Ed25519, key: pk, comment: String::new() };
        (k.blob(), k.fingerprint())
    }

    pub fn state(&self) -> State {
        self.state
    }

    /// Prihlaseny uzivatel.
    pub fn user(&self) -> Option<&str> {
        self.user.as_deref()
    }

    /// Bajty od klienta (TCP). Vraci, kolik se prijalo (zbytek poslat
    /// znovu po `progress`).
    pub fn input(&mut self, data: &[u8]) -> Result<usize, sunset::Error> {
        self.runner.input(data)
    }

    /// Ceka sunset na dalsi vstup?
    pub fn wants_input(&self) -> bool {
        self.runner.is_input_ready()
    }

    /// Bajty pro klienta (TCP): zkopiruje do `buf`, vraci delku.
    pub fn output(&mut self, buf: &mut [u8]) -> usize {
        self.runner.output(buf)
    }

    /// Klient zavrel TCP.
    pub fn close(&mut self) {
        self.runner.close_input();
        self.runner.close_output();
        self.state = State::Closed;
    }

    fn chan(&self, num: u32) -> Option<&ChanHandle> {
        self.chans.iter().find(|c| c.num().0 == num)
    }


    /// Zpracuje udalosti sunset, dokud nejake jsou.
    pub fn progress(&mut self, b: &mut dyn Backend) -> Result<(), sunset::Error> {
        loop {
            {
            let ev = self.runner.progress()?;
            match ev {
                Event::Serv(ServEvent::Hostkeys(h)) => {
                    h.hostkeys(&[&self.hostkey])?;
                }
                Event::Serv(ServEvent::FirstAuth(mut a)) => {
                    // Jen klice; heslo se neinzeruje.
                    a.set_auth_methods(false, true)?;
                    a.reject()?;
                }
                Event::Serv(ServEvent::PasswordAuth(a)) => {
                    a.reject()?;
                }
                Event::Serv(ServEvent::PubkeyAuth(a)) => {
                    let user = String::from(a.username().unwrap_or(""));
                    let mut buf = vec![0u8; PARTS];
                    let (kl, ml, sl) = match a.parts(&mut buf) {
                        Ok(p) => p,
                        Err(_) => {
                            a.reject()?;
                            continue;
                        }
                    };
                    let key = &buf[..kl];
                    if !a.has_signature() {
                        if b.probe(&user, key) { a.allow()? } else { a.reject()? }
                        continue;
                    }
                    let (data, sig) = (&buf[kl..kl + ml], &buf[kl + ml..kl + ml + sl]);
                    match b.auth(&user, key, data, sig) {
                        Auth::Ok => {
                            self.user = Some(user);
                            a.allow()?;
                        }
                        Auth::Denied => {
                            self.failures += 1;
                            a.reject()?;
                            if self.failures >= MAX_AUTH_TRIES {
                                self.state = State::Refused;
                                return Ok(());
                            }
                        }
                        Auth::Locked => {
                            a.reject()?;
                            self.state = State::Refused;
                            return Ok(());
                        }
                    }
                }
                Event::Serv(ServEvent::Authenticated) => {}
                Event::Serv(ServEvent::OpenSession(o)) => {
                    if self.user.is_some() && self.chans.len() < MAX_CHANNELS {
                        let h = o.accept()?;
                        self.chans.push(h);
                    } else {
                        o.reject(sunset::ChanFail::SSH_OPEN_RESOURCE_SHORTAGE)?;
                    }
                }
                Event::Serv(ServEvent::SessionPty(p)) => {
                    let num = p.channel().0;
                    match p.size() {
                        Ok((cols, rows, term)) => {
                            let pty = Pty { cols, rows, term: String::from(term) };
                            self.ptys.retain(|x| x.0 != num);
                            self.ptys.push((num, pty));
                            p.succeed()?;
                        }
                        Err(_) => p.fail()?,
                    }
                }
                Event::Serv(ServEvent::SessionEnv(e)) => {
                    // Promenne prostredi od klienta se neprebiraji.
                    e.fail()?;
                }
                Event::Serv(ServEvent::SessionShell(s)) => {
                    let num = s.channel().0;
                    let pty = take_pty(&mut self.ptys, num);
                    if b.start(num, Start::Shell { pty }) { s.succeed()? } else { s.fail()? }
                }
                Event::Serv(ServEvent::SessionExec(s)) => {
                    let num = s.channel().0;
                    let command = String::from(s.command().unwrap_or(""));
                    let pty = take_pty(&mut self.ptys, num);
                    if b.start(num, Start::Exec { command, pty }) { s.succeed()? } else { s.fail()? }
                }
                Event::Serv(ServEvent::SessionSubsystem(s)) => {
                    let num = s.channel().0;
                    let name = String::from(s.command().unwrap_or(""));
                    if b.start(num, Start::Subsystem { name }) { s.succeed()? } else { s.fail()? }
                }
                Event::Serv(ServEvent::SessionWinch { num, cols, rows }) => b.winch(num.0, cols, rows),
                Event::Serv(ServEvent::Defunct) => {
                    self.state = State::Closed;
                    return Ok(());
                }
                Event::Serv(ServEvent::PollAgain) | Event::Progressed => {}
                Event::None => return Ok(()),
                _ => {}
            }
            }
            // Data kanalu cekaji na `read`: dalsi paket az po nich.
            if self.runner.read_channel_ready().is_some() {
                return Ok(());
            }
        }
    }

    /// Data od klienta na nekterem kanale: (kanal, delka v `buf`).
    pub fn read(&mut self, buf: &mut [u8]) -> Result<Option<(u32, usize)>, sunset::Error> {
        let Some((num, dt, _)) = self.runner.read_channel_ready() else { return Ok(None) };
        let Some(i) = self.chans.iter().position(|c| c.num() == num) else { return Ok(None) };
        if dt != ChanData::Normal {
            // Stderr od klienta neni; zahodit.
            self.runner.discard_read_channel(&self.chans[i])?;
            return Ok(None);
        }
        let n = self.runner.read_channel(&self.chans[i], ChanData::Normal, buf)?;
        // 0 = data paketu uz prectena (dalsi az po `progress`).
        Ok((n > 0).then_some((num.0, n)))
    }

    /// Klient poslal EOF na kanale?
    pub fn eof(&self, chan: u32) -> bool {
        self.chan(chan).is_none_or(|h| self.runner.is_channel_eof(h))
    }

    /// Data klientovi; vraci, kolik se vejde do okna kanalu (zbytek
    /// pozdeji).
    pub fn write(&mut self, chan: u32, data: &[u8], stderr: bool) -> Result<usize, sunset::Error> {
        let Some(i) = self.chans.iter().position(|c| c.num().0 == chan) else { return Ok(0) };
        let dt = if stderr { ChanData::Stderr } else { ChanData::Normal };
        self.runner.write_channel(&self.chans[i], dt, data)
    }

    /// Kod ukonceni a zavreni kanalu.
    pub fn finish(&mut self, chan: u32, status: u32) -> Result<(), sunset::Error> {
        let Some(i) = self.chans.iter().position(|c| c.num().0 == chan) else { return Ok(()) };
        let h = self.chans.remove(i);
        self.runner.exit_status(&h, status)?;
        self.runner.channel_done(h)
    }
}
