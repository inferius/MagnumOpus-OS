//! sshd-conn: jedno spojeni SSH (`docs/cs/ssh.md`). Cte data ze site
//! (neduveryhodny vstup), proto bezi jako samostatny proces v jobu
//! s limitem pameti a bez prav: overeni preda sshd (a ten sluzbe uctu),
//! relaci dostane az po prihlaseni jako kanal okna terminalu.
//!
//! Start (kind USER): spojeni TCP (kanal netproto), kanal k sshd. Na
//! bootstrap prijde `key=<hex>\npeer=<adresa>`.
//!
//! - shell s PTY: okno terminalu (`term::ui::App`) prevedene na VT100
//!   (alternativni obrazovka, prekresleni od leveho horniho rohu),
//!   klavesy z `term::vtkeys`; zmena velikosti okna = `App::resize`;
//!   `exit` nebo Ctrl+D na prazdnem radku kanal ukonci.
//! - shell bez PTY nebo exec: radek v relaci, vystup jako text
//!   (`term::vt::item`), kod ukonceni 0 / 1.
//! - subsystem (sftp): zatim ne [navrzeno].

#![no_std]
#![no_main]

extern crate alloc;

mod proto;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use libos::{Channel, Error, Handle, Signals, Startup, println, time};
use netproto::{Deadline, TcpStream};
use ssh::server::{Auth, Backend, Pty, ServerConn, Start, State};
use term::proto::{ClientMsg, ServerMsg};
use term::theme::Theme;
use term::ui::{App, Key};
use term::vtkeys::VtKeys;

use proto::Req;

const TICK: Duration = Duration::from_millis(100);
/// Odpoved sshd / initu.
const REPLY: Duration = Duration::from_secs(15);

/// Volani sshd (synchronni, na rozdil od dat neni ceho se bat).
fn call(parent: &Channel, r: &Req) -> Result<libos::Message, Error> {
    parent.write(&r.encode(), Vec::new())?;
    parent.read(Some(time::deadline_after(REPLY)))
}

struct Back<'a> {
    parent: &'a Channel,
    starts: Vec<(u32, Start)>,
}

impl Backend for Back<'_> {
    fn probe(&mut self, user: &str, key: &[u8]) -> bool {
        call(self.parent, &Req::Probe { user: user.into(), key: key.to_vec() }).is_ok_and(|m| m.data.first() == Some(&1))
    }
    fn auth(&mut self, user: &str, key: &[u8], data: &[u8], sig: &[u8]) -> Auth {
        let r = Req::Auth { user: user.into(), key: key.to_vec(), data: data.to_vec(), sig: sig.to_vec() };
        match call(self.parent, &r).map(|m| m.data.first().copied()) {
            Ok(Some(proto::AUTH_OK)) => Auth::Ok,
            Ok(Some(proto::AUTH_LOCKED)) => Auth::Locked,
            _ => Auth::Denied,
        }
    }
    fn start(&mut self, chan: u32, start: Start) -> bool {
        if matches!(start, Start::Subsystem { .. }) {
            // sftp [navrzeno].
            return false;
        }
        self.starts.push((chan, start));
        true
    }
    fn winch(&mut self, _chan: u32, _cols: u32, _rows: u32) {}
}

/// Interaktivni terminal (shell s PTY).
struct Shell {
    app: App,
    keys: VtKeys,
    dirty: bool,
    drawn_at: u64,
    /// Vstup od klienta pred pripojenim k relaci (jinak by se ztratil).
    early: Vec<u8>,
}

/// Velikost terminalu klienta; 0 (klient bez terminalu, `ssh -tt`
/// z roury) = 80 x 24.
fn size(cols: u32, rows: u32) -> (usize, usize) {
    let c = if cols == 0 { 80 } else { cols as usize };
    let r = if rows == 0 { 24 } else { rows as usize };
    (c.clamp(20, 500), r.clamp(5, 200))
}

/// Radek bez terminalu (exec, shell bez PTY).
struct Line {
    line: String,
    block: Option<u64>,
    /// Shell bez PTY: radky ze vstupu kanalu.
    input: Option<Vec<u8>>,
}

enum Kind {
    Shell(Shell),
    Line(Line),
}

/// Kanal relace a jeho spojeni s termd.
struct Chan {
    num: u32,
    kind: Kind,
    termd: Channel,
    session: u64,
    /// Data pro klienta, ktera se jeste nevesla do okna kanalu.
    out: Vec<u8>,
    /// Konec: kod ukonceni (az se odesle `out`).
    exit: Option<u32>,
}

/// Snimek okna terminalu jako VT100: kurzor vlevo nahoru, radky s vymazanim
/// zbytku, CR LF.
fn vt_frame(app: &App) -> Vec<u8> {
    let f = app.frame();
    let f = f.strip_prefix("\x1b[2J\x1b[H").unwrap_or(&f);
    let mut s = String::from("\x1b[H");
    for (i, l) in f.split('\n').enumerate() {
        if i > 0 {
            s.push_str("\r\n");
        }
        s.push_str(l);
        s.push_str("\x1b[K");
    }
    s.push_str("\x1b[J");
    s.into_bytes()
}

/// Kanal okna terminalu -> termd (`connect` u initu).
fn connect_termd(win: &Channel) -> Result<Channel, Error> {
    win.write(b"connect", Vec::new())?;
    let m = win.read(Some(time::deadline_after(REPLY)))?;
    if !m.data.starts_with(b"ok") {
        println!("sshd-conn: termd: {}", String::from_utf8_lossy(&m.data));
        return Err(Error::IoError);
    }
    m.handles.into_iter().next().map(Channel::from_handle).ok_or(Error::IoError)
}

fn send(termd: &Channel, msgs: Vec<ClientMsg>) -> bool {
    msgs.iter().all(|m| termd.write(&m.encode(), Vec::new()).is_ok())
}

struct Worker {
    conn: ServerConn,
    stream: TcpStream,
    parent: Channel,
    /// Kanal okna terminalu od initu (po vyzvednuti relace).
    win: Option<Channel>,
    chans: Vec<Chan>,
    tcp_in: Vec<u8>,
    tcp_out: Vec<u8>,
    closed: bool,
}

impl Worker {
    /// Vyzvednuti relace (jednou, pri prvnim kanalu).
    fn claim(&mut self) -> Result<(), Error> {
        if self.win.is_some() {
            return Ok(());
        }
        let m = call(&self.parent, &Req::Claim)?;
        if !m.data.starts_with(b"ok") {
            println!("sshd-conn: relace: {}", String::from_utf8_lossy(&m.data));
            return Err(Error::AccessDenied);
        }
        let mut hs = m.handles.into_iter();
        self.win = hs.next().map(Channel::from_handle);
        // Relace FS uctu (sftp [navrzeno]).
        drop(hs);
        Ok(())
    }

    fn open(&mut self, num: u32, start: Start) -> Result<(), Error> {
        self.claim()?;
        let termd = connect_termd(self.win.as_ref().ok_or(Error::BadState)?)?;
        let kind = match start {
            Start::Shell { pty: Some(Pty { cols, rows, .. }) } => {
                let (cols, rows) = size(cols, rows);
                let mut app = App::new(Theme::dark(), cols, rows);
                app.welcome = true;
                app.start(0);
                send(&termd, app.take_out());
                Kind::Shell(Shell { app, keys: VtKeys::new(), dirty: true, drawn_at: 0, early: Vec::new() })
            }
            Start::Shell { pty: None } => {
                send(&termd, vec![ClientMsg::Attach { session: 0 }]);
                Kind::Line(Line { line: String::new(), block: None, input: Some(Vec::new()) })
            }
            Start::Exec { command, .. } => {
                send(&termd, vec![ClientMsg::Attach { session: 0 }]);
                Kind::Line(Line { line: command, block: None, input: None })
            }
            Start::Subsystem { .. } => return Err(Error::NotSupported),
        };
        let mut c = Chan { num, kind, termd, session: 0, out: Vec::new(), exit: None };
        if let Kind::Shell(_) = c.kind {
            c.out.extend_from_slice(b"\x1b[?1049h\x1b[?25l");
        }
        self.chans.push(c);
        Ok(())
    }

    /// Zpravy termd jednoho kanalu.
    fn termd_msgs(c: &mut Chan) {
        while let Ok(m) = c.termd.try_read() {
            let Ok(sm) = ServerMsg::decode(&m.data) else { continue };
            match &mut c.kind {
                Kind::Shell(s) => {
                    s.app.on_msg(sm);
                    s.dirty = true;
                    // Pripojeno: vstup, ktery prisel driv.
                    if s.app.session != 0 && !s.early.is_empty() {
                        let early = core::mem::take(&mut s.early);
                        Self::input(c, &early);
                        return;
                    }
                }
                Kind::Line(l) => match sm {
                    ServerMsg::Attached { session, .. } => {
                        c.session = session;
                        if l.input.is_none() {
                            let line = core::mem::take(&mut l.line);
                            let _ = c.termd.write(&ClientMsg::Run { session, line }.encode(), Vec::new());
                        }
                    }
                    ServerMsg::BlockStart { block, .. } if l.block.is_none() => l.block = Some(block),
                    ServerMsg::Item { block, item, .. } if Some(block) == l.block => {
                        // Bez PTY obycejne LF (klient nema terminal).
                        c.out.extend(term::vt::item(&item, None).bytes());
                    }
                    ServerMsg::BlockEnd { block, ok, .. } if Some(block) == l.block => {
                        l.block = None;
                        if l.input.is_none() {
                            c.exit = Some(if ok { 0 } else { 1 });
                        }
                    }
                    _ => {}
                },
            }
        }
    }

    /// Data od klienta pro kanal.
    fn input(c: &mut Chan, data: &[u8]) {
        match &mut c.kind {
            Kind::Shell(s) => {
                if s.app.session == 0 {
                    s.early.extend_from_slice(data);
                    return;
                }
                for k in s.keys.feed(data) {
                    // exit / Ctrl+D na prazdnem radku: konec terminalu.
                    let empty = s.app.editor.text.trim().is_empty();
                    if (k == Key::Ctrl('d') && empty) || (k == Key::Enter && s.app.editor.text.trim() == "exit") {
                        c.exit = Some(0);
                        return;
                    }
                    s.app.key(k);
                    s.dirty = true;
                }
                send(&c.termd, s.app.take_out());
            }
            Kind::Line(l) => {
                if let Some(buf) = &mut l.input {
                    buf.extend_from_slice(data);
                    while let Some(i) = buf.iter().position(|&b| b == b'\n') {
                        let line: Vec<u8> = buf.drain(..=i).collect();
                        let line = String::from_utf8_lossy(&line).trim().into();
                        if line == "exit" {
                            c.exit = Some(0);
                            return;
                        }
                        if c.session != 0 {
                            let _ = c.termd.write(&ClientMsg::Run { session: c.session, line }.encode(), Vec::new());
                        }
                    }
                }
            }
        }
    }

    fn step(&mut self, back_starts: Vec<(u32, Start)>) {
        for (num, start) in back_starts {
            if let Err(e) = self.open(num, start) {
                println!("sshd-conn: kanal {num}: {e}");
                let _ = self.conn.finish(num, 1);
            }
        }
        // Data od klienta.
        let mut buf = [0u8; 4096];
        while let Ok(Some((num, n))) = self.conn.read(&mut buf) {
            if let Some(c) = self.chans.iter_mut().find(|c| c.num == num) {
                Self::input(c, &buf[..n]);
            }
        }
        let now = time::now();
        for c in &mut self.chans {
            Self::termd_msgs(c);
            if let Kind::Shell(s) = &mut c.kind {
                // Samotne Esc po pauze.
                if s.keys.has_pending() {
                    for k in s.keys.flush() {
                        s.app.key(k);
                    }
                    send(&c.termd, s.app.take_out());
                    s.dirty = true;
                }
                s.app.tick(time::wall().map(|t| t.secs.saturating_mul(1_000_000_000)));
                if s.app.busy() {
                    s.dirty = true;
                }
                // Prekreslit nejvys 25x za sekundu a jen kdyz je misto.
                if s.dirty && c.out.is_empty() && now >= s.drawn_at + 40_000_000 {
                    c.out.extend(vt_frame(&s.app));
                    s.dirty = false;
                    s.drawn_at = now;
                }
            }
        }
        // Data a konce kanalu ke klientovi.
        let mut done = Vec::new();
        for c in &mut self.chans {
            while !c.out.is_empty() {
                match self.conn.write(c.num, &c.out, false) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        c.out.drain(..n);
                    }
                }
            }
            if let Some(code) = c.exit
                && c.out.is_empty()
            {
                if let Kind::Shell(_) = c.kind {
                    let _ = self.conn.write(c.num, b"\x1b[?1049l\x1b[?25h", false);
                }
                let _ = self.conn.finish(c.num, code);
                done.push(c.num);
            }
        }
        self.chans.retain(|c| !done.contains(&c.num));
    }

    /// TCP: prijmout, co prislo, a odeslat, co je pripravene.
    fn pump_tcp(&mut self) -> Result<(), Error> {
        let mut buf = [0u8; 8192];
        loop {
            match self.stream.recv_ctx(&mut buf, Deadline::TRY, None) {
                Ok(0) => {
                    self.closed = true;
                    break;
                }
                Ok(n) => self.tcp_in.extend_from_slice(&buf[..n]),
                Err(Error::ShouldWait) | Err(Error::TimedOut) => break,
                Err(e) => return Err(e),
            }
            if self.tcp_in.len() > 64 * 1024 {
                break;
            }
        }
        Ok(())
    }

    fn flush_tcp(&mut self) -> Result<(), Error> {
        let mut buf = [0u8; 8192];
        loop {
            let n = self.conn.output(&mut buf);
            if n == 0 {
                break;
            }
            self.tcp_out.extend_from_slice(&buf[..n]);
        }
        while !self.tcp_out.is_empty() {
            match self.stream.send_ctx(&self.tcp_out, Deadline::TRY) {
                Ok(0) => break,
                Ok(n) => {
                    self.tcp_out.drain(..n);
                }
                Err(Error::ShouldWait) | Err(Error::TimedOut) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

libos::entry!(main);

fn main(mut startup: Startup) -> i64 {
    let ctl = startup.bootstrap;
    let mut hs = core::mem::take(&mut startup.handles).into_iter();
    let (Some(tcp), Some(parent)) = (hs.next(), hs.next()) else { return 1 };
    let cfg = match ctl.read(Some(time::deadline_after(Duration::from_secs(5)))) {
        Ok(m) => String::from_utf8(m.data).unwrap_or_default(),
        Err(_) => return 2,
    };
    let key: Option<[u8; 32]> =
        cfg.lines().find_map(|l| l.strip_prefix("key=")).and_then(|k| proto::unhex(k.trim())).and_then(|v| v.try_into().ok());
    let Some(key) = key else { return 3 };
    let peer = String::from(cfg.lines().find_map(|l| l.strip_prefix("peer=")).unwrap_or("?"));
    let stream = match TcpStream::from_channel(Channel::from_handle(tcp)) {
        Ok(s) => s,
        Err(_) => return 4,
    };
    let ready = match stream.readiness() {
        Ok(r) => r,
        Err(_) => return 5,
    };
    let conn = match ServerConn::new(&key) {
        Ok(c) => c,
        Err(_) => return 6,
    };
    let mut w = Worker {
        conn,
        stream,
        parent: Channel::from_handle(parent),
        win: None,
        chans: Vec::new(),
        tcp_in: Vec::new(),
        tcp_out: Vec::new(),
        closed: false,
    };
    let _ = &peer;
    loop {
        if w.pump_tcp().is_err() {
            break;
        }
        // Vstup do sunset, dokud ho bere.
        let mut back = Back { parent: &w.parent, starts: Vec::new() };
        loop {
            if !w.tcp_in.is_empty() {
                match w.conn.input(&w.tcp_in) {
                    Ok(n) => {
                        w.tcp_in.drain(..n);
                    }
                    Err(_) => {
                        w.closed = true;
                        break;
                    }
                }
            }
            if w.conn.progress(&mut back).is_err() {
                w.closed = true;
                break;
            }
            if w.tcp_in.is_empty() || !w.conn.wants_input() {
                break;
            }
        }
        let starts = back.starts;
        w.step(starts);
        if w.flush_tcp().is_err() {
            break;
        }
        if w.closed || w.conn.state() != State::Running {
            let _ = w.flush_tcp();
            break;
        }
        // Cekani: data ze site, zpravy termd, tik terminalu.
        let rd = Signals::READABLE | Signals::PEER_CLOSED;
        let mut mask = netproto::ready::READABLE | netproto::ready::HUP;
        if !w.tcp_out.is_empty() {
            mask = mask | netproto::ready::WRITABLE;
        }
        let mut items: Vec<(&Handle, Signals)> = vec![(ready.handle(), mask)];
        for c in &w.chans {
            items.push((c.termd.handle(), rd));
        }
        let _ = libos::wait_many(&items, Some(time::deadline_after(TICK)));
    }
    0
}
