//! Mys a schranka v okne GUI: vlozeni textu do radku vstupu a klik do
//! radku vstupu (kurzor na znak pod ukazatelem).

use alloc::string::String;

use super::{App, Focus};

/// Nejdelsi vkladany text (znaky); delsi se oriznou, radek je pro prikazy,
/// ne pro data.
pub const PASTE_MAX: usize = 64 * 1024;

impl App {
    /// Vlozi text ze schranky na kurzor radku vstupu. Konce radku zustanou
    /// (Enter spusti vse jako jeden skript, radek je ukaze jako znak
    /// zalomeni); `\r` se zahodi, posledni konec radku taky (vlozeni
    /// nic samo nespusti). Ridici znaky krome tabulatoru (-> mezera) se
    /// zahodi.
    pub fn paste(&mut self, text: &str) {
        let mut s: String = text
            .chars()
            .filter(|&c| c != '\r')
            .map(|c| if c == '\t' { ' ' } else { c })
            .filter(|&c| c == '\n' || !c.is_control())
            .take(PASTE_MAX)
            .collect();
        while s.ends_with('\n') {
            s.pop();
        }
        if s.is_empty() || self.prompt.is_some() {
            return;
        }
        self.focus = Focus::Editor;
        self.menu = None;
        self.scroll = 0;
        self.editor.insert(&s);
    }

    /// Klik do radku vstupu na sloupec `col` (bunky od leveho okraje):
    /// kurzor na znak pod ukazatelem (za koncem textu = na konec).
    pub fn click_input(&mut self, col: usize) {
        if self.prompt.is_some() {
            return;
        }
        let k = col.saturating_sub(self.input_col(0));
        let text = &self.editor.text;
        self.editor.cursor = text.char_indices().nth(k).map_or(text.len(), |(p, _)| p);
        self.focus = Focus::Editor;
        self.menu = None;
    }

    /// Sloupec radku vstupu, na kterem lezi bajt `at` textu editoru (za
    /// vyzvou: ikona, kratka slozka, `" > "`; konec radku = jedna bunka).
    pub fn input_col(&self, at: usize) -> usize {
        let start = self.cwd_lead + self.cwd_short().chars().count() + 3;
        let at = at.min(self.editor.text.len());
        start + self.editor.text.get(..at).map_or(0, |s| s.chars().count())
    }
}
