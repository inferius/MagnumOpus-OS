//! sshtools: klient SSH v jednom programu, chovani podle jmena (`argv[0]`):
//! `ssh`, `scp`, `sftp` (`docs/cs/ssh.md`, kapitola Klient). V initfs pod
//! kazdym jmenem s vlastnim manifestem. Spousti je shell sluzbou exec;
//! init jim jako jedinym da kanal podpisu u sluzby uctu (soukromy klic
//! program nema).
//!
//! Prikazy a prepinace anglicky, hlaseni cesky. Kod 255 = chyba spojeni
//! nebo prihlaseni (jako OpenSSH), jinak kod prikazu na serveru.

#![feature(magnumopus_runtime, rustc_private)]

mod session;
mod transfer;

use std::io::Write;
use std::sync::mpsc;

use session::{Sess, Target};
use ssh::client::{CState, Request};

fn main() {
    let mut args = std::env::args();
    let me = args.next().unwrap_or_default();
    let name = me.rsplit('/').next().unwrap_or(&me).to_string();
    let rest: Vec<String> = args.collect();
    let code = match name.as_str() {
        "ssh" => ssh(&rest),
        "scp" => transfer::scp(&rest),
        "sftp" => transfer::sftp(&rest),
        _ => {
            eprintln!("sshtools: spusťte jako ssh, scp nebo sftp");
            2
        }
    };
    std::process::exit(code);
}

/// Port z `--port N` / `-p N` / `-P N`; zbytek argumentu.
pub fn port_and_rest(args: &[String]) -> Result<(u16, Vec<String>), String> {
    let mut port = 22;
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if rest.is_empty() && matches!(a.as_str(), "--port" | "-p" | "-P") {
            let v = it.next().ok_or("přepínač --port čeká číslo")?;
            port = v.parse().ok().filter(|p| *p != 0).ok_or_else(|| format!("{v}: port 1 až 65535"))?;
        } else if let Some(v) = a.strip_prefix("--port=") {
            port = v.parse().ok().filter(|p| *p != 0).ok_or_else(|| format!("{v}: port 1 až 65535"))?;
        } else {
            rest.push(a.clone());
        }
    }
    Ok((port, rest))
}

/// `ssh [--port N] [uzivatel@]server [prikaz...]`
fn ssh(args: &[String]) -> i32 {
    const USE: &str = "použití: ssh [--port N] [uživatel@]server [příkaz...]";
    let (port, rest) = match port_and_rest(args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("ssh: {e}\n{USE}");
            return 2;
        }
    };
    let Some((dest, cmd)) = rest.split_first() else {
        eprintln!("{USE}");
        return 2;
    };
    let t = match Target::parse(dest, port) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("ssh: {e}");
            return 2;
        }
    };
    // Bez prikazu shell serveru po radcich (surovy terminal zatim neni).
    let req = if cmd.is_empty() { Request::Shell { pty: None } } else { Request::Exec(cmd.join(" ")) };
    let mut s = match Sess::open(&t, req) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ssh: {e}");
            return 255;
        }
    };
    // Vstup ve vlakne (cteni stdin blokuje); konec vstupu = EOF serveru.
    let (tx, rx) = mpsc::channel::<Option<Vec<u8>>>();
    std::thread::spawn(move || {
        loop {
            match session::read_line() {
                Some(l) => {
                    let mut d = l.into_bytes();
                    d.push(b'\n');
                    if tx.send(Some(d)).is_err() {
                        return;
                    }
                }
                None => {
                    let _ = tx.send(None);
                    return;
                }
            }
        }
    });
    let mut stdin_open = true;
    loop {
        if let Err(e) = s.pump() {
            eprintln!("ssh: {e}");
            return 255;
        }
        while let Some((err, d)) = s.take() {
            let _ = if err { std::io::stderr().write_all(&d) } else { std::io::stdout().write_all(&d) };
        }
        let _ = std::io::stdout().flush();
        while stdin_open {
            match rx.try_recv() {
                Ok(Some(d)) => {
                    if s.write_all(&d).is_err() {
                        stdin_open = false;
                    }
                }
                Ok(None) | Err(mpsc::TryRecvError::Disconnected) => {
                    stdin_open = false;
                    let _ = s.conn.send_eof();
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        match s.conn.state() {
            CState::Closed(code) if !s.pending() => return code.map_or(255, |c| c as i32),
            CState::Failed(_) => return 255,
            _ => {}
        }
    }
}
