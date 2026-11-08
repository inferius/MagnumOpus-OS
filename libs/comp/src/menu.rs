//! Menu oken (`MENU_UPDATE`, `guiproto::menu`): kompozitor drzi rozbalene
//! menu kazdeho okna (paleta se aplikace pri hledani nepta) a pri prijeti
//! ho overi:
//! - polozka smi odkazovat jen na akci z manifestu aplikace (system ji
//!   spusti stejne jako volani `actions`, vcetne potvrzeni),
//! - vyhrazene nazvy systemu (Vypnout, Restartovat, Odhlasit, Zamknout,
//!   Opravneni...) aplikace pouzit nesmi: polozka se zahodi.

use alloc::vec::Vec;

use guiproto::Error;
use guiproto::menu::{MenuItem, flatten};

use crate::{ClientId, Comp};

/// Vyhrazene nazvy (bez diakritiky, mala pismena): patri jen systemu.
const RESERVED: &[&str] = &["vypnout", "restartovat", "odhlasit", "zamknout", "opravneni", "diagnosticky snimek"];

/// Je text vyhrazeny systemu (i jako konec cesty "Soubor > Vypnout")?
pub fn reserved(label: &str) -> bool {
    let l = crate::palette::fold(label);
    let last = l.rsplit('›').next().unwrap_or(&l).trim();
    RESERVED.iter().any(|r| last == *r || last.starts_with(&alloc::format!("{r} ")))
}

impl Comp {
    /// Nove menu okna `window` klienta `c`.
    pub(crate) fn menu_update(&mut self, c: ClientId, window: u32, items: &[MenuItem]) {
        let Some(cl) = self.clients.get(&c) else { return };
        let Some(&gid) = cl.windows.get(&window) else {
            return self.error(c, guiproto::window::op::MENU_UPDATE, Error::NotFound);
        };
        let declared = |a: &str| cl.info.actions.iter().any(|d| d.id == a);
        let mut dropped = 0;
        let flat: Vec<_> = flatten(items)
            .into_iter()
            .filter(|f| {
                let ok = declared(&f.item.action) && !reserved(&f.path);
                dropped += !ok as u32;
                ok
            })
            .collect();
        let app = cl.info.id.clone();
        if dropped > 0 {
            self.trace(self.input_now, format_args!("menu {app}: {dropped} polozek zahozeno (akce mimo manifest nebo vyhrazeny nazev)"));
        }
        if let Some(w) = self.windows.get_mut(&gid) {
            w.menu = flat;
        }
        if self.palette.is_some() {
            let p = self.palette.clone();
            self.set_palette(p);
        }
    }
}
