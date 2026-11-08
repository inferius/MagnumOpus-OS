//! Kanal AUX DisplayPortu na Intel (pred Meteor Lake): `DDI_AUX_CTL`
//! a 5 datovych registru (20 B, bajty big-endian v ramci slova).
//!
//! Postup podle `intel_dp_aux.c` (SPDX MIT): data zapsat, v CTL nastavit
//! `SEND_BUSY` s delkou, casovym limitem a synchronizacnimi pulzy (SKL:
//! 32 + 32) a zaroven zapsat 1 do `DONE` a chybovych bitu (tim se smazou),
//! cekat na konec `SEND_BUSY`, chyby (timeout = nic nepripojeno, chyba
//! prijmu), pak precist odpoved. Opakovani pri DEFER resi
//! `display::dp::auxch::transact`.

use display::dp::auxch::Aux;
use display::{Error, Result};

use crate::regs::{self, aux};
use crate::{Clock, Mmio, wait_for};

/// AUX kanal `ch` (0 = A ... u TGL+ D.. = Type-C porty).
pub struct IntelAux<'a, M: Mmio + ?Sized, C: Clock + ?Sized> {
    pub mmio: &'a mut M,
    pub clock: &'a mut C,
    pub ch: u32,
}

/// Limit cekani na dokonceni transakce (hardware ma vlastni 1.6 ms).
const BUSY_TIMEOUT_US: u32 = 10_000;

impl<M: Mmio + ?Sized, C: Clock + ?Sized> Aux for IntelAux<'_, M, C> {
    fn transfer(&mut self, tx: &[u8], rx: &mut [u8; 20]) -> Result<usize> {
        if tx.is_empty() || tx.len() > 20 {
            return Err(Error::Invalid);
        }
        let ctl = regs::aux_ctl(self.ch);
        if self.mmio.read32(ctl) & aux::SEND_BUSY != 0 {
            return Err(Error::Io);
        }
        let mut words = [0u8; 20];
        words[..tx.len()].copy_from_slice(tx);
        for i in 0..5 {
            let w = u32::from_be_bytes([words[i * 4], words[i * 4 + 1], words[i * 4 + 2], words[i * 4 + 3]]);
            self.mmio.write32(regs::aux_data(self.ch, i as u32), w);
        }
        let cmd = aux::SEND_BUSY
            | aux::DONE
            | aux::TIME_OUT_ERROR
            | aux::TIME_OUT_MAX
            | aux::RECEIVE_ERROR
            | (tx.len() as u32) << aux::SIZE_SHIFT
            | aux::SYNC_PULSES_SKL;
        self.mmio.write32(ctl, cmd);
        let st = wait_for(self.mmio, self.clock, ctl, BUSY_TIMEOUT_US, |v| v & aux::SEND_BUSY == 0).ok_or(Error::Timeout)?;
        // Stavove bity smazat (zapis 1).
        self.mmio.write32(ctl, st & (aux::DONE | aux::TIME_OUT_ERROR | aux::RECEIVE_ERROR));
        if st & aux::TIME_OUT_ERROR != 0 {
            return Err(Error::Timeout);
        }
        if st & aux::RECEIVE_ERROR != 0 {
            return Err(Error::Io);
        }
        let n = (((st & aux::SIZE_MASK) >> aux::SIZE_SHIFT) as usize).min(20);
        if n == 0 {
            return Err(Error::Io);
        }
        for i in 0..5 {
            let w = self.mmio.read32(regs::aux_data(self.ch, i as u32)).to_be_bytes();
            rx[i * 4..i * 4 + 4].copy_from_slice(&w);
        }
        Ok(n)
    }

    fn delay_us(&mut self, us: u32) {
        self.clock.delay_us(us);
    }
}
