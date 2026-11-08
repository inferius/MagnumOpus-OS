//! Stav pripojeni portu (hotplug) z registru ISR - bez preruseni, ovladac
//! je cte periodicky. Bity podle `intel_display_regs.h` (MIT):
//!
//! - SKL..CML (PCH SPT/CNP): port A v `GEN8_DE_PORT_ISR` bit 3, porty
//!   B/C/D v `SDEISR` bity 21/22/23.
//! - ICL+ (PCH ICP+): porty DDI A.. v `SDEISR` bity 16+, Type-C porty
//!   v `GEN11_DE_HPD_ISR` (DP alt mode 16+tc, Thunderbolt tc).
//!
//! ISR ukazuje aktualni uroven HPD (ne zachycenou udalost), takze cteni
//! ho nemaze. Na diskretnich kartach a MTL+ se cisla portu Type-C mohou
//! lisit - neovereno.

use crate::Mmio;
use crate::platform::{Family, Pch, Platform};
use crate::regs;

/// Je na portu (0 = A) pripojeny monitor?
pub fn connected<M: Mmio + ?Sized>(m: &mut M, p: &Platform, port: u32) -> bool {
    match (p.family, p.pch) {
        (Family::Gen9, _) => match port {
            0 => m.read32(regs::GEN8_DE_PORT_ISR) & regs::DE_PORT_HOTPLUG_A != 0,
            1 => m.read32(regs::SDEISR) & regs::SDE_PORTB_HOTPLUG_CPT != 0,
            2 => m.read32(regs::SDEISR) & regs::SDE_PORTC_HOTPLUG_CPT != 0,
            3 => m.read32(regs::SDEISR) & regs::SDE_PORTD_HOTPLUG_CPT != 0,
            _ => false,
        },
        (_, Pch::None) => port <= 3 && m.read32(regs::SDEISR) & regs::sde_ddi_hotplug_icp(port) != 0,
        _ => {
            if port <= 2 {
                m.read32(regs::SDEISR) & regs::sde_ddi_hotplug_icp(port) != 0
            } else {
                m.read32(regs::GEN11_DE_HPD_ISR) & regs::gen11_tc_hotplug(port - 3) != 0
            }
        }
    }
}
