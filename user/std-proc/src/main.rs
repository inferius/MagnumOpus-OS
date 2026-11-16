//! std-proc: test `std::process` (docs/cs/rust-std.md, faze 2).
//!
//! Bez argumentu je rodic a overuje spousteni pres launchd (sam sebe
//! v rezimu potomka). Spousti ho test v initu
//! (`user/init/src/tests/std_proc.rs`) s relaci FS, relaci site (jen
//! TCP na echo server, promenna `ECHO`), kanalem ke launchd a kopii
//! programu v `/std-test/bin/std-proc`. Kazda kontrola vypise `ok <jmeno>`,
//! chyba `FAIL <jmeno>: <popis>` na stderr. Posledni radek: `std-proc: hotovo`.
//!
//! Rezimy potomka (`std-proc <rezim> ...`): `echo <slova>`, `exit <kod>`,
//! `upper` (stdin -> stdout velkymi), `env` (promenna X a pracovni
//! adresar), `sleep`, `net` (spojeni na `ECHO`).

use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::os::magnumopus::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::Duration;

type Check = Result<(), String>;

fn ensure(cond: bool, msg: impl Into<String>) -> Check {
    if cond { Ok(()) } else { Err(msg.into()) }
}

fn me() -> Command {
    Command::new("std-proc")
}

fn out_of(cmd: &mut Command) -> Result<(i32, String, String), String> {
    let o = cmd.output().map_err(|e| format!("{cmd:?}: {e}"))?;
    Ok((
        o.status.code().unwrap_or(i32::MIN),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    ))
}

fn output() -> Check {
    let (code, out, err) = out_of(me().args(["echo", "ahoj", "svete"]))?;
    ensure(code == 0 && out == "ahoj svete\n", format!("kod {code}, stdout {out:?}, stderr {err:?}"))
}

fn exit_code() -> Check {
    let s = me().args(["exit", "7"]).status().map_err(|e| e.to_string())?;
    ensure(s.code() == Some(7) && !s.success(), format!("status {s}"))
}

fn stdin_pipe() -> Check {
    let mut c = me().arg("upper").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    let mut input = c.stdin.take().ok_or("chybi stdin")?;
    input.write_all(b"ahoj z rodice").map_err(|e| e.to_string())?;
    drop(input);
    let mut s = String::new();
    c.stdout.take().ok_or("chybi stdout")?.read_to_string(&mut s).map_err(|e| e.to_string())?;
    let st = c.wait().map_err(|e| e.to_string())?;
    ensure(st.success() && s == "AHOJ Z RODICE", format!("{st}: {s:?}"))
}

fn env_cwd() -> Check {
    let (code, out, _) = out_of(me().arg("env").env("X", "hodnota").current_dir("/std-test"))?;
    ensure(code == 0 && out == "X=hodnota cwd=/std-test\n", format!("{out:?}"))?;
    // env_clear: potomek nezdedi promenne rodice.
    let (_, out, _) = out_of(me().arg("env").env_clear())?;
    ensure(out.starts_with("X=- "), format!("env_clear: {out:?}"))
}

fn null_and_inherit() -> Check {
    let s = me().args(["echo", "zahozeno"]).stdout(Stdio::null()).status().map_err(|e| e.to_string())?;
    ensure(s.success(), format!("null: {s}"))?;
    // Zdedeny stdout: radek prijde do stdout rodice (overi test v initu).
    let s = me().args(["echo", "zdedeny", "vystup"]).status().map_err(|e| e.to_string())?;
    ensure(s.success(), format!("inherit: {s}"))
}

fn kill() -> Check {
    let mut c = me().arg("sleep").spawn().map_err(|e| e.to_string())?;
    ensure(c.try_wait().map_err(|e| e.to_string())?.is_none(), "potomek uz skoncil")?;
    c.kill().map_err(|e| format!("kill: {e}"))?;
    let s = c.wait().map_err(|e| e.to_string())?;
    // EXIT_KILLED (abi::process): ukoncen jinym procesem.
    ensure(s.code() == Some(-1_000_000), format!("po kill: {s}"))
}

/// Kopie programu ve FS nema vedle sebe manifest: pomocny program dostane
/// relaci site rodice (role Net: `MO_NET` v prostredi nebo `network(true)`);
/// bez `MO_NET` a bez `network(true)` zadnou. (`std-proc` z initfs ma
/// manifest bez `[network]`, ten sit nedostane nikdy.)
fn network() -> Check {
    let helper = || Command::new("/std-test/bin/std-proc");
    let (_, out, _) = out_of(helper().arg("net").env_remove("MO_NET"))?;
    ensure(out == "bez site\n", format!("bez MO_NET a bez network(): {out:?}"))?;
    let (_, out, err) = out_of(helper().arg("net"))?;
    ensure(out == "sit ok\n", format!("zdedene MO_NET (role Net): {out:?} {err:?}"))?;
    let (_, out, err) = out_of(helper().arg("net").network(true))?;
    ensure(out == "sit ok\n", format!("s network(true): {out:?} {err:?}"))?;
    let (_, out, _) = out_of(me().arg("net").network(true))?;
    ensure(out == "bez site\n", format!("manifest bez [network]: {out:?}"))
}

fn not_found() -> Check {
    match Command::new("neexistuje-xyz").status() {
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        r => Err(format!("{r:?} (cekano NotFound)")),
    }
}

fn absolute_path() -> Check {
    let (code, out, err) = out_of(Command::new("/std-test/bin/std-proc").args(["echo", "z", "cesty"]))?;
    ensure(code == 0 && out == "z cesty\n", format!("kod {code}: {out:?} {err:?}"))
}

/// Prikaz z registru (`[[command]]` v `std-proc.manifest.toml`): manifest
/// nema `[network]`, takze potomek sit nedostane ani s `network(true)`.
fn registry() -> Check {
    let (code, out, err) = out_of(Command::new("std-proc-kid").args(["echo", "z", "registru"]))?;
    ensure(code == 0 && out == "z registru\n", format!("kod {code}: {out:?} {err:?}"))?;
    let (_, out, _) = out_of(Command::new("std-proc-kid").arg("net").network(true))?;
    ensure(out == "bez site\n", format!("manifest bez site: {out:?}"))
}

fn io_pipe() -> Check {
    let (mut r, w) = std::io::pipe().map_err(|e| e.to_string())?;
    let mut c = me().args(["echo", "pres", "rouru"]).stdout(w).spawn().map_err(|e| e.to_string())?;
    let mut s = String::new();
    r.read_to_string(&mut s).map_err(|e| e.to_string())?;
    c.wait().map_err(|e| e.to_string())?;
    ensure(s == "pres rouru\n", format!("{s:?}"))?;
    ensure(std::process::id() != 0, "process::id")
}

fn child(mode: &str, rest: &[String]) -> i32 {
    match mode {
        "echo" => println!("{}", rest.join(" ")),
        "exit" => return rest.first().and_then(|c| c.parse().ok()).unwrap_or(1),
        "upper" => {
            let mut s = String::new();
            if std::io::stdin().read_to_string(&mut s).is_err() {
                return 2;
            }
            print!("{}", s.to_uppercase());
        }
        "env" => {
            let x = std::env::var("X").unwrap_or_else(|_| String::from("-"));
            let cwd = std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default();
            println!("X={x} cwd={cwd}");
        }
        "sleep" => std::thread::sleep(Duration::from_secs(60)),
        "net" => {
            let addr = std::env::var("ECHO").ok().and_then(|a| a.parse().ok());
            let r = addr.ok_or(ErrorKind::InvalidInput.into()).and_then(|a| TcpStream::connect_timeout(&a, Duration::from_secs(10)));
            match r {
                Ok(_) => println!("sit ok"),
                Err(e) if e.kind() == ErrorKind::Unsupported => println!("bez site"),
                Err(e) => println!("chyba site: {e}"),
            }
        }
        _ => return 3,
    }
    0
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(mode) = args.get(1) {
        std::process::exit(child(mode, &args[2..]));
    }
    let checks: [(&str, fn() -> Check); 11] = [
        ("output", output),
        ("exit_code", exit_code),
        ("stdin_pipe", stdin_pipe),
        ("env_cwd", env_cwd),
        ("null_and_inherit", null_and_inherit),
        ("kill", kill),
        ("network", network),
        ("not_found", not_found),
        ("absolute_path", absolute_path),
        ("registry", registry),
        ("io_pipe", io_pipe),
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
    println!("std-proc: hotovo");
}
