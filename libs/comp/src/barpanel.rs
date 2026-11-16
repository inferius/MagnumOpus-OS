//! Panely listy nad knihovnou `ui` (komponenta panel, `libs/ui/src/panel.rs`,
//! design/bubliny-panely.html): Vzhled, Sit, Zvuk, Oznameni, Rezim
//! vyvojare. Pohled se sklada z beznych prvku `ui` (volby radio, prepinac,
//! posuvnik, tlacitko v pate), takze fokus, klavesy, najeti a pristupnost
//! jsou stejne jako v aplikacich.
//!
//! - Sirka `size.panel_w`, vyska podle obsahu, nad ikonou (stred na ikone,
//!   v obrazovce), zaobleni `radius.window`, stin `shadow.popover`.
//! - Material: rozmazana scena pod panelem s kryti `opacity.bar` (jako
//!   lista). Spocita se jednou pri otevreni a znovu jen pri poskozeni
//!   sceny pod panelem; trva-li vypocet dele nez rozpocet skladani
//!   (`COMPOSE_BUDGET`), panel prejde na plne pozadi a zaloguje to jednou.
//!   Vysoky kontrast (`opacity.bar` = 1) = plne pozadi.
//! - Klik, pohyb a klavesy jdou pohledu, dokud je panel otevreny; Esc nebo
//!   klik mimo ho zavre (`input.rs`).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gfx::{Canvas, Rect};
use guiproto::window::InputEvent;
use theme::Scheme;
use ui::{Node, PanelSize, Ui, button, column, label, panel, panel_footer, panel_header, panel_line, panel_state, radio, slider, switch};

use crate::flyout::Flyout;
use crate::prof::Part;
use crate::render::NetState;
use crate::{COMPOSE_BUDGET, Comp};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PanelMsg {
    Scheme(Scheme),
    Contrast(bool),
    #[cfg(feature = "dev-mode")]
    Dev(bool),
    Volume(i32),
    Mute,
}

/// Otevreny panel listy.
pub(crate) struct BarPanel {
    pub kind: Flyout,
    pub ui: Ui<PanelMsg>,
    /// Obdelnik na obrazovce (pri sestaveni pohledu).
    pub rect: Rect,
    /// Material pod panelem (pixely `rect`); `None` = plne pozadi.
    pub blur: Option<Vec<u32>>,
    pub blur_dirty: bool,
    /// Stisk zacal v panelu (pusteni jde pohledu i mimo nej).
    pub pressed: bool,
    /// Ukazatel je nad panelem.
    pub hover: bool,
}

/// Duvod nastaveni, ktere zatim nejsou (vypnute tlacitko v pate).
const NO_SETTINGS: &str = "Nastavení zatím nejsou";

impl Comp {
    /// Kresli panel `f` knihovna ui (ostatni karty jsou ve `flyout.rs`)?
    pub(crate) fn is_ui_panel(f: Flyout) -> bool {
        match f {
            Flyout::Look | Flyout::Net | Flyout::Volume | Flyout::Bell => true,
            #[cfg(feature = "dev-mode")]
            Flyout::Dev => true,
            _ => false,
        }
    }

    fn look_view(&self, s: &PanelSize) -> Node<PanelMsg> {
        let (_, p) = self.appearance();
        let now = match self.mode() {
            theme::Mode::Light => "teď světlý",
            theme::Mode::Dark => "teď tmavý",
            theme::Mode::Contrast => "vysoký kontrast",
        };
        let opts: Vec<Node<PanelMsg>> = Scheme::ALL
            .iter()
            .map(|&sc| {
                let (icon, desc) = match sc {
                    Scheme::Light => ("sun", ""),
                    Scheme::Dark => ("moon", ""),
                    Scheme::Auto => ("clock", if self.appearance.place.is_some() { "podle východu a západu slunce" } else { "světlý od 7 do 19 h" }),
                };
                radio(sc.label(), p.scheme == sc).on_click(PanelMsg::Scheme(sc)).panel_row(Some(icon), desc)
            })
            .collect();
        let state = self.look_state();
        panel(vec![
            panel_header(s, self.look_icon(), "Vzhled", now),
            column(opts).gap(0).key("volby"),
            panel_state(s, "info", &state, false),
            switch("Vysoký kontrast", p.contrast).on_toggle(PanelMsg::Contrast).panel_row(Some("contrast"), "z Přístupnosti; přebíjí světlý i tmavý").key("kontrast"),
            panel_footer(s, vec![button("Nastavení vzhledu").icon("palette").disabled(true).tooltip(NO_SETTINGS).grow()]),
        ])
    }

    fn net_view(&self, s: &PanelSize) -> Node<PanelMsg> {
        let (sum, desc) = match self.status.net {
            Some(NetState::Online) => ("připojeno", "připojeno"),
            Some(NetState::Connecting) => ("připojuji", "kabel připojen, čekám na adresu (DHCP)"),
            Some(NetState::Offline) => ("odpojeno", "kabel odpojen"),
            Some(NetState::NoDevice) => ("bez sítě", "žádná podporovaná síťová karta"),
            None => ("neodpovídá", "služba sítě neodpovídá"),
        };
        let icon = if self.status.net == Some(NetState::NoDevice) { "network" } else { "ethernet" };
        panel(vec![
            panel_header(s, "network", "Síť", sum),
            label("Pevná síť").panel_row(Some(icon), desc),
            panel_footer(s, vec![button("Nastavení sítě").icon("settings").disabled(true).tooltip(NO_SETTINGS).grow()]),
        ])
    }

    fn volume_view(&self, s: &PanelSize) -> Node<PanelMsg> {
        let (sum, muted) = match self.status.volume {
            Some((_, true)) => (String::from("ztlumeno"), true),
            Some((v, false)) => (alloc::format!("{v} %"), false),
            None => (String::from("neodpovídá"), false),
        };
        let level = self.status.volume.map_or(0, |(v, _)| v as i32);
        // Ovladani jen s kanalem ke sluzbe zvuku (od initu).
        let off = self.status.volume.is_none() || !self.audio_ctl;
        let mut items = vec![
            panel_header(s, if muted { "volume-off" } else { "volume" }, "Zvuk", &sum),
            panel_line(
                s,
                vec![
                    button("").icon(if muted { "volume-off" } else { "volume" }).tooltip(if muted { "Zapnout zvuk" } else { "Ztlumit" }).on_click(PanelMsg::Mute).disabled(off),
                    slider(level, 0, 100).step(5).on_value(PanelMsg::Volume).disabled(off).grow().key("hlasitost"),
                ],
            ),
        ];
        if !self.status.capturing.is_empty() {
            let who = self.status.capturing.join(", ");
            items.push(panel_state(s, "mic", &alloc::format!("Nahrává: {who}"), true));
        }
        items.push(panel_footer(s, vec![button("Nastavení zvuku").icon("settings").disabled(true).tooltip(NO_SETTINGS).grow()]));
        panel(items)
    }

    fn bell_view(&self, s: &PanelSize) -> Node<PanelMsg> {
        let dnd = self.do_not_disturb();
        let mut items = vec![panel_header(s, if dnd { "bell-off" } else { "bell" }, "Oznámení", if dnd { "Nerušit" } else { "žádná nová" })];
        if dnd {
            items.push(label("Nerušit").panel_row(Some("bell-off"), "zapnuto při celé obrazovce; oznámení se ukážou po návratu"));
        } else {
            items.push(label("Žádná oznámení").panel_row(Some("bell"), ""));
        }
        items.push(panel_footer(s, vec![button("Nastavení oznámení").icon("settings").disabled(true).tooltip(NO_SETTINGS).grow()]));
        panel(items)
    }

    #[cfg(feature = "dev-mode")]
    fn dev_view(&self, s: &PanelSize) -> Node<PanelMsg> {
        let on = self.dev_mode.on;
        let mut items = vec![
            panel_header(s, "code", "Režim vývojáře", "vývojové sestavení"),
            switch("Režim vývojáře", on).on_toggle(PanelMsg::Dev).panel_row(Some("shield-alert"), "Úkony správy se potvrdí samy, bez dialogu.").key("rezim"),
        ];
        if on {
            items.push(panel_state(s, "alert", "Systém teď není chráněný: aplikace se správou můžou měnit systém bez potvrzení. Vypněte, až nebudete ladit.", true));
        }
        items.push(panel_footer(s, vec![label("Jen vývojové sestavení; vydání tuto volbu nemá.").tone(ui::TextTone::Small)]));
        panel(items)
    }

    fn bar_panel_view(&self, f: Flyout) -> Option<Node<PanelMsg>> {
        let s = PanelSize::of(&self.style.theme);
        Some(match f {
            Flyout::Look => self.look_view(&s),
            Flyout::Net => self.net_view(&s),
            Flyout::Volume => self.volume_view(&s),
            Flyout::Bell => self.bell_view(&s),
            #[cfg(feature = "dev-mode")]
            Flyout::Dev => self.dev_view(&s),
            _ => return None,
        })
    }

    /// Otevre panel podle jmena (`panel` v `guiproto::control`); `none`
    /// zavre. Vraci, zda jmeno zna.
    pub fn open_panel(&mut self, name: &str) -> bool {
        let f = match name {
            "look" => Flyout::Look,
            "net" => Flyout::Net,
            "volume" => Flyout::Volume,
            "bell" => Flyout::Bell,
            #[cfg(feature = "dev-mode")]
            "dev" => Flyout::Dev,
            "none" => {
                self.set_flyout(None);
                return true;
            }
            _ => return false,
        };
        self.set_flyout(Some(f));
        true
    }

    /// Nadpis pohledu pro pristupnost.
    fn bar_panel_title(f: Flyout) -> &'static str {
        match f {
            Flyout::Look => "Vzhled",
            Flyout::Net => "Síť",
            Flyout::Volume => "Zvuk",
            Flyout::Bell => "Oznámení",
            _ => "Panel",
        }
    }

    /// Znovu sestavi pohled otevreneho panelu (po zmene stavu, rezimu).
    pub(crate) fn bar_panel_rebuild(&mut self) {
        let f = match self.flyout {
            Some(f) if Self::is_ui_panel(f) => f,
            _ => {
                if let Some(old) = self.barpanel.take() {
                    self.damage_top(self.panel_shadow_area(old.rect));
                }
                return;
            }
        };
        let Some(anchor) = self.flyout_anchor(f) else {
            self.barpanel = None;
            return;
        };
        let t = self.style.theme.clone();
        let edge = t.size.bar_inset;
        let w = t.size.panel_w.min(self.w.saturating_sub(2 * edge)).max(1);
        let max_h = self.panel_rect().y.saturating_sub((t.space[3] + edge) as i32).max(1) as u32;
        let Some(view) = self.bar_panel_view(f) else { return };
        let mode = self.style.mode();
        let mut bp = match self.barpanel.take() {
            Some(b) if b.kind == f => b,
            _ => {
                let mut ui = Ui::new(ui::Look::ui_fonts(), w, max_h);
                ui.window_pad = false;
                ui.transparent = true;
                BarPanel { kind: f, ui, rect: Rect::default(), blur: None, blur_dirty: true, pressed: false, hover: false }
            }
        };
        if bp.ui.look.theme.mode != mode {
            bp.ui.look.set_mode(mode);
        }
        // Vyska podle obsahu (prirozena vyska pohledu), nejvys nad listou.
        let h = bp.ui.natural_height(&view).clamp(1, max_h);
        if (bp.ui.w, bp.ui.h) != (w, h) {
            bp.ui.resize(w, h);
        }
        bp.ui.rebuild(String::from(Self::bar_panel_title(f)), view);
        let bottom = self.panel_rect().y - t.space[3] as i32;
        let cx = anchor.x + anchor.w as i32 / 2;
        let x = (cx - w as i32 / 2).clamp(edge as i32, (self.w - w - edge) as i32);
        let rect = Rect::new(x, bottom - h as i32, w, h);
        if rect != bp.rect {
            let old = bp.rect;
            bp.rect = rect;
            bp.blur_dirty = true;
            if old.w > 0 {
                self.damage_top(self.panel_shadow_area(old));
            }
        }
        let area = self.panel_shadow_area(rect);
        self.barpanel = Some(bp);
        self.damage_top(area);
    }

    /// Obdelnik otevreneho panelu (`None` = zadny panel nad ui).
    pub(crate) fn bar_panel_rect(&self) -> Option<Rect> {
        self.barpanel.as_ref().map(|b| b.rect)
    }

    /// Panel a jeho stin (poskozeni pri otevreni a zavreni).
    pub(crate) fn panel_shadow_area(&self, r: Rect) -> Rect {
        let mut a = r;
        for s in &self.style.theme.shadow.popover {
            let e = s.blur as i32 + s.spread;
            a = a.union(&Rect::new(r.x + s.x - e, r.y + s.y - e, r.w + 2 * e as u32, r.h + 2 * e as u32));
        }
        a.intersect(&self.screen())
    }

    /// Udalost pro pohled panelu (souradnice obrazovky). Vraci, zda ji
    /// panel vzal.
    pub(crate) fn bar_panel_event(&mut self, ev: InputEvent, mods: u16, time: u64, now: u64) -> bool {
        let Some(bp) = self.barpanel.as_mut() else { return false };
        let r = bp.rect;
        let ev = match ev {
            InputEvent::Button { code, pressed, x, y } => InputEvent::Button { code, pressed, x: x - r.x, y: y - r.y },
            InputEvent::PointerMove { x, y } => InputEvent::PointerMove { x: x - r.x, y: y - r.y },
            e => e,
        };
        bp.ui.set_input_class(self.input.class);
        let resp = bp.ui.event_at(ev, mods, Some(time));
        let redraw = resp.redraw || !resp.msgs.is_empty();
        for m in resp.msgs {
            self.bar_panel_msg(m, now);
        }
        if redraw {
            self.bar_panel_rebuild();
        }
        true
    }

    fn bar_panel_msg(&mut self, m: PanelMsg, now: u64) {
        match m {
            PanelMsg::Scheme(s) => self.choose_scheme(s, now),
            PanelMsg::Contrast(on) => self.choose_contrast(on, now),
            #[cfg(feature = "dev-mode")]
            PanelMsg::Dev(on) => {
                if let Some(peer) = self.dev_mode.peer {
                    self.out.push(crate::Out::Dialog(peer, guiproto::control::DialogReply::DevMode { on }));
                }
            }
            // Hlasitost meni sluzba zvuku (kanal od initu); bez nej je
            // ovladani vypnute.
            PanelMsg::Volume(v) => self.out.push(crate::Out::Audio(crate::AudioReq::Master(v.clamp(0, 100) as u8))),
            PanelMsg::Mute => {
                let muted = self.status.volume.is_some_and(|(_, m)| m);
                self.out.push(crate::Out::Audio(crate::AudioReq::Mute(!muted)));
            }
        }
    }

    /// Material pod panelem: rozmazana scena (jednou, znovu po poskozeni
    /// pod panelem). Nad rozpocet skladani -> plne pozadi natrvalo.
    fn ensure_panel_blur(&mut self, now: u64) {
        let solid = self.style.theme.opacity.bar >= 255 || self.panel_material_off;
        let Some(bp) = self.barpanel.as_ref() else { return };
        if solid {
            if bp.blur.is_some() {
                self.barpanel.as_mut().unwrap().blur = None;
            }
            return;
        }
        if !bp.blur_dirty && bp.blur.is_some() {
            return;
        }
        let r = bp.rect;
        let t0 = self.prof.start();
        let band = self.scene_blur(r, now);
        self.prof.add(Part::PanelBlur, t0);
        let spent = self.prof.start().saturating_sub(t0);
        if let Some(bp) = self.barpanel.as_mut() {
            bp.blur = Some(band);
            bp.blur_dirty = false;
        }
        if t0 != 0 && spent > COMPOSE_BUDGET {
            self.panel_material_off = true;
            self.log(format_args!("compd: material panelu {} ms nad rozpocet {} ms, plne pozadi", spent / 1_000_000, COMPOSE_BUDGET / 1_000_000));
            if let Some(bp) = self.barpanel.as_mut() {
                bp.blur = None;
            }
        }
    }

    /// Kresli otevreny panel (vrstva nad okny).
    pub(crate) fn draw_bar_panel(&mut self, c: &mut Canvas<'_>, now: u64) {
        if self.barpanel.is_none() {
            return;
        }
        self.ensure_panel_blur(now);
        let t = self.style.theme.clone();
        let col = &t.color;
        let Some(bp) = self.barpanel.as_mut() else { return };
        let r = bp.rect;
        let n = (r.w * r.h) as usize;
        // Pozadi: material (rozmazana scena + surface s kryti listy) nebo plne.
        let mut px = match &bp.blur {
            Some(b) if b.len() == n => {
                let k = t.opacity.bar;
                b.iter().map(|&p| over(col.surface, p, k)).collect()
            }
            _ => vec![col.surface; n],
        };
        if let Some(mut pc) = Canvas::new(&mut px, r.w, r.h, r.w) {
            bp.ui.draw(&mut pc, 0);
        }
        let pop = t.shadow.popover.clone();
        let rad = t.radius.window;
        self.style.draw_shadow(c, &pop, r, rad, Rect::default());
        c.round_image(&px, r, rad);
        let fw = if t.mode == theme::Mode::Contrast { 2 } else { 1 };
        c.round_stroke(r, rad, fw, col.border);
    }
}

/// Barva `fg` s kryti `k`/255 pres `bg` (nepruhledne).
fn over(fg: u32, bg: u32, k: u32) -> u32 {
    let m = |sh: u32| ((fg >> sh & 0xFF) * k + (bg >> sh & 0xFF) * (255 - k)) / 255;
    0xFF00_0000 | m(16) << 16 | m(8) << 8 | m(0)
}
