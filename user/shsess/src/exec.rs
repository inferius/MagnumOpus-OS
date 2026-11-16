//! Spousteni programu (sluzba "exec" initu, `launchproto::exec`): relace
//! shellu sama sit ani pravo spoustet programy nema. Kanal sluzby prijde
//! od termd zpravou `SessMsg::Exec`; kazda zadost ma vlastni kanal pro
//! odpoved. Bezici program obsluhuje vlakno radku (`term::program`).

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::time::Duration;

use launchproto::exec::{Exec, Mount, parse_reply};
use libos::{Channel, Error, Process, time};
use shlang::ShError;
use term::program::{Output, Request, Running};

/// Kanal sluzby relace (`None` = relace ho nedostala: konzole systemu,
/// testy bez prihlaseni).
pub static SERVICE: spin::Mutex<Option<Channel>> = spin::Mutex::new(None);

/// init ceka na launchd 30 s; pojistka navic.
const REPLY_TIMEOUT: Duration = Duration::from_secs(35);

/// Spusti program. `None` = programy tu spoustet nejde nebo program
/// neexistuje (shell hlasi neznamy prikaz).
pub fn start(req: &Request) -> Option<Result<Box<dyn Running>, ShError>> {
    let guard = SERVICE.lock();
    let svc = guard.as_ref()?;
    let e = Exec {
        program: req.program.clone(),
        cwd: req.cwd.clone(),
        args: req.args.clone(),
        mounts: req.mounts.iter().map(|m| Mount { name: m.name.clone(), target: m.target.clone(), read_only: m.read_only }).collect(),
        net: req.net,
    };
    let fail = |e: Error, what: &str| Some(Err(ShError::sys(e, alloc::format!("{}: {what}", req.program))));
    let msg = match e.encode() {
        Ok(m) => m,
        Err(err) => return fail(err, "příliš mnoho argumentů"),
    };
    let pairs = (|| -> Result<_, Error> { Ok((Channel::create()?, Channel::create()?, Channel::create()?, Channel::create()?)) })();
    let ((back, back_t), (in_w, in_t), (out_r, out_t), (err_r, err_t)) = match pairs {
        Ok(p) => p,
        Err(err) => return fail(err, "kanály"),
    };
    let hs = alloc::vec![back_t.into_handle(), in_t.into_handle(), out_t.into_handle(), err_t.into_handle()];
    if let Err(err) = svc.write(&msg, hs) {
        return fail(err, "služba spouštění");
    }
    drop(guard);
    let m = match back.read(Some(time::deadline_after(REPLY_TIMEOUT))) {
        Ok(m) => m,
        Err(err) => return fail(err, "služba spouštění neodpověděla"),
    };
    match parse_reply(&m.data) {
        Ok(Ok(())) => {}
        // Neznamy program: shell nabidne podobne jmeno.
        Ok(Err((Error::NotFound, _))) => return None,
        Ok(Err((err, text))) => return Some(Err(ShError::sys(err, text))),
        Err(err) => return fail(err, "odpověď služby"),
    }
    let mut hs = m.handles.into_iter();
    let Some(process) = hs.next().map(Process::from_handle) else {
        return fail(Error::Corrupted, "chybí proces");
    };
    let control = hs.next().map(Channel::from_handle);
    Some(Ok(Box::new(Program { process, _control: control, stdin: Some(in_w), out: [Some(out_r), Some(err_r)], code: None })))
}

struct Program {
    process: Process,
    /// Kanal k potomkovi od launchd (drzet po dobu behu).
    _control: Option<Channel>,
    stdin: Option<Channel>,
    /// stdout, stderr (`None` = zavreny a vycerpany).
    out: [Option<Channel>; 2],
    code: Option<i64>,
}

impl Running for Program {
    fn output(&mut self) -> Vec<Output> {
        let mut v = Vec::new();
        for (i, slot) in self.out.iter_mut().enumerate() {
            // Nejvys par zprav najednou: jinak by velky vystup zablokoval
            // reakci na Ctrl+C a vstup.
            for _ in 0..64 {
                let Some(ch) = slot.as_ref() else { break };
                match ch.try_read() {
                    Ok(m) => v.push(Output { stderr: i == 1, data: m.data }),
                    Err(Error::ShouldWait) => break,
                    Err(_) => {
                        *slot = None;
                        break;
                    }
                }
            }
        }
        v
    }

    fn write_stdin(&mut self, data: &[u8]) -> bool {
        let Some(ch) = &self.stdin else { return false };
        for chunk in data.chunks(libos::abi::syscall::CHANNEL_MAX_DATA) {
            if ch.write(chunk, Vec::new()).is_err() {
                self.stdin = None;
                return false;
            }
        }
        true
    }

    fn close_stdin(&mut self) {
        self.stdin = None;
    }

    fn exit_code(&mut self) -> Option<i64> {
        if self.code.is_none()
            && let Ok(c) = self.process.wait_exit(Some(time::now()))
        {
            self.code = Some(c);
        }
        // Konec az po precteni vystupu (zavrene kanaly); po konci procesu
        // se zavrou samy (zadny jiny drzitel konce programu).
        match self.code {
            Some(c) if self.out.iter().all(Option::is_none) => Some(c),
            _ => None,
        }
    }

    fn kill(&mut self) {
        let _ = self.process.kill();
        self.stdin = None;
    }
}
