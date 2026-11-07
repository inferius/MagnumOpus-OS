//! Vkladani vstupu do QEMU pres QMP (QEMU Machine Protocol) pro test vstupu.
//!
//! Test v hostovanem systemu (`user/init/src/tests/input.rs`) vypise do
//! seriove linky znacku [`MARKER`]. Vlakno xtasku sleduje log seriove linky,
//! a jakmile znacku uvidi, pripoji se na QMP a posle `input-send-event`:
//! klavesy, pohyb a tlacitka mysi, kolecko a Ctrl+Alt+Delete. Udalosti tak
//! projdou cele: QEMU -> virtio-input -> ovladac -> inputd -> test.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Musi sedet s `QEMU_MARKER` v `user/init/src/tests/input.rs`.
pub const MARKER: &str = "init: test vstupu: cekam na udalosti z QEMU";

/// Jak dlouho nejdyl cekat na znacku (cely beh testu).
const WAIT_MAX: Duration = Duration::from_secs(600);

/// Volny port TCP na localhostu pro QMP.
pub fn free_port() -> std::io::Result<u16> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// Argumenty QEMU pro QMP server na `port`.
pub fn qemu_arg(port: u16) -> String {
    format!("tcp:127.0.0.1:{port},server=on,wait=off")
}

/// Spusti vlakno, ktere po objeveni znacky v `log` posle udalosti.
pub fn spawn_injector(log: PathBuf, port: u16) {
    std::thread::spawn(move || {
        let start = Instant::now();
        loop {
            if start.elapsed() > WAIT_MAX {
                return;
            }
            let seen = std::fs::read(&log)
                .map(|b| String::from_utf8_lossy(&b).contains(MARKER))
                .unwrap_or(false);
            if seen {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        match inject(port) {
            Ok(()) => println!("xtask: QMP: udalosti vstupu poslany"),
            Err(e) => eprintln!("xtask: QMP: vkladani vstupu selhalo: {e}"),
        }
    });
}

/// Klavesa (qcode QEMU).
fn key(qcode: &str, down: bool) -> String {
    format!(
        r#"{{"type":"key","data":{{"down":{down},"key":{{"type":"qcode","data":"{qcode}"}}}}}}"#
    )
}

fn btn(button: &str, down: bool) -> String {
    format!(r#"{{"type":"btn","data":{{"down":{down},"button":"{button}"}}}}"#)
}

fn rel(axis: &str, value: i32) -> String {
    format!(r#"{{"type":"rel","data":{{"axis":"{axis}","value":{value}}}}}"#)
}

/// Krok skriptu: udalosti jednoho prikazu `input-send-event` (= jeden
/// snimek), nebo pauza.
enum Step {
    Send(Vec<String>),
    Pause(Duration),
}

/// Posloupnost kroku. Test v `input.rs` ocekava presne tyto udalosti.
/// Pred bezpecnou zkratkou je pauza: klavesnice a mys jsou ruzna zarizeni
/// a jejich udalosti si navzajem nedrzi poradi, mys musi dorazit driv, nez
/// zkratka prepne vstup na kanal `secure`.
fn script() -> Vec<Step> {
    let mut s: Vec<Step> = Vec::new();
    for (k, d) in [
        ("a", true),
        ("a", false),
        ("shift", true),
        ("1", true),
        ("1", false),
        ("shift", false),
    ] {
        s.push(Step::Send(vec![key(k, d)]));
    }
    s.push(Step::Send(vec![rel("x", 10), rel("y", -5)]));
    s.push(Step::Send(vec![btn("left", true)]));
    s.push(Step::Send(vec![btn("left", false)]));
    s.push(Step::Send(vec![btn("wheel-down", true)]));
    s.push(Step::Send(vec![btn("wheel-down", false)]));
    s.push(Step::Pause(Duration::from_secs(1)));
    for (k, d) in [
        ("ctrl", true),
        ("alt", true),
        ("delete", true),
        ("delete", false),
        ("alt", false),
        ("ctrl", false),
    ] {
        s.push(Step::Send(vec![key(k, d)]));
    }
    s
}

/// Precte odpoved na prikaz (preskoci asynchronni udalosti QMP).
fn reply(reader: &mut impl BufRead) -> Result<(), String> {
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            return Err("QMP spojeni skoncilo".into());
        }
        if line.contains("\"return\"") {
            return Ok(());
        }
        if line.contains("\"error\"") {
            return Err(format!("QMP chyba: {}", line.trim()));
        }
    }
}

fn inject(port: u16) -> Result<(), String> {
    let stream = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| e.to_string())?;
    let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    // Uvitani serveru, pak vyjednani schopnosti.
    let mut greeting = String::new();
    reader
        .read_line(&mut greeting)
        .map_err(|e| e.to_string())?;
    let mut send = |cmd: String, reader: &mut BufReader<TcpStream>| -> Result<(), String> {
        writer
            .write_all(cmd.as_bytes())
            .and_then(|_| writer.write_all(b"\n"))
            .map_err(|e| e.to_string())?;
        reply(reader)
    };
    send(r#"{"execute":"qmp_capabilities"}"#.into(), &mut reader)?;
    for step in script() {
        let events = match step {
            Step::Send(e) => e,
            Step::Pause(d) => {
                std::thread::sleep(d);
                continue;
            }
        };
        let cmd = format!(
            r#"{{"execute":"input-send-event","arguments":{{"events":[{}]}}}}"#,
            events.join(",")
        );
        send(cmd, &mut reader)?;
    }
    Ok(())
}
