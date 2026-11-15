//! Mod POSIX (`st_mode`, `mkdir`, `chmod`) jako pohled na opravneni
//! (rozhodnuti T1 koordinatora, varianta A; docs/cs/souborovy-system.md,
//! kapitola Mod POSIX).
//!
//! FS ma granty, ne bity. Devet bitu `rwxrwxrwx` je jen pohled:
//!
//! - **user** = vlastnik: ma vzdy vsechna prava (`rw-` u souboru - bit
//!   spusteni urcuje manifest a zakaz spousteni datasetu, ne bit; `rwx`
//!   u adresare),
//! - **group** = granty ostatnim prijemcum krome `EVERYONE` (skupiny
//!   i jini uzivatele),
//! - **other** = grant `EVERYONE`.
//!
//! Prevod prav: soubor `r` = `READ`, `w` = `WRITE`, `x` = `EXECUTE`;
//! adresar `r` = `LIST`, `w` = `CREATE` (a `DELETE`), `x` = `TRAVERSE`.
//! Bity setuid, setgid a sticky se ignoruji.
//!
//! Mod pri vytvoreni (`mkdir`, `open(O_CREAT)`) zdedene granty jen
//! **zuzi**, nikdy nerozsiri; `chmod` (pravo `MANAGE`) smi i rozsirit.
//! Zmena = vlastni popis s `PROTECTED` (zdedene granty se zkopiruji
//! zuzene, potomci adresare je dal dedi).

use alloc::vec::Vec;

use crate::perm::{Acl, EVERYONE, Grant, Inherited, acl_flags, grant, rights};

/// Bity jedne tridy (`r` 4, `w` 2, `x` 1).
const R: u16 = 4;
const W: u16 = 2;
const X: u16 = 1;

/// Prava, ktera bity jedne tridy popisuji (ostatni prava mod nemeni).
fn representable(dir: bool) -> u8 {
    if dir {
        rights::LIST | rights::READ | rights::CREATE | rights::DELETE | rights::WRITE | rights::TRAVERSE
    } else {
        rights::READ | rights::WRITE | rights::EXECUTE
    }
}

/// Bity tridy z prav.
pub fn bits_of(r: u8, dir: bool) -> u16 {
    let (rr, ww, xx) = if dir {
        (rights::LIST, rights::CREATE, rights::TRAVERSE)
    } else {
        (rights::READ, rights::WRITE, rights::EXECUTE)
    };
    let mut b = 0;
    if r & rr != 0 {
        b |= R;
    }
    if r & ww != 0 {
        b |= W;
    }
    if r & xx != 0 {
        b |= X;
    }
    b
}

/// Prava z bitu tridy.
pub fn rights_of(bits: u16, dir: bool) -> u8 {
    let mut r = 0;
    if bits & R != 0 {
        r |= if dir { rights::LIST | rights::READ } else { rights::READ };
    }
    if bits & W != 0 {
        r |= if dir { rights::CREATE | rights::DELETE | rights::WRITE } else { rights::WRITE };
    }
    if bits & X != 0 {
        r |= if dir { rights::TRAVERSE } else { rights::EXECUTE };
    }
    r
}

/// Granty, ktere plati pro objekt: zdedene od rodice (bez `PROTECTED`)
/// a vlastni s `THIS`.
pub fn effective(above: &Inherited, acl: Option<&Acl>) -> Vec<Grant> {
    let mut out = Vec::new();
    if !acl.is_some_and(Acl::protected) {
        out.extend(above.grants.iter().copied());
    }
    if let Some(a) = acl {
        out.extend(a.grants.iter().copied().filter(|g| g.flags & grant::THIS != 0));
    }
    out
}

/// Trida grantu: 0 = vlastnik (neni v modu), 1 = group, 2 = other.
fn class(who: u64, owner: u64) -> u8 {
    if who == owner {
        0
    } else if who == EVERYONE {
        2
    } else {
        1
    }
}

/// Mod (9 bitu) objektu s vlastnikem `owner`, druhem a granty.
pub fn mode_of(owner: u64, dir: bool, grants: &[Grant]) -> u16 {
    let user = if dir { R | W | X } else { R | W };
    let (mut g, mut o) = (0u8, 0u8);
    for x in grants {
        match class(x.who, owner) {
            1 => g |= x.rights,
            2 => o |= x.rights,
            _ => {}
        }
    }
    (user << 6) | (bits_of(g, dir) << 3) | bits_of(o, dir)
}

/// Novy popis objektu pro mod `mode` (9 bitu): `None` = beze zmeny.
/// `widen` = smi pridat prava (`chmod`; rozsiri existujici granty tridy
/// a pro other prida grant `EVERYONE`), jinak jen ubira (vytvoreni).
pub fn apply(owner: u64, dir: bool, above: &Inherited, acl: Option<&Acl>, mode: u16, widen: bool) -> Option<Acl> {
    let rep = representable(dir);
    let want = |c: u8| -> u8 {
        let bits = if c == 1 { (mode >> 3) & 7 } else { mode & 7 };
        rights_of(bits, dir)
    };
    let adjust = |g: Grant| -> Option<Grant> {
        let c = class(g.who, owner);
        if c == 0 {
            return Some(g);
        }
        let allowed = want(c);
        let mut r = (g.rights & !rep) | (g.rights & allowed);
        if widen {
            r |= allowed;
        }
        // Trida bez bitu: grant zmizi cely (i prava mimo mod).
        if allowed == 0 {
            return None;
        }
        (r != 0).then_some(Grant { rights: r, ..g })
    };
    let mut grants: Vec<Grant> = Vec::new();
    if !acl.is_some_and(Acl::protected) {
        // Zdedene granty se zkopiruji (potomci je dedi dal jako predtim).
        grants.extend(above.grants.iter().filter_map(|&g| adjust(Grant { flags: grant::BOTH, ..g })));
    }
    if let Some(a) = acl {
        for &g in &a.grants {
            if g.flags & grant::THIS == 0 {
                // Jen pro potomky: mod objektu se ho netyka.
                grants.push(g);
            } else if let Some(n) = adjust(g) {
                grants.push(n);
            }
        }
    }
    if widen && want(2) != 0 && !grants.iter().any(|g| g.who == EVERYONE && g.flags & grant::THIS != 0) {
        grants.push(Grant { who: EVERYONE, rights: want(2), flags: grant::THIS });
    }
    let new = Acl { flags: acl_flags::PROTECTED, grants };
    let new = new.normalized().ok()?;
    // Stejny ucinek na objekt i na dedeni do potomku = nic nemenit (zadny
    // zbytecny vlastni popis).
    let unchanged = same(&effective(above, acl), &effective(above, Some(&new)))
        && same(&above.child(acl).grants, &above.child(Some(&new)).grants);
    if unchanged {
        return None;
    }
    Some(new)
}

/// Stejna prava pro kazdeho prijemce?
fn same(a: &[Grant], b: &[Grant]) -> bool {
    let fold = |v: &[Grant]| {
        let mut m: Vec<(u64, u8)> = Vec::new();
        for g in v {
            match m.iter_mut().find(|x| x.0 == g.who) {
                Some(x) => x.1 |= g.rights,
                None => m.push((g.who, g.rights)),
            }
        }
        m.sort();
        m
    };
    fold(a) == fold(b)
}
