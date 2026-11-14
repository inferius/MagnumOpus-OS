//! Kurzor okna, zamek ukazatele a cela obrazovka na zadost aplikace
//! (zpravy `SET_CURSOR`, `POINTER_LOCK`, `WINDOW_STATE`; guiproto 5).
//! Pouzivaji je okna, ktera kresli sama (SDL pres `mosdl`).

use guiproto::cursor::Cursor;
use guiproto::window::ClientMsg;
use libos::Result;

use super::Window;

impl Window {
    /// Kurzor nad obsahem okna (systemovy tvar, skryty, vlastni obrazek).
    pub fn set_cursor(&mut self, cursor: Cursor) -> Result<()> {
        self.send(ClientMsg::SetCursor { window: 1, cursor })
    }

    /// Zadost o celou obrazovku nebo navrat. Vstup jen v reakci na fyzicky
    /// stisk (posle cislo gesta posledni udalosti); odpoved je `Resized`
    /// a stav v `CONFIGURE`.
    pub fn set_fullscreen(&mut self, on: bool) -> Result<()> {
        let gesture = self.gesture;
        self.send(ClientMsg::WindowState { window: 1, fullscreen: on, gesture })
    }

    /// Zadost o zamek ukazatele (relativni pohyb, skryty kurzor) nebo jeho
    /// zruseni. Vysledek prijde jako [`super::Event::PointerLock`].
    pub fn pointer_lock(&mut self, lock: bool) -> Result<()> {
        let gesture = self.gesture;
        self.send(ClientMsg::PointerLock { window: 1, lock, gesture })
    }
}
