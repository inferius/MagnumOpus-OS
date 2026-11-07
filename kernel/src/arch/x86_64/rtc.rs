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
//!
//! Zapis drzi delic hodin (stav A) v resetu: po jeho uvolneni prijde prvni
//! sekunda za pul sekundy, proto jadro zapisuje v polovine sekundy
//! ([`WRITE_PHASE_NS`]) a hodiny pak tikaji na hranici sekund (bez toho by
//! byly az o 1 s pozadu).

use abi::time::{Civil, WallTime};

use super::{cpu, time};
use crate::sync::SpinLock;

const INDEX: u16 = 0x70;
const DATA: u16 = 0x71;
/// Bit 7 indexoveho portu vypina NMI; nechavame NMI zapnute.
const NMI_ENABLED: u8 = 0;

/// Pristup k portum CMOS. Jedina instance je v zamku [`CMOS`], takze cist
/// a zapisovat registry jde jen pod nim: dvojice index/data se nikdy
/// neprolne (`CLOCK_SET` muze prijit z kterehokoli CPU; zamek zaroven
/// vypne preruseni).
struct Port(());

static CMOS: SpinLock<Port> = SpinLock::new(Port(()));

impl Port {
    fn read(&self, reg: u8) -> u8 {
        // SAFETY: porty 0x70/0x71 patri CMOS; `&self` dokazuje drzeni
        // zamku `CMOS`, takze se pristupy neprolinaji.
        unsafe {
            cpu::outb(INDEX, NMI_ENABLED | reg);
            cpu::inb(DATA)
        }
    }

    fn write(&self, reg: u8, value: u8) {
        // SAFETY: jako u `read`.
        unsafe {
            cpu::outb(INDEX, NMI_ENABLED | reg);
            cpu::outb(DATA, value);
        }
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
struct Raw([u8; 7]);

fn read_raw(port: &Port, century_reg: u8) -> Raw {
    // Pockat, az neprobiha aktualizace (trva < 2 ms; 1 s = mez).
    let end = time::now_ns() + 1_000_000_000;
    while port.read(0x0A) & 0x80 != 0 && time::now_ns() < end {
        core::hint::spin_loop();
    }
    let century = if century_reg != 0 { port.read(century_reg) } else { 0 };
    let r = |reg| port.read(reg);
    Raw([r(0), r(2), r(4), r(7), r(8), r(9), century])
}

fn bcd(v: u8) -> u8 {
    (v & 0x0F) + (v >> 4) * 10
}

/// Precte RTC; `century_reg` = registr stoleti z ACPI FADT (0 = neni).
/// `None` = hodiny vraci nesmysl (chybi, nebo vybita baterie).
pub fn read_time(century_reg: u8) -> Option<WallTime> {
    let port = CMOS.lock();
    let mut last = read_raw(&port, century_reg);
    let mut raw = last;
    for _ in 0..10 {
        raw = read_raw(&port, century_reg);
        if raw == last {
            break;
        }
        last = raw;
    }
    if raw != last {
        return None;
    }
    let status_b = port.read(0x0B);
    drop(port);
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

fn to_bcd(v: u8) -> u8 {
    (v / 10) << 4 | v % 10
}

/// Za jak dlouho po zapisu hodiny poprve pricitaji sekundu: po uvolneni
/// resetu delice (stav A) zacne prvni aktualizace az za pul sekundy.
/// Volajici proto zapisuje v polovine sekundy (viz `api::Time`), a hodiny
/// pak tikaji presne na hranici sekund.
pub const WRITE_PHASE_NS: u32 = 500_000_000;

/// Delic stavu A (bity 6..4): 010 = bezny chod z krystalu 32,768 kHz,
/// 11x = delic v resetu (hodiny stoji, citac zlomku sekundy se nuluje).
/// Bity 3..0 = frekvence periodickeho preruseni (zachovat), bit 7 = jen
/// pro cteni (probiha aktualizace).
const RATE_MASK: u8 = 0x0F;
const DIVIDER_RESET: u8 = 0x60;
const DIVIDER_NORMAL: u8 = 0x20;

/// Zapise cas do RTC (ve formatu, ktery hodiny pouzivaji: BCD/binarne,
/// 12/24 h podle stavu B). `century_reg` = registr stoleti z ACPI FADT
/// (0 = neni, pak hodiny umi jen roky 2000..=2099). `false` = rok mimo
/// rozsah hodin.
///
/// Zlomek sekundy: zapis drzi delic v resetu, takze dalsi sekunda prijde
/// [`WRITE_PHASE_NS`] po zapisu. S `t.nanos` kolem 0,5 s hodiny tikaji
/// presne; jinak je chyba `|t.nanos - 0,5 s|`.
pub fn write_time(century_reg: u8, t: WallTime) -> bool {
    let c = t.to_civil();
    let years = if century_reg != 0 { 1900..=9999 } else { 2000..=2099 };
    if !years.contains(&c.year) {
        return false;
    }
    // RTC cisluje dny v tydnu 1..=7 od nedele.
    let weekday = abi::time::weekday(t.secs.div_euclid(86_400)) + 1;
    let port = CMOS.lock();
    let status_a = port.read(0x0A);
    let status_b = port.read(0x0B);
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
    // Bit SET (7) stavu B zastavi aktualizaci hodin behem zapisu; reset
    // delice vynuluje zlomek sekundy.
    port.write(0x0B, status_b | 0x80);
    port.write(0x0A, (status_a & RATE_MASK) | DIVIDER_RESET);
    port.write(0, conv(c.second));
    port.write(2, conv(c.minute));
    port.write(4, hour);
    port.write(6, conv(weekday));
    port.write(7, conv(c.day));
    port.write(8, conv(c.month));
    port.write(9, conv((c.year % 100) as u8));
    if century_reg != 0 {
        port.write(century_reg, conv((c.year / 100) as u8));
    }
    port.write(0x0B, status_b & !0x80);
    // Uvolneni delice: prvni aktualizace za WRITE_PHASE_NS. Bity 3..0
    // (periodicke preruseni) zustavaji, jak byly.
    port.write(0x0A, (status_a & RATE_MASK) | DIVIDER_NORMAL);
    true
}
