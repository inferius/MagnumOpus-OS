//! Registry radice xHCI (specifikace xHCI 1.2, kapitola 5).

use libos::Mapping;

// Capability registry (od zacatku BAR0).
pub const CAPLENGTH: u64 = 0x00;
pub const HCSPARAMS1: u64 = 0x04;
pub const HCSPARAMS2: u64 = 0x08;
pub const HCCPARAMS1: u64 = 0x10;
pub const DBOFF: u64 = 0x14;
pub const RTSOFF: u64 = 0x18;

// Operacni registry (od CAPLENGTH).
pub const USBCMD: u64 = 0x00;
pub const USBSTS: u64 = 0x04;
pub const CRCR: u64 = 0x18;
pub const DCBAAP: u64 = 0x30;
pub const CONFIG: u64 = 0x38;
/// Registry portu: `PORTSC` portu N (od 1) na 0x400 + 0x10 * (N - 1).
pub const PORT_BASE: u64 = 0x400;

pub const CMD_RUN: u32 = 1 << 0;
pub const CMD_RESET: u32 = 1 << 1;
pub const CMD_INTE: u32 = 1 << 2;

pub const STS_HALTED: u32 = 1 << 0;
pub const STS_HOST_ERROR: u32 = 1 << 2;
pub const STS_EINT: u32 = 1 << 3;
pub const STS_PCD: u32 = 1 << 4;
pub const STS_NOT_READY: u32 = 1 << 11;

// PORTSC.
pub const PORT_CCS: u32 = 1 << 0;
pub const PORT_PED: u32 = 1 << 1;
pub const PORT_OCA: u32 = 1 << 3;
pub const PORT_PR: u32 = 1 << 4;
pub const PORT_PP: u32 = 1 << 9;
pub const PORT_SPEED_SHIFT: u32 = 10;
pub const PORT_CSC: u32 = 1 << 17;
pub const PORT_PEC: u32 = 1 << 18;
pub const PORT_WRC: u32 = 1 << 19;
pub const PORT_OCC: u32 = 1 << 20;
pub const PORT_PRC: u32 = 1 << 21;
pub const PORT_PLC: u32 = 1 << 22;
pub const PORT_CEC: u32 = 1 << 23;
/// Bity, ktere se pri zapisu musi zachovat (ostatni jsou RW1C, strobe nebo
/// PED, jehoz zapis port vypne): napajeni, indikator, probuzeni.
pub const PORT_PRESERVE: u32 = PORT_PP | (3 << 14) | (7 << 25);

// Interrupter 0 (runtime registry + 0x20).
pub const IR0: u64 = 0x20;
pub const IMAN: u64 = 0x00;
pub const IMOD: u64 = 0x04;
pub const ERSTSZ: u64 = 0x08;
pub const ERSTBA: u64 = 0x10;
pub const ERDP: u64 = 0x18;
pub const IMAN_IP: u32 = 1 << 0;
pub const IMAN_IE: u32 = 1 << 1;
/// ERDP: Event Handler Busy (zapis 1 smaze).
pub const ERDP_EHB: u64 = 1 << 3;

// Rozsirene capability.
pub const XCAP_LEGACY: u8 = 1;
pub const XCAP_PROTOCOL: u8 = 2;
pub const LEGACY_BIOS_OWNED: u32 = 1 << 16;
pub const LEGACY_OS_OWNED: u32 = 1 << 24;

/// Okno na registry (BAR0) s offsety skupin.
pub struct Regs {
    pub map: Mapping,
    pub op: u64,
    pub rt: u64,
    pub db: u64,
}

impl Regs {
    pub fn new(map: Mapping) -> Self {
        let cap_len = map.read::<u8>(CAPLENGTH) as u64;
        let db = (map.read::<u32>(DBOFF) & !0x3) as u64;
        let rt = (map.read::<u32>(RTSOFF) & !0x1F) as u64;
        Self {
            map,
            op: cap_len,
            rt,
            db,
        }
    }

    pub fn cap32(&self, off: u64) -> u32 {
        self.map.read(off)
    }

    pub fn op32(&self, off: u64) -> u32 {
        self.map.read(self.op + off)
    }

    pub fn set_op32(&self, off: u64, v: u32) {
        self.map.write(self.op + off, v)
    }

    pub fn set_op64(&self, off: u64, v: u64) {
        self.map.write::<u32>(self.op + off, v as u32);
        self.map.write::<u32>(self.op + off + 4, (v >> 32) as u32);
    }

    pub fn set_ir32(&self, off: u64, v: u32) {
        self.map.write(self.rt + IR0 + off, v)
    }

    pub fn set_ir64(&self, off: u64, v: u64) {
        self.map.write::<u32>(self.rt + IR0 + off, v as u32);
        self.map.write::<u32>(self.rt + IR0 + off + 4, (v >> 32) as u32);
    }

    pub fn portsc(&self, port: u8) -> u32 {
        self.op32(PORT_BASE + 0x10 * (port as u64 - 1))
    }

    pub fn set_portsc(&self, port: u8, v: u32) {
        self.set_op32(PORT_BASE + 0x10 * (port as u64 - 1), v)
    }

    /// Zvoneni: slot 0 = prikazy (cil 0), jinak cil = DCI endpointu.
    pub fn ring_doorbell(&self, slot: u8, target: u32) {
        libos::barrier::mmio_wmb();
        self.map.write::<u32>(self.db + 4 * slot as u64, target);
    }

    pub fn read32(&self, off: u64) -> u32 {
        self.map.read(off)
    }

    pub fn write32(&self, off: u64, v: u32) {
        self.map.write(off, v)
    }
}
