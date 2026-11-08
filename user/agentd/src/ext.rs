//! Nastroje sluzeb systemu (protokol `agentproto::registry`).
//!
//! Poskytovatel je kanal: nejdriv posle `Register` se seznamem nastroju,
//! pak odpovida na `Call`. Volani nese v handlu capability relace (nove
//! spojeni FS se stejnou identitou), aby sluzba jednala s pravy relace.
//! Globalni poskytovatele (registrovane pres branu) sdili relace; kazdy
//! ma vlastni zamek (jedno volani naraz).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use agentproto::registry::ProviderMsg;
use agentproto::tool::ToolSpec;
use libos::{Channel, Error, Handle, Result, time};

/// Jak dlouho ceka `agentd` na registraci a na vysledek volani.
pub const REGISTER_WAIT: Duration = Duration::from_secs(5);
pub const CALL_WAIT: Duration = Duration::from_secs(30);

pub struct Ext {
    pub name: String,
    pub tools: Vec<ToolSpec>,
    chan: Channel,
    next: u32,
}

impl Ext {
    /// Precte registraci z noveho kanalu poskytovatele.
    pub fn register(name: &str, chan: Channel) -> Result<Self> {
        let m = chan.read(Some(time::deadline_after(REGISTER_WAIT)))?;
        match ProviderMsg::decode(&m.data)? {
            ProviderMsg::Register { tools } => Ok(Self { name: String::from(name), tools, chan, next: 1 }),
            _ => Err(Error::InvalidArgs),
        }
    }

    /// Zavola nastroj; vraci (obsah, chyba?).
    pub fn call(&mut self, tool: &str, input: &str, cap: Option<Handle>) -> Result<(String, bool)> {
        let id = self.next;
        self.next = self.next.wrapping_add(1);
        let msg = ProviderMsg::Call { id, tool: String::from(tool), input: String::from(input) };
        self.chan.write(&msg.encode(), cap.into_iter().collect())?;
        let deadline = time::deadline_after(CALL_WAIT);
        loop {
            let m = self.chan.read(Some(deadline))?;
            match ProviderMsg::decode(&m.data) {
                Ok(ProviderMsg::Result { id: rid, is_error, content }) if rid == id => return Ok((content, is_error)),
                // Pozdni odpoved na volani, ktere uz vyprselo.
                Ok(ProviderMsg::Result { .. }) => continue,
                _ => {
                    libos::println!("agentd: poskytovatel {} poslal neplatnou odpoved", self.name);
                    return Err(Error::Corrupted);
                }
            }
        }
    }
}

/// Text chyby nastroje sluzby pro model.
pub fn failed(name: &str, e: Error) -> String {
    format!("nastroj {name} selhal: {}", e.name())
}
