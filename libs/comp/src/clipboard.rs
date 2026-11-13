//! Schranka (protokol `clipproto`, zpravy v obalce `CLIPBOARD`).
//!
//! Samostatny modul: stav je v [`Clipboard`], s kompozitorem ho spojuje
//! jen kontrola gesta (fokus a fyzicky vstup), fokus v zabezpecenem poli
//! a odesilani zprav. Pozdeji pujde do vlastni sluzby beze zmeny zprav.
//!
//! - Nabidnout (kopirovat) i vyzadat (vlozit) smi jen klient okna
//!   s fokusem a jen s cislem gesta (`InputRecord::serial`), ktere mu
//!   kompozitor doruci s fyzickym stiskem klavesy nebo tlacitka: jen
//!   jednou, jen z poslednich [`GESTURES`] stisku klienta a nejvys
//!   [`GESTURE_MAX`] po doruceni.
//! - Data se prenasi line: na vlozeni kompozitor pozada vlastnika
//!   (`Send`) a odpoved preda zadateli (`Deliver`).
//! - Vsechny nabidnute formaty si kompozitor vyzada hned po nabidce
//!   a drzi jejich kopii (kazdy do [`clipproto::MAX_INLINE`], dohromady
//!   do [`KEEP_MAX`]): po skonceni vlastnika zustanou (text, seznam cest).
//! - Nabidka z okna, jehoz uzel s fokusem je `password-input`, se odmitne.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use clipproto::{ClientMsg as CMsg, Denied, ServerMsg as SMsg};
use guiproto::window::ServerMsg;

use crate::{ClientId, Comp};

/// Zalozni horni mez platnosti gesta od doruceni stisku (ns).
pub const GESTURE_MAX: u64 = 10_000_000_000;

/// Kolik poslednich stisku klienta se pamatuje (rychle psani po Ctrl+C).
pub const GESTURES: usize = 8;

/// Doruceny fyzicky stisk: cislo, cas doruceni, uz pouzity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Gesture {
    serial: u32,
    time: u64,
    used: bool,
}

/// Nejvic bajtu kopii vsech formatu, ktere kompozitor drzi.
pub const KEEP_MAX: usize = 256 * 1024;

/// Komu patri odpoved vlastnika.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Wait {
    /// Zadatel (klient, jeho cislo pozadavku, format).
    Paste(ClientId, u32, String),
    /// Kopie pro dobu po skonceni vlastnika.
    Keep(String),
}

#[derive(Default)]
pub struct Clipboard {
    /// Vlastnik nabidky (None = jen kopie nebo prazdno).
    owner: Option<ClientId>,
    formats: Vec<String>,
    /// Kopie malych formatu (format, data).
    kept: Vec<(String, Vec<u8>)>,
    serial: u32,
    wait: BTreeMap<u32, Wait>,
    /// Posledni fyzicke stisky dorucene klientum (nejvys [`GESTURES`]
    /// na klienta).
    gestures: BTreeMap<ClientId, Vec<Gesture>>,
    /// Posledni pridelene cislo gesta (0 = zadne).
    gesture_serial: u32,
}

impl Clipboard {
    /// Formaty, ktere schranka prave nabizi.
    pub fn formats(&self) -> Vec<String> {
        if self.owner.is_some() {
            self.formats.clone()
        } else {
            self.kept.iter().map(|(f, _)| f.clone()).collect()
        }
    }

    fn next_serial(&mut self) -> u32 {
        self.serial = self.serial.wrapping_add(1).max(1);
        self.serial
    }
}

impl Comp {
    fn clip_send(&mut self, c: ClientId, m: SMsg) {
        self.send(c, ServerMsg::Clipboard { data: m.encode() });
    }

    /// Fyzicky stisk klavesy nebo tlacitka doruceny klientovi: prideli
    /// mu cislo gesta (do `InputRecord::serial`).
    pub(crate) fn clip_gesture(&mut self, c: ClientId, time: u64) -> u32 {
        let cb = &mut self.clipboard;
        cb.gesture_serial = cb.gesture_serial.wrapping_add(1).max(1);
        let g = cb.gestures.entry(c).or_default();
        if g.len() >= GESTURES {
            g.remove(0);
        }
        g.push(Gesture { serial: cb.gesture_serial, time, used: false });
        cb.gesture_serial
    }

    /// Klient okna s fokusem.
    fn focused_client(&self) -> Option<ClientId> {
        self.focus.and_then(|g| self.windows.get(&g)).map(|w| w.client)
    }

    /// Vezme gesto `serial` klienta `c`: okno ma fokus, cislo mu
    /// kompozitor dorucil (mezi poslednimi [`GESTURES`]), jeste nebylo
    /// pouzite a neni starsi nez [`GESTURE_MAX`]. Platne gesto se tim
    /// spotrebuje (jedno gesto = jedna akce).
    fn clip_take_gesture(&mut self, c: ClientId, serial: u32, now: u64) -> bool {
        if serial == 0 || self.focused_client() != Some(c) {
            return false;
        }
        let Some(g) = self.clipboard.gestures.get_mut(&c).and_then(|v| v.iter_mut().find(|g| g.serial == serial)) else { return false };
        if g.used || now.saturating_sub(g.time) > GESTURE_MAX {
            return false;
        }
        g.used = true;
        true
    }

    /// Je v okne s fokusem zamereny uzel s heslem?
    fn focus_in_secure_field(&self) -> bool {
        let Some(w) = self.focus.and_then(|g| self.windows.get(&g)) else { return false };
        w.a11y.nodes().any(|n| n.role == guiproto::a11y::Role::PasswordInput && n.state & guiproto::a11y::state::FOCUSED != 0)
    }

    /// Zprava schranky od klienta `c` (obalka `CLIPBOARD`).
    pub(crate) fn clipboard_msg(&mut self, c: ClientId, data: &[u8], now: u64) {
        let Ok(m) = CMsg::decode(data) else {
            return self.error(c, guiproto::window::op::CLIPBOARD, guiproto::Error::InvalidArgs);
        };
        match m {
            CMsg::Offer { gesture, formats } => {
                if self.focus_in_secure_field() {
                    return self.trace_clip("nabidka odmitnuta: fokus v zabezpecenem poli");
                }
                if !self.clip_take_gesture(c, gesture, now) {
                    return self.trace_clip("nabidka odmitnuta: okno bez fokusu nebo bez platneho gesta");
                }
                // Stara nabidka konci: rozpracovana vlozeni od ni selzou.
                self.clip_fail_owner();
                self.clipboard.owner = Some(c);
                self.clipboard.formats = formats.clone();
                self.clipboard.kept.clear();
                for f in &formats {
                    let s = self.clipboard.next_serial();
                    self.clipboard.wait.insert(s, Wait::Keep(f.clone()));
                    self.clip_send(c, SMsg::Send { serial: s, format: f.clone() });
                }
                self.clip_changed();
            }
            CMsg::Request { gesture, serial, format } => {
                if !self.clip_take_gesture(c, gesture, now) {
                    return self.clip_send(c, SMsg::Denied { serial, why: Denied::NoGesture });
                }
                match self.clipboard.owner {
                    Some(o) if self.clipboard.formats.contains(&format) => {
                        let s = self.clipboard.next_serial();
                        self.clipboard.wait.insert(s, Wait::Paste(c, serial, format.clone()));
                        self.clip_send(o, SMsg::Send { serial: s, format });
                    }
                    _ => {
                        let kept = self.clipboard.kept.iter().find(|(f, _)| *f == format).map(|(_, d)| d.clone());
                        match kept {
                            Some(data) => self.clip_send(c, SMsg::Deliver { serial, format, data }),
                            None => self.clip_send(c, SMsg::Denied { serial, why: Denied::Empty }),
                        }
                    }
                }
            }
            CMsg::Data { serial, data } => {
                // Odpovedet smi jen soucasny vlastnik na nas pozadavek.
                if self.clipboard.owner != Some(c) {
                    return;
                }
                match self.clipboard.wait.remove(&serial) {
                    Some(Wait::Paste(to, s, format)) => self.clip_send(to, SMsg::Deliver { serial: s, format, data }),
                    Some(Wait::Keep(format)) => {
                        self.clipboard.kept.retain(|(f, _)| *f != format);
                        let used: usize = self.clipboard.kept.iter().map(|(_, d)| d.len()).sum();
                        if used + data.len() <= KEEP_MAX {
                            self.clipboard.kept.push((format, data));
                        }
                    }
                    None => {}
                }
            }
        }
    }

    /// Vlastnik skoncil nebo ho nahradil jiny: cekajici vlozeni selzou.
    fn clip_fail_owner(&mut self) {
        let waits = core::mem::take(&mut self.clipboard.wait);
        for (_, w) in waits {
            if let Wait::Paste(to, s, _) = w {
                self.clip_send(to, SMsg::Denied { serial: s, why: Denied::Unavailable });
            }
        }
    }

    /// Klient skoncil: zustane kopie jeho nabidky.
    pub(crate) fn clip_client_gone(&mut self, c: ClientId) {
        self.clipboard.gestures.remove(&c);
        self.clipboard.wait.retain(|_, w| !matches!(w, Wait::Paste(to, ..) if *to == c));
        if self.clipboard.owner == Some(c) {
            self.clip_fail_owner();
            self.clipboard.owner = None;
            self.clipboard.formats.clear();
        }
    }

    /// Oknu s fokusem: co je ve schrance (po zmene obsahu nebo fokusu).
    pub(crate) fn clip_changed(&mut self) {
        if let Some(c) = self.focused_client() {
            let formats = self.clipboard.formats();
            self.clip_send(c, SMsg::Changed { formats });
        }
    }

    fn trace_clip(&mut self, why: &str) {
        let now = self.input_now;
        self.trace(now, format_args!("schranka: {why}"));
    }
}
