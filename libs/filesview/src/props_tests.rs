//! Testy panelu Vlastnosti a karty Opravneni: `cargo test -p filesview`.

extern crate std;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use fsproto::perm::{ACL_PROTECTED, Acl, AclView, EVERYONE, Grant, InheritedGrant, Permissions, grant, rights};
use guiproto::a11y::{Role, actions as a11y_actions, state};
use ui::App;

use crate::app::{FileManager, Msg};
use crate::backend::TestBackend;
use crate::props::{Ask, GROUP_USERS, Level, PropsMsg};

type Fm = FileManager<TestBackend>;

const JANA: u64 = 1000;
const PETR: u64 = 1001;

fn fm_with(f: impl FnOnce(&mut TestBackend)) -> Fm {
    let mut b = TestBackend::sample();
    f(&mut b);
    FileManager::new(b)
}

fn ui_for(m: &Fm) -> ui::Ui<Msg> {
    let mut u = ui::Ui::new(ui::Look::ui_fonts(), 1100, 900);
    u.refresh(m);
    u
}

fn nodes(u: &ui::Ui<Msg>, role: Role) -> Vec<(String, u32)> {
    u.a11y().into_iter().filter(|n| n.role == role).map(|n| (n.name, n.state)).collect()
}

fn has_label(u: &ui::Ui<Msg>, text: &str) -> bool {
    nodes(u, Role::Label).iter().any(|(n, _)| n == text)
}

fn press(m: &mut Fm, u: &mut ui::Ui<Msg>, name: &str) {
    let n = u
        .a11y()
        .into_iter()
        .find(|n| n.role == Role::Button && n.name == name)
        .unwrap_or_else(|| panic!("tlacitko {name}: {:?}", nodes(u, Role::Button)));
    assert_eq!(n.state & state::DISABLED, 0, "{name} je zakazane");
    for msg in u.a11y_action(n.id, a11y_actions::ACTIVATE, "").msgs {
        m.update(msg);
    }
    u.refresh(m);
}

fn open(m: &mut Fm, item: Option<&str>) {
    let sel = item.map(|n| m.listing.items.iter().find(|i| i.e.name == n).unwrap_or_else(|| panic!("{n}")).e.oid);
    m.update(Msg::Select(sel.into_iter().collect()));
    m.update(Msg::Properties);
    assert!(m.props.is_some(), "panel Vlastnosti");
}

fn view(owner: u64, mine: u8, own: Option<Acl>, inherited: &[(u64, u8, &str)]) -> AclView {
    AclView {
        perms: Permissions { owner, rights: mine, acl: own },
        inherited: inherited.iter().map(|&(who, rights, from)| InheritedGrant { who, rights, from: from.to_string() }).collect(),
    }
}

fn acl(flags: u32, grants: &[(u64, u8, u8)]) -> Option<Acl> {
    Some(Acl { flags, grants: grants.iter().map(|&(who, rights, flags)| Grant { who, rights, flags }).collect() })
}

#[test]
fn levels_are_ascending_and_custom_is_kept() {
    assert_eq!(Level::of(rights::VIEW), Level::View);
    assert_eq!(Level::of(rights::RUN), Level::Run);
    assert_eq!(Level::of(rights::EDIT | rights::EXECUTE), Level::Edit);
    assert_eq!(Level::of(rights::ALL), Level::Manage);
    // Kazda uroven obsahuje predchozi.
    for w in Level::ALL.windows(2) {
        assert_eq!(w[0].rights() & w[1].rights(), w[0].rights(), "{:?} < {:?}", w[0], w[1]);
    }
    let odd = Level::of(rights::READ | rights::WRITE);
    assert_eq!(odd, Level::Custom(rights::READ | rights::WRITE));
    assert_eq!(odd.rights(), rights::READ | rights::WRITE);
    assert_eq!(odd.label(), "Vlastní (číst, měnit)");
    assert_eq!(odd.index(), 4);
}

#[test]
fn properties_open_with_general_and_owner() {
    let mut m = fm_with(|_| {});
    // Alt+Enter a nabidka aplikace.
    assert_eq!(crate::key_msg(false, crate::app::Mode::Folder, inputproto::code::KEY_ENTER, None, inputproto::mods::ALT), Some(Msg::Properties));
    assert!(crate::app_menu(false)[0].children.iter().any(|i| i.action == "files.properties"));
    open(&mut m, Some("zprava.txt"));
    let mut u = ui_for(&m);
    assert!(has_label(&u, "Vlastnosti: zprava.txt"), "{:?}", nodes(&u, Role::Label));
    assert!(has_label(&u, "Druh: Soubor"));
    assert!(has_label(&u, "Umístění: /home/jana"));
    // Tabulka vypisu ani nahled nejsou videt.
    assert!(nodes(&u, Role::Table).is_empty());
    m.update(Msg::Props(PropsMsg::Tab(1)));
    u.refresh(&m);
    assert!(has_label(&u, "Vlastník: Jana Nováková (vy)"), "{:?}", nodes(&u, Role::Label));
    assert!(has_label(&u, "Nikdo další nemá u této položky vlastní oprávnění."));
    // Escape zavre a vrati vypis.
    let esc = m.key(inputproto::code::KEY_ESC, None, 0);
    assert_eq!(esc, Some(Msg::Props(PropsMsg::Close)));
    m.update(esc.unwrap());
    assert!(m.props.is_none());
    u.refresh(&m);
    assert!(!nodes(&u, Role::Table).is_empty());
    // Bez vyberu: vlastnosti aktualni slozky.
    open(&mut m, None);
    assert_eq!(m.props.as_ref().unwrap().path, "/home/jana");
}

#[test]
fn add_share_with_inheritance_and_save() {
    let mut m = fm_with(|_| {});
    open(&mut m, Some("Dokumenty"));
    m.update(Msg::Props(PropsMsg::Tab(1)));
    let p = m.props.as_ref().unwrap();
    // Kandidati: bez vlastnika (Jana), pak Uzivatele a Vsichni.
    assert_eq!(p.candidates(m.names.as_deref().unwrap()), [PETR, GROUP_USERS, EVERYONE]);
    let mut u = ui_for(&m);
    assert_eq!(nodes(&u, Role::Button).iter().find(|b| b.0 == "Uložit oprávnění").unwrap().1 & state::DISABLED, state::DISABLED);
    m.update(Msg::Props(PropsMsg::AddWho(0)));
    m.update(Msg::Props(PropsMsg::AddLevel(2)));
    u.refresh(&m);
    press(&mut m, &mut u, "Přidat sdílení");
    // Ve slozce se novy radek dedi do obsahu (zaskrtnuto).
    let cb = nodes(&u, Role::Checkbox);
    let inh = cb.iter().find(|c| c.0 == "Petr Svoboda: i pro obsah složky").unwrap_or_else(|| panic!("{cb:?}"));
    assert_ne!(inh.1 & state::CHECKED, 0);
    press(&mut m, &mut u, "Uložit oprávnění");
    let (path, set) = m.b.acl_sets.last().cloned().unwrap();
    assert_eq!(path, "/home/jana/Dokumenty");
    assert_eq!(set, acl(0, &[(PETR, rights::EDIT | rights::EXECUTE, grant::BOTH)]));
    assert!(has_label(&u, "Oprávnění uložena."));
    // Jen pro slozku samotnou, ne pro obsah.
    m.update(Msg::Props(PropsMsg::Inherit(0, false)));
    u.refresh(&m);
    press(&mut m, &mut u, "Uložit oprávnění");
    assert_eq!(m.b.acl_sets.last().unwrap().1, acl(0, &[(PETR, rights::EDIT | rights::EXECUTE, grant::THIS)]));
    // Odebrat posledni radek = jen dedit (zadny vlastni popis).
    press(&mut m, &mut u, "Odebrat: Petr Svoboda");
    press(&mut m, &mut u, "Uložit oprávnění");
    assert_eq!(m.b.acl_sets.last().unwrap().1, None);
}

#[test]
fn inherited_rows_are_fixed_and_protect_copies_them() {
    let file = "/home/jana/Dokumenty/smlouva 2.txt";
    let mut m = fm_with(|b| {
        b.perms.insert(file.into(), view(JANA, rights::ALL, None, &[(PETR, rights::VIEW, "/home/jana/Dokumenty"), (EVERYONE, rights::RUN, "")]));
    });
    m.update(Msg::Go("/home/jana/Dokumenty".into()));
    open(&mut m, Some("smlouva 2.txt"));
    m.update(Msg::Props(PropsMsg::Tab(1)));
    let mut u = ui_for(&m);
    let labels = nodes(&u, Role::Label);
    let row = labels.iter().find(|l| l.0 == "Petr Svoboda: Zobrazit (zděděno ze složky /home/jana/Dokumenty)").unwrap_or_else(|| panic!("{labels:?}"));
    assert_ne!(row.1 & state::DISABLED, 0, "zdedeny radek je zde nemenny");
    assert!(labels.iter().any(|l| l.0 == "Všichni: Spouštět (zděděno mimo vaše místa)"));
    // Soubor: zadny prepinac "i pro obsah".
    assert!(nodes(&u, Role::Checkbox).iter().all(|c| !c.0.contains("i pro obsah")));
    // Vypnout dedeni: zeptat se, zkopirovat.
    m.update(Msg::Props(PropsMsg::Inheritance(false)));
    assert_eq!(m.props.as_ref().unwrap().ask, Some(Ask::Protect));
    u.refresh(&m);
    press(&mut m, &mut u, "Zkopírovat zděděná");
    press(&mut m, &mut u, "Uložit oprávnění");
    assert_eq!(
        m.b.acl_sets.last().unwrap().1,
        acl(ACL_PROTECTED, &[(PETR, rights::VIEW, grant::THIS), (EVERYONE, rights::RUN, grant::THIS)])
    );
    assert!(has_label(&u, "Dědění je vypnuté: platí jen oprávnění výše."), "{:?}", nodes(&u, Role::Label));
    // Zapnout zpet; "Ponechat dedeni" pri dalsim vypnuti nic nezmeni.
    m.update(Msg::Props(PropsMsg::Inheritance(true)));
    assert!(!m.props.as_ref().unwrap().protected);
    m.update(Msg::Props(PropsMsg::Revert));
    assert!(m.props.as_ref().unwrap().protected);
}

#[test]
fn without_manage_the_tab_is_read_only() {
    let mut m = fm_with(|b| {
        b.perms.insert("/home/jana/zprava.txt".into(), view(PETR, rights::VIEW, acl(0, &[(JANA, rights::VIEW, grant::THIS)]), &[]));
    });
    open(&mut m, Some("zprava.txt"));
    m.update(Msg::Props(PropsMsg::Tab(1)));
    let u = ui_for(&m);
    let buttons: Vec<String> = nodes(&u, Role::Button).into_iter().map(|b| b.0).collect();
    assert!(!buttons.iter().any(|b| b == "Uložit oprávnění" || b.starts_with("Odebrat") || b == "Přidat sdílení"), "{buttons:?}");
    assert!(nodes(&u, Role::Label).iter().any(|l| l.0.starts_with("Jen pro čtení: oprávnění může měnit vlastník")));
    assert!(nodes(&u, Role::Checkbox).iter().all(|c| c.1 & state::DISABLED != 0));
    // Zpravy se bez prava ignoruji.
    m.update(Msg::Props(PropsMsg::Remove(0)));
    m.update(Msg::Props(PropsMsg::Save));
    assert_eq!(m.props.as_ref().unwrap().shares.len(), 1);
    assert!(m.b.acl_sets.is_empty());
}

#[test]
fn custom_rights_survive_unrelated_change() {
    let odd = rights::READ | rights::WRITE;
    let mut m = fm_with(|b| {
        b.perms.insert("/home/jana/zprava.txt".into(), view(JANA, rights::ALL, acl(0, &[(PETR, odd, grant::THIS)]), &[]));
    });
    open(&mut m, Some("zprava.txt"));
    m.update(Msg::Props(PropsMsg::Tab(1)));
    m.update(Msg::Props(PropsMsg::AddWho(0)));
    m.update(Msg::Props(PropsMsg::Add));
    m.update(Msg::Props(PropsMsg::Save));
    assert_eq!(m.b.acl_sets.last().unwrap().1, acl(0, &[(PETR, odd, grant::THIS), (GROUP_USERS, rights::VIEW, grant::THIS)]));
}

#[test]
fn dropping_own_manage_asks_first() {
    let mut m = fm_with(|b| {
        b.perms.insert("/home/jana/zprava.txt".into(), view(PETR, rights::ALL, acl(0, &[(JANA, rights::ALL, grant::THIS)]), &[]));
    });
    open(&mut m, Some("zprava.txt"));
    m.update(Msg::Props(PropsMsg::Tab(1)));
    m.update(Msg::Props(PropsMsg::Level(0, 0)));
    let mut u = ui_for(&m);
    press(&mut m, &mut u, "Uložit oprávnění");
    assert_eq!(m.props.as_ref().unwrap().ask, Some(Ask::DropManage));
    assert!(m.b.acl_sets.is_empty());
    press(&mut m, &mut u, "Neukládat");
    assert!(m.b.acl_sets.is_empty());
    press(&mut m, &mut u, "Uložit oprávnění");
    press(&mut m, &mut u, "Přesto uložit");
    assert_eq!(m.b.acl_sets.last().unwrap().1, acl(0, &[(JANA, rights::VIEW, grant::THIS)]));
    // Vlastnik se neptá nikdy.
    let mut m = fm_with(|b| {
        b.perms.insert("/home/jana/zprava.txt".into(), view(JANA, rights::ALL, acl(0, &[(JANA, rights::ALL, grant::THIS)]), &[]));
    });
    open(&mut m, Some("zprava.txt"));
    m.update(Msg::Props(PropsMsg::Level(0, 0)));
    m.update(Msg::Props(PropsMsg::Save));
    assert_eq!(m.props.as_ref().unwrap().ask, None);
    assert_eq!(m.b.acl_sets.len(), 1);
}
