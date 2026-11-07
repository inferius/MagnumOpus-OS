//! Stav sluzby: zdroje, vlastnik fokusu, bezpecna zkratka, opakovani
//! klaves a doruceni udalosti.

use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use inputproto::code::KEY_DELETE;
use inputproto::device::{self, DeviceMessage};
use inputproto::{Action, Event, Record, event};
use keymap::{Layout, LayoutId, layout};
use libos::{Channel, Error, println, time};

use crate::source::{KeyVerdict, Source};

/// Prodleva pred prvnim opakovanim drzene klavesy.
pub const REPEAT_DELAY: Duration = Duration::from_millis(500);
/// Interval opakovani (30 za sekundu).
pub const REPEAT_INTERVAL: Duration = Duration::from_millis(33);
/// Nejvic soucasne pripojenych zdroju vstupu.
pub const MAX_SOURCES: usize = 32;
/// Nejvic zprav od jednoho zdroje v jednom kole (ostatni zdroje se
/// mezitim nezablokuji).
pub const MAX_BATCH: usize = 64;

/// Opakovana klavesa.
pub struct Repeat {
    /// Index zdroje (v `Service::sources`).
    pub source: usize,
    pub code: u16,
    pub scancode: u32,
    /// Cas pristiho opakovani (ns od startu).
    pub next: u64,
}

pub struct Service {
    pub sources: Vec<Source>,
    pub focus: Option<Channel>,
    pub secure: Option<Channel>,
    /// Po bezpecne zkratce: vse jde na kanal `secure`, dokud jeho vlastnik
    /// neposle `release`.
    pub secure_mode: bool,
    pub layout: LayoutId,
    pub repeat: Option<Repeat>,
    /// Zahozene zpravy (plna fronta prijemce).
    pub dropped: u32,
}

impl Service {
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
            focus: None,
            secure: None,
            secure_mode: false,
            layout: LayoutId::Us,
            repeat: None,
            dropped: 0,
        }
    }

    fn layout(&self) -> &'static Layout {
        layout(self.layout)
    }

    /// Aktivni zdroje umeleho vstupu.
    pub fn injecting(&self) -> u32 {
        self.sources.iter().filter(|s| s.synthetic).count() as u32
    }

    /// Posle zaznamy prijemci; plna fronta = zahodit (nikdy neblokovat).
    /// Vraci `false`, kdyz prijemce kanal zavrel.
    fn send_to(channel: &Channel, records: &[Record], dropped: &mut u32) -> bool {
        if records.is_empty() {
            return true;
        }
        for chunk in records.chunks(event::MAX_RECORDS) {
            match channel.write(&event::encode(chunk), Vec::new()) {
                Ok(()) => {}
                Err(Error::ShouldWait) => *dropped = dropped.saturating_add(1),
                Err(_) => return false,
            }
        }
        true
    }

    /// Doruci zaznamy aktualnimu prijemci: v bezpecnem rezimu kanalu
    /// `secure`, jinak vlastnikovi fokusu.
    pub fn deliver(&mut self, records: &[Record]) {
        let target = if self.secure_mode {
            &mut self.secure
        } else {
            &mut self.focus
        };
        if let Some(ch) = target
            && !Self::send_to(ch, records, &mut self.dropped)
        {
            *target = None;
            if self.secure_mode {
                self.leave_secure_mode();
            }
        }
    }

    /// Posle jednu udalost vlastnikovi fokusu (bez ohledu na rezim).
    fn notify_focus(&mut self, e: Event) {
        let r = [Record::new(e, 0, 0, time::now())];
        if let Some(ch) = &self.focus
            && !Self::send_to(ch, &r, &mut self.dropped)
        {
            self.focus = None;
        }
    }

    /// Novy vlastnik fokusu: predchozi dostane `Focus(false)`, novy uvodni
    /// stav (fokus, rozlozeni, umely vstup).
    pub fn set_focus(&mut self, ch: Channel) {
        self.notify_focus(Event::Focus(false));
        self.focus = Some(ch);
        self.notify_focus(Event::Focus(!self.secure_mode));
        self.notify_focus(Event::Layout(self.layout as u32));
        let active = self.injecting();
        self.notify_focus(Event::Injection { active });
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
        self.sources.push(Source::new(ch, synthetic));
        if synthetic {
            self.injection_changed();
        }
        Ok(())
    }

    /// Indikator umeleho vstupu vlastnikovi fokusu i kanalu `secure`.
    fn injection_changed(&mut self) {
        let active = self.injecting();
        self.notify_focus(Event::Injection { active });
        if let Some(ch) = &self.secure {
            let r = [Record::new(Event::Injection { active }, 0, 0, time::now())];
            if !Self::send_to(ch, &r, &mut self.dropped) {
                self.secure = None;
            }
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

    /// Bezpecna zkratka: oznameni systemu a bezpecny rezim.
    fn secure_attention(&mut self) {
        self.repeat = None;
        let Some(ch) = &self.secure else {
            println!("inputd: bezpecna zkratka, ale nikdo ji neposloucha");
            return;
        };
        let r = [Record::new(Event::SecureAttention, 0, 0, time::now())];
        if !Self::send_to(ch, &r, &mut self.dropped) {
            self.secure = None;
            return;
        }
        if !self.secure_mode {
            self.notify_focus(Event::Focus(false));
            self.secure_mode = true;
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
        let Some(msg) = device::decode(data) else {
            println!("inputd: neplatna zprava od zdroje {index}");
            return;
        };
        if self.sources[index].hello(&msg) {
            let s = &self.sources[index];
            if !s.synthetic {
                println!("inputd: zarizeni '{}' pripojeno", s.name);
            }
            return;
        }
        let DeviceMessage::Events(events) = msg else {
            return;
        };
        let now = time::now();
        let layout = self.layout();
        let mut out = Vec::new();
        for e in events {
            let mut sak = false;
            let mut repeat = None::<Option<Repeat>>;
            let current = self.repeat.as_ref().map(|r| (r.source, r.code));
            self.sources[index].raw(e, layout, now, &mut out, |s, code, action| {
                // Bezpecnou zkratku umi jen fyzicka klavesnice.
                if !s.synthetic
                    && code == KEY_DELETE
                    && action == Action::Press
                    && s.keyboard.ctrl_alt_held()
                {
                    sak = true;
                    return KeyVerdict::SecureAttention;
                }
                match action {
                    Action::Press if !keymap::is_modifier(code) => {
                        repeat = Some(Some(Repeat {
                            source: index,
                            code,
                            scancode: code as u32,
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
            if sak {
                // Co prislo pred zkratkou, patri dosavadnimu prijemci; vse
                // za ni uz jde na kanal `secure`.
                self.deliver_frame(&mut out, layout);
                self.secure_attention();
            }
        }
        // Konec zpravy uzavre rozpracovany snimek (pohyb a kolecko).
        self.sources[index].flush(layout, now, &mut out);
        self.deliver_frame(&mut out, layout);
    }

    /// Doplni modifikatory udalostem mysi, doruci zaznamy a vyprazdni `out`.
    fn deliver_frame(&mut self, out: &mut Vec<Record>, layout: &Layout) {
        // Udalosti mysi nesou modifikatory vsech klavesnic (Ctrl+klik).
        let all = self
            .sources
            .iter()
            .fold(0, |m, s| m | s.keyboard.mods(layout));
        for r in out.iter_mut() {
            if matches!(
                r.event,
                Event::Motion { .. } | Event::Button { .. } | Event::Scroll { .. }
            ) {
                r.mods |= all;
            }
        }
        self.deliver(out);
        out.clear();
    }

    /// Termin pristiho opakovani klavesy.
    pub fn next_deadline(&self) -> Option<u64> {
        self.repeat.as_ref().map(|r| r.next)
    }

    /// Opakovani drzene klavesy, pokud nastal jeho cas.
    pub fn tick(&mut self) {
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
        self.deliver(&out);
    }
}
