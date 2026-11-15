//! Dynamicke linkovani (DL-2, `docs/cs/dynamicke-linkovani.md`): program PIE
//! v C (`user/dltest`) se zavadecem `ld.so` a dvema knihovnami z initfs.

use alloc::vec::Vec;
use core::time::Duration;

use libos::initfs::Initfs;
use libos::{Error, Startup, time};

use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("dynamicke linkovani: PIE s knihovnami (relokace, TLS, inicializatory)", program_with_libraries),
    ("dynamicke linkovani: ASLR - dva behy, ruzne baze", aslr_differs),
    ("dynamicke linkovani: chybejici knihovna -> kod 127", missing_library),
    ("dynamicke linkovani: vlakna libos s TLS knihoven, dl_iterate_phdr, r_debug, fini", rust_program),
    ("dynamicke linkovani: libd sdili sablony knihoven mezi procesy", libd_shares),
    ("dynamicke linkovani: rezerva statickeho TLS z voleb spusteni (manifest)", tls_reserve),
    ("dynamicke linkovani: rezervace adres pred zavedenim (AT_MO_RESERVE)", address_reserve),
    ("dynamicke linkovani: obnova po padu libd pres listky (spawn i dlopen)", libd_recovery),
    ("dynamicke linkovani: pad libd pred prevzetim listku (ADOPT) - dite obnovi dlopen", libd_crash_before_adopt),
    ("dynamicke linkovani: rezie spawnu a propustnost initu (davka 200, 4 x 50 soubezne)", spawn_batch),
];

fn deadline() -> Option<u64> {
    Some(time::deadline_after(Duration::from_secs(20)))
}

/// Spusti program z initfs a vrati jeho navratovy kod.
fn run(s: &Startup, name: &str) -> Result<i64, alloc::string::String> {
    let fs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = fs.find(name).ok_or_else(|| alloc::format!("{name} neni v initfs"))?;
    let (p, _ch) = libos::spawn(name, image, Vec::new()).map_err(|e| fail("spusteni", e))?;
    p.wait_exit(deadline()).map_err(|e| fail("konec programu", e))
}

fn program_with_libraries(s: &Startup) -> TestResult {
    let code = run(s, "dltest")?;
    // Bity 0-7: chyby v programu (viz user/dltest/main.c).
    check_eq!(code & 0xFF, 0, "kontroly programu (1 volani, 2 ukazatel na funkci, 4 inicializator, 8/16 TLS knihoven, 32/64 TLS programu)");
    Ok(())
}

fn aslr_differs(s: &Startup) -> TestResult {
    let a = run(s, "dltest")?;
    let b = run(s, "dltest")?;
    check!(a & 0xFF == 0 && b & 0xFF == 0, "oba behy bez chyby");
    check!(a >> 8 != b >> 8, alloc::format!("stejna baze programu v obou behech ({:#x})", a >> 8));
    Ok(())
}

fn missing_library(s: &Startup) -> TestResult {
    check_eq!(run(s, "dltest-missing")?, abi::auxv::EXIT_LOAD_FAILED, "kod zavadece pri chybejici knihovne");
    Ok(())
}

fn rust_program(s: &Startup) -> TestResult {
    let code = run(s, "dltest-rs")?;
    // Bity viz user/dltest-rs/src/main.rs (E_*).
    check_eq!(
        code,
        0,
        "dltest-rs: 1 hlavni vlakno, 2 TLS vlaken, 4 tabulka zavadece, 8 dl_iterate_phdr, 16 r_debug, 32 fini, 64 vlakna, \
         128 dl* zaklad/dladdr, 256 pecet kodu/RELRO, 512 dlopen s TLS, 1024 line TLS ve vlaknech, 2048 chyba rezervy"
    );
    Ok(())
}

/// Dva behy `dltest`: druhy nevytvori zadnou novou sablonu (libdltest,
/// libdltest2 uz libd ma; ld.so si spawn drzi sam), jen zasahy mezipameti.
fn libd_shares(s: &Startup) -> TestResult {
    run(s, "dltest")?;
    let a = libos::libd::stats().map_err(|e| fail("statistika libd", e))?;
    check_eq!(run(s, "dltest")? & 0xFF, 0, "druhy beh dltest");
    let b = libos::libd::stats().map_err(|e| fail("statistika libd", e))?;
    check_eq!(b.templates, a.templates, "zadna nova sablona pri druhem behu");
    // libdltest a libdltest2 (ld.so ma spawn v mezipameti procesu).
    check!(b.hits >= a.hits + 2, alloc::format!("zasahy mezipameti {} -> {}", a.hits, b.hits));
    Ok(())
}

/// `dltest-dl` nahraje pres `dlopen` knihovnu se 4 KiB TLS initial-exec:
/// s vychozi rezervou (2 KiB) odmitnuto s radou v `dlerror` (kod 1),
/// s rezervou 8 KiB z voleb spusteni (`[app] static_tls_reserve`) uspech.
fn tls_reserve(s: &Startup) -> TestResult {
    check_eq!(run(s, "dltest-dl")?, 1, "vychozi rezerva: odmitnuto s radou static_tls_reserve");
    let fs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = fs.find("dltest-dl").ok_or("dltest-dl neni v initfs")?;
    let job = libos::job::current().map_err(|e| fail("job", e))?;
    let opts = libos::spawn::Options { static_tls_reserve: Some(8192), ..Default::default() };
    let (p, _ch) = libos::spawn::spawn_with(&job, "dltest-dl", image, Vec::new(), opts).map_err(|e| fail("spusteni", e))?;
    let code = p.wait_exit(deadline()).map_err(|e| fail("konec programu", e))?;
    check_eq!(code, 0, "rezerva 8 KiB: knihovna nahrana a TLS sedi");
    Ok(())
}

/// Rozsahy Windows vrstvy (dolni 2 GiB, obvykla baze PE) rezervovane
/// spawnem: program je najde v auxv a v pameti jsou cele jako rezervace;
/// chybne rozsahy spawn odmitne.
fn address_reserve(s: &Startup) -> TestResult {
    let fs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = fs.find("dltest-rs").ok_or("dltest-rs neni v initfs")?;
    let job = libos::job::current().map_err(|e| fail("job", e))?;
    let spawn = |reserve: &[(u64, u64)]| {
        let mut opts = libos::spawn::Options::default();
        opts.reserve[..reserve.len()].copy_from_slice(reserve);
        libos::spawn::spawn_with(&job, "dltest-rs", image, Vec::new(), opts)
    };
    let (p, _ch) = spawn(&[(0x1_0000, 0x7ffe_0000), (0x1_4000_0000, 0x1000_0000)]).map_err(|e| fail("spusteni", e))?;
    let code = p.wait_exit(deadline()).map_err(|e| fail("konec programu", e))?;
    check_eq!(code & 0xFFFF, 0, "dltest-rs s rezervacemi bez chyby (4096 = rezervace nesedi)");
    check_eq!(code >> 16, 2, "program videl obe rezervace");
    check_eq!(spawn(&[(0x1_0000, 0x2_0000), (0x2_0000, 0x1_0000)]).err(), Some(Error::InvalidArgs), "prekryv");
    check_eq!(spawn(&[(0x1_0800, 0x1_0000)]).err(), Some(Error::InvalidArgs), "nezarovnany zacatek");
    let st = fs.find("echo").ok_or("echo neni v initfs")?;
    let mut opts = libos::spawn::Options::default();
    opts.reserve[0] = (0x1_0000, 0x1_0000);
    check_eq!(
        libos::spawn::spawn_with(&job, "echo", st, Vec::new(), opts).err(),
        Some(Error::NotSupported),
        "staticky program rezervace nedostane"
    );
    Ok(())
}

/// Spoustec `dlspawn` (staticky, s relaci libd a listkem) s kanalem pro
/// vysledek; posle mu pocet programu.
fn spawner(s: &Startup, n: u32) -> Result<(libos::Process, libos::Channel), alloc::string::String> {
    let (p, c) = spawner_idle(s)?;
    c.write(&n.to_le_bytes(), Vec::new()).map_err(|e| fail("zadani", e))?;
    Ok((p, c))
}

/// Spoustec bez zadani.
fn spawner_idle(s: &Startup) -> Result<(libos::Process, libos::Channel), alloc::string::String> {
    let fs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = fs.find("dlspawn").ok_or("dlspawn neni v initfs")?;
    let initfs = s.initfs.as_ref().ok_or("init nema initfs")?;
    let ro = initfs
        .handle()
        .duplicate(Some(libos::Rights::READ | libos::Rights::MAP | libos::Rights::BASIC))
        .map_err(|e| fail("initfs", e))?;
    let (ours, theirs) = libos::Channel::create().map_err(|e| fail("kanal", e))?;
    let handles = alloc::vec![
        libos::spawn::StartHandle::new(abi::bootstrap::kind::INITFS, 0, ro),
        libos::spawn::StartHandle::user(theirs.into_handle()),
    ];
    let job = libos::job::current().map_err(|e| fail("job", e))?;
    let opts = libos::spawn::Options { libd: true, ..Default::default() };
    let (p, _ch) = libos::spawn::spawn_with(&job, "dlspawn", image, handles, opts).map_err(|e| fail("spusteni", e))?;
    Ok((p, ours))
}

/// Ceka na vysledky spoustecu a pritom ridi supervisor (`NARROW`
/// a `RECONNECT` na listcich obsluhuje init).
fn collect(s: &Startup, chans: &[libos::Channel], limit: Duration) -> Result<Vec<[u64; 7]>, alloc::string::String> {
    let end = time::deadline_after(limit);
    let mut out: Vec<Option<[u64; 7]>> = chans.iter().map(|_| None).collect();
    while out.iter().any(Option::is_none) {
        if time::now() > end {
            return Err("spoustece nedobehly vcas".into());
        }
        crate::supervisor::SUPERVISOR.lock().step(s, Some(time::deadline_after(Duration::from_millis(2))));
        for (i, c) in chans.iter().enumerate() {
            if out[i].is_none()
                && let Ok(m) = c.try_read()
            {
                let mut v = [0u64; 7];
                for (k, w) in m.data.chunks_exact(8).take(7).enumerate() {
                    v[k] = u64::from_le_bytes(w.try_into().unwrap_or([0; 8]));
                }
                out[i] = Some(v);
            }
        }
    }
    Ok(out.into_iter().flatten().collect())
}

/// Soucty vysledku a radek do logu: prumery v us a podil `NARROW` na
/// spawnu programu se zavadecem.
fn report(what: &str, rs: &[[u64; 7]], wall: u64) -> (u64, u64, u64) {
    let mut t = [0u64; 7];
    for r in rs {
        for k in 0..7 {
            t[k] += r[k];
        }
    }
    let n = t[0].max(1);
    let (dyn_us, narrow_us, static_us, rtt_us) = (t[1] / n / 1000, t[3] / n / 1000, t[4] / n / 1000, t[5] / n / 1000);
    libos::println!(
        "init: mereni spawnu ({what}): {} programu za {} ms; spawn se zavadecem {dyn_us} us (z toho relace a listek pro dite {narrow_us} us = {} %), beh {} us; spawn staticky {static_us} us; obeh libd {rtt_us} us; chyby {}",
        t[0],
        wall / 1_000_000,
        narrow_us * 100 / dyn_us.max(1),
        t[2] / n / 1000,
        t[6]
    );
    (t[6], narrow_us, dyn_us)
}

/// 200 programu se zavadecem za sebou z jednoho spoustece a 4 x 50
/// soubezne: vse projde a log ukaze rezii `NARROW` (init) proti spawnu.
fn spawn_batch(s: &Startup) -> TestResult {
    let t0 = time::now();
    let (_p, c) = spawner(s, 200)?;
    let one = collect(s, core::slice::from_ref(&c), Duration::from_secs(240))?;
    let (err1, _, _) = report("1 spoustec x 200", &one, time::now() - t0);
    check_eq!(err1, 0, "chyby v davce 200");
    let t1 = time::now();
    let mut chans = Vec::new();
    let mut procs = Vec::new();
    for _ in 0..4 {
        let (p, c) = spawner(s, 50)?;
        procs.push(p);
        chans.push(c);
    }
    let four = collect(s, &chans, Duration::from_secs(240))?;
    let (err4, _, _) = report("4 spoustece x 50 soubezne", &four, time::now() - t1);
    check_eq!(err4, 0, "chyby v soubeznych davkach");
    Ok(())
}

/// Pad libd: spoustec, ktery bezel pred padem, po restartu spusti dalsi
/// programy se zavadecem (relace pres `RECONNECT` na listku, deti pres
/// `NARROW`); init spousti dal z noveho korene. Klient libd v zavadeci
/// (`dlopen`) je tentyz kod libos (`libos::libd`).
fn libd_recovery(s: &Startup) -> TestResult {
    let gen1 = crate::supervisor::SUPERVISOR.lock().generation("libd").ok_or("libd neni v registru")?;
    // Spoustec s listkem, ktery prezije pad.
    let (_p, c) = spawner(s, 1)?;
    collect(s, core::slice::from_ref(&c), Duration::from_secs(60))?;
    kill_libd(s, true)?;
    check!(crate::supervisor::SUPERVISOR.lock().generation("libd").is_some_and(|g| g > gen1), "nova generace libd");
    // Init (korenove spojeni od adapteru) spousti dal.
    check_eq!(run(s, "dltest")? & 0xFF, 0, "dltest z initu po restartu libd");
    check_eq!(run(s, "dltest-rs")?, 0, "dltest-rs (dlopen) po restartu libd");
    // Stary spoustec (relace na mrtve instanci, listek zije): dalsi davka.
    c.write(&3u32.to_le_bytes(), Vec::new()).map_err(|e| fail("zadani", e))?;
    let r = collect(s, core::slice::from_ref(&c), Duration::from_secs(60))?;
    check_eq!(r.first().map_or(1, |v| v[6]), 0, "spoustec z doby pred padem libd");
    check_eq!(r.first().map_or(0, |v| v[0]), 3, "pocet programu po padu");
    Ok(())
}

/// Zabije libd a pocka (s rizenim supervisoru) na novou generaci.
fn kill_libd(s: &Startup, wait_restart: bool) -> TestResult {
    // Zamerne ukonceni: pod --repeat se nepocita do limitu padu supervisoru.
    let gen1 = crate::supervisor::SUPERVISOR.lock().kill_for_test("libd").map_err(|e| fail("ukonceni libd", e))?;
    if !wait_restart {
        return Ok(());
    }
    let end = time::deadline_after(Duration::from_secs(15));
    loop {
        let g = {
            let mut sup = crate::supervisor::SUPERVISOR.lock();
            sup.step(s, Some(time::deadline_after(Duration::from_millis(20))));
            if sup.is_running("libd") { sup.generation("libd") } else { None }
        };
        if g.is_some_and(|g| g > gen1) {
            return Ok(());
        }
        if time::now() > end {
            return Err("libd se po padu nespustila".into());
        }
    }
}

/// Zprava od spoustece (supervisor se pritom ridi).
fn reply_of(s: &Startup, c: &libos::Channel, limit: Duration) -> Result<Vec<u8>, alloc::string::String> {
    let end = time::deadline_after(limit);
    loop {
        if let Ok(m) = c.try_read() {
            return Ok(m.data);
        }
        if time::now() > end {
            return Err("spoustec neodpovedel".into());
        }
        crate::supervisor::SUPERVISOR.lock().step(s, Some(time::deadline_after(Duration::from_millis(2))));
    }
}

/// Poradi: spoustec spusti dite (relace pres `Connect`, listek pres `ADOPT`
/// - oboji bez cekani), init libd zabije **driv, nez supervisor `ADOPT`
/// zpracuje** (supervisor se mezitim neridi), a dite pak udela `dlopen`:
/// zavadec posle `RECONNECT` na listek, ktery supervisor jeste nezna;
/// pozadavek pocka v kanalu a po prevzeti listku a restartu libd projde.
fn libd_crash_before_adopt(s: &Startup) -> TestResult {
    let (_p, c) = spawner_idle(s)?;
    c.write(b"hold", Vec::new()).map_err(|e| fail("hold", e))?;
    // Bez rizeni supervisoru: ADOPT spoustece zustane nezpracovany.
    let r = c.read(Some(time::deadline_after(Duration::from_secs(30)))).map_err(|e| fail("odpoved hold", e))?;
    check_eq!(r.data.as_slice(), b"ok".as_slice(), "dite nahralo knihovny");
    kill_libd(s, false)?;
    c.write(b"go", Vec::new()).map_err(|e| fail("go", e))?;
    let r = reply_of(s, &c, Duration::from_secs(60))?;
    let code = i64::from_le_bytes(r.get(..8).and_then(|b| b.try_into().ok()).ok_or("kratka odpoved")?);
    check_eq!(code, 0, "dlopen po padu libd pred prevzetim listku (0 = ok, 2 = chyba dlopen)");
    Ok(())
}
