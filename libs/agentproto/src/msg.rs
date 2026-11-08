//! Zpravy konverzace nezavisle na poskytovateli modelu.
//!
//! Konverzace je seznam [`Message`]: role a bloky obsahu. Blok, kteremu
//! vrstva nerozumi (myslenkovy blok Claude s podpisem, cokoli noveho od
//! poskytovatele), je [`Block::Opaque`] a posila se zpet **beze zmeny** -
//! poskytovatel ho muze vyzadovat (podpis myslenek, vazba na konverzaci).

use alloc::string::String;
use alloc::vec::Vec;

use crate::json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Text(String),
    /// Model chce zavolat nastroj (vstup je JSON objekt).
    ToolUse { id: String, name: String, input: Value },
    /// Vysledek nastroje (posila uzivatelska strana).
    ToolResult { id: String, content: String, is_error: bool },
    /// Blok poskytovatele, kteremu vrstva nerozumi (puvodni JSON).
    Opaque(Value),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub role: Role,
    pub content: Vec<Block>,
}

impl Message {
    pub fn user_text(text: &str) -> Self {
        Self { role: Role::User, content: alloc::vec![Block::Text(String::from(text))] }
    }

    pub fn assistant_text(text: &str) -> Self {
        Self { role: Role::Assistant, content: alloc::vec![Block::Text(String::from(text))] }
    }

    /// Spojeny text vsech textovych bloku.
    pub fn text(&self) -> String {
        let mut s = String::new();
        for b in &self.content {
            if let Block::Text(t) = b {
                s.push_str(t);
            }
        }
        s
    }

    /// Volani nastroju ve zprave (id, jmeno, vstup).
    pub fn tool_uses(&self) -> impl Iterator<Item = (&str, &str, &Value)> {
        self.content.iter().filter_map(|b| match b {
            Block::ToolUse { id, name, input } => Some((id.as_str(), name.as_str(), input)),
            _ => None,
        })
    }

    /// Priblizna velikost obsahu v bajtech (odhad tokenu, audit).
    pub fn approx_bytes(&self) -> usize {
        self.content
            .iter()
            .map(|b| match b {
                Block::Text(t) => t.len(),
                Block::ToolUse { name, input, .. } => name.len() + input.to_json().len(),
                Block::ToolResult { content, .. } => content.len() + 16,
                Block::Opaque(v) => v.to_json().len(),
            })
            .sum()
    }
}

/// Proc model skoncil.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    EndTurn,
    ToolUse,
    MaxTokens,
    /// Poskytovatel odmitl (bezpecnostni klasifikator); nic dalsiho nevolat.
    Refusal,
    Other(String),
}

impl Stop {
    pub fn parse(s: &str) -> Self {
        match s {
            "end_turn" | "stop_sequence" => Stop::EndTurn,
            "tool_use" => Stop::ToolUse,
            "max_tokens" => Stop::MaxTokens,
            "refusal" => Stop::Refusal,
            other => Stop::Other(String::from(other)),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Stop::EndTurn => "end_turn",
            Stop::ToolUse => "tool_use",
            Stop::MaxTokens => "max_tokens",
            Stop::Refusal => "refusal",
            Stop::Other(s) => s,
        }
    }
}

/// Spotreba tokenu jednoho volani (nebo soucet).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_write: u64,
    pub cache_read: u64,
}

impl Usage {
    pub fn add(&mut self, o: &Usage) {
        self.input = self.input.saturating_add(o.input);
        self.output = self.output.saturating_add(o.output);
        self.cache_write = self.cache_write.saturating_add(o.cache_write);
        self.cache_read = self.cache_read.saturating_add(o.cache_read);
    }

    pub fn put(&self, w: &mut wire::Writer) {
        w.u64(self.input).u64(self.output).u64(self.cache_write).u64(self.cache_read);
    }

    pub fn get(r: &mut wire::Reader) -> crate::Result<Self> {
        Ok(Self { input: r.u64()?, output: r.u64()?, cache_write: r.u64()?, cache_read: r.u64()? })
    }
}
