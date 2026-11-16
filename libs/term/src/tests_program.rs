//! Program ze shellu: pozadavek initu a deleni vystupu (`program.rs`).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use shlang::host::ExecNeeds;
use shlang::value::Value;

use crate::perm::MountSpec;
use crate::program::{Request, lines};

#[test]
fn request_args_and_mounts() {
    let args = vec![
        (None, Value::str("10.0.2.2")),
        (Some(String::from("count")), Value::Int(3)),
        (Some(String::from("verbose")), Value::Bool(true)),
        (Some(String::from("quiet")), Value::Bool(false)),
    ];
    // Plna prava: cely FS bez zuzeni, sit ano.
    let all = ExecNeeds { all: true, net: true, cwd: String::from("/home/a"), ..Default::default() };
    let r = Request::new("ping", &args, &all);
    assert_eq!(r.args, vec!["10.0.2.2", "--count", "3", "--verbose"]);
    assert_eq!(r.cwd, "/home/a");
    assert!(r.mounts.is_empty() && r.net);
    // Skript s needs: jen jeho cesty, sit podle needs.
    let some = ExecNeeds { read: vec![String::from("/home/a")], cwd: String::from("/home/a"), ..Default::default() };
    let r = Request::new("ping", &[], &some);
    assert_eq!(r.mounts, vec![MountSpec { name: "/home/a".into(), target: "/home/a".into(), read_only: true }]);
    assert!(!r.net);
}

#[test]
fn output_lines() {
    let mut buf = Vec::new();
    assert_eq!(lines(&mut buf, b"ab"), Vec::<String>::new());
    assert_eq!(lines(&mut buf, b"c\r\nd\n\ne"), vec!["abc", "d", ""]);
    assert_eq!(buf, b"e");
    // Neplatne UTF-8 se nahradi, radek se neztrati.
    let mut buf = Vec::new();
    assert_eq!(lines(&mut buf, b"\xff x\n"), vec!["\u{fffd} x"]);
}
