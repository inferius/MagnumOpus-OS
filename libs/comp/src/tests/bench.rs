//! Mereni skladani snimku na hostiteli (spousti se jen vyslovne):
//! `cargo test -p comp [--release] bench_frame -- --ignored --nocapture`.
//! Rozlozi cas celeho snimku 1280x800 na casti (plocha, okna se stiny,
//! vrstva nad okny), aby slo porovnat s QEMU (`compd: snimek cele
//! obrazovky`).

extern crate std;

use alloc::vec;

use super::*;

fn ms(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

#[test]
#[ignore]
fn bench_frame() {
    let (w, h) = (1280u32, 800u32);
    let mut t = T::new();
    t.c.resize(w, h, t.now);
    for i in 0..3 {
        let c = t.client(&alloc::format!("app{i}"));
        t.window(c, Mode::Standard, RED);
    }
    let mut px = vec![0u32; (w * h) as usize];
    let n = 10;
    // Cely snimek.
    let start = std::time::Instant::now();
    for k in 0..n {
        t.c.redraw_all();
        let mut c = Canvas::new(&mut px, w, h, w).unwrap();
        t.now += 20_000_000;
        let _ = t.c.tick(t.now + k, &mut c);
        t.c.presented(t.now);
    }
    let full = start.elapsed() / n as u32;
    // Jen plocha (obraz pozadi do celeho platna).
    let start = std::time::Instant::now();
    for _ in 0..n {
        let mut c = Canvas::new(&mut px, w, h, w).unwrap();
        t.c.desktop(&mut c);
    }
    let desk = start.elapsed() / n as u32;
    // Plocha jako prosta vypln (dolni mez kopirovani 1 Mpx).
    let start = std::time::Instant::now();
    for _ in 0..n {
        let mut c = Canvas::new(&mut px, w, h, w).unwrap();
        c.fill(c.bounds(), 0xFF20_2020);
    }
    let fill = start.elapsed() / n as u32;
    // Vrstva nad okny (lista, kurzor) pres mezibuffer.
    let start = std::time::Instant::now();
    for _ in 0..n {
        let bar = t.c.bar_layout();
        let mut c = Canvas::new(&mut px, w, h, w).unwrap();
        for r in t.c.top_regions(false) {
            c.set_clip(r);
            t.c.draw_top(&mut c, &bar, false);
        }
    }
    let top = start.elapsed() / n as u32;
    std::println!(
        "snimek {w}x{h}: cely {:.1} ms, plocha {:.1} ms (vypln {:.1} ms), vrstva nad okny {:.1} ms",
        ms(full),
        ms(desk),
        ms(fill),
        ms(top)
    );
}

fn host_ns() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_nanos() as u64
}

/// Prechod rezimu (`fade.rs`) pri 1920x1080: cas snimku po castech
/// behem prechodu.
/// `cargo test -p comp --release bench_theme_fade -- --ignored --nocapture`
#[test]
#[ignore]
fn bench_theme_fade() {
    let (w, h) = (1920u32, 1080u32);
    let mut t = T::new();
    t.c.resize(w, h, t.now);
    t.c.prof.set_clock(Some(host_ns));
    for i in 0..3 {
        let c = t.client(&alloc::format!("app{i}"));
        t.window(c, Mode::Standard, RED);
    }
    let mut px = vec![0u32; (w * h) as usize];
    let mut tick = |t: &mut T, px: &mut [u32]| {
        let mut c = Canvas::new(px, w, h, w).unwrap();
        t.now += 16_000_000;
        let r = t.c.tick(t.now, &mut c);
        t.c.presented(t.now);
        r
    };
    tick(&mut t, &mut px);
    t.c.session_opened(1, 1000, "a", t.now);
    t.c.set_appearance_user(1000, theme::Pref { scheme: theme::Scheme::Dark, ..Default::default() }, t.now);
    t.c.prof.take(1);
    let (mut n, start) = (0u64, std::time::Instant::now());
    let mut worst = std::time::Duration::ZERO;
    while t.c.theme_animating() {
        let f = std::time::Instant::now();
        tick(&mut t, &mut px);
        worst = worst.max(f.elapsed());
        n += 1;
    }
    let all = start.elapsed();
    std::println!(
        "prolnuti {w}x{h}: {n} snimku, prumer {:.1} ms, nejhorsi {:.1} ms; casti: {}",
        ms(all) / n as f64,
        ms(worst),
        t.c.prof.take(n)
    );
}
