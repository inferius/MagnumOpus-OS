//! "Obnov po sobe": testy, ktere meni globalni stav FS (popis opravneni
//! `/` a `/home`, zakaz spousteni), ho menit jen pres [`Restore`]. Pri
//! dropu (konec testu, i po chybe a predcasnem `?`) vrati puvodni hodnoty
//! v opacnem poradi. Jinak by zmena prosakla do dalsich sad (napr.
//! `ucty`: vypis `/home` po testu, ktery na nem nechal jen TRAVERSE).

use alloc::string::String;
use alloc::vec::Vec;

use fsproto::FsClient;
use fsproto::perm::Acl;
use libos::println;

use super::{TestResult, fail};

enum Saved {
    Acl(String, Option<Acl>),
    Noexec(String, bool),
}

pub struct Restore<'a> {
    fs: &'a FsClient,
    saved: Vec<Saved>,
}

impl<'a> Restore<'a> {
    pub fn new(fs: &'a FsClient) -> Self {
        Self { fs, saved: Vec::new() }
    }

    /// Nastavi popis opravneni `path`; puvodni (pri prvni zmene teto
    /// cesty) se vrati pri dropu.
    pub fn set_acl(&mut self, path: &str, acl: Option<&Acl>) -> TestResult {
        if !self.saved.iter().any(|s| matches!(s, Saved::Acl(p, _) if p == path)) {
            let old = self.fs.permissions(path).map_err(|e| fail(&alloc::format!("opravneni {path}"), e))?.acl;
            self.saved.push(Saved::Acl(String::from(path), old));
        }
        self.fs.set_acl(path, acl).map_err(|e| fail(&alloc::format!("acl {path}"), e))
    }

    /// Nastavi zakaz spousteni `path`; puvodni se vrati pri dropu.
    pub fn set_noexec(&mut self, path: &str, on: bool) -> TestResult {
        if !self.saved.iter().any(|s| matches!(s, Saved::Noexec(p, _) if p == path)) {
            let old = self.fs.noexec(path).map_err(|e| fail(&alloc::format!("noexec {path}"), e))?;
            self.saved.push(Saved::Noexec(String::from(path), old));
        }
        self.fs.set_noexec(path, on).map_err(|e| fail(&alloc::format!("noexec {path}"), e))
    }
}

impl Drop for Restore<'_> {
    fn drop(&mut self) {
        for s in self.saved.drain(..).rev() {
            let r = match &s {
                Saved::Acl(p, acl) => self.fs.set_acl(p, acl.as_ref()).map_err(|e| (p.clone(), e)),
                Saved::Noexec(p, on) => self.fs.set_noexec(p, *on).map_err(|e| (p.clone(), e)),
            };
            if let Err((p, e)) = r {
                println!("init: test: puvodni stav {p} nejde vratit: {e}");
            }
        }
    }
}
