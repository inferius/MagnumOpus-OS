//! Panel Ulohy na pozadi (O8, design/bubliny-panely.html "Ulohy"): rychly
//! prehled bezicich uloh s prubehem, pozastavit, pokracovat, zrusit
//! a tlacitko "Otevrit Spravce uloh". Podrobnosti (vypis, rozpocet, kdo
//! spustil koho) jsou ve Spravci uloh.
//!
//! Seznam dava sluzba (`compd` odebira `taskd` kanalem od initu s identitou
//! uzivatele jako Spravce uloh); akce jdou zpet jako `Out::Task` a rozhoduje
//! o nich vzdy `taskd` (tlacitko je aktivni jen s jeho pravem). Bez seznamu
//! (sluzba ho neposlala) ukazuje panel jen pocet z `set_tasks`.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ui::{Node, PanelSize, button, column, label, panel, panel_footer, panel_header, panel_line, row};

use crate::Comp;
use crate::barpanel::PanelMsg;

/// Uloha v panelu (z `taskproto::Task`, prevadi sluzba).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRow {
    pub id: u64,
    pub name: String,
    /// Kdo spustil ("Soubory", "michal").
    pub who: String,
    /// Stav slovem ("běží", "čeká: ...").
    pub state: String,
    pub agent: bool,
    /// Prubeh 0-100, `None` = neznamy.
    pub progress: Option<u8>,
    pub paused: bool,
    /// Prava od `taskd` a stav: smi se pozastavit, pokracovat, zrusit.
    pub can_pause: bool,
    pub can_resume: bool,
    pub can_cancel: bool,
}

/// Akce nad ulohou z panelu (sluzba ji posle `taskd`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskAct {
    Pause(u64),
    Resume(u64),
    Cancel(u64),
}

/// Kolik uloh panel ukaze (zbytek ve Spravci uloh).
const MAX_ROWS: usize = 6;

impl Comp {
    /// Bezici ulohy pro panel (`None` = sluzba seznam nedava, jen pocet).
    pub fn set_task_rows(&mut self, rows: Option<Vec<TaskRow>>) {
        if self.task_rows != rows {
            self.task_rows = rows;
            self.bar_panel_changed();
        }
    }

    /// Pohled panelu uloh.
    pub(crate) fn tasks_view(&self, s: &PanelSize) -> Node<PanelMsg> {
        let (run, agent) = self.tasks;
        let sub = if agent > 0 { format!("{run} běží, agent {agent}") } else { format!("{run} běží") };
        let mut items = alloc::vec![panel_header(s, "tasks", "Úlohy na pozadí", &sub)];
        match &self.task_rows {
            Some(rows) if !rows.is_empty() => {
                for t in rows.iter().take(MAX_ROWS) {
                    items.push(task_line(s, t));
                }
                if rows.len() > MAX_ROWS {
                    items.push(panel_line(s, alloc::vec![label(&format!("a další ({}) ve Správci úloh", rows.len() - MAX_ROWS)).tone(ui::TextTone::Small)]));
                }
            }
            Some(_) => items.push(panel_line(s, alloc::vec![label("Nic neběží na pozadí.").tone(ui::TextTone::Small)])),
            None => items.push(panel_line(s, alloc::vec![label("Seznam úloh je ve Správci úloh.").tone(ui::TextTone::Small)])),
        }
        items.push(panel_footer(s, alloc::vec![button("Otevřít Správce úloh").icon("tasks").on_click(PanelMsg::OpenTasks).grow().key("spravce")]));
        panel(items)
    }
}

/// Radek ulohy (`.task-row`): tucne jmeno a stitek agenta, pod nim kdo,
/// prubeh a stav, pruh prubehu; vpravo pozastavit nebo pokracovat a zrusit.
fn task_line(s: &PanelSize, t: &TaskRow) -> Node<PanelMsg> {
    let mut head = alloc::vec![label(&t.name).tone(ui::TextTone::Strong)];
    if t.agent {
        head.push(label("agent").agent());
    }
    let pct = t.progress.map_or(String::new(), |p| format!(" · {p} %"));
    let who = if t.agent { String::from("agent") } else { t.who.clone() };
    let mut text = alloc::vec![row(head).gap(s.s2), label(&format!("{who}{pct} · {}", t.state)).tone(ui::TextTone::Small)];
    if let Some(p) = t.progress {
        let bar = ui::progress(p as u32);
        text.push(if t.agent { bar.agent() } else { bar });
    }
    let pause = if t.paused {
        button("").icon("play").ghost().tooltip(&format!("Pokračovat: {}", t.name)).disabled(!t.can_resume).on_click(PanelMsg::Task(crate::TaskAct::Resume(t.id)))
    } else {
        button("").icon("pause").ghost().tooltip(&format!("Pozastavit: {}", t.name)).disabled(!t.can_pause).on_click(PanelMsg::Task(crate::TaskAct::Pause(t.id)))
    };
    let cancel = button("").icon("stop").ghost().tooltip(&format!("Zrušit: {}", t.name)).disabled(!t.can_cancel).on_click(PanelMsg::Task(crate::TaskAct::Cancel(t.id)));
    panel_line(s, alloc::vec![column(text).gap(s.s1).grow(), pause, cancel])
}
