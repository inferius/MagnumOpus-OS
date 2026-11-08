use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::action::{Action, ActionFs, apply, diff_lines, preview_write};
use crate::audit::{Kind, Record, summarize_input};
use crate::fake::Fake;
use crate::json::parse;
use crate::msg::{Block, Message, Role, Stop, Usage};
use crate::price::{Limits, Over, Price, Spend, claude_price, usd};
use crate::provider::{Provider, ProviderError, Request};
use crate::secret::{Secret, open, seal};

fn req(messages: Vec<Message>) -> Request {
    Request { system: String::new(), messages, tools: Vec::new(), max_tokens: 100 }
}

#[test]
fn falesny_model_podle_skriptu() {
    let mut f = Fake::parse("# test\ntool fs.read {\"path\":\"/a\"}\n---\nsay vysledek: {result}\nusage 7 3\n").unwrap();
    let r = f.complete(&req(vec![Message::user_text("precti")]), &mut |_| {}).unwrap();
    assert_eq!(r.stop, Stop::ToolUse);
    let (id, name, input) = r.message.tool_uses().next().unwrap();
    assert_eq!((name, input.str_of("path")), ("fs.read", "/a"));
    let id = String::from(id);
    let msgs = vec![
        Message::user_text("precti"),
        r.message,
        Message { role: Role::User, content: vec![Block::ToolResult { id, content: String::from("OBSAH"), is_error: false }] },
    ];
    let r = f.complete(&req(msgs), &mut |_| {}).unwrap();
    assert_eq!(r.message.text(), "vysledek: OBSAH");
    assert_eq!(r.stop, Stop::EndTurn);
    assert_eq!(r.usage, Usage { input: 7, output: 3, ..Default::default() });
    assert_eq!(f.complete(&req(vec![]), &mut |_| {}).unwrap_err(), ProviderError::ScriptEnd);
    assert!(Fake::parse("tool x [1]").is_err());
    assert!(Fake::parse("neznamy").is_err());
}

#[test]
fn ceny_a_limity() {
    let p = claude_price("claude-opus-5-5");
    // 1M vstupu = 4 USD, 1M vystupu = 20 USD.
    assert_eq!(p.cost(&Usage { input: 1_000_000, ..Default::default() }), 4_000_000);
    assert_eq!(p.cost(&Usage { output: 1_000_000, ..Default::default() }), 20_000_000);
    assert_eq!(p.cost(&Usage { input: 1, ..Default::default() }), 4, "zaokrouhleni nahoru");
    assert_eq!(usd(1_234_567), "1.234567 USD");
    assert_eq!(Price::FREE.estimate(10_000, 16_000), 0);
    let l = Limits { max_tokens: 100, max_calls: 3, session: 1000, day: 1500 };
    let mut s = Spend::default();
    assert_eq!(s.check(&l, 500, 1), Ok(()));
    s.add(900, 1);
    assert_eq!(s.check(&l, 200, 2), Err(Over::Session));
    assert_eq!(s.check(&l, 1, 4), Err(Over::Calls));
    // Nova relace, stejny den: denni limit.
    let mut s2 = Spend { session: 0, day: 1400, day_index: 1 };
    assert_eq!(s2.check(&l, 200, 1), Err(Over::Day));
    s2.roll(2);
    assert_eq!(s2.check(&l, 200, 1), Ok(()));
    assert_eq!(Limits { max_tokens: 0, max_calls: 5, session: 0, day: 0 }.or_default().max_calls, 5);
}

#[test]
fn klic_vazany_na_uzivatele_a_poskytovatele() {
    let machine = [7u8; 32];
    let nonce = [1u8; 12];
    let sealed = seal(&machine, 1000, "claude", &nonce, b"sk-ant-tajne");
    assert!(!sealed.windows(5).any(|w| w == b"tajne"), "zasifrovany text nesmi obsahovat klic");
    assert_eq!(open(&machine, 1000, "claude", &sealed).unwrap().expose(), b"sk-ant-tajne");
    assert_eq!(open(&machine, 1001, "claude", &sealed).unwrap_err(), abi::Error::AccessDenied);
    assert_eq!(open(&machine, 1000, "jiny", &sealed).unwrap_err(), abi::Error::AccessDenied);
    assert_eq!(open(&[8u8; 32], 1000, "claude", &sealed).unwrap_err(), abi::Error::AccessDenied);
    let mut bad = sealed.clone();
    let last = bad.len() - 1;
    bad[last] ^= 1;
    assert!(open(&machine, 1000, "claude", &bad).is_err());
    assert!(open(&machine, 1000, "claude", b"kratke").is_err());
    assert_eq!(alloc::format!("{:?}", Secret::new(b"sk".to_vec())), "Secret(***)");
}

#[test]
fn audit_radek_tam_a_zpet() {
    let r = Record {
        seq: 1,
        time: 1_700_000_000,
        session: 2,
        step: 3,
        kind: Kind::Tool,
        what: String::from("fs.write"),
        input: String::from("path=/a\tb\nc"),
        decision: String::from("allow-ws"),
        result: String::from("ok"),
    };
    let line = r.line();
    assert_eq!(line.matches('\n').count(), 1);
    assert_eq!(Record::parse(&line).unwrap(), r);
    // Obsah zapisu se do auditu nedostane.
    let s = summarize_input(&parse(r#"{"path":"/a","content":"TAJNY OBSAH"}"#).unwrap());
    assert!(s.contains("path=/a") && !s.contains("TAJNY"), "{s}");
}

#[test]
fn nahled_zmeny() {
    let d = diff_lines("a\nb\nc\n", "a\nB\nc\nd\n");
    assert_eq!(d, " a\n-b\n+B\n c\n+d\n");
    let p = preview_write("/x.txt", None, b"novy\n");
    assert!(p.contains("(novy soubor)") && p.contains("+novy"), "{p}");
    let p = preview_write("/x.bin", Some(&[0xff, 0xfe]), &[0, 0xff]);
    assert!(p.contains("binarni"), "{p}");
}

#[derive(Default)]
struct MemFs {
    files: Vec<(String, Vec<u8>)>,
    log: Vec<String>,
}

impl ActionFs for MemFs {
    fn read(&mut self, path: &str) -> crate::Result<Vec<u8>> {
        self.files.iter().find(|(p, _)| p == path).map(|(_, d)| d.clone()).ok_or(abi::Error::NotFound)
    }
    fn write(&mut self, path: &str, data: &[u8]) -> crate::Result<()> {
        self.files.push((String::from(path), data.to_vec()));
        Ok(())
    }
    fn mkdir(&mut self, path: &str) -> crate::Result<()> {
        self.log.push(alloc::format!("mkdir {path}"));
        Ok(())
    }
    fn trash(&mut self, path: &str) -> crate::Result<()> {
        self.log.push(alloc::format!("trash {path}"));
        Ok(())
    }
    fn rename(&mut self, from: &str, to: &str) -> crate::Result<()> {
        self.log.push(alloc::format!("mv {from} {to}"));
        Ok(())
    }
}

#[test]
fn provedeni_akce_u_klienta() {
    let mut fs = MemFs::default();
    apply(&mut fs, &Action::FsWrite { path: String::from("/a"), content: b"x".to_vec() }).unwrap();
    assert_eq!(fs.read("/a").unwrap(), b"x");
    apply(&mut fs, &Action::FsTrash { path: String::from("/a") }).unwrap();
    assert_eq!(fs.log, vec![String::from("trash /a")]);
    assert_eq!(apply(&mut fs, &Action::Run { command: String::from("rm -r /") }).unwrap_err(), abi::Error::NotSupported);
}
