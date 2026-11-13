//! Spravce souboru v GUI (`user/gui-files`): model a rozhrani nad
//! knihovnou `libs/ui`, testovatelne na hostiteli (`cargo test -p
//! filesview`).
//!
//! - `app` - stav a zpracovani zprav, `view` - rozhrani;
//! - `listing` - vypis slozky (razeni, filtr, predpripravene radky pro
//!   virtualizovanou tabulku: 10 000 polozek stoji jen viditelne radky);
//! - `nav` - historie a drobeckova navigace; `preview` - nahled textu
//!   a BMP; `format` - texty hodnot a tri stavy hodnot z FS;
//! - `backend` - co aplikace potrebuje od systemu (v OS `FsClient`
//!   a kanal k `files-copy`, v testech FS v pameti).
//!
//! Kopirovani a presun dela pomocny proces `files-copy` (protokol
//! `fileops::proto`): uloha prezije zavreni okna a je v prehledu uloh.

#![no_std]

extern crate alloc;

pub mod app;
pub mod backend;
pub mod format;
pub mod listing;
pub mod nav;
pub mod preview;
pub mod view;

#[cfg(test)]
mod tests;

use alloc::string::String;

pub use abi::{Error, Result};
pub use app::{FileManager, Msg};
pub use backend::{Backend, TestBackend};

/// Kratky cesky text chyby pro rozhrani.
pub fn error_text(e: Error) -> &'static str {
    match e {
        Error::NotFound => "neexistuje",
        Error::AccessDenied => "chybí oprávnění",
        Error::AlreadyExists => "už existuje",
        Error::NotEmpty => "složka není prázdná",
        Error::NotSupported => "tady to nejde",
        Error::InvalidArgs => "neplatné jméno nebo cíl",
        Error::NoSpace => "došlo místo",
        Error::IoError => "chyba disku",
        _ => e.name(),
    }
}

impl<B: Backend> ui::App for FileManager<B> {
    type Msg = Msg;

    fn title(&self) -> String {
        self.title_text()
    }

    fn view(&self, cx: &ui::Context) -> ui::Node<Msg> {
        self.view_root(cx)
    }

    fn update(&mut self, m: Msg) {
        self.handle(m);
    }

    fn external(&mut self, data: &[u8]) -> bool {
        self.copier_msg(data)
    }

    fn menu(&self) -> alloc::vec::Vec<guiproto::menu::MenuItem> {
        app_menu(!self.sel.is_empty(), self.clip.is_some())
    }

    fn action(&mut self, id: &str, args: &[guiproto::action::Arg]) -> core::result::Result<Option<guiproto::action::Value>, String> {
        let m = match id {
            "files.new-folder" => Msg::NewFolder,
            "files.rename" => Msg::Rename,
            "files.trash" => Msg::Trash,
            "files.copy" => Msg::Copy,
            "files.cut" => Msg::Cut,
            "files.paste" => Msg::Paste,
            "files.back" => Msg::Back,
            "files.up" => Msg::Up,
            "files.refresh" => Msg::Refresh,
            "files.show" => {
                let path = args.iter().find(|a| a.0 == "path").and_then(|a| match &a.1 {
                    guiproto::action::Value::Path(p) | guiproto::action::Value::Text(p) => Some(p.clone()),
                    _ => None,
                });
                let Some(p) = path else { return Err(String::from("chybí cesta")) };
                return self.show_path(&p).map(|_| None);
            }
            _ => return Err(String::from("neznámá akce")),
        };
        self.handle(m);
        Ok(None)
    }
}

/// Menu aplikace (paleta prikazu Ctrl+K, menu v titulku). Zkratky se
/// zatim jen zobrazuji; klavesy aplikace dodava `libs/ui` (pozadavek 6c).
pub fn app_menu(selected: bool, clip: bool) -> alloc::vec::Vec<guiproto::menu::MenuItem> {
    use guiproto::menu::MenuItem;
    let item = |label: &str, action: &str, shortcut: &str, enabled: bool| MenuItem {
        label: String::from(label),
        action: String::from(action),
        shortcut: String::from(shortcut),
        disabled: if enabled { String::new() } else { String::from("Nic není vybráno") },
        ..MenuItem::default()
    };
    alloc::vec![MenuItem {
        label: String::from("Soubor"),
        children: alloc::vec![
            item("Nová složka", "files.new-folder", "Ctrl+Shift+N", true),
            item("Přejmenovat", "files.rename", "F2", selected),
            item("Do koše", "files.trash", "Delete", selected),
            item("Kopírovat", "files.copy", "Ctrl+C", selected),
            item("Vyjmout", "files.cut", "Ctrl+X", selected),
            MenuItem { disabled: if clip { String::new() } else { String::from("Schránka je prázdná") }, ..item("Vložit", "files.paste", "Ctrl+V", true) },
            item("Zpět", "files.back", "Alt+Left", true),
            item("Nahoru", "files.up", "Backspace", true),
            item("Znovu načíst", "files.refresh", "F5", true),
        ],
        ..MenuItem::default()
    }]
}
