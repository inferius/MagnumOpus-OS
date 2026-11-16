//! Test vzdaleneho pristupu SSH z hostitele (`docs/cs/ssh.md`, sada `ssh`).
//!
//! Klientem je skutecne OpenSSH hostitele (`ssh`, `ssh-keygen`; ve
//! Windows `C:\Windows\System32\OpenSSH`), ne knihovna: test tak overi
//! i spolupraci s klientem, ktery lide pouzivaji.
//!
//! - Pred startem QEMU: dva klice ed25519 (spravny a cizi) ve vlastni
//!   slozce behu, port hostitele presmerovany na port 22 hosta
//!   (`hostfwd`), pomocny server na localhostu.
//! - Host dostane v initfs soubor `ssh-test`: port pomocneho serveru
//!   a verejny spravny klic (test ho prida uctu).
//! - Test v hostu se pripoji k pomocnemu serveru (10.0.2.2) a posle radek:
//!   `positive <uzivatel>` = prihlaseni klicem, prikaz s vystupem a kodem
//!   ukonceni, chyba prikazu, interaktivni terminal (PTY), cizi klic
//!   a samotne heslo odmitnute, relace v domove uctu, scp tam a zpet
//!   a sftp (vypis, smazani); `negative` = vypnuty server spojeni
//!   neprijme. Odpoved `ok` nebo `err <duvod>`.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Port hosta.
const GUEST_PORT: u16 = 22;

pub struct SshTest {
    /// Dalsi argument `hostfwd` (pridat za ostatni s carkou).
    pub hostfwd: String,
    /// Obsah souboru `ssh-test` pro initfs.
    pub config: String,
}

/// Program OpenSSH v PATH (ve Windows i `System32\OpenSSH`).
fn tool(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    if cfg!(windows) {
        let p = PathBuf::from(r"C:\Windows\System32\OpenSSH").join(&exe);
        if p.exists() {
            return Some(p);
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(&exe)).find(|p| p.exists())
}

fn keygen(keygen: &Path, file: &Path) -> Result<String, String> {
    let _ = std::fs::remove_file(file);
    let _ = std::fs::remove_file(file.with_extension("pub"));
    let st = Command::new(keygen)
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "xtask-ssh-test", "-f"])
        .arg(file)
        .status()
        .map_err(|e| format!("ssh-keygen: {e}"))?;
    if !st.success() {
        return Err(format!("ssh-keygen skoncil {st}"));
    }
    std::fs::read_to_string(file.with_extension("pub")).map(|s| s.trim().to_string()).map_err(|e| format!("klic: {e}"))
}

/// Pripravi klice, port a pomocny server. `dir` = slozka behu.
pub fn start(dir: &Path) -> Result<SshTest, String> {
    let ssh = tool("ssh").ok_or("chybi klient OpenSSH (ssh): ve Windows Volitelne funkce -> Klient OpenSSH, v Linuxu balik openssh-client")?;
    let kg = tool("ssh-keygen").ok_or("chybi ssh-keygen (OpenSSH)")?;
    let scp = tool("scp").ok_or("chybi scp (OpenSSH)")?;
    let sftp = tool("sftp").ok_or("chybi sftp (OpenSSH)")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let key = dir.join("klic");
    let other = dir.join("cizi");
    let public = keygen(&kg, &key)?;
    keygen(&kg, &other)?;
    let fwd = TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map_err(|e| format!("port: {e}"))?.port();
    let helper = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("pomocny server: {e}"))?;
    let hport = helper.local_addr().map_err(|e| e.to_string())?.port();
    let known = dir.join("known_hosts");
    std::thread::spawn(move || {
        for s in helper.incoming().flatten() {
            let mut r = BufReader::new(match s.try_clone() {
                Ok(c) => c,
                Err(_) => continue,
            });
            let mut line = String::new();
            if r.read_line(&mut line).is_err() {
                continue;
            }
            let c = Client { ssh: ssh.clone(), scp: scp.clone(), sftp: sftp.clone(), port: fwd, key: key.clone(), other: other.clone(), known: known.clone() };
            let res = match line.split_whitespace().collect::<Vec<_>>().as_slice() {
                ["positive", user] => c.positive(user),
                ["negative"] => c.negative(),
                _ => Err(format!("neznamy pozadavek {line:?}")),
            };
            let reply = match res {
                Ok(()) => String::from("ok\n"),
                Err(e) => format!("err {}\n", e.replace('\n', " | ")),
            };
            println!("xtask: test ssh ({}): {}", line.trim(), reply.trim());
            let mut s = s;
            let _ = s.write_all(reply.as_bytes());
        }
    });
    Ok(SshTest {
        hostfwd: format!("hostfwd=tcp:127.0.0.1:{fwd}-:{GUEST_PORT}"),
        config: format!("helper={hport}\nkey={public}\n"),
    })
}

struct Client {
    ssh: PathBuf,
    scp: PathBuf,
    sftp: PathBuf,
    port: u16,
    key: PathBuf,
    other: PathBuf,
    known: PathBuf,
}

/// Vystup prikazu klienta: (kod, stdout, stderr).
type Out = (i32, String, String);

impl Client {
    fn cmd(&self, key: &Path, extra: &[&str]) -> Command {
        let mut c = Command::new(&self.ssh);
        c.arg("-p")
            .arg(self.port.to_string())
            .arg("-i")
            .arg(key)
            .args(["-o", "IdentitiesOnly=yes", "-o", "StrictHostKeyChecking=no", "-o", "ConnectTimeout=20", "-o", "ServerAliveInterval=0"])
            .arg("-o")
            .arg(format!("UserKnownHostsFile={}", self.known.display()))
            .args(extra);
        c
    }

    fn run(&self, mut c: Command, stdin: &[u8]) -> Result<Out, String> {
        let mut child = c
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("spusteni ssh: {e}"))?;
        if let Some(mut i) = child.stdin.take() {
            let data = stdin.to_vec();
            std::thread::spawn(move || {
                // Terminal: znaky po kouscich, at je stihne (jako clovek);
                // bajt 0 = pauza 3 s (vystup prikazu se vykresli).
                for part in data.split(|&b| b == 0) {
                    for chunk in part.chunks(16) {
                        let _ = i.write_all(chunk);
                        let _ = i.flush();
                        std::thread::sleep(Duration::from_millis(150));
                    }
                    std::thread::sleep(Duration::from_secs(3));
                }
                std::thread::sleep(Duration::from_secs(3));
            });
        }
        let out = child.wait_with_output().map_err(|e| format!("ssh: {e}"))?;
        let s = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
        Ok((out.status.code().unwrap_or(-1), s(&out.stdout), s(&out.stderr)))
    }

    fn positive(&self, user: &str) -> Result<(), String> {
        let host = format!("{user}@127.0.0.1");
        let batch = ["-o", "BatchMode=yes"];
        // 1. Prikaz s vystupem.
        let mut c = self.cmd(&self.key, &batch);
        c.arg(&host).arg("echo ahoj-ze-ssh");
        let (code, out, err) = self.run(c, b"")?;
        if code != 0 || out.trim() != "ahoj-ze-ssh" {
            return Err(format!("exec: kod {code}, vystup {out:?}, chyby {err:?}"));
        }
        // 2. Chyba prikazu = nenulovy kod.
        let mut c = self.cmd(&self.key, &batch);
        c.arg(&host).arg("rm /neni-tu-ssh");
        let (code, _, err) = self.run(c, b"")?;
        if code == 0 || code == 255 {
            return Err(format!("chybny prikaz: kod {code} ({err:?})"));
        }
        // 3. Interaktivni terminal (PTY): prikaz, jeho vystup na obrazovce,
        // exit ukonci spojeni.
        let mut c = self.cmd(&self.key, &batch);
        c.arg("-tt").arg(&host);
        let (code, out, err) = self.run(c, b"echo pty-ok\r\0exit\r")?;
        let screen = strip_ansi(&out);
        if code != 0 || !screen.contains("pty-ok") {
            let tail: String = screen.chars().rev().take(600).collect::<Vec<_>>().into_iter().rev().collect();
            return Err(format!("terminal: kod {code}, chyby {err:?}, obrazovka ...{tail:?}"));
        }
        // 4. Cizi klic: odmitnuto.
        let mut c = self.cmd(&self.other, &batch);
        c.arg(&host).arg("echo nesmi");
        let (code, out, _) = self.run(c, b"")?;
        if code != 255 || out.contains("nesmi") {
            return Err(format!("cizi klic prosel (kod {code})"));
        }
        // 5. Samotne heslo: server ho ani nenabidne.
        let mut c = self.cmd(&self.key, &["-o", "BatchMode=yes", "-o", "PreferredAuthentications=password,keyboard-interactive", "-o", "PubkeyAuthentication=no"]);
        c.arg(&host).arg("echo nesmi");
        let (code, _, err) = self.run(c, b"")?;
        if code != 255 || !err.contains("publickey") {
            return Err(format!("heslo: kod {code}, {err:?}"));
        }
        // 6. Relace patri uctu: pracovni slozka = jeho domov.
        let mut c = self.cmd(&self.key, &batch);
        c.arg(&host).arg("pwd");
        let (code, out, err) = self.run(c, b"")?;
        if code != 0 || out.trim() != format!("/home/{user}") {
            return Err(format!("pwd: kod {code}, vystup {out:?} ({err:?})"));
        }
        // 7. scp tam a zpet (OpenSSH 9+ pres SFTP), obsah s diakritikou
        // a binarni bajty.
        let dir = self.known.parent().unwrap_or(Path::new(".")).to_path_buf();
        let up = dir.join("nahoru.bin");
        let down = dir.join("dolu.bin");
        let mut data: Vec<u8> = "Zkouška přenosu přes SSH\n".as_bytes().to_vec();
        data.extend((0..=255u8).cycle().take(70_000));
        std::fs::write(&up, &data).map_err(|e| format!("soubor: {e}"))?;
        let _ = std::fs::remove_file(&down);
        let scp = |args: &[&std::ffi::OsStr]| -> Result<(), String> {
            let mut c = Command::new(&self.scp);
            c.arg("-P").arg(self.port.to_string()).arg("-i").arg(&self.key);
            c.args(["-o", "IdentitiesOnly=yes", "-o", "StrictHostKeyChecking=no", "-o", "BatchMode=yes"]);
            c.arg("-o").arg(format!("UserKnownHostsFile={}", self.known.display()));
            c.args(args);
            let (code, _, err) = self.run(c, b"")?;
            if code != 0 { Err(format!("scp: kod {code} ({err:?})")) } else { Ok(()) }
        };
        let remote = format!("{user}@127.0.0.1:prenos.bin");
        scp(&[up.as_os_str(), std::ffi::OsStr::new(&remote)])?;
        scp(&[std::ffi::OsStr::new(&remote), down.as_os_str()])?;
        let back = std::fs::read(&down).map_err(|e| format!("stazeny soubor: {e}"))?;
        if back != data {
            return Err(format!("scp: obsah se lisi ({} B tam, {} B zpet)", data.len(), back.len()));
        }
        // 8. sftp: vypis a smazani.
        let mut c = Command::new(&self.sftp);
        c.arg("-P").arg(self.port.to_string()).arg("-i").arg(&self.key);
        c.args(["-o", "IdentitiesOnly=yes", "-o", "StrictHostKeyChecking=no", "-o", "BatchMode=yes", "-b", "-"]);
        c.arg("-o").arg(format!("UserKnownHostsFile={}", self.known.display()));
        c.arg(format!("{user}@127.0.0.1"));
        let (code, out, err) = self.run(c, b"ls -l\nrm prenos.bin\nls\n")?;
        if code != 0 || !out.contains("prenos.bin") {
            return Err(format!("sftp: kod {code}, vystup {out:?} ({err:?})"));
        }
        Ok(())
    }

    fn negative(&self) -> Result<(), String> {
        let mut c = self.cmd(&self.key, &["-o", "BatchMode=yes", "-o", "ConnectTimeout=10"]);
        c.arg("nikdo@127.0.0.1").arg("echo nesmi");
        let (code, out, _) = self.run(c, b"")?;
        if code == 255 && !out.contains("nesmi") { Ok(()) } else { Err(format!("vypnuty server prijal spojeni (kod {code})")) }
    }
}

/// Text obrazovky bez escape sekvenci CSI a OSC.
fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('[') => {
                for d in it.by_ref() {
                    if ('@'..='~').contains(&d) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(d) = it.next() {
                    if d == '\x07' || (d == '\x1b' && it.peek() == Some(&'\\')) {
                        it.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}
