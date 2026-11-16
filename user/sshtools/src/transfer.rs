//! Prenos souboru pres subsystem `sftp`: `scp` (jeden soubor tam nebo
//! zpet, jen pres SFTP jako OpenSSH 9) a `sftp` (prikazy ze vstupu).

use std::io::Write;

use ssh::client::Request;
use ssh::sftp_client::{CHUNK, Reply, SftpClient, describe, eof, ok};

use crate::session::{Sess, Target, read_line};

/// Spojeni se subsystemem sftp.
struct Sftp {
    s: Sess,
    c: SftpClient,
    /// Pracovni slozka na serveru (`sftp`, `cd`).
    cwd: String,
}

impl Sftp {
    fn open(t: &Target) -> Result<Sftp, String> {
        let s = Sess::open(t, Request::Subsystem(String::from("sftp")))?;
        let mut f = Sftp { s, c: SftpClient::new(), cwd: String::new() };
        let init = f.c.init();
        f.s.write_all(&init)?;
        match f.wait(None)? {
            Reply::Version(3) => {}
            r => return Err(format!("server SFTP: {r:?}")),
        }
        let p = f.c.realpath(".");
        f.cwd = match f.call(p)? {
            Reply::Name { entries, .. } => entries.into_iter().next().map(|(n, _)| n).unwrap_or_else(|| String::from("/")),
            _ => String::from("/"),
        };
        Ok(f)
    }

    /// Ceka na odpoved s cislem `id` (`None` = VERSION).
    fn wait(&mut self, id: Option<u32>) -> Result<Reply, String> {
        loop {
            while let Some(r) = self.c.next_reply().map_err(|_| String::from("server poslal neplatný paket SFTP"))? {
                if r.id() == id {
                    return Ok(r);
                }
            }
            self.s.pump()?;
            while let Some((err, d)) = self.s.take() {
                if !err {
                    self.c.feed(&d);
                }
            }
            if !matches!(self.s.conn.state(), ssh::client::CState::Running) && !self.s.pending() {
                return Err(String::from("server ukončil spojení"));
            }
        }
    }

    fn call(&mut self, (id, p): (u32, Vec<u8>)) -> Result<Reply, String> {
        self.s.write_all(&p)?;
        self.wait(Some(id))
    }

    /// Cesta na serveru vuci pracovni slozce.
    fn path(&self, p: &str) -> String {
        if p.starts_with('/') || self.cwd.is_empty() { p.to_string() } else { format!("{}/{p}", self.cwd.trim_end_matches('/')) }
    }

    fn is_dir(&mut self, p: &str) -> bool {
        let q = self.c.stat(p);
        matches!(self.call(q), Ok(Reply::Attrs { attrs, .. }) if attrs.is_dir())
    }

    fn upload(&mut self, data: &[u8], remote: &str) -> Result<(), String> {
        let q = self.c.open(remote, true);
        let h = match self.call(q)? {
            Reply::Handle { handle, .. } => handle,
            r => return Err(format!("{remote}: {}", describe(&r))),
        };
        for (i, ch) in data.chunks(CHUNK).enumerate() {
            let q = self.c.write(&h, (i * CHUNK) as u64, ch);
            let r = self.call(q)?;
            if !ok(&r) {
                return Err(format!("{remote}: {}", describe(&r)));
            }
        }
        let q = self.c.close(&h);
        let r = self.call(q)?;
        if ok(&r) { Ok(()) } else { Err(format!("{remote}: {}", describe(&r))) }
    }

    fn download(&mut self, remote: &str) -> Result<Vec<u8>, String> {
        let q = self.c.open(remote, false);
        let h = match self.call(q)? {
            Reply::Handle { handle, .. } => handle,
            r => return Err(format!("{remote}: {}", describe(&r))),
        };
        let mut out = Vec::new();
        loop {
            let q = self.c.read(&h, out.len() as u64, CHUNK as u32);
            match self.call(q)? {
                Reply::Data { data, .. } if !data.is_empty() => out.extend(data),
                r if eof(&r) => break,
                Reply::Data { .. } => break,
                r => return Err(format!("{remote}: {}", describe(&r))),
            }
        }
        let q = self.c.close(&h);
        let _ = self.call(q);
        Ok(out)
    }

    fn list(&mut self, p: &str) -> Result<Vec<(String, u64, bool)>, String> {
        let q = self.c.opendir(p);
        let h = match self.call(q)? {
            Reply::Handle { handle, .. } => handle,
            r => return Err(format!("{p}: {}", describe(&r))),
        };
        let mut out = Vec::new();
        loop {
            let q = self.c.readdir(&h);
            match self.call(q)? {
                Reply::Name { entries, .. } => {
                    out.extend(entries.into_iter().filter(|(n, _)| n != "." && n != "..").map(|(n, a)| (n, a.size.unwrap_or(0), a.is_dir())))
                }
                r if eof(&r) => break,
                r => return Err(format!("{p}: {}", describe(&r))),
            }
        }
        let q = self.c.close(&h);
        let _ = self.call(q);
        out.sort();
        Ok(out)
    }

    fn simple(&mut self, q: (u32, Vec<u8>), what: &str) -> Result<(), String> {
        let r = self.call(q)?;
        if ok(&r) { Ok(()) } else { Err(format!("{what}: {}", describe(&r))) }
    }
}

fn basename(p: &str) -> &str {
    p.trim_end_matches('/').rsplit('/').next().unwrap_or(p)
}

/// Vzdalena cesta `[uzivatel@]server:cesta`.
fn remote_spec(s: &str) -> Option<(&str, &str)> {
    let (host, path) = s.split_once(':')?;
    (!host.is_empty() && !host.contains('/') && !s.starts_with('.') && !s.starts_with('/')).then_some((host, path))
}

/// `scp [--port N] zdroj cil` (prave jedna strana na serveru).
pub fn scp(args: &[String]) -> i32 {
    const USE: &str = "použití: scp [--port N] soubor [uživatel@]server:[cesta] | [uživatel@]server:cesta [soubor]";
    let (port, rest) = match crate::port_and_rest(args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("scp: {e}\n{USE}");
            return 2;
        }
    };
    let [src, dst] = rest.as_slice() else {
        eprintln!("{USE}");
        return 2;
    };
    let r = match (remote_spec(src), remote_spec(dst)) {
        (None, Some((host, path))) => (|| -> Result<(), String> {
            let data = std::fs::read(src).map_err(|e| format!("{src}: {e}"))?;
            let mut f = Sftp::open(&Target::parse(host, port)?)?;
            let mut target = if path.is_empty() { basename(src).to_string() } else { path.to_string() };
            target = f.path(&target);
            if f.is_dir(&target) {
                target = format!("{}/{}", target.trim_end_matches('/'), basename(src));
            }
            f.upload(&data, &target)
        })(),
        (Some((host, path)), None) => (|| -> Result<(), String> {
            let mut f = Sftp::open(&Target::parse(host, port)?)?;
            let remote = f.path(path);
            let data = f.download(&remote)?;
            let mut local = std::path::PathBuf::from(dst);
            if local.is_dir() {
                local.push(basename(path));
            }
            std::fs::write(&local, data).map_err(|e| format!("{}: {e}", local.display()))
        })(),
        _ => Err(String::from("právě jedna strana musí být na serveru (server:cesta)")),
    };
    match r {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("scp: {e}");
            1
        }
    }
}

/// `sftp [--port N] [uzivatel@]server`: prikazy ze vstupu.
pub fn sftp(args: &[String]) -> i32 {
    const USE: &str = "použití: sftp [--port N] [uživatel@]server";
    let (port, rest) = match crate::port_and_rest(args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("sftp: {e}\n{USE}");
            return 2;
        }
    };
    let [dest] = rest.as_slice() else {
        eprintln!("{USE}");
        return 2;
    };
    let mut f = match Target::parse(dest, port).and_then(|t| Sftp::open(&t)) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("sftp: {e}");
            return 255;
        }
    };
    println!("Připojeno. Příkazy: ls, cd, pwd, get, put, rm, mkdir, quit.");
    let mut failed = false;
    while let Some(line) = read_line() {
        let w: Vec<&str> = line.split_whitespace().collect();
        let r: Result<(), String> = match w.as_slice() {
            [] => Ok(()),
            ["quit" | "exit" | "bye"] => break,
            ["pwd"] => {
                println!("{}", f.cwd);
                Ok(())
            }
            ["cd", p] => {
                let p = f.path(p);
                let q = f.c.realpath(&p);
                match f.call(q) {
                    Ok(Reply::Name { entries, .. }) if !entries.is_empty() => {
                        let n = entries[0].0.clone();
                        if f.is_dir(&n) {
                            f.cwd = n;
                            Ok(())
                        } else {
                            Err(format!("{p}: není složka"))
                        }
                    }
                    Ok(r) => Err(format!("{p}: {}", describe(&r))),
                    Err(e) => Err(e),
                }
            }
            ["ls"] | ["ls", _] => {
                let p = f.path(w.get(1).copied().unwrap_or("."));
                f.list(&p).map(|l| {
                    for (n, size, dir) in l {
                        if dir { println!("{n}/") } else { println!("{n}\t{size}") }
                    }
                })
            }
            ["get", r] | ["get", r, _] => {
                let remote = f.path(r);
                let local = w.get(2).map_or_else(|| basename(r).to_string(), |s| s.to_string());
                f.download(&remote).and_then(|d| std::fs::write(&local, d).map_err(|e| format!("{local}: {e}")))
            }
            ["put", l] | ["put", l, _] => {
                let remote = f.path(w.get(2).copied().unwrap_or_else(|| basename(l)));
                std::fs::read(l).map_err(|e| format!("{l}: {e}")).and_then(|d| f.upload(&d, &remote))
            }
            ["rm", p] => {
                let p = f.path(p);
                let q = f.c.remove(&p);
                f.simple(q, &p)
            }
            ["mkdir", p] => {
                let p = f.path(p);
                let q = f.c.mkdir(&p);
                f.simple(q, &p)
            }
            _ => Err(format!("{line}: neznámý příkaz (ls, cd, pwd, get, put, rm, mkdir, quit)")),
        };
        if let Err(e) = r {
            eprintln!("sftp: {e}");
            failed = true;
        }
        let _ = std::io::stdout().flush();
    }
    if failed { 1 } else { 0 }
}
