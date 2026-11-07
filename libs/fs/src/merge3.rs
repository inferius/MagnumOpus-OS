//! Trojcestne slouceni textu po radcich (diff3).
//!
//! Vstup: spolecny predek (`base`), nase verze (`ours`) a jejich (`theirs`).
//! 1. Nejdelsi spolecna podposloupnost radku `base`-`ours` a `base`-`theirs`
//!    (Myersuv algoritmus, cena podle velikosti zmeny).
//! 2. Pruchod `base`: "stabilni" radek je spolecny vsem trem; mezi
//!    stabilnimi radky lezi useky, kde se nekdo lisi. Usek zmeneny jen na
//!    jedne strane se prevezme, stejna zmena na obou stranach jednou,
//!    ruzne zmeny = **konflikt** se znackami jako v gitu:
//!
//! ```text
//! <<<<<<< nase
//! radky ours
//! =======
//! radky theirs
//! >>>>>>> jejich
//! ```
//!
//! Text = platne UTF-8 bez NUL do `TEXT_MAX` bajtu; jine soubory se po
//! radcich neslucuji (binarni konflikt resi volajici).

use alloc::vec::Vec;

/// Nejvetsi soubor, ktery se slucuje po radcich.
pub const TEXT_MAX: usize = 4 * 1024 * 1024;
/// Nejvic rozdilu (vlozenych + smazanych radku) pro Myersuv algoritmus;
/// nad nim se cely soubor bere jako jeden zmeneny usek (pamet O(D^2)).
const MAX_EDITS: usize = 4096;

/// Je obsah text, ktery se smi slucovat po radcich?
pub fn is_text(b: &[u8]) -> bool {
    b.len() <= TEXT_MAX && !b.contains(&0) && core::str::from_utf8(b).is_ok()
}

/// Radky vcetne zakonceni `\n` (posledni muze byt bez nej).
fn lines(b: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, &c) in b.iter().enumerate() {
        if c == b'\n' {
            out.push(&b[start..=i]);
            start = i + 1;
        }
    }
    if start < b.len() {
        out.push(&b[start..]);
    }
    out
}

/// Pro kazdy radek `a` index shodneho radku v `b` (nejdelsi spolecna
/// podposloupnost), jinak `None`.
fn matches(a: &[&[u8]], b: &[&[u8]]) -> Vec<Option<usize>> {
    let mut out = alloc::vec![None; a.len()];
    // Spolecny zacatek a konec zvlast (casty pripad, levny).
    let mut pre = 0;
    while pre < a.len() && pre < b.len() && a[pre] == b[pre] {
        out[pre] = Some(pre);
        pre += 1;
    }
    let mut suf = 0;
    while suf < a.len() - pre && suf < b.len() - pre && a[a.len() - 1 - suf] == b[b.len() - 1 - suf] {
        out[a.len() - 1 - suf] = Some(b.len() - 1 - suf);
        suf += 1;
    }
    let (a2, b2) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    if let Some(pairs) = myers(a2, b2) {
        for (i, j) in pairs {
            out[pre + i] = Some(pre + j);
        }
    }
    out
}

/// Myersuv algoritmus O((N+M)D): shodne dvojice (i, j) vzestupne. `None`,
/// kdyz je rozdilu vic nez `MAX_EDITS`.
fn myers(a: &[&[u8]], b: &[&[u8]]) -> Option<Vec<(usize, usize)>> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    if n == 0 || m == 0 {
        return Some(Vec::new());
    }
    let max = ((n + m) as usize).min(MAX_EDITS) as isize;
    let off = max + 1;
    let mut v = alloc::vec![0isize; (2 * off + 1) as usize];
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut found = None;
    'outer: for d in 0..=max {
        trace.push(v.clone());
        let mut k = -d;
        while k <= d {
            let idx = (k + off) as usize;
            let mut x = if k == -d || (k != d && v[idx - 1] < v[idx + 1]) { v[idx + 1] } else { v[idx - 1] + 1 };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[idx] = x;
            if x >= n && y >= m {
                found = Some(d);
                break 'outer;
            }
            k += 2;
        }
    }
    let d_end = found?;
    // Zpetny pruchod: od konce po diagonalach k zacatku.
    let mut pairs = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (0..=d_end).rev() {
        let v = &trace[d as usize];
        let k = x - y;
        let prev_k = if k == -d || (k != d && v[(k - 1 + off) as usize] < v[(k + 1 + off) as usize]) { k + 1 } else { k - 1 };
        let prev_x = if d == 0 { 0 } else { v[(prev_k + off) as usize] };
        let prev_y = prev_x - prev_k;
        let (sx, sy) = if d == 0 { (0, 0) } else if prev_k == k + 1 { (prev_x, prev_y + 1) } else { (prev_x + 1, prev_y) };
        while x > sx && y > sy {
            x -= 1;
            y -= 1;
            pairs.push((x as usize, y as usize));
        }
        x = prev_x;
        y = prev_y;
        if d == 0 {
            break;
        }
    }
    pairs.reverse();
    Some(pairs)
}

/// Vysledek slouceni.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    pub data: Vec<u8>,
    /// Pocet konfliktnich useku (0 = ciste slouceni).
    pub conflicts: usize,
}

fn push_lines(out: &mut Vec<u8>, ls: &[&[u8]]) {
    for l in ls {
        out.extend_from_slice(l);
    }
}

/// Znacka konfliktu musi zacinat na novem radku.
fn ensure_newline(out: &mut Vec<u8>) {
    if out.last().is_some_and(|&c| c != b'\n') {
        out.push(b'\n');
    }
}

/// Trojcestne slouceni po radcich; `ours_label`/`theirs_label` do znacek.
pub fn merge(base: &[u8], ours: &[u8], theirs: &[u8], ours_label: &str, theirs_label: &str) -> Merged {
    let (o, a, b) = (lines(base), lines(ours), lines(theirs));
    let ma = matches(&o, &a);
    let mb = matches(&o, &b);
    let mut out = Vec::with_capacity(ours.len().max(theirs.len()));
    let mut conflicts = 0;
    let (mut io, mut ia, mut ib) = (0, 0, 0);
    loop {
        // Dalsi radek predka shodny v obou verzich (za aktualni pozici).
        let next = (io..o.len()).find_map(|i| match (ma[i], mb[i]) {
            (Some(x), Some(y)) if x >= ia && y >= ib => Some((i, x, y)),
            _ => None,
        });
        let (eo, ea, eb) = next.unwrap_or((o.len(), a.len(), b.len()));
        if (eo, ea, eb) != (io, ia, ib) {
            // Nestabilni usek.
            let (co, ca, cb) = (&o[io..eo], &a[ia..ea], &b[ib..eb]);
            if ca == co {
                push_lines(&mut out, cb);
            } else if cb == co || ca == cb {
                push_lines(&mut out, ca);
            } else {
                conflicts += 1;
                ensure_newline(&mut out);
                out.extend_from_slice(b"<<<<<<< ");
                out.extend_from_slice(ours_label.as_bytes());
                out.push(b'\n');
                push_lines(&mut out, ca);
                ensure_newline(&mut out);
                out.extend_from_slice(b"=======\n");
                push_lines(&mut out, cb);
                ensure_newline(&mut out);
                out.extend_from_slice(b">>>>>>> ");
                out.extend_from_slice(theirs_label.as_bytes());
                out.push(b'\n');
            }
        }
        if next.is_none() {
            break;
        }
        // Stabilni radek.
        out.extend_from_slice(o[eo]);
        (io, ia, ib) = (eo + 1, ea + 1, eb + 1);
    }
    Merged { data: out, conflicts }
}
