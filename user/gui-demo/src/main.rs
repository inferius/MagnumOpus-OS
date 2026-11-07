//! gui-demo: ukazkova aplikace nad knihovnou `ui` (milnik 6c).
//!
//! Pocitadlo, textove pole, zaskrtavaci pole a seznam poznamek v
//! posouvane oblasti. Akce z manifestu (`counter.add`, `note.add`,
//! `notes.clear`) dela totez co tlacitka - volat je muze shell, asistent
//! i agent (pres kompozitor, ktery overi argumenty).
//!
//! Startovni zprava (kind USER): kanal okna ke kompozitoru.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use guiproto::action::{Arg, Value};
use guiproto::window::Mode;
use libos::{Channel, Startup, println};
use ui::{App, Node, button, checkbox, column, label, list, progress, row, scroll, spacer, text_input};

libos::entry!(main);

#[derive(Clone)]
enum Msg {
    Add,
    Text(String),
    Important(bool),
    Save,
    Select(usize),
}

struct Demo {
    count: i64,
    text: String,
    important: bool,
    notes: Vec<String>,
    selected: Option<usize>,
}

impl Demo {
    fn add_note(&mut self, text: &str, important: bool) {
        let t = text.trim();
        if t.is_empty() {
            return;
        }
        self.notes.push(if important { format!("! {t}") } else { String::from(t) });
    }
}

impl App for Demo {
    type Msg = Msg;

    fn title(&self) -> String {
        format!("Ukázka ({} poznámek)", self.notes.len())
    }

    fn view(&self) -> Node<Msg> {
        column([
            label(&format!("Počítadlo: {}", self.count)).heading(),
            row([button("Přičíst").on_click(Msg::Add), spacer()]),
            text_input(&self.text).placeholder("Nová poznámka").on_change(Msg::Text).on_submit(Msg::Save),
            row([
                checkbox("Důležitá", self.important).on_toggle(Msg::Important),
                spacer(),
                button("Uložit").on_click(Msg::Save).disabled(self.text.trim().is_empty()),
            ]),
            scroll(list(self.notes.clone(), self.selected).on_select(Msg::Select)).height(150),
            progress((self.count.clamp(0, 10) * 10) as u32),
        ])
    }

    fn update(&mut self, m: Msg) {
        match m {
            Msg::Add => self.count += 1,
            Msg::Text(t) => self.text = t,
            Msg::Important(b) => self.important = b,
            Msg::Save => {
                let (t, i) = (core::mem::take(&mut self.text), self.important);
                self.add_note(&t, i);
            }
            Msg::Select(i) => self.selected = Some(i),
        }
    }

    fn action(&mut self, id: &str, args: &[Arg]) -> Result<Option<Value>, String> {
        let arg = |n: &str| args.iter().find(|(k, _)| k == n).map(|(_, v)| v);
        match id {
            "counter.add" => {
                let Some(Value::Int(by)) = arg("by") else { return Err("chybi by".into()) };
                self.count = self.count.saturating_add(*by);
                Ok(Some(Value::Int(self.count)))
            }
            "note.add" => {
                let Some(Value::Text(t)) = arg("text") else { return Err("chybi text".into()) };
                let imp = matches!(arg("important"), Some(Value::Bool(true)));
                self.add_note(t, imp);
                Ok(Some(Value::Int(self.notes.len() as i64)))
            }
            "notes.clear" => {
                self.notes.clear();
                self.selected = None;
                Ok(None)
            }
            _ => Err("neznama akce".into()),
        }
    }
}

fn main(startup: Startup) -> i64 {
    let Some(h) = startup.handles.into_iter().next() else {
        println!("gui-demo: chybi kanal okna");
        return 1;
    };
    let app = Demo {
        count: 0,
        text: String::new(),
        important: false,
        notes: Vec::new(),
        selected: None,
    };
    match ui::client::run(Channel::from_handle(h), app, Mode::Standard, 420, 380) {
        Ok(()) => 0,
        Err(e) => {
            println!("gui-demo: {e}");
            2
        }
    }
}
