//! Spravce souboru v GUI (`user/gui-files`): model a rozhrani nad
//! knihovnou `libs/ui`, testovatelne na hostiteli (`cargo test -p
//! filesview`).
//!
//! - `app` - stav a zpracovani zprav, `view` - rozhrani;
//! - `listing` - vypis slozky (razeni, filtr, predpripravene radky pro
//!   virtualizovanou tabulku: 10 000 polozek stoji jen viditelne radky);
//! - `nav` - historie a drobeckova navigace; `preview` - nahled textu
//!   a BMP; `format` - texty hodnot a tri stavy hodnot z FS;
//! - `props` + `props_view` - panel Vlastnosti (karty Obecne
//!   a Opravneni);
//! - `backend` - co aplikace potrebuje od systemu (v OS `FsClient`
//!   a kanal k `files-copy`, v testech FS v pameti).
//!
//! Kopirovani a presun dela pomocny proces `files-copy` (protokol
//! `fileops::proto`): uloha prezije zavreni okna a je v prehledu uloh.

#![no_std]

extern crate alloc;

pub mod app;
pub mod backend;
pub mod clipfmt;
pub mod format;
pub mod listing;
pub mod nav;
pub mod preview;
pub mod props;
mod props_view;
pub mod view;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod props_tests;

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

    /// Kanal 0 = `files-copy`, 1 = odpovedi sluzby "otevri cestu".
    fn external_from(&mut self, index: usize, data: &[u8]) -> bool {
        match index {
            0 => self.copier_msg(data),
            1 => self.open_reply(data),
            _ => false,
        }
    }

    fn key(&self, code: u16, ch: Option<char>, mods: u16) -> Option<Msg> {
        if self.props.is_some() {
            // Panel Vlastnosti: Escape zavre, ostatni klavesy ne (radky
            // vypisu nejsou videt).
            return (code == inputproto::code::KEY_ESC).then_some(Msg::Props(props::PropsMsg::Close));
        }
        key_msg(self.edit.is_some(), self.mode, code, ch, mods)
    }

    fn effects(&mut self) -> alloc::vec::Vec<ui::Effect> {
        core::mem::take(&mut self.effects)
    }

    fn pasted(&mut self, format: &str, data: &[u8]) -> bool {
        FileManager::pasted(self, format, data)
    }

    fn menu(&self) -> alloc::vec::Vec<guiproto::menu::MenuItem> {
        app_menu(!self.sel.is_empty())
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
            "files.properties" => Msg::Properties,
            "files.download" => Msg::Download,
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

/// Klavesy aplikace (`App::key`: jen ty, ktere prvek s fokusem
/// nespotreboval; tabulka si necha sipky, Enter, psani, Ctrl+A). Znak
/// `ch` je podle rozlozeni, takze Ctrl+C je `c` s Ctrl i na jine
/// klavesnici.
pub fn key_msg(editing: bool, mode: app::Mode, code: u16, ch: Option<char>, mods: u16) -> Option<Msg> {
    use inputproto::code::*;
    use inputproto::mods::{ALT, CTRL, SHIFT};
    let (ctrl, alt, shift) = (mods & CTRL != 0, mods & ALT != 0, mods & SHIFT != 0);
    let letter = ch.map(|c| c.to_ascii_lowercase());
    if editing {
        // Pole jmena si nechava editaci; Escape zrusi.
        return (code == KEY_ESC).then_some(Msg::NameCancel);
    }
    let trash = mode == app::Mode::Trash;
    Some(match (code, ctrl, alt) {
        (KEY_LEFT, false, true) => Msg::Back,
        (KEY_RIGHT, false, true) => Msg::Forward,
        (KEY_UP, false, true) | (KEY_BACKSPACE, false, false) => Msg::Up,
        (KEY_F5, false, false) => Msg::Refresh,
        (KEY_F2, false, false) if !trash => Msg::Rename,
        (KEY_DELETE, false, false) if trash => Msg::TrashPurge,
        (KEY_DELETE, false, false) => Msg::Trash,
        (KEY_ENTER, false, true) if !trash => Msg::Properties,
        _ if ctrl && !alt => match letter? {
            'c' if !trash => Msg::Copy,
            'x' if !trash => Msg::Cut,
            'v' if !trash => Msg::Paste,
            'n' if shift && !trash => Msg::NewFolder,
            _ => return None,
        },
        _ => return None,
    })
}

/// Menu aplikace (paleta prikazu Ctrl+K, menu v titulku, kontextova
/// nabidka). Zkratky obsluhuje `key_msg`.
pub fn app_menu(selected: bool) -> alloc::vec::Vec<guiproto::menu::MenuItem> {
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
            item("Stáhnout z adresy…", "files.download", "", true),
            item("Přejmenovat", "files.rename", "F2", selected),
            item("Do koše", "files.trash", "Delete", selected),
            item("Kopírovat", "files.copy", "Ctrl+C", selected),
            item("Vyjmout", "files.cut", "Ctrl+X", selected),
            // Vlozit i ze schranky jineho okna (stav schranky menu nezna).
            item("Vložit", "files.paste", "Ctrl+V", true),
            item("Zpět", "files.back", "Alt+Left", true),
            item("Nahoru", "files.up", "Backspace", true),
            item("Znovu načíst", "files.refresh", "F5", true),
            // Bez vyberu vlastnosti aktualni slozky.
            item("Vlastnosti", "files.properties", "Alt+Enter", true),
        ],
        ..MenuItem::default()
    }]
}
