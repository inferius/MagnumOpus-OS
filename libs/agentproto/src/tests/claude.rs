use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::claude::{Claude, Options, SseParser, Transport, decode_error, decode_response, encode_request};
use crate::json::{Value, parse};
use crate::msg::{Block, Message, Role, Stop};
use crate::provider::{Provider, ProviderError, Request};
use crate::secret::Secret;
use crate::stream::Accumulator;
use crate::tool::{Effect, Schema, ToolSpec};

fn request() -> Request {
    Request {
        system: String::from("Jsi agent."),
        messages: vec![
            Message::user_text("precti /a"),
            Message {
                role: Role::Assistant,
                content: vec![
                    Block::Opaque(parse(r#"{"type":"thinking","thinking":"","signature":"sig"}"#).unwrap()),
                    Block::ToolUse { id: String::from("t1"), name: String::from("fs.read"), input: parse(r#"{"path":"/a"}"#).unwrap() },
                ],
            },
            Message { role: Role::User, content: vec![Block::ToolResult { id: String::from("t1"), content: String::from("obsah"), is_error: false }] },
        ],
        tools: vec![ToolSpec::new("fs.read", "Precte soubor.", Effect::Read, Schema::object().string("path", "cesta", true).build())],
        max_tokens: 1000,
    }
}

#[test]
fn telo_pozadavku() {
    let body = encode_request(&request(), &Options::default());
    let v = parse(&body).unwrap();
    assert_eq!(v.str_of("model"), "claude-opus-5-5");
    assert_eq!(v.u64_of("max_tokens"), 1000);
    assert_eq!(v.str_of("system"), "Jsi agent.");
    assert!(v.get("temperature").is_none(), "verze API 2026-10-21 nepovoli temperature");
    let tools = v.get("tools").and_then(Value::as_array).unwrap();
    assert_eq!(tools[0].str_of("name"), "fs__read");
    assert_eq!(tools[0].get("input_schema").unwrap().str_of("type"), "object");
    let msgs = v.get("messages").and_then(Value::as_array).unwrap();
    // Myslenkovy blok projde beze zmeny, jmeno nastroje s __.
    let a = msgs[1].get("content").and_then(Value::as_array).unwrap();
    assert_eq!(a[0].str_of("signature"), "sig");
    assert_eq!(a[1].str_of("name"), "fs__read");
    let r = msgs[2].get("content").and_then(Value::as_array).unwrap();
    assert_eq!(r[0].str_of("type"), "tool_result");
    assert_eq!(r[0].str_of("tool_use_id"), "t1");
    assert_eq!(v.get("output_config").unwrap().str_of("effort"), "medium");
    // Deterministicke (cache promptu): stejny vstup = stejne bajty.
    assert_eq!(body, encode_request(&request(), &Options::default()));
}

#[test]
fn odpoved_s_volanim_nastroje() {
    let body = r#"{"id":"msg_1","type":"message","role":"assistant","content":[
        {"type":"thinking","thinking":"","signature":"abc"},
        {"type":"text","text":"Prectu."},
        {"type":"tool_use","id":"toolu_1","name":"fs__read","input":{"path":"/a"}}],
        "stop_reason":"tool_use","usage":{"input_tokens":100,"output_tokens":20,"cache_read_input_tokens":5}}"#;
    let (m, stop, usage) = decode_response(body).unwrap();
    assert_eq!(stop, Stop::ToolUse);
    assert_eq!(usage.input, 100);
    assert_eq!(usage.cache_read, 5);
    assert!(matches!(&m.content[0], Block::Opaque(v) if v.str_of("signature") == "abc"));
    let calls: Vec<_> = m.tool_uses().collect();
    assert_eq!(calls[0].1, "fs.read");
    assert_eq!(calls[0].2.str_of("path"), "/a");
}

#[test]
fn chyby_api() {
    assert!(matches!(decode_error(401, r#"{"type":"error","error":{"type":"authentication_error","message":"x"}}"#), ProviderError::Auth(_)));
    assert!(matches!(decode_error(529, r#"{"type":"error","error":{"type":"overloaded_error","message":"x"}}"#), ProviderError::Busy(_)));
    assert!(matches!(decode_error(400, "neni json"), ProviderError::Request(_)));
    assert!(matches!(decode_response("{"), Err(ProviderError::Protocol(_))));
    let (_, stop, _) = decode_response(r#"{"content":[],"stop_reason":"refusal"}"#).unwrap();
    assert_eq!(stop, Stop::Refusal);
}

#[test]
fn proud_sse_po_castech() {
    let stream = concat!(
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":50,\"output_tokens\":1}}}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Ahoj\"}}\n\n",
        "event: ping\ndata: {\"type\":\"ping\"}\n\n",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"t\",\"name\":\"fs__list\",\"input\":{}}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\":\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"/\\\"}\"}}\n\n",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":30}}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    );
    // Rozdelit na kousky po 7 bajtech (hranice udalosti uprostred).
    let mut p = SseParser::new();
    let mut acc = Accumulator::new();
    let bytes = stream.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let end = (i + 7).min(bytes.len());
        for ev in p.feed(core::str::from_utf8(&bytes[i..end]).unwrap()) {
            acc.push(&ev);
        }
        i = end;
    }
    assert!(p.error.is_none());
    let (m, stop, usage, bad) = acc.finish();
    assert!(bad.is_empty());
    assert_eq!(stop, Some(Stop::ToolUse));
    assert_eq!(usage.input, 50);
    assert_eq!(usage.output, 30);
    assert_eq!(m.text(), "Ahoj");
    let calls: Vec<_> = m.tool_uses().collect();
    assert_eq!(calls[0].1, "fs.list");
    assert_eq!(calls[0].2.str_of("path"), "/");
}

#[test]
fn neplatny_json_vstupu_v_proudu() {
    let mut acc = Accumulator::new();
    let mut p = SseParser::new();
    let s = concat!(
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"t\",\"name\":\"x\"}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"a\\\":\"}}\n\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"max_tokens\"},\"usage\":{\"output_tokens\":3}}\n\n",
    );
    for ev in p.feed(s) {
        acc.push(&ev);
    }
    let (_, stop, _, bad) = acc.finish();
    assert_eq!(stop, Some(Stop::MaxTokens));
    assert_eq!(bad.len(), 1, "orezany vstup se nesmi spustit");
}

struct Mock {
    seen: Vec<(String, String)>,
    reply: (u16, String),
}

impl Transport for Mock {
    fn post(&mut self, _url: &str, headers: &[(&str, &str)], _body: &[u8]) -> Result<(u16, Vec<u8>), ProviderError> {
        self.seen = headers.iter().map(|(k, v)| (String::from(*k), String::from(*v))).collect();
        Ok((self.reply.0, self.reply.1.clone().into_bytes()))
    }
}

#[test]
fn poskytovatel_s_dopravou() {
    let mock = Mock {
        seen: Vec::new(),
        reply: (200, String::from(r#"{"content":[{"type":"text","text":"hotovo"}],"stop_reason":"end_turn","usage":{"input_tokens":10,"output_tokens":2}}"#)),
    };
    let mut c = Claude::new(mock, Secret::new(b"sk-test".to_vec()), Options::default());
    let mut events = 0;
    let r = c.complete(&request(), &mut |_| events += 1).unwrap();
    assert_eq!(r.message.text(), "hotovo");
    assert!(events > 0);
    assert!(r.sent_bytes > 0);
    // Bez klice nic neposle.
    let mut c = Claude::new(Mock { seen: Vec::new(), reply: (200, String::new()) }, Secret::empty(), Options::default());
    assert_eq!(c.complete(&request(), &mut |_| {}).unwrap_err(), ProviderError::NoKey);
}

#[test]
fn klic_jde_jen_do_hlavicky() {
    let mock = Mock { seen: Vec::new(), reply: (401, String::from(r#"{"type":"error","error":{"type":"authentication_error","message":"bad"}}"#)) };
    let mut c = Claude::new(mock, Secret::new(b"sk-tajne".to_vec()), Options::default());
    let e = c.complete(&request(), &mut |_| {}).unwrap_err();
    assert!(matches!(e, ProviderError::Auth(_)));
    // Chyba klic neobsahuje.
    assert!(!alloc::format!("{e} {e:?}").contains("sk-tajne"));
}
