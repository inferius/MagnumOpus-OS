//! Chybejici hodnoty s duvodem (`Value::Missing`): skryte (chybi pravo)
//! a nedostupne (FS hodnotu nevede) vs. prazdna slozka 0 B / 0 polozek.

use crate::eval::Interp;
use crate::ast::BinOp;
use crate::eval::ops::{binary, compare};
use crate::host::{Meta, NodeKind, hidden};
use crate::value::{Value, Why};

#[test]
fn missing_behaves_like_null() {
    let h = Value::Missing(Why::Hidden);
    assert_eq!(binary(BinOp::Eq, h.clone(), Value::Null).unwrap(), Value::Bool(true));
    assert_eq!(binary(BinOp::Eq, h.clone(), Value::Size(5)).unwrap(), Value::Bool(false));
    // `where size > 1MB` chybejici hodnotu vynecha, misto chyby.
    assert_eq!(binary(BinOp::Gt, h.clone(), Value::Size(1)).unwrap(), Value::Bool(false));
    assert_eq!(binary(BinOp::Lt, h.clone(), Value::Size(1)).unwrap(), Value::Bool(false));
    // Razeni: na zacatek.
    assert_eq!(compare(&h, &Value::Size(1)).unwrap(), core::cmp::Ordering::Less);
    assert_eq!(h.type_name(), "null");
    assert_eq!(h.to_text(), "\u{2014}");
    assert_eq!(crate::json::to_json(&h, false).unwrap(), "null");
}

fn record(m: &Meta) -> Value {
    Interp::file_record("/d", "d", m)
}

#[test]
fn folder_record_distinguishes_hidden_unavailable_and_empty() {
    let dir = Meta::basic(NodeKind::Dir, 0);
    // Bez prava spravovat: souhrn podstromu skryty.
    let r = record(&Meta { hidden: hidden::SIZE | hidden::TREE, ..dir });
    assert_eq!(r.field("size"), Some(Value::Missing(Why::Hidden)));
    assert_eq!(r.field("files"), Some(Value::Missing(Why::Hidden)));
    // FS souhrn nevede, cas nezna: nedostupne.
    let r = record(&dir);
    assert_eq!(r.field("files"), Some(Value::Missing(Why::Unavailable)));
    assert_eq!(r.field("mtime"), Some(Value::Missing(Why::Unavailable)));
    // Prazdna slozka: 0 B, 0 souboru, 0 slozek.
    let r = record(&Meta { tree: Some((0, 0)), ..dir });
    assert_eq!(r.field("size"), Some(Value::Size(0)));
    assert_eq!(r.field("files"), Some(Value::Int(0)));
    assert_eq!(r.field("dirs"), Some(Value::Int(0)));
    // Soubor: obsah podstromu nepatri (null, ne chybejici).
    let r = Interp::file_record("/a", "a", &Meta::basic(NodeKind::File, 3));
    assert_eq!(r.field("files"), Some(Value::Null));
    assert_eq!(r.field("size"), Some(Value::Size(3)));
}
