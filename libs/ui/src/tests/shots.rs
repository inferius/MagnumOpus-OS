//! Snimkove testy vzhledu: komponenty se vykresli na hostiteli (svetly,
//! tmavy a vysoky kontrast) a porovnaji s referencnimi PNG v
//! `libs/ui/tests/shots/`.
//!
//! - Porovnani s malou toleranci (vyhlazovani pisma): kanal se smi lisit
//!   o [`TOL`], lisit se smi nejvys [`MAX_BAD`] promile pixelu.
//! - Pri selhani se do `target/shots/` ulozi `<jmeno>-actual.png`
//!   a `<jmeno>-diff.png` (cervene = rozdil).
//! - Reference se meni jen vyslovne: `cargo xtask test --update-shots`
//!   (nastavi `UPDATE_SHOTS=1`). Chybejici reference je chyba, ne novy
//!   snimek.
//! - Kresleni je deterministicke (celociselna geometrie, rasterizace pisma
//!   v f32 bez FMA), stejne na x86_64 i aarch64.

extern crate std;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use std::path::PathBuf;

use gfx::Canvas;

use crate::*;

/// Tolerance kanalu a nejvic promile lisicich se pixelu.
const TOL: u32 = 24;
const MAX_BAD: usize = 2;

#[derive(Clone, Debug, PartialEq)]
enum M {}

fn modes() -> [(theme::Mode, &'static str); 3] {
    [(theme::Mode::Light, "svetly"), (theme::Mode::Dark, "tmavy"), (theme::Mode::Contrast, "kontrast")]
}

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("shots")
}

fn out_dir() -> PathBuf {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/shots");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Vykresli `view` v rezimu `mode` do pixelu `w` x `h` (s fokusem
/// a najetim podle `prep`).
fn render(view: &dyn Fn() -> Node<M>, mode: theme::Mode, w: u32, h: u32, prep: &dyn Fn(&mut Ui<M>)) -> Vec<u32> {
    let mut u: Ui<M> = Ui::new(crate::Look::ui_fonts(), w, h);
    u.look.set_mode(mode);
    u.rebuild("s".into(), view());
    prep(&mut u);
    let mut px = vec![0u32; (w * h) as usize];
    let mut c = Canvas::new(&mut px, w, h, w).unwrap();
    u.draw(&mut c, 0);
    px
}

fn ch(p: u32, s: u32) -> u32 {
    p >> s & 0xFF
}

/// Porovna snimek `name` ve vsech rezimech s referenci.
fn shot(name: &str, w: u32, h: u32, view: &dyn Fn() -> Node<M>, prep: &dyn Fn(&mut Ui<M>)) {
    let update = std::env::var("UPDATE_SHOTS").is_ok_and(|v| v == "1");
    let mut errors: Vec<String> = Vec::new();
    for (mode, mname) in modes() {
        let px = render(view, mode, w, h, prep);
        // Snimek je deterministicky: dvakrat stejne.
        assert_eq!(px, render(view, mode, w, h, prep), "{name}-{mname}: kresleni neni deterministicke");
        let file = dir().join(format!("{name}-{mname}.png"));
        if update {
            std::fs::create_dir_all(dir()).unwrap();
            std::fs::write(&file, gfx::png::encode(&px, w, h, w)).unwrap();
            continue;
        }
        let Ok(data) = std::fs::read(&file) else {
            errors.push(format!("{name}-{mname}: chybi reference {} (vytvori ji `cargo xtask test --update-shots`)", file.display()));
            continue;
        };
        let Some((rw, rh, refpx)) = gfx::png::decode(&data) else {
            errors.push(format!("{name}-{mname}: reference neni platne PNG"));
            continue;
        };
        let bad: Vec<bool> = if (rw, rh) != (w, h) {
            vec![true; (w * h) as usize]
        } else {
            px.iter()
                .zip(&refpx)
                .map(|(&a, &b)| [16, 8, 0].iter().any(|&s| ch(a, s).abs_diff(ch(b, s)) > TOL))
                .collect()
        };
        let n = bad.iter().filter(|&&b| b).count();
        if n * 1000 > bad.len() * MAX_BAD || (rw, rh) != (w, h) {
            let o = out_dir();
            let _ = std::fs::write(o.join(format!("{name}-{mname}-actual.png")), gfx::png::encode(&px, w, h, w));
            let diff: Vec<u32> = px.iter().zip(&bad).map(|(&p, &b)| if b { 0xFFFF_0000 } else { 0xFF00_0000 | ((ch(p, 16) + ch(p, 8) + ch(p, 0)) / 6) * 0x01_0101 }).collect();
            let _ = std::fs::write(o.join(format!("{name}-{mname}-diff.png")), gfx::png::encode(&diff, w, h, w));
            errors.push(format!("{name}-{mname}: lisi se {n} z {} pixelu (rozdil v {})", bad.len(), o.display()));
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

fn none(_: &mut Ui<M>) {}

#[test]
fn shot_buttons_and_fields() {
    shot(
        "pole",
        360,
        300,
        &|| {
            column([
                row([button::<M>("Uložit"), button("Zrušit").disabled(true), spacer()]),
                text_input("Žluťoučký kůň").placeholder("Název"),
                text_input("").placeholder("Hledat…"),
                password_input("tajne").placeholder("Heslo").error("Heslo je příliš krátké"),
                text_area("První řádek\nDruhý řádek s delším textem, který se zalomí").rows(2),
            ])
        },
        &none,
    );
}

#[test]
fn shot_choices() {
    shot(
        "volby",
        360,
        340,
        &|| {
            column([
                checkbox::<M>("Zaškrtnuto", true),
                checkbox("Nezaškrtnuto", false),
                switch("Wi-Fi", true),
                switch("Bluetooth", false),
                row([radio("Světlý", true), radio("Tmavý", false), spacer()]),
                slider(40, 0, 100),
                segmented(["Vše".into(), "Běží".into(), "Agent".into()], 1),
            ])
        },
        &none,
    );
}

#[test]
fn shot_table() {
    shot(
        "tabulka",
        360,
        220,
        &|| {
            column([table::<M>(vec![col("Název", 0).sortable(), col("Velikost", 90).numeric()], 40, |i| i as u64, |i| {
                Cells::new([format!("dokument {i}.txt"), format!("{} kB", i * 13)]).icon(if i % 3 == 0 { "folder" } else { "file" })
            })
            .selection(vec![1, 2])
            .sorted(0, true)
            .multi_select(true)
            .height(200)])
        },
        &none,
    );
}

#[test]
fn shot_tabs_select_and_charts() {
    shot(
        "zalozky-graf",
        420,
        260,
        &|| {
            column([
                tabs::<M>(["Relace 1".into(), "Relace 2".into()], 0),
                select(["Čeština".into(), "English".into()], 0),
                progress(60),
                segments(vec![Segment::new(1, "EFI", Tone::Muted), Segment::new(48, "Windows C:", Tone::Other), Segment::new(30, "MagnumOpus", Tone::Accent), Segment::new(21, "Volné", Tone::Free)]).legend(true),
            ])
        },
        &none,
    );
}

/// Otevrena nabidka nad oknem.
#[test]
fn shot_menu() {
    shot(
        "nabidka",
        320,
        240,
        &|| {
            column([button::<M>("Soubor").context_menu(vec![
                MenuEntry { label: "Kopírovat".into(), shortcut: "Ctrl+C".into(), disabled: String::new(), checked: None, separator: false, act: None },
                MenuEntry { label: "Vložit".into(), shortcut: "Ctrl+V".into(), disabled: "Schránka je prázdná".into(), checked: None, separator: false, act: None },
                menu_separator(),
                MenuEntry { label: "Zobrazit skryté".into(), shortcut: String::new(), disabled: String::new(), checked: Some(true), separator: false, act: None },
            ])])
        },
        &|u| {
            u.event(guiproto::window::InputEvent::Button { code: inputproto::code::BTN_RIGHT, pressed: true, x: 30, y: 30 }, 0);
        },
    );
}
