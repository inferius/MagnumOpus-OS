use super::*;

const ACCENT: u32 = 0xFF6B3FD6;

fn frame(w: u32, h: u32, st: &State) -> (Splash, Vec<u32>) {
    let mut s = Splash::new(w, h);
    let mut out = vec![0u32; (w * h) as usize];
    s.paint_all(st, &mut out, w as usize);
    (s, out)
}

/// Nahled do souboru PNG (`BOOTSPLASH_DUMP=adresar cargo test`).
fn dump(name: &str, w: u32, h: u32, px: &[u32]) {
    let Ok(dir) = std::env::var("BOOTSPLASH_DUMP") else { return };
    std::fs::write(format!("{dir}/{name}.png"), gfx::png::encode(px, w, h, w)).unwrap();
}

fn close(a: u32, b: u32, tol: i32) -> bool {
    (0..3).all(|k| (((a >> (k * 8)) & 0xFF) as i32 - ((b >> (k * 8)) & 0xFF) as i32).abs() <= tol)
}

#[test]
fn logo_data_parses() {
    let sym = art::symbol();
    assert!(!sym.is_empty());
    let (x0, y0, x1, y1) = art::bounds(&sym);
    // Symbol lezi uvnitr viewBoxu -8 -12 400 400.
    assert!(x0 >= -8.0 && y0 >= -12.0 && x1 <= 392.0 && y1 <= 388.0, "{x0} {y0} {x1} {y1}");
    let wm = art::wordmark();
    // Nazev "MagnumOpus OS": 17 obrysu (pismena a diry).
    assert_eq!(wm.len(), 17);
}

#[test]
fn even_odd_keeps_holes() {
    // Ctverec s dirou toceny stejnym smerem: nenulove vinuti by diru zaplnilo.
    let outer = vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
    let inner = vec![(3.0, 3.0), (7.0, 3.0), (7.0, 7.0), (3.0, 7.0)];
    let m = art::render(&[outer, inner], 10, 10, 0.0, 0.0, 1.0);
    assert_eq!(m.get(1, 1), 255);
    assert_eq!(m.get(5, 5), 0);
}

#[test]
fn path_parser_handles_js_escapes() {
    let c = art::parse_path("M0,0 L10,0 L10,10 Z\\r\\nM20,20 C20,20 30,20 30,30 Z");
    assert_eq!(c.len(), 2);
    assert_eq!(c[0].len(), 3);
}

#[test]
fn layout_is_centered_and_scaled() {
    let l = Layout::new(1280, 800);
    assert_eq!(l.s, 1.0);
    assert_eq!(l.cx, 640.0);
    // Sloupec 192 px na stred: znak 304..436, stred 370.
    assert_eq!(l.cy, 370.0);
    assert_eq!(l.word_top, 452.0);
    let l = Layout::new(1920, 1080);
    assert!((l.s - 1.35).abs() < 1e-4);
    // Male obrazovky: meritko nejmene 0,5.
    assert!(Layout::new(320, 200).s >= 0.5);
}

#[test]
fn first_frame_is_black_with_mark() {
    let (s, out) = frame(1280, 800, &State::FIRST);
    let l = s.layout;
    assert_eq!(out[0], 0xFF00_0000);
    assert_eq!(out[(799 * 1280 + 1279) as usize], 0xFF00_0000);
    // Stred loga je akcent (logo je plne uprostred symbolu? ne nutne) -
    // prstenec vpravo je podklad: akcent 22 % na cerne.
    let x = (l.cx + size::RING_R) as usize;
    let p = out[l.cy as usize * 1280 + x];
    assert!(close(p, 0xFF17_0E2F, 3), "{p:#x}");
    // Nazev v prvnim snimku neni.
    let wy = (l.word_top + 22.0) as usize;
    assert!((0..1280).all(|x| out[wy * 1280 + x] == 0xFF00_0000));
    dump("first", 1280, 800, &out);
}

#[test]
fn ring_progress_follows_clock() {
    let (s, out) = frame(1280, 800, &State::lit(0.5));
    let l = s.layout;
    let at = |x: f32, y: f32| out[y as usize * 1280 + x as usize];
    // 3 hodiny (25 %) vyplneno, 9 hodin (75 %) jen podklad.
    let right = at(l.cx + size::RING_R, l.cy);
    let left = at(l.cx - size::RING_R - 1.0, l.cy);
    assert!(close(right, ACCENT, 8), "{right:#x}");
    assert!(!close(left, ACCENT, 40), "{left:#x}");
    dump("half", 1280, 800, &out);
    let (_, full) = frame(1280, 800, &State { glow: 0.7, ..State::lit(1.0) });
    assert!(close(full[l.cy as usize * 1280 + (l.cx - size::RING_R - 1.0) as usize], ACCENT, 8));
    dump("full", 1280, 800, &full);
}

#[test]
fn lit_frame_matches_desktop_backdrop() {
    // Mimo znak a nazev je obraz presne pozadi plochy kompozitoru.
    let (mut s, out) = frame(800, 600, &State::lit(0.0));
    let bg = s.backdrop().to_vec();
    assert_eq!(out[0], bg[0]);
    assert_eq!(out[600 * 800 - 1], bg[600 * 800 - 1]);
    assert_eq!(out[10 * 800 + 700], bg[10 * 800 + 700]);
}

#[test]
fn word_is_drawn_when_visible() {
    let (s, out) = frame(1280, 800, &State::lit(1.0));
    let l = s.layout;
    let wy = (l.word_top + 22.0) as usize;
    // Nekde v radku nazvu je tmava barva pisma.
    assert!((0..1280).any(|x| close(out[wy * 1280 + x], 0xFF2A1F2D, 10)));
}

#[test]
fn rgb_framebuffer_swaps_channels() {
    let mut s = Splash::new(640, 400);
    s.set_rgb(true);
    let mut out = vec![0u32; 640 * 400];
    s.paint_all(&State::lit(1.0), &mut out, 640);
    let l = s.layout;
    let p = out[l.cy as usize * 640 + (l.cx + size::RING_R * l.s) as usize];
    // Akcent 6B3FD6 v poradi R, G, B v pameti = u32 0x..D63F6B.
    assert!(close(p, 0xFFD63F6B, 10), "{p:#x}");
}

#[test]
fn partial_paint_stays_inside_area() {
    let mut s = Splash::new(640, 400);
    let mut out = vec![0x1234_5678u32; 640 * 400];
    let r = s.layout.mark_rect(1.0, 0.0, 0.0);
    s.paint(&State::lit(0.3), &mut out, 640, r);
    assert_eq!(out[0], 0x1234_5678);
    assert_ne!(out[s.layout.cy as usize * 640 + s.layout.cx as usize], 0x1234_5678);
    // Mimo obrazovku ani s nesmyslnou oblasti nic nespadne.
    s.paint(&State::lit(0.3), &mut out, 640, Rect { x: -50, y: 390, w: 10_000, h: 100 });
}

#[test]
fn backdrop_under_shows_through() {
    let mut s = Splash::new(320, 200);
    let mut out = vec![0xFFFF_FFFFu32; 320 * 200];
    s.paint_all(&State { backdrop: 0.0, mark_opacity: 0.0, word: 0.0, ..State::lit(1.0) }, &mut out, 320);
    assert!(out.iter().all(|&p| p == 0xFFFF_FFFF));
}

#[test]
fn breathe_and_ease_ranges() {
    let (o0, s0) = breathe(0);
    assert!((o0 - 0.25).abs() < 1e-3 && (s0 - 0.92).abs() < 1e-3);
    let (o1, s1) = breathe(timing::BREATHE / 2);
    assert!((o1 - 0.7).abs() < 1e-3 && (s1 - 1.05).abs() < 1e-3);
    assert_eq!(breathe(5), breathe(5 + timing::BREATHE));
    let mut prev = 0.0;
    for k in 0..=20 {
        let v = ease(k as f32 / 20.0);
        assert!(v >= prev - 1e-4);
        prev = v;
    }
    assert!(ease(0.0).abs() < 1e-3 && (ease(1.0) - 1.0).abs() < 1e-3);
}

#[test]
fn transformed_mark_moves() {
    let mut s = Splash::new(1280, 800);
    let mut out = vec![0u32; 1280 * 800];
    let st = State { mark_scale: 0.5, mark_dx: 200.0, mark_dy: -100.0, ..State::lit(1.0) };
    s.paint_all(&st, &mut out, 1280);
    let l = s.layout;
    let x = (l.cx + 200.0 + size::RING_R * 0.5) as usize;
    let y = (l.cy - 100.0) as usize;
    assert!(close(out[y * 1280 + x], ACCENT, 70), "{:#x}", out[y * 1280 + x]);
}

#[test]
fn sqrt_is_precise() {
    for &x in &[1e-4f32, 0.5, 1.0, 2.0, 9.0, 1234.5, 3.0e6] {
        let r = geom::sqrt(x);
        assert!((r * r - x).abs() <= x * 1e-5, "{x}: {r}");
    }
    assert_eq!(geom::sqrt(0.0), 0.0);
}

#[test]
fn fast_path_matches_generic() {
    let mut s = Splash::new(640, 400);
    let mut a = vec![0u32; 640 * 400];
    let mut b = vec![0u32; 640 * 400];
    s.paint_all(&State { glow: 0.5, ..State::lit(0.6) }, &mut a, 640);
    // Pozadi 0,99999 vynuti obecnou cestu (rozdil pod jednu uroven).
    s.paint_all(&State { glow: 0.5, wall: 0.99999, ..State::lit(0.6) }, &mut b, 640);
    assert!(a.iter().zip(&b).all(|(&x, &y)| close(x, y, 1)));
}

/// Cisla do dokumentace: `cargo test --release -p bootsplash -- --ignored --nocapture`.
#[test]
#[ignore]
fn perf_numbers() {
    let (w, h) = (1920u32, 1080u32);
    let t = std::time::Instant::now();
    let mut s = Splash::new(w, h);
    let new = t.elapsed();
    let mut out = vec![0u32; (w * h) as usize];
    let t = std::time::Instant::now();
    s.paint_all(&State::FIRST, &mut out, w as usize);
    let first = t.elapsed();
    let t = std::time::Instant::now();
    s.backdrop();
    let bg = t.elapsed();
    let st = State { glow: 0.5, ..State::lit(0.4) };
    let t = std::time::Instant::now();
    s.paint_all(&st, &mut out, w as usize);
    let lit = t.elapsed();
    let fade = State { wall: 0.5, word: 0.5, ..st };
    let t = std::time::Instant::now();
    s.paint_all(&fade, &mut out, w as usize);
    let fading = t.elapsed();
    let mut under = out.clone();
    let t = std::time::Instant::now();
    s.paint_all(&State { backdrop: 0.5, mark_opacity: 0.5, ..st }, &mut under, w as usize);
    let blend = t.elapsed();
    let r = s.layout.mark_rect(1.0, 0.0, 0.0);
    let t = std::time::Instant::now();
    for _ in 0..100 {
        s.paint(&st, &mut out, w as usize, r);
    }
    let mark = t.elapsed() / 100;
    println!("1920x1080: new {new:?}, prvni snimek {first:?}, pozadi {bg:?}, cely snimek {lit:?}, rozsviceni {fading:?}, rozplynuti {blend:?}, znak {mark:?} ({}x{})", r.w, r.h);
}

#[test]
fn integer_mark_matches_float() {
    // Klidovy znak (tabulka) proti presunutemu o 0 px pres float cestu:
    // posun 1e-6 vynuti float, vysledek se lisi nejvys o 2 urovne.
    for p in [0.0f32, 0.37, 1.0] {
        let mut s = Splash::new(1280, 800);
        let mut a = vec![0u32; 1280 * 800];
        let mut b = vec![0u32; 1280 * 800];
        let st = State { glow: 0.6, glow_scale: 1.02, ..State::lit(p) };
        s.paint_all(&st, &mut a, 1280);
        s.paint_all(&State { mark_dx: 1e-6, ..st }, &mut b, 1280);
        let bad = a.iter().zip(&b).filter(|(x, y)| !close(**x, **y, 2)).count();
        assert_eq!(bad, 0, "progress {p}: {bad} pixelu se lisi");
    }
}

/// Otisk snimku (FNV-1a 64).
fn fnv(px: &[u32]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for p in px {
        for b in p.to_le_bytes() {
            h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// Tri stavy: prvni snimek loaderu, rozsviceny obraz s prstencem a zari
/// (init), posunuty a pruhledny znak nad sedou plochou (prechod v compd).
fn reference_frames(w: u32, h: u32) -> [u64; 3] {
    let mut s = Splash::new(w, h);
    let n = (w * h) as usize;
    let mut a = vec![0u32; n];
    s.paint_all(&State::FIRST, &mut a, w as usize);
    let mut b = vec![0u32; n];
    s.paint_all(&State { glow: 0.55, glow_scale: 1.01, ..State::lit(0.6) }, &mut b, w as usize);
    let mut c = vec![0xFF80_8080u32; n];
    let st = State { backdrop: 0.4, word: 0.3, mark_scale: 0.6, mark_dx: 40.0, mark_dy: -25.0, mark_opacity: 0.7, ..State::lit(1.0) };
    s.paint_all(&st, &mut c, w as usize);
    [fnv(&a), fnv(&b), fnv(&c)]
}

/// Loader (UEFI, FPU), proces splash a kompozitor (plovouci carka
/// v softwaru) musi kreslit bit po bitu stejne, jinak by prechod mezi nimi
/// poskakoval. Knihovna pouziva jen zakladni operace IEEE 754 (scitani,
/// nasobeni, deleni, prevody; vlastni odmocnina, sin a atan), ktere jsou
/// presne zaokrouhlene v hardwaru i v softwaru, a zadne `mul_add`.
/// Reference zachycuje aktualni kresbu: zamerna zmena vzhledu ji musi
/// prepsat (`BOOTSPLASH_REF=1 cargo test -p bootsplash reference -- --nocapture`
/// vypise nove hodnoty).
#[test]
fn reference_output_is_stable() {
    const REF: [((u32, u32), [u64; 3]); 3] = [
        ((1280, 800), [0x03bb8d75132b0232, 0x1419f744ddcfdc79, 0x47652a8baa31a137]),
        ((1920, 1080), [0x81cd1f73b0bbb90f, 0x8bda400092419968, 0xa671506d7b66d8f6]),
        ((1024, 768), [0xcd3d605c75a9cb5c, 0xcb500ac2c810b8b4, 0xb5075b905a0bfac0]),
    ];
    for ((w, h), want) in REF {
        let got = reference_frames(w, h);
        if std::env::var("BOOTSPLASH_REF").is_ok() {
            println!("(({w}, {h}), [{:#018x}, {:#018x}, {:#018x}]),", got[0], got[1], got[2]);
            continue;
        }
        assert_eq!(got, want, "{w}x{h}");
        // Opakovane kresleni (mezipameti loga a znaku) dava totez.
        assert_eq!(reference_frames(w, h), got);
    }
}

#[test]
fn self_check_matches() {
    let v = self_check();
    assert_eq!(v, SELF_CHECK, "novy otisk: {v:#018x}");
}
