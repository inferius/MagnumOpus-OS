//! Strom pristupnosti: struktura okna (role, texty, stav) jako soucast
//! protokolu. Kompozitor z nej slouzi ctecce obrazovky, testum a agentovi
//! (klik pres uzel misto souradnic) a dekoracim (vlastni titulek).

use alloc::string::String;
use alloc::collections::{BTreeMap, BTreeSet};

use crate::{ReadExt, Reader, Rect, Result, WriteExt, Writer};

/// Role uzlu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Window,
    Titlebar,
    Button,
    Label,
    TextInput,
    Checkbox,
    List,
    ListItem,
    Group,
    Image,
    Progress,
    Terminal,
    /// Role, kterou tato verze nezna.
    Unknown(u16),
}

impl Role {
    pub const fn to_u16(self) -> u16 {
        match self {
            Role::Window => 1,
            Role::Titlebar => 2,
            Role::Button => 3,
            Role::Label => 4,
            Role::TextInput => 5,
            Role::Checkbox => 6,
            Role::List => 7,
            Role::ListItem => 8,
            Role::Group => 9,
            Role::Image => 10,
            Role::Progress => 11,
            Role::Terminal => 12,
            Role::Unknown(v) => v,
        }
    }

    pub const fn from_u16(v: u16) -> Self {
        match v {
            1 => Role::Window,
            2 => Role::Titlebar,
            3 => Role::Button,
            4 => Role::Label,
            5 => Role::TextInput,
            6 => Role::Checkbox,
            7 => Role::List,
            8 => Role::ListItem,
            9 => Role::Group,
            10 => Role::Image,
            11 => Role::Progress,
            12 => Role::Terminal,
            v => Role::Unknown(v),
        }
    }

    /// Jmeno role (dokumentace, log, dotazy agenta).
    pub const fn name(self) -> &'static str {
        match self {
            Role::Window => "window",
            Role::Titlebar => "titlebar",
            Role::Button => "button",
            Role::Label => "label",
            Role::TextInput => "text-input",
            Role::Checkbox => "checkbox",
            Role::List => "list",
            Role::ListItem => "list-item",
            Role::Group => "group",
            Role::Image => "image",
            Role::Progress => "progress",
            Role::Terminal => "terminal",
            Role::Unknown(_) => "unknown",
        }
    }
}

/// Bity stavu uzlu.
pub mod state {
    pub const FOCUSED: u32 = 1 << 0;
    pub const CHECKED: u32 = 1 << 1;
    pub const DISABLED: u32 = 1 << 2;
    pub const SELECTED: u32 = 1 << 3;
    pub const EXPANDED: u32 = 1 << 4;
    pub const BUSY: u32 = 1 << 5;
    /// Ukazatel je nad prvkem.
    pub const HOVER: u32 = 1 << 6;
    /// Tlacitko mysi stisknute na prvku (jeste nepustene).
    pub const PRESSED: u32 = 1 << 7;
}

/// Bity akci, ktere uzel podporuje (a kody v `A11Y_ACTION`).
pub mod actions {
    /// Aktivovat (klik na tlacitko, vyber polozky).
    pub const ACTIVATE: u32 = 1 << 0;
    pub const FOCUS: u32 = 1 << 1;
    /// Nastavit hodnotu (textove pole).
    pub const SET_VALUE: u32 = 1 << 2;
    pub const TOGGLE: u32 = 1 << 3;
    pub const SCROLL: u32 = 1 << 4;
}

/// Uzel stromu pristupnosti jednoho okna.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// Id v ramci okna (nenulove).
    pub id: u32,
    /// Rodic (0 = koren okna).
    pub parent: u32,
    pub role: Role,
    /// Text pro cloveka ("Ulozit").
    pub name: String,
    /// Hodnota (text pole, procenta).
    pub value: String,
    pub state: u32,
    /// Vrstva, ve ktere uzel lezi (0 = okno samo), a obdelnik v ni.
    pub layer: u32,
    pub bounds: Rect,
    pub actions: u32,
}

impl Node {
    /// Nejmensi delka zakodovaneho uzlu.
    pub const MIN_SIZE: usize = 4 + 4 + 2 + 2 + 2 + 4 + 4 + 16 + 4;

    pub fn new(id: u32, parent: u32, role: Role, name: &str) -> Self {
        Self {
            id,
            parent,
            role,
            name: name.into(),
            value: String::new(),
            state: 0,
            layer: 0,
            bounds: Rect::default(),
            actions: 0,
        }
    }

    pub fn encode(&self, w: &mut Writer) {
        w.u32(self.id)
            .u32(self.parent)
            .u16(self.role.to_u16())
            .str(&self.name)
            .str(&self.value)
            .u32(self.state)
            .u32(self.layer)
            .rect(&self.bounds)
            .u32(self.actions);
    }

    pub fn decode(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            id: r.u32()?,
            parent: r.u32()?,
            role: Role::from_u16(r.u16()?),
            name: r.str()?,
            value: r.str()?,
            state: r.u32()?,
            layer: r.u32()?,
            bounds: r.rect()?,
            actions: r.u32()?,
        })
    }

    /// Velikost zakodovaneho uzlu.
    pub fn encoded_len(&self) -> usize {
        Self::MIN_SIZE + self.name.len().min(u16::MAX as usize) + self.value.len().min(u16::MAX as usize)
    }
}

/// Strom jednoho okna, jak ho drzi kompozitor (a test). Uzly podle id
/// (operace v O(log n) - strom posila neduveryhodna aplikace).
#[derive(Debug, Clone, Default)]
pub struct Tree {
    nodes: BTreeMap<u32, Node>,
}

/// Nejvic uzlu jednoho okna (ochrana pameti kompozitoru).
pub const MAX_NODES: usize = 4096;

impl Tree {
    pub fn new() -> Self {
        Self::default()
    }

    /// Prida nebo nahradi uzly. `false` = prekrocen limit (zbytek zahozen).
    pub fn update(&mut self, nodes: impl IntoIterator<Item = Node>) -> bool {
        for n in nodes {
            if n.id == 0 {
                continue;
            }
            if self.nodes.len() >= MAX_NODES && !self.nodes.contains_key(&n.id) {
                return false;
            }
            self.nodes.insert(n.id, n);
        }
        true
    }

    /// Odebere uzly i s jejich potomky.
    pub fn remove(&mut self, ids: &[u32]) {
        let mut gone: BTreeSet<u32> = ids.iter().copied().filter(|i| self.nodes.contains_key(i)).collect();
        if gone.is_empty() {
            return;
        }
        // Potomci: opakovane pruchody, dokud pribyvaji (hloubka stromu).
        loop {
            let before = gone.len();
            for n in self.nodes.values() {
                if gone.contains(&n.parent) {
                    gone.insert(n.id);
                }
            }
            if gone.len() == before {
                break;
            }
        }
        for id in gone {
            self.nodes.remove(&id);
        }
    }

    pub fn get(&self, id: u32) -> Option<&Node> {
        self.nodes.get(&id)
    }

    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Prvni uzel s roli a jmenem (dotaz testu nebo agenta).
    pub fn find(&self, role: Role, name: &str) -> Option<&Node> {
        self.nodes.values().find(|n| n.role == role && n.name == name)
    }

    pub fn children(&self, id: u32) -> impl Iterator<Item = &Node> {
        self.nodes.values().filter(move |n| n.parent == id)
    }
}
