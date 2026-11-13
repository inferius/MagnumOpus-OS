//! installd jako producent prehledu uloh (`taskd`): instalace je uloha
//! "Instalace MagnumOpus" s prubehem a vysledkem, takze je videt ve
//! Spravci uloh a v terminalu i po zavreni okna instalatoru.
//!
//! Kanal producenta posle init zpravou `tasks` na bootstrap kanalu (stejne
//! jako fsd). Bez nej (taskd nebezi) installd ulohu nehlasi; instalace tim
//! neni dotcena. Zruseni instalace taskd nenabizi (zapis disku se
//! neprerusuje).

use alloc::string::String;
use alloc::vec::Vec;
use core::cell::Cell;
use core::time::Duration;

use libos::{Channel, println, time};
use taskproto::{Origin, Produce, State};

/// Jak dlouho nejvys cekat na REGISTERED (instalace na nej neceka dal).
const REGISTER_WAIT: Duration = Duration::from_secs(2);

#[derive(Default)]
pub struct Tasks {
    pub chan: Option<Channel>,
    /// Cislo ulohy po REGISTERED (Cell: hlaseni jde i pres sdileny odkaz
    /// behem instalace, kdy je sluzba pujcena).
    id: Cell<Option<u64>>,
}

impl Tasks {
    /// Chyba zapisu (taskd skoncil) jen ztlumi hlaseni; instalace bezi dal.
    fn send(&self, m: &Produce) {
        if let Some(c) = &self.chan {
            let _ = c.write(&m.encode(), Vec::new());
        }
    }

    /// Zacatek instalace na disk `model`.
    pub fn start(&self, model: &str) {
        self.id.set(None);
        self.send(&Produce::Register {
            req: 1,
            parent: 0,
            name: alloc::format!("Instalace MagnumOpus na {model}"),
            origin: Origin::Service,
            who: String::from("installd"),
            agent: false,
            job: None,
            attach: None,
            actions: 0,
        });
        let Some(c) = &self.chan else { return };
        let end = time::deadline_after(REGISTER_WAIT);
        while let Ok(m) = c.read(Some(end)) {
            if let Ok((1, id)) = taskproto::decode_registered(&m.data) {
                self.id.set(Some(id));
                println!("installd: instalace je uloha {id} v prehledu uloh");
                return;
            }
        }
        println!("installd: prehled uloh neodpovedel, instalace bez ulohy");
    }

    /// Krok `step` z `total`.
    pub fn progress(&self, step: u32, total: u32) {
        if let Some(id) = self.id.get() {
            let pct = (step * 100 / total.max(1)).min(99) as u8;
            self.send(&Produce::Update { id, state: State::Running, progress: Some(pct), eta_ms: None });
        }
    }

    pub fn finish(&self, r: Result<&str, (i32, &str)>) {
        let Some(id) = self.id.take() else { return };
        let state = match r {
            Ok(text) => State::Done(String::from(text)),
            Err((code, message)) => State::Failed { code, message: String::from(message) },
        };
        self.send(&Produce::Finish { id, state });
    }
}
