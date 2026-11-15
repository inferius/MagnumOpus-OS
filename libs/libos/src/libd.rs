//! Klient sluzby knihoven `libd` (`libdproto`, `docs/cs/dynamicke-linkovani.md`).
//!
//! Proces ma nejvys jednu relaci s libd a k ni **listek** (obnova po padu,
//! `docs/cs/obnova-sluzeb.md`). Relaci dostane:
//!
//! - zavadec `ld.so` ve zprave zavadece (kazdy program s `PT_INTERP`),
//! - staticky program jen s `spawn::Options::libd` (spoustece: shell,
//!   launchd, relace) - ostatni staticke programy zadnou (zadna rezie),
//! - program pod zavadecem az pri prvni potrebe od zavadece
//!   (`abi::loader::Interface::libd_session`),
//! - init primo od adapteru sluzby (korenove spojeni, bez listku).
//!
//! Kdyz libd spadne (`PEER_CLOSED`), klient pozada pres listek supervisor
//! o novou relaci (`RECONNECT`) a pozadavek zopakuje (vsechny jsou
//! idempotentni). Relace a listek pro dite ([`child`]) vzniknou **bez
//! cekani**: `Connect` na vlastni relaci (libd, bez odpovedi) a `ADOPT` na
//! vlastnim listku (supervisor, bez odpovedi) - spawn tak na libd ani init
//! neceka. U initu da relaci adapter z korene a listek supervisor primo
//! (poskytovatel [`set_provider`]).
//!
//! Odmitnuti bez odpovedi se ukaze pri prvnim pouziti, nikdy visenim:
//! relace zavrena libd (limit relaci procesu) = `RECONNECT` vrati
//! `PEER_CLOSED` (instance bezi dal, supervisor to pozna do 200 ms);
//! listek zavreny supervisorem (limit `ADOPT`) = `SERVICE_UNAVAILABLE`.

use alloc::vec::Vec;
use core::time::Duration;

use abi::{Error, Result};
use leaseproto::{Reply as LeaseReply, Request as LeaseRequest};
use libdproto::{Library, Reply, Request, Stats};

use crate::sync::Mutex;
use crate::{Channel, Handle, Vmo};

struct Conn {
    session: Option<Channel>,
    lease: Option<Channel>,
    /// Cislo dalsiho pozadavku na listku.
    next_id: u32,
}

static CONN: Mutex<Conn> = Mutex::new(Conn { session: None, lease: None, next_id: 1 });

/// Zdroj relaci pro deti v initu (supervisor bez IPC sam na sebe).
type Provider = fn() -> Result<(Channel, Channel)>;
static PROVIDER: Mutex<Option<Provider>> = Mutex::new(None);

/// Jak dlouho cekat na odpoved libd.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Jak dlouho cekat na novou instanci libd po padu (supervisor).
const WAIT_MS: u32 = 10_000;

/// Nastavi relaci s libd a listek k ni (start procesu ze startovni zpravy,
/// zavadec ze zpravy zavadece, init korenove spojeni bez listku).
pub fn set_connection(session: Channel, lease: Option<Channel>) {
    let mut c = CONN.lock();
    c.session = Some(session);
    if lease.is_some() {
        c.lease = lease;
    }
}

/// Listek ze startovni zpravy (prijde za relaci).
pub fn set_lease(lease: Channel) {
    CONN.lock().lease = Some(lease);
}

/// Init: relace pro deti dava supervisor primo (ne `NARROW` na listku).
pub fn set_provider(f: Provider) {
    *PROVIDER.lock() = Some(f);
}

/// Muze proces mluvit s libd (ma relaci, listek, poskytovatele nebo
/// zavadec, ktery relaci da)?
pub fn available() -> bool {
    let c = CONN.lock();
    c.session.is_some() || c.lease.is_some() || PROVIDER.lock().is_some() || from_loader_possible()
}

/// Konec pole `libd_session` v tabulce zavadece (starsi zavadec ho nema).
const LIBD_SESSION_END: usize = core::mem::offset_of!(abi::loader::Interface, libd_session) + size_of::<usize>();

fn from_loader_possible() -> bool {
    crate::loader::interface().is_some_and(|i| i.size as usize >= LIBD_SESSION_END)
}

/// Relace a listek od zavadece (program pod zavadecem, prvni pouziti).
fn from_loader() -> Result<(Channel, Channel)> {
    let i = crate::loader::interface().ok_or(Error::NotFound)?;
    if (i.size as usize) < LIBD_SESSION_END {
        return Err(Error::NotSupported);
    }
    let (mut s, mut l) = (0u32, 0u32);
    // SAFETY: funkce zavadece zapise dva handly (nebo vrati chybu).
    let r = unsafe { (i.libd_session)(&mut s, &mut l) };
    if r != 0 {
        return Err(Error::from_code(i64::from(r)).unwrap_or(Error::IoError));
    }
    Ok((Channel::from_handle(Handle::from_raw(s)), Channel::from_handle(Handle::from_raw(l))))
}

/// Zavreny listek (supervisor ho odmitl nebo zrusil) = sluzba nedostupna.
fn lease_err(e: Error) -> Error {
    if e == Error::PeerClosed {
        crate::println!("libd: listek k libd zavren (odmitnuty ADOPT nebo zrusen): obnova relace neni");
        Error::ServiceUnavailable
    } else {
        e
    }
}

/// Pozadavek na listku a odpoved (se stejnym `id`; pozdni odpovedi jinych
/// pozadavku se zahodi).
fn lease_call(c: &mut Conn, req: LeaseRequest, wait: Duration) -> Result<Vec<Handle>> {
    let lease = c.lease.as_ref().ok_or(Error::ServiceUnavailable)?;
    let id = req.id();
    lease.write(&req.encode(), Vec::new()).map_err(lease_err)?;
    let deadline = crate::time::deadline_after(wait + Duration::from_secs(5));
    loop {
        let m = lease.read(Some(deadline)).map_err(lease_err)?;
        let r = LeaseReply::decode(&m.data)?;
        if r.id != id {
            continue;
        }
        if r.status == Err(Error::PeerClosed) {
            // Instance bezi, relaci zavrela sama: odmitnuti (limit relaci).
            crate::println!("libd: relace odmitnuta sluzbou knihoven (limit relaci procesu); listek zanikl");
            c.lease = None;
        }
        r.status?;
        if m.handles.len() != LeaseReply::handles_for(r.op) {
            return Err(Error::Corrupted);
        }
        return Ok(m.handles);
    }
}

fn next_id(c: &mut Conn) -> u32 {
    let id = c.next_id;
    c.next_id = c.next_id.wrapping_add(1).max(1);
    id
}

/// Nova relace pres listek po padu libd.
fn reconnect(c: &mut Conn) -> Result<()> {
    c.session = None;
    let id = next_id(c);
    let mut h = lease_call(c, LeaseRequest::Reconnect { id, wait_ms: WAIT_MS }, Duration::from_millis(WAIT_MS.into()))?;
    c.session = Some(Channel::from_handle(h.remove(0)));
    Ok(())
}

/// Relace musi byt: chybi-li, od zavadece (program pod nim) nebo pres
/// listek.
fn ensure(c: &mut Conn) -> Result<()> {
    if c.session.is_some() {
        return Ok(());
    }
    if c.lease.is_some() {
        return reconnect(c);
    }
    let (s, l) = from_loader()?;
    c.session = Some(s);
    c.lease = Some(l);
    Ok(())
}

/// Zadost a odpoved (spojeni je pod zamkem, odpovedi se nemichaji). Po padu
/// libd jednou obnovi relaci pres listek a zadost zopakuje (vsechny zadosti
/// klienta jsou idempotentni a bez handlu).
fn call(req: &Request) -> Result<(Reply, Vec<Handle>)> {
    let mut c = CONN.lock();
    ensure(&mut c)?;
    let data = req.encode();
    let once = |c: &Conn| -> Result<crate::Message> {
        let ch = c.session.as_ref().ok_or(Error::NotFound)?;
        ch.write(&data, Vec::new())?;
        ch.read(Some(crate::time::deadline_after(TIMEOUT)))
    };
    let msg = match once(&c) {
        Err(Error::PeerClosed) if c.lease.is_some() => {
            reconnect(&mut c)?;
            once(&c)?
        }
        r => r?,
    };
    let reply = Reply::decode(&msg.data).ok_or(Error::Corrupted)?;
    if let Reply::Error(code) = reply {
        return Err(Error::from_code(i64::from(code)).unwrap_or(Error::IoError));
    }
    Ok((reply, msg.handles))
}

/// Nova relace z vlastni (`Connect`, bez odpovedi).
fn connect_from(c: &Conn) -> Result<Channel> {
    let s = c.session.as_ref().ok_or(Error::NotFound)?;
    let (ours, theirs) = Channel::create()?;
    s.write(&Request::Connect.encode(), alloc::vec![theirs.into_handle()])?;
    Ok(ours)
}

/// Init (adapter sluzby): nova relace pres korenove spojeni. Bez cekani
/// na libd (`Connect` nema odpoved); libd ji odmitne zavrenim.
pub fn connect_root() -> Result<Channel> {
    connect_from(&CONN.lock())
}

/// Relace a listek pro dite (spawn programu se zavadecem nebo spoustece),
/// **bez cekani**: u initu od adapteru a supervisoru primo, jinak `Connect`
/// na vlastni relaci a `ADOPT` na vlastnim listku.
pub fn child() -> Result<(Channel, Channel)> {
    let provider = *PROVIDER.lock();
    if let Some(f) = provider {
        return f();
    }
    let mut c = CONN.lock();
    ensure(&mut c)?;
    let session = match connect_from(&c) {
        // Vlastni relace na mrtve instanci: nejdriv obnovit.
        Err(Error::PeerClosed) if c.lease.is_some() => {
            reconnect(&mut c)?;
            connect_from(&c)?
        }
        r => r?,
    };
    let (lease, server) = Channel::create()?;
    let id = next_id(&mut c);
    let own = c.lease.as_ref().ok_or(Error::ServiceUnavailable)?;
    own.write(&LeaseRequest::Adopt { id }.encode(), alloc::vec![server.into_handle()]).map_err(lease_err)?;
    Ok((session, lease))
}

/// Knihovna `name` z `/system/lib`: rozlozeni a VMO segmentu (kod
/// zapeceteny ke sdileni, data jako sablona pro klon s kopii pri zapisu).
pub fn open(name: &str) -> Result<(Library, Vec<Vmo>)> {
    if !libdproto::valid_name(name) {
        return Err(Error::InvalidArgs);
    }
    match call(&Request::Open { name: alloc::string::String::from(name) })? {
        (Reply::Library(l), handles) if handles.len() == l.segments.len() => {
            Ok((l, handles.into_iter().map(Vmo::from_handle).collect()))
        }
        _ => Err(Error::Corrupted),
    }
}

/// Statistika libd (sablony, zasahy mezipameti, spojeni).
pub fn stats() -> Result<Stats> {
    match call(&Request::Stats)? {
        (Reply::Stats(s), _) => Ok(s),
        _ => Err(Error::Corrupted),
    }
}
