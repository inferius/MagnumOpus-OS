//! Bulk-Only Transport nad kanalem rozhrani (`usbclient`): jeden prikaz
//! SCSI = CBW, data, CSW. Obnova po chybach podle BOT 5.3 a 6.6.

use alloc::vec::Vec;
use core::time::Duration;

use libos::{Error, Result, println};
use usbclient::{UsbClient, UsbError, is_stall};
use usbproto::msg::{MAX_DATA, op};
use usbproto::storage::{self, Cbw, Csw, CswStatus, scsi};

/// Jak dlouho cekat na jeden bulk prenos (pomale flash disky pri zapisu
/// mazou bloky i stovky ms; SYNCHRONIZE CACHE muze trvat sekundy).
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(20);

/// Vysledek prikazu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Prikaz prosel; pocet prenesenych bajtu dat.
    Passed(usize),
    /// Zarizeni hlasi chybu (CSW Failed): duvod v REQUEST SENSE.
    Failed,
}

pub struct Bot {
    pub usb: UsbClient,
    pub interface: u8,
    pub ep_in: u8,
    pub ep_out: u8,
    tag: u32,
}

/// Chyba prenosu na kod `abi::Error` (STALL resi volajici zvlast).
fn err(e: UsbError) -> Error {
    e.error
}

impl Bot {
    pub fn new(usb: UsbClient, interface: u8, ep_in: u8, ep_out: u8) -> Self {
        Self {
            usb,
            interface,
            ep_in,
            ep_out,
            tag: 1,
        }
    }

    fn bulk_out(&mut self, data: &[u8]) -> core::result::Result<(), UsbError> {
        self.usb
            .call(op::BULK_OUT, self.ep_out as u32, 0, data, Some(TRANSFER_TIMEOUT))
            .map(|_| ())
    }

    fn bulk_in(&mut self, len: usize) -> core::result::Result<Vec<u8>, UsbError> {
        self.usb
            .call(op::BULK_IN, self.ep_in as u32, len as u32, &[], Some(TRANSFER_TIMEOUT))
            .map(|t| t.data)
    }

    /// Uvolni zastaveny endpoint na strane zarizeni (radic uz svou stranu
    /// obnovil sam).
    fn clear_halt(&mut self, ep: u8) {
        if let Err(e) = self.usb.control(storage::clear_halt(ep), &[]) {
            println!("usb-storage: CLEAR_FEATURE(HALT) {ep:#04x} selhalo: {e}");
        }
    }

    /// Reset Recovery (BOT 5.3.4): reset rozhrani a uvolneni obou
    /// endpointu. Po nem zarizeni ceka na novy CBW.
    pub fn reset_recovery(&mut self) {
        if let Err(e) = self.usb.control(storage::reset(self.interface), &[]) {
            println!("usb-storage: Bulk-Only reset selhal: {e}");
        }
        let (i, o) = (self.ep_in, self.ep_out);
        self.clear_halt(i);
        self.clear_halt(o);
    }

    /// Precte CSW; po STALL jeden dalsi pokus (BOT 6.7.2 a 6.7.3).
    fn read_csw(&mut self) -> Result<Csw> {
        let mut raw = self.bulk_in(storage::CSW_LEN);
        if let Err(e) = &raw
            && is_stall(e)
        {
            let ep = self.ep_in;
            self.clear_halt(ep);
            raw = self.bulk_in(storage::CSW_LEN);
        }
        let raw = raw.map_err(err)?;
        Csw::parse(&raw).ok_or(Error::Corrupted)
    }

    /// Provede prikaz `cdb` pro `lun`. Data IN: `data_in` = kolik bajtu
    /// cist (vysledek v `out`); data OUT: `data_out`.
    pub fn command(
        &mut self,
        lun: u8,
        cdb: &[u8],
        data_in: usize,
        out: &mut Vec<u8>,
        data_out: &[u8],
    ) -> Result<Status> {
        let r = self.command_once(lun, cdb, data_in, out, data_out);
        if matches!(r, Err(Error::Corrupted) | Err(Error::IoError)) {
            // Zarizeni je mimo synchronizaci (neplatne CSW, chyba faze,
            // chyba prenosu): obnovit, prikaz zopakuje volajici.
            self.reset_recovery();
        }
        r
    }

    fn command_once(
        &mut self,
        lun: u8,
        cdb: &[u8],
        data_in: usize,
        out: &mut Vec<u8>,
        data_out: &[u8],
    ) -> Result<Status> {
        let tag = self.tag;
        self.tag = self.tag.wrapping_add(1).max(1);
        let is_in = data_in > 0;
        let length = if is_in { data_in } else { data_out.len() };
        let cbw = Cbw::new(tag, lun, length as u32, is_in, cdb).encode();
        if let Err(e) = self.bulk_out(&cbw) {
            // CBW odmitnut: zarizeni je zasekle (BOT 6.6.1).
            println!("usb-storage: CBW neprosel: {e}");
            return Err(Error::IoError);
        }
        out.clear();
        let mut moved = 0usize;
        // Datova faze po kusech nejvys MAX_DATA (jedna zprava kanalu).
        // STALL ukonci fazi predcasne (zarizeni nema vic dat nebo zapis
        // odmitlo): uvolnit endpoint a pokracovat ke CSW.
        while moved < length {
            let n = (length - moved).min(MAX_DATA);
            if is_in {
                match self.bulk_in(n) {
                    Ok(d) => {
                        let short = d.len() < n;
                        moved += d.len();
                        out.extend_from_slice(&d);
                        if short {
                            break;
                        }
                    }
                    Err(e) if is_stall(&e) => {
                        let ep = self.ep_in;
                        self.clear_halt(ep);
                        break;
                    }
                    Err(e) => return Err(err(e)),
                }
            } else {
                match self.bulk_out(&data_out[moved..moved + n]) {
                    Ok(()) => moved += n,
                    Err(e) if is_stall(&e) => {
                        let ep = self.ep_out;
                        self.clear_halt(ep);
                        break;
                    }
                    Err(e) => return Err(err(e)),
                }
            }
        }
        let csw = self.read_csw()?;
        if csw.tag != tag {
            return Err(Error::Corrupted);
        }
        match csw.status {
            CswStatus::Passed => {
                // U zapisu rika residue, kolik dat zarizeni nevzalo.
                let done = if is_in { moved } else { length.saturating_sub(csw.residue as usize) };
                Ok(Status::Passed(done))
            }
            CswStatus::Failed => Ok(Status::Failed),
            CswStatus::PhaseError => Err(Error::IoError),
        }
    }

    /// REQUEST SENSE: duvod posledni chyby (`None` = nejde zjistit).
    pub fn sense(&mut self, lun: u8) -> Option<scsi::Sense> {
        let mut out = Vec::new();
        match self.command(lun, &scsi::request_sense(), scsi::SENSE_LEN as usize, &mut out, &[]) {
            Ok(Status::Passed(_)) => scsi::parse_sense(&out),
            _ => None,
        }
    }
}
