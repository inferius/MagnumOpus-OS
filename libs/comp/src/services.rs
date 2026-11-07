//! Akce aplikaci (kanal `actions`) a inspekce (kanal `inspect`).
//!
//! Volani akce: kompozitor najde aplikaci podle id z manifestu, overi
//! argumenty proti deklaraci a teprve pak ji preda `ACTION_INVOKE`. Akce
//! s `confirm = true` nebo volana automatem (`synthetic`) se pred predanim
//! potvrdi systemovym dialogem.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use guiproto::Rect;
use guiproto::action::{Arg, Value};
use guiproto::control::{CallerMsg, CallerReply, DialogKind, InspectReply, InspectRequest, WindowInfo};
use guiproto::window::ServerMsg;

use crate::dialog::{Dialog, Owner};
use crate::{ClientId, Comp, Out, PeerId};

struct Call {
    caller: PeerId,
    caller_name: String,
    /// Id volani, jak ho zvolil volajici.
    call: u32,
    client: ClientId,
    action: String,
    args: Vec<Arg>,
    synthetic: bool,
    /// Uz predano aplikaci (ceka se na `ACTION_RESULT`).
    invoked: bool,
}

/// Nejvic rozpracovanych volani.
const MAX_CALLS: usize = 256;

#[derive(Default)]
pub struct Services {
    calls: BTreeMap<u32, Call>,
    next: u32,
}

fn fail(call: u32, msg: &str) -> CallerReply {
    CallerReply::Result {
        call,
        ok: false,
        value: None,
        message: msg.into(),
    }
}

impl Services {
    pub(crate) fn action_result(&mut self, client: ClientId, id: u32, ok: bool, value: Option<Value>, message: String, out: &mut Vec<Out>) {
        let Some(c) = self.calls.get(&id) else { return };
        if c.client != client || !c.invoked {
            return;
        }
        let c = self.calls.remove(&id).unwrap();
        out.push(Out::Caller(
            c.caller,
            CallerReply::Result {
                call: c.call,
                ok,
                value,
                message,
            },
        ));
    }

    pub(crate) fn client_gone(&mut self, client: ClientId, out: &mut Vec<Out>) {
        let gone: Vec<u32> = self.calls.iter().filter(|(_, c)| c.client == client).map(|(&k, _)| k).collect();
        for k in gone {
            let c = self.calls.remove(&k).unwrap();
            out.push(Out::Caller(c.caller, fail(c.call, "aplikace skoncila")));
        }
    }
}

impl Comp {
    /// Zprava na kanalu `actions` od volajiciho `peer` (jmeno mu dal init).
    pub fn caller_msg(&mut self, peer: PeerId, caller_name: &str, msg: CallerMsg) {
        match msg {
            CallerMsg::List => {
                let list = self
                    .clients
                    .values()
                    .flat_map(|c| c.info.actions.iter().map(move |a| (c.info.id.clone(), a.clone())))
                    .collect();
                self.out.push(Out::Caller(peer, CallerReply::Actions { list }));
            }
            CallerMsg::Call {
                call,
                app,
                action,
                args,
                synthetic,
            } => {
                let Some((&cid, cl)) = self.clients.iter().find(|(_, c)| c.info.id == app) else {
                    return self.out.push(Out::Caller(peer, fail(call, "aplikace nebezi")));
                };
                let Some(decl) = cl.info.actions.iter().find(|a| a.id == action) else {
                    return self.out.push(Out::Caller(peer, fail(call, "aplikace takovou akci nema")));
                };
                if decl.check(&args).is_err() {
                    return self.out.push(Out::Caller(peer, fail(call, "argumenty neodpovidaji deklaraci")));
                }
                if self.services.calls.len() >= MAX_CALLS {
                    return self.out.push(Out::Caller(peer, fail(call, "prilis mnoho volani")));
                }
                let confirm = decl.confirm || synthetic;
                let title = decl.title.clone();
                let needs = decl.needs.join(", ");
                let app_name = cl.info.name.clone();
                let id = self.services.next;
                self.services.next = self.services.next.wrapping_add(1);
                self.services.calls.insert(
                    id,
                    Call {
                        caller: peer,
                        caller_name: caller_name.into(),
                        call,
                        client: cid,
                        action,
                        args,
                        synthetic,
                        invoked: false,
                    },
                );
                if confirm {
                    let who = if synthetic { format!("{caller_name} (automat)") } else { String::from(caller_name) };
                    let mut text = format!("{who} chce v aplikaci {app_name} provést akci „{title}“.");
                    if !needs.is_empty() {
                        text.push_str(&format!("\nAkce potřebuje: {needs}."));
                    }
                    self.open_dialog(Dialog {
                        owner: Owner::Action(id),
                        id,
                        kind: DialogKind::Confirm,
                        app: app_name,
                        text,
                        options: alloc::vec![String::from("Povolit"), String::from("Zamítnout")],
                        sel: 1,
                        password: String::new(),
                    });
                } else {
                    self.invoke(id);
                }
            }
        }
    }

    fn invoke(&mut self, id: u32) {
        let Some(c) = self.services.calls.get_mut(&id) else { return };
        c.invoked = true;
        let msg = ServerMsg::ActionInvoke {
            call: id,
            action: c.action.clone(),
            args: c.args.clone(),
            caller: c.caller_name.clone(),
            synthetic: c.synthetic,
        };
        let client = c.client;
        self.send(client, msg);
    }

    pub(crate) fn action_confirmed(&mut self, id: u32, ok: bool) {
        if ok {
            return self.invoke(id);
        }
        if let Some(c) = self.services.calls.remove(&id) {
            self.out.push(Out::Caller(c.caller, fail(c.call, "zamitnuto uzivatelem")));
        }
    }

    /// Kanal `actions` zavren.
    pub fn caller_gone(&mut self, peer: PeerId) {
        let gone: Vec<u32> = self.services.calls.iter().filter(|(_, c)| c.caller == peer).map(|(&k, _)| k).collect();
        self.services.calls.retain(|_, c| c.caller != peer);
        // Potvrzeni volani, ktere uz nikdo neceka, zmizi.
        if self.dialogs.remove_where(|o| matches!(o, Owner::Action(id) if gone.contains(&id))) {
            self.damage_all();
        }
    }

    /// Zadost na kanalu `inspect`. `Pixel` vyrizuje sluzba (ma snimek).
    pub fn inspect(&mut self, peer: PeerId, req: InspectRequest) {
        let reply = match req {
            InspectRequest::Windows => InspectReply::Windows(
                self.z
                    .iter()
                    .filter_map(|g| {
                        // Jen zobrazena okna (po prvnim COMMIT).
                        let w = self.windows.get(g).filter(|w| w.mapped)?;
                        let c = self.clients.get(&w.client)?;
                        Some(WindowInfo {
                            id: *g,
                            app: c.info.id.clone(),
                            title: w.title.clone(),
                            rect: w.rect,
                            focused: self.focus == Some(*g),
                        })
                    })
                    .collect(),
            ),
            InspectRequest::Tree { window } => match self.windows.get(&window) {
                None => InspectReply::Error(guiproto::Error::NotFound),
                Some(w) => {
                    let (c, origin) = (w.client, (w.rect.x, w.rect.y));
                    let nodes = w
                        .a11y
                        .nodes()
                        .iter()
                        .map(|n| {
                            let mut n = n.clone();
                            let (lx, ly) = if n.layer == 0 { (0, 0) } else { self.layer_origin(c, n.layer).unwrap_or((0, 0)) };
                            n.bounds = Rect::new(origin.0 + lx + n.bounds.x, origin.1 + ly + n.bounds.y, n.bounds.w, n.bounds.h);
                            n.layer = 0;
                            n
                        })
                        .collect();
                    InspectReply::Tree(nodes)
                }
            },
            InspectRequest::Act { window, node, action, value } => match self.windows.get(&window) {
                Some(w) if w.a11y.get(node).is_some_and(|n| n.actions & action != 0) => {
                    let (c, l) = (w.client, w.local);
                    self.send(c, ServerMsg::A11yAction { window: l, node, action, value });
                    InspectReply::Done
                }
                _ => InspectReply::Error(guiproto::Error::NotFound),
            },
            InspectRequest::Settle => {
                if self.is_settled() {
                    InspectReply::Done
                } else {
                    self.frame.settle.push(peer);
                    return;
                }
            }
            InspectRequest::Pixel { .. } => InspectReply::Error(guiproto::Error::NotSupported),
        };
        self.out.push(Out::Inspect(peer, reply));
    }
}
