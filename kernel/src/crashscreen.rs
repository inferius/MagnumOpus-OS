//! Obrazovka pady jadra (navrh `design/pad.html`, vyvojova verze: rovnou
//! technicka zprava).
//!
//! Kresli se primo do framebufferu z firmwaru (i kdyz ho mel kompozitor),
//! bez alokace: nadpis, kod a id zaznamu, kod QR se souhrnem (telefonem
//! vyfotit a poslat vyvojari), konec logu jadra s hlasenim pady a odpocet
//! restartu. Pocitac s nativnim ovladacem grafiky (virtio-gpu, Intel) uz
//! framebuffer firmwaru nemusi zobrazovat - zaznam se pak ulozi jen do RAM
//! a na seriovou linku.

use core::fmt::Write;

use crate::console;
use crate::framebuffer::FbConsole;

/// Za jak dlouho se po pade restartuje (fotka obrazovky, pak zaznam
/// pri dalsim startu ulozi sluzba logu).
pub const RESTART_AFTER_NS: u64 = 60_000_000_000;

/// Kolik poslednich radku logu ukazat.
const LOG_LINES: usize = 40;

/// Ukaze obrazovku pady pro zaznam `record` (`None` = oblast zaznamu neni).
pub fn show(record: Option<(crashrec::Header, u32)>) {
    let Some(fb) = console::framebuffer() else {
        return;
    };
    // SAFETY: ostatni CPU stoji a konzole uz nic nekresli (fatal_stop);
    // framebuffer je namapovany po celou dobu behu.
    let mut con = unsafe { FbConsole::new(&fb) };
    let (w, h) = con.size();
    let text = crate::pstore::text();

    // Kod QR vpravo nahore: modul aspon 3 px, cely kod nejvys tretina vysky.
    let mut payload = [0u8; 180];
    let n = qr_payload(record.as_ref(), text, &mut payload);
    let qr = qr::encode(&payload[..n]);
    let mut right = w;
    if let Some(q) = &qr {
        let quiet = 4;
        let px = (h / 3 / (q.size + 2 * quiet)).max(3);
        let side = (q.size + 2 * quiet) * px;
        let x0 = w.saturating_sub(side + 16);
        let y0 = 16;
        con.fill_rect(x0, y0, side, side, (0xFF, 0xFF, 0xFF));
        for y in 0..q.size {
            for x in 0..q.size {
                if q.get(x, y) {
                    con.fill_rect(x0 + (x + quiet) * px, y0 + (y + quiet) * px, px, px, (0, 0, 0));
                }
            }
        }
        right = x0.saturating_sub(16);
    }
    con.set_crash_layout(right);

    let _ = writeln!(con, "Systém musel zastavit");
    let _ = writeln!(con);
    match &record {
        Some((h, crc)) => {
            let id = crashrec::crash_id(*crc);
            let _ = writeln!(
                con,
                "{} · #{}",
                crashrec::field_str(&h.reason),
                core::str::from_utf8(&id).unwrap_or("?")
            );
            let _ = writeln!(con, "{}", crashrec::field_str(&h.version));
            let _ = writeln!(con, "Hlášení je uložené v paměti; po dalším startu ho služba logu");
            let _ = writeln!(con, "zapíše do /system/pady na datovém oddílu.");
        }
        None => {
            let _ = writeln!(con, "Hlášení nejde uložit (chybí vyhrazená paměť) - vyfoťte obrazovku.");
        }
    }
    let _ = writeln!(con, "Kód QR: souhrn pro vývojáře (vyfoťte telefonem).");
    let _ = writeln!(con, "Restart za {} s.", RESTART_AFTER_NS / 1_000_000_000);
    let _ = writeln!(con);

    // Konec logu: hlaseni pady, registry, zasobnik, posledni zpravy.
    let mut start = text.len();
    let mut lines = 0;
    while start > 0 && lines <= LOG_LINES {
        start -= 1;
        if text[start] == b'\n' {
            lines += 1;
        }
    }
    let tail = &text[if start == 0 { 0 } else { start + 1 }..];
    let _ = con.write_str(core::str::from_utf8(tail).unwrap_or("(log neni UTF-8)"));
}

/// Text kodu QR: id, duvod, verze, cas od startu a radek s hlasenim
/// (`KERNEL...`). Vraci delku.
fn qr_payload(record: Option<&(crashrec::Header, u32)>, text: &[u8], out: &mut [u8]) -> usize {
    struct Buf<'a> {
        b: &'a mut [u8],
        n: usize,
    }
    impl Write for Buf<'_> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            for &c in s.as_bytes() {
                if self.n < self.b.len() {
                    self.b[self.n] = c;
                    self.n += 1;
                }
            }
            Ok(())
        }
    }
    let mut b = Buf { b: out, n: 0 };
    match record {
        Some((h, crc)) => {
            let _ = write!(
                b,
                "MO-CRASH #{} {} {} t={}s\n",
                core::str::from_utf8(&crashrec::crash_id(*crc)).unwrap_or("?"),
                crashrec::field_str(&h.reason),
                crashrec::field_str(&h.version),
                h.uptime_ns / 1_000_000_000
            );
        }
        None => {
            let _ = b.write_str("MO-CRASH (bez zaznamu)\n");
        }
    }
    // Posledni radek s hlasenim pady jadra.
    let report = text
        .split(|&c| c == b'\n')
        .filter(|l| l.starts_with(b"KERNEL"))
        .next_back();
    if let Some(l) = report {
        let _ = b.write_str(core::str::from_utf8(l).unwrap_or(""));
    }
    b.n
}
