//! Panely nad listou (flyout): klik na ikonu site, hlasitosti, oznameni
//! nebo uloh v liste ukaze kartu se stavem nad ikonou. Druhy klik na tutez
//! ikonu, klik jinam nebo Esc ji zavre. Ikona uloh prednostne prepne na
//! Spravce uloh, kdyz bezi.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use guiproto::Rect;
use gfx::Canvas;

use crate::Comp;
use crate::render::NetState;

/// Ktery panel je otevreny.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flyout {
    Tasks,
    Net,
    Volume,
    Bell,
}

/// Ceske tvary "uloha".
fn tasks_word(n: u32) -> &'static str {
    match n {
        1 => "úloha",
        2..=4 => "úlohy",
        _ => "úloh",
    }
}

impl Comp {
    /// Text popisku ikony uloh (najeti mysi): co cislo v liste znamena.
    pub(crate) fn tasks_tip(&self) -> String {
        let (run, agent) = self.tasks;
        let mut s = format!("Úlohy: {run} běží");
        if agent > 0 {
            s = format!("{s}, z toho agent AI {agent}");
        }
        s
    }

    /// Obdelnik popisku ikony uloh (nad ikonou, at je mys nad ni nebo ne:
    /// i pro poskozeni po odjeti).
    pub(crate) fn tasks_tip_rect(&self) -> Option<Rect> {
        let a = self.bar_layout().tasks?;
        let t = &self.style.theme;
        let s = t.text.small;
        let w = (self.style.width(&self.tasks_tip(), s) + 2 * t.space[3]).min(self.w);
        let h = s.line + 2 * t.space[2];
        let x = (a.x + (a.w as i32 - w as i32) / 2).clamp(0, (self.w - w) as i32);
        Some(Rect::new(x, self.panel_rect().y - t.space[2] as i32 - h as i32, w, h))
    }

    /// Je popisek uloh videt (mys nad ikonou, zadny panel)?
    pub(crate) fn tasks_tip_visible(&self) -> bool {
        self.input.hover == Some(crate::render::Hover::Tasks) && self.flyout.is_none()
    }

    /// Nadpis a radky panelu.
    fn flyout_text(&self, f: Flyout) -> (String, Vec<String>) {
        match f {
            Flyout::Net => {
                let line = match self.status.net {
                    Some(NetState::Online) => "Pevná síť: připojeno",
                    Some(NetState::Connecting) => "Pevná síť: kabel připojen, čekám na adresu (DHCP)",
                    Some(NetState::Offline) => "Pevná síť: kabel odpojen",
                    Some(NetState::NoDevice) => "Bez sítě: žádná podporovaná síťová karta",
                    None => "Služba sítě neodpovídá",
                };
                (String::from("Síť"), vec![String::from(line)])
            }
            Flyout::Volume => {
                let line = match self.status.volume {
                    Some((_, true)) => String::from("Ztlumeno"),
                    Some((v, false)) => format!("Hlasitost {v} %"),
                    None => String::from("Zvuková služba neodpovídá"),
                };
                (String::from("Zvuk"), vec![line])
            }
            Flyout::Bell => (String::from("Oznámení"), vec![String::from("Žádná oznámení")]),
            Flyout::Tasks => {
                let (run, agent) = self.tasks;
                let mut l = vec![format!("Běží {run} {}", tasks_word(run))];
                if agent > 0 {
                    l.push(format!("Z toho agent AI: {agent}"));
                }
                l.push(String::from("Správce úloh není spuštěný"));
                (String::from("Úlohy"), l)
            }
        }
    }

    /// Ikona v liste, ke ktere panel patri.
    fn flyout_anchor(&self, f: Flyout) -> Option<Rect> {
        let bar = self.bar_layout();
        match f {
            Flyout::Tasks => bar.tasks,
            Flyout::Net => bar.net,
            Flyout::Volume => bar.volume,
            Flyout::Bell => Some(bar.bell),
        }
    }

    /// Obdelnik otevreneho panelu (nad listou, zarovnany k ikone).
    pub(crate) fn flyout_rect(&self) -> Option<Rect> {
        let f = self.flyout?;
        let a = self.flyout_anchor(f)?;
        let t = &self.style.theme;
        let (title, lines) = self.flyout_text(f);
        let pad = t.space[4];
        let (hs, bs) = (t.text.body_strong, t.text.body);
        let tw = lines.iter().map(|l| self.style.width(l, bs)).chain([self.style.width(&title, hs)]).max().unwrap_or(0);
        let w = (tw + 2 * pad).min(self.w);
        let h = hs.line + lines.len() as u32 * (bs.line + t.space[2]) + 2 * pad;
        let bottom = self.panel_rect().y - t.space[3] as i32;
        let x = (a.right() as i32 - w as i32).clamp(0, (self.w - w) as i32);
        Some(Rect::new(x, bottom - h as i32, w, h))
    }

    /// Otevre nebo zavre panel `f` (druhy klik na tutez ikonu zavira).
    pub(crate) fn toggle_flyout(&mut self, f: Flyout) {
        let next = if self.flyout == Some(f) { None } else { Some(f) };
        self.set_flyout(next);
    }

    pub(crate) fn set_flyout(&mut self, f: Option<Flyout>) {
        if let Some(r) = self.flyout_rect() {
            self.damage(r);
        }
        self.flyout = f;
        if let Some(r) = self.flyout_rect() {
            self.damage(r);
        }
    }

    /// Kresli otevreny panel a popisek uloh (vrstva nad okny).
    pub(crate) fn draw_flyout(&mut self, c: &mut Canvas<'_>) {
        if self.tasks_tip_visible()
            && let Some(r) = self.tasks_tip_rect()
        {
            let t = self.style.theme.clone();
            let s = t.text.small;
            let text = self.tasks_tip();
            c.round_fill_bordered(r, t.radius.control, t.color.surface, t.color.border_strong, 1);
            let face = self.style.face(s);
            let base = r.y + face.baseline_in(s.size, r.h);
            face.draw(c, r.x + t.space[3] as i32, base, &text, s.size, t.color.ink);
        }
        let (Some(f), Some(r)) = (self.flyout, self.flyout_rect()) else { return };
        let (title, lines) = self.flyout_text(f);
        let t = self.style.theme.clone();
        let fw = if t.mode == theme::Mode::Contrast { 2 } else { 1 };
        c.round_fill_bordered(r, t.radius.card, t.color.surface, t.color.border_strong, fw);
        let pad = t.space[4] as i32;
        let mut y = r.y + pad;
        let rows = core::iter::once((title, t.text.body_strong, t.color.ink)).chain(lines.into_iter().map(|l| (l, t.text.body, t.color.ink_muted)));
        for (s, style, ink) in rows {
            let face = self.style.face(style);
            let base = y + face.baseline_in(style.size, style.line);
            face.draw(c, r.x + pad, base, &s, style.size, ink);
            y += (style.line + t.space[2]) as i32;
        }
    }
}
