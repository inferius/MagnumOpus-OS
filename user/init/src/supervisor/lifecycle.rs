//! Zivotni cyklus sluzeb v registru: konec instance, prodleva, start
//! v poradi zavislosti, potvrzeni, vzdani a obnova vzdane sluzby.

use alloc::vec::Vec;

use libos::{Signals, Startup, println, time};

use super::service::{DepMode, StartCtx};
use super::{Entry, Happened, Phase, RESTART_DELAY, RESTART_DELAY_MAX, RESTART_MAX, RESTART_WINDOW, Supervisor};

/// Proc to supervisor vzdal (prehled uloh).
const REASON_CRASHES: &str = "prilis mnoho padu za 60 s";
const REASON_START: &str = "nova instance nejde spustit";
const REASON_DEP: &str = "vzdala to zavislost";

impl Supervisor {
    /// Instance sluzby `i` skoncila.
    pub(super) fn exited(&mut self, i: usize, out: &mut Vec<Happened>) {
        let e = &mut self.entries[i];
        let Some(h) = e.process.take() else { return };
        e.ready = None;
        let code = libos::sys::process_exit_code(h.raw()).unwrap_or(i64::MIN);
        e.last_exit = Some(code);
        if let Some(s) = e.service.as_mut() {
            s.stopped();
        }
        println!(
            "init: sluzba {} skoncila: {}",
            e.name,
            libos::abi::process::describe_exit(code)
        );
        // Pad (vyjimka CPU, panika): vypis s logem procesu do /system/pady.
        let fault = code == libos::abi::process::EXIT_PANIC
            || (libos::abi::process::EXIT_FAULT_BASE - 255..=libos::abi::process::EXIT_FAULT_BASE).contains(&code);
        if fault {
            crate::logd::proc_crash(e.name, libos::abi::process::describe_exit(code));
        }
        out.push(Happened::Exited { name: e.name, code });
        let name = e.name;
        if let Some(dep) = e.stopping_for.take() {
            // Ukoncena kvuli padu zavislosti: nepocita se jako pad.
            e.phase = Phase::WaitingDeps;
            println!("init: sluzba {name} pocka na {dep}");
        } else if e.service.is_none() {
            e.phase = Phase::Stopped;
            println!("init: sluzbu {name} zatim neumim obnovit");
        } else {
            self.schedule_restart(i, out);
        }
        self.stop_dependents(i, out);
    }

    /// Po padu: naplanuje restart s prodlevou, nebo to vzda.
    fn schedule_restart(&mut self, i: usize, out: &mut Vec<Happened>) {
        let e = &mut self.entries[i];
        let now = time::now();
        let window = RESTART_WINDOW.as_nanos() as u64;
        e.history.retain(|&t| now.saturating_sub(t) < window);
        // Zamerne ukonceni testem: restart bez zapoctu do limitu padu.
        let counted = !core::mem::take(&mut e.test_kill);
        if counted && e.history.len() >= RESTART_MAX {
            println!(
                "init: sluzba {} spadla {}x za {} s, vzdavam to",
                e.name,
                RESTART_MAX + 1,
                RESTART_WINDOW.as_secs()
            );
            self.give_up(i, REASON_CRASHES, out);
            return;
        }
        let delay = (RESTART_DELAY.as_nanos() as u64)
            .saturating_mul(1 << e.history.len())
            .min(RESTART_DELAY_MAX.as_nanos() as u64);
        // Do limitu se pocitaji jen vlastni pady (ne restart kvuli zavislosti
        // ani zamerne ukonceni testem).
        if counted {
            e.history.push(now);
        }
        e.phase = Phase::Backoff {
            at: now.saturating_add(delay),
        };
    }

    fn give_up(&mut self, i: usize, reason: &'static str, out: &mut Vec<Happened>) {
        let e = &mut self.entries[i];
        e.phase = Phase::GaveUp;
        e.reason = Some(reason);
        e.kill();
        out.push(Happened::GaveUp(e.name));
    }

    /// Sluzby s `Dep::restart` na sluzbe `i` (i neprimo): ukoncit, spusti
    /// se znovu po ni. Bezici instance se ukonci (konec pak zpracuje
    /// [`Self::exited`]), cekajici prejdou do `WaitingDeps`.
    fn stop_dependents(&mut self, i: usize, out: &mut Vec<Happened>) {
        let dep = self.entries[i].name;
        for j in 0..self.entries.len() {
            let d = &mut self.entries[j];
            if !d.deps.iter().any(|x| x.name == dep && x.mode == DepMode::Restart) {
                continue;
            }
            match d.phase {
                Phase::GaveUp | Phase::Stopped | Phase::WaitingDeps => continue,
                Phase::Running | Phase::Starting { .. } if d.process.is_some() => {
                    if d.stopping_for.is_none() {
                        d.stopping_for = Some(dep);
                        println!("init: ukoncuji {} (zavisi na {dep})", d.name);
                        d.kill();
                        out.push(Happened::Stopped { name: d.name, because: dep });
                    }
                }
                _ => {
                    d.phase = Phase::WaitingDeps;
                    out.push(Happened::Stopped { name: d.name, because: dep });
                    // Rekurzivne: kdo zavisi na cekajici sluzbe.
                    self.stop_dependents(j, out);
                }
            }
        }
    }

    /// Nova instance poslala potvrzeni (nebo zavrela kanal).
    pub(super) fn started(&mut self, i: usize, out: &mut Vec<Happened>) {
        let e = &mut self.entries[i];
        if e.ready.is_none() {
            return;
        }
        let res = match e.service.as_mut() {
            Some(s) => s.ready(),
            None => Ok(true),
        };
        match res {
            Ok(true) => {
                e.ready = None;
                self.running(i, out);
            }
            // Jeste ne: cekat dal (kanal zustava polozkou smycky).
            Ok(false) => {}
            Err(err) => {
                // Ukoncit: konec procesu pak zpracuje `exited` jako pad.
                e.ready = None;
                println!("init: sluzba {} nepotvrdila start: {err}", e.name);
                e.kill();
            }
        }
    }

    /// Instance `i` bezi: ohlasit a dat vedet sluzbam s `Dep::notify`.
    fn running(&mut self, i: usize, out: &mut Vec<Happened>) {
        let e = &mut self.entries[i];
        e.phase = Phase::Running;
        e.reason = None;
        println!("init: sluzba {} znovu spustena (generace {})", e.name, e.generation);
        out.push(Happened::Restarted(e.name));
        let (dep, generation) = (e.name, e.generation);
        for d in self.entries.iter_mut() {
            if d.phase == Phase::Running
                && d.deps.iter().any(|x| x.name == dep && x.mode == DepMode::Notify)
                && let Some(s) = d.service.as_mut()
            {
                s.dependency_restarted(dep, generation);
                out.push(Happened::Notified { name: d.name, dep });
            }
        }
    }

    /// Sluzby, ktere do lhuty nepotvrdily start, ukonci (konec procesu
    /// zpracuje `exited` jako pad).
    pub(super) fn start_timeouts(&mut self) {
        let now = time::now();
        for e in self.entries.iter_mut() {
            if let Phase::Starting { deadline } = e.phase
                && deadline <= now
                && e.ready.is_some()
            {
                e.ready = None;
                println!("init: sluzba {} neodpovedela, ukoncuji ji", e.name);
                e.kill();
            }
        }
    }

    /// Nejblizsi cas, kdy je co delat (prodleva, lhuta potvrzeni).
    pub(super) fn next_due(&self) -> Option<u64> {
        self.entries
            .iter()
            .filter_map(|e| match e.phase {
                Phase::Backoff { at } => Some(at),
                Phase::Starting { deadline } if e.ready.is_some() => Some(deadline),
                _ => None,
            })
            .min()
    }

    /// Bezi vsechny zavislosti `Dep::restart` sluzby `i`? `Err` = nektera
    /// to vzdala nebo bez restartu skoncila (nepobezi).
    fn deps_ready(&self, i: usize) -> Result<bool, ()> {
        let mut all = true;
        for d in self.entries[i].deps.iter().filter(|d| d.mode == DepMode::Restart) {
            match self.find(d.name).map(|k| &self.entries[k]) {
                // Zavislost mimo registr (napr. v testu): neomezuje.
                None => {}
                Some(x) => match x.phase {
                    Phase::Running => {}
                    Phase::GaveUp | Phase::Stopped => return Err(()),
                    _ => all = false,
                },
            }
        }
        Ok(all)
    }

    /// Spusti sluzby, jejichz prodleva uplynula a zavislosti bezi.
    pub(super) fn restart_due(&mut self, startup: &Startup, out: &mut Vec<Happened>) {
        let now = time::now();
        // Opakovat, dokud se neco meni (vzdani se siri po zavislostech).
        let mut changed = true;
        while changed {
            changed = false;
            for i in 0..self.entries.len() {
                let due = match self.entries[i].phase {
                    Phase::Backoff { at } => at <= now,
                    Phase::WaitingDeps => self.entries[i].process.is_none(),
                    _ => false,
                };
                if !due || self.entries[i].service.is_none() {
                    continue;
                }
                match self.deps_ready(i) {
                    Ok(true) => {
                        self.start(i, startup, out);
                        changed = true;
                    }
                    Ok(false) => self.entries[i].phase = Phase::WaitingDeps,
                    Err(()) => {
                        println!("init: sluzba {} nepobezi: {REASON_DEP}", self.entries[i].name);
                        self.give_up(i, REASON_DEP, out);
                        changed = true;
                    }
                }
            }
        }
    }

    /// Spusti novou instanci sluzby `i`.
    fn start(&mut self, i: usize, startup: &Startup, out: &mut Vec<Happened>) {
        let timeout = self.start_timeout.as_nanos() as u64;
        let e = &mut self.entries[i];
        let now = time::now();
        let restart = e.generation > 0;
        if restart {
            e.restarts += 1;
        }
        let generation = e.generation + 1;
        let Some(service) = e.service.as_mut() else { return };
        let ctx = StartCtx {
            startup,
            generation,
            restart,
        };
        match service.start(&ctx) {
            Ok(s) => {
                e.generation = generation;
                e.process = Some(s.process);
                match s.ready {
                    Some(ch) => {
                        e.ready = Some(ch);
                        e.phase = Phase::Starting {
                            deadline: now.saturating_add(timeout),
                        };
                    }
                    None => self.running(i, out),
                }
            }
            Err(err) => {
                // Start selhal: pocita se jako dalsi pad.
                println!("init: sluzbu {} nelze spustit: {err}", e.name);
                e.history.push(now);
                if e.history.len() > RESTART_MAX {
                    self.give_up(i, REASON_START, out);
                } else {
                    e.phase = Phase::Backoff {
                        at: now.saturating_add(RESTART_DELAY_MAX.as_nanos() as u64),
                    };
                }
            }
        }
    }

    /// Sluzba, ktera to vzdala, dostane novou sanci (rucni zasah spravce
    /// systemu; testy). S ni i sluzby, ktere to vzdaly kvuli ni.
    #[cfg(feature = "tests")]
    pub fn revive(&mut self, name: &str) -> bool {
        let Some(i) = self.find(name) else { return false };
        if self.entries[i].phase != Phase::GaveUp {
            return false;
        }
        let e = &mut self.entries[i];
        e.history.clear();
        e.reason = None;
        e.phase = Phase::Backoff { at: time::now() };
        // I neprimo zavisle (C -> B -> A): projit do hloubky.
        let mut todo = alloc::vec![e.name];
        while let Some(dep) = todo.pop() {
            for d in self.entries.iter_mut() {
                if d.phase == Phase::GaveUp
                    && d.reason == Some(REASON_DEP)
                    && d.deps.iter().any(|x| x.name == dep)
                {
                    d.reason = None;
                    d.phase = Phase::WaitingDeps;
                    todo.push(d.name);
                }
            }
        }
        true
    }

    /// Zije jeste proces instance `e` (konec mohl prijit a nebyt
    /// zpracovany)?
    pub(super) fn alive(e: &Entry) -> bool {
        match &e.process {
            Some(p) => libos::wait_one(p, Signals::TERMINATED, Some(0)).is_err(),
            None => false,
        }
    }
}
