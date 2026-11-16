//! Obsluha `LAUNCH` a `LOOKUP`: hledani programu, manifest, job, sit
//! a spusteni potomka.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use fsproto::FsClient;
use fsproto::v2::{access, share};
use launchproto::{Command, Launch, Reader, Registry, Role, SYSTEM_BIN, Source, Writer, flags, is_bare_name};
use libos::abi::bootstrap::kind;
use libos::abi::params::{self, tag};
use libos::initfs::Initfs;
use libos::abi::job::jit;
use libos::job::{Job, pool};
use libos::spawn::{Options, Origin, StartHandle};
use libos::{Channel, Error, Handle, Rights, Vmo};
use manifest::Manifest;
use netproto::Net;

use crate::Launchd;

/// Chyba s popisem pro uzivatele.
pub type Fail = (Error, String);

fn fail(e: Error, msg: impl Into<String>) -> Fail {
    (e, msg.into())
}

/// Nejvetsi program a manifest, ktere launchd nacte ze souboru.
const MAX_IMAGE: u64 = 512 << 20;
const MAX_MANIFEST: usize = 64 << 10;

/// Registr prikazu z manifestu v initfs (`[[command]]`).
pub fn registry(fs: &Initfs) -> Registry {
    let mut reg = Registry::new();
    for file in fs.names() {
        let Some(program) = file.strip_suffix(".manifest.toml") else { continue };
        let Some(m) = fs.find(file).and_then(|t| core::str::from_utf8(t).ok()).and_then(|t| Manifest::parse(t).ok()) else {
            continue;
        };
        for c in &m.commands {
            let target = String::from(c.program.as_deref().unwrap_or(program));
            if !reg.add(Command { name: c.name.clone(), source: Source::Initfs(target), app: m.id.clone() }) {
                libos::println!("launchd: prikaz {} uz je registrovany, {} ho neprepise", c.name, m.id);
            }
        }
    }
    reg
}

fn initfs_manifest(fs: &Initfs, program: &str) -> Result<Option<Manifest>, Fail> {
    let Some(text) = fs.find(&format!("{program}.manifest.toml")) else { return Ok(None) };
    let text = core::str::from_utf8(text).map_err(|_| fail(Error::Corrupted, format!("manifest {program}: neni UTF-8")))?;
    Manifest::parse(text).map(Some).map_err(|e| fail(Error::Corrupted, format!("manifest {program}: {e}")))
}

/// Najde zdroj programu (bez nacteni).
fn find(d: &Launchd, program: &str, fs: Option<&FsClient>) -> Result<Source, Fail> {
    if !is_bare_name(program) {
        if !program.starts_with('/') {
            return Err(fail(Error::InvalidArgs, format!("{program}: cesta musi byt absolutni")));
        }
        return Ok(Source::Path(String::from(program)));
    }
    if let Some(c) = d.registry.lookup(program) {
        return Ok(c.source.clone());
    }
    if let Some(fs) = fs {
        let path = format!("{SYSTEM_BIN}/{program}");
        match fs.stat(&path) {
            Ok(_) => return Ok(Source::Path(path)),
            Err(Error::NotFound | Error::AccessDenied) => {}
            Err(e) => return Err(fail(e, format!("{path}"))),
        }
    }
    if d.initfs.find(program).is_some() {
        return Ok(Source::Initfs(String::from(program)));
    }
    Err(fail(Error::NotFound, format!("{program}: neni v registru prikazu, v {SYSTEM_BIN} ani v systemu")))
}

/// `LOOKUP`: kde by se program vzal (zdroj a jestli ma manifest).
pub fn lookup(d: &mut Launchd, program: &str) -> Result<Vec<u8>, Fail> {
    let source = find(d, program, None)?;
    let manifest = match &source {
        Source::Initfs(p) => initfs_manifest(&d.initfs, p)?.is_some(),
        _ => false,
    };
    let mut w = Writer::new();
    source.encode(&mut w);
    w.u8(manifest as u8);
    Ok(w.finish())
}

/// Nacte cely soubor programu s pravem spusteni (relace FS volajiciho).
fn read_program(fs: &FsClient, path: &str) -> Result<Vec<u8>, Fail> {
    let opened = fs
        .open(path, access::READ | access::EXECUTE, share::READ, 0)
        .map_err(|e| fail(e, format!("{path}: nelze otevrit ke spusteni")))?;
    let size = opened.stat.size;
    let r = if opened.stat.kind != fsproto::kind::FILE {
        Err(fail(Error::IsADirectory, format!("{path}: neni soubor")))
    } else if size > MAX_IMAGE {
        Err(fail(Error::OutOfRange, format!("{path}: program je vetsi nez {} MiB", MAX_IMAGE >> 20)))
    } else {
        fs.read_h(opened.handle, 0, size as usize).map_err(|e| fail(e, format!("{path}: cteni")))
    };
    let _ = fs.close(opened.handle);
    r
}

/// Manifest vedle souboru programu (`<cesta>.manifest.toml`), kdyz je.
fn path_manifest(fs: &FsClient, path: &str) -> Result<Option<Manifest>, Fail> {
    let mpath = format!("{path}.manifest.toml");
    let data = match fs.read(&mpath, 0, MAX_MANIFEST) {
        Ok(d) => d,
        Err(Error::NotFound) => return Ok(None),
        Err(e) => return Err(fail(e, format!("{mpath}"))),
    };
    let text = core::str::from_utf8(&data).map_err(|_| fail(Error::Corrupted, format!("{mpath}: neni UTF-8")))?;
    Manifest::parse(text).map(Some).map_err(|e| fail(Error::Corrupted, format!("{mpath}: {e}")))
}

/// Podrizeny job s limity z manifestu (stejne jako `init::apps::job_in`).
///
/// JIT (`docs/cs/sprava-pameti.md`): launchd je brana - `jit::RUN` jen
/// programu s `[app] jit = true` v manifestu; pravo udelovat (`GRANT`) jde
/// po retezu spousteni dal (aplikace s JIT spousti sve procesy pres launchd,
/// napr. Wine). Jadro oboji povoli jen pod jobem s `GRANT` (relace
/// uzivatele, init), takze launchd nic nepridava: bez `GRANT` u volajiciho
/// program bez JIT dostane job bez JIT a program s `jit = true` chybu.
fn child_job(parent: &Job, m: Option<&Manifest>) -> Result<Job, Fail> {
    let job = Job::create(parent).map_err(|e| fail(e, "podrizeny job"))?;
    let want = m.is_some_and(|m| m.jit);
    let bits = jit::GRANT | if want { jit::RUN } else { 0 };
    match job.set_jit(bits) {
        Ok(()) => {}
        Err(Error::AccessDenied) if !want => {}
        Err(e) => return Err(fail(e, "aplikace chce JIT ([app] jit), volajici ho nesmi udelit")),
    }
    libos::sched::set_job_max(&job, libos::sched::Qos::Normal).map_err(|e| fail(e, "trida jobu"))?;
    if let Some(m) = m {
        if let Some(v) = m.limits.memory {
            job.set_limit(pool::USER, v).map_err(|e| fail(e, "limit pameti"))?;
        }
        if let Some(v) = m.limits.kernel_memory {
            job.set_limit(pool::KERNEL, v).map_err(|e| fail(e, "limit pameti jadra"))?;
        }
    }
    Ok(job)
}

/// Blok parametru jako objekt pameti jen pro cteni.
fn params_vmo(block: &[u8]) -> Result<Vmo, Fail> {
    if params::Params::parse(block).is_none() {
        return Err(fail(Error::InvalidArgs, "neplatny blok parametru"));
    }
    let vmo = Vmo::create(block.len() as u64).map_err(|e| fail(e, "blok parametru"))?;
    vmo.write(0, block).map_err(|e| fail(e, "blok parametru"))?;
    let ro = vmo
        .handle()
        .duplicate(Some(Rights::READ | Rights::TRANSFER | Rights::DUPLICATE))
        .map_err(|e| fail(e, "blok parametru"))?;
    Ok(Vmo::from_handle(ro))
}

/// `LAUNCH`: vraci handly odpovedi `[proces, kanal k potomkovi]`.
pub fn launch(d: &mut Launchd, r: &mut Reader, handles: Vec<Handle>) -> Result<Vec<Handle>, Fail> {
    let req = Launch::decode(r).map_err(|e| fail(e, "neplatny pozadavek"))?;
    if handles.len() != req.roles.len() {
        return Err(fail(Error::InvalidArgs, "pocet handlu neodpovida rolim"));
    }
    let mut by_role: Vec<(Role, Handle)> = req.roles.iter().copied().zip(handles).collect();
    let mut take = |role: Role| by_role.iter().position(|(r, _)| *r == role).map(|i| by_role.remove(i).1);
    let parent_job = Job::from_handle(take(Role::Job).ok_or_else(|| fail(Error::InvalidArgs, "chybi job volajiciho"))?);
    let fs = take(Role::Fs).map(|h| FsClient::new(Channel::from_handle(h)));
    let net = take(Role::Net);
    let stdio: Vec<(u32, Handle)> = [(Role::Stdin, tag::STDIN), (Role::Stdout, tag::STDOUT), (Role::Stderr, tag::STDERR)]
        .into_iter()
        .filter_map(|(role, t)| take(role).map(|h| (t, h)))
        .collect();
    // Handly zdedenych fd v poradi zpravy (polozky jsou v bloku parametru).
    let fds: Vec<Handle> = core::iter::from_fn(|| take(Role::Fd)).collect();
    // Relace okna od rodice (role Gui): jen relace i listek spolu; launchd
    // je jen preda (identitu a zuzeni udelal supervisor pri NARROW).
    let gui = match (take(Role::Gui), take(Role::GuiLease)) {
        (Some(s), Some(l)) => Some((s, l)),
        (None, None) => None,
        _ => return Err(fail(Error::InvalidArgs, "role Gui jen s listkem GuiLease")),
    };
    // Relace zvuku od rodice (role Audio): stejne pravidlo.
    let audio = match (take(Role::Audio), take(Role::AudioLease)) {
        (Some(s), Some(l)) => Some((s, l)),
        (None, None) => None,
        _ => return Err(fail(Error::InvalidArgs, "role Audio jen s listkem AudioLease")),
    };
    let tty = take(Role::Tty);
    let agent = take(Role::Agent);
    let initfs_only = req.flags & flags::INITFS_ONLY != 0;
    // Kanal podpisu SSH jen programu, ktery si jmeno nemuze vzit nikdo
    // jiny (initfs).
    if agent.is_some() && !initfs_only {
        return Err(fail(Error::AccessDenied, "role Agent jen s INITFS_ONLY"));
    }

    let source = if initfs_only {
        if !is_bare_name(&req.program) || d.initfs.find(&req.program).is_none() {
            return Err(fail(Error::NotFound, format!("{}: neni v systemu (initfs)", req.program)));
        }
        Source::Initfs(req.program.clone())
    } else {
        find(d, &req.program, fs.as_ref())?
    };
    let (name, image, manifest): (String, Vec<u8>, Option<Manifest>) = match &source {
        Source::Initfs(p) => {
            let image = d.initfs.find(p).ok_or_else(|| fail(Error::NotFound, format!("{p}: chybi v initfs")))?;
            (p.clone(), image.to_vec(), initfs_manifest(&d.initfs, p)?)
        }
        Source::Path(path) => {
            let fs = fs.as_ref().ok_or_else(|| fail(Error::AccessDenied, format!("{path}: bez relace FS nelze cist program")))?;
            let image = read_program(fs, path)?;
            let name = path.rsplit('/').next().unwrap_or(path);
            (String::from(name), image, path_manifest(fs, path)?)
        }
        Source::Volume { volume, path } => {
            // Instalace aplikaci na svazky zatim neni (docs/cs/rust-std.md).
            return Err(fail(Error::NotMounted, format!("{path}: svazek {volume} neni pripojen")));
        }
    };

    // Program ze souboru se zavadecem: zavadec dostane vlastni relaci FS se
    // stejnou identitou a cestami (CONNECT na relaci volajiciho) a cestu
    // programu - knihovny z disku a $ORIGIN cte s pravy aplikace (DL-4c).
    let origin = match (&source, fs.as_ref()) {
        (Source::Path(path), Some(fs)) if libos::spawn::needs_loader(&image) => {
            let own = fs.connect().map_err(|e| fail(e, "relace FS pro zavadec"))?;
            Some(Origin { fs: own.into_channel().into_handle(), path: path.clone() })
        }
        _ => None,
    };
    let job = child_job(&parent_job, manifest.as_ref())?;
    let mut start = alloc::vec![StartHandle::new(kind::PARAMS, 0, params_vmo(&req.params)?.into_handle())];
    for (t, h) in stdio {
        start.push(StartHandle::tagged(t, h));
    }
    for h in fds {
        start.push(StartHandle::tagged(tag::FD, h));
    }
    if let Some((s, l)) = gui {
        start.push(StartHandle::tagged(tag::GUI, s));
        start.push(StartHandle::tagged(tag::GUI_LEASE, l));
    }
    if let Some((s, l)) = audio {
        start.push(StartHandle::tagged(tag::AUDIO, s));
        start.push(StartHandle::tagged(tag::AUDIO_LEASE, l));
    }
    if let Some(h) = tty {
        start.push(StartHandle::tagged(tag::TTY, h));
    }
    if let Some(h) = agent {
        start.push(StartHandle::tagged(tag::SSH_AGENT, h));
    }
    if let Some(fs) = fs {
        start.push(StartHandle::tagged(tag::FS, fs.into_channel().into_handle()));
    }
    if let Some(h) = net {
        let ch = Channel::from_handle(h);
        // Program s manifestem dostane jen prunik site rodice a manifestu
        // (netd zuzuje relaci, nikdy nerozsiri); bez `[network]` nic.
        let ch = match manifest.as_ref().map(|m| &m.network) {
            None if req.flags & flags::NET_NEEDS_MANIFEST != 0 => None,
            None => Some(ch),
            Some(None) => None,
            Some(Some(rule)) => Some(
                Net::new(ch).restrict_with(rule).map_err(|e| fail(e, "zuzeni site podle manifestu"))?.into_channel().map_err(|e| fail(e, "predani site podle manifestu"))?,
            ),
        };
        if let Some(ch) = ch {
            start.push(StartHandle::tagged(tag::NET, ch.into_handle()));
        }
    }
    let own = d.new_client().map_err(|e| fail(e, "kanal ke launchd pro potomka"))?;
    start.push(StartHandle::tagged(tag::LAUNCH, own.into_handle()));

    let mut opts = Options::default();
    if let Some(m) = manifest.as_ref() {
        if let Some(s) = m.limits.stack {
            opts.stack_size = s;
        }
        // Rezerva TLS a rezervace adres jako u aplikaci z initfs
        // (`init::apps`); Windows vrstva (T5).
        opts.static_tls_reserve = m.static_tls_reserve;
        for (slot, r) in opts.reserve.iter_mut().zip(m.reserve.iter()) {
            *slot = *r;
        }
    }
    let (process, _, control) = libos::spawn::spawn_from(&job, &name, &image, start, opts, origin)
        .map_err(|e| fail(e, format!("{name}: spusteni")))?;
    // Job zije s procesem (proces drzi svuj job); handle tu neni potreba.
    drop(job);
    Ok(alloc::vec![process.into_handle(), control.into_handle()])
}
