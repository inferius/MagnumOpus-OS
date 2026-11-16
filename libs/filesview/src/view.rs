//! Rozhrani spravce souboru (knihovna `libs/ui`): lista s navigaci
//! a hledanim, postranni panel mist, tabulka polozek, nahled, ulohy
//! kopirovani a konflikty. Sirka okna: uzke bez postranniho panelu
//! a nahledu, stredni bez nahledu.

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use fileops::job::{Op, Phase, Resolution};
use fileops::path::name;
use fileops::proto::Status;
use fsproto::{avail, kind};
use ui::{Cells, Context, MenuEntry, Node, SizeClass, TextTone, button, checkbox, col, column, image, label, menu_item, menu_separator, progress, row, scroll, spacer, table, text_input};

use crate::app::{FileManager, Mode, Msg};
use crate::backend::Backend;
use crate::format::{self, Avail, DASH};
use crate::listing::SortBy;
use crate::preview::Content;

/// Sirka postranniho panelu a nahledu (logicke px; rozvrzeni, ne vzhled).
const SIDEBAR_W: u32 = 200;
const PREVIEW_W: u32 = 300;
/// Nejmensi sirka tabulky vedle nahledu (sloupce Zmeneno, Velikost, Druh
/// a aspon tolik pro jmeno).
const TABLE_MIN_W: u32 = 600;
/// Sirka pole hledani v liste.
const SEARCH_W: u32 = 220;
/// Id nadpisu skupin v seznamu mist (mista maji index + 1).
const PLACE_HEAD: u64 = 1 << 32;

impl<B: Backend> FileManager<B> {
    pub fn title_text(&self) -> String {
        match self.mode {
            Mode::Trash => String::from("Koš - Soubory"),
            Mode::Folder => {
                let n = if self.nav.cur == self.home { "Domov" } else if self.nav.cur == "/" { "Svazek" } else { name(&self.nav.cur) };
                format!("{n} - Soubory")
            }
        }
    }

    pub fn view_root(&self, cx: &Context) -> Node<Msg> {
        let mut main = Vec::new();
        if cx.class != SizeClass::Compact {
            // Tabulka mist roste; pevnou sirku drzi obalovy sloupec.
            main.push(column([self.sidebar()]).width(SIDEBAR_W));
        }
        main.push(match (&self.props, self.mode) {
            (Some(p), _) => self.props_panel(p),
            (None, Mode::Folder) => self.folder(cx),
            (None, Mode::Trash) => self.trash_view(),
        });
        // Nahled jen kdyz tabulce zbyde misto (jinak by sloupec Jmeno mel
        // nulovou sirku).
        let room = cx.w >= SIDEBAR_W + PREVIEW_W + TABLE_MIN_W;
        if cx.class == SizeClass::Expanded && room && self.mode == Mode::Folder && self.props.is_none() {
            // Nahled (text, obrazek, historie verzi) muze byt vyssi nez okno.
            main.push(scroll(self.preview_panel()).width(PREVIEW_W));
        }
        let mut items = vec![self.toolbar(cx), row(main).grow()];
        items.extend(self.jobs_view());
        if !self.status.is_empty() {
            let mut bar = vec![label(&self.status)];
            if !self.last_trashed.is_empty() {
                bar.push(button("Vrátit").on_click(Msg::UndoTrash));
            }
            items.push(row(bar));
        }
        column(items)
    }

    /// Lista v jednom radku (soubory.html, `.toolbar`): ikony Zpet, Vpred,
    /// Nahoru, drobecky cesty, hledani s lupou a jedna akce (znovu nacist).
    /// Akce nad soubory jsou v nabidce a v radku pod seznamem.
    fn toolbar(&self, cx: &Context) -> Node<Msg> {
        let mut v = vec![
            button("").icon("left").ghost().tooltip("Zpět").on_click(Msg::Back).disabled(!self.nav.can_back()),
            button("").icon("right").ghost().tooltip("Vpřed").on_click(Msg::Forward).disabled(!self.nav.can_forward()),
            button("").icon("up").ghost().tooltip("Nahoru").on_click(Msg::Up).disabled(!self.nav.can_up() || self.mode != Mode::Folder),
        ];
        let mut crumbs: Vec<Node<Msg>> = Vec::new();
        if self.mode == Mode::Folder {
            let all = self.nav.crumbs(&self.home);
            // Uzke okno: jen posledni dva drobky.
            let skip = if cx.class == SizeClass::Compact { all.len().saturating_sub(2) } else { 0 };
            let n = all.len();
            for (k, (label_text, path)) in all.into_iter().enumerate().skip(skip) {
                if k > skip {
                    crumbs.push(ui::icon("right"));
                }
                let mut b = button(&label_text).ghost().on_click(Msg::Go(path.clone()));
                if path == self.home {
                    b = b.icon("home");
                }
                // Posledni drobek je aktualni slozka (`aria-current`).
                if k + 1 == n {
                    b = b.tone(TextTone::Strong);
                }
                crumbs.push(b);
            }
        } else {
            crumbs.push(ui::icon("trash"));
            crumbs.push(label("Koš").tone(TextTone::Strong));
        }
        v.push(row(crumbs).gap(0).grow());
        if self.mode == Mode::Folder {
            // Hledani s lupou; v koncovem slotu pocet nalezenych.
            let mut q = ui::search_input(&self.search).placeholder("Hledat ve složce").on_change(Msg::Search).width(SEARCH_W);
            if !self.search.is_empty() {
                q = q.trailing(ui::fieldopts::Trailing::Text(format::plural(self.listing.len() as u64, "výsledek", "výsledky", "výsledků")));
            }
            v.push(q);
        }
        v.push(button("").icon("refresh").ghost().tooltip("Znovu načíst").on_click(Msg::Refresh));
        row(v)
    }

    /// Mista (soubory.html, `.side`): seznam s ikonami bez ramecku, skupiny
    /// Mista, Media, Dalsi (svazek, kos).
    fn sidebar(&self) -> Node<Msg> {
        let sel = match self.mode {
            Mode::Trash => self.places.iter().position(|p| p.path.is_empty()),
            Mode::Folder => self.places.iter().position(|p| p.path == self.nav.cur),
        };
        // Radky: nadpis skupiny (`None`) nebo misto (index v `places`).
        let mut rows: Vec<(Option<usize>, &'static str)> = Vec::new();
        let groups: [(&str, fn(&crate::app::Place) -> bool); 3] = [
            ("Místa", |p| p.icon != "usb" && p.icon != "disk" && p.icon != "trash"),
            ("Média", |p| p.icon == "usb"),
            ("Další", |p| p.icon == "disk" || p.icon == "trash"),
        ];
        for (head, f) in groups {
            let idx: Vec<usize> = self.places.iter().enumerate().filter(|(_, p)| f(p)).map(|(i, _)| i).collect();
            if idx.is_empty() {
                continue;
            }
            rows.push((None, head));
            rows.extend(idx.into_iter().map(|i| (Some(i), "")));
        }
        let places: Rc<Vec<(String, &'static str)>> = Rc::new(self.places.iter().map(|p| (p.label.clone(), p.icon)).collect());
        let rows = Rc::new(rows);
        let r2 = Rc::clone(&rows);
        // Id radku: misto = index + 1, nadpis = PLACE_HEAD + poradi.
        let id = move |k: usize| match r2.get(k) {
            Some((Some(i), _)) => *i as u64 + 1,
            _ => PLACE_HEAD + k as u64,
        };
        let n = rows.len();
        table(vec![col("", 0)], n, id, move |k| match rows.get(k) {
            Some((None, head)) => Cells::header(head),
            Some((Some(i), _)) => places.get(*i).map_or_else(Cells::default, |(l, icon)| Cells::new([l.clone()]).icon(icon)),
            None => Cells::default(),
        })
        .header(false)
        .plain()
        .selection(sel.map(|i| i as u64 + 1).into_iter().collect())
        .on_selection(|ids: Vec<u64>| match ids.first() {
            Some(&id) if id >= 1 && id < PLACE_HEAD => Msg::Place(id as usize - 1),
            _ => Msg::Place(usize::MAX),
        })
        .key("mista")
    }

    /// Je co vlozit (schranka okna nebo systemova se seznamem cest)?
    fn can_paste(&self, cx: &Context) -> bool {
        self.clip.is_some() || crate::clipfmt::PASTE_FORMATS.iter().any(|f| cx.clipboard_has(f))
    }

    /// Kontextova nabidka radku vypisu (stejne zkratky jako `key_msg`).
    fn row_menu(&self, cx: &Context) -> Vec<MenuEntry<Msg>> {
        let one = self.sel.len() == 1;
        let none = self.sel.is_empty();
        let why = "Nic není vybráno";
        let mut v = vec![
            menu_item("Otevřít", Msg::OpenSelected).shortcut("Enter").icon("external"),
            menu_separator(),
            menu_item("Kopírovat", Msg::Copy).shortcut("Ctrl+C").icon("copy"),
            menu_item("Vyjmout", Msg::Cut).shortcut("Ctrl+X").icon("cut"),
            menu_item("Vložit", Msg::Paste).shortcut("Ctrl+V").icon("paste"),
            menu_separator(),
            menu_item("Přejmenovat", Msg::Rename).shortcut("F2").icon("rename"),
            menu_item("Do koše", Msg::Trash).shortcut("Delete").icon("trash"),
            menu_separator(),
            menu_item("Nová složka", Msg::NewFolder).shortcut("Ctrl+Shift+N").icon("folder-plus"),
            menu_separator(),
            menu_item("Vlastnosti", Msg::Properties).shortcut("Alt+Enter").icon("info"),
        ];
        if none {
            for i in [0, 2, 3, 7] {
                v[i] = v[i].clone().disabled(why);
            }
        }
        if !one {
            v[6] = v[6].clone().disabled("Vyberte jednu položku");
        }
        if !self.can_paste(cx) {
            v[4] = v[4].clone().disabled("Schránka je prázdná");
        }
        if self.sel.len() > 1 {
            v[11] = v[11].clone().disabled("Vyberte jednu položku");
        }
        v
    }

    fn folder(&self, cx: &Context) -> Node<Msg> {
        let mut v = Vec::new();
        if let Some(e) = &self.edit {
            let title = if e.url {
                "Stáhnout z adresy:"
            } else if e.item.is_some() {
                "Nové jméno:"
            } else {
                "Jméno nové složky:"
            };
            let mut field = text_input(&e.value).on_change(Msg::NameValue).on_submit(Msg::NameOk).grow();
            if !e.error.is_empty() {
                field = field.error(&e.error);
            }
            v.push(row([label(title), field, button(if e.url { "Stáhnout" } else { "Uložit" }).on_click(Msg::NameOk), button("Zrušit").on_click(Msg::NameCancel)]));
            if !e.error.is_empty() {
                v.push(label(&e.error));
            }
        }
        if let Some(e) = self.list_error {
            // Odmitnuty vypis neni prazdna slozka (tabulka ani akce nemaji smysl).
            v.clear();
            v.push(label(&denied_text(&self.nav.cur, e)).heading());
            return column(v).grow();
        }
        if self.listing.is_empty() {
            v.push(label(if self.listing.filter.is_empty() { "Složka je prázdná." } else { "Nic neodpovídá hledání." }));
        }
        let (items, order) = (Rc::clone(&self.listing.items), Rc::clone(&self.listing.order));
        let (ids_items, ids_order) = (Rc::clone(&items), Rc::clone(&order));
        let (now, zone) = (self.listing.now, self.listing.zone.clone());
        let t = table(
            vec![col("Jméno", 0).sortable(), col("Změněno", 150).sortable(), col("Velikost", 90).numeric().sortable(), col("Druh", 150).sortable()],
            order.len(),
            move |i| ids_order.get(i).map_or(0, |&k| ids_items[k].e.oid),
            move |i| match order.get(i) {
                Some(&k) => {
                    let r = crate::listing::row_of(&items[k], now, zone.as_ref());
                    Cells::new(r.cells).icon(r.icon)
                }
                None => Cells::default(),
            },
        )
        .selection(self.sel.clone())
        .multi_select(true)
        .sorted(self.listing.sort.col(), self.listing.ascending)
        .on_selection(Msg::Select)
        .on_activate(Msg::Activate)
        .on_sort(Msg::Sort)
        .on_context(Msg::Context)
        .context_menu(self.row_menu(cx))
        .on_drag(Msg::DragStart)
        .on_drop(Msg::Drop);
        v.push(t.grow());
        // Akce nad vyberem v radku pod seznamem (`#rowacts`).
        v.push(self.actions(cx));
        column(v).grow()
    }

    fn actions(&self, cx: &Context) -> Node<Msg> {
        let none = self.sel.is_empty();
        let paste_text = match &self.clip {
            Some((Op::Move, p)) => format!("Vložit ({}, přesun)", p.len()),
            Some((Op::Copy, p)) => format!("Vložit ({})", p.len()),
            None => String::from("Vložit"),
        };
        let mut buttons = vec![
            button("Nová složka").icon("folder-plus").on_click(Msg::NewFolder),
            // Akce nad vyberem jako ikony (popisek v bubline a pristupnosti).
            button("").icon("rename").ghost().tooltip("Přejmenovat").on_click(Msg::Rename).disabled(self.sel.len() != 1),
            button("").icon("trash").ghost().tooltip("Do koše").on_click(Msg::Trash).disabled(none),
            button("").icon("copy").ghost().tooltip("Kopírovat").on_click(Msg::Copy).disabled(none),
            button("").icon("cut").ghost().tooltip("Vyjmout").on_click(Msg::Cut).disabled(none),
            button("").icon("paste").ghost().tooltip(&paste_text).on_click(Msg::Paste).disabled(!self.can_paste(cx)),
        ];
        // Ve slozce media: bezpecne odpojeni.
        if self.current_medium().is_some() {
            buttons.push(spacer());
            buttons.push(button("Odpojit").icon("eject").on_click(Msg::Eject));
        }
        row(buttons)
    }

    fn preview_panel(&self) -> Node<Msg> {
        let mut v = Vec::new();
        let Some(p) = &self.preview else {
            let n = self.sel.len();
            v.push(label(&if n == 0 {
                format!("{}. Vyberte položku.", format::plural(self.listing.len() as u64, "položka", "položky", "položek"))
            } else {
                format!("Vybráno: {}.", format::plural(n as u64, "položka", "položky", "položek"))
            }));
            return column(v);
        };
        // Velka ikona druhu (`.thumb`) a jmeno (`type.title`, zalomene).
        let icon = match &p.stat {
            Ok(st) if matches!(st.kind, kind::DIR | kind::DATASET) => "folder",
            _ => "file",
        };
        v.push(ui::icon_sized(icon, 0));
        v.push(label(name(&p.path)).tone(TextTone::Title));
        match &p.stat {
            Err(e) => v.push(label(&format!("Nejde zjistit: {}", crate::error_text(*e)))),
            Ok(st) => {
                let dir = matches!(st.kind, kind::DIR | kind::DATASET);
                if st.kind == kind::DATASET {
                    v.push(label("Dataset: vlastní historie verzí a koš."));
                }
                let (bit, value) = if dir { (avail::TREE, st.tree_bytes) } else { (avail::SIZE, st.size) };
                v.push(kv(&value_line("Velikost", format::avail(st.known, st.hidden, bit), || format::size(value))));
                if dir {
                    v.push(kv(&value_line("Obsah", format::avail(st.known, st.hidden, avail::TREE), || format::contents(st.tree_files, st.tree_dirs))));
                }
                v.push(kv(&value_line("Změněno", format::avail(st.known, st.hidden, avail::MTIME), || format::date(st.mtime, self.b.now(), self.b.zone()))));
                v.push(kv(&value_line("Smíte", format::avail(st.known, st.hidden, avail::RIGHTS), || rights_text(st.rights))));
            }
        }
        if let Some((url, when)) = &p.origin {
            // Puvod stazeneho souboru: zapsal ho jen system (fetch).
            v.push(kv(&format!("Staženo z: {url}")));
            v.push(kv(&format!("Staženo: {}", format::date(*when, self.b.now(), self.b.zone()))));
        }
        if let Some(ds) = &p.dataset {
            let ds_name = if ds == "/" { "Svazek" } else if *ds == self.home { "Domov" } else { name(ds) };
            v.push(kv(&format!("Úložiště: {ds_name}")));
        }
        match &p.content {
            Some(Content::Text { lines, more }) => {
                let mut t: Vec<Node<Msg>> = lines.iter().take(40).map(|l| label(if l.is_empty() { " " } else { l })).collect();
                if *more || lines.len() > 40 {
                    t.push(label("..."));
                }
                v.push(scroll(column(t)).height(220));
            }
            Some(Content::Bmp(i)) => {
                let alt = format!("Obrázek BMP {} x {}, {} bitů", i.width, i.height, i.bpp);
                if let Some((w, h, px)) = &p.image {
                    v.push(image(*w, *h, Rc::clone(px), &alt).width(PREVIEW_W).height(PREVIEW_W * 3 / 4));
                }
                v.push(label(&alt));
            }
            Some(Content::Binary) => v.push(label("Binární obsah (bez náhledu).")),
            Some(Content::Unreadable(e)) => v.push(label(&format!("Obsah nejde přečíst: {}", crate::error_text(*e)))),
            None => {}
        }
        match &p.versions {
            Ok(vs) if !vs.is_empty() => {
                v.push(label("Historie verzí").tone(TextTone::Strong));
                for (i, ver) in vs.iter().enumerate().rev().take(10) {
                    v.push(row([
                        label(&format!("{} ({})", format::date(ver.time, self.b.now(), self.b.zone()), format::size(ver.size))).grow(),
                        button("Obnovit").on_click(Msg::RestoreVersion(i)),
                        button("Kopie vedle").on_click(Msg::CopyVersion(i)),
                    ]));
                }
            }
            Ok(_) => {}
            Err(e) => v.push(label(&format!("Historie verzí: {DASH} ({})", crate::error_text(*e)))),
        }
        column(v)
    }

    fn trash_view(&self) -> Node<Msg> {
        let rows: Rc<Vec<(u64, [String; 4])>> = Rc::new(
            self.trash
                .iter()
                .map(|t| {
                    let place = fileops::path::join(&t.e.original_dir, "");
                    (t.row_id(), [t.e.name.clone(), place, format::date(t.e.time, self.b.now(), self.b.zone()), format::size(t.e.size)])
                })
                .collect(),
        );
        let none = self.trash_sel.is_empty();
        let mut v = vec![row([
            button("Obnovit").icon("restore").on_click(Msg::TrashRestore).disabled(none),
            button("Smazat trvale").icon("trash").danger().on_click(Msg::TrashPurge).disabled(none),
        ])];
        if self.confirm_purge {
            v.push(row([
                label(&format!("Trvale smazat {}? Nejde vrátit.", format::plural(self.trash_sel.len() as u64, "položku", "položky", "položek"))).grow(),
                button("Smazat trvale").danger().on_click(Msg::TrashPurgeConfirm(true)),
                button("Zrušit").on_click(Msg::TrashPurgeConfirm(false)),
            ]));
        }
        if self.trash.is_empty() {
            v.push(label("Koš je prázdný."));
        }
        let ids = Rc::clone(&rows);
        v.push(
            table(
                vec![col("Jméno", 0), col("Původní místo", 220), col("Smazáno", 150), col("Velikost", 90).numeric()],
                rows.len(),
                move |i| ids.get(i).map_or(0, |r| r.0),
                move |i| rows.get(i).map_or_else(Cells::default, |r| Cells::new(r.1.iter().cloned()).icon("file")),
            )
            .selection(self.trash_sel.clone())
            .multi_select(true)
            .on_selection(Msg::TrashSelect)
            .context_menu(vec![
                menu_item("Obnovit", Msg::TrashRestore).shortcut("Enter").icon("restore"),
                menu_item("Smazat trvale", Msg::TrashPurge).shortcut("Delete").icon("trash"),
            ])
            .grow(),
        );
        column(v).grow()
    }

    fn jobs_view(&self) -> Vec<Node<Msg>> {
        let mut out = Vec::new();
        for j in &self.jobs {
            out.push(job_row(j));
            if let Some(c) = &j.conflict {
                out.push(row([
                    label(&format!("\"{}\" už v cíli je.", name(&c.target))).grow(),
                    button("Přepsat").on_click(Msg::Decide(j.job, Resolution::Overwrite)),
                    button("Přeskočit").on_click(Msg::Decide(j.job, Resolution::Skip)),
                    button("Ponechat obě").on_click(Msg::Decide(j.job, Resolution::Rename)),
                    checkbox("Pro všechny", self.for_all).on_toggle(Msg::ForAll),
                ]));
            }
        }
        out
    }
}

fn job_row(j: &Status) -> Node<Msg> {
    let what = if j.op == Op::Copy { "Kopírování" } else { "Přesun" };
    let dest = name(&j.dest);
    let state = match j.phase {
        Phase::Running => format!("{} %", j.percent),
        Phase::Paused => String::from("pozastaveno"),
        Phase::Conflict => String::from("čeká na rozhodnutí"),
        Phase::Done => String::from("hotovo"),
        Phase::Canceled => String::from("zrušeno"),
    };
    let mut text = format!("{what} do {dest}: {state}");
    if j.skipped > 0 {
        text.push_str(&format!(", přeskočeno {}", j.skipped));
    }
    if j.errors > 0 {
        text.push_str(&format!(", chyby {}", j.errors));
        if let Some((p, e)) = &j.last_error {
            text.push_str(&format!(" ({}: {})", name(p), crate::error_text(*e)));
        }
    }
    let mut v = vec![label(&text).grow()];
    match j.phase {
        Phase::Running | Phase::Paused | Phase::Conflict => {
            v.push(progress(j.percent as u32).width(160));
            v.push(if j.phase == Phase::Paused { button("Pokračovat").on_click(Msg::ResumeJob(j.job)) } else { button("Pozastavit").on_click(Msg::PauseJob(j.job)) });
            v.push(button("Zrušit").on_click(Msg::CancelJob(j.job)));
        }
        Phase::Done | Phase::Canceled => v.push(button("Zavřít").on_click(Msg::DismissJob(j.job))),
    }
    row(v)
}

/// "Velikost: 1,2 MB" nebo "Velikost: - (skryto: ...)".
/// Radek "klic: hodnota" nahledu (`.kv`): klic `ink_muted` pevne siroky,
/// hodnota se zalamuje v sirce panelu (O19). Text bez dvojtecky je cely
/// hodnota.
fn kv(line: &str) -> Node<Msg> {
    match line.split_once(": ") {
        Some((k, val)) => row([label(k).tone(TextTone::Muted).width(KV_W), label(val).grow()]),
        None => label(line),
    }
}

/// Sirka sloupce klicu v nahledu.
const KV_W: u32 = 90;

pub(crate) fn value_line(what: &str, a: Avail, value: impl FnOnce() -> String) -> String {
    match a {
        Avail::Known => format!("{what}: {}", value()),
        _ => format!("{what}: {DASH} ({})", format::reason(a)),
    }
}

fn rights_text(r: u8) -> String {
    use fsproto::perm::rights;
    let mut v = Vec::new();
    for (bit, w) in [(rights::READ, "číst"), (rights::WRITE, "měnit"), (rights::CREATE, "přidávat"), (rights::DELETE, "mazat"), (rights::MANAGE, "spravovat")] {
        if r & bit != 0 {
            v.push(w);
        }
    }
    if v.is_empty() { String::from("jen vidět") } else { v.join(", ") }
}

/// Sloupec razeni pro zahlavi (pro testy).
pub fn sort_col(s: SortBy) -> usize {
    s.col()
}

/// Text misto obsahu slozky, ktera nejde vypsat. Bez prava -> "nemate
/// pristup" (jmeno slozky, ne cela cesta), jinak duvod chyby.
pub fn denied_text(path: &str, e: abi::Error) -> String {
    let n = if path == "/" { "Svazek" } else { name(path) };
    match e {
        abi::Error::AccessDenied => format!("Ke složce {n} nemáte přístup."),
        _ => format!("Složku {n} nejde otevřít: {}.", crate::error_text(e)),
    }
}
