//! Stav spravce souboru a zpracovani zprav (rozhrani je ve `view.rs`).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use fileops::job::{Op, Phase, Resolution};
use fileops::path::{is_within, join, name, numbered, parent, valid_name};
use fileops::proto::{self, ClientMsg, ServerMsg, Status};
use fsproto::hist::{FileVersion, TrashEntry};
use fsproto::{Stat, kind};

use crate::backend::Backend;
use crate::listing::{Listing, SortBy};
use crate::nav::Nav;
use crate::preview::{self, Content};

/// Co je v hlavni casti okna.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Folder,
    Trash,
}

/// Misto v postrannim panelu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub label: String,
    /// Cesta; prazdna = Kos.
    pub path: String,
    pub icon: &'static str,
}

/// Nahled vybrane polozky.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub path: String,
    pub stat: Result<Stat, abi::Error>,
    pub dataset: Option<String>,
    pub content: Option<Content>,
    pub versions: Result<Vec<FileVersion>, abi::Error>,
}

/// Polozka kose (dataset, polozka).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trashed {
    pub dataset: String,
    pub e: TrashEntry,
}

impl Trashed {
    /// Id radku: dataset a cislo polozky (v kazdem datasetu vlastni rada).
    pub fn row_id(&self) -> u64 {
        crate::backend::oid(&self.dataset) ^ self.e.id.rotate_left(17)
    }
}

/// Rozpracovane zadani jmena (nova slozka, prejmenovani).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameEdit {
    /// `Some(oid)` = prejmenovani polozky, `None` = nova slozka.
    pub item: Option<u64>,
    pub value: String,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    Go(String),
    Back,
    Forward,
    Up,
    Refresh,
    Place(usize),
    Search(String),
    Select(Vec<u64>),
    Activate(u64),
    Sort(usize, bool),
    NewFolder,
    Rename,
    NameValue(String),
    NameOk,
    NameCancel,
    Trash,
    UndoTrash,
    Copy,
    Cut,
    Paste,
    Decide(u64, Resolution),
    ForAll(bool),
    PauseJob(u64),
    ResumeJob(u64),
    CancelJob(u64),
    DismissJob(u64),
    RestoreVersion(usize),
    CopyVersion(usize),
    TrashSelect(Vec<u64>),
    TrashRestore,
    TrashPurge,
    TrashPurgeConfirm(bool),
}

pub struct FileManager<B: Backend> {
    pub b: B,
    pub home: String,
    pub nav: Nav,
    pub mode: Mode,
    pub listing: Listing,
    pub sel: Vec<u64>,
    pub places: Vec<Place>,
    /// Zprava v liste (chyba posledni operace, vysledek).
    pub status: String,
    pub search: String,
    pub edit: Option<NameEdit>,
    /// Schranka okna: operace a cesty (mezi okny az se schrankou formatu
    /// cest v `libs/ui`).
    pub clip: Option<(Op, Vec<String>)>,
    pub jobs: Vec<Status>,
    pub for_all: bool,
    pub preview: Option<Preview>,
    pub trash: Vec<Trashed>,
    pub trash_sel: Vec<u64>,
    pub confirm_purge: bool,
    /// Posledni presun do kose (dataset, id) pro Vratit.
    pub last_trashed: Vec<(String, u64)>,
    req: u32,
    /// Po navratu do rodice vybrat tuto polozku.
    reselect: Option<String>,
}

impl<B: Backend> FileManager<B> {
    pub fn new(b: B) -> Self {
        let home = b.home();
        let mut m = Self {
            b,
            nav: Nav::new(&home),
            home,
            mode: Mode::Folder,
            listing: Listing::default(),
            sel: Vec::new(),
            places: Vec::new(),
            status: String::new(),
            search: String::new(),
            edit: None,
            clip: None,
            jobs: Vec::new(),
            for_all: false,
            preview: None,
            trash: Vec::new(),
            trash_sel: Vec::new(),
            confirm_purge: false,
            last_trashed: Vec::new(),
            req: 0,
            reselect: None,
        };
        m.b.copier(ClientMsg::Hello { version: proto::VERSION });
        m.b.copier(ClientMsg::Subscribe);
        m.load_places();
        m.load();
        m
    }

    /// Postranni panel: domov, datasety (nejdriv pod domovem), svazek, kos.
    pub fn load_places(&mut self) {
        let mut v = alloc::vec![Place { label: String::from("Domov"), path: self.home.clone(), icon: "home" }];
        let mut ds = self.b.datasets().unwrap_or_default();
        ds.sort_by_key(|d| (!is_within(&d.path, &self.home), d.path.clone()));
        for d in ds {
            if d.path == "/" || d.path == self.home {
                continue;
            }
            let label = if is_within(&d.path, &self.home) { String::from(name(&d.path)) } else { d.path.clone() };
            v.push(Place { label, path: d.path, icon: "dataset" });
        }
        v.push(Place { label: String::from("Svazek"), path: String::from("/"), icon: "disk" });
        v.push(Place { label: String::from("Koš"), path: String::new(), icon: "trash" });
        self.places = v;
    }

    /// Nacte aktualni slozku (nebo kos).
    pub fn load(&mut self) {
        if self.mode == Mode::Trash {
            return self.load_trash();
        }
        let cur = self.nav.cur.clone();
        match self.b.list(&cur) {
            Ok(entries) => {
                let (now, zone) = (self.b.now(), self.b.zone().cloned());
                self.listing.set(entries, now, zone.as_ref());
                // Vyber jen z polozek, ktere jeste jsou.
                let ids: Vec<u64> = self.listing.items.iter().map(|i| i.e.oid).collect();
                self.sel.retain(|s| ids.contains(s));
                if let Some(n) = self.reselect.take()
                    && let Some(i) = self.listing.items.iter().find(|i| i.e.name == n)
                {
                    self.sel = alloc::vec![i.e.oid];
                }
            }
            Err(e) => {
                self.listing.set(Vec::new(), None, None);
                self.sel.clear();
                self.status = format!("Složku {cur} nejde otevřít: {}", crate::error_text(e));
            }
        }
        self.update_preview();
    }

    fn load_trash(&mut self) {
        let mut all = Vec::new();
        let mut denied = 0;
        for d in self.b.datasets().unwrap_or_default() {
            match self.b.trash_list(&d.path) {
                Ok(list) => all.extend(list.into_iter().map(|e| Trashed { dataset: d.path.clone(), e })),
                Err(_) => denied += 1,
            }
        }
        all.sort_by(|a, b| b.e.time.cmp(&a.e.time));
        self.trash = all;
        let ids: Vec<u64> = self.trash.iter().map(Trashed::row_id).collect();
        self.trash_sel.retain(|s| ids.contains(s));
        if denied > 0 {
            self.status = format!("Koš {} nejde zobrazit (chybí právo Spravovat).", crate::format::plural(denied, "datasetu", "datasetů", "datasetů"));
        }
    }

    /// Cesta polozky vypisu.
    pub fn path_of(&self, id: u64) -> Option<String> {
        self.listing.by_id(id).map(|i| join(&self.nav.cur, &i.e.name))
    }

    pub fn selected_paths(&self) -> Vec<String> {
        // V poradi vypisu (predvidatelne poradi kopirovani).
        self.listing.order.iter().map(|&k| &self.listing.items[k]).filter(|i| self.sel.contains(&i.e.oid)).map(|i| join(&self.nav.cur, &i.e.name)).collect()
    }

    fn update_preview(&mut self) {
        let one = if self.sel.len() == 1 { self.path_of(self.sel[0]) } else { None };
        let Some(path) = one else {
            self.preview = None;
            return;
        };
        if self.preview.as_ref().is_some_and(|p| p.path == path) {
            return;
        }
        let stat = self.b.stat(&path);
        let is_file = stat.as_ref().is_ok_and(|s| s.kind == kind::FILE);
        let content = is_file.then(|| match self.b.read(&path, 0, preview::TEXT_BYTES) {
            Ok(head) => preview::content(&head),
            Err(e) => Content::Unreadable(e),
        });
        let versions = if is_file { self.b.versions(&path) } else { Ok(Vec::new()) };
        let dataset = self.b.dataset_of(&path).ok();
        self.preview = Some(Preview { path, stat, dataset, content, versions });
    }

    fn go(&mut self, path: String) {
        self.mode = Mode::Folder;
        self.nav.go(&path);
        self.after_nav();
    }

    fn after_nav(&mut self) {
        self.sel.clear();
        self.edit = None;
        self.search.clear();
        self.listing.filter.clear();
        self.preview = None;
        self.load();
    }

    fn next_req(&mut self) -> u32 {
        self.req = self.req.wrapping_add(1);
        self.req
    }

    /// Volne jmeno nove slozky ("Nova slozka", "Nova slozka (2)", ...).
    fn free_name(&self, base: &str) -> String {
        let taken = |n: &str| self.listing.items.iter().any(|i| i.fold == n.to_lowercase());
        if !taken(base) {
            return String::from(base);
        }
        (2..).map(|k| numbered(base, k, true)).find(|n| !taken(n)).unwrap_or_default()
    }

    fn name_ok(&mut self) {
        let Some(e) = self.edit.clone() else { return };
        let v = e.value.trim();
        let err = if !valid_name(v) {
            "Jméno nesmí být prázdné ani obsahovat lomítko."
        } else if self.listing.items.iter().any(|i| i.fold == v.to_lowercase() && Some(i.e.oid) != e.item) {
            "Položka s tímto jménem už ve složce je."
        } else {
            ""
        };
        if !err.is_empty() {
            if let Some(ed) = &mut self.edit {
                ed.error = String::from(err);
            }
            return;
        }
        let target = join(&self.nav.cur, v);
        let r = match e.item.and_then(|id| self.path_of(id)) {
            Some(from) if from == target => Ok(()),
            Some(from) => self.b.rename(&from, &target),
            None => self.b.mkdir(&target),
        };
        match r {
            Ok(()) => {
                self.edit = None;
                self.reselect = Some(String::from(v));
                self.load();
            }
            Err(err) => {
                if let Some(ed) = &mut self.edit {
                    ed.error = String::from(crate::error_text(err));
                }
            }
        }
    }

    fn trash_selected(&mut self) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        self.last_trashed.clear();
        let mut failed = 0;
        let mut last_err = None;
        for p in &paths {
            let ds = self.b.dataset_of(p).unwrap_or_default();
            match self.b.trash(p) {
                Ok(id) => self.last_trashed.push((ds, id)),
                Err(e) => {
                    failed += 1;
                    last_err = Some(e);
                }
            }
        }
        let moved = self.last_trashed.len() as u64;
        self.status = match last_err {
            None => format!("{} v koši.", crate::format::plural(moved, "položka", "položky", "položek")),
            Some(e) => format!("{} v koši, {} ne: {}", crate::format::plural(moved, "položka", "položky", "položek"), failed, crate::error_text(e)),
        };
        self.sel.clear();
        self.load();
    }

    fn undo_trash(&mut self) {
        let items = core::mem::take(&mut self.last_trashed);
        let mut back = 0;
        for (ds, id) in items {
            if self.b.trash_restore(&ds, id).is_ok() {
                back += 1;
            }
        }
        self.status = format!("Vráceno: {}.", crate::format::plural(back, "položka", "položky", "položek"));
        self.load();
    }

    fn paste(&mut self) {
        let Some((op, sources)) = self.clip.clone() else { return };
        if sources.is_empty() || self.mode != Mode::Folder {
            return;
        }
        let dest = self.nav.cur.clone();
        for chunk in sources.chunks(proto::MAX_SOURCES) {
            let req = self.next_req();
            self.b.copier(ClientMsg::Start { req, op, sources: chunk.to_vec(), dest: dest.clone() });
        }
        if op == Op::Move {
            // Vyjmute polozky se vkladaji jednou.
            self.clip = None;
        }
    }

    fn restore_version(&mut self, i: usize, beside: bool) {
        let Some(p) = &self.preview else { return };
        let Some(v) = p.versions.as_ref().ok().and_then(|v| v.get(i)).cloned() else { return };
        let path = p.path.clone();
        let dest = beside.then(|| {
            let taken = |n: &str| self.listing.items.iter().any(|i| i.fold == n.to_lowercase());
            let base = name(&path);
            let first = format!("{} (verze){}", stem(base), ext(base));
            let free = if taken(&first) { (2..).map(|k| numbered(&first, k, false)).find(|n| !taken(n)).unwrap_or_default() } else { first };
            join(parent(&path), &free)
        });
        match self.b.restore_version(&path, &v.snapshot, dest.as_deref()) {
            Ok(out) => {
                self.status = if beside { format!("Verze uložena jako {}.", name(&out)) } else { String::from("Verze obnovena; předchozí obsah zůstal v historii.") };
                self.preview = None;
                self.load();
            }
            Err(e) => self.status = format!("Verzi nejde obnovit: {}", crate::error_text(e)),
        }
    }

    fn trash_action(&mut self, restore: bool) {
        let picked: Vec<Trashed> = self.trash.iter().filter(|t| self.trash_sel.contains(&t.row_id())).cloned().collect();
        let mut done = 0u64;
        let mut err = None;
        for t in picked {
            let r = if restore { self.b.trash_restore(&t.dataset, t.e.id).map(|_| ()) } else { self.b.trash_purge(&t.dataset, t.e.id) };
            match r {
                Ok(()) => done += 1,
                Err(e) => err = Some(e),
            }
        }
        let what = if restore { "Obnoveno" } else { "Trvale smazáno" };
        self.status = match err {
            None => format!("{what}: {}.", crate::format::plural(done, "položka", "položky", "položek")),
            Some(e) => format!("{what}: {}; chyba: {}", crate::format::plural(done, "položka", "položky", "položek"), crate::error_text(e)),
        };
        self.trash_sel.clear();
        self.confirm_purge = false;
        self.load_trash();
    }

    /// Zprava od `files-copy`. Vraci, zda prekreslit.
    pub fn copier_msg(&mut self, data: &[u8]) -> bool {
        let Ok(m) = ServerMsg::decode(data) else { return false };
        match m {
            ServerMsg::Started { .. } => false,
            ServerMsg::Rejected { error, .. } => {
                self.status = format!("Kopírování nejde spustit: {}", crate::error_text(error));
                true
            }
            ServerMsg::Status(s) => {
                let finished = matches!(s.phase, Phase::Done | Phase::Canceled);
                let touches = s.dest == self.nav.cur;
                match self.jobs.iter_mut().find(|j| j.job == s.job) {
                    Some(j) => *j = s,
                    None => self.jobs.push(s),
                }
                if finished && (touches || self.mode == Mode::Folder) {
                    self.load();
                }
                true
            }
        }
    }

    pub fn handle(&mut self, m: Msg) {
        match m {
            Msg::Go(p) => self.go(p),
            Msg::Back => {
                if self.nav.back() {
                    self.mode = Mode::Folder;
                    self.after_nav();
                }
            }
            Msg::Forward => {
                if self.nav.forward() {
                    self.mode = Mode::Folder;
                    self.after_nav();
                }
            }
            Msg::Up => {
                if let Some(from) = self.nav.up() {
                    self.mode = Mode::Folder;
                    self.reselect = Some(from);
                    self.after_nav();
                }
            }
            Msg::Refresh => {
                self.load_places();
                self.preview = None;
                self.load();
            }
            Msg::Place(i) => match self.places.get(i).cloned() {
                Some(p) if p.path.is_empty() => {
                    self.mode = Mode::Trash;
                    self.edit = None;
                    self.load_trash();
                }
                Some(p) => self.go(p.path),
                None => {}
            },
            Msg::Search(s) => {
                let (now, zone) = (self.b.now(), self.b.zone().cloned());
                self.listing.set_filter(&s, now, zone.as_ref());
                self.search = s;
            }
            Msg::Select(ids) => {
                self.sel = ids;
                self.update_preview();
            }
            Msg::Activate(id) => {
                let Some(it) = self.listing.by_id(id) else { return };
                let p = join(&self.nav.cur, &it.e.name);
                if it.is_dir() {
                    self.go(p);
                } else {
                    self.sel = alloc::vec![id];
                    self.update_preview();
                    self.status = if self.b.open(&p) { format!("Otevírám {} v aplikaci podle typu.", name(&p)) } else { String::from("Otevírání v aplikacích není k dispozici.") };
                }
            }
            Msg::Sort(c, asc) => {
                let (now, zone) = (self.b.now(), self.b.zone().cloned());
                self.listing.set_sort(SortBy::from_col(c), asc, now, zone.as_ref());
            }
            Msg::NewFolder => {
                if self.mode == Mode::Folder {
                    self.edit = Some(NameEdit { item: None, value: self.free_name("Nová složka"), error: String::new() });
                }
            }
            Msg::Rename => {
                if self.sel.len() == 1
                    && let Some(i) = self.listing.by_id(self.sel[0])
                {
                    self.edit = Some(NameEdit { item: Some(i.e.oid), value: i.e.name.clone(), error: String::new() });
                }
            }
            Msg::NameValue(v) => {
                if let Some(e) = &mut self.edit {
                    e.value = v;
                    e.error.clear();
                }
            }
            Msg::NameOk => self.name_ok(),
            Msg::NameCancel => self.edit = None,
            Msg::Trash => self.trash_selected(),
            Msg::UndoTrash => self.undo_trash(),
            Msg::Copy | Msg::Cut => {
                let paths = self.selected_paths();
                if !paths.is_empty() {
                    let op = if m == Msg::Copy { Op::Copy } else { Op::Move };
                    self.status = format!("{}: {} ve schránce.", if op == Op::Copy { "Kopírovat" } else { "Přesunout" }, crate::format::plural(paths.len() as u64, "položka", "položky", "položek"));
                    self.clip = Some((op, paths));
                }
            }
            Msg::Paste => self.paste(),
            Msg::Decide(job, r) => {
                let all = self.for_all;
                self.b.copier(ClientMsg::Decide { job, resolution: r, all });
                self.for_all = false;
            }
            Msg::ForAll(v) => self.for_all = v,
            Msg::PauseJob(job) => self.b.copier(ClientMsg::Pause { job }),
            Msg::ResumeJob(job) => self.b.copier(ClientMsg::Resume { job }),
            Msg::CancelJob(job) => self.b.copier(ClientMsg::Cancel { job }),
            Msg::DismissJob(job) => self.jobs.retain(|j| j.job != job),
            Msg::RestoreVersion(i) => self.restore_version(i, false),
            Msg::CopyVersion(i) => self.restore_version(i, true),
            Msg::TrashSelect(ids) => {
                self.trash_sel = ids;
                self.confirm_purge = false;
            }
            Msg::TrashRestore => self.trash_action(true),
            Msg::TrashPurge => self.confirm_purge = !self.trash_sel.is_empty(),
            Msg::TrashPurgeConfirm(yes) => {
                if yes {
                    self.trash_action(false);
                } else {
                    self.confirm_purge = false;
                }
            }
        }
    }

    /// Je cesta slozka (pro akci "ukazat cestu")?
    pub fn show_path(&mut self, path: &str) -> Result<(), String> {
        let st = self.b.stat(path).map_err(|e| format!("{path}: {}", crate::error_text(e)))?;
        if matches!(st.kind, kind::DIR | kind::DATASET) {
            self.go(String::from(path));
        } else {
            self.reselect = Some(String::from(name(path)));
            self.go(String::from(parent(path)));
        }
        Ok(())
    }
}

fn stem(n: &str) -> &str {
    match n.rfind('.') {
        Some(i) if i > 0 => &n[..i],
        _ => n,
    }
}

fn ext(n: &str) -> &str {
    match n.rfind('.') {
        Some(i) if i > 0 => &n[i..],
        _ => "",
    }
}
