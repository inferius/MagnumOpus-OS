//! Preklad ACPICA (third_party/acpica) a lepidla v C (`c/glue.c`).
//!
//! Prekladaji se vsechny soubory komponent krome debuggeru
//! a disassembleru (ty nejsou ani ve vendorovane kopii). Priznaky a kontroly
//! bezpecnosti jsou spolecne pro veskery kod C (`libs/cbuild`).

use std::path::{Path, PathBuf};

/// Soubory, ktere se neprekladaji:
/// - `rsdump.c`: vypis zdroju pro debugger (bez ACPI_DEBUG_OUTPUT nejde
///   prelozit, Linux ho vynechava stejne),
/// - `utprint.c`: vlastni `printf`/`snprintf` ACPICA - mame je v cstd
///   (stejne symboly by se tloukly a cstd ma bezpecne chovani `%n`).
const EXCLUDED: &[&str] = &["rsdump.c", "utprint.c"];

fn c_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).expect("adresar ACPICA").flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            c_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "c") && !EXCLUDED.iter().any(|x| p.ends_with(x)) {
            out.push(p);
        }
    }
}

fn main() {
    let here = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let root = here.join("../../third_party/acpica/source");
    let mut sources = Vec::new();
    c_files(&root.join("components"), &mut sources);
    let include = root.join("include");
    let cstd = here.join("../cstd-core/include");
    let own = here.join("include");

    cbuild::Build::new("acpica")
        .trust(cbuild::Trust::Vendored)
        .define("__MAGNUMOPUS__", None)
        .include(&own)
        .include(&include)
        .include(include.join("platform"))
        .include(&cstd)
        .files(&sources)
        .compile();

    cbuild::Build::new("acpica_glue")
        .define("__MAGNUMOPUS__", None)
        .include(&own)
        .include_system(&include)
        .include_system(include.join("platform"))
        .include(&cstd)
        .file(here.join("c/glue.c"))
        .compile();
}
