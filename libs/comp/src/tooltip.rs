//! Bubliny (tooltip) u prvku listy (`design/bubliny-panely.html`): po
//! najeti 500 ms, hned (0 ms), kdyz predchozi bublina zmizela pred mene nez
//! 300 ms; zmizi pri odjeti nebo kliku. Nad prvkem, zarovnana k okraji
//! obrazovky. Bez ovladacich prvku uvnitr.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::Canvas;
use guiproto::Rect;

use crate::Comp;
use crate::input::SysKey;

/// Zpozdeni bubliny po najeti.
pub const DELAY: u64 = 500_000_000;
/// Do jak dlouho po skryti predchozi bubliny se dalsi ukaze hned.
pub const REOPEN: u64 = 300_000_000;
/// Nejvetsi sirka bubliny.
pub const MAX_W: u32 = 280;

/// Obsah bubliny: titulek, popis (muze byt prazdny), klavesa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TipText {
    pub title: String,
    pub desc: String,
    pub key: Option<&'static str>,
}

/// Prvek s bublinou.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tip {
    Bar(SysKey),
    Clock,
}

impl Comp {
    /// Prvek listy s bublinou pod bodem.
    fn tip_at(&self, x: i32, y: i32) -> Option<Tip> {
        if !self.panel_rect().contains(x, y) || self.flyout.is_some() {
            return None;
        }
        if self.bar_layout().clock.is_some_and(|r| r.contains(x, y)) {
            return Some(Tip::Clock);
        }
        self.sys_key_at(x, y, self.input_now).map(Tip::Bar)
    }

    /// Obdelnik prvku, ke kteremu bublina patri.
    fn tip_anchor(&self, tip: Tip) -> Option<Rect> {
        let bar = self.bar_layout();
        match tip {
            Tip::Clock => bar.clock,
            Tip::Bar(SysKey::BarApp(k)) => bar.apps.get(k).map(|a| a.1),
            Tip::Bar(SysKey::BarLaunch(i)) => bar.launch.iter().find(|(j, _)| *j == i).map(|a| a.1),
            Tip::Bar(SysKey::Launcher) => Some(bar.launcher),
            Tip::Bar(SysKey::BarIcon(0)) => bar.net,
            Tip::Bar(SysKey::BarIcon(1)) => bar.volume,
            Tip::Bar(SysKey::BarIcon(2)) => Some(bar.bell),
            Tip::Bar(SysKey::BarIcon(_)) => Some(bar.look),
            Tip::Bar(SysKey::Tasks) => bar.tasks,
            Tip::Bar(_) => None,
        }
    }

    /// Obsah bubliny (`.tip`): tucny titulek, popis `ink_muted` pod nim
    /// a stitek klavesy za titulkem.
    pub(crate) fn tip_parts(&self, tip: Tip) -> TipText {
        let t = |title: String, desc: String| TipText { title, desc, key: None };
        // "Pevna sit: pripojeno" -> titulek a popis.
        let split = |s: String| match s.split_once(": ") {
            Some((a, b)) => t(String::from(a), b.chars().take(1).flat_map(char::to_uppercase).chain(b.chars().skip(1)).collect()),
            None => t(s, String::new()),
        };
        match tip {
            Tip::Clock => t(self.clock.as_ref().map_or(String::new(), |(_, d)| d.clone()), String::from("Kalendář a světový čas")),
            Tip::Bar(SysKey::BarApp(k)) => {
                let bar = self.bar_layout();
                let (rep, n) = (bar.apps.get(k).map_or(0, |a| a.0), bar.groups.get(k).map_or(1, |g| g.len()));
                let title = self.windows.get(&rep).map_or(String::new(), |w| w.title.clone());
                t(title, if n > 1 { format!("Oken: {n}") } else { String::new() })
            }
            Tip::Bar(SysKey::BarLaunch(i)) => t(self.launchable.get(i).map_or(String::new(), |(_, n)| n.clone()), String::from("Spustit")),
            Tip::Bar(SysKey::Launcher) => TipText { title: String::from("Aplikace"), desc: String::from("Spouštěč a hledání"), key: Some("Super") },
            Tip::Bar(SysKey::BarIcon(0)) => {
                let mut p = split(self.net_line());
                if let (Some(a), Some(crate::render::NetState::Online)) = (&self.status.net_addr, self.status.net) {
                    p.desc = format!("{} · IP {a}", p.desc);
                }
                p
            }
            Tip::Bar(SysKey::BarIcon(1)) if !self.status.capturing.is_empty() => t(String::from("Nahrává zvuk"), self.status.capturing.join(", ")),
            Tip::Bar(SysKey::BarIcon(1)) => t(
                match self.status.volume {
                    Some((_, true)) => String::from("Zvuk ztlumen"),
                    Some((v, false)) => format!("Hlasitost {v} %"),
                    None => String::from("Zvuk"),
                },
                String::new(),
            ),
            Tip::Bar(SysKey::BarIcon(2)) => t(String::from("Oznámení"), String::from(if self.do_not_disturb() { "Nerušit je zapnuté" } else { "" })),
            Tip::Bar(SysKey::BarIcon(_)) => split(self.look_tip()),
            Tip::Bar(SysKey::Tasks) => split(self.tasks_tip()),
            Tip::Bar(_) => t(String::new(), String::new()),
        }
    }

    /// Text bubliny jednim radkem (titulek - popis; testy).
    #[cfg(test)]
    pub(crate) fn tip_text(&self, tip: Tip) -> String {
        let p = self.tip_parts(tip);
        if p.desc.is_empty() { p.title } else { format!("{} - {}", p.title, p.desc) }
    }

    /// Popis zalomeny do sirky `w` (nejvys tri radky).
    fn tip_desc_lines(&self, desc: &str, w: u32) -> Vec<String> {
        let s = self.style.theme.text.small;
        let mut out: Vec<String> = Vec::new();
        let mut line = String::new();
        for word in desc.split(' ') {
            let cand = if line.is_empty() { String::from(word) } else { format!("{line} {word}") };
            if !line.is_empty() && self.style.width(&cand, s) > w {
                out.push(core::mem::replace(&mut line, String::from(word)));
            } else {
                line = cand;
            }
        }
        if !line.is_empty() {
            out.push(line);
        }
        out.truncate(3);
        out
    }

    /// Sirka stitku klavesy v bubline.
    fn tip_key_w(&self, key: &str) -> u32 {
        let t = &self.style.theme;
        self.style.width(key, t.text.caption) + 2 * t.space[2]
    }

    /// Obdelnik bubliny `tip` (8 px nad listou, nad prvkem, u okraje
    /// obrazovky zarovnany).
    fn tip_rect_of(&self, tip: Tip) -> Option<Rect> {
        let a = self.tip_anchor(tip)?;
        let p = self.tip_parts(tip);
        if p.title.is_empty() {
            return None;
        }
        let t = &self.style.theme;
        let (s, strong) = (t.text.small, t.text.body_strong);
        let key_w = p.key.map_or(0, |k| t.space[2] + self.tip_key_w(k));
        let room = MAX_W - 2 * t.space[4];
        let lines = self.tip_desc_lines(&p.desc, room);
        let inner = (self.style.width(&p.title, strong) + key_w).max(lines.iter().map(|l| self.style.width(l, s)).max().unwrap_or(0));
        let w = (inner + 2 * t.space[4]).min(MAX_W).min(self.w);
        let h = strong.line + lines.len() as u32 * s.line + 2 * t.space[2];
        let x = (a.x + (a.w as i32 - w as i32) / 2).clamp(0, (self.w - w) as i32);
        Some(Rect::new(x, self.panel_rect().y - t.space[3] as i32 - h as i32, w, h))
    }

    /// Obdelnik zobrazene bubliny (kresleni, vrstva nad okny).
    pub(crate) fn tip_rect(&self) -> Option<Rect> {
        self.tip_shown.and_then(|t| self.tip_rect_of(t))
    }

    /// Zobrazena bublina (testy, kresleni).
    pub fn tooltip_shown(&self) -> Option<Tip> {
        self.tip_shown
    }

    fn set_tip(&mut self, tip: Option<Tip>, now: u64) {
        if let Some(r) = self.tip_rect() {
            self.damage(r);
        }
        if self.tip_shown.is_some() && tip.is_none() {
            self.tip_hidden_at = now;
        }
        self.tip_shown = tip;
        if let Some(r) = self.tip_rect() {
            self.damage(r);
        }
    }

    /// Pohyb ukazatele: odpocet bubliny, nebo hned (tesne po predchozi).
    pub(crate) fn tip_pointer(&mut self, x: i32, y: i32, now: u64) {
        let at = if self.input.sys_press.is_some() { None } else { self.tip_at(x, y) };
        if at == self.tip_hover.map(|h| h.0) {
            return;
        }
        let recent = self.tip_shown.is_some() || now.saturating_sub(self.tip_hidden_at) <= REOPEN;
        self.tip_hover = at.map(|t| (t, now));
        self.set_tip(None, now);
        if at.is_some() && recent && self.tip_hidden_at != 0 {
            self.set_tip(at, now);
        }
    }

    /// Klik nebo klavesa: bublina hned zmizi (odpocet znovu az po pohybu).
    pub(crate) fn tip_hide(&mut self, now: u64) {
        self.tip_hover = None;
        if self.tip_shown.is_some() {
            self.set_tip(None, now);
        }
    }

    pub(crate) fn tip_due(&self) -> Option<u64> {
        let (_, since) = self.tip_hover?;
        self.tip_shown.is_none().then_some(since + DELAY)
    }

    pub(crate) fn tip_timer(&mut self, now: u64) {
        if let Some((tip, since)) = self.tip_hover
            && self.tip_shown.is_none()
            && now >= since + DELAY
        {
            self.set_tip(Some(tip), now);
        }
    }

    /// Kresli zobrazenou bublinu (systemovy material, bez sipky): tucny
    /// titulek (`body_strong`), za nim stitek klavesy (`.kb`), pod nim popis
    /// `small` barvou `ink_muted`.
    pub(crate) fn draw_tooltip(&mut self, c: &mut Canvas<'_>) {
        let (Some(tip), Some(r)) = (self.tip_shown, self.tip_rect()) else { return };
        let p = self.tip_parts(tip);
        let t = self.style.theme.clone();
        let (s, strong) = (t.text.small, t.text.body_strong);
        c.round_fill_bordered(r, t.radius.control, t.color.surface, t.color.border_strong, 1);
        let (x, room) = (r.x + t.space[4] as i32, r.w.saturating_sub(2 * t.space[4]));
        let mut y = r.y + t.space[2] as i32;
        let face = self.style.face(strong);
        face.draw_ellipsized(c, x, y + face.baseline_in(strong.size, strong.line), &p.title, strong.size, t.color.ink, room);
        if let Some(k) = p.key {
            let cs = t.text.caption;
            let kw = self.tip_key_w(k);
            let kx = x + (self.style.width(&p.title, strong).min(room) + t.space[2]) as i32;
            let kh = cs.line + t.space[1];
            let kr = Rect::new(kx, y + (strong.line as i32 - kh as i32) / 2, kw, kh);
            c.round_fill_bordered(kr, t.radius.xs, t.color.surface, t.color.border_strong, 1);
            c.fill(Rect::new(kr.x + t.radius.xs as i32 / 2, kr.y + kr.h as i32 - 2, kr.w.saturating_sub(t.radius.xs), 1), t.color.border_strong);
            let f = self.style.face(cs);
            f.draw(c, kx + t.space[2] as i32, kr.y + f.baseline_in(cs.size, kh), k, cs.size, t.color.ink);
        }
        y += strong.line as i32;
        let lines = self.tip_desc_lines(&p.desc, MAX_W - 2 * t.space[4]);
        let face = self.style.face(s);
        for l in lines {
            face.draw_ellipsized(c, x, y + face.baseline_in(s.size, s.line), &l, s.size, t.color.ink_muted, room);
            y += s.line as i32;
        }
    }
}
