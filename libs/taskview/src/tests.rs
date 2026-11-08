//! Testy spravce uloh (na hostiteli: `cargo test -p taskview`).

use guiproto::a11y::{Role, actions as a11y_actions, state};
use taskproto::{Error, Request, State};
use ui::App;

use crate::*;

fn ui_for<B: Backend>(m: &TaskManager<B>) -> ui::Ui<Msg> {
    let mut u = ui::Ui::new(gfx_text(), 560, 620);
    u.rebuild(m.title(), m.view());
    u
}

fn gfx_text() -> gfx::text::Family {
    ui::Look::ui_fonts()
}

fn names(u: &ui::Ui<Msg>, role: Role) -> Vec<String> {
    u.a11y().into_iter().filter(|n| n.role == role).map(|n| n.name).collect()
}

fn press<B: Backend>(m: &mut TaskManager<B>, u: &mut ui::Ui<Msg>, name: &str) -> bool {
    let n = u.a11y().into_iter().find(|n| n.role == Role::Button && n.name == name).unwrap();
    if n.state & state::DISABLED != 0 {
        return false;
    }
    let r = u.a11y_action(n.id, a11y_actions::ACTIVATE, "");
    for msg in r.msgs {
        m.update(msg);
    }
    u.rebuild(m.title(), m.view());
    true
}

fn select<B: Backend>(m: &mut TaskManager<B>, u: &mut ui::Ui<Msg>, prefix: &str) {
    let i = m.visible().iter().position(|t| TaskManager::<B>::row_text(t).contains(prefix)).unwrap();
    m.update(Msg::Select(i));
    u.rebuild(m.title(), m.view());
}

#[test]
fn agent_tasks_are_marked_and_sorted_first() {
    let m = TaskManager::new(TestBackend::default());
    let u = ui_for(&m);
    assert_eq!(m.title(), "Správce úloh (4 běží)");
    let items = names(&u, Role::ListItem);
    assert!(items[0].starts_with("agent: Refaktorace"), "{items:?}");
    assert!(items.last().unwrap().contains("hotovo"), "dokoncene dole");
    // Vyrazny stitek v zahlavi.
    assert!(names(&u, Role::Label).iter().any(|l| l == "agent: Agent AI: 1 běžící úloha"));
    assert_eq!(running_tasks(3), "3 běžící úlohy");
    assert_eq!(running_tasks(5), "5 běžících úloh");
    assert_eq!(running_tasks(0), "0 běžících úloh");
}

#[test]
fn filters_only_agent_and_hide_done() {
    let mut m = TaskManager::new(TestBackend::default());
    m.update(Msg::OnlyAgent(true));
    assert_eq!(m.visible().len(), 1);
    m.update(Msg::OnlyAgent(false));
    m.update(Msg::ShowDone(false));
    assert!(m.visible().iter().all(|t| !t.state.finished()));
}

#[test]
fn actions_respect_permissions_and_state() {
    let mut m = TaskManager::new(TestBackend::default());
    let mut u = ui_for(&m);
    // Systemova uloha bez prav: vsechna tlacitka neaktivni.
    select(&mut m, &mut u, "Scrub");
    assert!(!press(&mut m, &mut u, "Zrušit"));
    assert!(names(&u, Role::Label).iter().any(|l| l.contains("chybí oprávnění")));
    // Uloha agenta: oznaceni v detailu, pozastavit a pokracovat.
    select(&mut m, &mut u, "Refaktorace");
    assert!(names(&u, Role::Label).iter().any(|l| l == "agent: Spustil agent AI (asistent)"));
    assert!(press(&mut m, &mut u, "Pozastavit"));
    assert_eq!(m.selected().unwrap().state, State::Paused);
    assert!(!press(&mut m, &mut u, "Pozastavit"), "pozastavena nejde pozastavit");
    assert!(press(&mut m, &mut u, "Pokračovat"));
    assert!(press(&mut m, &mut u, "Zrušit"));
    assert_eq!(m.selected().unwrap().state, State::Cancelled);
    assert_eq!(m.status, "Provedeno.");
    assert_eq!(m.list.running(), (3, 0));
    // Vypis.
    select(&mut m, &mut u, "Kopírování");
    assert!(press(&mut m, &mut u, "Výpis"));
    assert_eq!(m.log.len(), 2);
}

#[test]
fn taskd_decides_even_if_window_offers() {
    // Okno akci nenabidne, ale i primy pozadavek taskd odmitne.
    let mut b = TestBackend::default();
    let r = b.send(Request::Cancel { req: 9, id: 3 });
    assert_eq!(r, vec![taskproto::Reply::Result { req: 9, result: Err(Error::AccessDenied) }]);
    let r = b.send(Request::Resume { req: 9, id: 1 });
    assert_eq!(r, vec![taskproto::Reply::Result { req: 9, result: Err(Error::InvalidArgs) }]);
    let r = b.send(Request::Cancel { req: 9, id: 99 });
    assert_eq!(r, vec![taskproto::Reply::Result { req: 9, result: Err(Error::NotFound) }]);
}

#[test]
fn channel_backend_sends_requests_and_applies_replies() {
    use std::cell::RefCell;
    use std::rc::Rc;
    let sent: Rc<RefCell<Vec<Vec<u8>>>> = Rc::default();
    let s2 = sent.clone();
    let mut m = TaskManager::new(ChannelBackend::new(move |d| s2.borrow_mut().push(d)));
    // Prvni pozadavek je odber.
    assert!(matches!(Request::decode(&sent.borrow()[0]), Ok(Request::Subscribe { since: None, .. })));
    // Snimek z taskd prijde zpravou kanalu.
    let snap = taskproto::Reply::Snapshot { req: 1, seq: 1, tasks: TestBackend::default().tasks, last: true };
    assert!(m.external(&snap.encode()));
    assert_eq!(m.list.tasks.len(), 5);
    let mut u = ui_for(&m);
    select(&mut m, &mut u, "Kopírování");
    assert!(press(&mut m, &mut u, "Zrušit"));
    assert!(matches!(Request::decode(sent.borrow().last().unwrap()), Ok(Request::Cancel { id: 1, .. })));
    assert_eq!(m.status, "Čekám na službu úloh...");
    // Odmitnuti od taskd.
    assert!(m.external(&taskproto::Reply::Result { req: 2, result: Err(Error::AccessDenied) }.encode()));
    assert_eq!(m.status, "Odmítnuto: ACCESS_DENIED");
    assert!(!m.external(b"nesmysl"));
}
