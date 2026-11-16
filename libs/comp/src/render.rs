//! Skladani snimku: plocha, okna (stin, ram, titulek, vrstvy), odznak
//! duvery, tlacitka, lista, ztlumeni a systemovy dialog, kurzor.
//!
//! Vzhled podle `design/okna.html` a `design/plocha.html`; barvy, rozmery,
//! zaobleni, stiny a pisma jsou z tokenu (`style.rs`), ne z kodu.
//!
//! Kresli se jen do poskozenych obdelniku (orez platna). Pruhlednost
//! skupiny se nasobi do deti (bez mezibufferu: prekryvajici se deti
//! poloprusvitne skupiny se prolnou i mezi sebou - zjednoduseni 2D cesty).
//!
//! Poradi kresleni okna (CPU i GPU dava stejny vysledek, `gpu.rs`):
//! 1. stin (vrstvy slozene do jedne barvy, jedno prolnuti),
//! 2. ulozit pixely rohu pod oknem,
//! 3. telo, titulek, obsah aplikace, prvky nad obsahem, ram (ctverec),
//! 4. rohy: okno pres ulozene pozadi s pokrytim zaobleneho tvaru.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::shape::core_rects;
use gfx::{Canvas, Image, scale};
use guiproto::Rect;
use guiproto::window::{LayerKind, Mode};
use theme::TextStyle;

use crate::Comp;
use crate::prof::Part;
use crate::scene::{Client, Layer, Window};
use crate::style::Style;

/// Id aplikace spravce uloh (klik na ikonu uloh ji da do popredi).
pub const TASKS_APP: &str = "org.magnumopus.tasks";

/// Kurzor: 12 x 19, 0 pruhledne, 1 obrys, 2 vypln.
const CURSOR: [&str; 19] = [
    "1...........",
    "11..........",
    "121.........",
    "1221........",
    "12221.......",
    "122221......",
    "1222221.....",
    "12222221....",
    "122222221...",
    "1222222221..",
    "12222222221.",
    "122222211111",
    "1222121.....",
    "122111 1....",
    "121..121....",
    "11...121....",
    "1.....121...",
    "......121...",
    ".......11...",
];
pub const CURSOR_W: u32 = 12;
/// Polomer kurzoru zmeny velikosti (aktivni bod uprostred, s obrysem).
pub const RESIZE_CURSOR: u32 = 9;
pub const CURSOR_H: u32 = 19;


/// Rozmazani pozadi listy (prototyp `.bar`: `backdrop-filter: blur(16px)`,
/// smerodatna odchylka v px).
pub const BAR_BLUR: u32 = 16;

/// Text ikony uloh (pristupnost, inspekce).
pub fn tasks_label(run: u32, agent: u32) -> String {
    if agent > 0 { format!("Úlohy: {run} (agent {agent})") } else { format!("Úlohy: {run}") }
}

/// Text upozorneni na umely vstup v liste.
pub const INJECT_LABEL: &str = "Vstup ovládá automat";

/// Text pilulky vzdaleneho pristupu: prvni pripojeny a pocet dalsich.
pub fn remote_label(who: &[String]) -> String {
    match who {
        [] => String::new(),
        [one] => format!("Vzdáleně: {one}"),
        [one, rest @ ..] => format!("Vzdáleně: {one} +{}", rest.len()),
    }
}

/// Ikona aplikace v liste podle id (bez vlastnich ikon aplikaci zatim
/// obecna sada).
pub fn app_icon(id: &str) -> &'static str {
    if id.contains("term") {
        "terminal"
    } else if id.contains("tasks") {
        "tasks"
    } else if id.contains("files") || id.contains("soubory") {
        "folder"
    } else if id.contains("settings") || id.contains("nastaveni") {
        "settings"
    } else if id.contains("install") {
        "download"
    } else {
        "window"
    }
}

/// Ikona opravneni v odznaku.
pub fn perm_icon(p: &str) -> &'static str {
    let p = p.to_lowercase();
    if p.contains("síť") || p.contains("net") {
        "globe"
    } else if p.contains("soub") || p.contains("fs") || p.contains("slož") {
        "folder"
    } else if p.contains("kamer") {
        "camera"
    } else if p.contains("mikro") {
        "mic"
    } else {
        "key"
    }
}

/// Rozlozeni listy.
#[derive(Debug, Clone, Default)]
pub struct Bar {
    pub rect: Rect,
    /// Spoustec aplikaci (ikona `apps`; nabidku dodela shell plochy A1).
    pub launcher: Rect,
    /// Tlacitka aplikaci s okny: (okno zastupce - s fokusem nebo nahore,
    /// obdelnik); `groups` = vsechna okna te aplikace shora dolu.
    pub apps: Vec<(u32, Rect)>,
    pub groups: Vec<Vec<u32>>,
    /// Spustitelne aplikace bez okna: (index v `launchable`, obdelnik).
    pub launch: Vec<(usize, Rect)>,
    pub agent: Option<Rect>,
    /// Pilulka vzdaleneho pristupu (pripojena relace SSH).
    pub remote: Option<Rect>,
    /// Pilulka rezimu vyvojare (jen vyvojove sestaveni, `devmode.rs`).
    #[cfg(feature = "dev-mode")]
    pub dev: Option<Rect>,
    pub tasks: Option<Rect>,
    /// Stav site, hlasitost, oznameni (zleva doprava jako v prototypu).
    pub net: Option<Rect>,
    pub volume: Option<Rect>,
    pub bell: Rect,
    /// Vzhled (svetly, tmavy, automaticky; `appearance.rs`).
    pub look: Rect,
    pub clock: Option<Rect>,
    /// Oddelovace (x).
    pub seps: Vec<i32>,
}

/// Stav site v liste.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetState {
    /// Bez linky.
    Offline,
    /// Linka je, adresa jeste ne (DHCP).
    Connecting,
    Online,
    /// Zadne rozhrani (karta chybi nebo ji neumime, napr. e1000 ve VMware).
    NoDevice,
}

/// Stav systemu zobrazeny v liste.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BarStatus {
    pub net: Option<NetState>,
    /// Hlasitost v procentech a ztlumeni.
    pub volume: Option<(u8, bool)>,
    /// Aplikace, ktere prave nahravaji zvuk (indikator mikrofonu v liste;
    /// soukromi: uzivatel vzdy vidi, ze se nahrava).
    pub capturing: Vec<String>,
    /// Zarizeni a aplikace pro panel Zvuk (`None` = sluzba je neposlala).
    pub audio: Option<AudioInfo>,
    /// Adresa rozhrani s prefixem (`10.0.2.15/24`), kdyz ji netd hlasi.
    pub net_addr: Option<String>,
    /// Vzdalene relace (`uzivatel@adresa`, SSH): pilulka v liste, at je
    /// u pocitace videt, ze je nekdo pripojeny (`docs/cs/ssh.md`).
    pub remote: Vec<String>,
}

/// Zvukove zarizeni v panelu Zvuk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDev {
    pub index: u8,
    pub name: String,
    /// Aktivni vystupy (`line-out,headphone`), prazdne = zadny.
    pub outputs: String,
    pub input: bool,
    /// Prave pouzivany vystup.
    pub active: bool,
}

/// Aplikace s proudy v panelu Zvuk (identita z manifestu).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioApp {
    pub id: String,
    pub name: String,
    pub volume: u8,
    pub playing: bool,
    pub capturing: bool,
}

/// Co panel Zvuk ukazuje krome hlasitosti.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AudioInfo {
    pub devices: Vec<AudioDev>,
    pub apps: Vec<AudioApp>,
    /// Zarizeni, ze ktereho se nahrava.
    pub input: Option<u8>,
}

/// Na co ukazuje mys (zvyrazneni pri najeti).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hover {
    /// Tlacitko okna (okno, index: 0 zavrit, 1 maximalizovat, 2 minimalizovat).
    Button(u32, usize),
    /// Tlacitko okna v liste.
    BarApp(u32),
    /// Nespustena aplikace v liste (index v `launchable`).
    BarLaunch(usize),
    Tasks,
    Launcher,
}

impl Comp {
    /// Kolik mista zabira lista u spodniho okraje (vcetne odsazeni).
    pub fn bar_reserve(&self) -> u32 {
        let s = &self.style.theme.size;
        s.bar_h + 2 * s.bar_inset
    }

    /// Plocha pro okna (obrazovka bez listy).
    pub fn work_area(&self) -> Rect {
        Rect::new(0, 0, self.w, self.h.saturating_sub(self.bar_reserve()))
    }

    pub(crate) fn panel_rect(&self) -> Rect {
        let s = &self.style.theme.size;
        // Cela obrazovka: lista skryta (mimo obrazovku, nic ji nezasahne).
        let y = if self.fullscreen_active() { self.h as i32 + 64 } else { (self.h - s.bar_h - s.bar_inset) as i32 };
        Rect::new(s.bar_inset as i32, y, self.w.saturating_sub(2 * s.bar_inset), s.bar_h)
    }

    /// Lista vcetne stinu (poskozeni).
    pub(crate) fn panel_visual(&self) -> Rect {
        let p = self.panel_rect();
        let mut v = p;
        for s in &self.style.theme.shadow.bar {
            let e = s.blur as i32 + s.spread;
            v = v.union(&Rect::new(p.x + s.x - e, p.y + s.y - e, p.w + 2 * e as u32, p.h + 2 * e as u32));
        }
        v
    }

    pub(crate) fn damage_panel(&mut self) {
        let r = self.panel_visual();
        self.damage_top(r);
    }

    /// Odkud se bere rozmazane pozadi listy: lista a okraj 3 sigma (dal
    /// Gaussovo jadro nesaha).
    pub(crate) fn blur_source(&self) -> Rect {
        let p = self.panel_rect();
        let e = 3 * BAR_BLUR;
        Rect::new(p.x - e as i32, p.y - e as i32, p.w + 2 * e, p.h + 2 * e).intersect(&self.screen())
    }

    /// Rozmazane pozadi listy: prepocita se, jen kdyz se pod listou neco
    /// zmenilo (pohyb okna, obsah, motiv). Scena se pro oblast pod listou
    /// vykresli znovu do mezibufferu (stejne na CPU i GPU, proto stejne
    /// pixely), rozmaze a orizne na listu.
    pub(crate) fn ensure_bar_blur(&mut self, now: u64) {
        if self.style.theme.opacity.bar >= 255 {
            self.bar_blur = None;
            return;
        }
        if !self.bar_blur_dirty && self.bar_blur.is_some() {
            return;
        }
        let p = self.panel_rect();
        self.bar_blur = Some(self.scene_blur(p, now));
        self.bar_blur_dirty = false;
    }

    /// Rozmazana scena (plocha a okna) pod obdelnikem `p`: pixely `p`
    /// (radky po `p.w`). Scena se pro okoli vykresli znovu do mezibufferu
    /// (stejne na CPU i GPU), rozmaze a orizne.
    pub(crate) fn scene_blur(&mut self, p: Rect, now: u64) -> Vec<u32> {
        let e = 3 * BAR_BLUR;
        let src = Rect::new(p.x - e as i32, p.y - e as i32, p.w + 2 * e, p.h + 2 * e).intersect(&self.screen());
        let p = p.intersect(&src);
        let (w, h) = (self.w, self.h);
        let mut scratch = core::mem::take(&mut self.blur_scratch);
        scratch.resize((w * h) as usize, 0);
        {
            let mut c = Canvas::new(&mut scratch, w, h, w).unwrap();
            c.set_clip(src);
            self.desktop(&mut c);
            for &gid in &self.z {
                let Some(win) = self.windows.get(&gid) else { continue };
                if !win.mapped || win.minimized || win.visual().intersect(&src).is_empty() {
                    continue;
                }
                let Some(cl) = self.clients.get(&win.client) else { continue };
                let kids = children(cl, win.local);
                let focused = self.focus == Some(gid);
                draw_window(&mut c, &mut self.style, win, cl, &kids, focused, now, None);
                c.set_clip(src);
            }
        }
        let blurred = gfx::blur::blurred_region(&scratch, w, src, BAR_BLUR as f32);
        let mut band = Vec::with_capacity((p.w * p.h) as usize);
        for y in 0..p.h {
            let row = ((p.y - src.y) as u32 + y) * src.w + (p.x - src.x) as u32;
            band.extend_from_slice(&blurred[row as usize..(row + p.w) as usize]);
        }
        self.blur_scratch = scratch;
        band
    }

    /// Stav site pro ikonu v liste (`None` = sluzba site neni).
    pub fn set_net(&mut self, net: Option<NetState>) {
        if self.status.net != net {
            self.status.net = net;
            self.damage_panel();
            self.bar_panel_changed();
        }
    }

    /// Hlasitost a ztlumeni pro ikonu v liste (`None` = zvuk neni).
    pub fn set_volume(&mut self, volume: Option<(u8, bool)>) {
        if self.status.volume != volume {
            self.status.volume = volume;
            self.damage_panel();
            self.bar_panel_changed();
        }
    }

    /// Adresa rozhrani pro panel Sit (`None` = bez adresy).
    pub fn set_net_addr(&mut self, addr: Option<String>) {
        if self.status.net_addr != addr {
            self.status.net_addr = addr;
            self.bar_panel_changed();
        }
    }

    /// Zarizeni a aplikace pro panel Zvuk (`OVERVIEW` sluzby zvuku).
    pub fn set_audio_info(&mut self, info: Option<AudioInfo>) {
        if self.status.audio != info {
            self.status.audio = info;
            self.bar_panel_changed();
        }
    }

    /// Aplikace, ktere nahravaji (ze stavu `audiod`); prazdne = nikdo.
    pub fn set_capturing(&mut self, apps: Vec<String>) {
        if self.status.capturing != apps {
            self.status.capturing = apps;
            self.damage_panel();
            self.bar_panel_changed();
        }
    }

    /// Vzdalene relace (`uzivatel@adresa`) pro pilulku v liste.
    pub fn set_remote(&mut self, who: Vec<String>) {
        if self.status.remote != who {
            self.status.remote = who;
            self.damage_panel();
        }
    }

    /// Hodiny v liste (cas a datum; `None` = nezobrazovat).
    pub fn set_clock(&mut self, clock: Option<(String, String)>) {
        if self.clock != clock {
            self.clock = clock;
            self.damage_panel();
        }
    }

    /// Rozlozeni listy (`design/plocha.html`, poloha "dole, odsazena").
    pub fn bar_layout(&self) -> Bar {
        let t = &self.style.theme;
        let p = self.panel_rect();
        let item = t.size.bar_icon;
        let pad = t.space[2] as i32;
        let gap = t.space[1] as i32;
        let y = p.y + (p.h as i32 - item as i32) / 2;
        let mut bar = Bar { rect: p, ..Default::default() };
        // Vlevo spoustec, oddelovac a okna.
        let mut x = p.x + pad;
        bar.launcher = Rect::new(x, y, item, item);
        x += item as i32 + gap;
        bar.seps.push(x);
        x += 1 + gap;
        // Jedno tlacitko na aplikaci (poradi podle prvniho okna).
        let mut ids: Vec<String> = Vec::new();
        for w in self.windows.values() {
            if let (true, Some(c)) = (w.mapped, self.clients.get(&w.client))
                && !ids.contains(&c.info.id)
            {
                ids.push(c.info.id.clone());
            }
        }
        for id in ids {
            let group = self.app_windows(&id);
            let Some(&top) = group.first() else { continue };
            let rep = group.iter().copied().find(|g| self.focus == Some(*g)).unwrap_or(top);
            bar.apps.push((rep, Rect::new(x, y, item, item)));
            bar.groups.push(group);
            x += item as i32 + gap;
        }
        // Aplikace bez okna (ztlumena ikona, klik spusti).
        for (i, (id, _)) in self.launchable.iter().enumerate() {
            if self.app_window(id).is_none() {
                bar.launch.push((i, Rect::new(x, y, item, item)));
                x += item as i32 + gap;
            }
        }
        // Vpravo: hodiny, oddelovac, ulohy, agent.
        let mut right = p.right() as i32 - pad;
        if let Some((time, date)) = &self.clock {
            let w = self.style.width(time, clock_time(t)).max(self.style.width(date, t.text.caption)) + t.space[3];
            right -= w as i32;
            bar.clock = Some(Rect::new(right, y, w, item));
            right -= gap;
            bar.seps.push(right);
            right -= gap + 1;
        }
        // Oznameni (zatim zastupna ikona), hlasitost, sit.
        right -= item as i32;
        bar.bell = Rect::new(right, y, item, item);
        right -= gap;
        right -= item as i32;
        bar.look = Rect::new(right, y, item, item);
        right -= gap;
        if self.status.volume.is_some() {
            right -= item as i32;
            bar.volume = Some(Rect::new(right, y, item, item));
            right -= gap;
        }
        if self.status.net.is_some() {
            right -= item as i32;
            bar.net = Some(Rect::new(right, y, item, item));
            right -= gap;
        }
        let (run, agent) = self.tasks;
        if run > 0 {
            let w = self.tasks_width(run, agent);
            right -= w as i32;
            bar.tasks = Some(Rect::new(right, y, w, item));
            right -= gap;
        }
        #[cfg(feature = "dev-mode")]
        {
            right = self.dev_chip_layout(&mut bar, right, item, gap);
        }
        if self.input.injecting > 0 {
            let h = t.size.control_h.min(item);
            let w = t.space[4] + t.size.icon_s + t.space[2] + self.style.width(INJECT_LABEL, small_strong(t)) + t.space[4];
            right -= w as i32;
            bar.agent = Some(Rect::new(right, p.y + (p.h as i32 - h as i32) / 2, w, h));
            right -= gap;
        }
        if !self.status.remote.is_empty() {
            let h = t.size.control_h.min(item);
            let label = remote_label(&self.status.remote);
            let w = t.space[4] + t.size.icon_s + t.space[2] + self.style.width(&label, small_strong(t)) + t.space[4];
            right -= w as i32;
            bar.remote = Some(Rect::new(right, p.y + (p.h as i32 - h as i32) / 2, w, h));
        }
        let _ = right;
        bar
    }

    fn tasks_width(&self, run: u32, agent: u32) -> u32 {
        let t = &self.style.theme;
        let mut w = t.space[4] + t.size.icon_m + t.space[2] + self.style.width(&format!("{run}"), small_strong(t)) + t.space[4];
        if agent > 0 {
            w += t.space[2] + self.agent_chip_width(agent);
        }
        w
    }

    fn agent_chip_width(&self, agent: u32) -> u32 {
        let t = &self.style.theme;
        t.space[3] + t.size.icon_s - 2 + t.space[1] + self.style.width(&format!("{agent}"), t.text.caption) + t.space[3]
    }

    /// Obdelnik ikony uloh v liste (`None` = nic nebezi).
    pub fn tasks_icon(&self) -> Option<Rect> {
        self.bar_layout().tasks
    }

    /// Polozky listy: (okno, obdelnik).
    pub(crate) fn panel_items(&self) -> Vec<(u32, Rect)> {
        self.bar_layout().apps
    }

    /// Sirka odznaku duvery okna klienta.
    pub(crate) fn badge_width(&self, cl: &Client) -> u32 {
        let t = &self.style.theme;
        let mut w = t.space[2] + t.size.icon_s + t.space[2] + self.style.width(badge_name(cl), badge_text(t)) + t.space[3];
        if !cl.info.perms.is_empty() {
            let n = cl.info.perms.len() as u32;
            w += t.space[1] + t.space[2] + 1 + n * perm_icon_size(t) + (n - 1) * t.space[1];
        }
        w
    }

    /// Slozi obdelniky `rects` do platna.
    pub(crate) fn compose(&mut self, canvas: &mut Canvas<'_>, rects: &[Rect], now: u64) {
        let bar = self.bar_layout();
        let dialog_active = self.dialogs.active().is_some();
        // Deti vrstev kazdeho viditelneho okna jednou za snimek (ne pro
        // kazdy poskozeny obdelnik). Pod prihlasovaci obrazovkou zadna okna.
        let mut trees: Vec<(u32, Kids)> = Vec::new();
        let hidden = self.greeter_visible();
        let owners = self.owner_popovers(hidden);
        for &gid in self.z.iter().filter(|_| !hidden) {
            let Some(w) = self.windows.get(&gid) else { continue };
            // Okno i s popovery vlastnika (presahuji okno).
            let hit = |a: &Rect| rects.iter().any(|r| !a.intersect(r).is_empty());
            let pops = owners.iter().find(|o| o.0 == gid).is_some_and(|o| o.2.iter().any(hit));
            if !w.mapped || w.minimized || !(hit(&w.visual()) || pops) {
                continue;
            }
            let Some(cl) = self.clients.get(&w.client) else { continue };
            trees.push((gid, children(cl, w.local)));
        }
        let hover = self.input.hover;
        let popover = self.popover_pass(hidden);
        if rects.iter().any(|r| !r.intersect(&self.panel_visual()).is_empty()) {
            let t0 = self.prof.start();
            self.ensure_bar_blur(now);
            self.prof.add(Part::BarBlur, t0);
        }
        // Telo okna je nepruhledne (vypln `surface` pod obsahem), takze
        // jadro okna bez rohu zakryje vse pod nim: plocha a nizsi okna se
        // tam nekresli.
        let opaque = self.style.theme.color.surface >> 24 == 255 && !self.prof.no_cull;
        for r in rects {
            let (free, parts) = self.visible_parts(*r, &trees, opaque);
            let t0 = self.prof.start();
            for f in &free {
                canvas.set_clip(*f);
                self.desktop(canvas);
            }
            self.prof.add(Part::Desktop, t0);
            for ((gid, kids), pieces) in trees.iter().zip(&parts) {
                let w = &self.windows[gid];
                let cl = &self.clients[&w.client];
                let focused = self.focus == Some(*gid);
                for piece in pieces {
                    canvas.set_clip(*piece);
                    let t0 = self.prof.start();
                    draw_window_shadow(canvas, &mut self.style, w, focused);
                    self.prof.add(Part::Shadow, t0);
                    let t0 = self.prof.start();
                    draw_window_unshadowed(canvas, &mut self.style, w, cl, kids, focused, now, hover.and_then(|h| button_of(h, *gid)));
                    self.prof.add(Part::Window, t0);
                }
                // Popovery vlastnika okna s fokusem: hned nad nim (okna
                // vys, i jeho dialog, je prekresli svymi castmi).
                if let Some((_, okids, orects)) = owners.iter().find(|o| o.0 == *gid) {
                    for pr in orects {
                        let clip = r.intersect(pr);
                        if !clip.is_empty() {
                            canvas.set_clip(clip);
                            draw_children(canvas, cl, okids, 0, (w.rect.x, w.rect.y), clip, 255, 0);
                        }
                    }
                }
            }
            canvas.set_clip(*r);
            // Popover okna s fokusem: nad vsemi okny, okno ho neorezava.
            if let Some((g, kids, rects)) = &popover {
                let w = &self.windows[g];
                let cl = &self.clients[&w.client];
                for pr in rects {
                    let clip = r.intersect(pr);
                    if !clip.is_empty() {
                        draw_children(canvas, cl, kids, 0, (w.rect.x, w.rect.y), clip, 255, 0);
                    }
                }
                canvas.set_clip(*r);
            }
            // Vse nad okny pres mezibuffer (stejne jako GPU backend: jedno
            // prolnuti kazdeho pixelu); jen tam, kde vrstva neco kresli.
            let t0 = self.prof.start();
            for t in self.top_regions(dialog_active) {
                let part = r.intersect(&t);
                if !part.is_empty() {
                    self.top_via_scratch(canvas, part, &bar, dialog_active);
                }
            }
            self.prof.add(Part::Top, t0);
        }
        canvas.reset_clip();
    }

    /// Viditelne casti poskozeneho obdelniku `r`: (plocha, casti kazdeho
    /// okna z `trees` ve stejnem poradi). Okno dostane, co z jeho
    /// `visual()` nezakryji jadra oken nad nim; plocha zbytek. S `opaque`
    /// = false (pruhledny povrch motivu) nic nezakryva.
    pub(crate) fn visible_parts(&self, r: Rect, trees: &[(u32, Kids)], opaque: bool) -> (Vec<Rect>, Vec<Vec<Rect>>) {
        let mut free = alloc::vec![r];
        let mut parts = alloc::vec![Vec::new(); trees.len()];
        for (i, (gid, _)) in trees.iter().enumerate().rev() {
            let w = &self.windows[gid];
            let vis = w.visual();
            parts[i] = free.iter().map(|f| f.intersect(&vis)).filter(|f| !f.is_empty()).collect();
            if opaque {
                for k in window_core(w) {
                    if !k.is_empty() {
                        free = free.iter().flat_map(|f| subtract(*f, k)).collect();
                    }
                }
            }
        }
        (free, parts)
    }

    /// Popovery okna s fokusem: (okno, deti, obdelniky na obrazovce).
    pub(crate) fn popover_pass(&self, hidden: bool) -> Option<(u32, Kids, Vec<Rect>)> {
        let g = self.focus.filter(|_| !hidden)?;
        let w = self.windows.get(&g).filter(|w| w.mapped && !w.minimized)?;
        let rects = self.popover_rects(g);
        if rects.is_empty() {
            return None;
        }
        let cl = self.clients.get(&w.client)?;
        Some((g, popover_children(cl, w.local), rects))
    }

    /// Popovery vlastniku okna s fokusem (retezec `owner`, bez okna
    /// s fokusem): dialog, ktery vlastnikovi vzal fokus, nema schovat jeho
    /// popupy (Wine: bublina, rozbalena nabidka vlastnika). Kresli se hned
    /// nad vlastnikem, tedy pod jeho dialogem a okny nad nim.
    pub(crate) fn owner_popovers(&self, hidden: bool) -> Vec<(u32, Kids, Vec<Rect>)> {
        let mut out = Vec::new();
        let Some(f) = self.focus.filter(|_| !hidden) else { return out };
        let mut g = self.windows.get(&f).and_then(|w| w.owner);
        // Ochrana proti cyklu (owner.rs ho nedovoli, ale nepocitat s tim).
        for _ in 0..16 {
            let Some(o) = g.filter(|&o| o != f) else { break };
            let Some(w) = self.windows.get(&o) else { break };
            if w.mapped && !w.minimized {
                let rects = self.popover_rects(o);
                if let (false, Some(cl)) = (rects.is_empty(), self.clients.get(&w.client)) {
                    out.push((o, popover_children(cl, w.local), rects));
                }
            }
            g = w.owner;
        }
        out
    }

    /// Obdelniky popoveru okna `g` na obrazovce (viditelne korenove vrstvy
    /// s `popover`), oriznute na pracovni plochu: popover nevyjede pod listu
    /// ani mimo obrazovku (systemove vrstvy, dialogy a zamek jsou nad nim).
    pub(crate) fn popover_rects(&self, g: u32) -> Vec<Rect> {
        let Some(w) = self.windows.get(&g) else { return Vec::new() };
        let Some(cl) = self.clients.get(&w.client) else { return Vec::new() };
        let screen = self.work_area();
        cl.layers
            .values()
            .filter(|l| l.window == w.local && l.parent == 0 && l.props.popover && l.props.visible)
            .map(|l| Rect::new(w.rect.x + l.props.rect.x, w.rect.y + l.props.rect.y, l.props.rect.w, l.props.rect.h).intersect(&screen))
            .filter(|r| !r.is_empty())
            .collect()
    }

    /// Kde vrstva nad okny kresli: lista se stinem, kurzor, pri dialogu cela
    /// obrazovka.
    pub(crate) fn top_regions(&self, dialog_active: bool) -> Vec<Rect> {
        if dialog_active || self.greeter_visible() {
            return alloc::vec![self.screen()];
        }
        let mut v = alloc::vec![self.panel_visual(), self.cursor_rect()];
        v.extend(self.flyout_rect());
        v.extend(self.palette_visual());
        // Nabidka u maximalizace i se stinem.
        v.extend(self.max_menu_rect().map(|r| Rect::new(r.x - 24, r.y - 24, r.w + 48, r.h + 48)));
        v.extend(self.tip_rect());
        v.extend(self.toasts_visual());
        v
    }

    /// Nakresli vrstvu nad okny do mezibufferu a prolne ji do platna.
    fn top_via_scratch(&mut self, canvas: &mut Canvas<'_>, r: Rect, bar: &Bar, dialog_active: bool) {
        let r = r.intersect(&canvas.bounds());
        if r.is_empty() {
            return;
        }
        let (w, h) = (canvas.w, canvas.h);
        let mut scratch = core::mem::take(&mut self.top_scratch);
        scratch.resize((w * h) as usize, 0);
        {
            let mut tc = Canvas::new(&mut scratch, w, h, w).unwrap();
            tc.set_clip(r);
            tc.fill(r, 0);
            self.draw_top(&mut tc, bar, dialog_active);
        }
        if let Some(img) = Image::new(&scratch, w, h, w) {
            canvas.set_clip(r);
            canvas.draw_image(&img, r, r, 255, false);
        }
        self.top_scratch = scratch;
    }

    /// Plocha (obraz z tokenu, spocitany jednou pro velikost).
    pub(crate) fn desktop(&mut self, c: &mut Canvas<'_>) {
        let (w, h) = (self.w, self.h);
        let img = self.style.backdrop(w, h);
        if let Some(img) = Image::new(img, w, h, w) {
            let clip = c.clip();
            c.draw_image(&img, clip, clip, 255, true);
        }
    }

    /// Vse nad okny: lista, ztlumeni a systemovy dialog, kurzor.
    pub(crate) fn draw_top(&mut self, canvas: &mut Canvas<'_>, bar: &Bar, dialog_active: bool) {
        // Prihlaseni / zamceno: jen obrazovka systemu (bez listy), pripadny
        // dialog nad ni a kurzor.
        if self.greeter_visible() {
            let now = self.input_now;
            self.draw_greeter(canvas, now);
            if dialog_active {
                self.dialogs.draw(canvas, &mut self.style, self.w, self.h);
            }
            draw_cursor(canvas, self.input.px, self.input.py);
            return;
        }
        let info = BarInfo {
            windows: &self.windows,
            clients: &self.clients,
            focus: self.focus,
            tasks: self.tasks,
            status: &self.status,
            clock: self.clock.as_ref(),
            blur: self.bar_blur.as_deref(),
            hover: self.input.hover,
            launchable: &self.launchable,
            dnd: self.do_not_disturb(),
            look_icon: self.look_icon(),
            pressed: self.input.sys_press.map(|(k, ..)| k).filter(|&k| self.sys_pressed(k)),
            open: match self.flyout {
                Some(crate::flyout::Flyout::Net) => Some(0),
                Some(crate::flyout::Flyout::Volume) => Some(1),
                Some(crate::flyout::Flyout::Bell) => Some(2),
                Some(crate::flyout::Flyout::Look) => Some(3),
                _ => None,
            },
        };
        let t0 = self.prof.start();
        draw_bar(canvas, &mut self.style, bar, &info);
        #[cfg(feature = "dev-mode")]
        self.draw_dev_chip(canvas, bar);
        self.prof.add(Part::Bar, t0);
        if !dialog_active {
            self.draw_flyout(canvas, self.input_now);
            self.draw_max_menu(canvas);
            self.draw_palette(canvas);
        }
        // Oznameni (vrstva `notification` nad panely, pod systemovym dialogem).
        let t0 = self.prof.start();
        self.draw_toasts(canvas);
        self.prof.add(Part::Bar, t0);
        if dialog_active {
            // Ztlumeni vseho pod dialogem (jen system kresli nad nim).
            let t0 = self.prof.start();
            canvas.blend(self.screen(), self.style.theme.color.scrim);
            self.dialogs.draw(canvas, &mut self.style, self.w, self.h);
            self.prof.add(Part::Dialog, t0);
        }
        self.draw_pointer_notice(canvas);
        match self.input.cursor {
            crate::input::Cursor::Arrow => draw_cursor(canvas, self.input.px, self.input.py),
            crate::input::Cursor::Hidden => {}
            crate::input::Cursor::App(g) => self.draw_app_cursor(canvas, g),
            k => draw_resize_cursor(canvas, self.input.px, self.input.py, k),
        }
    }
}

fn button_of(h: Hover, gid: u32) -> Option<usize> {
    match h {
        Hover::Button(g, i) if g == gid => Some(i),
        _ => None,
    }
}

/// Styl casu v hodinach listy (`small` s vahou `body_strong`).
fn clock_time(t: &theme::Theme) -> TextStyle {
    TextStyle { weight: t.text.body_strong.weight, ..t.text.small }
}

fn small_strong(t: &theme::Theme) -> TextStyle {
    TextStyle { weight: t.text.body_strong.weight, ..t.text.small }
}

/// Text odznaku: `caption` s vahou `body_strong` (`.trust` v prototypu).
fn badge_text(t: &theme::Theme) -> TextStyle {
    TextStyle { weight: t.text.body_strong.weight, ..t.text.caption }
}

/// Sirka ramu okna a listy: ve vysokem kontrastu 2 px (prototyp
/// `[data-contrast="more"] .win { border-width: 2px }`), jinak 1 px.
fn frame_width(t: &theme::Theme) -> u32 {
    if t.mode == theme::Mode::Contrast { 2 } else { 1 }
}

/// Jmeno v odznaku: jmeno aplikace z manifestu (preda ho init, ne
/// aplikace). Systemovou aplikaci oznaci stit s fajfkou (okna.html,
/// soubory.html), ne slovo "Systém" (O15).
fn badge_name(cl: &Client) -> &str {
    &cl.info.name
}

/// Ikona opravneni v odznaku (`size.icon_xs`).
fn perm_icon_size(t: &theme::Theme) -> u32 {
    t.size.icon_xs
}

/// Deti vrstev okna: rodic -> (poradi, id) serazene.
pub(crate) type Kids = BTreeMap<u32, Vec<(i32, u32)>>;

pub(crate) fn children(cl: &Client, window: u32) -> Kids {
    let mut kids: Kids = BTreeMap::new();
    for (&id, l) in &cl.layers {
        // Popovery (korenove vrstvy) se kresli az nad vsemi okny.
        if l.window == window && !(l.parent == 0 && l.props.popover) {
            kids.entry(l.parent).or_default().push((l.props.order, id));
        }
    }
    for v in kids.values_mut() {
        v.sort();
    }
    kids
}

/// Deti pro kresleni popoveru okna: koren jen popovery, ostatni jako
/// [`children`] (jejich potomci).
pub(crate) fn popover_children(cl: &Client, window: u32) -> Kids {
    let mut kids = children(cl, window);
    let mut roots: Vec<(i32, u32)> = cl.layers.iter().filter(|(_, l)| l.window == window && l.parent == 0 && l.props.popover && l.props.visible).map(|(&id, l)| (l.props.order, id)).collect();
    roots.sort();
    kids.insert(0, roots);
    kids
}

/// Prostredni pas okna, ktery stin nemusi kreslit (prekryje ho telo).
pub(crate) fn shadow_skip(w: &Window) -> Rect {
    let o = w.outer();
    let r = w.radius();
    Rect::new(o.x, o.y + r as i32, o.w, o.h.saturating_sub(2 * r))
}

/// Stin okna podle stavu.
pub(crate) fn draw_window_shadow(c: &mut Canvas<'_>, st: &mut Style, w: &Window, focused: bool) {
    if !w.mode.has_frame() {
        return;
    }
    let sh = if focused { st.theme.shadow.window.clone() } else { st.theme.shadow.window_inactive.clone() };
    st.draw_shadow(c, &sh, w.outer(), w.radius(), shadow_skip(w));
}

#[allow(clippy::too_many_arguments)]
fn draw_window(c: &mut Canvas<'_>, st: &mut Style, w: &Window, cl: &Client, kids: &Kids, focused: bool, now: u64, hover: Option<usize>) {
    draw_window_shadow(c, st, w, focused);
    draw_window_unshadowed(c, st, w, cl, kids, focused, now, hover);
}

/// Okno bez stinu: telo a obsah, rohy zaoblene pres ulozene pozadi.
#[allow(clippy::too_many_arguments)]
fn draw_window_unshadowed(c: &mut Canvas<'_>, st: &mut Style, w: &Window, cl: &Client, kids: &Kids, focused: bool, now: u64, hover: Option<usize>) {
    let clip = c.clip();
    let o = w.outer();
    let rad = w.radius();
    let under = if rad > 0 { c.save_corners(o, rad) } else { Vec::new() };
    draw_window_body(c, st, w, cl, kids, focused, now, hover);
    c.set_clip(clip);
    if rad > 0 {
        c.restore_corners(o, rad, &under);
    }
}

/// Cele okno bez stinu jako ctverec (telo, titulek, obsah, prvky nad
/// obsahem, ram). Rohy zaobli volajici (CPU: `restore_corners`, GPU:
/// rohova zaplata).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_window_body(c: &mut Canvas<'_>, st: &mut Style, w: &Window, cl: &Client, kids: &Kids, focused: bool, now: u64, hover: Option<usize>) {
    let clip = c.clip();
    draw_window_below(c, st, w, cl, focused, hover);
    let content_clip = clip.intersect(&w.rect);
    if !content_clip.is_empty() {
        draw_children(c, cl, kids, 0, (w.rect.x, w.rect.y), content_clip, 255, 0);
        c.set_clip(clip);
    }
    draw_window_above(c, st, w, cl, now, hover, focused);
}

/// Cast okna pod obsahem aplikace: telo, titulek (pozadi, odznak, text,
/// tlacitka). Kresli se jako ctverec (bez stinu a zaobleni).
pub(crate) fn draw_window_below(c: &mut Canvas<'_>, st: &mut Style, w: &Window, cl: &Client, focused: bool, hover: Option<usize>) {
    let col = st.theme.color.clone();
    if w.mode.has_frame() {
        c.fill(w.outer(), col.surface);
    } else {
        c.fill(w.rect, col.surface);
    }
    if w.mode != Mode::Standard {
        return;
    }
    let t = w.title_bar().unwrap();
    let o = w.outer();
    // Titulek pres celou sirku okna (az k ramu).
    let bar = Rect::new(o.x, o.y, o.w, t.bottom() as u32 - o.y as u32);
    c.fill(bar, if focused { col.surface_2 } else { col.surface });
    let badge = draw_badge(c, st, w, cl, focused);
    draw_buttons(c, st, w, hover);
    // Jmeno: na stred mezi odznakem a tlacitky, zkracene.
    let th = &st.theme;
    let style = if focused { th.text.body_strong } else { TextStyle { weight: th.text.caption.weight, ..th.text.body_strong } };
    let gap = th.space[3] as i32;
    let b = w.buttons();
    let (left, right) = match w.side {
        guiproto::window::ButtonSide::Right => (badge.right() as i32 + gap, b[2].x - gap),
        guiproto::window::ButtonSide::Left => (b[2].right() as i32 + gap, badge.x - gap),
    };
    if right > left {
        let room = (right - left) as u32;
        let ink = if focused { col.ink } else { col.ink_muted };
        let face = st.face(style);
        let tw = face.width(&w.title, style.size).min(room);
        let x = left + (room - tw) as i32 / 2;
        let base = t.y + face.baseline_in(style.size, t.h);
        let old = c.clip();
        c.set_clip(old.intersect(&Rect::new(left, t.y, room, t.h)));
        face.draw_ellipsized(c, x, base, &w.title, style.size, ink, room);
        c.set_clip(old);
    }
}

/// Prvky nad obsahem: tlacitka a odznak oken bez titulku (vlastni titulek,
/// bez ramu), ztlumeni neodpovidajici aplikace, ram okna.
pub(crate) fn draw_window_above(c: &mut Canvas<'_>, st: &mut Style, w: &Window, cl: &Client, now: u64, hover: Option<usize>, focused: bool) {
    if w.mode.has_frame() {
        if w.mode != Mode::Standard {
            draw_buttons(c, st, w, hover);
            draw_badge(c, st, w, cl, focused);
        }
    } else if w.badge_until > now {
        let b = draw_badge(c, st, w, cl, focused);
        // Systemova cela obrazovka: tlacitko navratu vedle odznaku.
        if w.fs_saved.is_some() {
            let t = st.theme.clone();
            let s = t.text.small;
            let bw = st.width(crate::fullscreen::EXIT_LABEL, s) + 2 * t.space[4];
            let r = Rect::new(b.right() as i32 + t.space[3] as i32, b.y, bw, b.h);
            c.round_fill_bordered(r, r.h / 2, t.color.surface, t.color.border_strong, 1);
            let face = st.face(s);
            let base = r.y + face.baseline_in(s.size, r.h);
            face.draw(c, r.x + t.space[4] as i32, base, crate::fullscreen::EXIT_LABEL, s.size, t.color.ink);
        }
    }
    if cl.unresponsive {
        draw_hung(c, st, w, cl);
    }
    if w.mode.has_frame() {
        let col = st.theme.color.border;
        c.inner_ring(w.outer(), w.radius(), frame_width(&st.theme), col);
    }
}

/// Aplikace nereaguje: ztlumeny obsah a karta uprostred (kresli system).
fn draw_hung(c: &mut Canvas<'_>, st: &mut Style, w: &Window, cl: &Client) {
    let t = st.theme.clone();
    // Obsah na kryti `opacity.hung_content` = plocha s doplnkovym kryti pres nej.
    c.blend(w.rect, scale(t.color.surface, 255 - t.opacity.hung_content));
    let title = format!("{} nereaguje", cl.info.name);
    let note = format!("{} s bez odpovědi.", t.motion.unresponsive_ms / 1000);
    let (s1, s2) = (t.text.body_strong, t.text.small);
    let pad = t.space[5];
    let cw = (st.width(&title, s1).max(st.width(&note, s2)) + 2 * pad).min(w.rect.w);
    let ch = (s1.line + t.space[2] + s2.line + 2 * pad).min(w.rect.h);
    let card = Rect::new(w.rect.x + (w.rect.w as i32 - cw as i32) / 2, w.rect.y + (w.rect.h as i32 - ch as i32) / 2, cw, ch);
    let pop = t.shadow.popover.clone();
    st.draw_shadow(c, &pop, card, t.radius.card, Rect::default());
    c.round_fill_bordered(card, t.radius.card, t.color.surface, t.color.border_strong, 1);
    let mut y = card.y + pad as i32;
    for (s, style, ink) in [(&title, s1, t.color.ink), (&note, s2, t.color.ink_muted)] {
        let face = st.face(style);
        let tw = face.width(s, style.size) as i32;
        let base = y + face.baseline_in(style.size, style.line);
        face.draw(c, card.x + (card.w as i32 - tw) / 2, base, s, style.size, ink);
        y += style.line as i32 + t.space[2] as i32;
    }
}

/// Odznak duvery: stit, jmeno z manifestu (od initu), ikony opravneni.
/// Vraci jeho obdelnik.
fn draw_badge(c: &mut Canvas<'_>, st: &mut Style, w: &Window, cl: &Client, focused: bool) -> Rect {
    let mut t = st.theme.clone();
    if !focused {
        // Neaktivni okno: odznak s nizsi sytosti (prototyp `saturate(.6)`),
        // porad citelny.
        for c in [&mut t.color.trust_bg, &mut t.color.trust_ink, &mut t.color.trust_border] {
            *c = saturate(*c, 153);
        }
    }
    let style = badge_text(&t);
    let name = badge_name(cl);
    let mut bw = t.space[2] + t.size.icon_s + t.space[2] + st.width(name, style) + t.space[3];
    let n = cl.info.perms.len() as u32;
    let pi = perm_icon_size(&t);
    if n > 0 {
        bw += t.space[1] + t.space[2] + 1 + n * pi + (n - 1) * t.space[1];
    }
    let r = w.badge(bw);
    let col = &t.color;
    c.round_fill_bordered(r, t.radius.badge, col.trust_bg, col.trust_border, 1);
    // Obsah jen uvnitr (mimo vyhlazene rohy): v uzkem okne se jmeno zkrati
    // a ikony opravneni vynechaji.
    let old = c.clip();
    let inner = Rect::new(r.x + t.space[2] as i32, r.y, r.w.saturating_sub(t.space[2] + t.space[3]), r.h);
    c.set_clip(old.intersect(&inner));
    let is = t.size.icon_s;
    let mut x = inner.x;
    let shield = if cl.info.system { "shieldcheck" } else { "shield" };
    st.icons.draw(c, shield, x, r.y + (r.h as i32 - is as i32) / 2, is, t.icon_stroke, col.trust_ink);
    x += (is + t.space[2]) as i32;
    let perms_w = if n > 0 { t.space[1] + t.space[2] + 1 + n * pi + (n - 1) * t.space[1] } else { 0 };
    let room = (inner.right() as i32 - x).max(0) as u32;
    let fits = st.width(name, style) + perms_w <= room;
    let face = st.face(style);
    let base = r.y + face.baseline_in(style.size, r.h);
    x = face.draw_ellipsized(c, x, base, name, style.size, col.trust_ink, if fits { room } else { room.saturating_sub(perms_w.min(room / 2)) });
    if n > 0 && fits {
        x += t.space[1] as i32;
        // Svisla cara oddelujici opravneni.
        c.fill(Rect::new(x, r.y + 5, 1, r.h.saturating_sub(10)), col.trust_border);
        x += 1 + t.space[2] as i32;
        for p in &cl.info.perms {
            st.icons.draw(c, perm_icon(p), x, r.y + (r.h as i32 - pi as i32) / 2, pi, t.icon_stroke, col.trust_ink);
            x += (pi + t.space[1]) as i32;
        }
    }
    c.set_clip(old);
    r
}

/// Sytost barvy `k`/255 (0 = sede podle jasu, 255 = beze zmeny).
fn saturate(c: u32, k: u32) -> u32 {
    let (r, g, b) = (c >> 16 & 0xFF, c >> 8 & 0xFF, c & 0xFF);
    // Jas podle Rec. 709 (jako CSS filtr saturate).
    let l = (r * 54 + g * 183 + b * 19) >> 8;
    let m = |v: u32| (l * (255 - k) + v * k) / 255;
    (c & 0xFF00_0000) | m(r) << 16 | m(g) << 8 | m(b)
}

/// Tlacitka okna: minimalizovat, maximalizovat, zavrit (pilulky, ikona
/// vedlejsi barvou; pri najeti zvyraznene, zavrit cervene).
fn draw_buttons(c: &mut Canvas<'_>, st: &mut Style, w: &Window, hover: Option<usize>) {
    let t = st.theme.clone();
    let col = &t.color;
    let icons = ["close", if w.restore.is_some() { "restore" } else { "max" }, "min"];
    for (i, b) in w.buttons().iter().enumerate() {
        let hot = hover == Some(i);
        let (bg, ink) = match (hot, i) {
            (true, 0) => (Some(col.danger), col.on_danger),
            (true, _) => (Some(col.surface_3), col.ink),
            _ => (None, col.ink_muted),
        };
        if let Some(bg) = bg {
            c.round_fill(*b, b.w / 2, bg);
        } else if w.button_color != 0 {
            c.round_fill(*b, b.w / 2, w.button_color | 0xFF00_0000);
        }
        let s = t.size.icon_xs;
        st.icons.draw(c, icons[i], b.x + (b.w - s) as i32 / 2, b.y + (b.h - s) as i32 / 2, s, t.size.title_btn_stroke as f32, ink);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_children(
    c: &mut Canvas<'_>,
    cl: &Client,
    kids: &BTreeMap<u32, Vec<(i32, u32)>>,
    parent: u32,
    origin: (i32, i32),
    clip: Rect,
    opacity: u32,
    depth: u32,
) {
    if depth > 32 {
        return;
    }
    let Some(list) = kids.get(&parent) else { return };
    for &(_, id) in list {
        let Some(l) = cl.layers.get(&id) else { continue };
        draw_layer(c, cl, kids, id, l, origin, clip, opacity, depth);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_layer(
    c: &mut Canvas<'_>,
    cl: &Client,
    kids: &BTreeMap<u32, Vec<(i32, u32)>>,
    id: u32,
    l: &Layer,
    origin: (i32, i32),
    clip: Rect,
    opacity: u32,
    depth: u32,
) {
    let p = &l.props;
    if !p.visible || p.opacity == 0 {
        return;
    }
    let r = p.rect.offset(origin.0, origin.1);
    let op = opacity * p.opacity as u32 / 255;
    let vis = clip.intersect(&r);
    if !vis.is_empty() {
        c.set_clip(vis);
        match l.kind {
            LayerKind::Solid => c.blend(r, scale(p.color, op)),
            LayerKind::Buffer => {
                if let Some(b) = cl.buffers.get(&p.buffer)
                    && let Some(px) = b.px.pixels()
                    && let Some(img) = Image::new(px, b.w, b.h, b.stride)
                {
                    let src = if p.src.is_empty() { Rect::new(0, 0, b.w, b.h) } else { p.src };
                    c.draw_image(&img, src, r, op as u8, false);
                }
            }
            LayerKind::Group => {}
        }
    }
    let child_clip = if p.clip { clip.intersect(&r) } else { clip };
    if child_clip.is_empty() {
        return;
    }
    draw_children(c, cl, kids, id, (r.x.saturating_sub(p.scroll.0), r.y.saturating_sub(p.scroll.1)), child_clip, op, depth + 1);
}

/// Co lista potrebuje vedet o scene.
struct BarInfo<'a> {
    windows: &'a BTreeMap<u32, Window>,
    clients: &'a BTreeMap<crate::ClientId, Client>,
    focus: Option<u32>,
    tasks: (u32, u32),
    status: &'a BarStatus,
    clock: Option<&'a (String, String)>,
    /// Rozmazane pozadi pod listou (`None` = plna plocha).
    blur: Option<&'a [u32]>,
    hover: Option<Hover>,
    launchable: &'a [(String, String)],
    /// Nerusit (zvonek preskrtnuty).
    dnd: bool,
    /// Ikona vzhledu (slunce, mesic, kontrast).
    look_icon: &'static str,
    /// Drzeny prvek listy (stav "stisknuto", akce az pri pusteni).
    pressed: Option<crate::input::SysKey>,
    /// Ikona stavove oblasti s otevrenym panelem (0 sit, 1 zvuk, 2
    /// oznameni, 3 vzhled).
    open: Option<u8>,
}

fn draw_bar(c: &mut Canvas<'_>, st: &mut Style, bar: &Bar, info: &BarInfo<'_>) {
    let t = st.theme.clone();
    let col = &t.color;
    let p = bar.rect;
    let vis = c.clip();
    let shadow = t.shadow.bar.clone();
    let mut area = p;
    for s in &shadow {
        let e = s.blur as i32 + s.spread;
        area = area.union(&Rect::new(p.x + s.x - e, p.y + s.y - e, p.w + 2 * e as u32, p.h + 2 * e as u32));
    }
    if vis.intersect(&area).is_empty() {
        return;
    }
    // Pod telem listy stin nekreslit, kdyz ho prekryje nepruhledne
    // rozmazane pozadi (plocha a okna jsou nepruhledne): mimo zaoblene
    // radky je pokryti tvaru 255.
    let rad = t.radius.window.min(p.w / 2).min(p.h / 2);
    let skip = if info.blur.is_some() { Rect::new(p.x, p.y + rad as i32, p.w, p.h.saturating_sub(2 * rad)) } else { Rect::default() };
    st.draw_shadow(c, &shadow, p, t.radius.window, skip);
    if let Some(b) = info.blur {
        c.round_image(b, p, t.radius.window);
    }
    c.round_fill_bordered(p, t.radius.window, scale(col.surface, t.opacity.bar), col.border, frame_width(&t));
    let ctl = t.radius.control;
    // Spoustec.
    let il = t.size.icon_l;
    let l = bar.launcher;
    // Pozadi tlacitka listy: stisknuto (accent_soft) / najeti (surface_3).
    let state_bg = |c: &mut Canvas<'_>, r: Rect, k: crate::input::SysKey, hover: bool| {
        if info.pressed == Some(k) {
            c.round_fill(r, ctl, col.accent_soft);
        } else if hover {
            c.round_fill(r, ctl, col.surface_3);
        }
    };
    if info.pressed == Some(crate::input::SysKey::Launcher) || info.hover == Some(Hover::Launcher) {
        state_bg(c, l, crate::input::SysKey::Launcher, info.hover == Some(Hover::Launcher));
    }
    st.icons.draw(c, "apps", l.x + (l.w - il) as i32 / 2, l.y + (l.h - il) as i32 / 2, il, t.icon_stroke, col.ink);
    // Okna.
    for (k, (g, r)) in bar.apps.iter().enumerate() {
        let Some(w) = info.windows.get(g) else { continue };
        let group = bar.groups.get(k).map_or(&[][..], |v| v.as_slice());
        let active = info.focus == Some(*g) && !w.minimized;
        let attention = group.iter().any(|x| info.windows.get(x).is_some_and(|w| w.attention));
        {
            state_bg(c, *r, crate::input::SysKey::BarApp(k), info.hover == Some(Hover::BarApp(*g)));
        }
        let icon = info.clients.get(&w.client).map(|cl| app_icon(&cl.info.id)).unwrap_or("window");
        st.icons.draw(c, icon, r.x + (r.w - il) as i32 / 2, r.y + (r.h - il) as i32 / 2, il, t.icon_stroke, col.ink);
        // Znacka bezici / aktivni (pod ikonou).
        let s = &t.size;
        let (mw, mc) = if active { (s.bar_indicator_active_w, col.accent) } else { (s.bar_indicator_w, col.ink_faint) };
        let mh = s.bar_indicator_h;
        // Okno otevrene na pozadi: znacka v barve zvyrazneni a siroka.
        let (mw, mc) = if attention && !active { (s.bar_indicator_active_w, col.warn) } else { (mw, mc) };
        c.round_fill(Rect::new(r.x + (r.w - mw) as i32 / 2, r.bottom() as i32 - mh as i32 - 1, mw, mh), mh, mc);
        // Vic oken: pocet vpravo nahore.
        if group.len() > 1 {
            let cs = t.text.caption;
            let n = format!("{}", group.len());
            let face = st.face(cs);
            let tw = face.width(&n, cs.size) as i32;
            let d = Rect::new(r.right() as i32 - tw - t.space[2] as i32 - 2, r.y + 1, tw as u32 + t.space[2] + 2, cs.line);
            c.round_fill(d, d.h / 2, col.accent);
            let base = d.y + face.baseline_in(cs.size, d.h);
            face.draw(c, d.x + (t.space[2] / 2 + 1) as i32, base, &n, cs.size, col.on_accent);
        }
    }
    // Nespustene aplikace: ikona vedlejsi barvou, bez znacky.
    for (i, r) in &bar.launch {
        let Some((id, _)) = info.launchable.get(*i) else { continue };
        {
            state_bg(c, *r, crate::input::SysKey::BarLaunch(*i), info.hover == Some(Hover::BarLaunch(*i)));
        }
        st.icons.draw(c, app_icon(id), r.x + (r.w - il) as i32 / 2, r.y + (r.h - il) as i32 / 2, il, t.icon_stroke, col.ink_muted);
    }
    for &x in &bar.seps {
        let h = t.space[7];
        c.fill(Rect::new(x, p.y + (p.h - h) as i32 / 2, 1, h), col.border);
    }
    // Ulohy: ikona, pocet, stitek agenta.
    if let Some(r) = bar.tasks {
        let (run, agent) = info.tasks;
        {
            state_bg(c, r, crate::input::SysKey::Tasks, info.hover == Some(Hover::Tasks));
        }
        let im = t.size.icon_m;
        let mut x = r.x + t.space[4] as i32;
        st.icons.draw(c, "tasks", x, r.y + (r.h - im) as i32 / 2, im, t.icon_stroke, col.ink);
        x += (im + t.space[2]) as i32;
        let s = small_strong(&t);
        let face = st.face(s);
        let base = r.y + face.baseline_in(s.size, r.h);
        x = face.draw(c, x, base, &format!("{run}"), s.size, col.ink);
        if agent > 0 {
            x += t.space[2] as i32;
            let cw = t.space[3] + t.size.icon_s - 2 + t.space[1] + st.width(&format!("{agent}"), t.text.caption) + t.space[3];
            let chip_h = t.space[7] - 2;
            let chip = Rect::new(x, r.y + (r.h - chip_h) as i32 / 2, cw, chip_h);
            c.round_fill_bordered(chip, chip_h / 2, col.ai_soft, col.ai, 1);
            let is = t.size.icon_s - 2;
            st.icons.draw(c, "agent", chip.x + t.space[3] as i32, chip.y + (chip.h - is) as i32 / 2, is, t.icon_stroke, col.ai_ink);
            let cs = t.text.caption;
            let face = st.face(cs);
            let base = chip.y + face.baseline_in(cs.size, chip.h);
            face.draw(c, chip.x + (t.space[3] + is + t.space[1]) as i32, base, &format!("{agent}"), cs.size, col.ai_ink);
        }
    }
    // Sit, hlasitost, oznameni: ikona 20 px; bez spojeni nebo ztlumeno
    // vedlejsi barvou s preskrtnutim.
    let im = t.size.icon_m;
    let icons = [
        // Pevna sit (vsechny podporovane karty jsou zatim pevne; Wi-Fi
        // ovladac ani typ rozhrani v netproto jeste nejsou), bez karty
        // ikona site preskrtnuta.
        (
            bar.net,
            if info.status.net == Some(NetState::NoDevice) { "network" } else { "ethernet" },
            info.status.net.is_some_and(|n| n != NetState::Online),
        ),
        // Pri nahravani mikrofon v barve nebezpeci (a text v bublince).
        (bar.volume, if info.status.capturing.is_empty() { "volume" } else { "mic" }, info.status.volume.is_some_and(|(v, m)| m || v == 0) && info.status.capturing.is_empty()),
        (Some(bar.bell), if info.dnd { "bell-off" } else { "bell" }, false),
        (Some(bar.look), info.look_icon, false),
    ];
    for (k, (r, name, off)) in icons.into_iter().enumerate() {
        let Some(r) = r else { continue };
        if info.open == Some(k as u8) {
            // Otevreny panel: `surface_3` a podtrzeni `accent` (k cemu panel patri).
            c.round_fill(r, ctl, col.surface_3);
            let lh = t.size.focus_ring;
            c.fill(Rect::new(r.x + ctl as i32 / 2, r.bottom() as i32 - lh as i32, r.w.saturating_sub(ctl), lh), col.accent);
        }
        state_bg(c, r, crate::input::SysKey::BarIcon(k as u8), false);
        let recording = k == 1 && !info.status.capturing.is_empty();
        let ink = if off {
            col.ink_faint
        } else if recording {
            col.danger
        } else {
            col.ink
        };
        let (x, y) = (r.x + (r.w - im) as i32 / 2, r.y + (r.h - im) as i32 / 2);
        st.icons.draw(c, name, x, y, im, t.icon_stroke, ink);
        if off {
            // Preskrtnuti (stav nese i barva, ne jen barva).
            for d in 0..im as i32 {
                let k = t.size.focus_ring;
                c.fill(Rect::new(x + d, y + d, k, k), col.ink_muted);
            }
        }
    }
    // Umely vstup: pilulka agenta (barva jen pro agenta, vzdy s ikonou
    // a textem).
    if let Some(r) = bar.agent {
        c.round_fill_bordered(r, r.h / 2, col.ai_soft, col.ai, 1);
        let is = t.size.icon_s;
        st.icons.draw(c, "agent", r.x + t.space[4] as i32, r.y + (r.h - is) as i32 / 2, is, t.icon_stroke, col.ai_ink);
        let s = small_strong(&t);
        let face = st.face(s);
        let base = r.y + face.baseline_in(s.size, r.h);
        face.draw(c, r.x + (t.space[4] + is + t.space[2]) as i32, base, INJECT_LABEL, s.size, col.ai_ink);
    }
    // Vzdaleny pristup (SSH): pilulka v barve upozorneni s ikonou
    // a textem (stav nese text, ne jen barva).
    if let Some(r) = bar.remote {
        c.round_fill_bordered(r, r.h / 2, col.warn_soft, col.warn, 1);
        let is = t.size.icon_s;
        st.icons.draw(c, "terminal", r.x + t.space[4] as i32, r.y + (r.h - is) as i32 / 2, is, t.icon_stroke, col.ink);
        let s = small_strong(&t);
        let face = st.face(s);
        let base = r.y + face.baseline_in(s.size, r.h);
        face.draw(c, r.x + (t.space[4] + is + t.space[2]) as i32, base, &remote_label(&info.status.remote), s.size, col.ink);
    }
    // Hodiny: cas a pod nim datum, zarovnane vpravo.
    if let (Some(r), Some((time, date))) = (bar.clock, info.clock) {
        let (ts, ds) = (clock_time(&t), t.text.caption);
        // Dva radky s vyskou 1,15 x velikost (prototyp `.bar-clock`).
        let l1 = ts.size as i32 * 115 / 100;
        let l2 = ds.size as i32 * 115 / 100;
        let top = r.y + (r.h as i32 - l1 - l2) / 2;
        let right = r.right() as i32;
        let face = st.face(ts);
        let tw = face.width(time, ts.size) as i32;
        let b1 = top + face.baseline_in(ts.size, l1 as u32);
        face.draw(c, right - tw, b1, time, ts.size, col.ink);
        let face = st.face(ds);
        let dw = face.width(date, ds.size) as i32;
        let b2 = top + l1 + face.baseline_in(ds.size, l2 as u32);
        face.draw(c, right - dw, b2, date, ds.size, col.ink_muted);
    }
}

/// Kurzor zmeny velikosti: dvojita sipka (cerna s bilym obrysem), aktivni
/// bod uprostred. Body cerne cary vzhledem ke stredu.
pub(crate) fn resize_cursor_points(k: crate::input::Cursor) -> Vec<(i32, i32)> {
    use crate::input::Cursor;
    let mut v = Vec::new();
    // Vodorovna: drik a hroty na obou koncich.
    let horizontal = |v: &mut Vec<(i32, i32)>| {
        for t in -7..=7 {
            v.push((t, 0));
        }
        for k in 1..=3 {
            v.extend([(-7 + k, k), (-7 + k, -k), (7 - k, k), (7 - k, -k)]);
        }
    };
    // Sikma vlevo nahore - vpravo dole.
    let diag = |v: &mut Vec<(i32, i32)>| {
        for t in -6..=6 {
            v.push((t, t));
        }
        for k in 1..=3 {
            v.extend([(6 - k, 6), (6, 6 - k), (-6 + k, -6), (-6, -6 + k)]);
        }
    };
    match k {
        Cursor::Horizontal => horizontal(&mut v),
        Cursor::Vertical => {
            horizontal(&mut v);
            v.iter_mut().for_each(|p| *p = (p.1, p.0));
        }
        Cursor::DiagDown => diag(&mut v),
        Cursor::DiagUp => {
            diag(&mut v);
            v.iter_mut().for_each(|p| p.0 = -p.0);
        }
        Cursor::Arrow | Cursor::App(_) | Cursor::Hidden => {}
    }
    v
}

pub(crate) fn draw_resize_cursor(c: &mut Canvas<'_>, x: i32, y: i32, k: crate::input::Cursor) {
    let pts = resize_cursor_points(k);
    for &(dx, dy) in &pts {
        c.fill(Rect::new(x + dx - 1, y + dy - 1, 3, 3), gfx::rgb(255, 255, 255));
    }
    for &(dx, dy) in &pts {
        c.fill(Rect::new(x + dx, y + dy, 1, 1), gfx::rgb(0, 0, 0));
    }
}

pub(crate) fn draw_cursor(c: &mut Canvas<'_>, x: i32, y: i32) {
    for (row, line) in CURSOR.iter().enumerate() {
        for (col, ch) in line.bytes().enumerate() {
            let p = match ch {
                b'1' => gfx::rgb(0, 0, 0),
                b'2' => gfx::rgb(255, 255, 255),
                _ => continue,
            };
            c.fill(Rect::new(x + col as i32, y + row as i32, 1, 1), p);
        }
    }
}

/// Pasy okna bez rohovych ctvercu (GPU backend kresli obsah jen do nich).
/// `r` bez `k` jako nejvys ctyri obdelniky (pruh nad, pod, vlevo, vpravo).
pub(crate) fn subtract(r: Rect, k: Rect) -> Vec<Rect> {
    let i = r.intersect(&k);
    if i.is_empty() {
        return alloc::vec![r];
    }
    let mut out = Vec::with_capacity(4);
    let (rb, ib) = (r.bottom() as i32, i.bottom() as i32);
    let (rr, ir) = (r.right() as i32, i.right() as i32);
    if i.y > r.y {
        out.push(Rect::new(r.x, r.y, r.w, (i.y - r.y) as u32));
    }
    if ib < rb {
        out.push(Rect::new(r.x, ib, r.w, (rb - ib) as u32));
    }
    if i.x > r.x {
        out.push(Rect::new(r.x, i.y, (i.x - r.x) as u32, i.h));
    }
    if ir < rr {
        out.push(Rect::new(ir, i.y, (rr - ir) as u32, i.h));
    }
    out
}

pub(crate) fn window_core(w: &Window) -> [Rect; 3] {
    core_rects(w.outer(), w.radius())
}
