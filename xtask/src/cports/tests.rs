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
    for t in [Tool::CMake, Tool::Ninja, Tool::Sh, Tool::Make, Tool::Nasm, Tool::Meson, Tool::PkgConf] {
        assert!(!t.advice().is_empty());
    }
}

#[test]
fn ports_are_pinned() {
    for p in PORTS {
        assert_eq!(p.sha256.len(), 64, "{}: SHA-256", p.name);
        assert!(p.url.starts_with("https://"), "{}: jen HTTPS", p.name);
        assert!(p.url.ends_with(p.file), "{}: jmeno souboru z URL", p.name);
        if let Build::CMake { options, .. } | Build::Meson { options, .. } = &p.build {
            assert!(options.iter().all(|o| o.starts_with("-D")), "{}: volby", p.name);
        }
    }
}

#[test]
fn tool_versions_parse() {
    use super::tools::parse_version;
    assert_eq!(parse_version("cmake version 4.4.4\n\nCMake suite maintained"), Some((4, 4)));
    assert_eq!(parse_version("1.13.2\n"), Some((1, 13)));
    assert_eq!(parse_version("cmake version 3.24.0-rc1"), Some((3, 24)));
    assert_eq!(parse_version("NASM version 2.16.03 compiled on"), Some((2, 16)));
    assert_eq!(parse_version("bez verze"), None);
    assert!(Some((4, 4)) >= Tool::CMake.min_version());
    assert!(Some((3, 22)) < Tool::CMake.min_version());
    assert!(Some((2, 13)) < Tool::Nasm.min_version());
}

#[test]
fn ffmpeg_never_gpl_and_patent_switch() {
    use super::{FFMPEG_ARGS, PATENT_DECODERS, ffmpeg_args};
    for bad in ["--enable-gpl", "--enable-nonfree", "--enable-version3"] {
        assert!(!FFMPEG_ARGS.contains(&bad), "{bad}");
    }
    assert!(FFMPEG_ARGS.contains(&"--disable-gpl") && FFMPEG_ARGS.contains(&"--disable-nonfree"));
    let with = ffmpeg_args(FFMPEG_ARGS, false);
    assert!(!with.iter().any(|a| a.starts_with("--disable-decoder=")));
    let without = ffmpeg_args(FFMPEG_ARGS, true);
    for d in PATENT_DECODERS {
        assert!(without.contains(&format!("--disable-decoder={d}")), "{d}");
    }
}

#[test]
fn ffmpeg_config_check() {
    use super::{PATENT_DECODERS, check_ffmpeg_config};
    let dir = std::env::temp_dir().join(format!("xtask-ffcfg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let comps = |on: u8| PATENT_DECODERS.iter().map(|d| format!("#define CONFIG_{}_DECODER {on}\n", d.to_uppercase())).collect::<String>();
    let good = "#define CONFIG_GPL 0\n#define CONFIG_NONFREE 0\n#define CONFIG_VERSION3 0\n#define CONFIG_SHARED 1\n";
    std::fs::write(dir.join("config.h"), good).unwrap();
    std::fs::write(dir.join("config_components.h"), comps(1)).unwrap();
    assert!(check_ffmpeg_config(&dir, false).is_ok());
    assert!(check_ffmpeg_config(&dir, true).is_err(), "patentove dekodery zapnute, ackoli maji byt vypnute");
    std::fs::write(dir.join("config_components.h"), comps(0)).unwrap();
    assert!(check_ffmpeg_config(&dir, true).is_ok());
    std::fs::write(dir.join("config.h"), good.replace("CONFIG_GPL 0", "CONFIG_GPL 1")).unwrap();
    assert!(check_ffmpeg_config(&dir, true).is_err(), "GPL");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sh_paths() {
    use super::tools::sh_path;
    let p = std::path::Path::new(if cfg!(windows) { "J:\\a\\b" } else { "/a/b" });
    assert_eq!(sh_path(p), if cfg!(windows) { "/j/a/b" } else { "/a/b" });
}

#[test]
fn meson_cross_quotes() {
    // Retezce v souboru Mesonu: apostrof escapovany, zpetna lomitka na /.
    let a = super::meson::array_for_test(&[String::from("-DX='1'"), String::from("C:\\x")]);
    assert_eq!(a, "['-DX=\\'1\\'', 'C:/x']");
}
