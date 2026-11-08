use crate::json::{Value, parse};
use crate::tool::{Schema, validate};

#[test]
fn json_tam_a_zpet() {
    let text = r#"{"a":1,"b":[true,false,null],"c":"x\"y\\z\n\u00e9","d":-2.5,"e":{}}"#;
    let v = parse(text).unwrap();
    assert_eq!(v.get("a"), Some(&Value::Int(1)));
    assert_eq!(v.str_of("c"), "x\"y\\z\né");
    assert_eq!(v.get("d"), Some(&Value::Float(-2.5)));
    // Poradi klicu se zachova; zapis je kompaktni.
    assert_eq!(parse(&v.to_json()).unwrap(), v);
    assert!(v.to_json().starts_with(r#"{"a":1,"b":[true,false,null],"c":"#));
}

#[test]
fn json_unicode_par_a_utf8() {
    let v = parse(r#""\ud83d\ude00 čeština""#).unwrap();
    assert_eq!(v.as_str(), Some("\u{1F600} čeština"));
    assert_eq!(parse(&v.to_json()).unwrap(), v);
}

#[test]
fn json_chyby_bez_paniky() {
    for bad in ["", "{", "[1,", "{\"a\" 1}", "tru", "\"\\x\"", "1 2", "{\"a\":}", "\"\u{1}\"", "-", "1.", "\"\\ud800\""] {
        assert!(parse(bad).is_err(), "{bad:?} melo selhat");
    }
    // Hloubka je omezena.
    let deep = "[".repeat(100) + &"]".repeat(100);
    assert!(parse(&deep).is_err());
}

#[test]
fn json_ridici_znaky_escapovane() {
    let v = Value::str("a\u{1}b\tc");
    assert_eq!(v.to_json(), "\"a\\u0001b\\tc\"");
}

#[test]
fn schema_overi_vstup() {
    let s = Schema::object().string("path", "cesta", true).integer("max", "nejvic", false).strings("tags", "znacky", false).build();
    assert!(validate(&s, &parse(r#"{"path":"/a","max":3,"tags":["x"]}"#).unwrap()).is_ok());
    let e = validate(&s, &parse(r#"{"max":3}"#).unwrap()).unwrap_err();
    assert!(e.contains("path"), "{e}");
    let e = validate(&s, &parse(r#"{"path":1}"#).unwrap()).unwrap_err();
    assert!(e.contains("string"), "{e}");
    let e = validate(&s, &parse(r#"{"path":"/a","navic":1}"#).unwrap()).unwrap_err();
    assert!(e.contains("navic"), "{e}");
    let e = validate(&s, &parse(r#"{"path":"/a","tags":[1]}"#).unwrap()).unwrap_err();
    assert!(e.contains("tags[0]"), "{e}");
    assert!(validate(&s, &parse("[]").unwrap()).is_err());
}
