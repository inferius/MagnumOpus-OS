//! Popis rozhrani: strom uzlu, ktery aplikace vraci z `view`.
//!
//! ```ignore
//! column([
//!     label("Poznámky").heading(),
//!     text_input(&s.title).on_change(Msg::Title),
//!     row([checkbox("Důležitá", s.important).on_toggle(Msg::Important),
//!          button("Uložit").on_click(Msg::Save)]),
//! ])
//! ```

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

/// Stabilni klic prvku: podle nej knihovna drzi stav prvku (fokus, kurzor,
/// vyber, posun, stisk) mezi prerozvrzenimi a dava stabilni id uzlum
/// pristupnosti a vrstvam. Bez `Node::key` se klic odvodi z klice rodice,
/// druhu prvku a poradi mezi sourozenci stejneho druhu - staci, dokud se
/// sourozenci stejneho druhu nevkladaji pred sebe. Radky dat (seznam
/// souboru, uloh) maji mit klic z dat (id, cesta).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key(pub u64);

impl Key {
    /// Klic korene okna.
    pub const ROOT: Key = Key(0xcbf2_9ce4_8422_2325);

    /// Odvozeny klic (FNV-1a pres 8 bajtu `v`): deterministicky na vsech
    /// architekturach.
    pub const fn mix(self, v: u64) -> Key {
        let mut h = self.0;
        let mut i = 0;
        while i < 8 {
            h ^= (v >> (i * 8)) & 0xFF;
            h = h.wrapping_mul(0x0100_0000_01b3);
            i += 1;
        }
        Key(h)
    }

    pub fn from_bytes(b: &[u8]) -> Key {
        let mut h = Key::ROOT.0;
        for &x in b {
            h ^= x as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        Key(h)
    }
}

impl From<&str> for Key {
    fn from(s: &str) -> Key {
        Key::from_bytes(s.as_bytes())
    }
}

impl From<&String> for Key {
    fn from(s: &String) -> Key {
        Key::from_bytes(s.as_bytes())
    }
}

impl From<u64> for Key {
    fn from(v: u64) -> Key {
        Key::ROOT.mix(0x6964).mix(v)
    }
}

impl From<u32> for Key {
    fn from(v: u32) -> Key {
        Key::from(v as u64)
    }
}

impl From<usize> for Key {
    fn from(v: usize) -> Key {
        Key::from(v as u64)
    }
}

/// Druh prvku.
pub enum Kind<M> {
    /// `warn` = vyrazne oznaceni (napr. uloha agenta AI).
    Label { text: String, heading: bool, warn: bool },
    Button { text: String, on_click: Option<M> },
    /// `secure` = heslo: kresli se teckami, hodnota se nedava do stromu
    /// pristupnosti ani do trasovani, kompozitor pole zakryje ve snimku.
    TextInput { value: String, placeholder: String, secure: bool, on_change: Option<Box<dyn Fn(String) -> M>>, on_submit: Option<M> },
    Checkbox { text: String, checked: bool, on_toggle: Option<Box<dyn Fn(bool) -> M>> },
    /// `agents[i]` = radek patri agentovi AI (stitek a pruh barvy `ai`).
    List { items: Vec<String>, selected: Option<usize>, on_select: Option<Box<dyn Fn(usize) -> M>>, agents: Vec<bool> },
    Progress { value: u32 },
    Column(Vec<Node<M>>),
    Row(Vec<Node<M>>),
    /// Posouvany obsah: posun dela kompozitor (vlastni vrstva s orezem).
    Scroll(Box<Node<M>>),
    Spacer,
}

/// Uzel stromu: prvek, jmeno pro pristupnost a pruznost v rozvrzeni.
pub struct Node<M> {
    pub kind: Kind<M>,
    /// Bere zbyvajici misto v radku nebo sloupci.
    pub grow: bool,
    /// Pevna vyska (scroll, seznam), 0 = prirozena.
    pub height: u32,
    /// Pevna sirka, 0 = prirozena.
    pub width: u32,
    pub disabled: bool,
    /// Patri agentovi AI: stitek a prubeh v barve `ai` (vyhrazena jen pro
    /// agenta, vzdy s ikonou a slovem "agent").
    pub agent: bool,
    /// Vlastni klic (jinak odvozeny, viz [`Key`]).
    pub key: Option<Key>,
    /// Chyba (neplatna hodnota): stav `error`, text pro ctecku obrazovky.
    pub error: Option<String>,
    /// Fokus jen klikem, ne klavesou Tab (napr. pomocne tlacitko, ktere
    /// ma i klavesovou zkratku).
    pub skip_tab: bool,
}

fn node<M>(kind: Kind<M>) -> Node<M> {
    Node {
        kind,
        grow: false,
        height: 0,
        width: 0,
        disabled: false,
        agent: false,
        key: None,
        error: None,
        skip_tab: false,
    }
}

pub fn label<M>(text: &str) -> Node<M> {
    node(Kind::Label { text: text.into(), heading: false, warn: false })
}

pub fn button<M>(text: &str) -> Node<M> {
    node(Kind::Button { text: text.into(), on_click: None })
}

pub fn text_input<M>(value: &str) -> Node<M> {
    node(Kind::TextInput {
        value: value.into(),
        placeholder: String::new(),
        secure: false,
        on_change: None,
        on_submit: None,
    })
}

/// Pole s heslem (viz `Kind::TextInput::secure`).
pub fn password_input<M>(value: &str) -> Node<M> {
    let mut n = text_input(value);
    if let Kind::TextInput { secure, .. } = &mut n.kind {
        *secure = true;
    }
    n
}

pub fn checkbox<M>(text: &str, checked: bool) -> Node<M> {
    node(Kind::Checkbox { text: text.into(), checked, on_toggle: None })
}

pub fn list<M>(items: impl IntoIterator<Item = String>, selected: Option<usize>) -> Node<M> {
    node(Kind::List {
        items: items.into_iter().collect(),
        selected,
        on_select: None,
        agents: Vec::new(),
    })
}

pub fn progress<M>(value: u32) -> Node<M> {
    node(Kind::Progress { value: value.min(100) })
}

pub fn column<M>(children: impl IntoIterator<Item = Node<M>>) -> Node<M> {
    node(Kind::Column(children.into_iter().collect()))
}

pub fn row<M>(children: impl IntoIterator<Item = Node<M>>) -> Node<M> {
    node(Kind::Row(children.into_iter().collect()))
}

pub fn scroll<M>(child: Node<M>) -> Node<M> {
    node(Kind::Scroll(Box::new(child)))
}

pub fn spacer<M>() -> Node<M> {
    let mut n = node(Kind::Spacer);
    n.grow = true;
    n
}

impl<M> Node<M> {
    /// Stabilni klic (viz [`Key`]); staci jedinecny mezi sourozenci.
    pub fn key(mut self, k: impl Into<Key>) -> Self {
        self.key = Some(k.into());
        self
    }

    /// Chyba s textem (prazdny text = bez chyby).
    pub fn error(mut self, msg: &str) -> Self {
        self.error = (!msg.is_empty()).then(|| msg.into());
        self
    }

    pub fn skip_tab(mut self, s: bool) -> Self {
        self.skip_tab = s;
        self
    }

    pub fn grow(mut self) -> Self {
        self.grow = true;
        self
    }

    pub fn height(mut self, h: u32) -> Self {
        self.height = h;
        self
    }

    pub fn width(mut self, w: u32) -> Self {
        self.width = w;
        self
    }

    pub fn disabled(mut self, d: bool) -> Self {
        self.disabled = d;
        self
    }

    /// Vyrazne oznaceny stitek (vystraha, uloha agenta).
    pub fn warn(mut self) -> Self {
        if let Kind::Label { warn, .. } = &mut self.kind {
            *warn = true;
        }
        self
    }

    /// Patri agentovi AI (stitek, prubeh).
    pub fn agent(mut self) -> Self {
        self.agent = true;
        self
    }

    /// Radky seznamu, ktere patri agentovi AI.
    pub fn agent_rows(mut self, rows: Vec<bool>) -> Self {
        if let Kind::List { agents, .. } = &mut self.kind {
            *agents = rows;
        }
        self
    }

    pub fn heading(mut self) -> Self {
        if let Kind::Label { heading, .. } = &mut self.kind {
            *heading = true;
        }
        self
    }

    pub fn on_click(mut self, m: M) -> Self {
        if let Kind::Button { on_click, .. } = &mut self.kind {
            *on_click = Some(m);
        }
        self
    }

    pub fn placeholder(mut self, p: &str) -> Self {
        if let Kind::TextInput { placeholder, .. } = &mut self.kind {
            *placeholder = p.into();
        }
        self
    }

    pub fn on_change(mut self, f: impl Fn(String) -> M + 'static) -> Self {
        if let Kind::TextInput { on_change, .. } = &mut self.kind {
            *on_change = Some(Box::new(f));
        }
        self
    }

    pub fn on_submit(mut self, m: M) -> Self {
        if let Kind::TextInput { on_submit, .. } = &mut self.kind {
            *on_submit = Some(m);
        }
        self
    }

    pub fn on_toggle(mut self, f: impl Fn(bool) -> M + 'static) -> Self {
        if let Kind::Checkbox { on_toggle, .. } = &mut self.kind {
            *on_toggle = Some(Box::new(f));
        }
        self
    }

    pub fn on_select(mut self, f: impl Fn(usize) -> M + 'static) -> Self {
        if let Kind::List { on_select, .. } = &mut self.kind {
            *on_select = Some(Box::new(f));
        }
        self
    }
}
