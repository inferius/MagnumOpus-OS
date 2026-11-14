//! Zivotni cyklus ovladace a smycka udalosti.
//!
//! ```text
//!  probe --> start --> [ udalosti: preruseni | zprava | casovac ] --> stop
//!    |                                  | chyba / klient zavrel       ^
//!    +-- chyba: zarizeni se nepouzije --+-----------------------------+
//! ```
//!
//! - `probe`: najde a inicializuje zarizeni (reset, registry, DMA, preruseni).
//!   Po padu ovladace ho spravce zarizeni spusti znovu a `probe` zacne od
//!   resetu zarizeni: ovladac nesmi predpokladat zadny stav z minula.
//! - `start`: predstaveni klientovi (sluzbe), prvni pozadavky.
//! - udalosti: [`Driver::interrupt`], [`Driver::message`], [`Driver::timer`].
//! - `stop`: zastavi zarizeni tak, aby uz nesahalo do DMA pameti (reset,
//!   vypnuti bus master). Vola se vzdy, i po chybe.

use alloc::format;

use crate::platform::{Event, IrqId, Platform};
use crate::{DriverError, Result, proto};

/// Ovladac zarizeni.
pub trait Driver<P: Platform>: Sized {
    /// Inicializuje zarizeni.
    fn probe(p: &P) -> Result<Self>;

    /// Po uspesnem `probe`: predstaveni klientovi apod.
    fn start(&mut self, _p: &P) -> Result<()> {
        Ok(())
    }

    /// Preruseni `irq` (uz potvrzene: dalsi behem obsluhy se neztrati).
    fn interrupt(&mut self, _p: &P, _irq: IrqId) -> Result<()> {
        Ok(())
    }

    /// Zprava od klienta (ne `dk:`, ty vyridi smycka).
    fn message(&mut self, _p: &P, _msg: &[u8]) -> Result<()> {
        Ok(())
    }

    /// Termin dalsiho [`timer`](Self::timer) (ns od startu), `None` = zadny.
    fn next_timer(&self) -> Option<u64> {
        None
    }

    fn timer(&mut self, _p: &P) -> Result<()> {
        Ok(())
    }

    /// Zastavi zarizeni (zadne DMA ani preruseni po navratu).
    fn stop(&mut self, p: &P);

    /// Klient (sluzba) zavrel kanal. `false` (vychozi) = ovladac skonci.
    /// `true` = bezi dal bez klienta (obnova sluzeb: sluzba po padu
    /// pokracuje pres adapter supervisoru a ovladac nesmi zmizet, viz
    /// `docs/cs/obnova-sluzeb.md`); `send` pak vraci [`crate::Sent::NoClient`].
    fn client_closed(&mut self, _p: &P) -> bool {
        false
    }
}

/// Proc ovladac skoncil.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit {
    /// Klient zavrel kanal (normalni konec).
    ClientClosed,
    /// Restart na zadost (`dk:restart`).
    Restart,
    /// Simulovany pad (`dk:crash`): jako skutecny pad - ovladac se
    /// nezastavi a nic neuvolni (zarizeni zastavi a pamet uklidi jadro,
    /// karantena DMA).
    Crash,
    /// Chyba ovladace nebo zarizeni (`probe` = jeste pred startem).
    Failed { probe: bool, err: DriverError },
}

impl Exit {
    /// Navratovy kod procesu: 0 konec, [`proto::EXIT_RESTART`],
    /// [`proto::EXIT_CRASH`], 2 konfigurace, 3 chybejici pravo nebo kvota,
    /// 4 zarizeni.
    pub fn code(&self) -> i64 {
        match self {
            Exit::ClientClosed => 0,
            Exit::Restart => proto::EXIT_RESTART,
            Exit::Crash => proto::EXIT_CRASH,
            Exit::Failed { err, .. } => err.exit_code().max(1),
        }
    }
}

/// Ovladac v behu: smycka po krocich (testy muzou vkladat udalosti
/// a kontrolovat stav mezi kroky).
pub struct Runner<D> {
    pub driver: D,
}

impl<D> Runner<D> {
    /// `probe` + `start`. Pri chybe `start` zarizeni zastavi.
    pub fn new<P: Platform>(p: &P) -> core::result::Result<Self, Exit>
    where
        D: Driver<P>,
    {
        let mut driver = D::probe(p).map_err(|err| Exit::Failed { probe: true, err })?;
        if let Err(err) = driver.start(p) {
            driver.stop(p);
            return Err(Exit::Failed { probe: false, err });
        }
        Ok(Self { driver })
    }

    /// Jedna udalost. `Ok(None)` = pokracovat, `Ok(Some(exit))` = konec
    /// (zarizeni jeste nezastavene, viz [`finish`](Self::finish)).
    pub fn step<P: Platform>(&mut self, p: &P) -> Option<Exit>
    where
        D: Driver<P>,
    {
        let ev = match p.wait(self.driver.next_timer()) {
            Ok(ev) => ev,
            Err(DriverError::ClientClosed) => return self.client_lost(p),
            Err(err) => return Some(Exit::Failed { probe: false, err }),
        };
        self.handle(p, ev)
    }

    /// Klient zavrel kanal: konec, nebo beh bez klienta (podle ovladace).
    fn client_lost<P: Platform>(&mut self, p: &P) -> Option<Exit>
    where
        D: Driver<P>,
    {
        if self.driver.client_closed(p) {
            p.detach_client();
            None
        } else {
            Some(Exit::ClientClosed)
        }
    }

    /// Zpracuje udalost `ev` (bez cekani).
    pub fn handle<P: Platform>(&mut self, p: &P, ev: Event) -> Option<Exit>
    where
        D: Driver<P>,
    {
        let r = match ev {
            Event::Interrupt(irq) => self.driver.interrupt(p, irq),
            Event::Timer => self.driver.timer(p),
            Event::ClientClosed => return self.client_lost(p),
            Event::Message(m) if proto::is_reserved(&m) => return reserved(p, &m),
            Event::Message(m) => self.driver.message(p, &m),
        };
        match r {
            Ok(()) => None,
            Err(DriverError::ClientClosed) => self.client_lost(p),
            Err(err) => Some(Exit::Failed { probe: false, err }),
        }
    }

    /// Zastavi zarizeni a zaloguje duvod konce.
    pub fn finish<P: Platform>(mut self, p: &P, exit: &Exit)
    where
        D: Driver<P>,
    {
        self.driver.stop(p);
        log_exit(p, exit);
    }
}

fn reserved<P: Platform>(p: &P, m: &[u8]) -> Option<Exit> {
    match m {
        proto::PING => {
            let _ = p.send(format!("{} {}", proto::PONG, p.name()).as_bytes());
            None
        }
        proto::INFO => {
            let i = p.info();
            let _ = p.send(
                format!("dk:info {} {} {:04x}:{:04x}", p.name(), i, i.vendor, i.device_id).as_bytes(),
            );
            None
        }
        proto::RESTART => Some(Exit::Restart),
        proto::CRASH => Some(Exit::Crash),
        _ => {
            crate::log!(p, "neznama zprava driverkitu {:?}", core::str::from_utf8(m).unwrap_or("?"));
            None
        }
    }
}

fn log_exit<P: Platform>(p: &P, exit: &Exit) {
    match exit {
        Exit::ClientClosed => {}
        Exit::Restart => crate::log!(p, "restart na zadost"),
        Exit::Crash => crate::log!(p, "simulovany pad (dk:crash)"),
        Exit::Failed { probe: true, err } => crate::log!(p, "inicializace selhala: {err}"),
        Exit::Failed { probe: false, err } => crate::log!(p, "{err}"),
    }
}

/// Cely zivot ovladace: probe, start, udalosti az do konce, stop. Vraci
/// duvod konce (`Exit::code` = navratovy kod procesu).
pub fn run<P: Platform, D: Driver<P>>(p: &P) -> Exit {
    let mut runner = match Runner::<D>::new(p) {
        Ok(r) => r,
        Err(exit) => {
            log_exit(p, &exit);
            return exit;
        }
    };
    loop {
        if let Some(exit) = runner.step(p) {
            if exit == Exit::Crash {
                // Pad: bez `stop` a bez uvolneni (buffery DMA, mapovani,
                // prostredek zustanou az do konce procesu).
                log_exit(p, &exit);
                core::mem::forget(runner);
                return exit;
            }
            runner.finish(p, &exit);
            return exit;
        }
    }
}
