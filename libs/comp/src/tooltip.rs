//! Bubliny (tooltip) u prvku listy (`design/bubliny-panely.html`): po
//! najeti 500 ms, hned (0 ms), kdyz predchozi bublina zmizela pred mene nez
//! 300 ms; zmizi pri odjeti nebo kliku. Nad prvkem, zarovnana k okraji
//! obrazovky. Bez ovladacich prvku uvnitr.

use alloc::format;
use alloc::string::String;

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
            Tip::Bar(SysKey::BarIcon(_)) => Some(bar.bell),
            Tip::Bar(SysKey::Tasks) => bar.tasks,
            Tip::Bar(_) => None,
        }
    }

    /// Text bubliny.
    pub(crate) fn tip_text(&self, tip: Tip) -> String {
        match tip {
            Tip::Clock => self.clock.as_ref().map_or(String::new(), |(_, d)| d.clone()),
            Tip::Bar(SysKey::BarApp(k)) => {
                let bar = self.bar_layout();
                let (rep, n) = (bar.apps.get(k).map_or(0, |a| a.0), bar.groups.get(k).map_or(1, |g| g.len()));
                let title = self.windows.get(&rep).map_or(String::new(), |w| w.title.clone());
                if n > 1 { format!("{title} (oken: {n})") } else { title }
            }
            Tip::Bar(SysKey::BarLaunch(i)) => self.launchable.get(i).map_or(String::new(), |(_, n)| format!("{n} - spustit")),
            Tip::Bar(SysKey::Launcher) => String::from("Aplikace"),
            Tip::Bar(SysKey::BarIcon(0)) => self.net_line(),
            Tip::Bar(SysKey::BarIcon(1)) if !self.status.capturing.is_empty() => format!("Nahrává zvuk: {}", self.status.capturing.join(", ")),
            Tip::Bar(SysKey::BarIcon(1)) => match self.status.volume {
                Some((_, true)) => String::from("Zvuk: ztlumeno"),
                Some((v, false)) => format!("Zvuk: hlasitost {v} %"),
                None => String::from("Zvuk"),
            },
            Tip::Bar(SysKey::BarIcon(_)) => String::from(if self.do_not_disturb() { "Oznámení: nerušit" } else { "Oznámení" }),
            Tip::Bar(SysKey::Tasks) => self.tasks_tip(),
            Tip::Bar(_) => String::new(),
        }
    }

    /// Obdelnik bubliny `tip` (8 px nad listou, nad prvkem, u okraje
    /// obrazovky zarovnany).
    fn tip_rect_of(&self, tip: Tip) -> Option<Rect> {
        let a = self.tip_anchor(tip)?;
        let text = self.tip_text(tip);
        if text.is_empty() {
            return None;
        }
        let t = &self.style.theme;
        let s = t.text.small;
        let w = (self.style.width(&text, s) + 2 * t.space[4]).min(MAX_W).min(self.w);
        let h = s.line + 2 * t.space[2];
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

    /// Kresli zobrazenou bublinu (systemovy material, bez sipky).
    pub(crate) fn draw_tooltip(&mut self, c: &mut Canvas<'_>) {
        let (Some(tip), Some(r)) = (self.tip_shown, self.tip_rect()) else { return };
        let text = self.tip_text(tip);
        let t = self.style.theme.clone();
        let s = t.text.small;
        c.round_fill_bordered(r, t.radius.control, t.color.surface, t.color.border_strong, 1);
        let face = self.style.face(s);
        let base = r.y + face.baseline_in(s.size, r.h);
        face.draw_ellipsized(c, r.x + t.space[4] as i32, base, &text, s.size, t.color.ink, r.w.saturating_sub(2 * t.space[4]));
    }
}
