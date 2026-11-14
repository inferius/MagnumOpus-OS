//! Nabidka doplnovani zvenku: okno GUI ji kresli vlastni komponentou
//! (seznam navrhu `ui::complete` ve vrstve popoveru) misto textovych
//! radku. Okno cte [`App::menu`], typ polozek bere z [`kind_info`]
//! a vyber, prijeti a zavreni hlasi metodami nize.

use shlang::complete::CandKind;

use super::App;

/// Typ polozky nabidky: (ikona z `gfx::icon`, popis typu cesky).
pub fn kind_info(kind: u8) -> (&'static str, &'static str) {
    match kind {
        k if k == CandKind::Dir as u8 => ("folder", "složka"),
        k if k == CandKind::File as u8 => ("file", "soubor"),
        k if k == CandKind::Command as u8 => ("terminal", "příkaz"),
        k if k == CandKind::Flag as u8 => ("sliders", "přepínač"),
        k if k == CandKind::Var as u8 => ("tag", "proměnná"),
        _ => ("filter", "sloupec"),
    }
}

/// Zahlavi nabidky podle typu polozek: jeden druh = jeho jmeno, jinak
/// obecne "Návrhy".
pub fn header(kinds: impl IntoIterator<Item = u8>) -> &'static str {
    let mut it = kinds.into_iter();
    let Some(first) = it.next() else { return "Návrhy" };
    let group = |k: u8| match k {
        k if k == CandKind::Dir as u8 || k == CandKind::File as u8 => 0,
        k if k == CandKind::Command as u8 => 1,
        k if k == CandKind::Flag as u8 => 2,
        k if k == CandKind::Var as u8 => 3,
        _ => 4,
    };
    let g = group(first);
    if !it.all(|k| group(k) == g) {
        return "Návrhy";
    }
    ["Cesty", "Příkazy", "Přepínače", "Proměnné", "Sloupce vstupu"][g]
}

impl App {
    /// Vybrana polozka nabidky (sipky v seznamu okna, najeti ctecky).
    pub fn menu_select(&mut self, k: usize) {
        if let Some(m) = self.menu.as_mut()
            && k < m.items.len()
        {
            m.sel = k;
        }
    }

    /// Prijmout polozku `k` (Enter, Tab, klik): doplnit a nabidku zavrit.
    pub fn menu_accept(&mut self, k: usize) {
        let Some(m) = self.menu.take() else { return };
        if let Some((text, ..)) = m.items.get(k) {
            self.apply_completion(m.start, &text.clone());
        }
    }

    /// Zavrit nabidku bez doplneni (Esc, klik mimo).
    pub fn menu_close(&mut self) {
        self.menu = None;
    }

    /// Sloupec (bunka radku vstupu), kde zacina doplnovane slovo; `None`
    /// = nabidka zavrena. Kotva seznamu v okne.
    pub fn menu_col(&self) -> Option<usize> {
        let m = self.menu.as_ref()?;
        let start = m.start.min(self.editor.text.len());
        Some(self.input_col(start))
    }
}
