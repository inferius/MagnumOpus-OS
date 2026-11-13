//! Schranka (protokol `clipproto`, zpravy v obalce `CLIPBOARD`).
//!
//! Samostatny modul: stav je v [`Clipboard`], s kompozitorem ho spojuje
//! jen kontrola gesta (fokus a fyzicky vstup), fokus v zabezpecenem poli
//! a odesilani zprav. Pozdeji pujde do vlastni sluzby beze zmeny zprav.
//!
//! - Nabidnout (kopirovat) i vyzadat (vlozit) smi jen klient okna
//!   s fokusem do [`GESTURE`] po fyzickem stisku klavesy nebo tlacitka.
//!   Jedno gesto = jedno vlozeni.
//! - Data se prenasi line: na vlozeni kompozitor pozada vlastnika
//!   (`Send`) a odpoved preda zadateli (`Deliver`).
//! - Text si kompozitor vyzada hned po nabidce a drzi kopii (do
//!   [`clipproto::MAX_INLINE`]): po skonceni vlastnika zustane.
//! - Nabidka z okna, jehoz uzel s fokusem je `password-input`, se odmitne.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use clipproto::{ClientMsg as CMsg, Denied, ServerMsg as SMsg};
use guiproto::window::ServerMsg;

use crate::{ClientId, Comp};

/// Jak dlouho po fyzickem vstupu smi okno se schrankou pracovat (ns).
pub const GESTURE: u64 = 1_000_000_000;

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
    /// Posledni fyzicky vstup do klienta: (klient, cas, uz pouzit pro
    /// vlozeni).
    gesture: Option<(ClientId, u64, bool)>,
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

    /// Fyzicky vstup dorucen klientovi (stisk klavesy nebo tlacitka).
    pub(crate) fn clip_gesture(&mut self, c: ClientId, time: u64) {
        self.clipboard.gesture = Some((c, time, false));
    }

    /// Klient okna s fokusem.
    fn focused_client(&self) -> Option<ClientId> {
        self.focus.and_then(|g| self.windows.get(&g)).map(|w| w.client)
    }

    /// Ma klient `c` platne gesto (fokus a fyzicky vstup pred chvili)?
    fn clip_allowed(&self, c: ClientId, now: u64, paste: bool) -> bool {
        self.focused_client() == Some(c)
            && self.clipboard.gesture.is_some_and(|(g, t, used)| g == c && now.saturating_sub(t) <= GESTURE && !(paste && used))
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
            CMsg::Offer { formats } => {
                if self.focus_in_secure_field() {
                    return self.trace_clip("nabidka odmitnuta: fokus v zabezpecenem poli");
                }
                if !self.clip_allowed(c, now, false) {
                    return self.trace_clip("nabidka odmitnuta: okno bez fokusu nebo bez gesta");
                }
                // Stara nabidka konci: rozpracovana vlozeni od ni selzou.
                self.clip_fail_owner();
                self.clipboard.owner = Some(c);
                self.clipboard.formats = formats.clone();
                self.clipboard.kept.clear();
                if formats.iter().any(|f| f == clipproto::TEXT) {
                    let s = self.clipboard.next_serial();
                    self.clipboard.wait.insert(s, Wait::Keep(String::from(clipproto::TEXT)));
                    self.clip_send(c, SMsg::Send { serial: s, format: String::from(clipproto::TEXT) });
                }
                self.clip_changed();
            }
            CMsg::Request { serial, format } => {
                if !self.clip_allowed(c, now, true) {
                    return self.clip_send(c, SMsg::Denied { serial, why: Denied::NoGesture });
                }
                if let Some(g) = self.clipboard.gesture.as_mut() {
                    g.2 = true;
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
                        self.clipboard.kept.push((format, data));
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
        if self.clipboard.gesture.is_some_and(|g| g.0 == c) {
            self.clipboard.gesture = None;
        }
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
