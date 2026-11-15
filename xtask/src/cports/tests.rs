use super::configure::expand;
use super::tools::Tool;
use super::{Build, PORTS};

#[test]
fn configure_placeholders() {
    let vars = [("prefix", String::from("/p")), ("cc", String::from("clang")), ("host", String::from("x86_64-unknown-magnumopus"))];
    assert_eq!(expand("--prefix={prefix}", &vars), "--prefix=/p");
    assert_eq!(expand("--cc={cc} --host={host}", &vars), "--cc=clang --host=x86_64-unknown-magnumopus");
    // Nezname zastupne nechat (chybu ohlasi skript, ne tiche smazani).
    assert_eq!(expand("{nezname}", &vars), "{nezname}");
}

#[test]
fn every_tool_has_advice() {
    for t in [Tool::CMake, Tool::Ninja, Tool::Sh, Tool::Make] {
        assert!(!t.advice().is_empty());
    }
}

#[test]
fn ports_are_pinned() {
    for p in PORTS {
        assert_eq!(p.sha256.len(), 64, "{}: SHA-256", p.name);
        assert!(p.url.starts_with("https://"), "{}: jen HTTPS", p.name);
        assert!(p.url.ends_with(p.file), "{}: jmeno souboru z URL", p.name);
        if let Build::CMake { options, .. } = &p.build {
            assert!(options.iter().all(|o| o.starts_with("-D")), "{}: volby CMake", p.name);
        }
    }
}

#[test]
fn tool_versions_parse() {
    use super::tools::parse_version;
    assert_eq!(parse_version("cmake version 4.4.4\n\nCMake suite maintained"), Some((4, 4)));
    assert_eq!(parse_version("1.13.2\n"), Some((1, 13)));
    assert_eq!(parse_version("cmake version 3.24.0-rc1"), Some((3, 24)));
    assert_eq!(parse_version("bez verze"), None);
    assert!(Some((4, 4)) >= Tool::CMake.min_version());
    assert!(Some((3, 22)) < Tool::CMake.min_version());
}

#[test]
fn llvm_runtimes_pinned_and_subset() {
    let p = &super::cxx::LLVM;
    assert_eq!(p.sha256.len(), 64);
    assert!(p.url.starts_with("https://") && p.url.ends_with(p.file));
    // Verze runtime = verze clangu hostitele (ABI hlavicek, vestavene funkce).
    assert!(p.top.ends_with("23.1.5.src") && p.url.contains("llvmorg-23.1.5"));
}

#[test]
fn cxx_flags_order() {
    use crate::arch::X86_64;
    let root = std::path::Path::new("/s");
    let f = super::cxx::cxxflags(&X86_64, root, false).unwrap();
    let cxx = f.iter().position(|x| x.ends_with("include/c++/v1")).unwrap();
    let c = f.iter().position(|x| x.ends_with("/s/include")).unwrap();
    // Obalove hlavicky libc++ pred hlavickami C (#include_next).
    assert!(cxx < c && f.contains(&String::from("-nostdinc++")));
    assert!(f.contains(&String::from("-funwind-tables")));
    let l = super::cxx::cxx_ldflags(&X86_64, root, false);
    let lc = l.iter().position(|x| x == "-lc").unwrap();
    for lib in ["-lc++", "-lc++abi", "-lunwind"] {
        assert!(l.iter().position(|x| x == lib).unwrap() < lc, "{lib} pred -lc");
    }
    assert!(l.contains(&String::from("-Wl,--eh-frame-hdr")));
    let d = super::cxx::cxx_ldflags(&X86_64, root, true);
    assert!(d.iter().position(|x| x == "-l:libc++.so").unwrap() < d.iter().position(|x| x == "-l:libc.so").unwrap());
}

#[test]
fn shared_library_flags() {
    let f = crate::csysroot::ldflags_so(std::path::Path::new("/s"));
    assert!(f.iter().any(|x| x.ends_with("lib/crtdso.o")), "vlastni __dso_handle");
    assert!(!f.iter().any(|x| x.ends_with("crt1.o") || x == "-Wl,-pie" || x.starts_with("-Wl,--dynamic-linker")));
    // Ovladac clangu pro *-none-elf `-shared` linkeru nepreda.
    assert!(f.contains(&String::from("-Wl,-shared")));
}
