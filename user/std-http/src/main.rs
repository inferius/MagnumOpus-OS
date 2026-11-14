//! std-http: klient HTTP nad knihovnou `ureq` z crates.io (bez uprav)
//! - overeni `std::net` a vlaken v praxi (docs/cs/rust-std.md, Porty).
//!
//! Promenna `BASE=http://<adresa:port>` = testovaci server xtasku
//! (`xtask/src/webhelper.rs`). Kontroly: `GET /hello`, `POST /echo`,
//! 4 vlakna soubezne stahuji `/stream?n=...` a overi vzor bajtu.
//! Kazda kontrola vypise `ok <jmeno>`, posledni radek `std-http: hotovo`.

use std::io::Read;
use std::thread;

type Check = Result<(), String>;

fn ensure(cond: bool, msg: impl Into<String>) -> Check {
    if cond { Ok(()) } else { Err(msg.into()) }
}

fn base() -> Result<String, String> {
    std::env::var("BASE").map_err(|_| String::from("chybi BASE"))
}

/// Vzor tela `/stream` (stejny jako `webhelper::pattern`).
fn pattern(i: usize) -> u8 {
    (i.wrapping_mul(7) ^ (i >> 9)) as u8
}

fn hello() -> Check {
    let body = ureq::get(format!("{}/hello", base()?))
        .call()
        .map_err(|e| format!("GET /hello: {e}"))?
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("telo: {e}"))?;
    ensure(body == "ahoj z hostitele", format!("telo {body:?}"))
}

fn echo() -> Check {
    let mut r = ureq::post(format!("{}/echo", base()?)).send("telo pozadavku").map_err(|e| format!("POST: {e}"))?;
    let method = r.headers().get("x-method").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let body = r.body_mut().read_to_string().map_err(|e| e.to_string())?;
    ensure(method == "POST" && body == "telo pozadavku", format!("{method} {body:?}"))
}

fn parallel() -> Check {
    const N: usize = 256 * 1024;
    let base = base()?;
    let handles: Vec<_> = (0..4)
        .map(|t| {
            let url = format!("{base}/stream?n={}", N + t);
            thread::spawn(move || -> Check {
                let mut r = ureq::get(&url).call().map_err(|e| format!("vlakno {t}: {e}"))?;
                let mut data = Vec::new();
                r.body_mut()
                    .with_config()
                    .limit(4 << 20)
                    .reader()
                    .read_to_end(&mut data)
                    .map_err(|e| format!("vlakno {t} cteni: {e}"))?;
                ensure(data.len() == N + t, format!("vlakno {t}: {} B", data.len()))?;
                let bad = data.iter().enumerate().position(|(i, b)| *b != pattern(i));
                ensure(bad.is_none(), format!("vlakno {t}: spatny bajt na {bad:?}"))
            })
        })
        .collect();
    for h in handles {
        h.join().map_err(|_| "vlakno spadlo")??;
    }
    println!("soubezne: 4 x {} KiB", N / 1024);
    Ok(())
}

fn main() {
    let checks: [(&str, fn() -> Check); 3] = [("hello", hello), ("echo", echo), ("parallel", parallel)];
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
    println!("std-http: hotovo");
}
