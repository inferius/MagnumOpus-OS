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
    /// `multiline` = viceradkove pole (`text_area`) s `rows` radky.
    TextInput { value: String, placeholder: String, secure: bool, multiline: bool, rows: u32, on_change: Option<Box<dyn Fn(String) -> M>>, on_submit: Option<M>, key_map: Option<KeyMapFn> },
    Checkbox { text: String, checked: bool, on_toggle: Option<Box<dyn Fn(bool) -> M>> },
    /// `agents[i]` = radek patri agentovi AI (stitek a pruh barvy `ai`).
    List { items: Vec<String>, selected: Option<usize>, on_select: Option<Box<dyn Fn(usize) -> M>>, agents: Vec<bool> },
    Progress { value: u32 },
    /// Tabulka / seznam s virtualizaci (viz [`table`]).
    Table(Box<TableSpec<M>>),
    /// Prepinac zapnuto / vypnuto (nastaveni s okamzitym ucinkem).
    Switch { text: String, on: bool, on_toggle: Option<Box<dyn Fn(bool) -> M>> },
    /// Volba jedne z moznosti (skupinu tvori aplikace, jinak `segmented`).
    Radio { text: String, checked: bool, on_click: Option<M> },
    /// Posuvnik hodnoty `min..=max` s krokem `step`.
    Slider { value: i32, min: i32, max: i32, step: i32, on_change: Option<Box<dyn Fn(i32) -> M>> },
    /// Segmentovy prepinac (jedna volba z nekolika, napr. filtr).
    Segmented { options: Vec<String>, selected: usize, on_select: Option<Box<dyn Fn(usize) -> M>> },
    /// Zalozky; `on_close` = zavirani (krizek, prostredni tlacitko).
    Tabs { labels: Vec<String>, selected: usize, on_select: Option<Box<dyn Fn(usize) -> M>>, on_close: Option<Box<dyn Fn(usize) -> M>> },
    /// Neurcity prubeh (bezi, delka nezname).
    Busy,
    /// Pruhovy graf podilu (oddily disku, obsazeni, kvoty).
    Segments { items: Vec<Segment>, legend: bool },
    /// Obrazek (pixely 0xAARRGGBB, sdilene bez kopie), zmenseny do
    /// prvku se zachovanim pomeru stran; `alt` = popis pro ctecku.
    Image { w: u32, h: u32, px: alloc::rc::Rc<[u32]>, alt: String },
    /// Vyber jedne moznosti z rozbalovaciho seznamu.
    Select { options: Vec<String>, selected: usize, on_select: Option<Box<dyn Fn(usize) -> M>> },

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
    /// Kontextova nabidka (prave tlacitko, Shift+F10, klavesa Menu).
    pub menu: Option<Vec<MenuEntry<M>>>,
    /// Bublina s popisem (po 500 ms nad prvkem).
    pub tooltip: Option<String>,
    /// Skupina fokusu (rada, sloupec, mrizka) s pameti, viz `ui::spatial`.
    pub focus_group: Option<crate::spatial::Axis>,
    /// Vychozi fokus obrazovky.
    pub default_focus: bool,
}

/// Co polozka nabidky udela.
#[derive(Clone)]
pub enum MenuAct<M> {
    /// Zprava aplikaci.
    Msg(M),
    /// Akce z manifestu (`App::action`), napr. polozka z `App::menu`.
    Action(String, Vec<guiproto::action::Arg>),
}

/// Polozka nabidky (kontextove i rozbalovaci).
#[derive(Clone)]
pub struct MenuEntry<M> {
    pub label: String,
    /// Zkratka pro zobrazeni ("Ctrl+C").
    pub shortcut: String,
    /// Zakazano s duvodem (prazdny = povolena); duvod cte ctecka.
    pub disabled: String,
    /// Prepinac: `None` = neni.
    pub checked: Option<bool>,
    /// Oddelovac skupin.
    pub separator: bool,
    pub act: Option<MenuAct<M>>,
}

/// Polozka nabidky, ktera posle zpravu `m`.
pub fn menu_item<M>(label: &str, m: M) -> MenuEntry<M> {
    MenuEntry { label: label.into(), shortcut: String::new(), disabled: String::new(), checked: None, separator: false, act: Some(MenuAct::Msg(m)) }
}

/// Oddelovac skupin v nabidce.
pub fn menu_separator<M>() -> MenuEntry<M> {
    MenuEntry { label: String::new(), shortcut: String::new(), disabled: String::new(), checked: None, separator: true, act: None }
}

impl<M> MenuEntry<M> {
    pub fn shortcut(mut self, s: &str) -> Self {
        self.shortcut = s.into();
        self
    }

    /// Zakazat s duvodem.
    pub fn disabled(mut self, why: &str) -> Self {
        self.disabled = why.into();
        self
    }

    pub fn checked(mut self, c: bool) -> Self {
        self.checked = Some(c);
        self
    }
}

/// Polozky z menu aplikace (`App::menu`, akce z manifestu): podnabidky se
/// rozbali do jedne urovne oddelene oddelovaci (kontextova nabidka se
/// stejnymi polozkami jako menu okna).
pub fn menu_from<M>(items: &[guiproto::menu::MenuItem]) -> Vec<MenuEntry<M>> {
    let mut out: Vec<MenuEntry<M>> = Vec::new();
    for it in items {
        if it.children.is_empty() {
            if it.action.is_empty() {
                continue;
            }
            out.push(MenuEntry {
                label: it.label.clone(),
                shortcut: it.shortcut.clone(),
                disabled: it.disabled.clone(),
                checked: it.checked,
                separator: false,
                act: Some(MenuAct::Action(it.action.clone(), it.args.clone())),
            });
        } else {
            let sub = menu_from(&it.children);
            if sub.is_empty() {
                continue;
            }
            if !out.is_empty() && !out.last().is_some_and(|e| e.separator) {
                out.push(menu_separator());
            }
            out.extend(sub);
        }
    }
    while out.last().is_some_and(|e| e.separator) {
        out.pop();
    }
    out
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
        menu: None,
        tooltip: None,
        focus_group: None,
        default_focus: false,
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
        multiline: false,
        rows: 1,
        on_change: None,
        key_map: None,
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

/// Viceradkove pole (zalamuje podle slov, posouva se kolecko i kurzorem).
/// Enter vlozi radek, Ctrl+Enter odesle (`on_submit`).
pub fn text_area<M>(value: &str) -> Node<M> {
    let mut n = text_input(value);
    if let Kind::TextInput { multiline, rows, .. } = &mut n.kind {
        *multiline = true;
        *rows = 4;
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

/// Prepinac zapnuto / vypnuto.
pub fn switch<M>(text: &str, on: bool) -> Node<M> {
    node(Kind::Switch { text: text.into(), on, on_toggle: None })
}

/// Volba (kruh); zprava pri vyberu `on_click`.
pub fn radio<M>(text: &str, checked: bool) -> Node<M> {
    node(Kind::Radio { text: text.into(), checked, on_click: None })
}

/// Posuvnik hodnoty v rozsahu `min..=max` (krok 1, zmena `.step`).
pub fn slider<M>(value: i32, min: i32, max: i32) -> Node<M> {
    let max = max.max(min);
    node(Kind::Slider { value: value.clamp(min, max), min, max, step: 1, on_change: None })
}

/// Segmentovy prepinac.
pub fn segmented<M>(options: impl IntoIterator<Item = String>, selected: usize) -> Node<M> {
    node(Kind::Segmented { options: options.into_iter().collect(), selected, on_select: None })
}

/// Zalozky.
pub fn tabs<M>(labels: impl IntoIterator<Item = String>, selected: usize) -> Node<M> {
    node(Kind::Tabs { labels: labels.into_iter().collect(), selected, on_select: None, on_close: None })
}

/// Ton useku pruhoveho grafu (barvy jen z tokenu, dvojice s overenym
/// kontrastem textu).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Hlavni (napr. novy oddil systemu): `accent` / `on_accent`.
    Accent,
    /// Bezny obsah: `surface_3` / `ink`.
    Neutral,
    /// Jiny obsah (jiny system): `accent_soft` / `accent_ink`.
    Other,
    /// Mene dulezite (EFI, obnova): `surface_2` / `ink_muted`.
    Muted,
    /// Volne misto: srafovane `surface_3` a `surface`, text `ink_muted`.
    Free,
}

/// Usek pruhoveho grafu: vaha (napr. bajty), popisek (cely jde do stromu
/// pristupnosti, v pruhu jen kdyz se vejde) a ton.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub weight: u64,
    pub label: String,
    pub tone: Tone,
}

impl Segment {
    pub fn new(weight: u64, label: &str, tone: Tone) -> Self {
        Self { weight, label: label.into(), tone }
    }
}

/// Pruhovy graf podilu (`.dbar` v design/instalace.html): vodorovny pruh
/// rozdeleny na useky podle vah, volitelne legenda (`.legend(true)`).
pub fn segments<M>(items: Vec<Segment>) -> Node<M> {
    node(Kind::Segments { items, legend: false })
}

/// Obrazek `w` x `h` (napr. nahled souboru). Velikost prvku je velikost
/// obrazku, mensi prostor ho zmensi se zachovanim pomeru stran.
pub fn image<M>(w: u32, h: u32, px: alloc::rc::Rc<[u32]>, alt: &str) -> Node<M> {
    let ok = px.len() as u64 >= w as u64 * h as u64;
    let (w, h) = if ok { (w, h) } else { (0, 0) };
    node(Kind::Image { w, h, px, alt: alt.into() })
}

/// Rozbalovaci vyber jedne z `options` (zprava `on_pick`).
pub fn select<M>(options: impl IntoIterator<Item = String>, selected: usize) -> Node<M> {
    node(Kind::Select { options: options.into_iter().collect(), selected, on_select: None })
}

/// Neurcity prubeh (animace po snimcich).
pub fn busy<M>() -> Node<M> {
    node(Kind::Busy)
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

    /// Skupina fokusu: sipky podel osy zustanou v ni, pri navratu do ni
    /// dostane fokus naposledy vybrany prvek (rada dlazdic na televizi).
    pub fn focus_group(mut self, a: crate::spatial::Axis) -> Self {
        self.focus_group = Some(a);
        self
    }

    /// Vychozi fokus obrazovky (pri startu a kdyz prvek s fokusem zmizi).
    pub fn default_focus(mut self) -> Self {
        self.default_focus = true;
        self
    }

    /// Kontextova nabidka prvku (prave tlacitko, Shift+F10, klavesa Menu).
    pub fn context_menu(mut self, items: Vec<MenuEntry<M>>) -> Self {
        self.menu = Some(items);
        self
    }

    /// Bublina s popisem (po 500 ms nad prvkem). Tlacitku bez textu (jen
    /// ikona) da i jmeno pro ctecku obrazovky.
    pub fn tooltip(mut self, t: &str) -> Self {
        self.tooltip = (!t.is_empty()).then(|| t.into());
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
        match &mut self.kind {
            Kind::Button { on_click, .. } | Kind::Radio { on_click, .. } => *on_click = Some(m),
            _ => {}
        }
        self
    }

    /// Legenda pod pruhovym grafem.
    pub fn legend(mut self, l: bool) -> Self {
        if let Kind::Segments { legend, .. } = &mut self.kind {
            *legend = l;
        }
        self
    }

    /// Krok posuvniku.
    pub fn step(mut self, s: i32) -> Self {
        if let Kind::Slider { step, .. } = &mut self.kind {
            *step = s.max(1);
        }
        self
    }

    /// Nova hodnota posuvniku.
    pub fn on_value(mut self, f: impl Fn(i32) -> M + 'static) -> Self {
        if let Kind::Slider { on_change, .. } = &mut self.kind {
            *on_change = Some(Box::new(f));
        }
        self
    }

    /// Vybrana volba segmentu nebo zalozka.
    pub fn on_pick(mut self, f: impl Fn(usize) -> M + 'static) -> Self {
        match &mut self.kind {
            Kind::Segmented { on_select, .. } | Kind::Tabs { on_select, .. } | Kind::Select { on_select, .. } => *on_select = Some(Box::new(f)),
            _ => {}
        }
        self
    }

    /// Zavreni zalozky (krizek, prostredni tlacitko).
    pub fn on_close(mut self, f: impl Fn(usize) -> M + 'static) -> Self {
        if let Kind::Tabs { on_close, .. } = &mut self.kind {
            *on_close = Some(Box::new(f));
        }
        self
    }

    /// Pocet viditelnych radku viceradkoveho pole.
    pub fn rows(mut self, n: u32) -> Self {
        if let Kind::TextInput { rows, .. } = &mut self.kind {
            *rows = n.max(1);
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

    /// Vlastni preklad klaves na znaky (napr. zkouska rozlozeni vybraneho
    /// v instalatoru, ne rozlozeni systemu): pri stisku pole vlozi
    /// `f(kod, modifikatory)` misto znaku od systemu; `None` = nic.
    /// Editacni klavesy a zkratky s Ctrl zustanou.
    pub fn key_map(mut self, f: impl Fn(u16, u16) -> Option<char> + 'static) -> Self {
        if let Kind::TextInput { key_map, .. } = &mut self.kind {
            *key_map = Some(Box::new(f));
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
        match &mut self.kind {
            Kind::Checkbox { on_toggle, .. } | Kind::Switch { on_toggle, .. } => *on_toggle = Some(Box::new(f)),
            _ => {}
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

/// Sloupec tabulky.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Col {
    pub title: String,
    /// Sirka v px; 0 = zbytek mista (vic takovych si ho rozdeli).
    pub width: u32,
    /// Cislo: zarovnani vpravo.
    pub numeric: bool,
    /// Klik na zahlavi radi (zprava `on_sort`).
    pub sortable: bool,
}

/// Sloupec `title` sirky `width` (0 = zbytek mista).
pub fn col(title: &str, width: u32) -> Col {
    Col { title: title.into(), width, numeric: false, sortable: false }
}

impl Col {
    pub fn numeric(mut self) -> Self {
        self.numeric = true;
        self
    }

    pub fn sortable(mut self) -> Self {
        self.sortable = true;
        self
    }
}

/// Obsah jednoho radku tabulky (vytvari se jen pro viditelne radky).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cells {
    pub cells: Vec<String>,
    /// Ikona pred prvni bunkou (jmeno z `gfx::icon`, napr. "folder").
    pub icon: Option<&'static str>,
    /// Nadpis sekce (`.sec` v navrhu): nejde vybrat, klavesy ho preskoci.
    pub header: bool,
}

impl Cells {
    pub fn new(cells: impl IntoIterator<Item = String>) -> Self {
        Self { cells: cells.into_iter().collect(), icon: None, header: false }
    }

    /// Nadpis sekce (Bezi, Aplikace, System) v seznamu.
    pub fn header(text: &str) -> Self {
        Self { cells: alloc::vec![String::from(text)], icon: None, header: true }
    }

    pub fn icon(mut self, name: &'static str) -> Self {
        self.icon = Some(name);
        self
    }
}

/// Preklad klavesy (kod, modifikatory `inputproto::mods`) na znak.
pub type KeyMapFn = Box<dyn Fn(u16, u16) -> Option<char>>;

/// Zprava pri pusteni tazenych radku (cil, id radku).
pub type DropFn<M> = Box<dyn Fn(Option<u64>, Vec<u64>) -> M>;

/// Popis tabulky: data se ctou line pres `id` a `row` jen pro viditelne
/// radky, takze tisice polozek nestoji nic navic. Vyber drzi aplikace
/// (`selection`, zprava `on_selection` s novym vyberem).
pub struct TableSpec<M> {
    pub columns: Vec<Col>,
    pub count: usize,
    /// Stabilni id radku `i` (napr. cislo souboru, index v datech aplikace):
    /// podle nej se drzi vyber a kurzor pri razeni a zmene dat. Ma byt
    /// levne (vola se i pro rozsah pri Shift+klik).
    pub id: Box<dyn Fn(usize) -> u64>,
    /// Obsah radku `i`.
    pub row: Box<dyn Fn(usize) -> Cells>,
    pub selection: Vec<u64>,
    /// Vyber vic radku (Ctrl, Shift, Ctrl+A); jinak nejvys jeden.
    pub multi: bool,
    /// Zobrazit zahlavi sloupcu.
    pub header: bool,
    /// Razeni: sloupec a vzestupne.
    pub sort: Option<(usize, bool)>,
    pub on_selection: Option<Box<dyn Fn(Vec<u64>) -> M>>,
    /// Dvojklik nebo Enter na radku.
    pub on_activate: Option<Box<dyn Fn(u64) -> M>>,
    /// Klik na zahlavi: sloupec a vzestupne.
    pub on_sort: Option<Box<dyn Fn(usize, bool) -> M>>,
    /// Zacatek tazeni vybranych radku (id); cil a pusteni resi aplikace.
    pub on_drag: Option<Box<dyn Fn(Vec<u64>) -> M>>,
    /// Prave tlacitko na radku (kontextova nabidka).
    pub on_context: Option<Box<dyn Fn(u64) -> M>>,
    /// Pusteni tazenych radku (id) na radek `Some(id)` nebo na prazdne
    /// misto tabulky `None`; tah z teze nebo jine tabulky v okne.
    pub on_drop: Option<DropFn<M>>,
}

/// Tabulka s `count` radky. Jednosloupcovy seznam bez zahlavi (vyber
/// disku, rozlozeni klavesnice): `table(vec![col("", 0)], n, id, row)
/// .header(false)`.
///
/// ```ignore
/// let files = Rc::clone(&self.files);
/// table(vec![col("Název", 0).sortable(), col("Velikost", 100).numeric().sortable()],
///       self.files.len(),
///       { let f = files.clone(); move |i| f[i].inode },
///       move |i| Cells::new([files[i].name.clone(), files[i].size_text()]).icon("file"))
///     .selection(self.selected.clone()).multi_select(true)
///     .on_selection(Msg::Select).on_activate(Msg::Open).on_sort(Msg::Sort)
/// ```
pub fn table<M>(columns: Vec<Col>, count: usize, id: impl Fn(usize) -> u64 + 'static, row: impl Fn(usize) -> Cells + 'static) -> Node<M> {
    let mut n = node(Kind::Table(Box::new(TableSpec {
        columns,
        count,
        id: Box::new(id),
        row: Box::new(row),
        selection: Vec::new(),
        multi: false,
        header: true,
        sort: None,
        on_selection: None,
        on_activate: None,
        on_sort: None,
        on_drag: None,
        on_context: None,
        on_drop: None,
    })));
    n.grow = true;
    n
}

impl<M> Node<M> {
    fn spec(&mut self) -> Option<&mut TableSpec<M>> {
        match &mut self.kind {
            Kind::Table(t) => Some(t),
            _ => None,
        }
    }

    /// Vybrane radky (id).
    pub fn selection(mut self, ids: Vec<u64>) -> Self {
        if let Some(t) = self.spec() {
            t.selection = ids;
        }
        self
    }

    pub fn multi_select(mut self, m: bool) -> Self {
        if let Some(t) = self.spec() {
            t.multi = m;
        }
        self
    }

    pub fn header(mut self, h: bool) -> Self {
        if let Some(t) = self.spec() {
            t.header = h;
        }
        self
    }

    /// Ukazatel razeni v zahlavi.
    pub fn sorted(mut self, col: usize, ascending: bool) -> Self {
        if let Some(t) = self.spec() {
            t.sort = Some((col, ascending));
        }
        self
    }

    pub fn on_selection(mut self, f: impl Fn(Vec<u64>) -> M + 'static) -> Self {
        if let Some(t) = self.spec() {
            t.on_selection = Some(Box::new(f));
        }
        self
    }

    pub fn on_activate(mut self, f: impl Fn(u64) -> M + 'static) -> Self {
        if let Some(t) = self.spec() {
            t.on_activate = Some(Box::new(f));
        }
        self
    }

    pub fn on_sort(mut self, f: impl Fn(usize, bool) -> M + 'static) -> Self {
        if let Some(t) = self.spec() {
            t.on_sort = Some(Box::new(f));
        }
        self
    }

    pub fn on_drag(mut self, f: impl Fn(Vec<u64>) -> M + 'static) -> Self {
        if let Some(t) = self.spec() {
            t.on_drag = Some(Box::new(f));
        }
        self
    }

    pub fn on_context(mut self, f: impl Fn(u64) -> M + 'static) -> Self {
        if let Some(t) = self.spec() {
            t.on_context = Some(Box::new(f));
        }
        self
    }

    /// Cil pusteni tazenych radku (`on_drag` v teze nebo jine tabulce).
    pub fn on_drop(mut self, f: impl Fn(Option<u64>, Vec<u64>) -> M + 'static) -> Self {
        if let Some(t) = self.spec() {
            t.on_drop = Some(Box::new(f));
        }
        self
    }
}
