//! Rezim vyvojare: jen vyvojove sestaveni (feature `dev-mode`, zapina ji
//! xtask pro obrazy s vyvojovym profilem - zive USB, `xtask run`,
//! `xtask vmware`). Ve vydani tenhle modul ani zpravy, ktere obsluhuje,
//! vubec nejsou (`docs/cs/ucty.md`, kapitola Rezim vyvojare).
//!
//! Je to **jedine misto**, kde se rezim projevi: politika potvrzeni
//! `ELEVATE`. Zapnuty rezim potvrdi ukon spravy bez dialogu za ucet s roli
//! spravy (stejny potvrzujici jako u dialogu), zapise ho do auditu jako
//! `vyvojar` a pak pokracuje normalni cesta (ukon uctu, nebo grant pro
//! sluzbu ukonu). Opravneni FS ani jine sluzby se nemeni.
//!
//! Prepinac: prikaz shellu `devmode on|off` (pres init na kanalu self
//! relace; jen ucet s roli spravy) a nabidka listy (fyzicky vstup do
//! systemoveho UI, zprava `DialogReply::DevMode` na kanalu `dialog`). Stav
//! plati pro cely pocitac, jen v pameti: po startu je rezim vypnuty.
//! Lista dostava stav (`DialogRequest::DevMode`) a dokud plati, kresli
//! trvaly pruh.
//!
//! Zname omezeni: `ELEVATE` z umeleho zdroje (agent AI) ma podle navrhu
//! rezim vyvojare stejne jako rezim spravy nepouzit, accountd ale zatim
//! puvod zadosti nerozlisi - dnes rezim potvrdi i zadost agenta.

use alloc::vec::Vec;

use accountproto::{DEV_OFF, DEV_ON};
use guiproto::control::DialogRequest;
use libos::{Error, Result, println};

use crate::service::Service;

/// Requester `0` v auditu = prepnuti z listy (clovek u pocitace).
const FROM_BAR: u64 = 0;

impl Service {
    /// `DEV_MODE` z relace `uid`: `set` = `DEV_QUERY` | `DEV_OFF` | `DEV_ON`.
    /// Odpoved: 1 B stav.
    pub fn dev_mode_request(&mut self, uid: u64, set: u8) -> Result<Vec<u8>> {
        if set == DEV_ON || set == DEV_OFF {
            let allowed = self.eng.account(uid).is_some_and(|a| self.eng.is_admin(a));
            if !allowed {
                return Err(Error::AccessDenied);
            }
            self.set_dev_mode(uid, set == DEV_ON);
        }
        Ok(alloc::vec![self.admin.dev as u8])
    }

    /// Prepnuti z listy (kanal `dialog` od kompozitoru).
    pub fn dev_mode_from_bar(&mut self, on: bool) {
        self.set_dev_mode(FROM_BAR, on);
    }

    fn set_dev_mode(&mut self, requester: u64, on: bool) {
        if self.admin.dev != on {
            self.admin.dev = on;
            println!("accountd: rezim vyvojare {} (zadal {requester})", if on { "zapnut" } else { "vypnut" });
            self.audit_line(requester, requester, 0, if on { "vyvojar-zapnut" } else { "vyvojar-vypnut" });
        }
        self.dev_mode_to_bar();
    }

    /// Stav pro listu (i nove instanci kompozitoru po padu).
    pub fn dev_mode_to_bar(&mut self) {
        let on = self.admin.dev;
        if let Some(d) = &self.admin.dialog
            && let Err(e) = d.write(&DialogRequest::DevMode { on }.encode(), Vec::new())
        {
            println!("accountd: stav rezimu vyvojare pro listu: {e}");
        }
    }

    /// Politika potvrzeni: v rezimu vyvojare potvrdi ukon sam za
    /// `confirm_uid` (ucet s roli). `None` = rezim neplati, dal dialog.
    pub fn dev_mode_confirm(&mut self, session: u64, uid: u64, confirm_uid: u64, act: u32, params: &[u8]) -> Option<Result<Vec<u8>>> {
        if !self.admin.dev {
            return None;
        }
        println!("accountd: ukon {act} potvrzen automaticky (rezim vyvojare, relace {session})");
        self.audit_line(uid, confirm_uid, act, "vyvojar");
        Some(self.perform(session, uid, act, params, false))
    }
}
