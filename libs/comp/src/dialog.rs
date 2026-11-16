//! Systemove dialogy (opravneni, potvrzeni akce, heslo) a bezpecna
//! obrazovka. Kresli je jen kompozitor ve vrstve nad ztlumenymi okny;
//! prijimaji jen fyzicky vstup (umely vstup se ignoruje), takze dialog
//! nepotvrdi automat ani aplikace s capability "vkladat vstup".

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::Canvas;
use gfx::text::Text;
use guiproto::Rect;
use guiproto::control::{DialogKind, DialogReply, DialogRequest};
use inputproto::Action;
use inputproto::code::*;

use theme::{TextStyle, Theme};

use crate::style::Style;
use crate::{Comp, Out, PeerId};

/// Upozorneni pod textem dialogu.
const GUARD: &str = "Systémový dialog přijímá jen fyzický vstup. Bezpečná zkratka Ctrl+Alt+Del ukáže vždy pravý.";

/// Volba bezpecne obrazovky: zamknout relaci.
const LOCK: &str = "Zamknout";

/// Kdo dialog vyvolal (komu patri odpoved).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// Sluzba s capability `dialog`.
    Peer(PeerId),
    /// Potvrzeni volani akce (vnitrni id volani).
    Action(u32),
    /// Bezpecna obrazovka po bezpecne zkratce.
    Secure,
    /// Nove vstupni zarizeni ceka na overeni (id zdroje v `inputd`).
    Device(u16),
    /// Vypnuti (`false`) nebo restart (`true`) z palety prikazu.
    Power(bool),
}

#[derive(Clone)]
pub struct Dialog {
    pub owner: Owner,
    pub id: u32,
    pub kind: DialogKind,
    pub app: String,
    pub text: String,
    pub options: Vec<String>,
    pub sel: usize,
    /// Vychozi volba (primarni tlacitko): bezpecna volba, na ktere dialog
    /// zacina; nastavi `open_dialog` z `sel`.
    pub primary: usize,
    pub password: String,
    /// Zaskrtavatko (jen `Elevate`: dalsich 5 minut nepotvrzovat).
    pub flag: Option<bool>,
}

impl core::fmt::Debug for Dialog {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Heslo se nevypisuje (ani delka).
        f.debug_struct("Dialog").field("owner", &self.owner).field("id", &self.id).field("kind", &self.kind).field("app", &self.app).finish_non_exhaustive()
    }
}

impl Drop for Dialog {
    fn drop(&mut self) {
        crate::greeter::zero(&mut self.password);
    }
}

/// Nejvic radku textu dialogu (dialog roste do vysky obrazovky).
pub const MAX_LINES: usize = 24;

/// Jak dlouho po zobrazeni dialog vstup ignoruje. Pocita se od chvile,
/// kdy displej ukazal prvni snimek s dialogem (ne od pozadavku): pri
/// pomalem skladani na CPU (snimek 0,6-1,4 s) by jinak ochrana probehla
/// driv, nez dialog uzivatel vubec videl.
pub const INPUT_DELAY: u64 = 300_000_000;

/// Zobrazeni aktivniho dialogu na displeji.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Shown {
    /// Na displeji (cas v `since`), nebo zadny dialog.
    #[default]
    Yes,
    /// Zmenil se, ceka na slozeni snimku.
    Waiting,
    /// Slozen ve snimku, ktery ceka na zobrazeni.
    Composed,
}

/// Nejvic cekajicich dialogu (dalsi se odmitnou).
const MAX_QUEUE: usize = 32;
const MAX_PASSWORD: usize = 256;

#[derive(Default)]
pub struct Dialogs {
    queue: VecDeque<Dialog>,
    /// Od kdy je aktivni dialog videt (vstup tesne po zobrazeni se
    /// ignoruje - ochrana proti podstrceni kliku). Dokud neni zobrazen,
    /// `u64::MAX`.
    pub(crate) since: u64,
    pub(crate) shown: Shown,
    /// Bezpecnostni fraze uzivatele (aplikace ji nezna).
    phrase: String,
}

/// Rozvrzeni dialogu na obrazovce.
pub struct Layout {
    pub frame: Rect,
    pub buttons: Vec<Rect>,
    pub field: Option<Rect>,
    /// Zaskrtavatko (jen `Elevate`).
    pub check: Option<Rect>,
    pub lines: Vec<String>,
    /// Pruh "Systém" s frazi.
    band: Rect,
    /// Nadpis (zakladna), blok aplikace, prvni radek textu, upozorneni.
    title_y: i32,
    who: Option<Rect>,
    text_y: i32,
    guard: Vec<String>,
    guard_y: i32,
}

fn heading(d: &Dialog) -> String {
    String::from(match (d.owner, d.kind) {
        (Owner::Secure, _) => "Bezpečná obrazovka systému",
        (_, DialogKind::Permission) => "Povolit oprávnění?",
        (_, DialogKind::Confirm) => "Potvrzení",
        (_, DialogKind::Password) => "Potvrď heslem",
        (_, DialogKind::Elevate) if d.app.is_empty() => "Změna systému",
        (_, DialogKind::Elevate) => return alloc::format!("{} chce změnit systém", d.app),
    })
}

/// Zavaznost dialogu: dlazdice ikony u nadpisu (`.sev` v dialogy.html).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sev {
    Info,
    Warn,
    Danger,
}

/// Ikona a zavaznost dialogu.
fn severity(d: &Dialog) -> (&'static str, Sev) {
    match (d.owner, d.kind) {
        (Owner::Secure, _) => ("shieldcheck", Sev::Info),
        (Owner::Power(_), _) => ("power", Sev::Warn),
        (Owner::Device(_), _) => ("keyboard", Sev::Danger),
        (Owner::Action(_), _) => ("agent", Sev::Warn),
        (_, DialogKind::Permission) => ("shield", Sev::Info),
        (_, DialogKind::Password) => ("key", Sev::Info),
        (_, DialogKind::Elevate) => ("settings", Sev::Warn),
        (_, DialogKind::Confirm) => ("alert", Sev::Warn),
    }
}

/// Popisek zaskrtavatka rezimu spravy.
const ADMIN_MODE: &str = "Dalších 5 minut úkony správy nepotvrzovat";

/// Ma dialog pole pro tajemstvi?
fn has_field(k: DialogKind) -> bool {
    matches!(k, DialogKind::Password | DialogKind::Elevate)
}

/// Styl popisku pruhu (`small`, tucne).
fn band_style(t: &Theme) -> TextStyle {
    TextStyle { weight: t.text.body_strong.weight, ..t.text.small }
}

impl Dialogs {
    pub fn active(&self) -> Option<&Dialog> {
        self.queue.front()
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn secure_active(&self) -> bool {
        self.queue.iter().any(|d| d.owner == Owner::Secure)
    }

    /// Odebere dialogy vlastniku podle `f`; `true` = zmizel aktivni.
    pub(crate) fn remove_where(&mut self, f: impl Fn(Owner) -> bool) -> bool {
        let front = self.active().is_some_and(|d| f(d.owner));
        self.queue.retain(|d| !f(d.owner));
        front
    }

    pub fn next_wakeup(&self) -> Option<u64> {
        None
    }

    /// Rozvrzeni aktivniho dialogu (`design/plocha.html`, `.sysdlg`).
    pub fn layout(&self, st: &Style, sw: u32, sh: u32) -> Option<Layout> {
        let d = self.active()?;
        let t = &st.theme;
        let sp = &t.space;
        let w = t.size.dialog_w.min(sw.saturating_sub(2 * sp[4]));
        let inner = w.saturating_sub(2 * sp[6]);
        let body = t.text.body;
        let lines = wrap(st.face_ref(body), &d.text, body.size, inner);
        let guard = wrap(st.face_ref(t.text.caption), GUARD, t.text.caption.size, inner.saturating_sub(t.size.icon_s + sp[2]));
        let band_h = 2 * sp[3] + t.text.small.line + 1;
        // Svisle: pruh, obsah (nadpis, aplikace, text, pole), upozorneni, tlacitka.
        let mut y = band_h + sp[6];
        let title_y = y as i32;
        // Nadpis vedle dlazdice zavaznosti (vyska `bar_icon`).
        y += t.text.title.line.max(t.size.bar_icon) + sp[3];
        let who_y = if d.app.is_empty() {
            None
        } else {
            y += sp[4];
            let at = y;
            y += t.size.bar_icon + 2 * sp[3] + sp[4];
            Some(at)
        };
        let text_y = y as i32;
        y += lines.len() as u32 * body.line;
        let field_y = if has_field(d.kind) {
            y += sp[4];
            let at = y;
            y += t.size.control_h;
            Some(at)
        } else {
            None
        };
        let check_y = if d.flag.is_some() {
            y += sp[3];
            let at = y;
            y += t.size.icon_m.max(t.text.body.line);
            Some(at)
        } else {
            None
        };
        y += sp[5];
        let guard_y = y as i32;
        y += guard.len() as u32 * t.text.caption.line + sp[4];
        let buttons_y = y;
        y += t.size.control_h + sp[6];
        let h = y.min(sh);
        let x = (sw as i32 - w as i32) / 2;
        let top = (sh as i32 - h as i32) / 2;
        let frame = Rect::new(x, top, w, h);
        let mut buttons = Vec::new();
        let mut bx = x + w as i32 - sp[6] as i32;
        for o in d.options.iter().rev() {
            let bw = st.width(o, t.text.body) + 2 * sp[5];
            bx -= bw as i32;
            buttons.push(Rect::new(bx, top + buttons_y as i32, bw, t.size.control_h));
            bx -= sp[3] as i32;
        }
        buttons.reverse();
        let field = field_y.map(|fy| Rect::new(x + sp[6] as i32, top + fy as i32, inner, t.size.control_h));
        let check = check_y.map(|cy| Rect::new(x + sp[6] as i32, top + cy as i32, inner, t.size.icon_m.max(t.text.body.line)));
        let who = who_y.map(|wy| Rect::new(x + sp[6] as i32, top + wy as i32, inner, t.size.bar_icon + 2 * sp[3]));
        Some(Layout {
            frame,
            buttons,
            field,
            check,
            lines,
            band: Rect::new(x, top, w, band_h),
            title_y: top + title_y,
            who,
            text_y: top + text_y,
            guard,
            guard_y: top + guard_y,
        })
    }

    pub fn draw(&self, c: &mut Canvas<'_>, st: &mut Style, sw: u32, sh: u32) {
        let Some(l) = self.layout(st, sw, sh) else { return };
        let d = self.active().unwrap();
        let t = st.theme.clone();
        let col = &t.color;
        let sp = &t.space;
        let f = l.frame;
        let rad = t.radius.window;
        let shadow = t.shadow.dialog.clone();
        st.draw_shadow(c, &shadow, f, rad, Rect::default());
        // Ram 2 px barvou duvery: jen system smi tuto barvu pouzit.
        c.round_fill_bordered(f, rad, col.surface, col.trust_border, 2);
        let old = c.clip();
        // Pruh "Systém": zaobleny jen nahore (dolni cast orezana).
        let band = Rect::new(f.x + 2, f.y + 2, f.w - 4, l.band.h - 2);
        c.set_clip(old.intersect(&band));
        c.round_fill(Rect::new(band.x, band.y, band.w, band.h + rad), rad - 2, col.trust_bg);
        c.set_clip(old);
        c.fill(Rect::new(band.x, band.bottom() as i32 - 1, band.w, 1), col.trust_border);
        let bs = band_style(&t);
        let is = t.size.icon_s;
        let icon_y = band.y + (band.h as i32 - 1 - is as i32) / 2;
        let mut x = band.x + sp[5] as i32;
        st.icons.draw(c, "shieldcheck", x, icon_y, is, t.icon_stroke, col.trust_ink);
        x += (is + sp[3]) as i32;
        let face = st.face(bs);
        let base = band.y + face.baseline_in(bs.size, band.h - 1);
        face.draw(c, x, base, "Systém", bs.size, col.trust_ink);
        // Fraze vpravo: zna ji jen system a uzivatel.
        let label = "tvá fráze: ";
        let phrase = if self.phrase.is_empty() { String::from("(nenastavena)") } else { self.phrase.clone() };
        let ps = TextStyle { weight: t.text.caption.weight, ..t.text.small };
        let pb = TextStyle { weight: t.text.display.weight, ..t.text.small };
        let pw = st.width(label, ps) + st.width(&phrase, pb);
        let mut px = band.right() as i32 - sp[5] as i32 - pw as i32;
        st.icons.draw(c, "key", px - (is + sp[2]) as i32, icon_y, is, t.icon_stroke, col.trust_ink);
        px = st.face(ps).draw(c, px, base, label, ps.size, col.trust_ink);
        st.face(pb).draw(c, px, base, &phrase, pb.size, col.trust_ink);
        // Dlazdice zavaznosti a nadpis vedle ni.
        let (icon, sev) = severity(d);
        let (bg, ink) = match sev {
            Sev::Info => (col.accent_soft, col.accent_ink),
            Sev::Warn => (col.warn_soft, col.warn),
            Sev::Danger => (col.danger_soft, col.danger),
        };
        let tile_s = t.size.bar_icon;
        let tile = Rect::new(f.x + sp[6] as i32, l.title_y, tile_s, tile_s);
        c.round_fill(tile, t.radius.control, bg);
        let il = t.size.icon_l;
        st.icons.draw(c, icon, tile.x + (tile_s - il) as i32 / 2, tile.y + (tile_s - il) as i32 / 2, il, t.icon_stroke, ink);
        let ts = t.text.title;
        let face = st.face(ts);
        let base = l.title_y + (tile_s as i32 - ts.line as i32) / 2 + face.baseline_in(ts.size, ts.line);
        face.draw(c, tile.right() as i32 + sp[4] as i32, base, &heading(d), ts.size, col.ink);
        // Aplikace, ktera se pta.
        if let Some(w) = l.who {
            c.round_fill(w, t.radius.control, col.surface_2);
            let tile = Rect::new(w.x + sp[3] as i32, w.y + sp[3] as i32, t.size.bar_icon, t.size.bar_icon);
            c.round_fill(tile, t.radius.control, col.accent_soft);
            let il = t.size.icon_l;
            st.icons.draw(c, "window", tile.x + (tile.w - il) as i32 / 2, tile.y + (tile.h - il) as i32 / 2, il, t.icon_stroke, col.accent_ink);
            let s = t.text.body_strong;
            let face = st.face(s);
            let base = w.y + face.baseline_in(s.size, w.h);
            let room = w.w.saturating_sub(tile.w + 3 * sp[3]);
            face.draw_ellipsized(c, tile.right() as i32 + sp[3] as i32, base, &d.app, s.size, col.ink, room);
        }
        // Text.
        let bs = t.text.body;
        let face = st.face(bs);
        let mut y = l.text_y;
        for line in &l.lines {
            face.draw(c, f.x + sp[6] as i32, y + face.baseline_in(bs.size, bs.line), line, bs.size, col.ink);
            y += bs.line as i32;
        }
        if let Some(fr) = l.field {
            c.round_fill_bordered(fr, t.radius.control, col.surface, col.border_strong, 1);
            let dots: String = d.password.chars().map(|_| '\u{2022}').collect();
            let face = st.face(bs);
            face.draw(c, fr.x + sp[4] as i32, fr.y + face.baseline_in(bs.size, fr.h), &dots, bs.size, col.ink);
            if d.password.is_empty() && d.kind == DialogKind::Elevate {
                face.draw(c, fr.x + sp[4] as i32, fr.y + face.baseline_in(bs.size, fr.h), "Heslo nebo PIN", bs.size, col.ink_faint);
            }
            if d.sel < d.options.len() {
                c.round_stroke(Rect::new(fr.x - 2, fr.y - 2, fr.w + 4, fr.h + 4), t.radius.control + 2, t.size.focus_ring, col.focus);
            }
        }
        if let (Some(cr), Some(on)) = (l.check, d.flag) {
            let bx = t.size.icon_m;
            let b = Rect::new(cr.x, cr.y + (cr.h as i32 - bx as i32) / 2, bx, bx);
            if on {
                c.round_fill(b, t.radius.control, col.accent);
                st.icons.draw(c, "check", b.x, b.y, bx, t.icon_stroke, col.on_accent);
            } else {
                c.round_fill_bordered(b, t.radius.control, col.surface, col.border_strong, 1);
            }
            if d.sel == d.options.len() {
                c.round_stroke(Rect::new(b.x - 3, b.y - 3, b.w + 6, b.h + 6), t.radius.control + 3, t.size.focus_ring, col.focus);
            }
            let face = st.face(bs);
            face.draw(c, b.right() as i32 + sp[3] as i32, cr.y + face.baseline_in(bs.size, cr.h), ADMIN_MODE, bs.size, col.ink);
        }
        // Upozorneni: jen fyzicky vstup.
        let cs = t.text.caption;
        let mut y = l.guard_y;
        st.icons.draw(c, "info", f.x + sp[6] as i32, y, is, t.icon_stroke, col.ink_muted);
        let face = st.face(cs);
        for g in &l.guard {
            face.draw(c, f.x + (sp[6] + is + sp[2]) as i32, y + face.baseline_in(cs.size, cs.line), g, cs.size, col.ink_muted);
            y += cs.line as i32;
        }
        // Tlacitka: pilulky; vychozi (bezpecna volba) primarni `accent`,
        // vybrane ma ramecek fokusu.
        for (i, (b, o)) in l.buttons.iter().zip(&d.options).enumerate() {
            let primary = i == d.primary;
            if primary {
                c.round_fill(*b, b.h / 2, col.accent);
            } else {
                c.round_fill_bordered(*b, b.h / 2, col.surface, col.border_strong, 1);
            }
            if i == d.sel {
                let (fr, off) = (t.size.focus_ring, t.size.focus_offset);
                let ring = Rect::new(b.x - (fr + off) as i32, b.y - (fr + off) as i32, b.w + 2 * (fr + off), b.h + 2 * (fr + off));
                c.round_stroke(ring, ring.h / 2, fr, col.focus);
            }
            let s = if primary { t.text.body_strong } else { t.text.body };
            let face = st.face(s);
            let tw = face.width(o, s.size) as i32;
            face.draw(c, b.x + (b.w as i32 - tw) / 2, b.y + face.baseline_in(s.size, b.h), o, s.size, if primary { col.on_accent } else { col.ink });
        }
    }
}

/// Zalomi text na slova do sirky `w` (slovo delsi nez radek zustane cele).
pub fn wrap(text: &Text, s: &str, px: f32, w: u32) -> Vec<String> {
    let mut out = Vec::new();
    for para in s.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let cand = if line.is_empty() { String::from(word) } else { alloc::format!("{line} {word}") };
            if !line.is_empty() && text.width(&cand, px) > w {
                out.push(core::mem::take(&mut line));
                line = String::from(word);
            } else {
                line = cand;
            }
            // Slovo delsi nez radek (cesta): zalomit po znacich, cele.
            while text.width(&line, px) > w {
                let mut cut = String::new();
                let mut rest = String::new();
                for ch in line.chars() {
                    if rest.is_empty() && text.width(&alloc::format!("{cut}{ch}"), px) <= w {
                        cut.push(ch);
                    } else {
                        rest.push(ch);
                    }
                }
                if cut.is_empty() {
                    break;
                }
                out.push(cut);
                line = rest;
            }
        }
        out.push(line);
        if out.len() >= MAX_LINES {
            out.truncate(MAX_LINES);
            break;
        }
    }
    out
}

impl Comp {
    /// Zadost na kanalu `dialog`.
    pub fn dialog_request(&mut self, peer: PeerId, req: DialogRequest, now: u64) {
        self.input_now = now;
        match req {
            DialogRequest::Ask {
                id,
                kind,
                app,
                text,
                options,
            } => {
                if self.dialogs.queue.len() >= MAX_QUEUE {
                    self.out.push(Out::Dialog(peer, DialogReply::Cancelled { id }));
                    return;
                }
                self.open_dialog(Dialog {
                    owner: Owner::Peer(peer),
                    id,
                    kind,
                    app,
                    text,
                    flag: (kind == DialogKind::Elevate).then_some(false),
                    options,
                    sel: 0,
                    primary: 0,
                    password: String::new(),
                });
            }
            DialogRequest::Cancel { id } => {
                let before = self.dialogs.queue.len();
                let was_front = self.dialogs.active().is_some_and(|d| d.owner == Owner::Peer(peer) && d.id == id);
                self.dialogs.queue.retain(|d| !(d.owner == Owner::Peer(peer) && d.id == id));
                if self.dialogs.queue.len() != before {
                    self.out.push(Out::Dialog(peer, DialogReply::Cancelled { id }));
                    if was_front {
                        self.dialog_front_changed();
                    }
                }
            }
            DialogRequest::Phrase { text } => {
                self.dialogs.phrase = text.chars().take(64).collect();
            }
            // Stav rezimu vyvojare (jen vyvojove sestaveni).
            #[cfg(feature = "dev-mode")]
            DialogRequest::DevMode { on } => self.dev_mode_state(peer, on),
            // Zprava, kterou toto sestaveni nezna (protokol s feature, kompozitor bez).
            #[allow(unreachable_patterns)]
            _ => {}
        }
    }

    pub(crate) fn open_dialog(&mut self, mut d: Dialog) {
        d.primary = d.sel;
        let first = self.dialogs.queue.is_empty();
        let front_changes = first || d.owner == Owner::Secure;
        if d.owner == Owner::Secure {
            // Bezpecna obrazovka predbehne vse.
            self.dialogs.queue.push_front(d);
        } else {
            self.dialogs.queue.push_back(d);
        }
        if front_changes {
            self.dialog_front_changed();
        }
    }

    /// Aktivni dialog se zmenil (novy, zodpovezeny, zruseny): vstup az
    /// INPUT_DELAY po zobrazeni snimku s novym dialogem.
    pub(crate) fn dialog_front_changed(&mut self) {
        self.dialogs.since = u64::MAX;
        self.dialogs.shown = if self.dialogs.queue.is_empty() { Shown::Yes } else { Shown::Waiting };
        self.damage_all();
    }

    /// Snimek se slozil (`tick`): dialog, ktery na nej cekal, je v nem.
    pub(crate) fn dialog_composed(&mut self) {
        if self.dialogs.shown == Shown::Waiting {
            self.dialogs.shown = Shown::Composed;
        }
    }

    /// Displej snimek zobrazil v case `time`.
    pub(crate) fn dialog_presented(&mut self, time: u64) {
        if self.dialogs.shown == Shown::Composed {
            self.dialogs.shown = Shown::Yes;
            self.dialogs.since = time;
        }
    }

    /// Prijima aktivni dialog vstup (zobrazen a uplynula ochranna doba)?
    fn dialog_ready(&self, now: u64) -> bool {
        self.dialogs.shown == Shown::Yes && now >= self.dialogs.since.saturating_add(INPUT_DELAY)
    }

    /// Kanal `dialog` zavren: jeho dialogy zmizi.
    pub fn dialog_peer_gone(&mut self, peer: PeerId) {
        let front = self.dialogs.active().map(|d| d.owner);
        self.dialogs.queue.retain(|d| d.owner != Owner::Peer(peer));
        if front == Some(Owner::Peer(peer)) {
            self.dialog_front_changed();
        }
    }

    /// Bezpecna zkratka (Ctrl+Alt+Delete) od `inputd`.
    pub fn secure_attention(&mut self, now: u64) {
        self.input_now = now;
        if self.dialogs.secure_active() {
            return;
        }
        self.open_dialog(Dialog {
            owner: Owner::Secure,
            id: 0,
            kind: DialogKind::Confirm,
            app: String::new(),
            text: String::from("Systém převzal klávesnici a myš. Tuto obrazovku nemůže žádná aplikace napodobit ani zavřít."),
            // S prihlasenym uzivatelem i zamceni obrazovky.
            options: if self.has_active_session() {
                alloc::vec![String::from("Zpět"), String::from(LOCK)]
            } else {
                alloc::vec![String::from("Zpět")]
            },
            sel: 0,
            primary: 0,
            password: String::new(),
            flag: None,
        });
    }

    fn answer(&mut self, choice: usize) {
        let Some(mut d) = self.dialogs.queue.pop_front() else { return };
        self.dialog_front_changed();
        match d.owner {
            Owner::Peer(p) => self.out.push(Out::Dialog(
                p,
                DialogReply::Answer {
                    id: d.id,
                    // Elevate: povolit se zaskrtnutym rezimem spravy = 2.
                    choice: if choice == 0 && d.flag == Some(true) { 2 } else { choice as u32 },
                    text: core::mem::take(&mut d.password),
                },
            )),
            Owner::Action(call) => self.action_confirmed(call, choice == 0),
            Owner::Secure => {
                if d.options.get(choice).is_some_and(|o| o == LOCK) {
                    self.lock_session();
                }
                self.out.push(Out::SecureRelease)
            }
            Owner::Device(source) => {
                self.input.untrusted.retain(|&s| s != source);
                self.out.push(Out::Device { source, trust: choice == 0 });
            }
            Owner::Power(reboot) if choice == 0 => {
                self.system_requests.push(if reboot { crate::palette::SystemRequest::Reboot } else { crate::palette::SystemRequest::Shutdown })
            }
            Owner::Power(_) => {}
        }
    }

    /// Nevratna systemova akce z palety: nejdriv systemove potvrzeni.
    pub(crate) fn confirm_power(&mut self, reboot: bool) {
        let (what, button) = if reboot { ("restartovat", "Restartovat") } else { ("vypnout", "Vypnout") };
        self.open_dialog(Dialog {
            owner: Owner::Power(reboot),
            id: 0,
            kind: DialogKind::Confirm,
            app: String::from("Systém"),
            text: alloc::format!("Opravdu {what} počítač? Neuložená práce v aplikacích se může ztratit."),
            options: alloc::vec![String::from(button), String::from("Zrušit")],
            sel: 1,
            primary: 1,
            password: String::new(),
            flag: None,
        });
    }

    /// Nova klavesnice nebo mys ceka na overeni (proti BadUSB): jeji
    /// udalosti se do rozhodnuti ignoruji, potvrdit ji musi stavajici vstup.
    pub(crate) fn new_device(&mut self, source: u16, state: inputproto::DeviceTrust) {
        use inputproto::DeviceTrust::*;
        match state {
            Pending => {
                if self.input.untrusted.contains(&source) {
                    return;
                }
                self.input.untrusted.push(source);
                self.open_dialog(Dialog {
                    owner: Owner::Device(source),
                    id: source as u32,
                    kind: DialogKind::Confirm,
                    app: String::new(),
                    text: alloc::format!(
                        "Bylo připojeno nové vstupní zařízení (zdroj {source}). Povolte ho jen tehdy, když jste ho připojili vy. Zařízení do té doby nic nepíše."
                    ),
                    options: alloc::vec![String::from("Povolit zařízení"), String::from("Odmítnout")],
                    sel: 1,
                    primary: 1,
                    password: String::new(),
                    flag: None,
                });
            }
            Trusted | Rejected => {
                self.input.untrusted.retain(|&s| s != source);
                let front = self.dialogs.active().map(|d| d.owner) == Some(Owner::Device(source));
                self.dialogs.queue.retain(|d| d.owner != Owner::Device(source));
                if front {
                    self.dialog_front_changed();
                }
            }
        }
    }

    pub(crate) fn dialog_key(&mut self, code: u16, action: Action, _ch: Option<char>, _mods: u16, _now: u64) {
        if action == Action::Release || !self.dialog_ready(_now) {
            return;
        }
        let Some(d) = self.dialogs.queue.front_mut() else { return };
        // Zaskrtavatko je v poradi Tab za tlacitky.
        let n = d.options.len().max(1) + d.flag.is_some() as usize;
        match code {
            KEY_ENTER | KEY_KPENTER if d.flag.is_some() && d.sel == d.options.len() => {
                d.flag = d.flag.map(|f| !f);
            }
            KEY_ENTER | KEY_KPENTER => {
                let s = d.sel;
                return self.answer(s);
            }
            KEY_ESC => {
                let last = d.options.len().max(1) - 1;
                return self.answer(last);
            }
            KEY_TAB | KEY_RIGHT => d.sel = (d.sel + 1) % n,
            KEY_LEFT => d.sel = (d.sel + n - 1) % n,
            KEY_BACKSPACE => {
                d.password.pop();
            }
            _ => return,
        }
        self.damage_all();
    }

    pub(crate) fn dialog_text(&mut self, ch: char) {
        let Some(d) = self.dialogs.queue.front_mut() else { return };
        if has_field(d.kind) && d.password.chars().count() < MAX_PASSWORD {
            d.password.push(ch);
            self.damage_all();
        }
    }

    pub(crate) fn dialog_click(&mut self, x: i32, y: i32, now: u64) {
        if !self.dialog_ready(now) {
            return;
        }
        let Some(l) = self.dialogs.layout(&self.style, self.w, self.h) else { return };
        if let Some(i) = l.buttons.iter().position(|b| b.contains(x, y)) {
            self.answer(i);
        } else if l.check.is_some_and(|r| r.contains(x, y))
            && let Some(d) = self.dialogs.queue.front_mut()
        {
            d.flag = d.flag.map(|f| !f);
            self.damage_all();
        }
    }

    pub(crate) fn dialogs_timers(&mut self, _now: u64) {}

    /// Rozvrzeni aktivniho dialogu (testy, inspekce).
    pub fn dialog_layout(&self) -> Option<Layout> {
        self.dialogs.layout(&self.style, self.w, self.h)
    }
}
