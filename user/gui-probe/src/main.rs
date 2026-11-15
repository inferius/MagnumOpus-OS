//! gui-probe: test role `Gui` (relace okna zdedena od rodice,
//! `docs/cs/gui.md`, kapitola Relace okna pro potomky).
//!
//! Spousti ho test initu (`user/init/src/tests/gui_role.rs`) pres launchd
//! s roli `Gui`. Rezimy:
//!
//! - `okno`: otevre okno ze zdedene relace. Kod 0 = okno ma (prisel
//!   `CONFIGURE`), 3 = relaci nedostal, 4 = kompozitor odmitl.
//! - `dite`: spusti sam sebe v rezimu `okno` jednou s `MO_GUI` (ceka 0)
//!   a jednou bez ni (ceka 3). Kod 0 = obe dopadla podle ocekavani.
//! - `odvolani`: spusti dite v rezimu `cekej` s oknem, vypise
//!   `gui-probe: pripraveno` a pocka na konec ditete (jeho kod vrati).
//! - `cekej`: otevre okno, pak ceka, az mu kompozitor relaci zavre
//!   (odvolani GUI), a zkusi obnovu pres listek. Kod 0 = obnova odmitnuta.
//! - `vloz`: otevre okno, vypise jeho polohu (`gui-probe: okno na x y w h`)
//!   a po prvnim fyzickem stisku klavesy v okne (Ctrl+V z QEMU) pozada
//!   o text schranky; vypise `gui-probe: vlozeno <text>` (test schranky
//!   Wine -> kompozitor, `wine_gui.rs`). Kod 0 = vlozeno, 5 = odmitnuto.

#![feature(magnumopus_runtime, rustc_private)]

use std::os::magnumopus::runtime as rt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use guiproto::window::{ClientMsg, Mode, ServerMsg};

const NO_SESSION: i32 = 3;
const REFUSED: i32 = 4;
const TIMEOUT: Duration = Duration::from_secs(10);

/// Kanal relace okna (typ z runtime; jen zapis a cteni bez cekani).
type Win = rt::gui::Session;

/// Otevre okno 1 a pocka na `CONFIGURE` (nebo chybu).
fn open(ch: &Win) -> Result<(), i32> {
    let send = |m: ClientMsg| ch.write(&m.encode(), Vec::new()).map_err(|_| REFUSED);
    send(ClientMsg::Hello { version: guiproto::VERSION })?;
    send(ClientMsg::WindowCreate { window: 1, mode: Mode::Standard, w: 160, h: 90, title: "gui-probe".into() })?;
    let end = Instant::now() + TIMEOUT;
    while Instant::now() < end {
        match ch.try_read() {
            Ok(m) => match ServerMsg::decode(&m.data) {
                Ok(ServerMsg::Configure { window: 1, .. }) => return Ok(()),
                Ok(ServerMsg::Error { op, code }) => {
                    eprintln!("gui-probe: kompozitor odmitl operaci {op}: {code:?}");
                    return Err(REFUSED);
                }
                _ => {}
            },
            Err(rt::Error::ShouldWait) => std::thread::sleep(Duration::from_millis(5)),
            Err(e) => {
                eprintln!("gui-probe: relace okna: {e:?}");
                return Err(REFUSED);
            }
        }
    }
    eprintln!("gui-probe: CONFIGURE neprisel");
    Err(REFUSED)
}

fn window() -> Result<Win, i32> {
    let ch = rt::gui::take_session().ok_or(NO_SESSION)?;
    open(&ch)?;
    Ok(ch)
}

fn me(mode: &str) -> Command {
    let mut c = Command::new("gui-probe");
    c.arg(mode).stdout(Stdio::inherit()).stderr(Stdio::inherit());
    c
}

fn code(c: &mut Command) -> i32 {
    match c.status() {
        Ok(s) => s.code().unwrap_or(-1),
        Err(e) => {
            eprintln!("gui-probe: spusteni ditete: {e}");
            -1
        }
    }
}

fn run(mode: &str) -> i32 {
    match mode {
        "okno" => match window() {
            Ok(_) => {
                println!("gui-probe: okno otevreno");
                0
            }
            Err(c) => c,
        },
        "dite" => {
            let with = code(&mut me("okno"));
            let without = code(me("okno").env_remove("MO_GUI"));
            println!("gui-probe: dite s roli {with}, bez role {without}");
            if with == 0 && without == NO_SESSION { 0 } else { 1 }
        }
        "odvolani" => {
            let mut child = match me("cekej").spawn() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("gui-probe: {e}");
                    return 1;
                }
            };
            println!("gui-probe: pripraveno");
            child.wait().ok().and_then(|s| s.code()).unwrap_or(-1)
        }
        "cekej" => {
            let ch = match window() {
                Ok(ch) => ch,
                Err(c) => return c,
            };
            // Ceka na zavreni relace kompozitorem (odvolani).
            let end = Instant::now() + Duration::from_secs(60);
            loop {
                match ch.try_read() {
                    Ok(_) => {}
                    Err(rt::Error::ShouldWait) if Instant::now() < end => std::thread::sleep(Duration::from_millis(10)),
                    Err(rt::Error::PeerClosed) => break,
                    Err(e) => {
                        eprintln!("gui-probe: cekani na odvolani: {e:?}");
                        return 1;
                    }
                }
            }
            match rt::gui::reconnect(Duration::from_secs(2)) {
                Ok(_) => {
                    eprintln!("gui-probe: obnova po odvolani prosla");
                    1
                }
                Err(e) => {
                    println!("gui-probe: obnova odmitnuta ({e:?})");
                    0
                }
            }
        }
        "vloz" => paste(),
        _ => {
            eprintln!("gui-probe: neznamy rezim {mode}");
            2
        }
    }
}

/// Rezim `vloz`: poloha okna, pak vlozeni po fyzickem stisku klavesy.
fn paste() -> i32 {
    let ch = match window() {
        Ok(ch) => ch,
        Err(c) => return c,
    };
    // Okno je nakreslene az commitem: vrstva z jedne barvy staci.
    use guiproto::window::{LayerKind, LayerProps, prop};
    let send = |m: ClientMsg| ch.write(&m.encode(), Vec::new()).map_err(|_| REFUSED);
    let r = (|| {
        send(ClientMsg::LayerCreate { layer: 1, window: 1, parent: 0, kind: LayerKind::Solid })?;
        send(ClientMsg::LayerSet { layer: 1, props: LayerProps { mask: prop::RECT | prop::COLOR, rect: guiproto::Rect::new(0, 0, 160, 90), color: 0xFF20_4060, ..Default::default() } })?;
        send(ClientMsg::Commit { txn: 1, flags: 0, window: 1, serial: 0, token: 0 })
    })();
    if let Err(c) = r {
        return c;
    }
    let end = Instant::now() + Duration::from_secs(120);
    let mut asked = false;
    while Instant::now() < end {
        match ch.try_read() {
            Ok(m) => match ServerMsg::decode(&m.data) {
                Ok(ServerMsg::WindowPlace { window: 1, rect }) => {
                    println!("gui-probe: okno na {} {} {} {}", rect.x, rect.y, rect.w, rect.h);
                }
                Ok(ServerMsg::Input { records, .. }) => {
                    for r in records {
                        let key = matches!(r.event, guiproto::window::InputEvent::Key { action: 1, .. });
                        if key && r.serial != 0 && !asked {
                            asked = true;
                            let req = clipproto::ClientMsg::Request { gesture: r.serial, serial: 1, format: String::from(clipproto::TEXT) };
                            if send(ClientMsg::Clipboard { data: req.encode() }).is_err() {
                                return REFUSED;
                            }
                        }
                    }
                }
                Ok(ServerMsg::Clipboard { data }) => match clipproto::ServerMsg::decode(&data) {
                    Ok(clipproto::ServerMsg::Deliver { serial: 1, data, .. }) => {
                        println!("gui-probe: vlozeno {}", String::from_utf8_lossy(&data));
                        return 0;
                    }
                    Ok(clipproto::ServerMsg::Denied { serial: 1, why }) => {
                        println!("gui-probe: vlozeni odmitnuto {why:?}");
                        return 5;
                    }
                    _ => {}
                },
                Ok(ServerMsg::Ping { serial }) => {
                    let _ = send(ClientMsg::Pong { serial });
                }
                _ => {}
            },
            Err(rt::Error::ShouldWait) => std::thread::sleep(Duration::from_millis(5)),
            Err(e) => {
                eprintln!("gui-probe: relace okna: {e:?}");
                return REFUSED;
            }
        }
    }
    eprintln!("gui-probe: vlozeni nedoslo");
    1
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    std::process::exit(run(&mode));
}
