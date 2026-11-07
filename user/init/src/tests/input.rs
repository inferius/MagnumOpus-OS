//! Vstup: ovladac virtio-input, sluzba inputd, rozlozeni, bezpecna zkratka.
//!
//! Test `QEMU: klavesnice a mys` overuje celou cestu ze zarizeni: vypise
//! znacku [`QEMU_MARKER`], na kterou xtask posle pres QMP
//! (`input-send-event`) stisky klaves, pohyb mysi a Ctrl+Alt+Delete.
//! Ostatni testy pouzivaji umely vstup (`inject`), takze nezavisi na
//! hostiteli.

use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use inputproto::code::*;
use inputproto::device::{self, RawEvent};
use inputproto::{Action, Button, Event, Record, control};
use libos::{Error, Startup, println, time};

use super::{Test, TestResult, devices, fail};
use crate::input::{Input, drain, read_until};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("sluzba bezi a vidi klavesnici a mys", service_status),
    ("QEMU: klavesnice a mys az do sluzby", qemu_devices),
    ("umely vstup: text v rozlozeni US", inject_us),
    ("rozlozeni CZ: QWERTZ, hacky, carky, AltGr", layout_cz),
    ("mys: pohyb, tlacitka, kolecko", mouse_events),
    ("umely vstup nevyvola bezpecnou zkratku", inject_no_secure),
    ("opakovani drzene klavesy", key_repeat),
    ("neplatne prikazy a zpravy", bad_commands),
];

/// Znacka v logu, na kterou xtask posle udalosti pres QMP (viz
/// `xtask/src/qmp.rs`; text musi sedet).
pub const QEMU_MARKER: &str = "init: test vstupu: cekam na udalosti z QEMU";

/// Jak dlouho cekat na udalosti z QEMU (QMP spojeni, TCG muze byt pomaly).
const QEMU_TIMEOUT: Duration = Duration::from_secs(20);
/// Jak dlouho cekat na udalosti z umeleho vstupu.
const TIMEOUT: Duration = Duration::from_secs(10);

fn input() -> Result<&'static Input, String> {
    let d = devices().ok_or("spravce zarizeni nebezi")?;
    d.input.as_ref().ok_or_else(|| "sluzba vstupu nebezi".into())
}

/// Popis chyby cteni udalosti i s tim, co prislo (kratky zapis, aby se
/// vesel do radku logu): K30+ stisk klavesy 30, K30- uvolneni, K30r
/// opakovani, T'a' text, M(dx,dy), B272+ tlacitko, S(dx,dy), F0/F1 fokus,
/// SA zkratka, I1 umely vstup, L0 rozlozeni.
fn failr(what: &str, (e, got): (Error, Vec<Record>)) -> String {
    let mut s = alloc::format!("{what}: {e}; prislo {}:", got.len());
    for r in got.iter().take(48) {
        let a = |x: Action| match x {
            Action::Press => '+',
            Action::Release => '-',
            Action::Repeat => 'r',
        };
        let t = match r.event {
            Event::Key { code, action, .. } => alloc::format!("K{code}{}", a(action)),
            Event::Text(c) => alloc::format!("T{c:?}"),
            Event::Motion { dx, dy } => alloc::format!("M({dx},{dy})"),
            Event::Button { button, action } => alloc::format!("B{}{}", button.code(), a(action)),
            Event::Scroll { dx, dy } => alloc::format!("S({dx},{dy})"),
            Event::SecureAttention => String::from("SA"),
            Event::Focus(f) => alloc::format!("F{}", f as u8),
            Event::Injection { active } => alloc::format!("I{active}"),
            Event::Layout(l) => alloc::format!("L{l}"),
            Event::Unknown { kind } => alloc::format!("?{kind}"),
        };
        s.push(' ');
        s.push_str(&t);
    }
    s
}

fn deadline(d: Duration) -> u64 {
    time::deadline_after(d)
}

/// Vlozeny text ze zaznamu.
fn text(records: &[Record]) -> String {
    records
        .iter()
        .filter_map(|r| match r.event {
            Event::Text(c) => Some(c),
            _ => None,
        })
        .collect()
}

fn has(records: &[Record], e: Event) -> bool {
    records.iter().any(|r| r.event == e)
}

/// Stisk a uvolneni klavesy, kazde jako jeden snimek.
fn tap(code: u16) -> [RawEvent; 4] {
    [
        RawEvent::key(code, true),
        RawEvent::syn(),
        RawEvent::key(code, false),
        RawEvent::syn(),
    ]
}

/// Klavesa s drzenym modifikatorem.
fn chord(modifier: u16, code: u16) -> Vec<RawEvent> {
    let mut v = alloc::vec![RawEvent::key(modifier, true), RawEvent::syn()];
    v.extend(tap(code));
    v.extend([RawEvent::key(modifier, false), RawEvent::syn()]);
    v
}

fn send(ch: &libos::Channel, events: &[RawEvent]) -> TestResult {
    ch.write(&device::encode_events(events), Vec::new())
        .map_err(|e| fail("zapis umeleho vstupu", e))
}

fn service_status(_: &Startup) -> TestResult {
    let st = input()?.status().map_err(|e| fail("status", e))?;
    check!(st.keyboards >= 1, "sluzba nevidi klavesnici virtio");
    check!(st.pointers >= 1, "sluzba nevidi mys virtio");
    check_eq!(st.layout_name(), "us", "vychozi rozlozeni");
    check!(!st.secure, "sluzba je v bezpecnem rezimu");
    Ok(())
}

fn qemu_devices(_: &Startup) -> TestResult {
    let r = qemu_devices_inner();
    // Bezpecny rezim ukoncit vzdy, i po selhani, jinak by dalsi testy
    // nedostaly zadne udalosti.
    if let Ok(inp) = input() {
        let _ = inp.secure.write(control::RELEASE.as_bytes(), Vec::new());
    }
    r
}

fn qemu_devices_inner() -> TestResult {
    let inp = input()?;
    drain(&inp.focus);
    drain(&inp.secure);
    println!("{QEMU_MARKER}");
    let until = deadline(QEMU_TIMEOUT);
    // xtask: a, Shift+1, pohyb (10, -5), leve tlacitko, kolecko dolu.
    let got = read_until(&inp.focus, until, |r| {
        text(r).contains("a!")
            && r.iter().any(|x| matches!(x.event, Event::Motion { .. }))
            && has(
                r,
                Event::Button {
                    button: Button::Left,
                    action: Action::Release,
                },
            )
            && r.iter().any(|x| matches!(x.event, Event::Scroll { .. }))
    })
    .map_err(|e| failr("udalosti z QEMU (bezi xtask s QMP?)", e))?;
    check!(
        got.iter().all(|r| !r.is_synthetic()),
        "udalost ze zarizeni oznacena jako umela"
    );
    check!(
        has(
            &got,
            Event::Key {
                code: KEY_A,
                scancode: KEY_A as u32,
                action: Action::Press,
                ch: Some('a'),
            }
        ),
        "chybi stisk klavesy A se znakem 'a'"
    );
    let (dx, dy) = got.iter().fold((0, 0), |(x, y), r| match r.event {
        Event::Motion { dx, dy } => (x + dx, y + dy),
        _ => (x, y),
    });
    check_eq!((dx, dy), (10, -5), "soucet pohybu mysi");
    let wheel: i32 = got
        .iter()
        .map(|r| match r.event {
            Event::Scroll { dy, .. } => dy,
            _ => 0,
        })
        .sum();
    check_eq!(wheel, -1, "kolecko dolu");

    // Ctrl+Alt+Delete: jen kanal `secure`, fokus o nej prijde.
    let secure = read_until(&inp.secure, until, |r| {
        has(r, Event::SecureAttention)
            && r.iter().any(|x| {
                matches!(
                    x.event,
                    Event::Key {
                        code: KEY_LEFTCTRL,
                        action: Action::Release,
                        ..
                    }
                )
            })
    })
    .map_err(|e| failr("bezpecna zkratka z QEMU", e))?;
    check!(
        !secure
            .iter()
            .any(|r| matches!(r.event, Event::Key { code: KEY_DELETE, .. })),
        "klavesa Delete ze zkratky nemela projit"
    );
    let focus = read_until(&inp.focus, until, |r| has(r, Event::Focus(false)))
        .map_err(|e| failr("ztrata fokusu po zkratce", e))?;
    check!(
        !focus.iter().any(|r| matches!(
            r.event,
            Event::Key {
                code: KEY_DELETE,
                ..
            } | Event::SecureAttention
        )),
        "zkratka dorazila k vlastnikovi fokusu"
    );
    check!(
        input()?.status().map_err(|e| fail("status", e))?.secure,
        "sluzba neni v bezpecnem rezimu"
    );
    inp.secure
        .write(control::RELEASE.as_bytes(), Vec::new())
        .map_err(|e| fail("release", e))?;
    read_until(&inp.focus, deadline(TIMEOUT), |r| has(r, Event::Focus(true)))
        .map_err(|e| failr("navrat fokusu", e))?;
    Ok(())
}

fn inject_us(_: &Startup) -> TestResult {
    let inp = input()?;
    drain(&inp.focus);
    let inj = inp.inject().map_err(|e| fail("inject", e))?;
    read_until(&inp.focus, deadline(TIMEOUT), |r| {
        has(r, Event::Injection { active: 1 })
    })
    .map_err(|e| failr("indikator umeleho vstupu", e))?;
    inj.write(
        &device::encode_hello(device::caps::KEYBOARD, "test init"),
        Vec::new(),
    )
    .map_err(|e| fail("predstaveni", e))?;
    let mut ev = Vec::new();
    for k in [KEY_H, KEY_I, KEY_SPACE] {
        ev.extend(tap(k));
    }
    ev.extend(chord(KEY_LEFTSHIFT, KEY_1));
    ev.extend(tap(KEY_ENTER));
    send(&inj, &ev)?;
    let got = read_until(&inp.focus, deadline(TIMEOUT), |r| {
        r.iter().any(|x| {
            matches!(
                x.event,
                Event::Key {
                    code: KEY_ENTER,
                    action: Action::Release,
                    ..
                }
            )
        })
    })
    .map_err(|e| failr("udalosti umeleho vstupu", e))?;
    check_eq!(text(&got), String::from("hi !"), "vlozeny text");
    check!(
        got.iter()
            .filter(|r| !matches!(r.event, Event::Injection { .. }))
            .all(|r| r.is_synthetic()),
        "umela udalost bez priznaku SYNTHETIC"
    );
    drop(inj);
    read_until(&inp.focus, deadline(TIMEOUT), |r| {
        has(r, Event::Injection { active: 0 })
    })
    .map_err(|e| failr("konec umeleho vstupu", e))?;
    Ok(())
}

fn layout_cz(_: &Startup) -> TestResult {
    let inp = input()?;
    inp.set_layout("cz").map_err(|e| fail("layout cz", e))?;
    let r = (|| {
        drain(&inp.focus);
        let inj = inp.inject().map_err(|e| fail("inject", e))?;
        let mut ev = Vec::new();
        // z (klavesa Y), e s hackem (2), carka + e, hacek (Shift) + c, AltGr+V.
        ev.extend(tap(KEY_Y));
        ev.extend(tap(KEY_2));
        ev.extend(tap(KEY_EQUAL));
        ev.extend(tap(KEY_E));
        ev.extend(chord(KEY_LEFTSHIFT, KEY_EQUAL));
        ev.extend(tap(KEY_C));
        ev.extend(chord(KEY_RIGHTALT, KEY_V));
        ev.extend(tap(KEY_ENTER));
        send(&inj, &ev)?;
        let got = read_until(&inp.focus, deadline(TIMEOUT), |r| {
            r.iter().any(|x| {
                matches!(
                    x.event,
                    Event::Key {
                        code: KEY_ENTER,
                        action: Action::Press,
                        ..
                    }
                )
            })
        })
        .map_err(|e| failr("udalosti", e))?;
        check_eq!(text(&got), String::from("zěéč@"), "text v CZ");
        check!(
            got.iter().any(|r| matches!(
                r.event,
                Event::Key {
                    code: KEY_Y,
                    ch: Some('z'),
                    ..
                }
            )),
            "klavesa Y nema v CZ znak 'z'"
        );
        let st = input()?.status().map_err(|e| fail("status", e))?;
        check_eq!(
            st.layout_name(),
            "cz",
            "rozlozeni ve stavu"
        );
        Ok(())
    })();
    inp.set_layout("us").map_err(|e| fail("layout us", e))?;
    r
}

fn mouse_events(_: &Startup) -> TestResult {
    let inp = input()?;
    drain(&inp.focus);
    let inj = inp.inject().map_err(|e| fail("inject", e))?;
    send(
        &inj,
        &[
            // Jeden snimek: pohyb v obou osach se slozi do jedne udalosti.
            RawEvent::rel(REL_X, 3),
            RawEvent::rel(REL_Y, 4),
            RawEvent::rel(REL_X, 2),
            RawEvent::syn(),
            RawEvent::key(BTN_RIGHT, true),
            RawEvent::syn(),
            RawEvent::key(BTN_RIGHT, false),
            RawEvent::rel(REL_WHEEL, 2),
            RawEvent::rel(REL_HWHEEL, -1),
            RawEvent::syn(),
        ],
    )?;
    let got = read_until(&inp.focus, deadline(TIMEOUT), |r| {
        r.iter().any(|x| matches!(x.event, Event::Scroll { .. }))
    })
    .map_err(|e| failr("udalosti mysi", e))?;
    let events: Vec<Event> = got
        .iter()
        .map(|r| r.event)
        .filter(|e| !matches!(e, Event::Injection { .. }))
        .collect();
    check_eq!(
        events,
        alloc::vec![
            Event::Motion { dx: 5, dy: 4 },
            Event::Button {
                button: Button::Right,
                action: Action::Press
            },
            Event::Button {
                button: Button::Right,
                action: Action::Release
            },
            Event::Scroll { dx: -1, dy: 2 },
        ],
        "udalosti mysi"
    );
    Ok(())
}

fn inject_no_secure(_: &Startup) -> TestResult {
    let inp = input()?;
    drain(&inp.focus);
    drain(&inp.secure);
    let inj = inp.inject().map_err(|e| fail("inject", e))?;
    let mut ev = alloc::vec![
        RawEvent::key(KEY_LEFTCTRL, true),
        RawEvent::key(KEY_LEFTALT, true),
        RawEvent::syn(),
    ];
    ev.extend(tap(KEY_DELETE));
    ev.extend([
        RawEvent::key(KEY_LEFTALT, false),
        RawEvent::key(KEY_LEFTCTRL, false),
        RawEvent::syn(),
    ]);
    send(&inj, &ev)?;
    let got = read_until(&inp.focus, deadline(TIMEOUT), |r| {
        r.iter().any(|x| {
            matches!(
                x.event,
                Event::Key {
                    code: KEY_LEFTCTRL,
                    action: Action::Release,
                    ..
                }
            )
        })
    })
    .map_err(|e| failr("udalosti", e))?;
    check!(
        got.iter().any(|r| matches!(
            r.event,
            Event::Key {
                code: KEY_DELETE,
                action: Action::Press,
                ..
            }
        )),
        "umely Delete mel projit jako obycejna klavesa"
    );
    // Kanal `secure` muze dostat jen indikator umeleho vstupu.
    while let Ok(m) = inp.secure.try_read() {
        let recs = inputproto::event::decode(&m.data).ok_or("neplatna zprava na secure")?;
        check!(
            !has(&recs, Event::SecureAttention),
            "umely vstup vyvolal bezpecnou zkratku"
        );
    }
    check!(
        !input()?.status().map_err(|e| fail("status", e))?.secure,
        "sluzba je v bezpecnem rezimu"
    );
    Ok(())
}

fn key_repeat(_: &Startup) -> TestResult {
    let inp = input()?;
    drain(&inp.focus);
    let inj = inp.inject().map_err(|e| fail("inject", e))?;
    send(&inj, &[RawEvent::key(KEY_X, true), RawEvent::syn()])?;
    // Prvni opakovani za 500 ms, pak 30x za sekundu.
    let got = read_until(&inp.focus, deadline(TIMEOUT), |r| {
        r.iter().any(|x| {
            matches!(
                x.event,
                Event::Key {
                    code: KEY_X,
                    action: Action::Repeat,
                    ..
                }
            )
        })
    })
    .map_err(|e| failr("opakovani", e))?;
    send(&inj, &[RawEvent::key(KEY_X, false), RawEvent::syn()])?;
    let first = got.iter().find(|r| {
        matches!(
            r.event,
            Event::Key {
                code: KEY_X,
                action: Action::Press,
                ..
            }
        )
    });
    let rep = got.iter().find(|r| {
        matches!(
            r.event,
            Event::Key {
                action: Action::Repeat,
                ..
            }
        )
    });
    let (Some(first), Some(rep)) = (first, rep) else {
        return Err("chybi stisk nebo opakovani".into());
    };
    check!(
        rep.time - first.time >= 400_000_000,
        "opakovani prislo pred prodlevou 500 ms"
    );
    check!(text(&got).starts_with("xx"), "opakovani nepise text");
    // Po uvolneni opakovani skonci.
    read_until(&inp.focus, deadline(TIMEOUT), |r| {
        r.iter().any(|x| {
            matches!(
                x.event,
                Event::Key {
                    code: KEY_X,
                    action: Action::Release,
                    ..
                }
            )
        })
    })
    .map_err(|e| failr("uvolneni", e))?;
    libos::time::sleep(Duration::from_millis(200));
    while let Ok(m) = inp.focus.try_read() {
        let recs = inputproto::event::decode(&m.data).ok_or("neplatna zprava")?;
        check!(
            !recs.iter().any(|r| matches!(
                r.event,
                Event::Key {
                    action: Action::Repeat,
                    ..
                }
            )),
            "opakovani pokracuje po uvolneni"
        );
    }
    Ok(())
}

fn bad_commands(_: &Startup) -> TestResult {
    let inp = input()?;
    check_eq!(
        inp.set_layout("xx").err(),
        Some(Error::NotFound),
        "nezname rozlozeni"
    );
    check_eq!(
        inp.command("blabla", Vec::new()).err(),
        Some(Error::NotSupported),
        "neznamy prikaz"
    );
    check_eq!(
        inp.command(control::FOCUS, Vec::new()).err(),
        Some(Error::InvalidArgs),
        "focus bez kanalu"
    );
    // Neplatna zprava zdroje sluzbu neshodi.
    let inj = inp.inject().map_err(|e| fail("inject", e))?;
    inj.write(b"nesmysl", Vec::new())
        .map_err(|e| fail("zapis", e))?;
    let st = inp.status().map_err(|e| fail("status po nesmyslu", e))?;
    check!(st.injecting >= 1, "zdroj po neplatne zprave zmizel");
    Ok(())
}
