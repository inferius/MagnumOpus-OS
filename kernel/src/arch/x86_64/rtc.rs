//! Hodiny CMOS (RTC, MC146818): zalozni zdroj realneho casu, kdyz ho
//! nedal firmware UEFI.
//!
//! Registry: 0 sekundy, 2 minuty, 4 hodiny, 7 den, 8 mesic, 9 rok (2 cislice),
//! stoleti v registru z ACPI FADT (jinak se bere 20xx), 0x0A stav A (bit 7 =
//! probiha aktualizace), 0x0B stav B (bit 2 = binarne misto BCD, bit 1 =
//! 24hodinovy rezim). Hodiny se ctou dvakrat po sobe, dokud se cteni shoduje
//! (behem aktualizace by se mohly smichat dve sekundy).
//!
//! Pozor: RTC nezna casove pasmo. Linux tam typicky ma UTC, Windows mistni
//! cas - jadro cas bere jako UTC a hlasi to priznakem. Zapis (`CLOCK_SET`
//! s `WRITE_HARDWARE`) dostane cas uz posunuty o pasmo od sluzby casu.

use abi::time::{Civil, WallTime};

use super::{cpu, time};
use crate::sync::SpinLock;

const INDEX: u16 = 0x70;
const DATA: u16 = 0x71;
/// Bit 7 indexoveho portu vypina NMI; nechavame NMI zapnute.
const NMI_ENABLED: u8 = 0;

fn read(reg: u8) -> u8 {
    // SAFETY: porty 0x70/0x71 patri CMOS; jadro je pouziva jen tady a
    // jen pri startu (na jednom CPU), takze se cteni neprolinaji.
    unsafe {
        cpu::outb(INDEX, NMI_ENABLED | reg);
        cpu::inb(DATA)
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
struct Raw([u8; 7]);

fn read_raw(century_reg: u8) -> Raw {
    // Pockat, az neprobiha aktualizace (trva < 2 ms; 1 s = mez).
    let end = time::now_ns() + 1_000_000_000;
    while read(0x0A) & 0x80 != 0 && time::now_ns() < end {
        core::hint::spin_loop();
    }
    let century = if century_reg != 0 { read(century_reg) } else { 0 };
    Raw([read(0), read(2), read(4), read(7), read(8), read(9), century])
}

fn bcd(v: u8) -> u8 {
    (v & 0x0F) + (v >> 4) * 10
}

/// Precte RTC; `century_reg` = registr stoleti z ACPI FADT (0 = neni).
/// `None` = hodiny vraci nesmysl (chybi, nebo vybita baterie).
pub fn read_time(century_reg: u8) -> Option<WallTime> {
    let mut last = read_raw(century_reg);
    let mut raw = last;
    for _ in 0..10 {
        raw = read_raw(century_reg);
        if raw == last {
            break;
        }
        last = raw;
    }
    if raw != last {
        return None;
    }
    let status_b = read(0x0B);
    let binary = status_b & 0x04 != 0;
    let h24 = status_b & 0x02 != 0;
    let conv = |v: u8| if binary { v } else { bcd(v) };
    let [s, m, h, d, mo, y, c] = raw.0;
    // Ve 12hodinovem rezimu je bit 7 hodin = odpoledne.
    let pm = !h24 && h & 0x80 != 0;
    let mut hour = conv(h & 0x7F);
    if !h24 {
        hour %= 12;
        if pm {
            hour += 12;
        }
    }
    let year2 = conv(y) as i64;
    let year = if century_reg != 0 && conv(c) >= 19 {
        conv(c) as i64 * 100 + year2
    } else {
        2000 + year2
    };
    let civil = Civil {
        year,
        month: conv(mo),
        day: conv(d),
        hour,
        minute: conv(m),
        second: conv(s),
        nanos: 0,
    };
    WallTime::from_civil(&civil)
}

/// Zamek pristupu k CMOS za behu (`CLOCK_SET` muze prijit z kterehokoli
/// CPU; dvojice index/data se nesmi prolnout).
static CMOS: SpinLock<()> = SpinLock::new(());

fn write(reg: u8, value: u8) {
    // SAFETY: porty 0x70/0x71 patri CMOS; za behu se k nim pristupuje jen
    // pod zamkem `CMOS` (cteni pri startu bezi drive, na jednom CPU).
    unsafe {
        cpu::outb(INDEX, NMI_ENABLED | reg);
        cpu::outb(DATA, value);
    }
}

fn to_bcd(v: u8) -> u8 {
    (v / 10) << 4 | v % 10
}

/// Zapise cas do RTC (ve formatu, ktery hodiny pouzivaji: BCD/binarne,
/// 12/24 h podle stavu B). `century_reg` = registr stoleti z ACPI FADT
/// (0 = neni, pak hodiny umi jen roky 2000..=2099). Rozliseni je 1 s,
/// nanosekundy se zahodi. `false` = rok mimo rozsah hodin.
pub fn write_time(century_reg: u8, t: WallTime) -> bool {
    let c = t.to_civil();
    let years = if century_reg != 0 { 1900..=9999 } else { 2000..=2099 };
    if !years.contains(&c.year) {
        return false;
    }
    // Den v tydnu: 1970-01-01 byl ctvrtek; RTC cisluje nedeli jako 1.
    let weekday = (t.secs.div_euclid(86_400) + 4).rem_euclid(7) as u8 + 1;
    let _guard = CMOS.lock();
    let status_b = read(0x0B);
    let binary = status_b & 0x04 != 0;
    let h24 = status_b & 0x02 != 0;
    let conv = |v: u8| if binary { v } else { to_bcd(v) };
    let hour = if h24 {
        conv(c.hour)
    } else {
        let h12 = match c.hour % 12 {
            0 => 12,
            h => h,
        };
        conv(h12) | if c.hour >= 12 { 0x80 } else { 0 }
    };
    // Bit SET (7) stavu B zastavi aktualizaci hodin behem zapisu (a prerusi
    // probihajici), takze neni treba cekat na konec aktualizace.
    write(0x0B, status_b | 0x80);
    write(0, conv(c.second));
    write(2, conv(c.minute));
    write(4, hour);
    write(6, conv(weekday));
    write(7, conv(c.day));
    write(8, conv(c.month));
    write(9, conv((c.year % 100) as u8));
    if century_reg != 0 {
        write(century_reg, conv((c.year / 100) as u8));
    }
    write(0x0B, status_b & !0x80);
    true
}
