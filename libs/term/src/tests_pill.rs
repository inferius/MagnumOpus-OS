//! Pilulka slozky ve vyzve okna GUI (`ui::pill`): text radku vstupu
//! zacina pevnou mezerou za nakreslenou pilulkou pro kratkou, dlouhou
//! i zkracenou cestu a s diakritikou; kurzor a klik mysi sedi se znaky.
//! Jmeno se meri skutecnym pismem UI (Inter, jako okno `gui-term`).

use alloc::string::String;
use alloc::vec::Vec;

use crate::proto::ServerMsg;
use crate::theme::Theme;
use crate::ui::draw::Role;
use crate::ui::pill::{PillMetrics, cwd_pill, name_room};
use crate::ui::{App, Key};

struct Tokens {
    m: PillMetrics,
    small: f32,
    weight: u16,
    lead_px: u32,
}

fn tokens() -> Tokens {
    let t = theme::Theme::builtin(theme::Mode::Light);
    let sp = &t.space;
    Tokens {
        m: PillMetrics { pad: sp[3], icon: t.size.icon_s, icon_gap: sp[1], text_gap: sp[3] },
        small: t.text.small.size,
        weight: t.text.body_strong.weight,
        // Jako `Look::cwd_lead` v gui-term: okraj, ikona, mezera s1 a s3.
        lead_px: 2 * sp[3] + t.size.icon_s + sp[1] + sp[3],
    }
}

/// Aplikace s vyzvou okna GUI ve slozce `cwd` a textem `text` v editoru.
fn app(cwd: &str, text: &str, cw: u32, tk: &Tokens) -> App {
    let mut a = App::new(Theme::dark(), 80, 30);
    a.on_msg(ServerMsg::Attached { session: 1, cwd: cwd.into(), blocks: 0 });
    a.cwd_lead = (tk.lead_px.div_ceil(cw) as usize).saturating_sub(3).max(1);
    for c in text.chars() {
        a.key(Key::Char(c));
    }
    a
}

/// (zobrazene jmeno, bunka konce vyzvy) z radku vstupu.
fn prompt(a: &App) -> (String, usize) {
    let lines = a.draw();
    let l = lines.iter().find(|l| matches!(l.role, Role::Input { .. })).expect("radek vstupu");
    let Role::Input { prefix, cwd } = &l.role else { unreachable!() };
    (cwd.clone(), l.part_cols()[prefix - 1].1)
}

const CASES: &[&str] = &[
    "/home/alice",
    "/home/alice/apps/org.magnumopus.files",
    "/home/alice/Dokumenty/Úlohy/čeština",
    // Delsi nez tretina sirky: `short_path` zkrati zprostredka.
    "/home/alice/Dokumenty/Projekty/MagnumOpus/dokumentace/návrhy/terminál",
];

#[test]
fn text_zacina_pevnou_mezerou_za_pilulkou() {
    let tk = tokens();
    let fonts = ui::Look::ui_fonts();
    let face = fonts.face_ref(tk.weight);
    let mut gaps_before = Vec::new();
    for cw in [8u32, 9, 10] {
        for cwd in CASES {
            let a = app(cwd, "cd úlohy/", cw, &tk);
            let (name, end) = prompt(&a);
            let name_w = face.width(&name, tk.small);
            let p = cwd_pill(name_w, end, cw, &tk.m);
            // Prvni znak textu (bunka `end` = `input_col(0)`) na x
            // `indent + end * cw` (od leveho okraje mrizky).
            assert_eq!(a.input_col(0), end, "{cwd}");
            let text_x = p.indent + (end as u32 * cw) as i32;
            assert_eq!(text_x, (p.width + tk.m.text_gap) as i32, "{cwd} cw={cw}: mezera za pilulkou");
            assert!(p.indent <= 0, "{cwd}: text se nesmi posunout doprava");
            // Jmeno se do pilulky vejde cele (bez vypustky), i zkracene.
            assert!(name_w <= name_room(p.width, &tk.m), "{cwd} cw={cw}: {name} {name_w} px > {}", name_room(p.width, &tk.m));
            // Puvodni mezera (text az na bunce `end`).
            gaps_before.push((cwd, end as u32 * cw - p.width));
        }
    }
    // Chyba, kterou to opravuje: u dlouhe cesty byla mezera mnohem vetsi
    // nez s3 (jmeno v proporcionalnim pismu je uzsi nez bunky).
    let long = gaps_before.iter().filter(|(c, _)| c.contains("org.magnumopus")).map(|g| g.1).min().unwrap();
    assert!(long > 3 * tk.m.text_gap, "dlouha cesta: puvodni mezera {long} px");
    // Zkracena cesta je ve vyzve opravdu zkracena.
    let a = app(CASES[3], "", 8, &tk);
    assert!(prompt(&a).0.contains('\u{2026}'), "zkraceni: {}", prompt(&a).0);
}

#[test]
fn kurzor_a_klik_sedi_se_znaky() {
    let tk = tokens();
    let fonts = ui::Look::ui_fonts();
    let face = fonts.face_ref(tk.weight);
    let text = "cd úlohy/čtení";
    for cwd in CASES {
        let cw = 9;
        let mut a = app(cwd, text, cw, &tk);
        let (name, end) = prompt(&a);
        let p = cwd_pill(face.width(&name, tk.small), end, cw, &tk.m);
        for (k, (byte, _)) in text.char_indices().enumerate() {
            // Levy okraj znaku `k` v okne a klik do jeho stredu.
            let col = a.input_col(byte);
            assert_eq!(col, end + k, "{cwd}: sloupec znaku {k}");
            let x = p.indent + (col as u32 * cw) as i32;
            let click = x + cw as i32 / 2;
            let hit = ((click - p.indent).max(0) as u32 / cw) as usize;
            a.click_input(hit);
            assert_eq!(a.editor.cursor, byte, "{cwd}: klik na znak {k}");
        }
        // Klik za konec textu = kurzor na konec.
        a.click_input(end + text.chars().count() + 5);
        assert_eq!(a.editor.cursor, text.len());
    }
}

#[test]
fn pilulka_nejvys_po_konec_vyzvy() {
    let m = PillMetrics { pad: 12, icon: 16, icon_gap: 4, text_gap: 12 };
    // Jmeno sirsi nez misto: pilulka konci s3 pred bunkou `end`.
    let p = cwd_pill(10_000, 20, 8, &m);
    assert_eq!(p.width, 20 * 8 - 12);
    assert_eq!(p.indent, 0);
    // Uzke jmeno: text tesne za pilulkou.
    let p = cwd_pill(30, 20, 8, &m);
    assert_eq!(p.width, 12 + 12 + 16 + 4 + 30);
    assert_eq!(p.indent, (p.width + 12) as i32 - 160);
    assert_eq!(name_room(p.width, &m), 30);
}
