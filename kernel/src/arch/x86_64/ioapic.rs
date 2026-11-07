//! IOAPIC: stare linky preruseni (ISA IRQ 0-15, pozdeji PCI INTx).
//!
//! Zarizeni PCIe pouzivaji MSI; IOAPIC potrebuji jen starsi zarizeni na
//! deskach (PS/2 klavesnice IRQ 1, mys IRQ 12). Linka ISA se prevadi na
//! globalni cislo preruseni (GSI) podle prepisu z MADT (Interrupt Source
//! Override, napr. ISA 0 -> GSI 2), vcetne polarity a spousteni.
//!
//! Redirection entry (64 bitu): vektor (0-7), doruceni fixed (8-10 = 0),
//! cil fyzicky (11 = 0), polarita (13: 1 = aktivni v nule), spousteni
//! (15: 1 = urovni), maska (16), cil APIC ID (56-63).

use alloc::vec::Vec;

use super::acpi::{InterruptOverride, IoApic};
use crate::error::{Error, Result};
use crate::mm::{self, PhysAddr};
use crate::sync::SpinLock;

const REG_SELECT: usize = 0x00;
const REG_WINDOW: usize = 0x10;
const REG_VERSION: u32 = 0x01;
const REG_REDIR: u32 = 0x10;

const REDIR_ACTIVE_LOW: u64 = 1 << 13;
const REDIR_LEVEL: u64 = 1 << 15;
const REDIR_MASKED: u64 = 1 << 16;

/// Nejvyssi linka ISA.
pub const ISA_LINES: u32 = 16;

struct Chip {
    base: usize,
    gsi_base: u32,
    /// Pocet vstupu (redirection entries).
    inputs: u32,
}

struct State {
    chips: Vec<Chip>,
    overrides: Vec<InterruptOverride>,
    /// Linky ISA, ktere maji objekt preruseni (nejvys jeden na linku).
    used: u32,
}

static STATE: SpinLock<Option<State>> = SpinLock::new(None);

impl Chip {
    fn read(&self, reg: u32) -> u32 {
        // SAFETY: base je namapovane okno registru IOAPIC (4 KiB);
        // pristup pres vyber registru a okno je jeho jediny protokol.
        // Volajici drzi zamek STATE, takze se vyber a okno neprolinaji.
        unsafe {
            ((self.base + REG_SELECT) as *mut u32).write_volatile(reg);
            ((self.base + REG_WINDOW) as *const u32).read_volatile()
        }
    }

    fn write(&self, reg: u32, value: u32) {
        // SAFETY: viz `read`.
        unsafe {
            ((self.base + REG_SELECT) as *mut u32).write_volatile(reg);
            ((self.base + REG_WINDOW) as *mut u32).write_volatile(value);
        }
    }

    fn write_entry(&self, input: u32, value: u64) {
        // Nejdriv horni polovinu (cil), pak dolni (vektor, maska): zmena
        // masky prijde az s platnym cilem.
        self.write(REG_REDIR + 2 * input + 1, (value >> 32) as u32);
        self.write(REG_REDIR + 2 * input, value as u32);
    }

    fn read_low(&self, input: u32) -> u32 {
        self.read(REG_REDIR + 2 * input)
    }
}

/// Namapuje IOAPIC z MADT a zamaskuje vsechny vstupy (firmware je mohl
/// nechat smerovane). Volat jednou pri startu.
pub fn init(ioapics: &[IoApic], overrides: &[InterruptOverride]) {
    let mut chips = Vec::new();
    for io in ioapics {
        let base = mm::map_mmio(PhysAddr::new(io.phys), 4096).as_u64() as usize;
        let mut chip = Chip {
            base,
            gsi_base: io.gsi_base,
            inputs: 0,
        };
        chip.inputs = ((chip.read(REG_VERSION) >> 16) & 0xFF) + 1;
        for i in 0..chip.inputs {
            chip.write_entry(i, REDIR_MASKED);
        }
        chips.push(chip);
    }
    *STATE.lock() = Some(State {
        chips,
        overrides: overrides.to_vec(),
        used: 0,
    });
}

impl State {
    /// GSI a priznaky entry pro linku ISA (vychozi: hrana, aktivni v jednicce).
    fn isa(&self, line: u32) -> (u32, u64) {
        match self.overrides.iter().find(|o| o.source_irq as u32 == line) {
            Some(o) => {
                // Polarita bity 0-1 (3 = v nule), spousteni bity 2-3 (3 = urovni).
                let mut flags = 0;
                if o.flags & 3 == 3 {
                    flags |= REDIR_ACTIVE_LOW;
                }
                if (o.flags >> 2) & 3 == 3 {
                    flags |= REDIR_LEVEL;
                }
                (o.gsi, flags)
            }
            None => (line, 0),
        }
    }

    fn chip(&self, gsi: u32) -> Option<(&Chip, u32)> {
        self.chips.iter().find_map(|c| {
            let input = gsi.checked_sub(c.gsi_base)?;
            (input < c.inputs).then_some((c, input))
        })
    }
}

/// Nasmeruje linku ISA `line` na vektor `vector` na CPU `dest` (APIC ID).
/// Vraci, zda je linka spoustena urovni (pak ji obsluha preruseni maskuje
/// do potvrzeni). Linka uz pouzita = `ALREADY_EXISTS`.
pub fn route(line: u32, vector: u8, dest: u32) -> Result<bool> {
    if line >= ISA_LINES {
        return Err(Error::InvalidArgs);
    }
    let mut guard = STATE.lock();
    let st = guard.as_mut().ok_or(Error::NotSupported)?;
    if st.used & (1 << line) != 0 {
        return Err(Error::AlreadyExists);
    }
    let (gsi, flags) = st.isa(line);
    let (chip, input) = st.chip(gsi).ok_or(Error::NotFound)?;
    // xAPIC fyzicky cil ma 8 bitu; vetsi ID by chtelo preklad preruseni.
    let dest = u8::try_from(dest).map_err(|_| Error::NotSupported)? as u64;
    chip.write_entry(input, dest << 56 | flags | vector as u64);
    st.used |= 1 << line;
    Ok(flags & REDIR_LEVEL != 0)
}

/// Zamaskuje nebo odmaskuje linku ISA (linka spoustena urovni do potvrzeni).
pub fn set_masked(line: u32, masked: bool) {
    let guard = STATE.lock();
    let Some(st) = guard.as_ref() else {
        return;
    };
    let (gsi, _) = st.isa(line);
    if let Some((chip, input)) = st.chip(gsi) {
        let low = chip.read_low(input) as u64;
        let low = if masked {
            low | REDIR_MASKED
        } else {
            low & !REDIR_MASKED
        };
        chip.write(REG_REDIR + 2 * input, low as u32);
    }
}

/// Uvolni linku (objekt preruseni zanikl): zamaskovat a oznacit volnou.
pub fn release(line: u32) {
    if line >= ISA_LINES {
        return;
    }
    let mut guard = STATE.lock();
    let Some(st) = guard.as_mut() else {
        return;
    };
    let (gsi, _) = st.isa(line);
    if let Some((chip, input)) = st.chip(gsi) {
        chip.write_entry(input, REDIR_MASKED);
    }
    st.used &= !(1 << line);
}
