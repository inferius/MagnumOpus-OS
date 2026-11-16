//! Vyzvy a nahled zmen (`ui::preview`): seznam zmen z detailu vyzvy,
//! popisky tlacitek a odpoved klikem.

use shlang::value::Value;

use crate::proto::{ClientMsg, Item, ServerMsg};
use crate::ui::App;
use crate::ui::draw::{PromptRow, Role};

fn change(c: &str, p: &str) -> Value {
    Value::record(&[("change", Value::str(c)), ("path", Value::str(p))])
}

fn app_with_prompt(detail: Item) -> App {
    let mut a = App::new(crate::theme::Theme::dark(), 80, 30);
    a.on_msg(ServerMsg::Attached { session: 7, cwd: "/home/eva".into(), blocks: 0 });
    a.on_msg(ServerMsg::Prompt { prompt: 3, question: "Převzít změny?".into(), detail, choices: Vec::new(), label: String::new() });
    a
}

#[test]
fn preview_lists_changes_and_take_button() {
    let rows: Vec<Value> = (1..=9).map(|i| change("removed", &format!("/home/eva/tmp/{i}.log"))).collect();
    let a = app_with_prompt(Item::Value(Value::List(rows)));
    let (_, lines) = a.frame_lines();
    let text: Vec<String> = lines.iter().map(|l| (0..l.parts_len()).map(|i| l.part_text(i)).collect()).collect();
    assert!(text.iter().any(|t| t.contains("removed") && t.contains("~/tmp/1.log")), "{text:?}");
    assert!(text.iter().any(|t| t.contains("a 3 dalších")), "zbytek seznamu: {text:?}");
    assert!(lines.iter().any(|l| l.role == Role::Prompt(PromptRow::Question { preview: true })));
    let changes = lines.iter().filter(|l| l.role == Role::Prompt(PromptRow::Change)).count();
    assert_eq!(changes, 7, "6 zmen a radek se zbytkem");
    let buttons = lines.iter().find_map(|l| match &l.role {
        Role::Prompt(PromptRow::Buttons { labels, preview, .. }) => Some((labels.clone(), *preview)),
        _ => None,
    });
    let (labels, preview) = buttons.expect("tlacitka");
    assert!(preview);
    assert_eq!(labels, vec!["Převzít 9 změn (a)".to_string(), "Zahodit (n)".to_string()]);
}

#[test]
fn plain_question_keeps_yes_no() {
    let mut a = app_with_prompt(Item::Note(String::new()));
    assert_eq!(a.prompt_labels(), vec!["Ano (a)".to_string(), "Ne (n)".to_string()]);
    let (_, lines) = a.frame_lines();
    assert!(lines.iter().any(|l| l.role == Role::Prompt(PromptRow::Question { preview: false })));
    // Klik na druhe tlacitko = ne.
    a.take_out();
    a.prompt_pick(1);
    assert!(a.prompt.is_none());
    assert_eq!(a.take_out(), vec![ClientMsg::Answer { prompt: 3, yes: false }]);
    // Bez vyzvy klik nic neposle.
    a.prompt_pick(0);
    assert!(a.take_out().is_empty());
}

#[test]
fn pick_on_take_answers_yes() {
    let mut a = app_with_prompt(Item::Value(Value::List(vec![change("added", "/x")])));
    a.take_out();
    a.prompt_pick(0);
    assert_eq!(a.take_out(), vec![ClientMsg::Answer { prompt: 3, yes: true }]);
}
