//! Protokol poskytovatele nastroju (sluzba systemu <-> agentd).
//!
//! Podobne jako server MCP: sluzba na svem kanalu posle `Register` se
//! seznamem nastroju (strojove citelny popis se schematem), `agentd` pak
//! posila `Call` a ceka `Result`. Volani nese v handlech **capability
//! relace** (nove spojeni FS se stejnou identitou a pripojenymi cestami),
//! takze sluzba operaci provede s pravy relace, ne s pravy `agentd`.
//!
//! Sluzba nesmi verit vstupu (`input` vytvoril model): `agentd` ho sice
//! overil proti schematu, ale obsah (cesty, texty) je od modelu.

use alloc::string::String;
use alloc::vec::Vec;

use wire::{Reader, Writer};

use crate::tool::ToolSpec;
use crate::{Error, Result};

/// Nejvic nastroju od jednoho poskytovatele.
pub const MAX_TOOLS: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub enum ProviderMsg {
    Register { tools: Vec<ToolSpec> },
    /// Handly: capability relace (muze chybet).
    Call { id: u32, tool: String, input: String },
    Result { id: u32, is_error: bool, content: String },
}

impl ProviderMsg {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            ProviderMsg::Register { tools } => {
                w.u8(1).u32(tools.len() as u32);
                for t in tools {
                    t.put(&mut w);
                }
            }
            ProviderMsg::Call { id, tool, input } => {
                w.u8(2).u32(*id).str(tool).u32(input.len() as u32).bytes(input.as_bytes());
            }
            ProviderMsg::Result { id, is_error, content } => {
                w.u8(3).u32(*id).u8(*is_error as u8).u32(content.len() as u32).bytes(content.as_bytes());
            }
        }
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let long = |r: &mut Reader| -> Result<String> {
            let n = r.u32()? as usize;
            Ok(core::str::from_utf8(r.take(n)?).map_err(|_| Error::InvalidArgs)?.into())
        };
        Ok(match r.u8()? {
            1 => {
                let n = r.u32()? as usize;
                if n > MAX_TOOLS {
                    return Err(Error::InvalidArgs);
                }
                ProviderMsg::Register { tools: (0..n).map(|_| ToolSpec::get(&mut r)).collect::<Result<_>>()? }
            }
            2 => ProviderMsg::Call { id: r.u32()?, tool: r.str()?, input: long(&mut r)? },
            3 => ProviderMsg::Result { id: r.u32()?, is_error: r.u8()? != 0, content: long(&mut r)? },
            _ => return Err(Error::InvalidArgs),
        })
    }
}
