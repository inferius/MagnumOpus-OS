//! Panel Vlastnosti (kontextova nabidka, Alt+Enter): karta Obecne
//! (metadata jako v nahledu) a karta Opravneni (vlastnik, sdileni s
//! urovnemi, zdedene granty s puvodem, dedeni do obsahu slozky).
//!
//! - Urovne jsou vzestupne: Zobrazit < Spoustet < Upravovat < Spravovat
//!   (`Level`). Jina kombinace prav (prikaz `acl`) je "Vlastni" a zustane,
//!   dokud ji nikdo nezmeni.
//! - Menit smi jen kdo ma pravo Spravovat (vlastnik ho ma vzdy); jinak je
//!   karta jen pro cteni s vysvetlenim. Vlastnika meni jen system.
//! - Zmeny se ukladaji az tlacitkem (jeden `SET_ACL`); odebrani vlastniho
//!   prava Spravovat (kdo neni vlastnik) chce potvrzeni.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use fsproto::perm::{ACL_PROTECTED, Acl, AclView, EVERYONE, Grant, grant, rights};
use fsproto::{Stat, kind};

use crate::backend::Backend;

/// Skupina vsech lidi (`accountproto::ids::GROUP_USERS`).
pub const GROUP_USERS: u64 = 1_000_000;

/// Uroven sdileni.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    View,
    Run,
    Edit,
    Manage,
    /// Jina kombinace prav (zachova se beze zmeny).
    Custom(u8),
}

impl Level {
    /// Urovne v poradi vyberu.
    pub const ALL: [Level; 4] = [Level::View, Level::Run, Level::Edit, Level::Manage];

    pub fn of(r: u8) -> Self {
        match r {
            rights::VIEW => Level::View,
            x if x == rights::RUN => Level::Run,
            x if x == rights::EDIT | rights::EXECUTE => Level::Edit,
            rights::ALL => Level::Manage,
            _ => Level::Custom(r),
        }
    }

    pub fn rights(self) -> u8 {
        match self {
            Level::View => rights::VIEW,
            Level::Run => rights::RUN,
            Level::Edit => rights::EDIT | rights::EXECUTE,
            Level::Manage => rights::ALL,
            Level::Custom(r) => r,
        }
    }

    pub fn label(self) -> String {
        match self {
            Level::View => String::from("Zobrazit"),
            Level::Run => String::from("Spouštět"),
            Level::Edit => String::from("Upravovat"),
            Level::Manage => String::from("Spravovat"),
            Level::Custom(r) => format!("Vlastní ({})", rights_words(r)),
        }
    }

    /// Index ve vyberu (`Custom` = za ctyrmi urovnemi).
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|l| *l == self).unwrap_or(Self::ALL.len())
    }
}

/// Prava slovy ("číst, měnit, ...").
pub fn rights_words(r: u8) -> String {
    let mut v = Vec::new();
    for (bit, w) in [
        (rights::LIST, "vypsat"),
        (rights::READ, "číst"),
        (rights::WRITE, "měnit"),
        (rights::CREATE, "přidávat"),
        (rights::DELETE, "mazat"),
        (rights::EXECUTE, "spouštět"),
        (rights::MANAGE, "spravovat"),
    ] {
        if r & bit != 0 {
            v.push(w);
        }
    }
    if v.is_empty() { String::from("jen projít") } else { v.join(", ") }
}

/// Vlastni radek sdileni (upravitelny).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Share {
    pub who: u64,
    pub level: Level,
    /// Plati i pro obsah slozky (`grant::INHERIT`).
    pub inherit: bool,
    /// Plati pro objekt sam (`grant::THIS`); grant jen pro obsah
    /// (z prikazu `acl`) zustane jen pro obsah.
    pub this: bool,
}

/// Co ceka na potvrzeni.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    /// Vypnout dedeni: zkopirovat zdedene radky?
    Protect,
    /// Ulozit, i kdyz si uzivatel bere pravo Spravovat.
    DropManage,
}

/// Otevreny panel Vlastnosti.
#[derive(Debug, Clone, PartialEq)]
pub struct Props {
    pub path: String,
    /// 0 = Obecne, 1 = Opravneni.
    pub tab: usize,
    pub stat: Result<Stat, abi::Error>,
    pub view: Result<AclView, abi::Error>,
    /// Upravovane vlastni radky (z `view`).
    pub shares: Vec<Share>,
    /// Dedeni z nadrazene slozky vypnuto (`ACL_PROTECTED`).
    pub protected: bool,
    pub dirty: bool,
    pub ask: Option<Ask>,
    /// Vyber noveho radku: index v `candidates` a uroven.
    pub add_who: usize,
    pub add_level: usize,
    /// Vysledek posledniho ulozeni / chyba.
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropsMsg {
    Tab(usize),
    Close,
    Level(usize, usize),
    Inherit(usize, bool),
    Remove(usize),
    AddWho(usize),
    AddLevel(usize),
    Add,
    /// Prepinac "Dedit z nadrazene slozky".
    Inheritance(bool),
    /// Odpoved na `Ask::Protect`: `Some(true)` zkopirovat zdedene,
    /// `Some(false)` nekopirovat, `None` zrusit.
    ProtectCopy(Option<bool>),
    Save,
    /// Odpoved na `Ask::DropManage`.
    ConfirmSave(bool),
    Revert,
}

impl Props {
    pub fn load<B: Backend>(b: &mut B, path: &str, tab: usize) -> Self {
        let stat = b.stat(path);
        let view = b.acl_view(path);
        let mut p = Self {
            path: String::from(path),
            tab,
            stat,
            view,
            shares: Vec::new(),
            protected: false,
            dirty: false,
            ask: None,
            add_who: 0,
            add_level: 0,
            note: String::new(),
        };
        p.reset();
        p
    }

    /// Vlastni radky znovu z nacteneho stavu.
    fn reset(&mut self) {
        let acl = self.view.as_ref().ok().and_then(|v| v.perms.acl.clone());
        self.protected = acl.as_ref().is_some_and(|a| a.flags & ACL_PROTECTED != 0);
        self.shares = acl
            .map(|a| {
                a.grants
                    .iter()
                    .map(|g| Share { who: g.who, level: Level::of(g.rights), inherit: g.flags & grant::INHERIT != 0, this: g.flags & grant::THIS != 0 })
                    .collect()
            })
            .unwrap_or_default();
        self.dirty = false;
        self.ask = None;
        self.add_who = 0;
    }

    pub fn is_dir(&self) -> bool {
        self.stat.as_ref().is_ok_and(|s| matches!(s.kind, kind::DIR | kind::DATASET))
    }

    /// Smi volajici menit opravneni?
    pub fn can_manage(&self) -> bool {
        self.view.as_ref().is_ok_and(|v| v.perms.rights & rights::MANAGE != 0)
    }

    pub fn owner(&self) -> Option<u64> {
        self.view.as_ref().ok().map(|v| v.perms.owner)
    }

    /// Komu jde pridat sdileni: ucty, Uzivatele, Vsichni - bez vlastnika
    /// a bez tech, kdo uz vlastni radek maji.
    pub fn candidates(&self, names: &[(u64, String)]) -> Vec<u64> {
        let mut v: Vec<u64> = names.iter().map(|n| n.0).collect();
        v.push(GROUP_USERS);
        v.push(EVERYONE);
        v.retain(|w| Some(*w) != self.owner() && !self.shares.iter().any(|s| s.who == *w));
        v
    }

    /// Popis pro `SET_ACL` (`None` = jen dedit).
    pub fn acl(&self) -> Option<Acl> {
        if self.shares.is_empty() && !self.protected {
            return None;
        }
        let dir = self.is_dir();
        let grants = self
            .shares
            .iter()
            .map(|s| {
                let mut flags = if s.this { grant::THIS } else { 0 };
                if dir && s.inherit {
                    flags |= grant::INHERIT;
                }
                Grant { who: s.who, rights: s.level.rights(), flags: if flags == 0 { grant::THIS } else { flags } }
            })
            .collect();
        Some(Acl { flags: if self.protected { ACL_PROTECTED } else { 0 }, grants })
    }

    /// Prijde ukladanim volajici (`me`, neni vlastnik) o pravo Spravovat?
    /// Pocita jen granty, o kterych vime, ze na nej plati (on, Uzivatele,
    /// Vsichni); skupiny mimo ne panel nezna.
    pub fn loses_manage(&self, me: Option<u64>) -> bool {
        let Some(me) = me else { return false };
        let Ok(v) = &self.view else { return false };
        if v.perms.owner == me || v.perms.rights & rights::MANAGE == 0 {
            return false;
        }
        let mine = |who: u64| who == me || who == GROUP_USERS || who == EVERYONE;
        let inherited = !self.protected && v.inherited.iter().any(|g| mine(g.who) && g.rights & rights::MANAGE != 0);
        let own = self.shares.iter().any(|s| s.this && mine(s.who) && s.level.rights() & rights::MANAGE != 0);
        let had = v.perms.acl.as_ref().is_some_and(|a| a.grants.iter().any(|g| mine(g.who) && g.flags & grant::THIS != 0 && g.rights & rights::MANAGE != 0))
            || (!v.perms.acl.as_ref().is_some_and(|a| a.flags & ACL_PROTECTED != 0) && v.inherited.iter().any(|g| mine(g.who) && g.rights & rights::MANAGE != 0));
        had && !own && !inherited
    }

    /// Zpracuje zpravu; `true` = zavrit panel.
    pub fn update<B: Backend>(&mut self, b: &mut B, names: &[(u64, String)], me: Option<u64>, m: PropsMsg) -> bool {
        let edit = self.can_manage();
        match m {
            PropsMsg::Tab(t) => self.tab = t.min(1),
            PropsMsg::Close => return true,
            _ if !edit => {}
            PropsMsg::Level(i, l) => {
                if let Some(s) = self.shares.get_mut(i)
                    && let Some(level) = Level::ALL.get(l)
                    && s.level != *level
                {
                    s.level = *level;
                    self.dirty = true;
                }
            }
            PropsMsg::Inherit(i, on) => {
                if let Some(s) = self.shares.get_mut(i) {
                    s.inherit = on;
                    self.dirty = true;
                }
            }
            PropsMsg::Remove(i) => {
                if i < self.shares.len() {
                    self.shares.remove(i);
                    self.dirty = true;
                }
            }
            PropsMsg::AddWho(i) => self.add_who = i,
            PropsMsg::AddLevel(i) => self.add_level = i.min(Level::ALL.len() - 1),
            PropsMsg::Add => {
                if let Some(&who) = self.candidates(names).get(self.add_who) {
                    let dir = self.is_dir();
                    self.shares.push(Share { who, level: Level::ALL[self.add_level], inherit: dir, this: true });
                    self.add_who = 0;
                    self.dirty = true;
                }
            }
            PropsMsg::Inheritance(on) => {
                if on && self.protected {
                    self.protected = false;
                    self.dirty = true;
                } else if !on && !self.protected {
                    let any = self.view.as_ref().is_ok_and(|v| !v.inherited.is_empty());
                    if any {
                        self.ask = Some(Ask::Protect);
                    } else {
                        self.protected = true;
                        self.dirty = true;
                    }
                }
            }
            PropsMsg::ProtectCopy(answer) => {
                if self.ask == Some(Ask::Protect) {
                    self.ask = None;
                    if let Some(copy) = answer {
                        if copy {
                            self.copy_inherited();
                        }
                        self.protected = true;
                        self.dirty = true;
                    }
                }
            }
            PropsMsg::Save => {
                if self.dirty {
                    if self.loses_manage(me) {
                        self.ask = Some(Ask::DropManage);
                    } else {
                        self.save(b);
                    }
                }
            }
            PropsMsg::ConfirmSave(yes) => {
                if self.ask == Some(Ask::DropManage) {
                    self.ask = None;
                    if yes {
                        self.save(b);
                    }
                }
            }
            PropsMsg::Revert => {
                self.reset();
                self.note.clear();
            }
        }
        false
    }

    /// Zdedene radky jako vlastni (pred vypnutim dedeni); stejny prijemce
    /// = sjednoceni prav.
    fn copy_inherited(&mut self) {
        let Ok(v) = &self.view else { return };
        let dir = self.is_dir();
        for g in &v.inherited {
            match self.shares.iter_mut().find(|s| s.who == g.who) {
                Some(s) => s.level = Level::of(s.level.rights() | g.rights),
                None => self.shares.push(Share { who: g.who, level: Level::of(g.rights), inherit: dir, this: true }),
            }
        }
    }

    fn save<B: Backend>(&mut self, b: &mut B) {
        match b.set_acl(&self.path, self.acl().as_ref()) {
            Ok(()) => {
                self.view = b.acl_view(&self.path);
                self.reset();
                self.note = String::from("Oprávnění uložena.");
            }
            Err(e) => self.note = format!("Oprávnění nejde uložit: {}.", crate::error_text(e)),
        }
    }
}

/// Jmeno prijemce pro rozhrani.
pub fn who_name(who: u64, names: &[(u64, String)], me: Option<u64>) -> String {
    let base = match who {
        EVERYONE => String::from("Všichni"),
        GROUP_USERS => String::from("Uživatelé počítače"),
        0 => String::from("Systém"),
        _ => match names.iter().find(|n| n.0 == who) {
            Some((_, n)) if !n.is_empty() => n.clone(),
            _ if who > GROUP_USERS => format!("Skupina {who}"),
            _ => format!("Účet {who}"),
        },
    };
    if Some(who) == me { format!("{base} (vy)") } else { base }
}
