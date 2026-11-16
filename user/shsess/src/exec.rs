//! Spousteni programu (sluzba "exec" initu, `launchproto::exec`): relace
//! shellu sama sit ani pravo spoustet programy nema. Kanal sluzby prijde
//! od termd zpravou `SessMsg::Exec`; kazda zadost ma vlastni kanal pro
//! odpoved. Bezici program obsluhuje vlakno radku (`term::program`).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use launchproto::exec::{Exec, Mount, parse_reply};
use libos::{Channel, Error, Process, time};
use shlang::ShError;
use accountproto::Secret;
use accountproto::agent::{Prompt, PromptAnswer};
use term::program::{Output, Request, Running, SystemPrompt};
use ttyproto::{Request as TtyRequest, Size, Tty};

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
    let pairs = (|| -> Result<_, Error> {
        Ok((Channel::create()?, Channel::create()?, Channel::create()?, Channel::create()?, Channel::create()?, Channel::create()?))
    })();
    let ((back, back_t), (in_w, in_t), (out_r, out_t), (err_r, err_t), (tty, tty_t), (prompt, prompt_t)) = match pairs {
        Ok(p) => p,
        Err(err) => return fail(err, "kanály"),
    };
    // Ridici kanal terminalu programu a systemova vyzva (sluzba uctu, jen
    // u klienta SSH; init ji jinak zahodi).
    let hs = alloc::vec![
        back_t.into_handle(),
        in_t.into_handle(),
        out_t.into_handle(),
        err_t.into_handle(),
        tty_t.into_handle(),
        prompt_t.into_handle()
    ];
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
    let ttys = Tty::new(Size { rows: 24, cols: 80, xpixel: 0, ypixel: 0 });
    Some(Ok(Box::new(Program {
        process,
        _control: control,
        stdin: Some(in_w),
        out: [Some(out_r), Some(err_r)],
        code: None,
        tty: Some(tty),
        ttys,
        prompt: Some(prompt),
        asked: None,
    })))
}

struct Program {
    process: Process,
    /// Kanal k potomkovi od launchd (drzet po dobu behu).
    _control: Option<Channel>,
    stdin: Option<Channel>,
    /// stdout, stderr (`None` = zavreny a vycerpany).
    out: [Option<Channel>; 2],
    code: Option<i64>,
    /// Ridici kanal terminalu programu a jeho stav (ECHO, velikost).
    tty: Option<Channel>,
    ttys: Tty,
    /// Systemova vyzva od sluzby uctu (program ji nevidi).
    prompt: Option<Channel>,
    /// Prijata a jeste nezobrazena vyzva.
    asked: Option<SystemPrompt>,
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

    fn poll_control(&mut self) {
        if let Some(ch) = &self.tty {
            loop {
                match ch.try_read() {
                    Ok(m) => {
                        let out = TtyRequest::decode(&m.data).and_then(|r| self.ttys.handle(&r));
                        // Udalost zmeny velikosti zatim neni (okno shellu
                        // velikost programu nemeni): WATCH_SIZE bez handlu.
                        let _ = ch.write(&ttyproto::encode_reply(out), Vec::new());
                    }
                    Err(Error::ShouldWait) => break,
                    Err(_) => {
                        self.tty = None;
                        break;
                    }
                }
            }
        }
        if self.asked.is_none()
            && let Some(ch) = &self.prompt
        {
            match ch.try_read() {
                Ok(m) => {
                    self.asked = match Prompt::decode(&m.data) {
                        Ok(Prompt::Secret { id, question }) => Some(SystemPrompt { id, question, secret: true }),
                        Ok(Prompt::Confirm { id, question }) => Some(SystemPrompt { id, question, secret: false }),
                        Err(_) => None,
                    };
                }
                Err(Error::ShouldWait) => {}
                Err(_) => self.prompt = None,
            }
        }
    }

    fn echo(&self) -> bool {
        self.ttys.mode.lflag & ttyproto::lflag::ECHO != 0
    }

    fn system_prompt(&mut self) -> Option<SystemPrompt> {
        self.asked.take()
    }

    fn answer_system(&mut self, id: u32, ok: bool, text: String) {
        let Some(ch) = &self.prompt else { return };
        let mut a = PromptAnswer { id, ok, text: Secret::from(text.as_str()) }.encode();
        let _ = ch.write(&a, Vec::new());
        zeroize::Zeroize::zeroize(&mut a);
        let mut t = text;
        zeroize::Zeroize::zeroize(&mut t);
    }
}
