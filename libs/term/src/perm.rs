//! Opravneni spousteneho programu -> pripojene cesty relace fsd.
//!
//! Sluzba relaci zalozi programu vlastni relaci fsd (`CONNECT_AS`) jen
//! s cestami z jeho manifestu a z omezeni skriptu (`needs`). Cesty se
//! pripojuji na stejne misto (program vidi `/home/x` jako `/home/x`), cteni
//! jen pro cteni. Novy narok za behu = dotaz uzivatele a `GRANT`.

use alloc::string::String;
use alloc::vec::Vec;

use shlang::host::ExecNeeds;
use shlang::vfs::path::{normalize, within};

/// Pripojena cesta: (jmeno v relaci, cesta ve FS, jen pro cteni).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountSpec {
    pub name: String,
    pub target: String,
    pub read_only: bool,
}

/// Pripojene cesty pro program s danymi naroky. Zapis prebije cteni na
/// stejne nebo nadrazene ceste; cesty uvnitr jine pripojene se vynechaji.
pub fn mounts(needs: &ExecNeeds) -> Vec<MountSpec> {
    if needs.all {
        return alloc::vec![MountSpec { name: "/".into(), target: "/".into(), read_only: false }];
    }
    let mut out: Vec<MountSpec> = Vec::new();
    let add = |p: &str, ro: bool, out: &mut Vec<MountSpec>| {
        let p = normalize(p);
        // uz pokryto stejne silnym nebo silnejsim pripojenim
        if out.iter().any(|m| within(&p, &m.target) && (!m.read_only || ro)) {
            return;
        }
        out.retain(|m| !(within(&m.target, &p) && (m.read_only || !ro)));
        out.push(MountSpec { name: p.clone(), target: p, read_only: ro });
    };
    for p in &needs.write {
        add(p, false, &mut out);
    }
    for p in &needs.read {
        add(p, true, &mut out);
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}
