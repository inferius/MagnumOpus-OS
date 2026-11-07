//! Testy spravce uloh (na hostiteli: `cargo test -p taskview`).

use guiproto::a11y::{Role, actions as a11y_actions, state};
use taskproto::{Error, Request, State};
use ui::App;

use crate::*;

fn ui_for(m: &TaskManager<TestBackend>) -> ui::Ui<Msg> {
    let mut u = ui::Ui::new(gfx_text(), 560, 620);
    u.rebuild(m.title(), m.view());
    u
}

fn gfx_text() -> gfx::text::Text {
    gfx::text::Text::system()
}

fn names(u: &ui::Ui<Msg>, role: Role) -> Vec<String> {
    u.a11y().into_iter().filter(|n| n.role == role).map(|n| n.name).collect()
}

fn press(m: &mut TaskManager<TestBackend>, u: &mut ui::Ui<Msg>, name: &str) -> bool {
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

fn select(m: &mut TaskManager<TestBackend>, u: &mut ui::Ui<Msg>, prefix: &str) {
    let i = m.visible().iter().position(|t| TaskManager::<TestBackend>::row_text(t).contains(prefix)).unwrap();
    m.update(Msg::Select(i));
    u.rebuild(m.title(), m.view());
}

#[test]
fn agent_tasks_are_marked_and_sorted_first() {
    let m = TaskManager::new(TestBackend::default());
    let u = ui_for(&m);
    assert_eq!(m.title(), "Správce úloh (4 běží)");
    let items = names(&u, Role::ListItem);
    assert!(items[0].starts_with("[AGENT AI] Refaktorace"), "{items:?}");
    assert!(items.last().unwrap().contains("hotovo"), "dokoncene dole");
    // Vyrazny stitek v zahlavi.
    assert!(names(&u, Role::Label).iter().any(|l| l == "Agent AI: 1 běžících úloh"));
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
    assert!(names(&u, Role::Label).iter().any(|l| l == "Spustil agent AI (asistent)"));
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
