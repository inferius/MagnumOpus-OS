//! Prehravac (`user/gui-player`): plocha videa `libs/ui` (vrstva z bufferu
//! NV12 od dekoderu pres `mediad`, ploty ziskani a uvolneni) a ovladani.
//!
//! - pred prehranim je plocha videa prazdna (pozadi okna),
//! - po "Prehrat" (akce pres strom pristupnosti) se v plose objevi obraz
//!   ukazky a meni se (pohyblivy pruh), cas na stitku postupuje,
//! - po "Pozastavit" se obraz zastavi.

use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use guiproto::a11y::{Node, Role, actions};
use guiproto::control::{InspectReply, InspectRequest};
use libos::spawn::StartHandle;
use libos::{Startup, time};

use super::gui::{ask, has, node, pixel, settle, ui_bg, wait_tree, wait_window, with};
use super::{TestResult, fail};
use crate::{check, check_eq};

const APP: &str = "org.magnumopus.player";

fn act(inspect: &libos::Channel, window: u32, n: &Node) -> TestResult {
    let r = ask(inspect, InspectRequest::Act { window, node: n.id, action: actions::ACTIVATE, value: String::new() })?;
    check_eq!(r, InspectReply::Done, "akce tlacitka");
    Ok(())
}

/// Pixely v radku pres stred plochy videa (pruh se pohybuje vodorovne).
fn row(inspect: &libos::Channel, v: &Node) -> Result<Vec<u32>, String> {
    settle(inspect)?;
    let y = v.bounds.y + v.bounds.h as i32 / 2;
    let mut out = Vec::new();
    for i in 1..16 {
        out.push(pixel(inspect, v.bounds.x + v.bounds.w as i32 * i / 16, y)?);
    }
    Ok(out)
}

pub fn play_pause(startup: &Startup) -> TestResult {
    with(|s| {
        let mut extra = alloc::vec![StartHandle::user(crate::media::connect().map_err(|e| fail("relace mediad", e))?.into_handle())];
        if let Ok(a) = crate::audio::connect() {
            extra.push(StartHandle::user(a.into_handle()));
        }
        let i = s.gui.launch_extra(startup, "gui-player", extra).map_err(|e| fail("spusteni prehravace", e))?;
        let r = (|| {
            let w = wait_window(&s.inspect, APP)?;
            let t = wait_tree(&s.inspect, w.id, "okno prehravace", |t| has(t, Role::Button, "Přehrát"))?;
            let v = node(&t, Role::Image, "Video")?.clone();
            check!(v.bounds.w >= 64 && v.bounds.h >= 36, "plocha videa ma rozmer");
            let before = row(&s.inspect, &v)?;
            check!(before.iter().all(|&p| p == ui_bg()), "pred prehranim je plocha prazdna");
            act(&s.inspect, w.id, node(&t, Role::Button, "Přehrát")?)?;
            wait_tree(&s.inspect, w.id, "tlacitko Pozastavit", |t| has(t, Role::Button, "Pozastavit"))?;
            // Obraz ukazky (prechod jasu, ne pozadi) a pohyb pruhu.
            let end = time::deadline_after(Duration::from_secs(10));
            let first = loop {
                let r = row(&s.inspect, &v)?;
                if r.iter().filter(|&&p| p != ui_bg()).count() >= 12 {
                    break r;
                }
                check!(time::now() < end, "v plose videa se neobjevil obraz");
                libos::time::sleep(Duration::from_millis(50));
            };
            loop {
                libos::time::sleep(Duration::from_millis(200));
                if row(&s.inspect, &v)? != first {
                    break;
                }
                check!(time::now() < end, "obraz videa se nemeni");
            }
            wait_tree(&s.inspect, w.id, "cas postupuje", |t| has(t, Role::Label, "0:02 / 0:10"))?;
            let t = wait_tree(&s.inspect, w.id, "okno pri prehravani", |t| has(t, Role::Button, "Pozastavit"))?;
            act(&s.inspect, w.id, node(&t, Role::Button, "Pozastavit")?)?;
            wait_tree(&s.inspect, w.id, "tlacitko Přehrát po pauze", |t| has(t, Role::Button, "Přehrát"))?;
            let a = row(&s.inspect, &v)?;
            libos::time::sleep(Duration::from_millis(300));
            check_eq!(row(&s.inspect, &v)?, a, "obraz po pauze stoji");
            Ok(())
        })();
        let _ = s.gui.apps[i].process.kill();
        r
    })
}
