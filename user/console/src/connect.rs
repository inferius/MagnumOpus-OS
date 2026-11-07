//! Pripojeni okna terminalu k termd pres init, bez blokovani smycky.
//!
//! Okno (konzole, `gui-term`) ma od initu kanal pripojeni: posle
//! `connect`, init spusti termd a odpovi `ok` s kanalem k nemu (`err
//! CANCELED` = odpoved na drivejsi pokus, ktery init nahradil novejsim;
//! jina `err` = nejde). Odpoved se zpracuje jako dalsi polozka cekani
//! smycky okna, takze okno mezitim kresli a bere vstup - a init muze
//! okno zaroven ridit (konzoli posila `display` hned po spusteni).
//!
//! Sdileny soubor: `gui-term` ho vklada pres `#[path]`.

use alloc::vec::Vec;
use core::time::Duration;

use libos::{Channel, Error, Handle, println, time};

/// Jak dlouho cekat na odpoved initu.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Prodleva pred dalsim pokusem (pak dvojnasobna, nejvys `RETRY_MAX`).
const RETRY: Duration = Duration::from_millis(300);
const RETRY_MAX: Duration = Duration::from_secs(5);

pub struct Connector {
    /// Kanal pripojeni (`None` = init ho zavrel nebo ho okno nedostalo).
    pub conn: Option<Channel>,
    /// Ceka se na odpoved (nejzazsi cas).
    pending: Option<u64>,
    retry_at: Option<u64>,
    delay: Duration,
    who: &'static str,
}

impl Connector {
    pub fn new(conn: Option<Channel>, who: &'static str) -> Self {
        Self { conn, pending: None, retry_at: None, delay: RETRY, who }
    }

    /// Posle `connect` (zastarale odpovedi drivejsich pokusu zahodi:
    /// jejich termd init pri dalsim `connect` ukonci).
    pub fn start(&mut self) {
        let Some(c) = &self.conn else { return };
        while c.try_read().is_ok() {}
        match c.write(b"connect", Vec::new()) {
            Ok(()) => {
                self.pending = Some(time::deadline_after(TIMEOUT));
                self.retry_at = None;
            }
            Err(e) => {
                println!("{}: kanal pripojeni k termd: {e}", self.who);
                self.conn = None;
            }
        }
    }

    /// Spojeni s termd spadlo: zkusit znovu po prodleve.
    pub fn lost(&mut self) {
        if self.conn.is_some() && self.pending.is_none() {
            self.retry_at = Some(time::deadline_after(self.delay));
        }
    }

    /// Ceka se na odpoved nebo na dalsi pokus?
    pub fn waiting(&self) -> bool {
        self.pending.is_some() || self.retry_at.is_some()
    }

    /// Handle k cekani (jen kdyz se ceka na odpoved).
    pub fn handle(&self) -> Option<&Handle> {
        self.pending.and(self.conn.as_ref()).map(|c| c.handle())
    }

    /// Termin probuzeni (konec cekani na odpoved nebo dalsi pokus).
    pub fn deadline(&self) -> Option<u64> {
        self.pending.or(self.retry_at)
    }

    fn fail(&mut self) {
        self.pending = None;
        self.retry_at = Some(time::deadline_after(self.delay));
        self.delay = (self.delay * 2).min(RETRY_MAX);
    }

    /// Po probuzeni: vrati kanal k termd, kdyz prisel.
    pub fn poll(&mut self) -> Option<Channel> {
        if self.pending.is_some() {
            loop {
                let Some(c) = &self.conn else { return None };
                match c.try_read() {
                    Ok(m) if m.data.starts_with(b"ok") => {
                        self.pending = None;
                        self.delay = RETRY;
                        match m.handles.into_iter().next() {
                            Some(h) => return Some(Channel::from_handle(h)),
                            None => {
                                self.fail();
                                return None;
                            }
                        }
                    }
                    Ok(m) if m.data.starts_with(b"err CANCELED") => continue,
                    Ok(m) => {
                        println!("{}: termd nejde spustit: {}", self.who, alloc::string::String::from_utf8_lossy(&m.data));
                        self.fail();
                        return None;
                    }
                    Err(Error::ShouldWait) => break,
                    Err(_) => {
                        println!("{}: init zavrel kanal pripojeni", self.who);
                        self.conn = None;
                        self.pending = None;
                        return None;
                    }
                }
            }
            if self.pending.is_some_and(|d| time::now() >= d) {
                println!("{}: init neodpovedel na connect", self.who);
                self.fail();
            }
        } else if self.retry_at.is_some_and(|r| time::now() >= r) {
            self.start();
        }
        None
    }
}
