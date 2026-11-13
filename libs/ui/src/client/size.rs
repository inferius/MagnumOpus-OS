//! Pozadovana velikost okna (`WINDOW_SIZE`).

use alloc::vec::Vec;

use guiproto::window::ClientMsg;
use libos::{Error, Result};

use super::{Event, Window};

/// Jak dlouho pred prvnim zobrazenim cekat na odpoved kompozitoru (ms).
const WAIT_MS: u64 = 1000;

impl Window {
    /// Pozada o velikost obsahu `w` x `h` (logicke px, 0 = nechat na
    /// systemu) se stropem `max_frac_w` / `max_frac_h` promile pracovni
    /// plochy (0 = bez stropu). Napr. terminal: `request_size(1040, 520,
    /// 850, 0)` = 1040 px, nejvys 85 % sirky.
    ///
    /// Pred prvnim commitem pocka na `CONFIGURE` s vysledkem (okno se pak
    /// ukaze rovnou ve spravne velikosti); pozdeji prijde odpoved jako
    /// obvykla udalost `Resized`. Maximalizovane okno si velikost
    /// zapamatuje na obnoveni.
    pub fn request_size(&mut self, w: u32, h: u32, max_frac_w: u16, max_frac_h: u16) -> Result<()> {
        self.request_size_min(w, h, 0, 0, max_frac_w, max_frac_h)
    }

    /// Jako [`Window::request_size`] s nejmensi velikosti okna.
    pub fn request_size_min(&mut self, w: u32, h: u32, min_w: u32, min_h: u32, max_frac_w: u16, max_frac_h: u16) -> Result<()> {
        self.send(ClientMsg::WindowSize { window: 1, w, h, min_w, min_h, max_frac_w, max_frac_h })?;
        if self.txn > 0 {
            return Ok(());
        }
        // Jeste nic nezobrazeno: pockat na novou velikost (jine udalosti
        // zustanou ve fronte v puvodnim poradi).
        let deadline = Some(libos::time::deadline_after(core::time::Duration::from_millis(WAIT_MS)));
        let mut later: Vec<Event> = core::mem::take(&mut self.queue);
        let r = loop {
            match self.next(deadline) {
                Ok(Event::Resized { w, h }) => {
                    later.push(Event::Resized { w, h });
                    break Ok(());
                }
                Ok(e) => later.push(e),
                // Kompozitor velikost nezmenil (uz sedi): nic necekat.
                Err(Error::TimedOut) => break Ok(()),
                Err(e) => break Err(e),
            }
        };
        later.append(&mut self.queue);
        self.queue = later;
        r
    }
}
