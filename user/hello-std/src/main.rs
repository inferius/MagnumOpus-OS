//! hello-std: prvni aplikace se `std` pro MagnumOpus.
//!
//! Overuje zakladni casti std nad libos (docs/cs/rust-std.md, faze 1):
//! argumenty a prostredi, stdout/stderr, cas, nahodu (`HashMap`), vlakna
//! s `Mutex`, `Condvar` a `thread_local!`, soubory (vytvoreni, zapis,
//! cteni, vypis adresare, metadata, prejmenovani, smazani).
//!
//! Test v QEMU (`user/init/src/tests/std_app.rs`) ho spusti s argumenty,
//! promennymi `HELLO` a `HELLO_DIR` (zapisovatelny adresar v relaci FS)
//! a cte stdout. Kazda kontrola vypise `ok <jmeno>`, chyba `FAIL <jmeno>:
//! <popis>` na stderr a nenulovy navratovy kod. Posledni radek uspechu je
//! `hello-std: hotovo`.

use std::cell::Cell;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

/// Vysledek jedne kontroly.
type Check = Result<(), String>;

fn ensure(cond: bool, msg: impl Into<String>) -> Check {
    if cond { Ok(()) } else { Err(msg.into()) }
}

fn args_env() -> Check {
    let args: Vec<String> = std::env::args().collect();
    ensure(args.len() >= 3, format!("malo argumentu: {args:?}"))?;
    ensure(args[1] == "alfa" && args[2] == "beta", format!("argumenty: {args:?}"))?;
    ensure(std::env::var("HELLO").as_deref() == Ok("svet"), "promenna HELLO")?;
    // SAFETY: test bezi v jedinem vlakne (vlakna jeste nebezi).
    unsafe { std::env::set_var("DALSI", "1") };
    ensure(std::env::var("DALSI").as_deref() == Ok("1"), "set_var")?;
    ensure(std::env::vars().any(|(k, _)| k == "HELLO"), "vars()")?;
    println!("argumenty: {}", args[1..].join(" "));
    Ok(())
}

fn time() -> Check {
    let start = Instant::now();
    thread::sleep(Duration::from_millis(20));
    let el = start.elapsed();
    ensure(el >= Duration::from_millis(20), format!("sleep 20 ms trval {el:?}"))?;
    // Realny cas je po roce 2020 (hodiny z RTC/UEFI).
    let since = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_err(|e| e.to_string())?;
    ensure(since.as_secs() > 1_577_836_800, format!("SystemTime {since:?}"))
}

fn hashmap() -> Check {
    let mut m = HashMap::new();
    for i in 0..1000u32 {
        m.insert(i, i * 2);
    }
    ensure(m.len() == 1000 && m[&500] == 1000, "HashMap")
}

thread_local! {
    static PER_THREAD: Cell<u64> = const { Cell::new(0) };
}

fn threads() -> Check {
    const THREADS: u64 = 4;
    const ROUNDS: u64 = 10_000;
    let counter = Arc::new(Mutex::new(0u64));
    let ready = Arc::new((Mutex::new(0u64), Condvar::new()));
    let mut handles = Vec::new();
    for t in 0..THREADS {
        let counter = counter.clone();
        let ready = ready.clone();
        handles.push(thread::spawn(move || {
            for _ in 0..ROUNDS {
                *counter.lock().unwrap() += 1;
                PER_THREAD.with(|c| c.set(c.get() + 1));
            }
            let (lock, cv) = &*ready;
            *lock.lock().unwrap() += 1;
            cv.notify_all();
            // TLS je per vlakno: kazde napocitalo jen svoje.
            PER_THREAD.with(Cell::get) + t * 1_000_000
        }));
    }
    // Pockat na vsechna vlakna pres Condvar (overi futex wait/wake).
    {
        let (lock, cv) = &*ready;
        let guard = cv
            .wait_timeout_while(lock.lock().unwrap(), Duration::from_secs(30), |n| *n < THREADS)
            .unwrap()
            .0;
        ensure(*guard == THREADS, format!("Condvar: hotovo {} z {THREADS}", *guard))?;
    }
    for (t, h) in handles.into_iter().enumerate() {
        let own = h.join().map_err(|_| "vlakno spadlo")?;
        ensure(own == ROUNDS + t as u64 * 1_000_000, format!("TLS vlakna {t}: {own}"))?;
    }
    let total = *counter.lock().unwrap();
    ensure(total == THREADS * ROUNDS, format!("Mutex: {total}"))?;
    ensure(PER_THREAD.with(Cell::get) == 0, "TLS hlavniho vlakna")?;
    let cpus = thread::available_parallelism().map_err(|e| e.to_string())?;
    println!("vlakna: {THREADS} x {ROUNDS}, CPU {cpus}");
    Ok(())
}

fn files() -> Check {
    let base = PathBuf::from(std::env::var("HELLO_DIR").map_err(|_| "chybi HELLO_DIR")?);
    let dir = base.join("hello-std");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir(&dir).map_err(|e| format!("create_dir: {e}"))?;
    let path = dir.join("pozdrav.txt");
    let text = "Ahoj ze std!\n".repeat(1000);
    fs::write(&path, &text).map_err(|e| format!("write: {e}"))?;
    let back = fs::read_to_string(&path).map_err(|e| format!("read: {e}"))?;
    ensure(back == text, format!("obsah: {} B misto {}", back.len(), text.len()))?;
    let meta = fs::metadata(&path).map_err(|e| format!("metadata: {e}"))?;
    ensure(meta.is_file() && meta.len() == text.len() as u64, format!("metadata: {meta:?}"))?;

    // Pozice, dopsani, zkraceni.
    let mut f = fs::OpenOptions::new().read(true).write(true).open(&path).map_err(|e| format!("open: {e}"))?;
    f.seek(SeekFrom::Start(5)).map_err(|e| e.to_string())?;
    let mut five = [0u8; 4];
    f.read_exact(&mut five).map_err(|e| format!("read_exact: {e}"))?;
    ensure(&five == b"ze s", format!("seek+read: {five:?}"))?;
    f.set_len(4).map_err(|e| format!("set_len: {e}"))?;
    drop(f);
    let mut a = fs::OpenOptions::new().append(true).open(&path).map_err(|e| format!("append: {e}"))?;
    a.write_all(b"!!").map_err(|e| e.to_string())?;
    drop(a);
    ensure(fs::read(&path).map_err(|e| e.to_string())? == b"Ahoj!!", "set_len + append")?;

    // Relativni cesta vuci pracovnimu adresari.
    std::env::set_current_dir(&dir).map_err(|e| format!("set_current_dir: {e}"))?;
    fs::write("druhy.txt", b"2").map_err(|e| format!("relativni zapis: {e}"))?;
    fs::rename("druhy.txt", "treti.txt").map_err(|e| format!("rename: {e}"))?;
    let mut names: Vec<String> = fs::read_dir(".")
        .map_err(|e| format!("read_dir: {e}"))?
        .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    names.sort();
    ensure(names == ["pozdrav.txt", "treti.txt"], format!("read_dir: {names:?}"))?;
    ensure(
        fs::read("neni.txt").map_err(|e| e.kind()) == Err(std::io::ErrorKind::NotFound),
        "NotFound pro chybejici soubor",
    )?;
    std::env::set_current_dir(&base).map_err(|e| e.to_string())?;
    fs::remove_dir_all(&dir).map_err(|e| format!("remove_dir_all: {e}"))?;
    ensure(!dir.exists(), "adresar po smazani existuje")?;
    println!("soubory: {} B zapsano a precteno", text.len());
    Ok(())
}

fn main() {
    let checks: [(&str, fn() -> Check); 5] = [
        ("args_env", args_env),
        ("time", time),
        ("hashmap", hashmap),
        ("threads", threads),
        ("files", files),
    ];
    let mut failed = 0;
    for (name, check) in checks {
        match check() {
            Ok(()) => println!("ok {name}"),
            Err(e) => {
                eprintln!("FAIL {name}: {e}");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        std::process::exit(1);
    }
    println!("hello-std: hotovo");
}
