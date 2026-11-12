//! Diagnosticky snimek (zkratka Print Screen / Ctrl+Alt+Shift+S, viz
//! `libs/comp/src/diag.rs`): stav kompozitoru do logu jako blok radku
//! "snimek <id>: ..." a obrazek obrazovky (BMP, otevre ho i Windows) do
//! logd, ktery ho ulozi do `/system/log` a na oddil MO-LOGY.
//!
//! Zprava pro logd: `snimek <id> <delka>` + VMO s obsahem souboru.

use alloc::format;
use alloc::string::String;
use alloc::vec;

use comp::Comp;
use libos::{Channel, Vmo, println, time};

/// Id snimku: datum a cas UTC (`20261108-171502`), bez realneho casu cas
/// od startu (`t162.345`). Spoji blok v logu se souborem obrazku.
fn snapshot_id() -> String {
    match time::wall() {
        Some(t) if t.secs > 0 => {
            let days = t.secs.div_euclid(86_400);
            let s = t.secs.rem_euclid(86_400);
            let (y, m, d) = libos::abi::time::civil_from_days(days);
            format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", s / 3600, s / 60 % 60, s % 60)
        }
        _ => {
            let ms = time::now() / 1_000_000;
            format!("t{}.{:03}", ms / 1000, ms % 1000)
        }
    }
}

/// Obrazek `w x h` z pixelu XRGB na `addr` jako BMP (24 bitu, radky
/// odspodu) do noveho VMO; vraci (VMO, delka).
/// Pole s tajemstvim (`Comp::secure_fields`) se v obrazku vyplni sedou:
/// nezaznamena se obsah ani pocet tecek.
fn bmp(addr: u64, w: u32, h: u32, masks: &[guiproto::Rect]) -> libos::Result<(Vmo, usize)> {
    let row = (w as usize * 3).next_multiple_of(4);
    let len = 54 + row * h as usize;
    let vmo = Vmo::create(len as u64)?;
    let mut head = vec![0u8; 54];
    head[0..2].copy_from_slice(b"BM");
    head[2..6].copy_from_slice(&(len as u32).to_le_bytes());
    head[10..14].copy_from_slice(&54u32.to_le_bytes());
    head[14..18].copy_from_slice(&40u32.to_le_bytes());
    head[18..22].copy_from_slice(&w.to_le_bytes());
    head[22..26].copy_from_slice(&h.to_le_bytes());
    head[26..28].copy_from_slice(&1u16.to_le_bytes());
    head[28..30].copy_from_slice(&24u16.to_le_bytes());
    head[34..38].copy_from_slice(&((row * h as usize) as u32).to_le_bytes());
    vmo.write(0, &head)?;
    let mut line = vec![0u8; row];
    for y in 0..h {
        // SAFETY: `addr` je mapovani snimku `w * h` pixelu (zije po celou
        // dobu sluzby), radek `y < h`.
        let src = unsafe { core::slice::from_raw_parts((addr as *const u32).add((y * w) as usize), w as usize) };
        for (x, p) in src.iter().enumerate() {
            let hidden = masks.iter().any(|m| m.contains(x as i32, y as i32));
            let p = if hidden { 0x0080_8080 } else { *p };
            line[x * 3] = p as u8;
            line[x * 3 + 1] = (p >> 8) as u8;
            line[x * 3 + 2] = (p >> 16) as u8;
        }
        vmo.write((54 + (h - 1 - y) as usize * row) as u64, &line)?;
    }
    Ok((vmo, len))
}

/// Udela snimek: blok stavu do logu a obrazek do logd (kdyz init dal
/// kanal `snapshots` a obrazovka je namapovana). Cenu (cas, radky, bajty)
/// zapise na konec.
pub fn take(comp: &Comp, why: &str, frame: Option<(u64, u32, u32)>, logd: Option<&Channel>) {
    let id = snapshot_id();
    let t0 = time::now();
    let lines = comp.snapshot_lines(&id);
    let t1 = time::now();
    let bytes: usize = lines.iter().map(|l| l.len() + 1).sum();
    println!("snimek {id}: duvod {why}");
    for l in &lines {
        println!("{l}");
    }
    let t2 = time::now();
    let note = match (frame, logd) {
        (None, Some(ch)) => {
            // Bez obrazku: logd aspon vypise trasovani vstupu.
            let _ = ch.write(format!("snimek {id} 0").as_bytes(), vec![]);
            String::from("obrazek nejde: obrazovka neni namapovana (skladani na GPU)")
        }
        (None, None) => String::from("obrazek nejde: obrazovka neni namapovana (skladani na GPU)"),
        (_, None) => String::from("obrazek nejde: chybi kanal na logd"),
        (Some((addr, w, h)), Some(ch)) => match bmp(addr, w, h, &comp.secure_fields()) {
            Ok((vmo, len)) => match ch.write(format!("snimek {id} {len}").as_bytes(), vec![vmo.into_handle()]) {
                Ok(()) => format!("obrazek {w}x{h} poslan do logd (ulozi ho do /system/log a na MO-LOGY)"),
                Err(e) => format!("obrazek nejde poslat do logd: {e}"),
            },
            Err(e) => format!("obrazek nejde vytvorit: {e}"),
        },
    };
    let t3 = time::now();
    let us = |a: u64, b: u64| (b - a) / 1000;
    println!(
        "compd: snimek {id}: {note}; cena: stav {} us, vypis {} radku ({bytes} B) {} us, obrazek {} us",
        us(t0, t1),
        lines.len(),
        us(t1, t2),
        us(t2, t3)
    );
}
