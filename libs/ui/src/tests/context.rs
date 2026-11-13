//! Kontext okna: tridy sirky s hysterezi, kontext ve `view`.

use alloc::string::String;

use guiproto::a11y::Role;

use crate::*;

/// Prahy z tokenu (design/mobil.html): compact < 600, medium 600-839,
/// expanded >= 840; zpet az o 24 px pod prahem.
#[test]
fn size_class_thresholds_and_hysteresis() {
    let bp = theme::Theme::builtin(theme::Mode::Light).breakpoint;
    assert_eq!((bp.medium, bp.expanded, bp.hysteresis), (600, 840, 24));
    let of = |w, p| SizeClass::of(w, p, &bp);
    assert_eq!(of(599, None), SizeClass::Compact);
    assert_eq!(of(600, None), SizeClass::Medium);
    assert_eq!(of(839, None), SizeClass::Medium);
    assert_eq!(of(840, None), SizeClass::Expanded);
    // Nahoru hned od prahu.
    assert_eq!(of(600, Some(SizeClass::Compact)), SizeClass::Medium);
    // Dolu az pod prah minus hystereze.
    assert_eq!(of(576, Some(SizeClass::Medium)), SizeClass::Medium);
    assert_eq!(of(575, Some(SizeClass::Medium)), SizeClass::Compact);
    assert_eq!(of(816, Some(SizeClass::Expanded)), SizeClass::Expanded);
    assert_eq!(of(815, Some(SizeClass::Expanded)), SizeClass::Medium);
    // Velky skok dolu: rovnou o dve tridy.
    assert_eq!(of(400, Some(SizeClass::Expanded)), SizeClass::Compact);
}

struct A;

impl App for A {
    type Msg = ();

    fn title(&self) -> String {
        "a".into()
    }

    fn view(&self, cx: &Context) -> Node<()> {
        let t = match cx.class {
            SizeClass::Compact => "kompaktní",
            SizeClass::Medium => "střední",
            SizeClass::Expanded => "široké",
        };
        column([label(t), label(&alloc::format!("plocha {}", cx.work_area.w))])
    }

    fn update(&mut self, _: ()) {}
}

/// Aplikace dostane kontext sveho okna; zmena tridy se hlasi z `resize`.
#[test]
fn view_gets_window_context() {
    let mut u: Ui<()> = Ui::new(crate::Look::ui_fonts(), 900, 400);
    u.set_context(0, Rect::new(0, 0, 1280, 800), Rect::new(0, 0, 1280, 744), 100, 60_000, 2);
    u.refresh(&A);
    let has = |u: &Ui<()>, s: &str| u.a11y().iter().any(|n| n.role == Role::Label && n.name == s);
    assert!(has(&u, "široké"));
    assert!(has(&u, "plocha 1280"));
    assert!(!u.resize(830, 400), "v hysterezi trida zustane");
    assert!(u.resize(560, 400));
    u.refresh(&A);
    assert!(has(&u, "kompaktní"));
    assert_eq!(u.context().generation, 2);
}
