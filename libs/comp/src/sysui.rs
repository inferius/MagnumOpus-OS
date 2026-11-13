//! Systemove pohledy kompozitoru nad knihovnou `ui` (paleta prikazu,
//! spoustec): stejne textove pole (vyber, Ctrl+Z, diakritika) a seznam se
//! sekcemi jako v aplikacich. Data dava katalog palety (`palette.rs`),
//! tady se jen sklada pohled, predava vstup a kresli.
//!
//! Robustnost: kompozitor je kriticky. Popisky z manifestu a menu aplikaci
//! se pred predanim pohledu oriznou ([`MAX_LABEL`]) a pocet radku je
//! omezeny katalogem (`palette::ROWS`); zadny `unwrap` na datech zvenku.

use alloc::string::String;
use alloc::vec::Vec;

use gfx::{Canvas, Image, Rect};
use guiproto::window::InputEvent;
use ui::{Cells, Node, Ui, button, col, column, label, menu_item, menu_separator, row, spacer, table, text_input};

use crate::Comp;
use crate::palette::Row;

/// Nejdelsi popisek radku (znaky).
pub const MAX_LABEL: usize = 120;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SysMsg {
    /// Novy text hledani.
    Query(String),
    /// Vybrany radek (index v `palette_rows`).
    Pick(u64),
    /// Pata spoustece: napajeni a ucet.
    Lock,
    Reboot,
    Shutdown,
}

/// Otevreny systemovy pohled.
pub(crate) struct SysUi {
    pub ui: Ui<SysMsg>,
}

/// Radek pohledu: nadpis, popisek, ikona, text vpravo.
#[derive(Clone)]
struct ViewRow {
    header: bool,
    label: String,
    icon: &'static str,
    right: String,
}

fn clip(s: &str) -> String {
    s.chars().take(MAX_LABEL).collect()
}

impl Comp {
    /// Radky pohledu z katalogu palety.
    fn sys_rows(&self) -> Vec<ViewRow> {
        self.palette_rows()
            .into_iter()
            .map(|r| match r {
                Row::Header(t, icon, _) => ViewRow { header: true, label: clip(&t), icon, right: String::new() },
                Row::More(n) => ViewRow { header: true, label: alloc::format!("další... ({n})"), icon: "more", right: String::new() },
                Row::Item(it) => {
                    let (label, icon, right, dis) = self.palette_item_view(&it);
                    let label = match dis {
                        Some(why) => alloc::format!("{} - {}", clip(&label), clip(&why)),
                        None => clip(&label),
                    };
                    ViewRow { header: false, label, icon, right: clip(&right) }
                }
            })
            .collect()
    }

    /// Pohled palety: pole hledani a seznam se sekcemi.
    fn sys_view(&self, h: u32) -> Node<SysMsg> {
        let Some(p) = &self.palette else { return column([]) };
        let rows = self.sys_rows();
        // Vybrana polozka (index mezi spustitelnymi) -> index radku.
        let sel_row = rows.iter().enumerate().filter(|(_, r)| !r.header).nth(p.sel).map(|(i, _)| i as u64);
        let n = rows.len();
        // Tabulka vyplni misto mezi polem a patou (pohled ma okraj `pad`).
        // Stejne tokeny jako `ui::Look` (pad = space[5], gap = space[3]).
        let t = &self.style.theme;
        let (ctl, gap, pad) = (t.size.control_h, t.space[3], t.space[5]);
        let foot = if p.launcher { ctl + gap } else { 0 };
        let table_h = h.saturating_sub(2 * pad + ctl + gap + foot).max(ctl);
        // Pravy sloupec (zkratka, "2 okna"): ve spoustechi uzsi.
        // Z tokenu sirky spoustece: tretina (spoustec), tri petiny (paleta).
        let lw = self.style.theme.size.launcher_w;
        let right_w = if p.launcher { lw / 3 } else { lw * 3 / 5 };
        let data = alloc::rc::Rc::new(rows);
        let d2 = alloc::rc::Rc::clone(&data);
        column([
            text_input(&p.query).placeholder(if p.launcher { "Hledat" } else { "Hledat příkazy, okna a aplikace... (Tab: rozsah, > akce)" }).on_change(SysMsg::Query).key("hledat"),
            table(alloc::vec![col("", 0), col("", right_w).numeric()], n, |i| i as u64, move |i| match d2.get(i) {
                Some(r) if r.header => Cells::header(&r.label),
                Some(r) => Cells::new([r.label.clone(), r.right.clone()]).icon(r.icon),
                None => Cells::default(),
            })
            .header(false)
            .selection(sel_row.into_iter().collect())
            .on_selection(|ids: Vec<u64>| SysMsg::Pick(ids.first().copied().unwrap_or(0)))
            .key("radky")
            .height(table_h),
            self.launcher_footer(p.launcher),
        ])
    }

    /// Pata spoustece (varianta D): ucet a napajeni s nabidkou. V palete
    /// nic (stejne akce jsou v sekci System).
    fn launcher_footer(&self, launcher: bool) -> Node<SysMsg> {
        if !launcher {
            return column([]);
        }
        let session = self.has_active_session();
        let lock = menu_item("Zamknout", SysMsg::Lock);
        let lock = if session { lock } else { lock.disabled("nikdo není přihlášený") };
        row([
            label(if session { "Přihlášený uživatel" } else { "Bez přihlášení" }),
            spacer(),
            button("")
                .icon("power")
                .tooltip("Napájení")
                .context_menu(alloc::vec![lock, menu_item("Odhlásit", SysMsg::Lock).disabled("zatím není"), menu_separator(), menu_item("Restartovat", SysMsg::Reboot), menu_item("Vypnout", SysMsg::Shutdown)])
                .key("napajeni"),
        ])
    }

    /// Znovu sestavi pohled palety (po zmene dotazu, vyberu, oken).
    pub(crate) fn sys_rebuild(&mut self) {
        let Some(r) = self.palette_rect() else {
            self.sysui = None;
            return;
        };
        let pad = self.style.theme.space[3];
        let (w, h) = (r.w.saturating_sub(2 * pad).max(1), r.h.saturating_sub(2 * pad).max(1));
        let mode = self.style.mode();
        let view = self.sys_view(h);
        let s = self.sysui.get_or_insert_with(|| SysUi { ui: Ui::new(ui::Look::ui_fonts(), w, h) });
        if (s.ui.w, s.ui.h) != (w, h) {
            s.ui.resize(w, h);
        }
        if s.ui.look.theme.mode != mode {
            s.ui.look.set_mode(mode);
        }
        s.ui.rebuild(String::from("Paleta příkazů"), view);
    }

    /// Udalost pro pohled palety (souradnice uz v pohledu); zpracuje
    /// zpravy. Vraci, zda pohled udalost vzal.
    pub(crate) fn sys_event(&mut self, ev: InputEvent, mods: u16, time: u64) -> bool {
        if self.sysui.is_none() {
            self.sys_rebuild();
        }
        let Some(s) = self.sysui.as_mut() else { return false };
        s.ui.set_input_class(self.input.class);
        let r = s.ui.event_at(ev, mods, Some(time));
        for m in r.msgs {
            match m {
                SysMsg::Query(q) => {
                    if let Some(mut p) = self.palette.clone() {
                        p.query = q.chars().take(MAX_LABEL).collect();
                        p.sel = 0;
                        self.set_palette(Some(p));
                    }
                }
                SysMsg::Lock => {
                    self.set_palette(None);
                    self.lock_session();
                    return true;
                }
                SysMsg::Reboot | SysMsg::Shutdown => {
                    self.set_palette(None);
                    self.confirm_power(matches!(m, SysMsg::Reboot));
                    return true;
                }
                SysMsg::Pick(row) => {
                    // Radek -> polozka katalogu (nadpis nic).
                    if let Some(Row::Item(it)) = self.palette_rows().get(row as usize).cloned() {
                        self.palette_run(it, time);
                        return true;
                    }
                }
            }
        }
        if self.palette.is_some() {
            self.sys_rebuild();
            if let Some(r) = self.palette_visual() {
                self.damage(r);
            }
        }
        true
    }

    /// Je v pohledu otevrena nabidka (vse jde do ni)?
    pub(crate) fn sys_popup_open(&self) -> bool {
        self.sysui.as_ref().is_some_and(|s| s.ui.popup_open())
    }

    /// Nakresli pohled palety do karty `r` (ramecek a stin kresli volajici).
    pub(crate) fn draw_sysui(&mut self, c: &mut Canvas<'_>, r: Rect) {
        if self.sysui.is_none() {
            self.sys_rebuild();
        }
        let pad = self.style.theme.space[3] as i32;
        let Some(s) = self.sysui.as_mut() else { return };
        let (w, h) = (s.ui.w, s.ui.h);
        let mut px = alloc::vec![0u32; (w as usize) * (h as usize)];
        if let Some(mut pc) = Canvas::new(&mut px, w, h, w) {
            s.ui.draw(&mut pc, 0);
        }
        if let Some(img) = Image::new(&px, w, h, w) {
            let dst = Rect::new(r.x + pad, r.y + pad, w, h);
            let old = c.clip();
            c.set_clip(old.intersect(&dst));
            c.draw_image(&img, Rect::new(0, 0, w, h), dst, 255, true);
            c.set_clip(old);
        }
    }
}
