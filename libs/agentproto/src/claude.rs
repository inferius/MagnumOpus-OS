//! Poskytovatel Claude API (Messages API s tool use).
//!
//! Kodek je ciste funkce (testy na hostiteli): [`encode_request`] sestavi
//! telo `POST /v1/messages`, [`decode_response`] precte odpoved bez
//! streamovani, [`SseParser`] proud SSE a [`decode_error`] chybu. Doprava
//! (TLS + HTTP) je trait [`Transport`] - v systemu ho implementuje `agentd`
//! nad `libs/tls` + `libs/httpc`, v testech podvrh.
//!
//! - `anthropic-version: 2026-10-21` (bez `temperature` / `top_p` / `top_k`).
//! - Myslenkove bloky (`thinking`, `redacted_thinking`) a cokoli
//!   neznameho se vraci v historii **beze zmeny** ([`Block::Opaque`]).
//! - Vysledky nastroju jednoho kroku jsou v jedne zprave `user`.
//! - Jmena nastroju s teckou se posilaji s `__` (API tecku nepovoli).

use alloc::string::String;
use alloc::vec::Vec;

use crate::json::{self, Value};
use crate::msg::{Block, Message, Role, Stop, Usage};
use crate::price::{Price, claude_price};
use crate::provider::{Provider, ProviderError, Request, Response};
use crate::secret::Secret;
use crate::stream::{Accumulator, Event, events_of};
use crate::tool::{from_wire_name, wire_name};

pub const HOST: &str = "api.anthropic.com";
pub const URL: &str = "https://api.anthropic.com/v1/messages";
pub const API_VERSION: &str = "2026-10-21";
pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
/// Nejvetsi telo odpovedi, ktere prijmeme.
pub const MAX_RESPONSE: usize = 4 << 20;

/// Nastaveni pozadavku navic (vychozi pro agenta).
#[derive(Debug, Clone)]
pub struct Options {
    pub model: String,
    /// `low` / `medium` / `high` / `xhigh` / `max` (prazdne = vychozi modelu).
    pub effort: String,
    pub stream: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { model: String::from(DEFAULT_MODEL), effort: String::from("medium"), stream: false }
    }
}

fn block_json(b: &Block) -> Value {
    match b {
        Block::Text(t) => Value::object().with("type", "text".into()).with("text", t.as_str().into()),
        Block::ToolUse { id, name, input } => Value::object()
            .with("type", "tool_use".into())
            .with("id", id.as_str().into())
            .with("name", wire_name(name).into())
            .with("input", input.clone()),
        Block::ToolResult { id, content, is_error } => {
            let mut v = Value::object()
                .with("type", "tool_result".into())
                .with("tool_use_id", id.as_str().into())
                .with("content", content.as_str().into());
            if *is_error {
                v.set("is_error", Value::Bool(true));
            }
            v
        }
        Block::Opaque(v) => v.clone(),
    }
}

/// Telo pozadavku (JSON text).
pub fn encode_request(req: &Request, opts: &Options) -> String {
    let messages: Vec<Value> = req
        .messages
        .iter()
        .map(|m| {
            Value::object()
                .with("role", m.role.as_str().into())
                .with("content", Value::Array(m.content.iter().map(block_json).collect()))
        })
        .collect();
    let mut body = Value::object()
        .with("model", opts.model.as_str().into())
        .with("max_tokens", Value::Int(i64::from(req.max_tokens)));
    if !req.system.is_empty() {
        body.set("system", req.system.as_str().into());
    }
    if !req.tools.is_empty() {
        let tools = req
            .tools
            .iter()
            .map(|t| {
                Value::object()
                    .with("name", t.wire_name().into())
                    .with("description", t.description.as_str().into())
                    .with("input_schema", t.schema.clone())
            })
            .collect();
        body.set("tools", Value::Array(tools));
    }
    body.set("messages", Value::Array(messages));
    if !opts.effort.is_empty() {
        body.set("output_config", Value::object().with("effort", opts.effort.as_str().into()));
    }
    if opts.stream {
        body.set("stream", Value::Bool(true));
    }
    body.to_json()
}

fn usage_of(v: &Value) -> Usage {
    Usage {
        input: v.u64_of("input_tokens"),
        output: v.u64_of("output_tokens"),
        cache_write: v.u64_of("cache_creation_input_tokens"),
        cache_read: v.u64_of("cache_read_input_tokens"),
    }
}

fn parse_block(b: &Value) -> Block {
    match b.str_of("type") {
        "text" => Block::Text(String::from(b.str_of("text"))),
        "tool_use" => Block::ToolUse {
            id: String::from(b.str_of("id")),
            name: from_wire_name(b.str_of("name")),
            input: b.get("input").cloned().unwrap_or_else(Value::object),
        },
        _ => Block::Opaque(b.clone()),
    }
}

/// Odpoved bez streamovani -> zprava, duvod konce, spotreba.
pub fn decode_response(body: &str) -> Result<(Message, Stop, Usage), ProviderError> {
    let v = json::parse(body).map_err(|e| ProviderError::Protocol(alloc::format!("{e}")))?;
    if v.str_of("type") == "error" {
        return Err(error_of(&v, 0));
    }
    let content = v
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::Protocol(String::from("odpoved bez content")))?
        .iter()
        .map(parse_block)
        .collect();
    let stop = Stop::parse(v.str_of("stop_reason"));
    let usage = v.get("usage").map(usage_of).unwrap_or_default();
    Ok((Message { role: Role::Assistant, content }, stop, usage))
}

fn error_of(v: &Value, status: u16) -> ProviderError {
    let e = v.get("error").unwrap_or(v);
    let msg = alloc::format!("{} ({})", e.str_of("message"), e.str_of("type"));
    match (status, e.str_of("type")) {
        (401 | 403, _) | (_, "authentication_error" | "permission_error") => ProviderError::Auth(msg),
        (429 | 529, _) | (_, "rate_limit_error" | "overloaded_error") => ProviderError::Busy(msg),
        (500..=599, _) | (_, "api_error") => ProviderError::Busy(msg),
        _ => ProviderError::Request(msg),
    }
}

/// Chybova odpoved HTTP (stav a telo).
pub fn decode_error(status: u16, body: &str) -> ProviderError {
    match json::parse(body) {
        Ok(v) => error_of(&v, status),
        Err(_) => error_of(&Value::object(), status),
    }
}

/// Cteni proudu SSE: bajty prichazeji po castech, udalosti vychazi cele.
#[derive(Default)]
pub struct SseParser {
    buf: String,
    /// Chyba v proudu (udalost `error`).
    pub error: Option<ProviderError>,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Prida cast proudu; vraci hotove udalosti.
    pub fn feed(&mut self, data: &str) -> Vec<Event> {
        self.buf.push_str(data);
        let mut out = Vec::new();
        // Udalost konci prazdnym radkem.
        while let Some(end) = self.buf.find("\n\n") {
            let raw: String = self.buf.drain(..end + 2).collect();
            let mut payload = String::new();
            for line in raw.lines() {
                if let Some(d) = line.strip_prefix("data:") {
                    payload.push_str(d.trim_start());
                }
            }
            if payload.is_empty() {
                continue;
            }
            match json::parse(&payload) {
                Ok(v) => self.event(&v, &mut out),
                Err(e) => self.error = Some(ProviderError::Protocol(alloc::format!("SSE: {e}"))),
            }
        }
        out
    }

    fn event(&mut self, v: &Value, out: &mut Vec<Event>) {
        let index = v.u64_of("index") as u32;
        match v.str_of("type") {
            "message_start" => {
                let usage = v.get("message").and_then(|m| m.get("usage")).map(usage_of).unwrap_or_default();
                out.push(Event::MessageStart { usage });
            }
            "content_block_start" => {
                let b = v.get("content_block").cloned().unwrap_or_else(Value::object);
                match b.str_of("type") {
                    "text" => {
                        out.push(Event::TextStart { index });
                        let t = b.str_of("text");
                        if !t.is_empty() {
                            out.push(Event::TextDelta { index, text: String::from(t) });
                        }
                    }
                    "tool_use" => out.push(Event::ToolUseStart {
                        index,
                        id: String::from(b.str_of("id")),
                        name: from_wire_name(b.str_of("name")),
                    }),
                    _ => out.push(Event::OpaqueStart { index, block: b }),
                }
            }
            "content_block_delta" => {
                let d = v.get("delta").cloned().unwrap_or_else(Value::object);
                match d.str_of("type") {
                    "text_delta" => out.push(Event::TextDelta { index, text: String::from(d.str_of("text")) }),
                    "input_json_delta" => out.push(Event::ToolInputDelta { index, json: String::from(d.str_of("partial_json")) }),
                    "thinking_delta" => {
                        out.push(Event::OpaqueDelta { index, key: String::from("thinking"), text: String::from(d.str_of("thinking")) })
                    }
                    "signature_delta" => {
                        out.push(Event::OpaqueDelta { index, key: String::from("signature"), text: String::from(d.str_of("signature")) })
                    }
                    _ => {}
                }
            }
            "content_block_stop" => out.push(Event::BlockStop { index }),
            "message_delta" => {
                let stop = v.get("delta").and_then(|d| d.get("stop_reason")).and_then(Value::as_str).map(Stop::parse);
                let usage = v.get("usage").map(usage_of).unwrap_or_default();
                out.push(Event::MessageDelta { stop, usage });
            }
            "message_stop" => out.push(Event::MessageStop),
            "error" => self.error = Some(error_of(v, 0)),
            _ => {} // ping a nove druhy udalosti
        }
    }
}

/// Doprava: jeden pozadavek HTTPS POST na [`URL`].
pub trait Transport {
    /// Vraci stav HTTP a telo odpovedi.
    fn post(&mut self, url: &str, headers: &[(&str, &str)], body: &[u8]) -> Result<(u16, Vec<u8>), ProviderError>;
}

/// Poskytovatel Claude nad dopravou `T`.
pub struct Claude<T: Transport> {
    transport: T,
    key: Secret,
    opts: Options,
}

impl<T: Transport> Claude<T> {
    pub fn new(transport: T, key: Secret, opts: Options) -> Self {
        Self { transport, key, opts }
    }
}

impl<T: Transport> Provider for Claude<T> {
    fn name(&self) -> &str {
        "claude"
    }

    fn model(&self) -> &str {
        &self.opts.model
    }

    fn price(&self) -> Price {
        claude_price(&self.opts.model)
    }

    fn complete(&mut self, req: &Request, sink: &mut dyn FnMut(&Event)) -> Result<Response, ProviderError> {
        let key = core::str::from_utf8(self.key.expose()).map_err(|_| ProviderError::NoKey)?;
        if key.is_empty() {
            return Err(ProviderError::NoKey);
        }
        let body = encode_request(req, &self.opts);
        let headers = [
            ("content-type", "application/json"),
            ("x-api-key", key),
            ("anthropic-version", API_VERSION),
            ("accept", if self.opts.stream { "text/event-stream" } else { "application/json" }),
        ];
        let sent = body.len() as u64;
        let (status, resp) = self.transport.post(URL, &headers, body.as_bytes())?;
        let text = core::str::from_utf8(&resp).map_err(|_| ProviderError::Protocol(String::from("odpoved neni UTF-8")))?;
        if status != 200 {
            return Err(decode_error(status, text));
        }
        let (message, stop, usage, bad) = if self.opts.stream {
            let mut p = SseParser::new();
            let mut acc = Accumulator::new();
            for ev in p.feed(text) {
                sink(&ev);
                acc.push(&ev);
            }
            if let Some(e) = p.error {
                return Err(e);
            }
            let (m, stop, usage, bad) = acc.finish();
            (m, stop.unwrap_or(Stop::Other(String::from("bez konce"))), usage, bad)
        } else {
            let (m, stop, usage) = decode_response(text)?;
            for ev in events_of(&m, &stop, &usage) {
                sink(&ev);
            }
            (m, stop, usage, Vec::new())
        };
        Ok(Response { message, stop, usage, bad_inputs: bad, sent_bytes: sent })
    }
}
