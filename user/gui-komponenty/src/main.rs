//! gui-komponenty: galerie komponent knihovny `ui` (milnik 6c).
//!
//! Stranky po skupinach (pole, volby, tabulka, nabidky, grafy) ukazuji
//! vsechny stavy komponent v motivu systemu; dole je vypis udalosti
//! (co komponenta poslala). Slouzi ke kontrole vzhledu a chovani okem,
//! stejne skupiny hlidaji snimkove testy `libs/ui` (`tests/shots`).
//!
//! Startovni zprava (kind USER): kanal okna ke kompozitoru a listek
//! k supervisoru (obnova okna po padu kompozitoru).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use guiproto::window::Mode;
use libos::{Startup, println};
use ui::*;

libos::entry!(main);

#[derive(Clone, Debug)]
enum Msg {
    Page(usize),
    Text(String),
    Pass(String),
    Note(String),
    Check(bool),
    Wifi(bool),
    Theme(u8),
    Volume(i32),
    Filter(usize),
    Lang(usize),
    Rows(Vec<u64>),
    Open(u64),
    Sort(usize, bool),
    Drag(Vec<u64>),
    Ctx(u64),
    Menu(&'static str),
    Button(&'static str),
}

struct Gallery {
    page: usize,
    text: String,
    pass: String,
    note: String,
    check: bool,
    wifi: bool,
    theme: u8,
    volume: i32,
    filter: usize,
    lang: usize,
    rows: Vec<u64>,
    sort: (usize, bool),
    /// Posledni udalosti (nejnovejsi nahore).
    log: Vec<String>,
}

const PAGES: [&str; 5] = ["Pole", "Volby", "Tabulka", "Nabídky", "Grafy"];
const LANGS: [&str; 4] = ["Čeština", "Deutsch", "English", "Slovenčina"];

impl Gallery {
    fn fields(&self) -> Node<Msg> {
        let short = !self.pass.is_empty() && self.pass.chars().count() < 8;
        column([
            row([button("Primární").on_click(Msg::Button("Primární")), button("Nedostupné").disabled(true), button("").tooltip("Nastavení").on_click(Msg::Button("Ikona")), spacer()]),
            text_input(&self.text).placeholder("Název (Ctrl+Z zpět, dvojklik vybere slovo)").on_change(Msg::Text).key("text"),
            password_input(&self.pass).placeholder("Heslo").on_change(Msg::Pass).error(if short { "Heslo je kratší než 8 znaků" } else { "" }).key("heslo"),
            text_area(&self.note).placeholder("Víceřádková poznámka (Enter = nový řádek)").rows(4).on_change(Msg::Note).key("poznamka"),
        ])
    }

    fn choices(&self) -> Node<Msg> {
        column([
            checkbox("Zaškrtávací pole", self.check).on_toggle(Msg::Check),
            switch("Wi-Fi", self.wifi).on_toggle(Msg::Wifi),
            row([radio("Světlý", self.theme == 0).on_click(Msg::Theme(0)), radio("Tmavý", self.theme == 1).on_click(Msg::Theme(1)), radio("Kontrast", self.theme == 2).on_click(Msg::Theme(2)), spacer()]),
            label(&format!("Hlasitost {} %", self.volume)),
            slider(self.volume, 0, 100).step(5).on_value(Msg::Volume),
            segmented(["Vše".into(), "Běží".into(), "Agent".into()], self.filter).on_pick(Msg::Filter),
            select(LANGS.iter().map(|s| String::from(*s)), self.lang).on_pick(Msg::Lang),
        ])
    }

    fn table(&self) -> Node<Msg> {
        let (col_i, asc) = self.sort;
        // 5000 radku: virtualizace (kresli se jen viditelne).
        let order = move |i: usize| -> usize {
            let k = if asc { i } else { 4999 - i };
            if col_i == 1 { (k * 7919) % 5000 } else { k }
        };
        table(
            vec![col("Název", 0).sortable(), col("Velikost", 110).numeric().sortable()],
            5000,
            move |i| order(i) as u64,
            move |i| {
                let k = order(i);
                Cells::new([format!("soubor {k:04}.txt"), format!("{} kB", (k * 37) % 9000)]).icon(if k % 10 == 0 { "folder" } else { "file" })
            },
        )
        .selection(self.rows.clone())
        .multi_select(true)
        .sorted(col_i, asc)
        .on_selection(Msg::Rows)
        .on_activate(Msg::Open)
        .on_sort(Msg::Sort)
        .on_drag(Msg::Drag)
        .on_context(Msg::Ctx)
        .context_menu(vec![menu_item("Otevřít", Msg::Menu("Otevřít")).shortcut("Enter"), menu_item("Kopírovat", Msg::Menu("Kopírovat")).shortcut("Ctrl+C"), menu_separator(), menu_item("Smazat", Msg::Menu("Smazat")).disabled("Jen ukázka")])
        .key("tabulka")
    }

    fn menus(&self) -> Node<Msg> {
        column([
            label("Pravé tlačítko nebo Shift+F10 na tlačítku otevře kontextovou nabídku."),
            button("Nabídka")
                .on_click(Msg::Button("Nabídka"))
                .context_menu(vec![
                    menu_item("Kopírovat", Msg::Menu("Kopírovat")).shortcut("Ctrl+C"),
                    menu_item("Vložit", Msg::Menu("Vložit")).shortcut("Ctrl+V").disabled("Schránka je prázdná"),
                    menu_separator(),
                    menu_item("Zobrazit skryté", Msg::Menu("Zobrazit skryté")).checked(true),
                ])
                .tooltip("Má kontextovou nabídku"),
            select(LANGS.iter().map(|s| String::from(*s)), self.lang).on_pick(Msg::Lang),
        ])
    }

    fn charts(&self) -> Node<Msg> {
        column([
            label("Průběh"),
            progress(self.volume as u32),
            label("Neurčitý průběh"),
            busy(),
            label("Oddíly disku"),
            segments(vec![
                Segment::new(1, "EFI 100 MB", Tone::Muted),
                Segment::new(480, "Windows C: 480 GB", Tone::Other),
                Segment::new(300, "MagnumOpus 300 GB", Tone::Accent),
                Segment::new(220, "Volné 220 GB", Tone::Free),
            ])
            .legend(true),
        ])
    }
}

impl App for Gallery {
    type Msg = Msg;

    fn title(&self) -> String {
        String::from("Komponenty")
    }

    fn size_hint(&self) -> Option<(u32, u32, u16, u16)> {
        Some((760, 600, 900, 900))
    }

    fn view(&self, cx: &Context) -> Node<Msg> {
        let page = match self.page {
            0 => self.fields(),
            1 => self.choices(),
            2 => self.table(),
            3 => self.menus(),
            _ => self.charts(),
        };
        let log: Vec<String> = self.log.iter().take(if cx.class == SizeClass::Compact { 2 } else { 4 }).cloned().collect();
        column([
            tabs(PAGES.iter().map(|s| String::from(*s)), self.page).on_pick(Msg::Page).key("stranky"),
            page.grow(),
            label("Události").heading(),
            column(log.into_iter().map(|l| label(&l))),
        ])
    }

    fn update(&mut self, m: Msg) {
        // Hodnoty poli se do vypisu nepisou (heslo).
        let line = match &m {
            Msg::Text(_) | Msg::Note(_) => String::from("změna textu"),
            Msg::Pass(_) => String::from("změna hesla"),
            other => format!("{other:?}"),
        };
        match m {
            Msg::Page(p) => self.page = p,
            Msg::Text(t) => self.text = t,
            Msg::Pass(t) => self.pass = t,
            Msg::Note(t) => self.note = t,
            Msg::Check(b) => self.check = b,
            Msg::Wifi(b) => self.wifi = b,
            Msg::Theme(k) => self.theme = k,
            Msg::Volume(v) => self.volume = v,
            Msg::Filter(k) => self.filter = k,
            Msg::Lang(k) => self.lang = k,
            Msg::Rows(r) => self.rows = r,
            Msg::Sort(c, a) => self.sort = (c, a),
            Msg::Open(_) | Msg::Drag(_) | Msg::Ctx(_) | Msg::Menu(_) | Msg::Button(_) => {}
        }
        let mut l = line;
        if l.chars().count() > 80 {
            l = l.chars().take(77).collect::<String>() + "...";
        }
        self.log.insert(0, l);
        self.log.truncate(8);
    }
}

fn main(startup: Startup) -> i64 {
    let Some(link) = ui::client::Link::from_handles(&mut startup.handles.into_iter()) else {
        println!("gui-komponenty: chybi kanal okna");
        return 1;
    };
    let app = Gallery {
        page: 0,
        text: String::new(),
        pass: String::new(),
        note: String::new(),
        check: true,
        wifi: false,
        theme: 0,
        volume: 40,
        filter: 0,
        lang: 0,
        rows: Vec::new(),
        sort: (0, true),
        log: Vec::new(),
    };
    match ui::client::run(link, app, Mode::Standard, 760, 600) {
        Ok(()) => 0,
        Err(e) => {
            println!("gui-komponenty: {e}");
            2
        }
    }
}
