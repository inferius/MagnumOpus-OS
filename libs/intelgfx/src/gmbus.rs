//! GMBUS: hardwarovy radic I2C pro DDC (EDID u HDMI a DVI).
//!
//! Postup podle `intel_gmbus.c` (MIT): vyber pinu v GMBUS0, cteni s
//! indexem (offset v EDID) v GMBUS1, data po 4 B z GMBUS3 (cekat na
//! `HW_RDY`, `SATOER` = NAK), nakonec cyklus STOP a GMBUS0 = 0. Bloky
//! EDID 0 a 1 (offset 0 a 128); segmenty E-DDC (bloky 2+) GMBUS
//! nepouziva - u HDMI monitoru jsou vzacne.

use crate::platform::Pch;
use crate::regs::{self, gmbus};
use crate::{Clock, Mmio, wait_for};
use display::{Error, Result};

/// Pin GMBUS pro port DDI (`intel_gmbus.h`, tabulky `gmbus_pins_*`).
/// Port: 0 = A, 1 = B, ...; Type-C porty od 3 (TC1) u ICP+.
pub fn pin_for_port(pch: Pch, port: u32) -> Option<u32> {
    match pch {
        // SPT: DPB = 5, DPC = 4, DPD = 6.
        Pch::Spt => match port {
            1 => Some(5),
            2 => Some(4),
            3 => Some(6),
            _ => None,
        },
        // CNP: 1 = dpb, 2 = dpc, 4 = dpd.
        Pch::Cnp => match port {
            1 => Some(1),
            2 => Some(2),
            3 => Some(4),
            _ => None,
        },
        // ICP+: 1 = dpa, 2 = dpb, 3 = dpc, 9.. = TC1...
        Pch::Icp => match port {
            0..=2 => Some(port + 1),
            3..=8 => Some(port - 3 + 9),
            _ => None,
        },
        // Diskretni (DG2): jako DG1 (1..4), TC1 = 9.
        Pch::None => match port {
            0..=3 => Some(port + 1),
            4 => Some(9),
            _ => None,
        },
    }
}

const TIMEOUT_US: u32 = 50_000;

fn wait_ready<M: Mmio + ?Sized, C: Clock + ?Sized>(m: &mut M, c: &mut C) -> Result<()> {
    let s = wait_for(m, c, regs::GMBUS2, TIMEOUT_US, |s| s & (gmbus::HW_RDY | gmbus::SATOER) != 0).ok_or(Error::Timeout)?;
    if s & gmbus::SATOER != 0 { Err(Error::Nack) } else { Ok(()) }
}

fn stop<M: Mmio + ?Sized, C: Clock + ?Sized>(m: &mut M, c: &mut C) {
    m.write32(regs::GMBUS1, gmbus::SW_RDY | gmbus::CYCLE_STOP);
    let _ = wait_for(m, c, regs::GMBUS2, TIMEOUT_US, |s| s & gmbus::ACTIVE == 0);
    m.write32(regs::GMBUS0, 0);
}

/// Precte `buf.len()` bajtu (nejvic 256) od offsetu `index` z adresy `addr`.
pub fn read<M: Mmio + ?Sized, C: Clock + ?Sized>(m: &mut M, c: &mut C, pin: u32, addr: u8, index: u8, buf: &mut [u8]) -> Result<()> {
    if buf.is_empty() || buf.len() > 256 {
        return Err(Error::Invalid);
    }
    // Reset po predchozi chybe.
    m.write32(regs::GMBUS1, gmbus::SW_CLR_INT);
    m.write32(regs::GMBUS1, 0);
    m.write32(regs::GMBUS0, pin | gmbus::RATE_100KHZ);
    let cmd = gmbus::SW_RDY
        | gmbus::CYCLE_INDEX
        | gmbus::CYCLE_WAIT
        | (buf.len() as u32) << gmbus::BYTE_COUNT_SHIFT
        | (index as u32) << gmbus::INDEX_SHIFT
        | (addr as u32) << gmbus::ADDR_SHIFT
        | gmbus::READ;
    m.write32(regs::GMBUS1, cmd);
    let mut done = 0;
    let r = (|| {
        while done < buf.len() {
            wait_ready(m, c)?;
            let w = m.read32(regs::GMBUS3).to_le_bytes();
            for &b in w.iter().take(buf.len() - done) {
                buf[done] = b;
                done += 1;
            }
        }
        Ok(())
    })();
    stop(m, c);
    r
}

/// Blok EDID 0 nebo 1 pres DDC.
pub fn read_edid_block<M: Mmio + ?Sized, C: Clock + ?Sized>(m: &mut M, c: &mut C, pin: u32, block: u8, buf: &mut [u8]) -> Result<()> {
    if block > 1 {
        return Err(Error::Invalid);
    }
    read(m, c, pin, 0x50, block * 128, &mut buf[..128])
}
