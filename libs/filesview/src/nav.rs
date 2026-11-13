//! Navigace: aktualni slozka, historie Zpet / Vpred, Nahoru a drobeckova
//! navigace (cesta pod domovem zacina "Domov").

use alloc::string::String;
use alloc::vec::Vec;

use fileops::path::{join, parent};

/// Nejvic kroku historie.
const MAX_HISTORY: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nav {
    pub cur: String,
    back: Vec<String>,
    fwd: Vec<String>,
}

impl Nav {
    pub fn new(start: &str) -> Self {
        Self { cur: String::from(start), back: Vec::new(), fwd: Vec::new() }
    }

    /// Jit jinam (vymaze historii Vpred).
    pub fn go(&mut self, path: &str) {
        if path == self.cur {
            return;
        }
        self.back.push(core::mem::replace(&mut self.cur, String::from(path)));
        if self.back.len() > MAX_HISTORY {
            self.back.remove(0);
        }
        self.fwd.clear();
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_forward(&self) -> bool {
        !self.fwd.is_empty()
    }

    pub fn can_up(&self) -> bool {
        self.cur != "/"
    }

    pub fn back(&mut self) -> bool {
        let Some(p) = self.back.pop() else { return false };
        self.fwd.push(core::mem::replace(&mut self.cur, p));
        true
    }

    pub fn forward(&mut self) -> bool {
        let Some(p) = self.fwd.pop() else { return false };
        self.back.push(core::mem::replace(&mut self.cur, p));
        true
    }

    /// Nahoru; vraci jmeno slozky, ze ktere se odeslo (vyber v rodici).
    pub fn up(&mut self) -> Option<String> {
        if !self.can_up() {
            return None;
        }
        let from = String::from(fileops::path::name(&self.cur));
        let p = String::from(parent(&self.cur));
        self.go(&p);
        Some(from)
    }

    /// Drobeckova navigace: (popis, cesta). Pod domovem `Domov`, jinak
    /// koren `Svazek`.
    pub fn crumbs(&self, home: &str) -> Vec<(String, String)> {
        let mut v = Vec::new();
        let rest = if fileops::path::is_within(&self.cur, home) && home != "/" {
            v.push((String::from("Domov"), String::from(home)));
            &self.cur[home.trim_end_matches('/').len()..]
        } else {
            v.push((String::from("Svazek"), String::from("/")));
            self.cur.as_str()
        };
        let mut path = String::from(v[0].1.as_str());
        for part in rest.split('/').filter(|s| !s.is_empty()) {
            path = join(&path, part);
            v.push((String::from(part), path.clone()));
        }
        v
    }
}
