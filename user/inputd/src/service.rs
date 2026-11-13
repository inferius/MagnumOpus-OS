//! Stav sluzby: zdroje, vlastnik fokusu, bezpecna zkratka, opakovani
//! klaves a doruceni udalosti.


/// Levy Alt a levy Shift (evdev): prepinani rozlozeni.
const KEY_LEFTALT: u16 = 56;
const KEY_LEFTSHIFT: u16 = 42;

use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use inputproto::code::KEY_DELETE;
use inputproto::device::{self, DeviceMessage};
use inputproto::device::caps;
use inputproto::{Action, Event, DeviceTrust, Record, event};
use keymap::{Layout, LayoutId, layout};
use libos::{Channel, Error, println, time};

use crate::source::{KeyVerdict, Source, SyncWait};

/// Prodleva pred prvnim opakovanim drzene klavesy.
pub const REPEAT_DELAY: Duration = Duration::from_millis(500);
/// Interval opakovani (30 za sekundu).
pub const REPEAT_INTERVAL: Duration = Duration::from_millis(33);
/// Nejvic soucasne pripojenych zdroju vstupu.
pub const MAX_SOURCES: usize = 32;
/// Nejvic zprav od jednoho zdroje v jednom kole (ostatni zdroje se
/// mezitim nezablokuji).
pub const MAX_BATCH: usize = 64;
/// Po jake dobe zopakovat synchronizaci zdroje, ktery neodpovedel.
pub const SYNC_RETRY: Duration = Duration::from_millis(200);
/// Kolik pozadavku na synchronizaci nejvys (pak se zdroj odpoji).
pub const SYNC_TRIES: u32 = 25;

/// Opakovana klavesa.
pub struct Repeat {
    /// Index zdroje (v `Service::sources`).
    pub source: usize,
    pub code: u16,
    /// Scan kod ze stisku (opakovani hlasi stejny).
    pub scancode: u32,
    /// Cas pristiho opakovani (ns od startu).
    pub next: u64,
}

/// Prijemce udalosti (vlastnik fokusu nebo kanal `secure`) a pocet zprav,
/// ktere mu neprosly (plna fronta). Pri dalsi uspesne zprave dostane
/// napred `Dropped`, aby vedel, ze stav klaves muze byt jiny.
pub struct Receiver {
    pub channel: Channel,
    lost: u32,
}

/// Vysledek odeslani.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sent {
    Delivered,
    /// Plna fronta prijemce: zprava zahozena.
    Dropped,
    /// Prijemce zavrel kanal.
    Closed,
}

impl Receiver {
    pub fn new(channel: Channel) -> Self {
        Self { channel, lost: 0 }
    }

    /// Posle zaznamy; plna fronta = zahodit (nikdy neblokovat).
    fn send(&mut self, records: &[Record], dropped: &mut u32) -> Sent {
        if records.is_empty() {
            return Sent::Delivered;
        }
        let mut all;
        let records = if self.lost > 0 {
            all = Vec::with_capacity(records.len() + 1);
            all.push(Record::new(
                Event::Dropped { messages: self.lost },
                0,
                0,
                time::now(),
            ));
            all.extend_from_slice(records);
            &all[..]
        } else {
            records
        };
        let mut result = Sent::Delivered;
        for chunk in records.chunks(event::MAX_RECORDS) {
            match self.channel.write(&event::encode(chunk), Vec::new()) {
                Ok(()) => {
                    if result == Sent::Delivered {
                        self.lost = 0;
                    }
                }
                Err(Error::ShouldWait) => {
                    self.lost = self.lost.saturating_add(1);
                    *dropped = dropped.saturating_add(1);
                    result = Sent::Dropped;
                }
                Err(_) => return Sent::Closed,
            }
        }
        result
    }
}

pub struct Service {
    pub sources: Vec<Source>,
    pub focus: Option<Receiver>,
    pub secure: Option<Receiver>,
    /// Po bezpecne zkratce: vse jde na kanal `secure`, dokud jeho vlastnik
    /// neposle `release`.
    pub secure_mode: bool,
    pub layout: LayoutId,
    /// Rozlozeni k prepinani (levy Alt + levy Shift); prazdne = jen `layout`.
    pub layouts: Vec<LayoutId>,
    /// Drzeny levy Alt a levy Shift (prepinani rozlozeni).
    alt_shift: (bool, bool),
    pub repeat: Option<Repeat>,
    /// Zahozene zpravy (plna fronta prijemce).
    pub dropped: u32,
    /// Id pristiho zdroje (0 je vyhrazena pro udalosti sluzby).
    next_id: u16,
    /// Start dokoncen (`settled`): zarizeni pripojena potom jsou "nova".
    pub settled: bool,
    /// Volba zabezpeceni: nova klavesnice nebo mys ceka na overeni.
    pub verify_new: bool,
    /// Instance po padu predchozi (prikaz `restarted`): prvni vlastnik
    /// fokusu dostane `Focus(false)`, prvni kanal `secure` bezpecny rezim.
    restart_focus: bool,
    restart_secure: bool,
    /// Cislo pristiho pozadavku synchronizace.
    next_sync: u32,
}

impl Service {
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
            focus: None,
            secure: None,
            secure_mode: false,
            layout: LayoutId::Us,
            layouts: Vec::new(),
            alt_shift: (false, false),
            repeat: None,
            dropped: 0,
            next_id: 1,
            settled: false,
            verify_new: false,
            restart_focus: false,
            restart_secure: false,
            next_sync: 1,
        }
    }

    /// Instance po padu predchozi (`restarted`).
    pub fn set_restarted(&mut self) {
        self.restart_focus = true;
        self.restart_secure = true;
    }

    /// Zdroj `index` uz mela predchozi instance: poslat `ISYN` a do
    /// odpovedi zahazovat jeho zpravy.
    pub fn start_sync(&mut self, index: usize, tries: u32) {
        let token = self.next_sync;
        self.next_sync = self.next_sync.wrapping_add(1).max(1);
        let s = &mut self.sources[index];
        s.sync = Some(SyncWait {
            token,
            retry_at: time::now().saturating_add(SYNC_RETRY.as_nanos() as u64),
            tries,
        });
        // Plna fronta ovladace: pozadavek se zopakuje (`tick`).
        let _ = s.channel.write(&device::encode_sync(token), Vec::new());
    }

    fn layout(&self) -> &'static Layout {
        layout(self.layout)
    }

    /// Aktivni zdroje umeleho vstupu.
    pub fn injecting(&self) -> u32 {
        self.sources.iter().filter(|s| s.synthetic).count() as u32
    }

    /// Doruci zaznamy aktualnimu prijemci: v bezpecnem rezimu kanalu
    /// `secure`, jinak vlastnikovi fokusu.
    pub fn deliver(&mut self, records: &[Record]) {
        self.trace(records);
        let target = if self.secure_mode {
            &mut self.secure
        } else {
            &mut self.focus
        };
        if let Some(r) = target
            && r.send(records, &mut self.dropped) == Sent::Closed
        {
            *target = None;
            if self.secure_mode {
                self.leave_secure_mode();
            }
        }
    }

    /// Trasovani vstupu (cil logu `vstup`): stisk a pusteni tlacitek
    /// a klaves, kam jdou. Pohyb ukazatele jen jako pocet v davce. Kod
    /// klavesy se nepise (hesla by skoncila v logu), jen ze stisk byl.
    fn trace(&self, records: &[Record]) {
        let kam = if self.secure_mode { "secure" } else if self.focus.is_some() { "fokus" } else { "nikam (bez fokusu)" };
        let moves = records.iter().filter(|r| matches!(r.event, Event::Motion { .. } | Event::Absolute { .. })).count();
        for r in records {
            let (co, a) = match r.event {
                Event::Button { button, action } => (alloc::format!("tlacitko {button:?}"), action),
                Event::Key { action, .. } if action != Action::Repeat => (alloc::string::String::from("klavesa"), action),
                _ => continue,
            };
            let a = if a == Action::Press { "stisk" } else { "pusteni" };
            println!("vstup: id={} inputd {co} {a} zdroj={} mods={:#x} pohybu v davce {moves} -> {kam}", r.time / 1000, r.source, r.mods);
        }
    }

    /// Posle jednu udalost vlastnikovi fokusu (bez ohledu na rezim).
    fn notify_focus(&mut self, e: Event) {
        let r = [Record::new(e, 0, 0, time::now())];
        if let Some(f) = &mut self.focus
            && f.send(&r, &mut self.dropped) == Sent::Closed
        {
            self.focus = None;
        }
    }

    /// Posle jednu udalost na kanal `secure`.
    fn notify_secure(&mut self, e: Event) -> Sent {
        let r = [Record::new(e, 0, 0, time::now())];
        let Some(s) = &mut self.secure else {
            return Sent::Closed;
        };
        let sent = s.send(&r, &mut self.dropped);
        if sent == Sent::Closed {
            self.secure = None;
        }
        sent
    }

    /// Novy vlastnik fokusu: predchozi dostane `Focus(false)`, novy uvodni
    /// stav (fokus, rozlozeni, umely vstup).
    pub fn set_focus(&mut self, ch: Channel) {
        self.notify_focus(Event::Focus(false));
        self.focus = Some(Receiver::new(ch));
        if core::mem::take(&mut self.restart_focus) {
            // Stara instance mohla nechat klavesy stisknute.
            self.notify_focus(Event::Focus(false));
        }
        self.notify_focus(Event::Focus(!self.secure_mode));
        self.notify_focus(Event::Layout(self.layout as u32));
        let active = self.injecting();
        self.notify_focus(Event::Injection { active });
    }

    /// Novy prijemce bezpecne zkratky; dostane stav umeleho vstupu.
    pub fn set_secure(&mut self, ch: Channel) {
        self.secure = Some(Receiver::new(ch));
        let active = self.injecting();
        self.notify_secure(Event::Injection { active });
        if core::mem::take(&mut self.restart_secure) {
            // Stara instance mohla byt v bezpecnem rezimu (zadavani hesla):
            // radeji do nej, nez aby vstup sel aplikaci. System ho ukonci
            // `release` jako po Ctrl+Alt+Delete.
            println!("inputd: po restartu v bezpecnem rezimu");
            self.secure_attention();
        }
    }

    /// Kanal `secure` se zavrel: bezpecny rezim konci.
    pub fn secure_closed(&mut self) {
        self.secure = None;
        self.leave_secure_mode();
    }

    pub fn set_layout(&mut self, id: LayoutId) {
        self.layout = id;
        let r = [Record::new(Event::Layout(id as u32), 0, 0, time::now())];
        self.deliver(&r);
    }

    pub fn add_source(&mut self, ch: Channel, synthetic: bool) -> libos::Result<()> {
        if self.sources.len() >= MAX_SOURCES {
            return Err(Error::NoResources);
        }
        // Id se neopakuje, dokud citac nepretece (65535 pripojeni).
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).unwrap_or(1);
        self.sources.push(Source::new(ch, id, synthetic));
        if synthetic {
            self.injection_changed();
        }
        Ok(())
    }

    /// Indikator umeleho vstupu vlastnikovi fokusu i kanalu `secure`.
    fn injection_changed(&mut self) {
        let active = self.injecting();
        self.notify_focus(Event::Injection { active });
        if self.secure.is_some() {
            self.notify_secure(Event::Injection { active });
        }
    }

    pub fn remove_source(&mut self, index: usize) {
        let s = self.sources.remove(index);
        if !s.synthetic {
            println!("inputd: zarizeni '{}' odpojeno", s.name);
        }
        self.repeat = match self.repeat.take() {
            Some(r) if r.source == index => None,
            Some(mut r) if r.source > index => {
                r.source -= 1;
                Some(r)
            }
            other => other,
        };
        if s.synthetic {
            self.injection_changed();
        }
    }

    /// Bezpecna zkratka: oznameni systemu a bezpecny rezim. Do bezpecneho
    /// rezimu jen tehdy, kdyz system oznameni opravdu dostal - jinak by
    /// vstup zustal u prijemce, ktery o nem nevi a nikdy ho neuvolni.
    fn secure_attention(&mut self) {
        self.repeat = None;
        if self.secure.is_none() {
            println!("inputd: bezpecna zkratka, ale nikdo ji neposloucha");
            return;
        }
        match self.notify_secure(Event::SecureAttention) {
            Sent::Delivered => {
                if !self.secure_mode {
                    self.notify_focus(Event::Focus(false));
                    self.secure_mode = true;
                }
            }
            Sent::Dropped => {
                println!("inputd: bezpecna zkratka neprosla, kanal secure je plny");
            }
            Sent::Closed => {
                println!("inputd: bezpecna zkratka, ale kanal secure je zavreny");
                self.leave_secure_mode();
            }
        }
    }

    pub fn leave_secure_mode(&mut self) {
        if self.secure_mode {
            self.secure_mode = false;
            self.notify_focus(Event::Focus(true));
        }
    }

    /// Zpracuje zpravu od zdroje `index`.
    pub fn source_message(&mut self, index: usize, data: &[u8]) {
        let msg = device::decode(data);
        if let Some(w) = self.sources[index].sync {
            // Do odpovedi na posledni pozadavek jsou to stare zpravy.
            if msg == Some(DeviceMessage::Sync(w.token)) {
                self.sources[index].synced();
            }
            return;
        }
        let Some(msg) = msg else {
            println!("inputd: neplatna zprava od zdroje {index}");
            return;
        };
        if self.sources[index].hello(&msg) {
            let s = &mut self.sources[index];
            if !s.synthetic {
                println!("inputd: zarizeni '{}' pripojeno", s.name);
            }
            // Klavesnice nebo mys pripojena za behu muze byt podvrzena
            // (BadUSB: pise prikazy, klika do dialogu): s volbou overeni jde
            // jeji vstup jen systemu, dokud ji neoveri.
            let input =
                s.caps & (caps::KEYBOARD | caps::POINTER | caps::ABSOLUTE | caps::MEDIA) != 0;
            if !s.synthetic && input && self.settled && self.verify_new {
                s.trust = DeviceTrust::Pending;
                let id = s.id;
                println!("inputd: nove zarizeni '{}' ceka na overeni", s.name);
                self.trust_changed(id, DeviceTrust::Pending);
            }
            return;
        }
        let events = match msg {
            DeviceMessage::Events(e) => e,
            DeviceMessage::Axes(a) => {
                self.sources[index].set_axes(&a);
                return;
            }
            DeviceMessage::Hello { .. } | DeviceMessage::Sync(_) => return,
        };
        let now = time::now();
        let layout = self.layout();
        let mut out = Vec::new();
        for e in events {
            let mut sak = false;
            let mut repeat = None::<Option<Repeat>>;
            let current = self.repeat.as_ref().map(|r| (r.source, r.code));
            let trusted = self.sources[index].trust == DeviceTrust::Trusted;
            let mut cycle = false;
            let alt_shift = &mut self.alt_shift;
            self.sources[index].raw(e, layout, now, &mut out, |s, code, scancode, action| {
                // Levy Alt + levy Shift = dalsi rozlozeni (stisk druhe z nich).
                let down = action != Action::Release;
                match code {
                    KEY_LEFTALT => {
                        cycle |= action == Action::Press && alt_shift.1;
                        alt_shift.0 = down;
                    }
                    KEY_LEFTSHIFT => {
                        cycle |= action == Action::Press && alt_shift.0;
                        alt_shift.1 = down;
                    }
                    _ => {}
                }
                // Bezpecnou zkratku umi jen fyzicka klavesnice, ktera neni
                // ve stavu overovani.
                if !s.synthetic
                    && trusted
                    && code == KEY_DELETE
                    && action == Action::Press
                    && s.keyboard.ctrl_alt_held()
                {
                    sak = true;
                    return KeyVerdict::SecureAttention;
                }
                match action {
                    // Opakovani jen u duveryhodnych zdroju (doruceni opakovani
                    // jde vlastnikovi fokusu).
                    Action::Press if !keymap::is_modifier(code) && trusted => {
                        repeat = Some(Some(Repeat {
                            source: index,
                            code,
                            scancode,
                            next: now.saturating_add(REPEAT_DELAY.as_nanos() as u64),
                        }));
                    }
                    Action::Release if current == Some((index, code)) => repeat = Some(None),
                    _ => {}
                }
                KeyVerdict::Deliver
            });
            if let Some(r) = repeat {
                self.repeat = r;
            }
            if cycle && self.layouts.len() > 1 {
                let i = self.layouts.iter().position(|&l| l == self.layout).map_or(0, |i| (i + 1) % self.layouts.len());
                let next = self.layouts[i];
                self.set_layout(next);
            }
            if sak {
                // Co prislo pred zkratkou, patri dosavadnimu prijemci; vse
                // za ni uz jde na kanal `secure`.
                self.tag(index, &mut out);
                self.deliver_frame(&mut out, layout);
                self.secure_attention();
            }
        }
        // Konec zpravy uzavre rozpracovany snimek (pohyb a kolecko).
        self.sources[index].flush(layout, now, &mut out);
        self.tag(index, &mut out);
        match self.sources[index].trust {
            DeviceTrust::Trusted => self.deliver_frame(&mut out, layout),
            // Ceka na overeni: vstup vidi jen system (dialog overeni).
            DeviceTrust::Pending => {
                if let Some(s) = &mut self.secure
                    && s.send(&out, &mut self.dropped) == Sent::Closed
                {
                    self.secure = None;
                }
            }
            DeviceTrust::Rejected => {}
        }
    }

    /// Oznameni o overeni klavesnice vlastnikovi fokusu i kanalu `secure`.
    fn trust_changed(&mut self, source: u16, state: DeviceTrust) {
        let e = Event::NewDevice { source, state };
        self.notify_focus(e);
        if self.secure.is_some() {
            self.notify_secure(e);
        }
    }

    /// Rozhodnuti systemu o nove klavesnici (`trust` / `reject` na kanalu
    /// `secure`). Neznamy zdroj nebo zdroj, ktery neceka = nic.
    pub fn decide(&mut self, source: u16, state: DeviceTrust) {
        let Some(s) = self
            .sources
            .iter_mut()
            .find(|s| s.id == source && s.trust == DeviceTrust::Pending)
        else {
            return;
        };
        s.trust = state;
        println!(
            "inputd: zarizeni '{}' {}",
            s.name,
            if state == DeviceTrust::Trusted { "overeno" } else { "odmitnuto" }
        );
        self.trust_changed(source, state);
    }

    /// Oznaci zaznamy zdrojem `index`.
    fn tag(&self, index: usize, out: &mut [Record]) {
        let id = self.sources[index].id;
        for r in out {
            r.source = id;
        }
    }

    /// Doplni modifikatory udalostem mysi, doruci zaznamy a vyprazdni `out`.
    fn deliver_frame(&mut self, out: &mut Vec<Record>, layout: &Layout) {
        // Udalosti mysi nesou modifikatory vsech duveryhodnych klavesnic
        // (Ctrl+klik); zarizeni cekajici na overeni je ovlivnit nesmi.
        let all = self
            .sources
            .iter()
            .filter(|s| s.trust == DeviceTrust::Trusted)
            .fold(0, |m, s| m | s.keyboard.mods(layout));
        for r in out.iter_mut() {
            if matches!(
                r.event,
                Event::Motion { .. }
                    | Event::Absolute { .. }
                    | Event::Button { .. }
                    | Event::Scroll { .. }
            ) {
                r.mods |= all;
            }
        }
        self.deliver(out);
        out.clear();
    }

    /// Termin pristiho opakovani klavesy.
    pub fn next_deadline(&self) -> Option<u64> {
        let syncs = self.sources.iter().filter_map(|s| s.sync.map(|w| w.retry_at));
        self.repeat.as_ref().map(|r| r.next).into_iter().chain(syncs).min()
    }

    /// Opakovani drzene klavesy a synchronizace zdroju, pokud nastal jejich
    /// cas.
    pub fn tick(&mut self) {
        self.sync_tick();
        self.repeat_tick();
    }

    /// Zdroje, ktere neodpovedely na synchronizaci: novy pozadavek, po
    /// [`SYNC_TRIES`] odpojit.
    fn sync_tick(&mut self) {
        let now = time::now();
        for i in (0..self.sources.len()).rev() {
            let Some(w) = self.sources[i].sync else { continue };
            if w.retry_at > now {
                continue;
            }
            if w.tries >= SYNC_TRIES {
                println!(
                    "inputd: zdroj {} neodpovida na synchronizaci, odpojuji",
                    self.sources[i].id
                );
                self.remove_source(i);
                continue;
            }
            self.start_sync(i, w.tries + 1);
        }
    }

    fn repeat_tick(&mut self) {
        let now = time::now();
        let Some(r) = &mut self.repeat else {
            return;
        };
        if now < r.next {
            return;
        }
        r.next = now.saturating_add(REPEAT_INTERVAL.as_nanos() as u64);
        let (index, code, scancode) = (r.source, r.code, r.scancode);
        let layout = self.layout();
        let mut out = vec![];
        self.sources[index].key(layout, code, scancode, Action::Repeat, now, &mut out);
        self.tag(index, &mut out);
        self.deliver(&out);
    }
}
