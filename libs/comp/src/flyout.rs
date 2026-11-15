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

use theme::Scheme;

use crate::Comp;
use crate::render::NetState;

/// Ktery panel je otevreny.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flyout {
    Tasks,
    Net,
    Volume,
    Bell,
    /// Spoustec (ikona mrizky): radek = aplikace, klik spusti / aktivuje.
    Apps,
    /// Okna jedne aplikace (zastupce v liste): radek = okno, posledni
    /// "Nove okno".
    AppWindows(u32),
    /// Vzhled: radky Svetly, Tmavy, Automaticky (zvoleny s fajfkou),
    /// pak stav (kdy se automaticky prepne, kontrast z Pristupnosti).
    Look,
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

    /// Radek stavu site (panel i bublina).
    pub(crate) fn net_line(&self) -> String {
        String::from(match self.status.net {
            Some(NetState::Online) => "Pevná síť: připojeno",
            Some(NetState::Connecting) => "Pevná síť: kabel připojen, čekám na adresu (DHCP)",
            Some(NetState::Offline) => "Pevná síť: kabel odpojen",
            Some(NetState::NoDevice) => "Bez sítě: žádná podporovaná síťová karta",
            None => "Služba sítě neodpovídá",
        })
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
            Flyout::Bell => (String::from("Oznámení"), self.dnd_lines()),
            Flyout::Look => (String::from("Vzhled"), self.look_lines()),
            Flyout::AppWindows(g) => {
                let id = self.app_of(g).unwrap_or_default();
                let mut rows: Vec<String> = self.app_windows(&id).iter().filter_map(|w| self.windows.get(w)).map(|w| w.title.clone()).collect();
                if self.is_launchable(&id) {
                    rows.push(String::from("+ Nové okno"));
                }
                let name = self.windows.get(&g).and_then(|w| self.clients.get(&w.client)).map_or(String::new(), |c| c.info.name.clone());
                (name, rows)
            }
            Flyout::Apps => {
                let rows = self
                    .launchable
                    .iter()
                    .map(|(id, name)| if self.app_window(id).is_some() { format!("{name} (běží)") } else { name.clone() })
                    .collect();
                (String::from("Aplikace"), rows)
            }
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
            Flyout::Look => Some(bar.look),
            Flyout::Apps => Some(bar.launcher),
            Flyout::AppWindows(g) => bar.apps.iter().find(|(x, _)| *x == g).map(|(_, r)| *r),
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
        // Vzhled: misto na fajfku zvoleneho radku.
        let mark = if f == Flyout::Look { t.space[3] + t.size.icon_s } else { 0 };
        let w = (tw + mark + 2 * pad).min(self.w);
        let h = hs.line + lines.len() as u32 * (bs.line + t.space[2]) + 2 * pad;
        let bottom = self.panel_rect().y - t.space[3] as i32;
        // Spoustec vlevo: karta zarovnana k leve strane ikony.
        let x = if matches!(f, Flyout::Apps | Flyout::AppWindows(_)) { a.x } else { a.right() as i32 - w as i32 }.clamp(0, (self.w - w) as i32);
        Some(Rect::new(x, bottom - h as i32, w, h))
    }

    /// Radek panelu (bez nadpisu) na vysce `y`.
    pub(crate) fn flyout_row_at(&self, y: i32) -> Option<usize> {
        let r = self.flyout_rect()?;
        let t = &self.style.theme;
        let top = r.y + t.space[4] as i32 + (t.text.body_strong.line + t.space[2]) as i32;
        let row = (t.text.body.line + t.space[2]) as i32;
        (y >= top).then(|| ((y - top) / row) as usize)
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
        self.draw_tooltip(c);
        let (Some(f), Some(r)) = (self.flyout, self.flyout_rect()) else { return };
        let (title, lines) = self.flyout_text(f);
        let t = self.style.theme.clone();
        let fw = if t.mode == theme::Mode::Contrast { 2 } else { 1 };
        c.round_fill_bordered(r, t.radius.card, t.color.surface, t.color.border_strong, fw);
        let pad = t.space[4] as i32;
        let mut y = r.y + pad;
        // Vzhled: volby jsou plny text, zvolena s fajfkou vpravo.
        let chosen = (f == Flyout::Look).then(|| Scheme::ALL.iter().position(|&s| s == self.appearance().1.scheme)).flatten();
        let choices = if f == Flyout::Look { Scheme::ALL.len() } else { 0 };
        let rows = core::iter::once((title, t.text.body_strong, t.color.ink))
            .chain(lines.into_iter().enumerate().map(|(i, l)| (l, t.text.body, if i < choices { t.color.ink } else { t.color.ink_muted })));
        for (k, (s, style, ink)) in rows.enumerate() {
            if k > 0 && chosen == Some(k - 1) {
                let is = t.size.icon_s;
                let x = r.right() as i32 - pad - is as i32;
                self.style.icons.draw(c, "check", x, y + (style.line as i32 - is as i32) / 2, is, t.icon_stroke, t.color.accent_ink);
            }
            let face = self.style.face(style);
            let base = y + face.baseline_in(style.size, style.line);
            face.draw(c, r.x + pad, base, &s, style.size, ink);
            y += (style.line + t.space[2]) as i32;
        }
    }
}
