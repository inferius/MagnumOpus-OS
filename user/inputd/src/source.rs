//! Zdroj vstupu: ovladac zarizeni nebo umely vstup. Sklada surove udalosti
//! (evdev) do snimku a prevadi je na udalosti protokolu.

use alloc::string::String;
use alloc::vec::Vec;

use inputproto::code::*;
use inputproto::device::{DeviceMessage, RawEvent, caps};
use inputproto::event::flags;
use inputproto::{Action, Button, Event, Record};
use keymap::{Keyboard, Layout};
use libos::Channel;

/// Zdroj vstupu a jeho stav: klavesnice (modifikatory, mrtva klavesa),
/// rozpracovany snimek (pohyb, kolecko) a pohlcene klavesy.
pub struct Source {
    pub channel: Channel,
    /// Id v zaznamech (`Record::source`), jedinecne po dobu behu sluzby.
    pub id: u16,
    /// Umely vstup (z prikazu `inject`), ne fyzicke zarizeni.
    pub synthetic: bool,
    pub caps: u16,
    pub name: String,
    pub keyboard: Keyboard,
    /// Scan kod z `MSC_SCAN` pro pristi klavesu.
    scan: Option<u32>,
    dx: i32,
    dy: i32,
    wheel_x: i32,
    wheel_y: i32,
    /// Klavesy, jejichz stisk sluzba pohltila (bezpecna zkratka): pohltit
    /// i jejich uvolneni.
    swallowed: Vec<u16>,
    /// Overeni klavesnice pripojene za behu (volba zabezpeceni); ostatni
    /// zdroje jsou `Trusted`.
    pub trust: inputproto::DeviceTrust,
    /// Rozsahy absolutnich os X a Y (z `IAXS`).
    axes: [(i32, i32); 2],
    /// Posledni absolutni hodnoty X a Y (snimek muze nest jen jednu osu).
    abs: [i32; 2],
    abs_dirty: bool,
}

/// Prevod hodnoty osy z rozsahu `min..=max` na 0..=65535.
fn normalize(v: i32, (min, max): (i32, i32)) -> u16 {
    if max <= min {
        return 0;
    }
    let v = v.clamp(min, max) as i64 - min as i64;
    (v * 65535 / (max as i64 - min as i64)) as u16
}

/// Co ma sluzba udelat s udalosti klavesy (rozhoduje o bezpecne zkratce
/// a opakovani, ktere zavisi na stavu cele sluzby).
pub enum KeyVerdict {
    /// Predat dal (zaznamy uz jsou ve snimku).
    Deliver,
    /// Bezpecna zkratka: zaznamy se nepredavaji.
    SecureAttention,
}

impl Source {
    pub fn new(channel: Channel, id: u16, synthetic: bool) -> Self {
        Self {
            channel,
            id,
            synthetic,
            caps: 0,
            name: String::new(),
            keyboard: Keyboard::new(),
            scan: None,
            dx: 0,
            dy: 0,
            wheel_x: 0,
            wheel_y: 0,
            swallowed: Vec::new(),
            trust: inputproto::DeviceTrust::Trusted,
            axes: [(0, 65535); 2],
            abs: [0; 2],
            abs_dirty: false,
        }
    }

    pub fn flags(&self) -> u16 {
        if self.synthetic { flags::SYNTHETIC } else { 0 }
    }

    pub fn is_keyboard(&self) -> bool {
        !self.synthetic && self.caps & caps::KEYBOARD != 0
    }

    pub fn is_pointer(&self) -> bool {
        !self.synthetic && self.caps & caps::POINTER != 0
    }

    /// Rozsahy absolutnich os (`IAXS`); nezname osy a prazdny rozsah se
    /// ignoruji.
    pub fn set_axes(&mut self, axes: &[inputproto::device::AxisInfo]) {
        for a in axes {
            if a.max > a.min && (a.code == ABS_X || a.code == ABS_Y) {
                self.axes[a.code as usize] = (a.min, a.max);
            }
        }
    }

    /// Zpracuje predstaveni zdroje; vraci `false`, kdyz zprava neni
    /// predstaveni.
    pub fn hello(&mut self, msg: &DeviceMessage<'_>) -> bool {
        if let DeviceMessage::Hello { caps, name, .. } = msg {
            self.caps = *caps;
            self.name = String::from(*name);
            true
        } else {
            false
        }
    }

    /// Zpracuje jednu surovou udalost. Hotove zaznamy prida do `out`;
    /// klavesy hlasi pres `on_key` (sluzba rozhodne o bezpecne zkratce
    /// a opakovani). Vraci `true` na konci snimku.
    pub fn raw(
        &mut self,
        e: RawEvent,
        layout: &Layout,
        time: u64,
        out: &mut Vec<Record>,
        mut on_key: impl FnMut(&Self, u16, u32, Action) -> KeyVerdict,
    ) -> bool {
        match e.kind {
            EV_MSC if e.code == MSC_SCAN => self.scan = Some(e.value as u32),
            // Tlacitka (mys, joystick, gamepad, tablet) nejsou klavesy:
            // nejdou pres rozlozeni a neopakuji se.
            EV_KEY if is_button(e.code) => {
                let action = match e.value {
                    0 => Action::Release,
                    1 => Action::Press,
                    _ => return false,
                };
                let mods = self.keyboard.mods(layout);
                out.push(Record::new(
                    Event::Button {
                        button: Button::from_code(e.code),
                        action,
                    },
                    mods,
                    self.flags(),
                    time,
                ));
            }
            EV_KEY if e.code <= KEY_MAX => {
                // Opakovani od zarizeni ignorujeme: opakuje sluzba sama
                // (stejne tempo pro vsechna zarizeni).
                let action = match e.value {
                    0 => Action::Release,
                    1 => Action::Press,
                    _ => return false,
                };
                let scancode = self.scan.take().unwrap_or(e.code as u32);
                if action == Action::Release
                    && let Some(i) = self.swallowed.iter().position(|&c| c == e.code)
                {
                    self.swallowed.swap_remove(i);
                    return false;
                }
                if let KeyVerdict::SecureAttention = on_key(self, e.code, scancode, action) {
                    self.swallowed.push(e.code);
                    return false;
                }
                self.key(layout, e.code, scancode, action, time, out);
            }
            EV_ABS if e.code == ABS_X || e.code == ABS_Y => {
                self.abs[e.code as usize] = e.value;
                self.abs_dirty = true;
            }
            EV_REL => match e.code {
                REL_X => self.dx = self.dx.saturating_add(e.value),
                REL_Y => self.dy = self.dy.saturating_add(e.value),
                REL_WHEEL => self.wheel_y = self.wheel_y.saturating_add(e.value),
                REL_HWHEEL => self.wheel_x = self.wheel_x.saturating_add(e.value),
                _ => {}
            },
            EV_SYN if e.code == SYN_REPORT => {
                self.flush(layout, time, out);
                return true;
            }
            _ => {}
        }
        false
    }

    /// Klavesa: zaznam `Key` a pripadne `Text`.
    pub fn key(
        &mut self,
        layout: &Layout,
        code: u16,
        scancode: u32,
        action: Action,
        time: u64,
        out: &mut Vec<Record>,
    ) {
        let o = self.keyboard.key(layout, code, action);
        let f = self.flags();
        out.push(Record::new(
            Event::Key {
                code,
                scancode,
                action,
                ch: o.ch,
            },
            o.mods,
            f,
            time,
        ));
        for ch in o.text.into_iter().flatten() {
            out.push(Record::new(Event::Text(ch), o.mods, f, time));
        }
    }

    /// Konec snimku: nasbirany pohyb a kolecko jako jeden zaznam.
    pub fn flush(&mut self, layout: &Layout, time: u64, out: &mut Vec<Record>) {
        let mods = self.keyboard.mods(layout);
        if self.dx != 0 || self.dy != 0 {
            out.push(Record::new(
                Event::Motion {
                    dx: self.dx,
                    dy: self.dy,
                },
                mods,
                self.flags(),
                time,
            ));
        }
        if self.abs_dirty {
            self.abs_dirty = false;
            out.push(Record::new(
                Event::Absolute {
                    x: normalize(self.abs[0], self.axes[0]),
                    y: normalize(self.abs[1], self.axes[1]),
                },
                mods,
                self.flags(),
                time,
            ));
        }
        if self.wheel_x != 0 || self.wheel_y != 0 {
            out.push(Record::new(
                Event::Scroll {
                    dx: self.wheel_x,
                    dy: self.wheel_y,
                },
                mods,
                self.flags(),
                time,
            ));
        }
        self.dx = 0;
        self.dy = 0;
        self.wheel_x = 0;
        self.wheel_y = 0;
    }
}
