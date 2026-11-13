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
use ui::{Cells, Context, Node, SizeClass, button, checkbox, col, column, label, list, progress, row, scroll, spacer, table, text_input};

use crate::app::{FileManager, Mode, Msg};
use crate::backend::Backend;
use crate::format::{self, Avail, DASH};
use crate::listing::SortBy;
use crate::preview::Content;

/// Sirka postranniho panelu a nahledu (logicke px; rozvrzeni, ne vzhled).
const SIDEBAR_W: u32 = 200;
const PREVIEW_W: u32 = 300;

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
            main.push(self.sidebar().width(SIDEBAR_W));
        }
        main.push(match self.mode {
            Mode::Folder => self.folder(),
            Mode::Trash => self.trash_view(),
        });
        if cx.class == SizeClass::Expanded && self.mode == Mode::Folder {
            main.push(self.preview_panel().width(PREVIEW_W));
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

    fn toolbar(&self, cx: &Context) -> Node<Msg> {
        let mut v = vec![
            button("Zpět").on_click(Msg::Back).disabled(!self.nav.can_back()),
            button("Vpřed").on_click(Msg::Forward).disabled(!self.nav.can_forward()),
            button("Nahoru").on_click(Msg::Up).disabled(!self.nav.can_up() || self.mode != Mode::Folder),
        ];
        if self.mode == Mode::Folder {
            let crumbs = self.nav.crumbs(&self.home);
            // Uzke okno: jen posledni dva drobky.
            let skip = if cx.class == SizeClass::Compact { crumbs.len().saturating_sub(2) } else { 0 };
            for (label_text, path) in crumbs.into_iter().skip(skip) {
                v.push(button(&label_text).on_click(Msg::Go(path)));
            }
        } else {
            v.push(label("Koš").heading());
        }
        v.push(spacer());
        if self.mode == Mode::Folder {
            v.push(text_input(&self.search).placeholder("Hledat ve složce").on_change(Msg::Search).width(200));
        }
        v.push(button("Znovu načíst").on_click(Msg::Refresh));
        row(v)
    }

    fn sidebar(&self) -> Node<Msg> {
        let items: Vec<String> = self.places.iter().map(|p| p.label.clone()).collect();
        let sel = match self.mode {
            Mode::Trash => self.places.iter().position(|p| p.path.is_empty()),
            Mode::Folder => self.places.iter().position(|p| p.path == self.nav.cur),
        };
        column([label("Místa").heading(), list(items, sel).on_select(Msg::Place).grow()])
    }

    fn folder(&self) -> Node<Msg> {
        let mut v = vec![self.actions()];
        if let Some(e) = &self.edit {
            let title = if e.item.is_some() { "Nové jméno:" } else { "Jméno nové složky:" };
            let mut field = text_input(&e.value).on_change(Msg::NameValue).on_submit(Msg::NameOk).grow();
            if !e.error.is_empty() {
                field = field.error(&e.error);
            }
            v.push(row([label(title), field, button("Uložit").on_click(Msg::NameOk), button("Zrušit").on_click(Msg::NameCancel)]));
            if !e.error.is_empty() {
                v.push(label(&e.error));
            }
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
        .on_sort(Msg::Sort);
        v.push(t.grow());
        column(v).grow()
    }

    fn actions(&self) -> Node<Msg> {
        let none = self.sel.is_empty();
        let paste_text = match &self.clip {
            Some((Op::Move, p)) => format!("Vložit ({}, přesun)", p.len()),
            Some((Op::Copy, p)) => format!("Vložit ({})", p.len()),
            None => String::from("Vložit"),
        };
        row([
            button("Nová složka").on_click(Msg::NewFolder),
            button("Přejmenovat").on_click(Msg::Rename).disabled(self.sel.len() != 1),
            button("Do koše").on_click(Msg::Trash).disabled(none),
            button("Kopírovat").on_click(Msg::Copy).disabled(none),
            button("Vyjmout").on_click(Msg::Cut).disabled(none),
            button(&paste_text).on_click(Msg::Paste).disabled(self.clip.is_none()),
        ])
    }

    fn preview_panel(&self) -> Node<Msg> {
        let mut v = vec![label("Náhled").heading()];
        let Some(p) = &self.preview else {
            let n = self.sel.len();
            v.push(label(&if n == 0 {
                format!("{}. Vyberte položku.", format::plural(self.listing.len() as u64, "položka", "položky", "položek"))
            } else {
                format!("Vybráno: {}.", format::plural(n as u64, "položka", "položky", "položek"))
            }));
            return column(v);
        };
        v.push(label(name(&p.path)).heading());
        match &p.stat {
            Err(e) => v.push(label(&format!("Nejde zjistit: {}", crate::error_text(*e)))),
            Ok(st) => {
                let dir = matches!(st.kind, kind::DIR | kind::DATASET);
                if st.kind == kind::DATASET {
                    v.push(label("Dataset: vlastní historie verzí a koš."));
                }
                let (bit, value) = if dir { (avail::TREE, st.tree_bytes) } else { (avail::SIZE, st.size) };
                v.push(label(&value_line("Velikost", format::avail(st.known, st.hidden, bit), || format::size(value))));
                if dir {
                    v.push(label(&value_line("Obsah", format::avail(st.known, st.hidden, avail::TREE), || format::contents(st.tree_files, st.tree_dirs))));
                }
                v.push(label(&value_line("Změněno", format::avail(st.known, st.hidden, avail::MTIME), || format::date(st.mtime, self.b.now(), self.b.zone()))));
                v.push(label(&value_line("Smíte", format::avail(st.known, st.hidden, avail::RIGHTS), || rights_text(st.rights))));
            }
        }
        if let Some(ds) = &p.dataset {
            let ds_name = if ds == "/" { "Svazek" } else if *ds == self.home { "Domov" } else { name(ds) };
            v.push(label(&format!("Úložiště: {ds_name}")));
        }
        match &p.content {
            Some(Content::Text { lines, more }) => {
                let mut t: Vec<Node<Msg>> = lines.iter().take(40).map(|l| label(if l.is_empty() { " " } else { l })).collect();
                if *more || lines.len() > 40 {
                    t.push(label("..."));
                }
                v.push(scroll(column(t)).height(220));
            }
            Some(Content::Bmp(i)) => v.push(label(&format!("Obrázek BMP {} x {}, {} bitů", i.width, i.height, i.bpp))),
            Some(Content::Binary) => v.push(label("Binární obsah (bez náhledu).")),
            Some(Content::Unreadable(e)) => v.push(label(&format!("Obsah nejde přečíst: {}", crate::error_text(*e)))),
            None => {}
        }
        match &p.versions {
            Ok(vs) if !vs.is_empty() => {
                v.push(label("Historie verzí").heading());
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
            button("Obnovit").on_click(Msg::TrashRestore).disabled(none),
            button("Smazat trvale").on_click(Msg::TrashPurge).disabled(none),
        ])];
        if self.confirm_purge {
            v.push(row([
                label(&format!("Trvale smazat {}? Nejde vrátit.", format::plural(self.trash_sel.len() as u64, "položku", "položky", "položek"))).grow(),
                button("Smazat trvale").on_click(Msg::TrashPurgeConfirm(true)),
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
fn value_line(what: &str, a: Avail, value: impl FnOnce() -> String) -> String {
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
