use alloc::string::String;
use alloc::vec;

use crate::action::{Action, Outcome};
use crate::msg::{Stop, Usage};
use crate::policy::{Mode, ReadScope};
use crate::price::{Limits, Spend};
use crate::proto::{AgentMsg, ClientMsg, GateMsg, OpenSpec, StepInfo, WsChange};
use crate::registry::ProviderMsg;
use crate::tool::{Effect, Schema, ToolSpec};

#[test]
fn zpravy_brany_tam_a_zpet() {
    let mut s = OpenSpec::new(1000, Mode::Dev, "fake");
    s.workspace = String::from("/home/a/proj");
    s.read_scope = ReadScope::Workspace;
    s.never_leave = vec![String::from("/x/**")];
    s.script = String::from("say ahoj\n---\nsay dal");
    s.has_ws = true;
    let m = GateMsg::Open(s);
    assert_eq!(GateMsg::decode(&m.encode()).unwrap(), m);
    let p = GateMsg::Provider { name: String::from("sys") };
    assert_eq!(GateMsg::decode(&p.encode()).unwrap(), p);
}

#[test]
fn zpravy_klienta_tam_a_zpet() {
    let all = [
        ClientMsg::Ask { text: String::from("ahoj") },
        ClientMsg::Confirmed { id: 3, outcome: Outcome::Done(String::from("ok")) },
        ClientMsg::Confirmed { id: 4, outcome: Outcome::Declined },
        ClientMsg::Confirmed { id: 5, outcome: Outcome::Failed(abi::Error::AccessDenied, String::from("x")) },
        ClientMsg::Steps,
        ClientMsg::StepDiff { step: 2 },
        ClientMsg::Revert { to: 0 },
        ClientMsg::Audit { from: 7, max: 10 },
        ClientMsg::Spend,
        ClientMsg::SetKey { provider: String::from("claude"), key: b"k".to_vec() },
    ];
    for m in all {
        assert_eq!(ClientMsg::decode(&m.encode()).unwrap(), m);
    }
}

#[test]
fn zpravy_agentd_tam_a_zpet() {
    let all = [
        AgentMsg::Text { text: String::from("t") },
        AgentMsg::Tool { name: String::from("fs.read"), decision: String::from("allow"), summary: String::from("path=/a") },
        AgentMsg::Confirm { id: 1, action: Action::FsWrite { path: String::from("/a"), content: b"x".to_vec() }, preview: String::from("+x") },
        AgentMsg::Confirm { id: 2, action: Action::FsRename { from: String::from("/a"), to: String::from("/b") }, preview: String::new() },
        AgentMsg::Proposal { command: String::from("ls"), style: String::from("B"), other: String::from("ls()"), risky: false, explanation: String::from("e") },
        AgentMsg::TurnEnd { stop: Stop::EndTurn, usage: Usage { input: 1, output: 2, cache_write: 3, cache_read: 4 }, cost: 5, step: 6 },
        AgentMsg::Error { code: abi::Error::NoResources, message: String::from("limit") },
        AgentMsg::Steps(vec![StepInfo { step: 1, snapshot: String::from("s"), message: String::from("m"), time: 9 }]),
        AgentMsg::Diff(vec![WsChange { kind: 1, path: String::from("/a"), old_path: String::new(), is_dir: false }]),
        AgentMsg::Reverted { files: 2, step: 0 },
        AgentMsg::AuditPage { lines: vec![String::from("x")], next: 3 },
        AgentMsg::Spend { spend: Spend { session: 1, day: 2, day_index: 3 }, limits: Limits::default() },
        AgentMsg::KeyStored,
    ];
    for m in all {
        assert_eq!(AgentMsg::decode(&m.encode()).unwrap(), m);
    }
}

#[test]
fn neplatne_zpravy_bez_paniky() {
    for b in [&[][..], &[0], &[99], &[1, 255, 255, 255, 255]] {
        assert!(ClientMsg::decode(b).is_err());
        assert!(AgentMsg::decode(b).is_err());
        assert!(GateMsg::decode(b).is_err());
        assert!(ProviderMsg::decode(b).is_err());
    }
}

#[test]
fn registrace_nastroju() {
    let t = ToolSpec::new("sys.services", "Stav sluzeb.", Effect::Read, Schema::object().build());
    let m = ProviderMsg::Register { tools: vec![t] };
    assert_eq!(ProviderMsg::decode(&m.encode()).unwrap(), m);
    // Neplatne jmeno nastroje se odmitne.
    let bad = ProviderMsg::Register { tools: vec![ToolSpec::new("Spatne Jmeno", "", Effect::Read, Schema::object().build())] };
    assert!(ProviderMsg::decode(&bad.encode()).is_err());
    let c = ProviderMsg::Call { id: 1, tool: String::from("sys.services"), input: String::from("{}") };
    assert_eq!(ProviderMsg::decode(&c.encode()).unwrap(), c);
}
