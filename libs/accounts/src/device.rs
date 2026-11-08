//! Klic zarizeni: tajemstvi vazane na pocitac, na kterem zavisi PIN.
//!
//! Dnes [`FileKey`] (32 B v `/db/device.key`, chranene jen opravnenim FS).
//! Pozdeji TPM 2.0: HMAC uvnitr cipu, klic ho neopusti, citac proti hadani
//! vede TPM. POZOR: bez TPM PIN chrani jen proti hadani u klavesnice, ne proti
//! kradezi disku (viz `docs/cs/ucty.md`, kapitola PIN).

use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::Zeroize;

pub trait DeviceKey {
    /// HMAC-SHA-256 klicem zarizeni nad `label || data`.
    fn mac(&self, label: &[u8], data: &[u8]) -> [u8; 32];
    /// Klic je v hardwaru (TPM): PIN neni slaby.
    fn hardware(&self) -> bool;
}

/// Klic zarizeni v souboru (dokud neni TPM).
pub struct FileKey {
    key: [u8; 32],
}

impl FileKey {
    pub fn new(key: [u8; 32]) -> Self {
        FileKey { key }
    }
}

impl Drop for FileKey {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

impl DeviceKey for FileKey {
    fn mac(&self, label: &[u8], data: &[u8]) -> [u8; 32] {
        let mut m = <Hmac<Sha256> as Mac>::new_from_slice(&self.key).expect("klic HMAC");
        m.update(&(label.len() as u32).to_le_bytes());
        m.update(label);
        m.update(data);
        m.finalize().into_bytes().into()
    }

    fn hardware(&self) -> bool {
        false
    }
}
