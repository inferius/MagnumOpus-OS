//! Rozhrani panelu Vlastnosti (model v `props.rs`). Popisky tlacitek jsou
//! jmena v pristupnosti: kazde je v okne jen jednou (radky sdileni nesou
//! jmeno prijemce).

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use fileops::path::{name, parent};
use fsproto::{avail, kind};
use ui::{Node, button, checkbox, column, label, row, scroll, select, spacer, tabs};

use crate::app::{FileManager, Msg};
use crate::backend::Backend;
use crate::format;
use crate::props::{Ask, Level, Props, PropsMsg, rights_words, who_name};
use crate::view::value_line;

fn pm(m: PropsMsg) -> Msg {
    Msg::Props(m)
}

/// Texty urovni pro vyber (s "Vlastni ..." na konci, kdyz radek takovy je).
fn level_options(level: Level) -> Vec<String> {
    let mut v: Vec<String> = Level::ALL.iter().map(|l| l.label()).collect();
    if let Level::Custom(_) = level {
        v.push(level.label());
    }
    v
}

impl<B: Backend> FileManager<B> {
    pub(crate) fn props_panel(&self, p: &Props) -> Node<Msg> {
        let title = if p.path == "/" { String::from("Svazek") } else if p.path == self.home { String::from("Domov") } else { String::from(name(&p.path)) };
        let mut v = vec![
            row([label(&format!("Vlastnosti: {title}")).heading(), spacer(), button("Zavřít vlastnosti").on_click(pm(PropsMsg::Close))]),
            tabs([String::from("Obecné"), String::from("Oprávnění")], p.tab).on_pick(|t| pm(PropsMsg::Tab(t))),
        ];
        v.push(if p.tab == 0 { self.general_tab(p) } else { self.permissions_tab(p) });
        column(v).grow()
    }

    fn general_tab(&self, p: &Props) -> Node<Msg> {
        let mut v = Vec::new();
        match &p.stat {
            Err(e) => v.push(label(&format!("Nejde zjistit: {}", crate::error_text(*e)))),
            Ok(st) => {
                let what = match st.kind {
                    kind::DATASET => "Dataset (vlastní historie verzí a koš)",
                    kind::DIR => "Složka",
                    kind::SYMLINK => "Odkaz",
                    _ => "Soubor",
                };
                v.push(label(&format!("Druh: {what}")));
                if p.path != "/" {
                    v.push(label(&format!("Umístění: {}", parent(&p.path))));
                }
                let dir = matches!(st.kind, kind::DIR | kind::DATASET);
                let (bit, value) = if dir { (avail::TREE, st.tree_bytes) } else { (avail::SIZE, st.size) };
                v.push(label(&value_line("Velikost", format::avail(st.known, st.hidden, bit), || format::size(value))));
                if dir {
                    v.push(label(&value_line("Obsah", format::avail(st.known, st.hidden, avail::TREE), || format::contents(st.tree_files, st.tree_dirs))));
                }
                v.push(label(&value_line("Změněno", format::avail(st.known, st.hidden, avail::MTIME), || format::date(st.mtime, self.b.now(), self.b.zone()))));
                v.push(label(&value_line("Vytvořeno", format::avail(st.known, st.hidden, avail::BTIME), || format::date(st.btime, self.b.now(), self.b.zone()))));
                v.push(label(&value_line("Smíte", format::avail(st.known, st.hidden, avail::RIGHTS), || rights_words(st.rights))));
            }
        }
        column(v)
    }

    fn permissions_tab(&self, p: &Props) -> Node<Msg> {
        let names = self.names.as_deref().unwrap_or(&[]);
        let who = |w: u64| who_name(w, names, self.me);
        let view = match &p.view {
            Ok(v) => v,
            Err(e) => return label(&format!("Oprávnění nejde zjistit: {}.", crate::error_text(*e))),
        };
        let edit = p.can_manage();
        let dir = p.is_dir();
        let mut v = vec![
            label(&format!("Vlastník: {}", who(view.perms.owner))),
            label("Vlastník má vždy všechna práva; změnit vlastníka může jen systém."),
        ];
        if !edit {
            v.push(
                label(&format!(
                    "Jen pro čtení: oprávnění může měnit vlastník nebo kdo má právo Spravovat. Vy smíte: {}.",
                    rights_words(view.perms.rights)
                ))
                .warn(),
            );
        }
        v.push(label("Sdíleno").heading());
        if p.shares.is_empty() {
            v.push(label("Nikdo další nemá u této položky vlastní oprávnění."));
        }
        for (i, s) in p.shares.iter().enumerate() {
            let n = who(s.who);
            let mut r = vec![
                label(&if s.this { n.clone() } else { format!("{n} (jen pro obsah složky)") }).grow(),
                select(level_options(s.level), s.level.index()).on_pick(move |l| pm(PropsMsg::Level(i, l))).disabled(!edit).width(160),
            ];
            if dir {
                r.push(checkbox(&format!("{n}: i pro obsah složky"), s.inherit).on_toggle(move |on| pm(PropsMsg::Inherit(i, on))).disabled(!edit));
            }
            if edit {
                r.push(button(&format!("Odebrat: {n}")).on_click(pm(PropsMsg::Remove(i))));
            }
            v.push(row(r));
        }
        if edit {
            let cands = p.candidates(names);
            if !cands.is_empty() {
                let opts: Vec<String> = cands.iter().map(|&c| who(c)).collect();
                v.push(row([
                    label("Přidat:"),
                    select(opts, p.add_who.min(cands.len() - 1)).on_pick(|i| pm(PropsMsg::AddWho(i))).grow(),
                    select(Level::ALL.iter().map(|l| l.label()), p.add_level).on_pick(|i| pm(PropsMsg::AddLevel(i))).width(160),
                    button("Přidat sdílení").on_click(pm(PropsMsg::Add)),
                ]));
            }
        }
        if p.path != "/" {
            v.push(label("Zděděno").heading());
            v.push(checkbox("Dědit oprávnění z nadřazené složky", !p.protected).on_toggle(|on| pm(PropsMsg::Inheritance(on))).disabled(!edit));
            if p.ask == Some(Ask::Protect) {
                v.push(label("Vypnout dědění? Zděděná oprávnění můžete zkopírovat jako vlastní, jinak přestanou platit."));
                v.push(row([
                    button("Zkopírovat zděděná").on_click(pm(PropsMsg::ProtectCopy(Some(true)))),
                    button("Nekopírovat").on_click(pm(PropsMsg::ProtectCopy(Some(false)))),
                    button("Ponechat dědění").on_click(pm(PropsMsg::ProtectCopy(None))),
                ]));
            }
            if p.protected {
                v.push(label("Dědění je vypnuté: platí jen oprávnění výše."));
            } else if view.inherited.is_empty() {
                v.push(label("Ze složek výše se nic nedědí."));
            } else {
                // Zdedene radky: zde nemenne (zmenit jdou ve slozce puvodu).
                for g in &view.inherited {
                    let from = if g.from.is_empty() { String::from("mimo vaše místa") } else { format!("ze složky {}", g.from) };
                    let what = Level::of(g.rights).label();
                    v.push(label(&format!("{}: {what} (zděděno {from})", who(g.who))).disabled(true));
                }
            }
        }
        if p.ask == Some(Ask::DropManage) {
            v.push(label("Po uložení už oprávnění nebudete moct měnit: přijdete o právo Spravovat.").warn());
            v.push(row([
                button("Přesto uložit").on_click(pm(PropsMsg::ConfirmSave(true))),
                button("Neukládat").on_click(pm(PropsMsg::ConfirmSave(false))),
            ]));
        }
        if edit {
            v.push(row([
                button("Uložit oprávnění").on_click(pm(PropsMsg::Save)).disabled(!p.dirty || p.ask.is_some()),
                button("Zahodit změny").on_click(pm(PropsMsg::Revert)).disabled(!p.dirty),
            ]));
        }
        if !p.note.is_empty() {
            v.push(label(&p.note));
        }
        scroll(column(v)).grow()
    }
}
