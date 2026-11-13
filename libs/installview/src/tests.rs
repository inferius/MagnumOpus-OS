//! Testy grafickeho instalatoru (na hostiteli: `cargo test -p installview`)
//! s falesnou sluzbou installd nad skutecnym planem z `installplan`.

use alloc::rc::Rc;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

use guiproto::a11y::{Role, actions, state};
use installplan::proto::{DiskEntry, KeyboardEntry, Reply, Request};
use installplan::{DiskInfo, Mode, choices, map, plan, preview_text};
use ui::App;

use crate::*;

const GIB: u64 = 1 << 30;

type Outbox = Rc<RefCell<Vec<Vec<u8>>>>;

fn target() -> DiskInfo {
    DiskInfo { index: 1, model: String::from("QEMU NVMe Ctrl"), sectors: 64 * GIB / 512, sector_size: 512, live: false }
}

/// Falesny installd: dva disky (zive USB a prazdne NVMe), dve rozlozeni.
fn installd(req: &[u8]) -> Vec<Reply> {
    let d = target();
    match Request::decode(req).unwrap() {
        Request::Keyboards => vec![Reply::Keyboards(vec![
            KeyboardEntry {
                name: "cz".into(),
                description: "Česká (QWERTZ)".into(),
                rows: vec!["q w e r t z".into()],
                keys: vec![(21, [Some('z'), Some('Z'), None, None]), (3, [Some('ě'), Some('2'), Some('@'), None])],
            },
            KeyboardEntry {
                name: "us".into(),
                description: "Anglická (US)".into(),
                rows: vec!["q w e r t y".into()],
                keys: vec![(21, [Some('y'), Some('Y'), None, None]), (3, [Some('2'), Some('@'), None, None])],
            },
        ])],
        Request::Disks => vec![Reply::Disks(vec![
            DiskEntry { index: 0, model: "USB stick".into(), bytes: 8 * GIB, live: true, parts: vec!["MO-LOGY".into()], choices: vec![] },
            DiskEntry { index: 1, model: d.model.clone(), bytes: d.bytes(), live: false, parts: vec![], choices: choices(&d, None) },
        ])],
        Request::Preview { mode, .. } => {
            let p = plan(&d, None, mode, [[1; 16], [2; 16], [3; 16]]).unwrap();
            vec![Reply::Preview {
                text: preview_text(&p),
                type_name: Some(d.model.clone()),
                other_os: p.other_os,
                lines: p.preview.clone(),
                before: map::now(&d, None),
                after: map::after(&d, None, &p),
            }]
        }
        Request::Install { .. } => vec![
            Reply::Progress { step: 0, text: "Zapisuji tabulku oddílů".into() },
            Reply::Progress { step: 3, text: "Vytvářím souborový systém".into() },
            Reply::Done("Hotovo. Vyjmi USB a restartuj.".into()),
        ],
    }
}

struct Rig {
    m: Installer,
    u: ui::Ui<Msg>,
    out: Outbox,
    /// Vsechny pozadavky (kontrola obsahu instalace).
    sent: Vec<Request>,
}

fn rig_sized(w: u32) -> Rig {
    let out: Outbox = Rc::default();
    let o = out.clone();
    let m = Installer::new(move |b| o.borrow_mut().push(b));
    let u = ui::Ui::new(ui::Look::ui_fonts(), w, 600);
    let mut r = Rig { m, u, out, sent: Vec::new() };
    r.serve();
    r
}

fn rig() -> Rig {
    rig_sized(900)
}

impl Rig {
    /// Odpovi na vsechny cekajici pozadavky (i ty, ktere vzniknou).
    fn serve(&mut self) {
        loop {
            let reqs: Vec<Vec<u8>> = self.out.borrow_mut().drain(..).collect();
            if reqs.is_empty() {
                break;
            }
            for q in reqs {
                self.sent.push(Request::decode(&q).unwrap());
                for r in installd(&q) {
                    self.m.reply(&r.encode());
                }
            }
        }
        self.u.refresh(&self.m);
    }

    fn node(&self, role: Role, name: &str) -> guiproto::a11y::Node {
        self.u.a11y().into_iter().find(|n| n.role == role && n.name == name).unwrap_or_else(|| panic!("{role:?} {name} chybi: {:?}", self.names()))
    }

    fn names(&self) -> Vec<String> {
        self.u.a11y().into_iter().map(|n| n.name).filter(|n| !n.is_empty()).collect()
    }

    fn enabled(&self, name: &str) -> bool {
        self.node(Role::Button, name).state & state::DISABLED == 0
    }

    fn act(&mut self, role: Role, name: &str, action: u32, value: &str) {
        let n = self.node(role, name);
        let r = self.u.a11y_action(n.id, action, value);
        for msg in r.msgs {
            self.m.update(msg);
        }
        self.serve();
    }

    fn press(&mut self, name: &str) {
        assert!(self.enabled(name), "{name} je zakazane v kroku {}", self.m.step);
        self.act(Role::Button, name, actions::ACTIVATE, "");
    }

    fn type_into(&mut self, placeholder: &str, text: &str) {
        let n = self.u.a11y().into_iter().find(|n| matches!(n.role, Role::TextInput | Role::PasswordInput) && n.name == placeholder).unwrap();
        let r = self.u.a11y_action(n.id, actions::SET_VALUE, text);
        for msg in r.msgs {
            self.m.update(msg);
        }
        self.serve();
    }
}

#[test]
fn whole_flow_to_installed() {
    let mut r = rig();
    assert_eq!(r.m.title(), "Instalace MagnumOpus");
    // 1. Klavesnice: hlavni cz, pridat us, pasmo vychozi.
    assert_eq!(r.m.chosen, vec![0]);
    assert!(!r.enabled("Zpět"));
    r.m.update(Msg::KbCursor(vec![1]));
    r.serve();
    assert!(r.names().iter().any(|n| n == "q w e r t y"), "nahled klavesnice");
    r.press("Přidat");
    assert_eq!(r.m.settings().keyboards, vec!["cz", "us"]);
    r.press("Pokračovat");
    // 2. Kontrola: zive USB je oznacene, pokracovat jde.
    assert!(r.names().iter().any(|n| n.contains("živé USB")));
    r.press("Pokračovat");
    // 3. Jediny cilovy disk je vybrany sam, zpusob prvni dostupny.
    assert_eq!(r.m.disk, Some(1));
    assert_eq!(r.m.mode, Some(Mode::WholeDisk));
    r.press("Pokračovat");
    // 4. Nahled: graf (useky teď a po instalaci) a opsani nazvu disku.
    let p = r.m.preview.clone().unwrap();
    assert_eq!(p.after.len(), 2);
    // Graf oddilu: kazdy usek je polozka seznamu ve stromu pristupnosti
    // (cely popisek, i kdyz se do pruhu nevejde).
    let items = r.u.a11y().into_iter().filter(|n| n.role == Role::ListItem).map(|n| n.name).collect::<Vec<_>>();
    assert!(items.iter().any(|n| n.starts_with("volné místo")), "{items:?}");
    assert!(items.iter().any(|n| n.starts_with("MagnumOpus") && n.ends_with("(nový)")), "{items:?}");
    assert!(!r.enabled("Pokračovat"));
    r.type_into("QEMU NVMe Ctrl", "QEMU NVMe");
    assert!(!r.enabled("Pokračovat"));
    r.type_into("QEMU NVMe Ctrl", "QEMU NVMe Ctrl");
    r.press("Pokračovat");
    // 5. Ucet: jmeno a heslo podle pravidel installplan.
    r.type_into("jan", "Jan");
    assert!(r.names().iter().any(|n| n.contains("začíná písmenem")), "chyba jmena");
    r.type_into("jan", "jan");
    // Pole hesla prijmou jen mistni vstup (set_value pres pristupnost
    // knihovna odmita): hodnoty jako zpravy pole.
    let ids: Vec<u32> = r.u.a11y().into_iter().filter(|n| n.role == Role::PasswordInput).map(|n| n.id).collect();
    assert_eq!(ids.len(), 3);
    assert!(r.u.a11y_action(ids[0], actions::SET_VALUE, "x").msgs.is_empty(), "set_value na heslo odmitnuto");
    for m in [Msg::Password("dlouhe heslo".into()), Msg::Password2("dlouhe hesl".into())] {
        r.m.update(m);
    }
    r.serve();
    assert!(!r.enabled("Pokračovat"), "hesla se neshoduji");
    assert!(r.names().iter().any(|n| n == "Hesla se neshodují"));
    r.m.update(Msg::Password2("dlouhe heslo".into()));
    r.serve();
    // Heslo nikdy ve stromu pristupnosti.
    assert!(r.u.a11y().iter().all(|n| !n.value.contains("dlouhe")));
    r.press("Pokračovat");
    // 6. Zabezpeceni: sifrovani zasedle s vysvetlenim.
    let enc = r.node(Role::Checkbox, "Šifrovat disk");
    assert!(enc.state & state::DISABLED != 0);
    assert!(r.names().iter().any(|n| n.contains("šifrování disku se připravuje")));
    r.press("Pokračovat");
    // 7. Souhrn a instalace.
    assert!(r.names().iter().any(|n| n == "cz, us"));
    r.press("Nainstalovat");
    assert_eq!(r.m.step, step::PROGRESS);
    let Some(Request::Install { disk, mode, confirm, settings, account }) = r.sent.last().cloned() else { panic!("{:?}", r.sent) };
    assert_eq!((disk, mode, confirm.as_str()), (1, Mode::WholeDisk, "QEMU NVMe Ctrl"));
    assert!(settings.contains("klavesnice = [\"cz\", \"us\"]"), "{settings}");
    assert_eq!((account.name.as_str(), account.display.as_str(), account.password.as_str()), ("jan", "jan", "dlouhe heslo"));
    assert_eq!(r.m.outcome, Some(Outcome::Done("Hotovo. Vyjmi USB a restartuj.".into())));
    assert_eq!(r.m.percent(), 100);
    assert!(r.names().iter().any(|n| n == "Nainstalováno"));
    assert!(r.u.a11y().iter().all(|n| n.name != "Zpět"), "behem a po instalaci bez tlacitek");
}

#[test]
fn default_keyboard_from_settings() {
    // installd posila us pred cz (poradi keymap); vychozi je cz.
    let out: Outbox = Rc::default();
    let o = out.clone();
    let mut m = Installer::new(move |b| o.borrow_mut().push(b));
    let kb = |n: &str| KeyboardEntry { name: n.into(), description: n.into(), rows: vec![], keys: vec![] };
    m.reply(&Reply::Keyboards(vec![kb("us"), kb("cz")]).encode());
    assert_eq!(m.chosen, vec![1]);
    assert_eq!(m.settings().keyboards, vec!["cz"]);
}

#[test]
fn try_field_uses_selected_layout() {
    let mut r = rig();
    // cz (index 0): klavesa 21 = z, 3 = e s hackem, AltGr+3 = @.
    assert_eq!(r.m.translate(21, 0), Some('z'));
    assert_eq!(r.m.translate(21, 1 << 5), Some('Z'), "Caps Lock u pismene");
    assert_eq!(r.m.translate(21, (1 << 5) | 1), Some('z'), "Caps Lock + Shift");
    // Ceska horni rada: Caps Lock pise velke pismeno, se Shiftem cislici.
    assert_eq!(r.m.translate(3, 1 << 5), Some('Ě'));
    assert_eq!(r.m.translate(3, (1 << 5) | 1), Some('2'));
    assert_eq!(r.m.translate(3, 1 << 3), Some('@'));
    assert_eq!(r.m.translate(99, 0), None);
    // Vybrane us: stejna klavesa pise y.
    r.m.update(Msg::KbCursor(vec![1]));
    assert_eq!(r.m.translate(21, 0), Some('y'));
    assert_eq!(r.m.translate(3, 1), Some('@'));
}

/// Zkusebni pole: skutecna udalost klavesy pres `Ui` pise podle
/// vybraneho rozlozeni (cz: klavesa 21 = z), znak od systemu (us: y)
/// se ignoruje; po prepnuti nahledu na us pise y.
#[test]
fn try_field_types_in_selected_layout() {
    use guiproto::window::InputEvent;
    let mut r = rig();
    let type_key = |r: &mut Rig, code: u16, sys: char| {
        let n = r.node(Role::TextInput, "Příliš žluťoučký kůň úpěl ďábelské ódy");
        let f = r.u.a11y_action(n.id, actions::FOCUS, "");
        assert!(f.msgs.is_empty());
        let res = r.u.event(InputEvent::Key { code, action: 1, ch: Some(sys) }, 0);
        let res2 = r.u.event(InputEvent::Text(sys), 0);
        for m in res.msgs.into_iter().chain(res2.msgs) {
            r.m.update(m);
        }
        r.serve();
    };
    assert!(r.names().iter().any(|n| n == "Vyzkoušej klávesnici (Česká (QWERTZ))"));
    type_key(&mut r, 21, 'y');
    type_key(&mut r, 3, '2');
    assert_eq!(r.m.try_text, "zě");
    r.m.update(Msg::KbCursor(vec![1]));
    r.serve();
    type_key(&mut r, 21, 'z');
    assert_eq!(r.m.try_text, "zěy");
    // Zkouska nic nemeni: zvolene rozlozeni zustava.
    assert_eq!(r.m.settings().keyboards, vec!["cz"]);
}

#[test]
fn keyboard_main_and_remove() {
    let mut r = rig();
    r.m.update(Msg::KbCursor(vec![1]));
    r.m.update(Msg::KbMain);
    assert_eq!(r.m.chosen, vec![1, 0]);
    r.m.update(Msg::KbRemove);
    assert_eq!(r.m.chosen, vec![0]);
    // Posledni rozlozeni odebrat nejde.
    r.m.update(Msg::KbCursor(vec![0]));
    r.m.update(Msg::KbRemove);
    assert_eq!(r.m.chosen, vec![0]);
}

#[test]
fn compact_window_shows_step_heading() {
    let r = rig_sized(420);
    assert!(r.names().iter().any(|n| n == "Krok 1 z 8: Jazyk a klávesnice"));
    assert!(r.u.a11y().iter().all(|n| n.role != Role::List), "bez seznamu kroku");
    let wide = rig();
    assert!(wide.u.a11y().iter().any(|n| n.role == Role::List));
    assert!(wide.names().iter().all(|n| !n.starts_with("Krok 1 z 8")));
}

#[test]
fn failure_is_shown() {
    let mut r = rig();
    r.press("Pokračovat");
    r.press("Pokračovat");
    // Nahled installd odmitne: chyba je videt, pokracovat nejde.
    r.m.update(Msg::Next);
    r.out.borrow_mut().clear();
    r.m.reply(&Reply::Failed(installplan::proto::Error::NotFound, "disk zmizel".into()).encode());
    r.u.refresh(&r.m);
    assert_eq!(r.m.step, step::PREVIEW);
    assert!(r.names().iter().any(|n| n == "disk zmizel"));
    assert!(!r.enabled("Pokračovat"));
    // Zpet a znovu: novy pozadavek, chyba zmizi.
    r.press("Zpět");
    r.press("Pokračovat");
    assert!(r.m.error.is_empty());
    assert!(r.m.preview.is_some());
}

#[test]
fn autopilot_walks_all_steps() {
    let mut r = rig();
    let text = "disk = \"QEMU NVMe Ctrl\"\nzpusob = \"cely-disk\"\npotvrzeni = \"QEMU NVMe Ctrl\"\njmeno = \"jan\"\nheslo = \"dlouhe heslo\"\nklavesnice = [\"us\", \"cz\"]\n";
    r.m.autopilot(installplan::answers::parse(text).unwrap());
    for _ in 0..7 {
        r.m.reply(AUTO_STEP);
        r.serve();
    }
    let log = r.m.log.join("\n");
    for i in 1..=8 {
        assert!(log.contains(&format!("gui-install: krok {i} ")), "{log}");
    }
    assert!(log.contains("gui-install: HOTOVO;"), "{log}");
    assert_eq!(r.m.settings().keyboards, vec!["us", "cz"]);
    assert!(!log.contains("stoji"), "{log}");
}
