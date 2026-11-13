//! installd: sluzba instalatoru (kroky a texty v `libs/installplan`, navrh
//! `design/instalace.html`).
//!
//! Spousti ho init az na zadost pruvodce (`install` v shellu). Startovni
//! zprava (kind USER): raw kanaly disku (cely disk; zapis do cizich oddilu
//! odmita ovladac, `diskguard::install`). Prvni zprava na bootstrap kanalu:
//! sektor a model disku po radcich (`<sektor> <model>`) ve stejnem poradi
//! a volby (`qemu-kdf` = parametry Argon2id pro QEMU). Pak init posila
//! `klient` s kanalem pro pruvodce (protokol `installplan::proto`)
//! a `tasks` s kanalem producenta do prehledu uloh (`tasks.rs`).
//!
//! Zivy disk (flashka s oddilem MO-LOGY) je zdroj systemu; instalovat na
//! nej nejde. Disk se sektorem jinym nez 512 B zatim nepodporuje.

#![no_std]
#![no_main]

extern crate alloc;

mod account;
mod disks;
mod tasks;
mod write;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use blockproto::BlockClient;
use disks::Disk;
use installplan::config::Settings;
use installplan::proto::{DiskEntry, KeyboardEntry, Reply, Request};
use installplan::map::{self, Seg};
use installplan::{Mode, choices, plan, preview_text};
use libos::{Channel, Error, Signals, Startup, println};

libos::entry!(main);

/// Nahoda z jadra (ceka na nasazeny generator).
pub fn random(dest: &mut [u8]) {
    loop {
        match libos::random::fill_seeded(dest) {
            Ok(()) => return,
            Err(Error::ShouldWait) => libos::time::sleep(core::time::Duration::from_millis(20)),
            Err(e) => panic!("installd: generator nahody: {e}"),
        }
    }
}

struct Service {
    disks: Vec<Disk>,
    qemu_kdf: bool,
    /// Instalace probehla (dalsi se odmita do restartu).
    done: bool,
    /// Hlaseni instalace do prehledu uloh (taskd).
    tasks: tasks::Tasks,
}

fn main(startup: Startup) -> i64 {
    let boot = &startup.bootstrap;
    let cfg = match boot.read(None) {
        Ok(m) => String::from_utf8_lossy(&m.data).into_owned(),
        Err(_) => return 1,
    };
    let mut lines = cfg.lines();
    let qemu_kdf = lines.next().is_some_and(|l| l.contains("qemu-kdf"));
    let mut disks = Vec::new();
    for (h, line) in startup.handles.into_iter().zip(lines) {
        let (ss, model) = line.split_once(' ').unwrap_or((line, ""));
        match BlockClient::new(Channel::from_handle(h)) {
            Ok(client) => disks.push(Disk {
                client,
                model: String::from(model.trim()),
                sector_size: ss.parse().unwrap_or(0),
            }),
            Err(e) => println!("installd: disk '{model}' neodpovida: {e}"),
        }
    }
    println!("installd: bezim, {} disku", disks.len());
    let mut svc = Service { disks, qemu_kdf, done: false, tasks: tasks::Tasks::default() };
    let _ = boot.write(b"ok", Vec::new());
    let mut clients: Vec<Channel> = Vec::new();
    loop {
        let mut items = alloc::vec![(boot.handle(), Signals::READABLE | Signals::PEER_CLOSED)];
        for c in &clients {
            items.push((c.handle(), Signals::READABLE | Signals::PEER_CLOSED));
        }
        let observed = match libos::wait_many(&items, None) {
            Ok((_, o)) => o,
            Err(_) => return 2,
        };
        drop(items);
        if observed[0].intersects(Signals::READABLE) {
            if let Ok(m) = boot.try_read() {
                match &m.data[..] {
                    b"klient" => clients.extend(m.handles.into_iter().map(Channel::from_handle)),
                    b"tasks" => {
                        svc.tasks.chan = m.handles.into_iter().next().map(Channel::from_handle);
                        let _ = boot.write(b"ok", Vec::new());
                    }
                    _ => {}
                }
            }
        } else if observed[0].intersects(Signals::PEER_CLOSED) {
            return 0;
        }
        let mut closed = Vec::new();
        for (i, s) in observed[1..].iter().enumerate() {
            if s.intersects(Signals::READABLE) {
                if let Ok(m) = clients[i].try_read() {
                    svc.request(&clients[i], &m.data);
                }
            } else if s.intersects(Signals::PEER_CLOSED) {
                closed.push(i);
            }
        }
        for i in closed.into_iter().rev() {
            clients.remove(i);
        }
    }
}

fn fail(e: Error, text: impl Into<String>) -> Reply {
    Reply::Failed(e, text.into())
}

impl Service {
    fn request(&mut self, ch: &Channel, data: &[u8]) {
        let reply = match Request::decode(data) {
            Ok(Request::Disks) => self.list(),
            Ok(Request::Keyboards) => keyboards(),
            Ok(Request::Preview { disk, mode }) => self.preview(disk, mode),
            Ok(Request::Install { disk, mode, confirm, settings, account }) => {
                self.install(ch, disk, mode, &confirm, &settings, &account)
            }
            Err(e) => fail(e, "neplatný požadavek"),
        };
        if let Err(e) = ch.write(&reply.encode(), Vec::new()) {
            println!("installd: odpoved pruvodci nelze poslat: {e}");
        }
    }

    fn list(&self) -> Reply {
        let mut out = Vec::new();
        for (i, d) in self.disks.iter().enumerate() {
            let t = d.table();
            let info = d.info(i as u32, t.as_ref());
            let mut ch = choices(&info, t.as_ref());
            if info.live || d.sector_size != 512 {
                for c in &mut ch {
                    c.available = false;
                    c.detail = String::from(if info.live {
                        "Z tohoto disku (živé USB) systém právě běží."
                    } else {
                        "Disky se sektorem 4 KiB instalátor zatím nepodporuje."
                    });
                }
            }
            out.push(DiskEntry {
                index: i as u32,
                model: d.model.clone(),
                bytes: info.bytes(),
                live: info.live,
                parts: Disk::describe(t.as_ref()),
                choices: ch,
            });
        }
        Reply::Disks(out)
    }

    /// Plan, disk a mapa disku teď a po instalaci (graf v GUI).
    fn plan_for(&self, disk: u32, mode: Mode) -> Result<(installplan::Plan, &Disk), Reply> {
        self.plan_map(disk, mode).map(|(p, d, _)| (p, d))
    }

    fn plan_map(&self, disk: u32, mode: Mode) -> Result<(installplan::Plan, &Disk, [Vec<Seg>; 2]), Reply> {
        let d = self.disks.get(disk as usize).ok_or_else(|| fail(Error::NotFound, "disk neexistuje"))?;
        if d.sector_size != 512 {
            return Err(fail(Error::NotSupported, "disky se sektorem 4 KiB instalátor zatím nepodporuje"));
        }
        let t = d.table();
        let info = d.info(disk, t.as_ref());
        let mut g = [[0u8; 16]; 3];
        for x in &mut g {
            random(x);
            *x = diskguard::install::guid_from(*x);
        }
        let p = plan(&info, t.as_ref(), mode, g).map_err(|e| fail(Error::InvalidArgs, e))?;
        let segs = [map::now(&info, t.as_ref()), map::after(&info, t.as_ref(), &p)];
        Ok((p, d, segs))
    }

    fn preview(&self, disk: u32, mode: Mode) -> Reply {
        match self.plan_map(disk, mode) {
            Ok((p, _, [before, after])) => Reply::Preview {
                text: preview_text(&p),
                type_name: match &p.confirm {
                    installplan::Confirm::TypeName(n) => Some(n.clone()),
                    installplan::Confirm::Agree => None,
                },
                other_os: p.other_os,
                lines: p.preview.clone(),
                before,
                after,
            },
            Err(r) => r,
        }
    }

    fn install(
        &mut self,
        ch: &Channel,
        disk: u32,
        mode: Mode,
        confirm: &str,
        settings: &str,
        account: &installplan::proto::AccountInput,
    ) -> Reply {
        if self.done {
            return fail(Error::BadState, "instalace už proběhla - restartuj počítač");
        }
        let (plan, target) = match self.plan_for(disk, mode) {
            Ok(x) => x,
            Err(r) => return r,
        };
        // Potvrzeni: u celeho disku opsany nazev (presne).
        if let installplan::Confirm::TypeName(name) = &plan.confirm
            && confirm.trim() != name
        {
            return fail(Error::AccessDenied, format!("pro smazání disku je třeba opsat jeho název \"{name}\""));
        }
        let fresh = plan.format_data;
        if fresh && !installplan::valid_user_name(&account.name) {
            return fail(Error::InvalidArgs, "jméno účtu: malá písmena a-z, číslice, - a _, začíná písmenem, nejvýš 32 znaků");
        }
        if fresh && !installplan::password_ok(&account.password) {
            return fail(Error::InvalidArgs, "heslo: aspoň 10 znaků nebo 4 slova");
        }
        let Some(live) = self.disks.iter().find(|d| Disk::is_live(d.table().as_ref())) else {
            return fail(Error::NotFound, "instalace jde jen ze živého USB (flashka z cargo xtask usb)");
        };
        println!("installd: instaluji na '{}' ({}), {}", target.model, mode.key(), preview_text(&plan).replace('\n', "; "));
        let settings = Settings::parse(settings);
        // Uloha v prehledu uloh (Spravce uloh, terminal) se stejnym prubehem.
        let tasks = &self.tasks;
        tasks.start(&target.model);
        let total = installplan::PROGRESS.len() as u32;
        let mut progress = |step: u32, text: &str| {
            println!("installd: krok {}: {text}", step + 1);
            let _ = ch.write(&Reply::Progress { step, text: text.to_string() }.encode(), Vec::new());
            tasks.progress(step, total);
        };
        let mut cx = write::Ctx { target, live, progress: &mut progress, qemu_kdf: self.qemu_kdf };
        let reply = match write::install(&mut cx, &plan, &settings, account) {
            Ok(summary) => {
                tasks.finish(Ok(summary.lines().next().unwrap_or("hotovo")));
                Reply::Done(summary)
            }
            Err(e) => {
                println!("installd: instalace selhala: {e}");
                let text = format!("instalace selhala: {e}. Jiné oddíly na disku se nezměnily.");
                tasks.finish(Err((e.code(), &text)));
                fail(e, text)
            }
        };
        self.done = matches!(reply, Reply::Done(_));
        reply
    }
}

/// Rozlozeni klavesnice z `keymap` (stejny zdroj jako sluzba vstupu).
fn keyboards() -> Reply {
    let list = keymap::LayoutId::ALL
        .into_iter()
        .map(|id| {
            let l = keymap::layout(id);
            KeyboardEntry {
                name: String::from(id.name()),
                description: String::from(id.description()),
                rows: (0..keymap::PREVIEW_ROWS.len()).map(|r| l.preview_row(r).collect()).collect(),
            }
        })
        .collect();
    Reply::Keyboards(list)
}
