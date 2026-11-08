//! Ukazatel cinnosti a pruh prubehu: jeden modul pro vsechny vystupy.
//!
//! Prikazy cinnost nekresli samy: hlasi ji strukturovane (blok bezi,
//! `Item::Progress { done, total }`) a okno ji vykresli timto modulem
//! podle toho, kam kresli ([`Glyphs`]):
//!
//! - [`Glyphs::Drawn`] (GUI terminal `gui-term`): znaky ze soukrome oblasti
//!   Unicode ([`SPIN_BASE`], [`BAR_BASE`]), ktere okno nahradi vlastnim
//!   kreslenim - krouzek s otacejicim se obloukem v barve akcentu a pruh
//!   s dilcimi bunkami po 1/8 bunky.
//! - [`Glyphs::Text`] (zalozni konzole, cizi terminal): braillovy tecky
//!   `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏` (nejvys 10 snimku/s) a pruh z `▏▎▍▌▋▊▉█`.
//! - [`Glyphs::Static`] ("Omezit pohyb"): bez animace, `◌` bezi / `●` hotovo,
//!   pruh jen z celych bunek.
//!
//! Vystup, ktery neni terminal (presmerovani do souboru, skript), animaci
//! nedostava vubec: prubeh je polozka bloku a zapise se jen konecny stav.

use alloc::string::String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Glyphs {
    /// Okno kresli vlastni prvky (soukrome znaky).
    Drawn,
    /// Znakova zaloha (braille, osminy bunky).
    #[default]
    Text,
    /// Omezit pohyb: staticke symboly.
    Static,
}

/// Snimky krouzku: `SPIN_BASE + n` (n < [`SPIN_FRAMES`]) = oblouk otoceny
/// o n/SPIN_FRAMES otacky. Soukroma oblast roviny 15 (U+F0000..).
pub const SPIN_BASE: u32 = 0xF_0000;
pub const SPIN_FRAMES: u32 = 16;
/// Bunka pruhu zaplnena z `k` osmin: `BAR_BASE + k` (0..=8).
pub const BAR_BASE: u32 = 0xF_0100;

/// Doba jedne otacky krouzku (ms).
const TURN_MS: i64 = 960;
/// Znakova zaloha: nejvys 10 snimku/s.
const TEXT_FRAME_MS: i64 = 100;

const BRAILLE: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const EIGHTHS: [char; 9] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];

/// Znak ukazatele cinnosti v case `ms` (libovolny monotonni cas).
pub fn spinner(g: Glyphs, ms: i64) -> char {
    match g {
        Glyphs::Drawn => {
            let n = (ms.rem_euclid(TURN_MS) * SPIN_FRAMES as i64 / TURN_MS) as u32;
            char::from_u32(SPIN_BASE + n).unwrap_or('◌')
        }
        Glyphs::Text => BRAILLE[(ms.rem_euclid(TEXT_FRAME_MS * 10) / TEXT_FRAME_MS) as usize],
        Glyphs::Static => '◌',
    }
}

/// Symbol hotove cinnosti (staticky).
pub const DONE: char = '●';

/// Pruh sirky `width` bunek zaplneny z `frac` (0..=1): plne bunky, jedna
/// dilci (osminy) a mezery. Barvy (popredi = zaplneni, pozadi = koleje)
/// doplni volajici.
pub fn bar(g: Glyphs, width: usize, frac: f64) -> String {
    let frac = if frac.is_finite() { frac.clamp(0.0, 1.0) } else { 0.0 };
    // no_std: zaokrouhleni bez `round` (frac >= 0).
    let eighths = (frac * (width * 8) as f64 + 0.5) as usize;
    let (full, part) = match g {
        // Bez dilcich bunek: jen cele (zaokrouhlene).
        Glyphs::Static => ((eighths + 4) / 8, 0),
        _ => (eighths / 8, eighths % 8),
    };
    let cell = |k: usize| -> char {
        match g {
            Glyphs::Drawn => char::from_u32(BAR_BASE + k as u32).unwrap_or(' '),
            _ => EIGHTHS[k],
        }
    };
    let mut s = String::new();
    for _ in 0..full.min(width) {
        s.push(cell(8));
    }
    if full < width {
        if part > 0 {
            s.push(cell(part));
        } else {
            s.push(cell(0));
        }
        for _ in full + 1..width {
            s.push(cell(0));
        }
    }
    s
}

/// Rozlozeni soukromeho znaku pro kresleni oknem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drawn {
    /// Krouzek, oblouk otoceny o `n` / [`SPIN_FRAMES`] otacky.
    Spinner(u32),
    /// Bunka pruhu zaplnena z `k` osmin (zleva).
    Bar(u32),
}

pub fn drawn(c: char) -> Option<Drawn> {
    let u = c as u32;
    if (SPIN_BASE..SPIN_BASE + SPIN_FRAMES).contains(&u) {
        Some(Drawn::Spinner(u - SPIN_BASE))
    } else if (BAR_BASE..=BAR_BASE + 8).contains(&u) {
        Some(Drawn::Bar(u - BAR_BASE))
    } else {
        None
    }
}

/// Smer stredu oblouku pro snimek `n` z 16 (cos, sin) x 1000: otacka po
/// smeru hodin od 12 hodin.
const ARC_DIR: [(i32, i32); 16] = [
    (0, -1000), (383, -924), (707, -707), (924, -383), (1000, 0), (924, 383), (707, 707), (383, 924),
    (0, 1000), (-383, 924), (-707, 707), (-924, 383), (-1000, 0), (-924, -383), (-707, -707), (-383, -924),
];

/// Kryti (0..=255) barvy popredi v pixelu `(x, y)` bunky `w` x `h` pro
/// kresleny prvek: krouzek (slaba kolej + oblouk 120 stupnu, 2x2 vzorky
/// na pixel = vyhlazeni, bez trigonometrie) nebo dilci bunka pruhu.
/// Spolecne pro okno GUI i zalozni konzoli (jejich pismo znaky nema).
pub fn coverage(d: Drawn, w: u32, h: u32, x: u32, y: u32) -> u8 {
    match d {
        Drawn::Bar(k) => {
            let fill = w * k.min(8) / 8;
            // Pruh je o 2 px nizsi nez bunka (mezera mezi radky).
            if x < fill && y >= 2 && y + 2 < h { 255 } else { 0 }
        }
        Drawn::Spinner(n) => {
            let size = w.min(h) as i32;
            // V polovinach pixelu.
            let r_out = size * 2 * 45 / 100;
            let r_in = r_out - (size * 2 / 5).max(3);
            let (cx, cy) = (w as i32, h as i32);
            let (dx, dy) = ARC_DIR[(n % SPIN_FRAMES) as usize];
            let (mut ring, mut arc) = (0u32, 0u32);
            for (sx, sy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (vx, vy) = (x as i32 * 2 + sx - cx, y as i32 * 2 + sy - cy);
                let d2 = vx * vx + vy * vy;
                if d2 > r_out * r_out || d2 < r_in * r_in {
                    continue;
                }
                ring += 1;
                // Uhel ke stredu oblouku < 60 st.: cos > 0,5 bez odmocniny
                // (dot > 0 a 4 dot^2 > |v|^2 |d|^2).
                let dot = (vx * dx + vy * dy) as i64;
                if dot > 0 && dot * dot * 4 > d2 as i64 * 1_000_000 {
                    arc += 1;
                }
            }
            // Kolej 30 %, oblouk plne.
            ((ring - arc) * 255 * 30 / 400 + arc * 255 / 4) as u8
        }
    }
}
