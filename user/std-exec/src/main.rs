//! std-exec: test spousteni programu ze shellu (sluzba exec initu,
//! `user/init/src/tests/exec.rs`).
//!
//! - `std-exec net`: `sit: ano` / `sit: ne` (zda proces dostal relaci site),
//! - `std-exec echo`: stdin na stdout po radcich az do konce vstupu,
//! - `std-exec exit <n>`: vypise `konec <n>` (stderr) a skonci s kodem n,
//! - `std-exec cwd`: pracovni slozka,
//! - `std-exec write <soubor>`: zapise `std-exec` do souboru (identita FS).

use std::io::{BufRead, Write};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("net") => {
            let ok = std::net::UdpSocket::bind("0.0.0.0:0").is_ok();
            println!("sit: {}", if ok { "ano" } else { "ne" });
            0
        }
        Some("echo") => {
            let stdin = std::io::stdin();
            let mut out = std::io::stdout().lock();
            for line in stdin.lock().lines() {
                let Ok(line) = line else { break };
                let _ = writeln!(out, "{line}");
                let _ = out.flush();
            }
            0
        }
        Some("exit") => {
            let n: i32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
            eprintln!("konec {n}");
            n
        }
        Some("cwd") => {
            println!("{}", std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default());
            0
        }
        Some("write") => match args.get(1).map(|p| std::fs::write(p, b"std-exec")) {
            Some(Ok(())) => {
                println!("zapsano");
                0
            }
            Some(Err(e)) => {
                eprintln!("zapis: {e}");
                3
            }
            None => 2,
        },
        _ => {
            eprintln!("std-exec: net | echo | exit <n> | cwd | write <soubor>");
            2
        }
    };
    std::process::exit(code);
}
