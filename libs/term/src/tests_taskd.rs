//! Relace a prehled uloh: jadro taskd (`taskcore`) misto sluzby, kanaly
//! jsou fronty v pameti. Ulohy `async` se hlasi, `tasks` je vypise,
//! zruseni z panelu okna i prikazem dojde do ulohy, okno dostane seznam
//! a upozorneni.

use std::time::Duration;

use shlang::value::Value;
use taskcore::{ClientId, Core, Out, ProducerId, Scope};
use taskproto::State;

use crate::proto::{ClientMsg, ServerMsg};
use crate::taskboard::Chan;
use crate::tests_tasks::{Rig, last_value};
use crate::theme::Theme;
use crate::ui::{App, Key};

/// taskd v testu: zpravy relace hned zpracuje, odpovedi da do `inbox`.
pub(crate) struct Fake {
    pub core: Core,
    client: ClientId,
    producer: ProducerId,
    pub inbox: Vec<(Chan, Vec<u8>)>,
}

impl Fake {
    pub fn new() -> Self {
        let mut core = Core::new();
        let producer = core.add_producer("uzivatel");
        let client = core.add_client(Scope { identity: "uzivatel".into(), all: false, control: true, control_others: true });
        Self { core, client, producer, inbox: Vec::new() }
    }

    fn deliver(&mut self, out: Vec<Out>) {
        for o in out {
            match o {
                Out::Client(c, d) if c == self.client => self.inbox.push((Chan::Client, d)),
                Out::Producer(p, d) if p == self.producer => self.inbox.push((Chan::Producer, d)),
                _ => {}
            }
        }
    }

    pub fn send(&mut self, to: Chan, data: &[u8]) -> bool {
        let now = 1;
        let out = match to {
            Chan::Producer => self.core.producer_msg(self.producer, data, now),
            Chan::Client => self.core.client_msg(self.client, data, now),
        };
        self.deliver(out);
        true
    }
}

fn tasks_value(r: &mut Rig) -> Vec<Value> {
    let (items, ok) = r.run("tasks");
    assert!(ok, "{items:?}");
    match last_value(&items) {
        Some(Value::List(rows)) => rows,
        other => panic!("tasks: {other:?}"),
    }
}

fn field<'a>(row: &'a Value, name: &str) -> &'a Value {
    let Value::Record(r) = row else { panic!("{row:?}") };
    &r.iter().find(|(k, _)| k == name).unwrap().1
}

#[test]
fn async_task_is_listed_and_cancelled_from_the_window() {
    let mut r = Rig::with_taskd(true, Some(Fake::new()));
    let (_, ok) = r.run("let t = async { sleep 3s; 1 }");
    assert!(ok);
    // Seznam: uloha bezi, s blokem v terminalu.
    let rows = tasks_value(&mut r);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(matches!(field(&rows[0], "name"), Value::Str(n) if n.starts_with("async ")));
    assert_eq!(field(&rows[0], "state"), &Value::str("running"));
    let Value::Int(id) = field(&rows[0], "id") else { panic!() };
    // Okno dostalo seznam uloh (panel, stavovy radek).
    r.idle();
    assert!(r.link.sent.iter().any(|m| matches!(m, ServerMsg::Tasks { tasks } if tasks.len() == 1)));
    // Zrusit z panelu okna: taskd preda zruseni relaci, uloha skonci.
    let s = r.session;
    r.handle(ClientMsg::TaskAction { session: s, task: *id as u64, action: 0 });
    r.idle_until(|r| r.env.taskd.as_ref().is_some_and(|t| t.core.running().0 == 0));
    // Zrusena uloha vrati pri `await` preruseni (try ho nechyta, jako Ctrl-C).
    let (items, ok) = r.run("await t");
    assert!(!ok && format!("{items:?}").contains("SH_INTERRUPTED"), "{items:?}");
    let rows = tasks_value(&mut r);
    assert_eq!(field(&rows[0], "state"), &Value::str("cancelled"));
}

#[test]
fn tasks_command_actions_and_errors() {
    let mut r = Rig::with_taskd(true, Some(Fake::new()));
    r.run("let t = async { sleep 3s }");
    let rows = tasks_value(&mut r);
    let Value::Int(id) = field(&rows[0], "id") else { panic!() };
    // Pozastaveni uloha neumi: chyba od taskd.
    let (items, ok) = r.run(&format!("tasks pause {id}"));
    assert!(!ok, "{items:?}");
    // Neexistujici uloha.
    let (items, ok) = r.run("tasks cancel 999");
    assert!(!ok && format!("{items:?}").contains("NOT_FOUND"), "{items:?}");
    // Zruseni prikazem.
    let (items, ok) = r.run(&format!("tasks cancel {id}"));
    assert!(ok, "{items:?}");
    r.idle_until(|r| r.env.taskd.as_ref().is_some_and(|t| t.core.running().0 == 0));
    // Hotova `async` uloha: done.
    r.run("let q = async { 5 }\nawait q");
    r.idle();
    let rows = tasks_value(&mut r);
    assert!(rows.iter().any(|row| field(row, "state") == &Value::str("done")), "{rows:?}");
    // Neznama akce.
    let (_, ok) = r.run("tasks smazat 1");
    assert!(!ok);
}

#[test]
fn without_taskd_tasks_reports_not_supported() {
    let mut r = Rig::new(true);
    let (items, ok) = r.run("tasks");
    assert!(!ok && format!("{items:?}").contains("NOT_SUPPORTED"), "{items:?}");
    // Relace bez taskd funguje dal (async bez hlaseni).
    let (items, ok) = r.run("let a = async { 2 }\nawait a");
    assert!(ok && last_value(&items) == Some(Value::Int(2)), "{items:?}");
}

fn task(id: u64, state: State, acts: u32) -> taskproto::Task {
    taskproto::Task {
        id,
        parent: 0,
        name: format!("uloha {id}"),
        origin: taskproto::Origin::User,
        who: "shell".into(),
        agent: id == 2,
        state,
        progress: Some(30),
        eta_ms: None,
        memory: None,
        cpu_ns: None,
        started: 1,
        ended: None,
        attach: None,
        actions: acts,
    }
}

#[test]
fn window_panel_status_and_notice() {
    let mut a = App::new(Theme::dark(), 100, 30);
    a.on_msg(ServerMsg::Attached { session: 4, cwd: "/".into(), blocks: 0 });
    a.on_msg(ServerMsg::Tasks { tasks: vec![task(1, State::Running, taskproto::actions::CANCEL), task(2, State::Running, 0)] });
    assert!(a.frame().contains("úlohy 2"), "{}", a.frame());
    // Panel: Ctrl-T, vyber, zrusit (jen kde to uloha dovoli).
    a.key(Key::Ctrl('t'));
    assert!(a.tasks_open());
    let f = a.frame();
    assert!(f.contains("Úlohy (2 běží)") && f.contains("[AGENT AI]") && f.contains("30 %"), "{f}");
    a.take_out();
    a.key(Key::Char('c'));
    assert_eq!(a.take_out(), vec![ClientMsg::TaskAction { session: 4, task: 1, action: 0 }]);
    a.key(Key::Down);
    a.key(Key::Char('c'));
    assert!(a.take_out().is_empty(), "uloha 2 zruseni nedovoluje");
    a.key(Key::Esc);
    assert!(!a.tasks_open());
    // Uloha skoncila chybou: upozorneni ve stavovem radku.
    a.on_msg(ServerMsg::Tasks {
        tasks: vec![task(1, State::Failed { code: 1, message: "spatne".into() }, 0), task(2, State::Running, 0)],
    });
    assert!(a.frame().contains("úloha uloha 1: chyba: spatne"), "{}", a.frame());
    assert!(a.frame().contains("úlohy 1"));
    // Upozorneni po case zmizi.
    for _ in 0..60 {
        a.tick(None);
    }
    assert!(!a.frame().contains("chyba: spatne"));
    let _ = Duration::from_millis(1);
}
