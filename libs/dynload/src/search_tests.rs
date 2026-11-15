//! Testy hledani knihoven (DL-4c) na hostiteli.

use alloc::string::String;
use alloc::vec;

use super::search::*;

fn sys(n: &str) -> Source {
    Source::System(String::from(n))
}

fn disk(p: &str) -> Source {
    Source::Disk(String::from(p))
}

#[test]
fn normalize_paths() {
    assert_eq!(normalize("/a//b/./c/../d").as_deref(), Some("/a/b/d"));
    assert_eq!(normalize("/../..").as_deref(), Some("/"));
    assert_eq!(normalize("/").as_deref(), Some("/"));
    assert_eq!(normalize("a/b"), None);
    assert_eq!(normalize(""), None);
    assert_eq!(normalize("/a\0b"), None);
    assert_eq!(dir_of("/apps/x/bin/prog"), "/apps/x/bin");
    assert_eq!(dir_of("/prog"), "/");
    assert_eq!(file_name("/apps/x/lib/libz.so.1"), "libz.so.1");
}

#[test]
fn expand_origin() {
    assert_eq!(expand("$ORIGIN/../lib", Some("/apps/x/bin")).as_deref(), Some("/apps/x/lib"));
    assert_eq!(expand("${ORIGIN}/lib", Some("/apps/x")).as_deref(), Some("/apps/x/lib"));
    assert_eq!(expand("$ORIGIN", Some("/apps/x")).as_deref(), Some("/apps/x"));
    // Bez adresare modulu, neznama promenna, neuzavrena zavorka, relativni.
    assert_eq!(expand("$ORIGIN/lib", None), None);
    assert_eq!(expand("$LIB/x", Some("/a")), None);
    assert_eq!(expand("${ORIGIN/x", Some("/a")), None);
    assert_eq!(expand("lib", Some("/a")), None);
    assert_eq!(expand("/opt/lib", None).as_deref(), Some("/opt/lib"));
}

#[test]
fn candidates_order() {
    let c = candidates("libz.so.1", Some("$ORIGIN/../lib:/opt/lib:$ORIGIN/../lib"), Some("/apps/x/bin")).unwrap();
    assert_eq!(c, vec![disk("/apps/x/lib/libz.so.1"), disk("/opt/lib/libz.so.1"), sys("libz.so.1")]);
    // Bez RUNPATH jen system; RUNPATH na /system/lib se nezdvoji.
    assert_eq!(candidates("libc.so", None, Some("/a")).unwrap(), vec![sys("libc.so")]);
    assert_eq!(candidates("libc.so", Some("/system/lib:$ORIGIN"), Some("/a")).unwrap(), vec![sys("libc.so"), disk("/a/libc.so")]);
    // Modul bez adresare: polozky s $ORIGIN se preskoci.
    assert_eq!(candidates("libq.so", Some("$ORIGIN"), None).unwrap(), vec![sys("libq.so")]);
}

#[test]
fn candidates_paths() {
    assert_eq!(candidates("/apps/w/lib/../lib/ntdll.so", None, None).unwrap(), vec![disk("/apps/w/lib/ntdll.so")]);
    assert_eq!(candidates("/system/lib/libc.so", None, None).unwrap(), vec![sys("libc.so")]);
    assert_eq!(candidates("/system/lib/sub/x.so", None, None).unwrap(), vec![disk("/system/lib/sub/x.so")]);
    // Cesta mimo /system/lib pres `..` je disk, ne system.
    assert_eq!(candidates("/system/lib/../x.so", None, None).unwrap(), vec![disk("/system/x.so")]);
    assert!(candidates("lib/x.so", None, None).is_err());
    assert!(candidates("", None, None).is_err());
    assert!(candidates("..", None, None).is_err());
}
