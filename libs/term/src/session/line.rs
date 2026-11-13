//! Radky relace ve vlaknech (`crate::fg`): spusteni, obsluha z hlavniho
//! vlakna, Ctrl+Z (na pozadi), `&`, `jobs`, `fg`, `bg`, `kill`.
//!
//! Bez vlaken v prostredi (testy bez `spawn`) bezi radek jako drive
//! v hlavnim vlakne (`Service::run`).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::Ordering;

use shlang::sig::Builtins;
use shlang::value::Value;

use super::{Env, Link, Service, apply, end_tasks};
use crate::fg::{self, Answer, LineJob, LineShared, MainCtx};
use crate::proto::{Item, ServerMsg};
use crate::tasks::Joinable;

/// Radek ve vlakne: v poprede (`fg`) nebo uloha na pozadi (`bg`).
pub struct LineRun {
    pub sh: Arc<LineShared>,
    pub block: u64,
    pub line: String,
    join: Option<Box<dyn Joinable>>,
    /// Cislo ulohy na pozadi (`jobs`, `fg N`); `None` = nikdy nebyl na pozadi.
    pub job: Option<u64>,
    /// Po konci vratit interpret relaci (jen radek, ktery zacal v poprede
    /// a nebyl na pozadi).
    restore: bool,
    /// Blok `fg N`, ktery se ukonci spolu s ulohou.
    fg_block: Option<u64>,
}

/// Radkovy prikaz rizeni uloh relace (ne prikaz interpretu).
enum JobCmd {
    Jobs,
    Fg(Option<u64>),
    Bg(Option<u64>),
    Kill(u64),
}

fn job_cmd(line: &str) -> Option<JobCmd> {
    let mut w = line.split_whitespace();
    let cmd = w.next()?;
    let arg = w.next();
    if w.next().is_some() {
        return None;
    }
    // Cislo ulohy: "2" nebo "%2" (jako bash).
    let num = |a: Option<&str>| a.map(|s| s.trim_start_matches('%').parse::<u64>().ok());
    match cmd {
        "jobs" if arg.is_none() => Some(JobCmd::Jobs),
        "fg" => Some(JobCmd::Fg(num(arg)?)),
        // `bg { ... }` je klicove slovo jazyka, ne rizeni uloh.
        "bg" if arg.is_none_or(|a| !a.starts_with('{')) => Some(JobCmd::Bg(num(arg)?)),
        "kill" => Some(JobCmd::Kill(num(arg)??)),
        _ => None,
    }
}

/// `prikaz &` (ne `&&`): rovnou na pozadi.
fn strip_amp(line: &str) -> Option<&str> {
    let t = line.trim_end();
    let rest = t.strip_suffix('&')?;
    if rest.ends_with('&') {
        return None;
    }
    let rest = rest.trim_end();
    (!rest.is_empty()).then_some(rest)
}

impl Service {
    /// Spusti radek ve vlakne; `false` = prostredi vlakna nema (radek
    /// pobezi v hlavnim vlakne jako drive).
    pub(super) fn run_threaded(&mut self, i: usize, line: &str, link: &mut dyn Link, env: &mut dyn Env) -> bool {
        if !self.line_threads {
            return false;
        }
        let (Some(main), Some(clock)) = (env.notifier(), env.clock()) else { return false };
        if let Some(cmd) = job_cmd(line) {
            if self.sessions[i].fg.is_some() {
                self.sessions[i].queue.push(String::from(line));
            } else {
                self.job_command(i, line, cmd, link);
            }
            return true;
        }
        let (text, detach) = match strip_amp(line) {
            Some(t) => (t, true),
            None => (line, false),
        };
        // Radek v poprede bezi: dalsi radek do fronty (pobezi po nem).
        if !detach && self.sessions[i].fg.is_some() {
            self.sessions[i].queue.push(String::from(line));
            return true;
        }
        let block = self.next_block;
        let Some(fs) = env.task_fs(block) else { return false };
        self.start_block(i, line, link);
        let s = &mut self.sessions[i];
        // V poprede ma radek interpret relace (relace drzi kopii ze zacatku
        // radku); na pozadi (`&`) dostane kopii a relace se nemeni.
        let fork = Box::new(s.interp.fork(Box::new(Builtins)));
        let interp = if detach { fork } else { Box::new(core::mem::replace(&mut s.interp, *fork)) };
        let sh = LineShared::new(main, env.new_notifier());
        // `&`: od zacatku uloha na pozadi (sve klony si na konci zahodi sama -
        // relace jeji interpret nedostane).
        sh.detached.store(detach, Ordering::Release);
        let job = LineJob {
            sh: sh.clone(),
            interp,
            fs,
            clock,
            session: s.id,
            block,
            home: self.home.clone(),
            line: String::from(text),
            tasks: s.tasks.clone(),
        };
        let Some(join) = env.spawn(Box::new(move || fg::run(job))) else {
            // Vlakno nejde spustit: blok s chybou (interpret se nevratil).
            let msg = Item::Error { code: "NO_RESOURCES".into(), message: "vlákno řádku nejde spustit".into(), report: "vlákno řádku nejde spustit".into() };
            self.push_item(i, block, msg, link);
            self.end_block(i, block, false, 0, link);
            return true;
        };
        let mut run = LineRun { sh, block, line: String::from(text), join: Some(join), job: None, restore: !detach, fg_block: None };
        let s = &mut self.sessions[i];
        if detach {
            let n = s.next_bg;
            s.next_bg += 1;
            run.job = Some(n);
            s.bg.push(run);
            self.push_item(i, block, Item::Note(alloc::format!("[{n}] běží na pozadí · fg {n} ho vrátí, kill {n} ukončí")), link);
        } else {
            s.fg = Some(run);
        }
        true
    }

    /// Polozka do bloku relace (a oknu).
    fn push_item(&mut self, i: usize, block: u64, item: Item, link: &mut dyn Link) {
        let s = &mut self.sessions[i];
        let m = ServerMsg::Item { session: s.id, block, item };
        if s.attached {
            link.send(&m);
        }
        apply(&mut s.blocks, self.keep_blocks, m);
    }

    /// Ctrl+C: preruseni radku v poprede.
    pub(super) fn interrupt_line(&mut self, session: u64) {
        if let Some(i) = self.session_index(session)
            && let Some(f) = &self.sessions[i].fg
        {
            f.sh.cancel();
        }
    }

    /// Ctrl+Z: radek v poprede na pozadi (bezi dal, relace pokracuje kopii
    /// stavu ze zacatku radku a prijme dalsi radek).
    pub(super) fn background(&mut self, session: u64, link: &mut dyn Link, env: &mut dyn Env) {
        let Some(i) = self.session_index(session) else { return };
        let Some(mut run) = self.sessions[i].fg.take() else { return };
        run.sh.detached.store(true, Ordering::Release);
        run.restore = false;
        let n = match run.job {
            Some(n) => n,
            None => {
                let s = &mut self.sessions[i];
                let n = s.next_bg;
                s.next_bg += 1;
                n
            }
        };
        run.job = Some(n);
        let (block, fg_block) = (run.block, run.fg_block.take());
        self.sessions[i].bg.push(run);
        self.push_item(i, block, Item::Note(alloc::format!("[{n}] na pozadí (Ctrl+Z) · fg {n} ho vrátí, kill {n} ukončí")), link);
        // Blok `fg N` (radek, ktery ulohu vratil) konci.
        if let Some(b) = fg_block {
            self.end_block(i, b, true, 0, link);
        }
        self.next_queued(i, link, env);
    }

    /// Dalsi radek z fronty relace (po konci nebo Ctrl+Z radku v poprede).
    fn next_queued(&mut self, i: usize, link: &mut dyn Link, env: &mut dyn Env) {
        while self.sessions[i].fg.is_none() && !self.sessions[i].queue.is_empty() {
            let line = self.sessions[i].queue.remove(0);
            if !self.run_threaded(i, &line, link, env) {
                break;
            }
        }
    }

    fn job_command(&mut self, i: usize, line: &str, cmd: JobCmd, link: &mut dyn Link) {
        let block = self.next_block;
        self.start_block(i, line, link);
        let s = &mut self.sessions[i];
        let pick = |s: &super::Session, n: Option<u64>| match n {
            Some(n) => s.bg.iter().position(|r| r.job == Some(n)),
            None => (!s.bg.is_empty()).then(|| s.bg.len() - 1),
        };
        let (item, ok, keep_open) = match cmd {
            JobCmd::Jobs => {
                let rows = s
                    .bg
                    .iter()
                    .map(|r| {
                        Value::record(&[
                            ("job", Value::Int(r.job.unwrap_or(0) as i64)),
                            ("state", Value::str("běží")),
                            ("command", Value::str(&r.line)),
                        ])
                    })
                    .collect();
                (Item::Value(Value::List(rows)), true, false)
            }
            JobCmd::Fg(n) => match pick(s, n) {
                Some(k) => {
                    let mut run = s.bg.remove(k);
                    run.sh.detached.store(false, Ordering::Release);
                    run.fg_block = Some(block);
                    let n = run.job.unwrap_or(0);
                    let note = alloc::format!("[{n}] v popředí: {} · Ctrl+C přeruší, Ctrl+Z vrátí na pozadí", run.line);
                    s.fg = Some(run);
                    (Item::Note(note), true, true)
                }
                None => (Item::Error { code: "SH_ARGS".into(), message: "taková úloha na pozadí není (jobs)".into(), report: "taková úloha na pozadí není (jobs)".into() }, false, false),
            },
            JobCmd::Bg(n) => match pick(s, n) {
                Some(k) => (Item::Note(alloc::format!("[{}] už běží na pozadí", s.bg[k].job.unwrap_or(0))), true, false),
                None => (Item::Error { code: "SH_ARGS".into(), message: "taková úloha na pozadí není (jobs)".into(), report: "taková úloha na pozadí není (jobs)".into() }, false, false),
            },
            JobCmd::Kill(n) => match pick(s, Some(n)) {
                Some(k) => {
                    s.bg[k].sh.cancel();
                    (Item::Note(alloc::format!("[{n}] ukončuji")), true, false)
                }
                None => (Item::Error { code: "SH_ARGS".into(), message: alloc::format!("úloha {n} na pozadí není (jobs)"), report: alloc::format!("úloha {n} na pozadí není (jobs)") }, false, false),
            },
        };
        self.push_item(i, block, item, link);
        if !keep_open {
            self.end_block(i, block, ok, 0, link);
        }
    }

    /// Hlavni vlakno: zpravy, vyzvy, pozadavky a konce radku ve vlaknech.
    pub(super) fn pump_lines(&mut self, link: &mut dyn Link, env: &mut dyn Env) {
        for i in 0..self.sessions.len() {
            let n = self.sessions[i].bg.len() + self.sessions[i].fg.is_some() as usize;
            if n == 0 {
                continue;
            }
            // Radek v poprede (index None) a ulohy na pozadi.
            let mut k = 0;
            loop {
                let fg = k == 0;
                if !fg && k - 1 >= self.sessions[i].bg.len() {
                    break;
                }
                let sh = match fg {
                    true => self.sessions[i].fg.as_ref().map(|r| r.sh.clone()),
                    false => Some(self.sessions[i].bg[k - 1].sh.clone()),
                };
                k += 1;
                let Some(sh) = sh else { continue };
                let finished = self.pump_one(i, &sh, link, env);
                if finished {
                    self.finish(i, &sh, link, env);
                    if !fg {
                        k -= 1;
                    }
                }
            }
        }
    }

    /// Jeden radek: schranka, vyzva, pozadavek. `true` = dobehl.
    fn pump_one(&mut self, i: usize, sh: &Arc<LineShared>, link: &mut dyn Link, env: &mut dyn Env) -> bool {
        let (outbox, call, prompt) = {
            let mut st = sh.st.lock();
            let prompt = match st.prompt.as_mut() {
                Some(p) if p.sent.is_none() => {
                    let id = self.next_prompt;
                    self.next_prompt += 1;
                    p.sent = Some(id);
                    Some(ServerMsg::Prompt { prompt: id, question: p.question.clone(), detail: p.detail.clone(), choices: p.choices.clone() })
                }
                _ => None,
            };
            (core::mem::take(&mut st.outbox), st.call.take(), prompt)
        };
        let keep = self.keep_blocks;
        {
            let s = &mut self.sessions[i];
            for m in outbox {
                if s.attached {
                    link.send(&m);
                }
                apply(&mut s.blocks, keep, m);
            }
        }
        if let Some(m) = prompt {
            let ServerMsg::Prompt { prompt: id, .. } = &m else { unreachable!() };
            if self.sessions[i].attached {
                link.send(&m);
            } else {
                // Bez okna se vyzva odmita (jako u radku v hlavnim vlakne).
                sh.answer(*id, Answer::Refused);
            }
        }
        if let Some(call) = call {
            let mut log = Vec::new();
            let task_stack = self.task_stack;
            let Service { sessions, next_block, home, board, .. } = self;
            let s = &mut sessions[i];
            let mut c = MainCtx {
                env: &mut *env,
                board: &mut *board,
                link: &mut *link,
                attached: s.attached,
                tasks: &s.tasks,
                next_block,
                log: &mut log,
                session: s.id,
                home,
                task_stack,
            };
            let r = call(&mut c);
            sh.reply(r);
            end_tasks(board, &mut *env, &log);
            for m in log {
                apply(&mut s.blocks, self.keep_blocks, m);
            }
        }
        sh.st.lock().done.is_some()
    }

    /// Konec radku: interpret relaci (v poprede), konec bloku, dalsi z fronty.
    fn finish(&mut self, i: usize, sh: &Arc<LineShared>, link: &mut dyn Link, env: &mut dyn Env) {
        let Some(done) = sh.st.lock().done.take() else { return };
        let s = &mut self.sessions[i];
        let run = if s.fg.as_ref().is_some_and(|r| Arc::ptr_eq(&r.sh, sh)) {
            s.fg.take()
        } else {
            s.bg.iter().position(|r| Arc::ptr_eq(&r.sh, sh)).map(|k| s.bg.remove(k))
        };
        let Some(mut run) = run else { return };
        if let Some(j) = run.join.take() {
            j.join();
        }
        if run.restore {
            s.interp = *done.interp;
        } else if let Some(n) = run.job {
            // Uloha na pozadi: konec ohlasit radkem v jejim bloku.
            let what = if done.ok {
                "hotovo"
            } else if done.interrupted {
                "přerušeno"
            } else {
                "skončilo chybou"
            };
            let note = Item::Note(alloc::format!("[{n}] {what}: {}", run.line));
            self.push_item(i, run.block, note, link);
        }
        self.end_block(i, run.block, done.ok, done.duration, link);
        if let Some(b) = run.fg_block {
            self.end_block(i, b, done.ok, 0, link);
        }
        self.next_queued(i, link, env);
    }

    /// Bezi radek ve vlakne (v poprede nebo na pozadi)?
    pub fn lines_running(&self) -> bool {
        self.sessions.iter().any(|s| s.fg.is_some() || !s.bg.is_empty())
    }

    /// Odpoved okna na vyzvu radku ve vlakne; `false` = neni jeho.
    pub(super) fn answer_line(&mut self, m: &crate::proto::ClientMsg) -> bool {
        let Some((prompt, a)) = fg::answer_of(m) else { return false };
        for s in &self.sessions {
            for r in s.fg.iter().chain(s.bg.iter()) {
                if r.sh.answer(prompt, a) {
                    return true;
                }
            }
        }
        false
    }
}

/// Konec relace: radky ve vlaknech zrusit.
pub(super) fn cancel_all(fg: &Option<LineRun>, bg: &[LineRun]) {
    for r in fg.iter().chain(bg.iter()) {
        r.sh.cancel();
    }
}
