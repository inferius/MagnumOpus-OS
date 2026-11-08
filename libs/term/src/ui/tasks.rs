//! Panel uloh v okne terminalu (Ctrl-T), pocet bezicich uloh ve stavovem
//! radku a upozorneni na dokonceni nebo chybu. Data jsou z prehledu uloh
//! (`taskd`, zprava `ServerMsg::Tasks`); akce (zrusit, pozastavit,
//! pokracovat) jdou zpravou `ClientMsg::TaskAction` a o pravech rozhoduje
//! taskd.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use taskproto::{State, Task, actions};

use super::draw::Line;
use super::{App, Key};
use crate::proto::ClientMsg;
use crate::theme::Style;

/// Kolik uloh panel ukaze najednou.
pub const PANEL_ROWS: usize = 8;
/// Jak dlouho (snimku po ~150 ms) zustane upozorneni ve stavovem radku.
const NOTICE_TICKS: u32 = 40;

/// Upozorneni na konec ulohy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub error: bool,
    /// Do kdy (snimek) se ukazuje.
    until: u32,
}

impl Notice {
    /// Kratke upozorneni ve stavovem radku (od snimku `tick`).
    pub fn new(text: &str, error: bool, tick: u32) -> Self {
        Self { text: text.into(), error, until: tick.wrapping_add(NOTICE_TICKS) }
    }
}

fn state_text(s: &State) -> String {
    match s {
        State::Running => String::from("běží"),
        State::Paused => String::from("pozastaveno"),
        State::Waiting(p) => format!("čeká: {p}"),
        State::Cancelling => String::from("ruší se"),
        State::Done(r) if r.is_empty() => String::from("hotovo"),
        State::Done(r) => format!("hotovo: {r}"),
        State::Failed { message, .. } => format!("chyba: {message}"),
        State::Cancelled => String::from("zrušeno"),
    }
}

impl App {
    /// Novy seznam uloh: upozorneni na ulohy, ktere prave skoncily.
    pub(crate) fn set_tasks(&mut self, tasks: Vec<Task>) {
        for t in &tasks {
            let was_running = self.tasks.iter().any(|o| o.id == t.id && !o.state.finished());
            if was_running && t.state.finished() {
                let error = matches!(t.state, State::Failed { .. });
                self.notice = Some(Notice {
                    text: format!("úloha {}: {}", t.name, state_text(&t.state)),
                    error,
                    until: self.tick.wrapping_add(NOTICE_TICKS),
                });
            }
        }
        self.tasks = tasks;
        if let Some(sel) = self.task_sel.as_mut() {
            *sel = (*sel).min(self.tasks.len().saturating_sub(1));
        }
    }

    /// Bezici ulohy (stavovy radek).
    pub fn tasks_running(&self) -> usize {
        self.tasks.iter().filter(|t| !t.state.finished()).count()
    }

    /// Je panel uloh otevreny?
    pub fn tasks_open(&self) -> bool {
        self.task_sel.is_some()
    }

    /// Aktualni upozorneni (do vyprseni).
    pub fn notice(&self) -> Option<&Notice> {
        self.notice.as_ref().filter(|n| self.tick.wrapping_sub(n.until) > u32::MAX / 2)
    }

    /// Ctrl-T: otevrit nebo zavrit panel.
    pub(crate) fn toggle_tasks(&mut self) {
        self.task_sel = if self.task_sel.is_some() { None } else { Some(0) };
    }

    /// Klavesa v otevrenem panelu; `false` = panel ji nezpracoval.
    pub(crate) fn key_tasks(&mut self, k: Key) -> bool {
        let Some(sel) = self.task_sel else { return false };
        let n = self.tasks.len();
        let action = |what: u32, code: u8, app: &mut App| {
            if let Some(t) = app.tasks.get(sel)
                && t.actions & what != 0
            {
                app.out.push(ClientMsg::TaskAction { session: app.session, task: t.id, action: code });
            }
        };
        match k {
            Key::Esc | Key::Ctrl('t') => self.task_sel = None,
            Key::Up if sel > 0 => self.task_sel = Some(sel - 1),
            Key::Down if sel + 1 < n => self.task_sel = Some(sel + 1),
            Key::Up | Key::Down => {}
            Key::Char('c') => action(actions::CANCEL, 0, self),
            Key::Char('p') => action(actions::PAUSE, 1, self),
            Key::Char('r') => action(actions::RESUME, 2, self),
            _ => return false,
        }
        true
    }

    /// Radky panelu (nad editorem).
    pub(crate) fn draw_tasks(&self, out: &mut Vec<Line>) {
        let Some(sel) = self.task_sel else { return };
        let t = &self.theme;
        out.push(Line::new().text(Some(Style { bold: true, ..Style::fg(t.ui.fg) }), &format!(" Úlohy ({} běží)", self.tasks_running())));
        if self.tasks.is_empty() {
            out.push(Line::new().text(Some(Style::fg(t.ui.muted)), "   žádné úlohy"));
        }
        // Okno kolem vybrane ulohy.
        let start = sel.saturating_sub(PANEL_ROWS - 1);
        for (i, task) in self.tasks.iter().enumerate().skip(start).take(PANEL_ROWS) {
            let mut l = Line::new();
            let color = match task.state {
                State::Failed { .. } => t.ui.err,
                State::Done(_) => t.ui.ok,
                State::Cancelled | State::Cancelling => t.ui.warn,
                _ => t.ui.accent,
            };
            l.push(Some(Style::fg(t.ui.muted)), &format!(" {:>4} ", task.id));
            if task.agent {
                l.push(Some(Style::fg(t.ui.warn)), "[AGENT AI] ");
            }
            l.push(Some(Style::fg(t.ui.fg)), &task.name);
            l.push(Some(Style::fg(color)), &format!("  {}", state_text(&task.state)));
            if let Some(p) = task.progress.filter(|_| !task.state.finished()) {
                l.push(Some(Style::fg(t.ui.muted)), &format!("  {p} %"));
            }
            out.push(if i == sel { l.fill(t.ui.select) } else { l });
        }
    }
}
