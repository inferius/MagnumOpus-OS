//! Panel uloh vpravo v okne terminalu (`design/terminal.html`,
//! `.tasks-panel`): zahlavi s poctem bezicich uloh a karty uloh (ikona,
//! jmeno, stav, pruh prubehu, tlacitko Zrusit). Data jsou `App::tasks`
//! z prehledu uloh (`taskd`); klik na Zrusit posle `App::task_action`.
//!
//! Panel je videt sam jen na velke obrazovce (sirsi nez [`SMALL_SCREEN`])
//! a v okne aspon [`MIN_WINDOW`]; jinak ulohy ukaze textovy panel Ctrl-T.

use alloc::vec::Vec;

use gfx::{Canvas, Rect};
use taskproto::{State, Task, actions};

use crate::Look;

/// Obrazovka do teto sirky je "mala": panel jen pres Ctrl-T (rozhodnuti
/// koordinatora, krok 5 1A).
pub const SMALL_SCREEN: u32 = 1280;
/// Nejuzsi okno s panelem (zlom `900px` z navrhu).
pub const MIN_WINDOW: u32 = 900;

/// Ma byt panel videt?
pub fn shown(screen_w: u32, win_w: u32) -> bool {
    screen_w > SMALL_SCREEN && win_w >= MIN_WINDOW
}

/// Karta ulohy v panelu: plocha karty a tlacitka Zrusit (pokud uloha jde
/// zrusit).
pub struct Card {
    pub rect: Rect,
    pub cancel: Option<Rect>,
    pub task: usize,
}

/// Plocha panelu v okne `w` x `h` (pod titulkem).
pub fn area(look: &Look, w: u32, h: u32) -> Rect {
    let pw = look.t.size.side_panel.min(w);
    let th = look.title_h();
    Rect::new((w - pw) as i32, th as i32, pw, h.saturating_sub(th))
}

/// Vyska karty: okraj, jmeno, stav, radek s pruhem a tlacitkem, okraj.
fn card_h(look: &Look) -> u32 {
    let t = &look.t;
    2 * t.space[3] + t.text.small.line + t.space[1] + t.text.caption.line + t.space[2] + t.size.control_h_compact
}

fn header_h(look: &Look) -> u32 {
    let t = &look.t;
    t.space[5] + t.text.body.line + t.space[2]
}

/// Rozlozeni karet (stejne pro kresleni i klik); karty, ktere se nevejdou,
/// se vynechaji.
pub fn cards(look: &Look, tasks: &[Task], area: Rect) -> Vec<Card> {
    let t = &look.t;
    let sp = &t.space;
    let ch = card_h(look);
    let x = area.x + sp[3] as i32;
    let w = area.w.saturating_sub(2 * sp[3]);
    let mut y = area.y + header_h(look) as i32;
    let bottom = area.y + area.h as i32 - sp[3] as i32;
    let mut out = Vec::new();
    for (i, task) in tasks.iter().enumerate() {
        if y + ch as i32 > bottom {
            break;
        }
        let rect = Rect::new(x, y, w, ch);
        let b = t.size.control_h_compact;
        let cancel = (task.actions & actions::CANCEL != 0 && !task.state.finished()).then(|| {
            Rect::new(rect.x + rect.w as i32 - (sp[3] + b) as i32, rect.y + (ch - sp[3] - b) as i32, b, b)
        });
        out.push(Card { rect, cancel, task: i });
        y += (ch + sp[2]) as i32;
    }
    out
}

/// Nakresli panel do `area`.
pub fn paint(c: &mut Canvas<'_>, look: &mut Look, tasks: &[Task], area: Rect) {
    let t = look.t.clone();
    let (sp, col) = (&t.space, &t.color);
    c.fill(area, col.surface_2);
    c.fill(Rect::new(area.x, area.y, 1, area.h), col.border);
    // Zahlavi: ikona, "Ulohy", pocet bezicich vpravo.
    let hs = t.text.body_strong;
    let is = t.size.icon_s;
    let hy = area.y + sp[5] as i32;
    let hx = area.x + sp[4] as i32;
    look.icons.draw(c, "tasks", hx, hy + (hs.line - is) as i32 / 2, is, t.icon_stroke, col.ink);
    let f = look.ui.face(hs.weight);
    let base = hy + f.baseline_in(hs.size, hs.line);
    f.draw(c, hx + (is + sp[2]) as i32, base, "Úlohy", hs.size, col.ink);
    let running = tasks.iter().filter(|t| !t.state.finished()).count();
    let n = alloc::format!("{running}");
    let cs = t.text.caption;
    let fc = look.ui.face(t.text.body_strong.weight);
    let nw = fc.width(&n, cs.size) + 2 * sp[3];
    let nh = cs.line;
    let nr = Rect::new(area.x + area.w as i32 - (sp[4] + nw) as i32, hy + (hs.line - nh) as i32 / 2, nw, nh);
    c.round_fill(nr, nh / 2, col.surface_3);
    let base = nr.y + fc.baseline_in(cs.size, nh);
    fc.draw(c, nr.x + sp[3] as i32, base, &n, cs.size, col.ink);
    if tasks.is_empty() {
        let s = t.text.small;
        let y = area.y + header_h(look) as i32;
        let f = look.ui.face(s.weight);
        let base = y + f.baseline_in(s.size, s.line);
        f.draw(c, area.x + sp[4] as i32, base, "Žádné úlohy", s.size, col.ink_muted);
        return;
    }
    for card in cards(look, tasks, area) {
        paint_card(c, look, &tasks[card.task], &card);
    }
}

fn paint_card(c: &mut Canvas<'_>, look: &mut Look, task: &Task, card: &Card) {
    let t = look.t.clone();
    let (sp, col) = (&t.space, &t.color);
    let r = card.rect;
    c.round_fill_bordered(r, t.radius.control, col.surface, col.border, 1);
    if task.agent {
        // Uloha agenta: vyrazny levy okraj barvou AI (`.task.agent`).
        let old = c.clip();
        c.set_clip(old.intersect(&Rect::new(r.x, r.y, sp[2], r.h)));
        c.round_fill(r, t.radius.control, col.ai);
        c.set_clip(old);
    }
    let x = r.x + sp[3] as i32;
    let inner = r.w.saturating_sub(2 * sp[3]);
    // Radek 1: ikona (terminal / agent) a jmeno tucne.
    let s = t.text.small;
    let is = t.size.icon_s;
    let y1 = r.y + sp[3] as i32;
    let icon = if task.agent { "agent" } else { "terminal" };
    look.icons.draw(c, icon, x, y1 + (s.line - is) as i32 / 2, is, t.icon_stroke, if task.agent { col.ai } else { col.ink });
    let f = look.ui.face(t.text.body_strong.weight);
    let base = y1 + f.baseline_in(s.size, s.line);
    let nx = x + (is + sp[2]) as i32;
    f.draw_ellipsized(c, nx, base, &task.name, s.size, col.ink, inner.saturating_sub(is + sp[2]));
    // Radek 2: stav (cesky), chyba barvou chyby.
    let cs = t.text.caption;
    let y2 = y1 + (s.line + sp[1]) as i32;
    let state = term::ui::tasks::state_text(&task.state);
    let color = match task.state {
        State::Failed { .. } => col.danger,
        _ => col.ink_muted,
    };
    let f = look.ui.face(cs.weight);
    let base = y2 + f.baseline_in(cs.size, cs.line);
    f.draw_ellipsized(c, x, base, &state, cs.size, color, inner);
    // Radek 3: pruh prubehu, procenta, Zrusit.
    let b = t.size.control_h_compact;
    let y3 = y2 + (cs.line + sp[2]) as i32;
    let pct = match (task.progress, &task.state) {
        (_, State::Done(_)) => alloc::string::String::from("hotovo"),
        (Some(p), _) => alloc::format!("{p} %"),
        (None, _) => alloc::string::String::new(),
    };
    let pw = f.width(&pct, cs.size);
    let right = card.cancel.map_or(r.x + r.w as i32 - sp[3] as i32, |cr| cr.x - sp[2] as i32);
    let bar_w = (right - x - (pw + sp[2]) as i32).max(0) as u32;
    let bar_h = sp[2];
    let value = match task.state {
        State::Done(_) => 100,
        _ => task.progress.unwrap_or(0) as u32,
    };
    ui::draw::progress_bar(c, Rect::new(x, y3 + (b - bar_h) as i32 / 2, bar_w, bar_h), value, task.agent, col);
    let base = y3 + f.baseline_in(cs.size, b);
    f.draw(c, x + (bar_w + sp[2]) as i32, base, &pct, cs.size, col.ink_muted);
    if let Some(cr) = card.cancel {
        let ib = t.size.icon_s;
        look.icons.draw(c, "stop", cr.x + (cr.w - ib) as i32 / 2, cr.y + (cr.h - ib) as i32 / 2, ib, t.icon_stroke, col.ink_muted);
    }
}
