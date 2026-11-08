//! Rozhrani poskytovatele modelu.
//!
//! `agentd` zna jen tento trait; Claude API, falesny model pro testy
//! a pozdeji lokalni model jsou jeho implementace. Klienti (shell, GUI)
//! poskytovatele nevidi vubec - novy poskytovatel nevyzaduje zmenu klientu.

use alloc::string::String;
use alloc::vec::Vec;

use crate::msg::{Message, Stop, Usage};
use crate::price::Price;
use crate::stream::Event;
use crate::tool::ToolSpec;

/// Jeden pozadavek na model.
#[derive(Debug, Clone)]
pub struct Request {
    /// Systemovy pokyn relace (role, pravidla, nedůveryhodna data).
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub max_tokens: u32,
}

impl Request {
    /// Priblizna velikost pozadavku v bajtech (odhad ceny, audit).
    pub fn approx_bytes(&self) -> usize {
        self.system.len()
            + self.messages.iter().map(Message::approx_bytes).sum::<usize>()
            + self.tools.iter().map(|t| t.name.len() + t.description.len() + t.schema.to_json().len()).sum::<usize>()
    }
}

/// Odpoved modelu.
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub message: Message,
    pub stop: Stop,
    pub usage: Usage,
    /// Neplatne vstupy nastroju (id volani, popis chyby).
    pub bad_inputs: Vec<(String, String)>,
    /// Kolik bajtu odeslano poskytovateli (audit "co odchazi do cloudu").
    pub sent_bytes: u64,
}

/// Chyba poskytovatele.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    /// Chybi API klic (uzivatel ho jeste nezadal).
    NoKey,
    /// Klic odmitnut (HTTP 401/403).
    Auth(String),
    /// Prilis mnoho pozadavku nebo pretizeni (HTTP 429/529) - zkusit pozdeji.
    Busy(String),
    /// Neplatny pozadavek (HTTP 400 a pod.).
    Request(String),
    /// Chyba site nebo TLS.
    Transport(String),
    /// Odpoved, ktere kodek nerozumi.
    Protocol(String),
    /// Falesny poskytovatel: skript dosel.
    ScriptEnd,
}

impl ProviderError {
    pub fn code(&self) -> abi::Error {
        match self {
            ProviderError::NoKey => abi::Error::NotFound,
            ProviderError::Auth(_) => abi::Error::AccessDenied,
            ProviderError::Busy(_) => abi::Error::ShouldWait,
            ProviderError::Request(_) => abi::Error::InvalidArgs,
            ProviderError::Transport(_) => abi::Error::IoError,
            ProviderError::Protocol(_) => abi::Error::Corrupted,
            ProviderError::ScriptEnd => abi::Error::OutOfRange,
        }
    }
}

impl core::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ProviderError::NoKey => f.write_str("chybi API klic (ai --klic soubor)"),
            ProviderError::Auth(m) => write!(f, "klic odmitnut: {m}"),
            ProviderError::Busy(m) => write!(f, "poskytovatel je pretizen: {m}"),
            ProviderError::Request(m) => write!(f, "neplatny pozadavek: {m}"),
            ProviderError::Transport(m) => write!(f, "spojeni s poskytovatelem: {m}"),
            ProviderError::Protocol(m) => write!(f, "odpoved poskytovatele: {m}"),
            ProviderError::ScriptEnd => f.write_str("falesny model: skript dosel"),
        }
    }
}

pub trait Provider {
    /// Jmeno poskytovatele (`claude`, `fake`, ...).
    fn name(&self) -> &str;
    /// Model.
    fn model(&self) -> &str;
    /// Cenik (mikrodolary za milion tokenu).
    fn price(&self) -> Price;
    /// Jedno volani modelu. Udalosti proudu jdou do `sink` prubezne.
    fn complete(&mut self, req: &Request, sink: &mut dyn FnMut(&Event)) -> Result<Response, ProviderError>;
}
