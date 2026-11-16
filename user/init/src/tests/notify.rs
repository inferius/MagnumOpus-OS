//! Oznameni (`docs/cs/oznameni.md`): verejny protokol `notifyproto` proti
//! bezici sluzbe `notifyd` - identita odesilatele od initu, viditelnost
//! podle uzivatele, akce zpet odesilateli, nahrazeni, Nerusit, `Urgent`
//! jen pro system, ridici prikazy jen od initu.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use libos::{Channel, Error, Startup, time};
use notifyproto::{Action, Notification, Priority, Send, SendReply, View, ViewReply};

use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[("protokol: identita, viditelnost, akce, nahrazeni, Nerusit", protocol)];

const WAIT: Duration = Duration::from_secs(5);

fn read_reply(ch: &Channel, what: &str) -> Result<SendReply, String> {
    let m = ch.read(Some(time::deadline_after(WAIT))).map_err(|e| fail(what, e))?;
    SendReply::decode(&m.data).map_err(|e| fail(what, e))
}

/// Dalsi zmena prohlizece (preskoci `Result`).
fn next_view(ch: &Channel, what: &str) -> Result<ViewReply, String> {
    loop {
        let m = ch.read(Some(time::deadline_after(WAIT))).map_err(|e| fail(what, e))?;
        match ViewReply::decode(&m.data).map_err(|e| fail(what, e))? {
            ViewReply::Result { error: Some(e), op } => return Err(format!("{what}: op {op}: {e}")),
            ViewReply::Result { .. } => continue,
            r => return Ok(r),
        }
    }
}

fn added(ch: &Channel, what: &str) -> Result<Notification, String> {
    match next_view(ch, what)? {
        ViewReply::Added(n) => Ok(n),
        r => Err(format!("{what}: cekal jsem Added, prislo {r:?}")),
    }
}

/// Nic dalsiho neprislo (kratke cekani).
fn quiet(ch: &Channel) -> bool {
    matches!(ch.read(Some(time::deadline_after(Duration::from_millis(200)))), Err(Error::TimedOut))
}

fn notify(key: u64, title: &str, priority: Priority, actions: Vec<Action>) -> Vec<u8> {
    Send::Notify { key, title: title.into(), body: "Telo oznameni\ndruhy radek".into(), priority, actions }.encode()
}

fn protocol(_: &Startup) -> TestResult {
    let app = crate::notify::sender("org.magnumopus.test", "Test oznámení", 1000, false).map_err(|e| fail("odesilatel", e))?;
    let sys = crate::notify::sender("system", "Systém", 0, true).map_err(|e| fail("systemovy odesilatel", e))?;
    let v = crate::notify::viewer(1000).map_err(|e| fail("prohlizec", e))?;
    let other = crate::notify::viewer(1001).map_err(|e| fail("prohlizec jineho uzivatele", e))?;
    check!(crate::notify::sender("ma mezeru", "x", 1000, false).is_err(), "id aplikace s mezerou odmitnuto");
    for (ch, what) in [(&v, "snimek"), (&other, "snimek jineho")] {
        ch.write(&View::Subscribe.encode(), Vec::new()).map_err(|e| fail(what, e))?;
        match next_view(ch, what)? {
            ViewReply::Snapshot { last: true, .. } => {}
            r => return Err(format!("{what}: {r:?}")),
        }
        check!(matches!(next_view(ch, what)?, ViewReply::Dnd { until: 0 }), "Nerusit vypnuto");
    }

    // Odeslani: identita z initu, ne od aplikace.
    let acts = alloc::vec![Action { id: "open".into(), label: "Otevřít".into() }];
    app.write(&notify(1, "Stahování dokončeno", Priority::Normal, acts.clone()), Vec::new()).map_err(|e| fail("odeslani", e))?;
    check_eq!(read_reply(&app, "potvrzeni")?, SendReply::Sent { key: 1, popup: true }, "prijato a vyskoci");
    let n = added(&v, "nove oznameni")?;
    check_eq!((n.app_id.as_str(), n.app_name.as_str(), n.popup), ("org.magnumopus.test", "Test oznámení", true), "identita a vyskoceni");
    check!(quiet(&other), "jiny uzivatel oznameni nevidi");

    // Nahrazeni stejnym klicem: stejne cislo, nevyskakuje znovu.
    app.write(&notify(1, "Stahování 100 %", Priority::Normal, acts.clone()), Vec::new()).map_err(|e| fail("nahrazeni", e))?;
    read_reply(&app, "potvrzeni nahrazeni")?;
    let m = added(&v, "nahrazene")?;
    check_eq!((m.id, m.replaced, m.title.as_str()), (n.id, true, "Stahování 100 %"), "nahrazeni na miste");

    // Klik na akci: odesilatel dostane akci, prohlizec odebrani.
    v.write(&View::Invoke { id: n.id, action: "open".into() }.encode(), Vec::new()).map_err(|e| fail("akce", e))?;
    check_eq!(read_reply(&app, "akce u odesilatele")?, SendReply::Action { key: 1, action: "open".into() }, "akce dorucena");
    check!(matches!(next_view(&v, "odebrani")?, ViewReply::Removed { .. }), "oznameni zmizelo");

    // Nerusit: aplikace nevyskoci, system s Urgent ano; Urgent aplikace = High.
    v.write(&View::SetDnd { until: u64::MAX }.encode(), Vec::new()).map_err(|e| fail("Nerusit", e))?;
    check!(matches!(next_view(&v, "Nerusit")?, ViewReply::Dnd { until: u64::MAX }), "Nerusit zapnuto");
    app.write(&notify(2, "Falešně naléhavé", Priority::Urgent, Vec::new()), Vec::new()).map_err(|e| fail("urgent aplikace", e))?;
    check_eq!(read_reply(&app, "potvrzeni")?, SendReply::Sent { key: 2, popup: false }, "v Nerusit nevyskoci");
    let u = added(&v, "urgent aplikace")?;
    check_eq!((u.priority, u.popup), (Priority::High, false), "aplikace nesmi Urgent");
    sys.write(&notify(1, "Baterie 5 %", Priority::Urgent, Vec::new()), Vec::new()).map_err(|e| fail("systemove", e))?;
    check_eq!(read_reply(&sys, "potvrzeni systemu")?, SendReply::Sent { key: 1, popup: true }, "system projde Nerusit");
    check!(added(&v, "systemove")?.popup, "systemove vyskoci");
    check!(matches!(added(&other, "systemove u jineho")?, Notification { priority: Priority::Urgent, .. }), "systemove vidi vsichni");

    // Krizek a zavreni odesilatelem.
    v.write(&View::Dismiss { id: u.id }.encode(), Vec::new()).map_err(|e| fail("krizek", e))?;
    check!(matches!(read_reply(&app, "zavreni")?, SendReply::Closed { key: 2, .. }), "odesilatel vi o zavreni");
    sys.write(&Send::Close { key: 1 }.encode(), Vec::new()).map_err(|e| fail("zavreni systemem", e))?;
    v.write(&View::SetDnd { until: 0 }.encode(), Vec::new()).map_err(|e| fail("Nerusit vyp", e))?;
    let mut removed = false;
    for _ in 0..4 {
        if let Ok(ViewReply::Removed { .. }) = next_view(&v, "zbytek") {
            removed = true;
            break;
        }
    }
    check!(removed, "systemove zavreno odesilatelem");
    // Neplatna zprava: chyba, sluzba bezi dal.
    app.write(&[1, 2, 3], Vec::new()).map_err(|e| fail("neplatna zprava", e))?;
    check!(matches!(read_reply(&app, "neplatna")?, SendReply::Error { .. }), "neplatna zprava = chyba");
    Ok(())
}
