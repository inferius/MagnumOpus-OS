//! Udalosti proudu odpovedi a jejich skladani do zpravy.
//!
//! Poskytovatel posila udalosti prubezne (text po castech, zacatek volani
//! nastroje, cast JSON vstupu, ...). [`Accumulator`] z nich slozi celou
//! zpravu - stejne pro proud SSE Claude i pro falesny model, takze
//! `agentd` ma jednu cestu.

use alloc::string::String;
use alloc::vec::Vec;

use crate::json::{self, Value};
use crate::msg::{Block, Message, Role, Stop, Usage};

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    MessageStart { usage: Usage },
    /// Novy textovy blok na pozici `index`.
    TextStart { index: u32 },
    TextDelta { index: u32, text: String },
    ToolUseStart { index: u32, id: String, name: String },
    /// Cast JSON vstupu nastroje (spojuje se az do `BlockStop`).
    ToolInputDelta { index: u32, json: String },
    /// Blok, kteremu vrstva nerozumi (cely, i s pripadnymi zmenami).
    OpaqueStart { index: u32, block: Value },
    /// Doplneni nezname bloku (napr. podpis myslenek): klic a cast textu.
    OpaqueDelta { index: u32, key: String, text: String },
    BlockStop { index: u32 },
    MessageDelta { stop: Option<Stop>, usage: Usage },
    MessageStop,
}

enum Partial {
    Text(String),
    Tool { id: String, name: String, json: String },
    Opaque(Value),
}

/// Skladani zpravy z udalosti.
#[derive(Default)]
pub struct Accumulator {
    blocks: Vec<(u32, Partial)>,
    pub usage: Usage,
    pub stop: Option<Stop>,
    /// Neplatny JSON vstupu nastroje (id, popis) - agentd ho vrati modelu
    /// jako chybu nastroje, nespousti ho.
    pub bad_inputs: Vec<(String, String)>,
}

impl Accumulator {
    pub fn new() -> Self {
        Self::default()
    }

    fn slot(&mut self, index: u32) -> Option<&mut Partial> {
        self.blocks.iter_mut().find(|(i, _)| *i == index).map(|(_, p)| p)
    }

    pub fn push(&mut self, ev: &Event) {
        match ev {
            Event::MessageStart { usage } => self.usage.add(usage),
            Event::TextStart { index } => self.blocks.push((*index, Partial::Text(String::new()))),
            Event::TextDelta { index, text } => match self.slot(*index) {
                Some(Partial::Text(t)) => t.push_str(text),
                Some(_) => {}
                None => self.blocks.push((*index, Partial::Text(text.clone()))),
            },
            Event::ToolUseStart { index, id, name } => {
                self.blocks.push((*index, Partial::Tool { id: id.clone(), name: name.clone(), json: String::new() }))
            }
            Event::ToolInputDelta { index, json } => {
                if let Some(Partial::Tool { json: j, .. }) = self.slot(*index) {
                    j.push_str(json);
                }
            }
            Event::OpaqueStart { index, block } => self.blocks.push((*index, Partial::Opaque(block.clone()))),
            Event::OpaqueDelta { index, key, text } => {
                if let Some(Partial::Opaque(v)) = self.slot(*index) {
                    let mut s = String::from(v.str_of(key));
                    s.push_str(text);
                    v.set(key, Value::Str(s));
                }
            }
            Event::BlockStop { .. } => {}
            Event::MessageDelta { stop, usage } => {
                if stop.is_some() {
                    self.stop = stop.clone();
                }
                // Claude posila v message_delta souhrnny vystup; vstup uz byl.
                self.usage.output = self.usage.output.max(usage.output);
                self.usage.input = self.usage.input.max(usage.input);
                self.usage.cache_write = self.usage.cache_write.max(usage.cache_write);
                self.usage.cache_read = self.usage.cache_read.max(usage.cache_read);
            }
            Event::MessageStop => {}
        }
    }

    /// Hotova zprava asistenta.
    pub fn finish(mut self) -> (Message, Option<Stop>, Usage, Vec<(String, String)>) {
        self.blocks.sort_by_key(|(i, _)| *i);
        let mut content = Vec::new();
        for (_, p) in self.blocks {
            match p {
                Partial::Text(t) => content.push(Block::Text(t)),
                Partial::Tool { id, name, json } => {
                    let text = if json.trim().is_empty() { "{}" } else { json.as_str() };
                    match json::parse(text) {
                        Ok(v @ Value::Object(_)) => content.push(Block::ToolUse { id, name, input: v }),
                        Ok(_) => {
                            self.bad_inputs.push((id.clone(), String::from("vstup nastroje neni objekt")));
                            content.push(Block::ToolUse { id, name, input: Value::object() });
                        }
                        Err(e) => {
                            self.bad_inputs.push((id.clone(), alloc::format!("{e}")));
                            content.push(Block::ToolUse { id, name, input: Value::object() });
                        }
                    }
                }
                Partial::Opaque(v) => content.push(Block::Opaque(v)),
            }
        }
        (Message { role: Role::Assistant, content }, self.stop, self.usage, self.bad_inputs)
    }
}

/// Udalosti, ktere by poslal proud pro hotovou zpravu (poskytovatel bez
/// streamovani je tak pro `agentd` stejny jako se streamovanim).
pub fn events_of(msg: &Message, stop: &Stop, usage: &Usage) -> Vec<Event> {
    let mut out = alloc::vec![Event::MessageStart { usage: Usage { output: 0, ..*usage } }];
    for (i, b) in msg.content.iter().enumerate() {
        let index = i as u32;
        match b {
            Block::Text(t) => {
                out.push(Event::TextStart { index });
                out.push(Event::TextDelta { index, text: t.clone() });
            }
            Block::ToolUse { id, name, input } => {
                out.push(Event::ToolUseStart { index, id: id.clone(), name: name.clone() });
                out.push(Event::ToolInputDelta { index, json: input.to_json() });
            }
            Block::ToolResult { .. } => continue,
            Block::Opaque(v) => out.push(Event::OpaqueStart { index, block: v.clone() }),
        }
        out.push(Event::BlockStop { index });
    }
    out.push(Event::MessageDelta { stop: Some(stop.clone()), usage: *usage });
    out.push(Event::MessageStop);
    out
}
