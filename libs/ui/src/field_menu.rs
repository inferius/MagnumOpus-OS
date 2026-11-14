//! Kontextova nabidka textoveho pole (prave tlacitko, Shift+F10, klavesa
//! Menu): Zpet, Znovu, Vyjmout, Kopirovat, Vlozit, Smazat, Vybrat vse.
//! Nedostupna polozka rika proc (heslo, jen pro cteni, nic vybrano).
//! Vlastni nabidka aplikace (`Node::context_menu`) se pripoji za oddelovac.
//! Prave tlacitko mimo vyber presune kurzor na misto kliku (vyber uvnitr
//! zustane, aby sel kopirovat).

use alloc::string::String;
use alloc::vec::Vec;

use gfx::Rect;

use crate::Ui;
use crate::popup::{PAct, PItem};

/// Zda schranka nabizi text (vlozeni do pole).
fn has_text(formats: &[String]) -> bool {
    formats.is_empty() || formats.iter().any(|f| f.starts_with("text/"))
}

impl<M: Clone> Ui<M> {
    /// Polozky nabidky pole `i`.
    pub(crate) fn field_menu_items(&self, i: usize) -> Vec<PItem<M>> {
        let secure = self.field_kind(i).is_some_and(|k| k.1);
        let editable = self.field_editable(i);
        let ed = self.edits.get(&self.elems[i].key);
        let sel = ed.is_some_and(|e| e.selection().is_some());
        let all = ed.is_some_and(|e| e.selection() == Some((0, e.text().len())) || e.text().is_empty());
        let ro = "pole je jen pro čtení";
        let why = |ok: bool, reason: &str| if ok { String::new() } else { String::from(reason) };
        let item = |label: &str, short: &str, cmd: char, disabled: String| PItem {
            label: label.into(),
            shortcut: short.into(),
            disabled,
            checked: None,
            separator: false,
            act: Some(PAct::Field(cmd)),
            mnemonic: None,
        };
        let sep = || PItem { label: String::new(), shortcut: String::new(), disabled: String::new(), checked: None, separator: true, act: None, mnemonic: None };
        let copy_why = if secure {
            String::from("heslo nejde kopírovat")
        } else {
            why(sel, "nic není vybráno")
        };
        let cut_why = if !editable { String::from(ro) } else { copy_why.clone() };
        let mut out = alloc::vec![
            item("Zpět", "Ctrl+Z", 'z', if !editable { String::from(ro) } else { why(ed.is_some_and(|e| e.can_undo()), "není co vrátit") }),
            item("Znovu", "Ctrl+Y", 'y', if !editable { String::from(ro) } else { why(ed.is_some_and(|e| e.can_redo()), "není co opakovat") }),
            sep(),
            item("Vyjmout", "Ctrl+X", 'x', cut_why),
            item("Kopírovat", "Ctrl+C", 'c', copy_why),
            item("Vložit", "Ctrl+V", 'v', if !editable { String::from(ro) } else { why(has_text(&self.cx.clipboard), "schránka neobsahuje text") }),
            item("Smazat", "", 'd', if !editable { String::from(ro) } else { why(sel, "nic není vybráno") }),
            sep(),
            item("Vybrat vše", "Ctrl+A", 'a', why(!all, "vše už je vybráno")),
        ];
        if let Some(m) = &self.elems[i].menu {
            let extra = Self::items_from(m);
            if !extra.is_empty() {
                out.push(sep());
                out.extend(extra);
            }
        }
        out
    }

    /// Otevre nabidku pole `i`: u bodu `at` (mys, souradnice okna), jinak
    /// u kurzoru (klavesnice).
    pub(crate) fn open_field_menu(&mut self, i: usize, at: Option<(i32, i32)>) {
        self.focus = Some(i);
        let key = self.elems[i].key;
        if let Some((x, y)) = at {
            // Klik mimo vyber: kurzor na misto kliku.
            let (sx, sy) = self.to_space(i, x, y);
            let off = self.field_offset_at(i, sx, sy);
            if let Some(ed) = self.edits.get_mut(&key)
                && ed.selection().is_none_or(|(a, b)| off < a || off > b)
            {
                ed.set_cursor(off, false);
            }
        }
        let anchor = match at {
            Some((x, y)) => Rect::new(x, y - 2, 1, 1),
            None => {
                // Pod kurzorem pole.
                let w = self.window_rect(i);
                let lines = self.field_lines(i);
                let cur = self.edits.get(&key).map_or(0, |e| e.cursor());
                let (li, cx) = self.caret_pos(i, &lines, cur);
                let inner = self.field_inner(i);
                let (sx, sy) = self.tscroll.get(&key).copied().unwrap_or((0, 0));
                let lh = self.look.body().line as i32;
                let dx = inner.x - self.elems[i].rect.x;
                let dy = inner.y - self.elems[i].rect.y;
                let multi = self.field_kind(i).is_some_and(|k| k.0);
                let y = if multi { w.y + dy + (li as i32 + 1) * lh - sy } else { w.bottom() as i32 - 2 };
                Rect::new(w.x + dx + cx - sx + self.field_align(i), y - 1, 1, 1)
            }
        };
        let items = self.field_menu_items(i);
        self.open_popup(key, false, items, anchor, 0, None);
    }
}
