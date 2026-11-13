//! Diagnosticky snimek (`docs/cs/gui.md`, Diagnosticky snimek): systemova
//! zkratka Print Screen (zaloha Ctrl+Alt+Shift+S) zapise do logu stav
//! kompozitoru a sluzba ulozi obrazek obrazovky. Obsah oken jinych aplikaci
//! jde jen do mistniho logu; hodnoty textovych poli ani heslo dialogu se
//! nezapisuji.

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use guiproto::Rect;
use guiproto::a11y::{Node, actions, state};
use inputproto::code::{KEY_S, KEY_SYSRQ};
use inputproto::mods;

use crate::Comp;

/// Kolik poslednich kroku trasovani vstupu snimek ukaze.
pub const RECENT: usize = 20;

/// Zkratka snimku (nastaveni `zkratka`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    Off,
    /// Print Screen.
    Print,
    /// Ctrl+Alt+Shift+S (pres VMware a vzdalenou plochu Print Screen casto
    /// neprojde).
    CtrlAltShiftS,
    /// Obe (vychozi).
    Both,
}

/// Nastaveni snimku (od logd, soubor `/system/log/snimek.conf`). Snimek
/// vznika jen na vyzadani; automaticky jen pri zapnute udalosti a nejvys
/// jednou za `interval_s`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    pub shortcut: Shortcut,
    /// Automaticky snimek, kdyz aplikace prestane odpovidat (ztlumene okno).
    pub auto_hung: bool,
    /// Automaticky snimek po padu aplikace (zadost od logd).
    pub auto_crash: bool,
    /// Nejmensi odstup automatickych snimku.
    pub interval_s: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self { shortcut: Shortcut::Both, auto_hung: false, auto_crash: false, interval_s: 60 }
    }
}

impl Config {
    /// Radek `zkratka=obe auto_nereaguje=0 auto_pad=0 interval=60`
    /// (neznamy klic nebo hodnota se ignoruje).
    pub fn parse(text: &str) -> Self {
        let mut c = Self::default();
        for kv in text.split_whitespace() {
            let Some((k, v)) = kv.split_once('=') else { continue };
            match (k, v) {
                ("zkratka", "vypnuto") => c.shortcut = Shortcut::Off,
                ("zkratka", "print") => c.shortcut = Shortcut::Print,
                ("zkratka", "ctrl+alt+shift+s") => c.shortcut = Shortcut::CtrlAltShiftS,
                ("zkratka", "obe") => c.shortcut = Shortcut::Both,
                ("auto_nereaguje", v) => c.auto_hung = v == "1",
                ("auto_pad", v) => c.auto_crash = v == "1",
                ("interval", v) => c.interval_s = v.parse().unwrap_or(c.interval_s).max(1),
                _ => {}
            }
        }
        c
    }

    /// Je klavesa zkratkou snimku?
    pub fn is_shortcut(&self, code: u16, m: u16) -> bool {
        let all = mods::CTRL | mods::ALT | mods::SHIFT;
        let print = code == KEY_SYSRQ;
        let chord = code == KEY_S && m & all == all;
        match self.shortcut {
            Shortcut::Off => false,
            Shortcut::Print => print,
            Shortcut::CtrlAltShiftS => chord,
            Shortcut::Both => print || chord,
        }
    }
}

/// Posledni kroky trasovani (kruhova fronta).
pub(crate) fn remember(q: &mut VecDeque<String>, line: &str) {
    if q.len() >= RECENT {
        q.pop_front();
    }
    q.push_back(String::from(line));
}

fn bits(v: u32, names: &[(u32, &str)]) -> String {
    let s: Vec<&str> = names.iter().filter(|(b, _)| v & b != 0).map(|(_, n)| *n).collect();
    if s.is_empty() { String::from("-") } else { s.join(",") }
}

fn rect(r: Rect) -> String {
    format!("{},{} {}x{}", r.x, r.y, r.w, r.h)
}

const STATES: &[(u32, &str)] = &[
    (state::FOCUSED, "focused"),
    (state::CHECKED, "checked"),
    (state::DISABLED, "disabled"),
    (state::SELECTED, "selected"),
    (state::EXPANDED, "expanded"),
    (state::BUSY, "busy"),
    (state::HOVER, "hover"),
    (state::PRESSED, "pressed"),
    (state::INVALID, "invalid"),
];

const ACTIONS: &[(u32, &str)] = &[
    (actions::ACTIVATE, "activate"),
    (actions::FOCUS, "focus"),
    (actions::SET_VALUE, "set_value"),
    (actions::TOGGLE, "toggle"),
    (actions::SCROLL, "scroll"),
];

/// Plocha prekryvu `a` a `b` v pixelech.
fn overlap(a: Rect, b: Rect) -> u64 {
    let r = a.intersect(&b);
    r.w as u64 * r.h as u64
}

impl Comp {
    /// Strom pristupnosti okna `g` s obdelniky v souradnicich obrazovky
    /// (inspekce i snimek). U kazdeho uzlu i puvodni vrstva.
    pub(crate) fn screen_nodes(&self, g: u32) -> Option<Vec<(Node, u32)>> {
        let w = self.windows.get(&g)?;
        let (c, origin) = (w.client, (w.rect.x, w.rect.y));
        Some(
            w.a11y
                .nodes()
                .map(|n| {
                    let mut n = n.clone();
                    let layer = n.layer;
                    let (lx, ly) = if n.layer == 0 { (0, 0) } else { self.layer_origin(c, n.layer).unwrap_or((0, 0)) };
                    n.bounds = Rect::new(
                        origin.0.saturating_add(lx).saturating_add(n.bounds.x),
                        origin.1.saturating_add(ly).saturating_add(n.bounds.y),
                        n.bounds.w,
                        n.bounds.h,
                    );
                    n.layer = 0;
                    (n, layer)
                })
                .collect(),
        )
    }

    pub(crate) fn screen_tree(&self, g: u32) -> Option<Vec<Node>> {
        Some(self.screen_nodes(g)?.into_iter().map(|(n, _)| n).collect())
    }

    /// Obdelnik vrstvy `id` klienta okna `g` na obrazovce.
    fn layer_screen(&self, g: u32, id: u32) -> Option<Rect> {
        let w = self.windows.get(&g)?;
        let l = self.clients.get(&w.client)?.layers.get(&id)?;
        let (x, y) = self.layer_origin(w.client, id)?;
        Some(Rect::new(w.rect.x + x, w.rect.y + y, l.props.rect.w, l.props.rect.h))
    }

    /// Orez vrstvou `layer` a jejimi predky s `clip` (posuvne oblasti).
    pub(crate) fn clip_rect(&self, g: u32, layer: u32, mut r: Rect) -> Rect {
        let Some(w) = self.windows.get(&g) else { return r };
        let Some(cl) = self.clients.get(&w.client) else { return r };
        let mut id = layer;
        for _ in 0..64 {
            let Some(l) = cl.layers.get(&id) else { break };
            if l.props.clip
                && let Some(s) = self.layer_screen(g, id)
            {
                r = r.intersect(&s);
            }
            if l.parent == 0 {
                break;
            }
            id = l.parent;
        }
        r
    }

    /// Okna nad `g` (zobrazena, ne minimalizovana) s obdelnikem na obrazovce.
    fn above(&self, g: u32) -> Vec<(u32, Rect)> {
        let pos = self.z.iter().position(|&x| x == g).unwrap_or(0);
        self.z[pos + 1..]
            .iter()
            .filter_map(|&o| self.windows.get(&o).filter(|w| w.mapped && !w.minimized).map(|w| (o, w.outer())))
            .collect()
    }

    /// Viditelnost obdelniku `r` (po orezu `vis`) v okne `g`.
    fn visibility(&self, g: u32, r: Rect, vis: Rect, above: &[(u32, Rect)]) -> String {
        if self.windows.get(&g).is_some_and(|w| w.minimized || !w.mapped) {
            return String::from("skryty (okno neni videt)");
        }
        if r.w == 0 || r.h == 0 {
            return String::from("bez obdelniku");
        }
        if vis.w == 0 || vis.h == 0 {
            return String::from("mimo pohled (orez rodicem nebo scrollem)");
        }
        let mut s = if vis == r { String::from("viditelny") } else { format!("orezany na {}", rect(vis)) };
        let area = vis.w as u64 * vis.h as u64;
        for (o, orect) in above {
            let ov = overlap(vis, *orect);
            if ov == area {
                return format!("zakryty ({})", self.window_name(*o));
            }
            if ov > 0 {
                s = format!("{s}, castecne zakryty ({}, {} %)", self.window_name(*o), ov * 100 / area);
            }
        }
        s
    }

    /// Zadost o snimek (zkratka, prikaz, automaticka udalost) od posledniho
    /// volani: duvod.
    pub fn take_snapshot_request(&mut self) -> Option<String> {
        self.snapshot_request.take()
    }

    /// Snimek na prikaz (`snapshot` od initu, prikaz shellu).
    pub fn request_snapshot(&mut self, why: &str) {
        self.snapshot_request = Some(String::from(why));
    }

    /// Automaticky snimek pri udalosti (`nereaguje`, `pad`): jen kdyz je
    /// v nastaveni zapnuty a od posledniho uplynulo `interval_s`.
    pub fn request_auto_snapshot(&mut self, crash: bool, why: &str, now: u64) {
        let on = if crash { self.snapshot_cfg.auto_crash } else { self.snapshot_cfg.auto_hung };
        let gap = self.snapshot_cfg.interval_s as u64 * 1_000_000_000;
        if !on || self.last_auto_snapshot.is_some_and(|t| now < t + gap) {
            return;
        }
        self.last_auto_snapshot = Some(now);
        self.snapshot_request = Some(format!("automaticky: {why}"));
    }

    /// Nastaveni snimku (zprava `nastaveni ...` od logd).
    pub fn set_snapshot_config(&mut self, c: Config) {
        self.snapshot_cfg = c;
    }

    pub fn snapshot_config(&self) -> Config {
        self.snapshot_cfg
    }

    /// Radky stavu pro diagnosticky snimek `id` (jeden blok v logu, kazdy
    /// radek zacina "snimek <id>:").
    pub fn snapshot_lines(&self, id: &str) -> Vec<String> {
        let mut out = Vec::new();
        let p = format!("snimek {id}:");
        out.push(format!("{p} zacatek, obrazovka {}x{}", self.w, self.h));
        out.push(format!("{p} ukazatel {},{}, {}", self.input.px, self.input.py, self.input_summary()));
        out.push(format!("{p} fokus {}", self.focus.map_or(String::from("zadny"), |g| self.window_name(g))));
        if let Some(s) = self.greeter_summary() {
            out.push(format!("{p} {s}"));
        }
        match self.dialogs.active() {
            // Heslo dialogu se nezapisuje.
            Some(d) => out.push(format!(
                "{p} modalni dialog {:?} od {} (ve fronte {}): '{}' volby {:?} vybrana {}",
                d.kind,
                d.app,
                self.dialogs.len(),
                d.text,
                d.options,
                d.sel
            )),
            None => out.push(format!("{p} modalni dialog zadny")),
        }
        // Okna shora dolu (z=0 nahore, v popredi).
        for (i, &g) in self.z.iter().rev().enumerate() {
            let Some(w) = self.windows.get(&g) else { continue };
            let flags = [
                (w.mapped, "zobrazene"),
                (!w.mapped, "nezobrazene"),
                (w.minimized, "minimalizovane"),
                (w.restore.is_some(), "maximalizovane"),
                (self.focus == Some(g), "fokus"),
                (i == 0, "v popredi"),
                (self.clients.get(&w.client).is_some_and(|c| c.unresponsive), "nereaguje"),
            ];
            let st: Vec<&str> = flags.iter().filter(|s| s.0).map(|s| s.1).collect();
            let above = self.above(g);
            let outer = w.outer();
            let area = (outer.w as u64 * outer.h as u64).max(1);
            let cover: Vec<String> = above
                .iter()
                .filter(|(_, r)| overlap(outer, *r) > 0)
                .map(|(o, r)| format!("{} {} %", self.window_name(*o), overlap(outer, *r) * 100 / area))
                .collect();
            let cover = if cover.is_empty() { String::from("nic") } else { cover.join(", ") };
            out.push(format!(
                "{p} okno z={i} {} '{}' obsah {} s ramem {} rezim {:?} stav {} zakryva ho {cover}",
                self.window_name(g),
                w.title,
                rect(w.rect),
                rect(outer),
                w.mode,
                st.join(",")
            ));
            // Posuvne oblasti: vrstvy s orezem a obsahem (deti).
            if let Some(cl) = self.clients.get(&w.client) {
                for (&id, l) in cl.layers.iter().filter(|(_, l)| l.window == w.local && l.props.clip) {
                    let kids = cl.layers.values().filter(|k| k.parent == id);
                    let (cw, ch) = kids.fold((0i32, 0i32), |(a, b), k| (a.max(k.props.rect.x + k.props.rect.w as i32), b.max(k.props.rect.y + k.props.rect.h as i32)));
                    if ch == 0 && cw == 0 {
                        continue;
                    }
                    let (sx, sy) = l.props.scroll;
                    let at = self.layer_screen(g, id).map_or(String::from("?"), rect);
                    out.push(format!(
                        "{p}   posuvna oblast vrstva {id} na {at}: scroll {sy}/{ch}, vyrez {} (vodorovne {sx}/{cw}, vyrez {})",
                        l.props.rect.h, l.props.rect.w
                    ));
                }
            }
            // Hodnota uzlu (text pole) se nezapisuje: muze byt heslo.
            for (n, layer) in self.screen_nodes(g).unwrap_or_default() {
                let vis = self.clip_rect(g, layer, n.bounds.intersect(&w.rect));
                out.push(format!(
                    "{p}   uzel {} rodic {} {} '{}' {} stav {} akce {} {}",
                    n.id,
                    n.parent,
                    n.role.name(),
                    n.name,
                    rect(n.bounds),
                    bits(n.state, STATES),
                    bits(n.actions, ACTIONS),
                    self.visibility(g, n.bounds, vis, &above)
                ));
            }
        }
        out.push(format!("{p} poslednich {} kroku vstupu (compd):", self.recent_input.len()));
        for l in &self.recent_input {
            out.push(format!("{p}   {l}"));
        }
        out.push(format!("{p} konec"));
        out
    }
}
