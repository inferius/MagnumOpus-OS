//! Ulohy pro panel Ulohy v liste (O8): kanal na `taskd` od initu (zprava
//! `sources tasks`, identita uzivatele jako Spravce uloh: vidi vsechny
//! ulohy, ridi jen ty, ktere mu `taskd` povoli). Odber se zmenami, pri
//! vypadku udalosti znovu od posledniho cisla; akce z panelu jdou zpet
//! jako pozadavky, rozhoduje o nich `taskd`.

use alloc::string::String;
use alloc::vec::Vec;

use comp::{TaskAct, TaskRow};
use libos::Channel;
use taskproto::{Applied, Reply, Request, TaskList, actions};

pub struct TaskFeed {
    pub ch: Channel,
    list: TaskList,
    next_req: u32,
}

impl TaskFeed {
    pub fn new(ch: Channel) -> Self {
        let _ = ch.write(&Request::Subscribe { req: 1, since: None }.encode(), Vec::new());
        Self { ch, list: TaskList::default(), next_req: 2 }
    }

    fn req(&mut self) -> u32 {
        let r = self.next_req;
        self.next_req = self.next_req.wrapping_add(1).max(2);
        r
    }

    /// Precte zpravy bez cekani: `Ok(true)` = seznam se zmenil, `Err` =
    /// taskd skoncil.
    pub fn read(&mut self) -> Result<bool, ()> {
        let mut changed = false;
        loop {
            match self.ch.try_read() {
                Ok(m) => {
                    let Ok(r) = Reply::decode(&m.data) else { continue };
                    if matches!(r, Reply::Result { .. } | Reply::LogLines { .. }) {
                        continue;
                    }
                    changed = true;
                    if self.list.apply(&r) == Applied::Gap {
                        let req = self.req();
                        let since = Some(self.list.seq);
                        let _ = self.ch.write(&Request::Subscribe { req, since }.encode(), Vec::new());
                    }
                }
                Err(libos::Error::ShouldWait) => return Ok(changed),
                Err(_) => return Err(()),
            }
        }
    }

    /// Bezici ulohy pro panel (agent nahore, pak podle cisla).
    pub fn rows(&self) -> Vec<TaskRow> {
        let mut v: Vec<&taskproto::Task> = self.list.tasks.iter().filter(|t| !t.state.finished()).collect();
        v.sort_by_key(|t| (!t.agent, t.id));
        v.into_iter()
            .map(|t| TaskRow {
                id: t.id,
                name: t.name.clone(),
                who: String::from(t.who.as_str()),
                state: taskview::state_word(t),
                agent: t.agent,
                progress: t.progress,
                paused: t.state == taskproto::State::Paused,
                can_pause: taskview::can(t, actions::PAUSE),
                can_resume: taskview::can(t, actions::RESUME),
                can_cancel: taskview::can(t, actions::CANCEL),
            })
            .collect()
    }

    /// Akce z panelu (`taskd` overi pravo i stav).
    pub fn act(&mut self, a: TaskAct) -> bool {
        let req = self.req();
        let r = match a {
            TaskAct::Pause(id) => Request::Pause { req, id },
            TaskAct::Resume(id) => Request::Resume { req, id },
            TaskAct::Cancel(id) => Request::Cancel { req, id },
        };
        self.ch.write(&r.encode(), Vec::new()).is_ok()
    }
}
