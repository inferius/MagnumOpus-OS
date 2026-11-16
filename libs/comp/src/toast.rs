//! Vyskakovaci oznameni (`docs/cs/oznameni.md`, `docs/cs/vzhled.md`
//! kapitola Oznameni): karty vpravo nahore, vyjedou (posun 24 px, 240 ms),
//! `Normal` po 6 s zmizi do centra oznameni, `High` a `Urgent` zustanou do
//! zavreni. Oznameni prichazi od sluzby `notifyd` (prohlizec relace,
//! compd ho preda pres [`Comp::notify_event`]); klik, krizek a odjeti
//! do centra vraci jako pozadavky [`Comp::take_notify_requests`].
//!
//! Vse, co kompozitor o oznamenich vi (i pro centrum oznameni v panelu
//! zvonku), je v [`Notices`]: [`Comp::notifications`],
//! [`Comp::notify_dismiss`], [`Comp::notify_invoke`],
//! [`Comp::notify_dismiss_all`], [`Comp::notify_set_dnd`].
//!
//! Nerusit: uzivatelske hlida notifyd (prizna `popup`), pri cele obrazovce
//! kompozitor nevyskakuje sam (`do_not_disturb`), krome `Urgent`.
//! Prihlasovaci a zamcena obrazovka oznameni nekresli.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::Canvas;
use guiproto::Rect;
use notifyproto::{Notification, Priority, View, ViewReply};

use crate::Comp;

/// Vyjeti karty.
pub const SLIDE: u64 = 240_000_000;
/// Posun pri vyjeti (px).
const SLIDE_PX: i32 = 24;
/// Jak dlouho je `Normal` videt.
pub const SHOW: u64 = 6_000_000_000;
/// Nejvic karet naraz (dalsi pockaji).
pub const MAX_SHOWN: usize = 3;
/// Radku tela na karte.
const BODY_LINES: usize = 2;

/// Karta na obrazovce.
#[derive(Debug, Clone)]
pub struct Toast {
    pub uid: u64,
    pub n: Notification,
    /// Kdy vyjela (0 = jeste ceka ve fronte).
    pub since: u64,
}

/// Stav oznameni v kompozitoru.
#[derive(Debug, Default)]
pub struct Notices {
    /// Vsechna oznameni (uzivatel, oznameni) v poradi vzniku.
    pub all: Vec<(u64, Notification)>,
    /// Karty k zobrazeni (prvnich [`MAX_SHOWN`] je videt).
    pub toasts: Vec<Toast>,
    /// Pozadavky do notifyd (uzivatel, pozadavek).
    pub out: Vec<(u64, View)>,
    /// Nerusit uzivatele od notifyd (konec, 0 = vypnuto).
    pub dnd: BTreeMap<u64, u64>,
    /// Snimek uzivatele se prave nacita (prvni cast smaze stary stav).
    loading: Vec<u64>,
}

/// Rozlozeni karty.
struct Layout {
    card: Rect,
    close: Rect,
    actions: Vec<Rect>,
}

impl Comp {
    /// Zmena od notifyd pro prohlizec uzivatele `uid`.
    pub fn notify_event(&mut self, uid: u64, r: ViewReply, now: u64) {
        match r {
            ViewReply::Snapshot { list, last } => {
                if !self.notices.loading.contains(&uid) {
                    // Novy odber (restart notifyd nebo kompozitoru): stary stav pryc.
                    self.notices.all.retain(|(u, _)| *u != uid);
                    self.toasts_damage();
                    self.notices.toasts.retain(|t| t.uid != uid);
                    self.notices.loading.push(uid);
                }
                for n in list {
                    self.notice_added(uid, n, now);
                }
                if last {
                    self.notices.loading.retain(|u| *u != uid);
                }
            }
            ViewReply::Added(n) => self.notice_added(uid, n, now),
            ViewReply::Removed { id, .. } => {
                self.notices.all.retain(|(_, n)| n.id != id);
                self.toasts_damage();
                self.notices.toasts.retain(|t| t.n.id != id);
                self.toasts_damage();
            }
            ViewReply::Dnd { until } => {
                self.notices.dnd.insert(uid, until);
            }
            ViewReply::Result { .. } => {}
        }
    }

    fn notice_added(&mut self, uid: u64, n: Notification, now: u64) {
        let id = n.id;
        match self.notices.all.iter_mut().find(|(_, x)| x.id == id) {
            Some(e) => e.1 = n.clone(),
            None => self.notices.all.push((uid, n.clone())),
        }
        // Nahrazeni (prubeh): karta, ktera je videt, se jen prekresli.
        if let Some(t) = self.notices.toasts.iter_mut().find(|t| t.n.id == id) {
            t.n = n;
            self.toasts_damage();
            return;
        }
        if !n.popup || n.replaced {
            return;
        }
        // Cela obrazovka: jen do centra (a uz nevyskoci), krome Urgent.
        if n.priority != Priority::Urgent && self.do_not_disturb() {
            self.notices.out.push((uid, View::Seen { id }));
            return;
        }
        self.notices.toasts.push(Toast { uid, n, since: 0 });
        self.toasts_start(now);
    }

    /// Karty, ktere se vejdou, zacnou vyjizdet.
    fn toasts_start(&mut self, now: u64) {
        for t in self.notices.toasts.iter_mut().take(MAX_SHOWN) {
            if t.since == 0 {
                t.since = now.max(1);
            }
        }
        self.toasts_damage();
    }

    /// Pozadavky do notifyd (compd je posle prohlizeci uzivatele).
    pub fn take_notify_requests(&mut self) -> Vec<(u64, View)> {
        core::mem::take(&mut self.notices.out)
    }

    /// Vsechna oznameni (centrum oznameni).
    pub fn notifications(&self) -> &[(u64, Notification)] {
        &self.notices.all
    }

    /// Karty na obrazovce (testy).
    pub fn toasts(&self) -> &[Toast] {
        &self.notices.toasts[..self.notices.toasts.len().min(MAX_SHOWN)]
    }

    fn uid_of(&self, id: u64) -> Option<u64> {
        self.notices.all.iter().find(|(_, n)| n.id == id).map(|(u, _)| *u)
    }

    /// Zavrit oznameni (krizek na karte nebo v centru).
    pub fn notify_dismiss(&mut self, id: u64, now: u64) {
        if let Some(uid) = self.uid_of(id) {
            self.notices.out.push((uid, View::Dismiss { id }));
        }
        self.toast_remove(id, now);
    }

    /// Klik na akci (`action` prazdne = na oznameni samotne).
    pub fn notify_invoke(&mut self, id: u64, action: &str, now: u64) {
        if let Some(uid) = self.uid_of(id) {
            self.notices.out.push((uid, View::Invoke { id, action: String::from(action) }));
        }
        self.toast_remove(id, now);
    }

    /// Vymazat vse uzivatele `uid` (centrum oznameni).
    pub fn notify_dismiss_all(&mut self, uid: u64, now: u64) {
        self.notices.out.push((uid, View::DismissAll));
        let ids: Vec<u64> = self.notices.toasts.iter().filter(|t| t.uid == uid).map(|t| t.n.id).collect();
        for id in ids {
            self.toast_remove(id, now);
        }
    }

    /// Nerusit uzivatele `uid` do casu (realny cas ns, 0 = vypnout).
    pub fn notify_set_dnd(&mut self, uid: u64, until: u64) {
        self.notices.out.push((uid, View::SetDnd { until }));
    }

    /// Nerusit uzivatele od notifyd (centrum oznameni, ikona).
    pub fn notify_dnd(&self, uid: u64) -> u64 {
        self.notices.dnd.get(&uid).copied().unwrap_or(0)
    }

    fn toast_remove(&mut self, id: u64, now: u64) {
        self.toasts_damage();
        self.notices.toasts.retain(|t| t.n.id != id);
        self.toasts_start(now);
    }

    /// Kdy se probudit: vyjizdeni (kazdy snimek) nebo konec zobrazeni.
    pub(crate) fn toast_due(&self, now: u64) -> Option<u64> {
        let mut t: Option<u64> = None;
        let mut at = |d: u64| t = Some(t.map_or(d, |x: u64| x.min(d)));
        for x in self.toasts() {
            if x.since == 0 {
                continue;
            }
            if now < x.since + SLIDE {
                at(now);
            }
            if x.n.priority == Priority::Normal || x.n.priority == Priority::Low {
                at(x.since + SHOW);
            }
        }
        t
    }

    /// Prubeh vyjeti (prekresleni) a konec zobrazeni `Normal`: karta zmizi
    /// do centra (`Seen`).
    pub(crate) fn toast_timer(&mut self, now: u64) {
        let mut gone = Vec::new();
        for x in self.toasts() {
            if x.since != 0 && x.n.priority <= Priority::Normal && now >= x.since + SHOW {
                gone.push((x.uid, x.n.id));
            }
        }
        if self.toasts().iter().any(|x| x.since != 0 && now < x.since + SLIDE + 20_000_000) {
            self.toasts_damage();
        }
        for (uid, id) in gone {
            self.notices.out.push((uid, View::Seen { id }));
            self.toast_remove(id, now);
        }
    }

    /// Prekreslit oblast karet (cely sloupec, i posunuty pri vyjeti).
    fn toasts_damage(&mut self) {
        if let Some(r) = self.toasts_column() {
            self.damage(r);
        }
    }

    /// Kde karty kresli (vrstva nad okny, `render::top_regions`).
    pub(crate) fn toasts_visual(&self) -> Option<Rect> {
        if self.toasts().is_empty() { None } else { self.toasts_column() }
    }

    /// Sloupec karet vpravo nahore (pro poskozeni).
    fn toasts_column(&self) -> Option<Rect> {
        if self.notices.toasts.is_empty() {
            return None;
        }
        let t = &self.style.theme;
        let w = t.size.notification_w.min(self.w) + SLIDE_PX as u32;
        let x = self.w.saturating_sub(w + t.space[5]) as i32;
        Some(Rect::new(x, 0, w + t.space[5], self.h.saturating_sub(self.panel_rect().h)))
    }

    /// Rozlozeni karty `i` (od shora) s vyskou podle obsahu.
    fn toast_layout(&self, i: usize) -> Option<Layout> {
        let t = &self.style.theme;
        let shown = self.toasts();
        let w = t.size.notification_w.min(self.w.saturating_sub(2 * t.space[5]));
        let pad = t.space[4];
        let mut y = t.space[5] as i32;
        for (k, x) in shown.iter().enumerate() {
            let lines = self.body_lines(&x.n, w - 2 * pad).len();
            let actions = if x.n.actions.is_empty() { 0 } else { t.text.small.line + 2 * t.space[2] + t.space[3] };
            let h = 2 * pad + t.text.small.line + t.space[1] + t.text.body_strong.line + lines as u32 * t.text.body.line + actions;
            let card = Rect::new(self.w as i32 - w as i32 - t.space[5] as i32, y, w, h);
            if k == i {
                let cs = t.text.small.line;
                let close = Rect::new(card.x + w as i32 - pad as i32 - cs as i32, card.y + pad as i32, cs, cs);
                let mut acts = Vec::new();
                let bh = t.text.small.line + 2 * t.space[2];
                let mut ax = card.x + pad as i32;
                for a in &x.n.actions {
                    let bw = self.style.width(&a.label, t.text.small) + 2 * t.space[4];
                    acts.push(Rect::new(ax, card.y + h as i32 - pad as i32 - bh as i32, bw, bh));
                    ax += bw as i32 + t.space[2] as i32;
                }
                return Some(Layout { card, close, actions: acts });
            }
            y += h as i32 + t.space[3] as i32;
        }
        None
    }

    /// Rozlozeni karty (krizek, akce, karta) pro testy.
    #[cfg(test)]
    pub(crate) fn toast_layout_for_test(&self, i: usize) -> (Rect, Vec<Rect>, Rect) {
        let l = self.toast_layout(i).expect("karta");
        (l.close, l.actions, l.card)
    }

    /// Radky tela (nejvic [`BODY_LINES`], zalomene podle sirky).
    fn body_lines(&self, n: &Notification, w: u32) -> Vec<String> {
        let s = self.style.theme.text.body;
        let mut out: Vec<String> = Vec::new();
        for para in n.body.lines() {
            let mut line = String::new();
            for word in para.split(' ') {
                let cand = if line.is_empty() { String::from(word) } else { alloc::format!("{line} {word}") };
                if self.style.width(&cand, s) > w && !line.is_empty() {
                    out.push(core::mem::replace(&mut line, String::from(word)));
                } else {
                    line = cand;
                }
            }
            out.push(line);
            if out.len() >= BODY_LINES {
                break;
            }
        }
        out.truncate(BODY_LINES);
        out.retain(|l| !l.is_empty());
        out
    }

    /// Klik do karty; `true` = kliknuti patrilo oznameni.
    pub(crate) fn toast_click(&mut self, x: i32, y: i32, now: u64) -> bool {
        for i in 0..self.toasts().len() {
            let Some(l) = self.toast_layout(i) else { continue };
            if !l.card.contains(x, y) {
                continue;
            }
            let n = self.toasts()[i].n.clone();
            if l.close.contains(x, y) {
                self.notify_dismiss(n.id, now);
            } else if let Some(k) = l.actions.iter().position(|r| r.contains(x, y)) {
                self.notify_invoke(n.id, &n.actions[k].id, now);
            } else {
                self.notify_invoke(n.id, "", now);
            }
            return true;
        }
        false
    }

    /// Kresli karty (vrstva oznameni nad listou, pod dialogy).
    pub(crate) fn draw_toasts(&mut self, c: &mut Canvas<'_>) {
        let now = self.input_now;
        let t = self.style.theme.clone();
        let pad = t.space[4] as i32;
        for i in 0..self.toasts().len() {
            let (Some(l), x) = (self.toast_layout(i), self.toasts()[i].clone()) else { continue };
            if x.since == 0 {
                continue;
            }
            // Vyjeti zprava: zbyvajici posun klesa s kvadratickym zpomalenim.
            let p = (now.saturating_sub(x.since).min(SLIDE) * 1000 / SLIDE) as i32;
            let off = SLIDE_PX * (1000 - p) * (1000 - p) / 1_000_000;
            let card = Rect::new(l.card.x + off, l.card.y, l.card.w, l.card.h);
            let fw = if t.mode == theme::Mode::Contrast { 2 } else { 1 };
            c.round_fill_bordered(card, t.radius.card, t.color.surface, t.color.border_strong, fw);
            // Agent AI: levy pruh v barve `ai`.
            if x.n.app_id.starts_with("org.magnumopus.agent") {
                c.fill(Rect::new(card.x, card.y + t.radius.card as i32, 3, card.h.saturating_sub(2 * t.radius.card)), t.color.ai);
            }
            let inner = card.w.saturating_sub(2 * pad as u32);
            let mut y = card.y + pad;
            let s = t.text.small;
            let face = self.style.face(s);
            let base = y + face.baseline_in(s.size, s.line);
            face.draw_ellipsized(c, card.x + pad, base, &x.n.app_name, s.size, t.color.ink_muted, inner.saturating_sub(s.line + t.space[2]));
            draw_close(c, Rect::new(l.close.x + off, l.close.y, l.close.w, l.close.h), t.color.ink_muted);
            y += s.line as i32 + t.space[1] as i32;
            let b = t.text.body_strong;
            let face = self.style.face(b);
            let base = y + face.baseline_in(b.size, b.line);
            face.draw_ellipsized(c, card.x + pad, base, &x.n.title, b.size, t.color.ink, inner);
            y += b.line as i32;
            let body = t.text.body;
            for line in self.body_lines(&x.n, inner) {
                let face = self.style.face(body);
                let base = y + face.baseline_in(body.size, body.line);
                face.draw_ellipsized(c, card.x + pad, base, &line, body.size, t.color.ink_muted, inner);
                y += body.line as i32;
            }
            for (k, r) in l.actions.iter().enumerate() {
                let r = Rect::new(r.x + off, r.y, r.w, r.h);
                let primary = k == 0;
                let (fill, ink) = if primary { (t.color.accent, t.color.on_accent) } else { (t.color.surface_2, t.color.ink) };
                c.round_fill(r, t.radius.control, fill);
                let face = self.style.face(s);
                let base = r.y + face.baseline_in(s.size, r.h);
                face.draw_ellipsized(c, r.x + t.space[4] as i32, base, &x.n.actions[k].label, s.size, ink, r.w.saturating_sub(2 * t.space[4]));
            }
        }
    }
}

/// Krizek zavreni (dve uhlopricky v ctverci).
fn draw_close(c: &mut Canvas<'_>, r: Rect, color: u32) {
    let n = r.w.min(r.h) as i32 / 2;
    let (cx, cy) = (r.x + r.w as i32 / 2, r.y + r.h as i32 / 2);
    for d in -n / 2..=n / 2 {
        c.fill(Rect::new(cx + d, cy + d, 2, 2), color);
        c.fill(Rect::new(cx + d, cy - d, 2, 2), color);
    }
}
