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

/// Druh prvku.
pub enum Kind<M> {
    /// `warn` = vyrazne oznaceni (napr. uloha agenta AI).
    Label { text: String, heading: bool, warn: bool },
    Button { text: String, on_click: Option<M> },
    TextInput { value: String, placeholder: String, on_change: Option<Box<dyn Fn(String) -> M>>, on_submit: Option<M> },
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
}

fn node<M>(kind: Kind<M>) -> Node<M> {
    Node {
        kind,
        grow: false,
        height: 0,
        width: 0,
        disabled: false,
        agent: false,
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
        on_change: None,
        on_submit: None,
    })
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
