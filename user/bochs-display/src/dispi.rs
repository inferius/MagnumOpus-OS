//! Backend bochs-display: registry DISPI (Bochs VBE) v MMIO BAR2,
//! obraz v BAR0 (VRAM), EDID v BAR2 od offsetu 0.
//!
//! ```text
//! BAR2: 0x000-0x3FF EDID | 0x400 VGA (jen stdvga) | 0x500 DISPI (16bit, index * 2)
//!       | 0x600 rozsireni QEMU (0x604 poradi bajtu framebufferu)
//! ```
//!
//! Registry podle `hw/display/bochs-display.c` a `vga.c` v QEMU
//! (popis rozhrani, ne kod) a dokumentace Bochs VBE. Page flip: virtualni
//! vyska = 2 x vyska, `Y_OFFSET` vybere buffer. Zarizeni nema preruseni
//! vsync, zatemneni se simuluje hodinami ([`displaydrv::VblankClock`]).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use display::edid::Edid;
use display::modes::{Limits, modes};
use displaydrv::snapshot::{Snapshot, driver};
use displaydrv::{Backend, Scanout, VblankClock};
use fbcopy::{Format, Surface};
use guiproto::Error;
use guiproto::display::caps;
use guiproto::modes::{Connector, Mode, kind, mode_flags};
use libos::Mapping;

pub mod reg {
    pub const ID: u16 = 0;
    pub const XRES: u16 = 1;
    pub const YRES: u16 = 2;
    pub const BPP: u16 = 3;
    pub const ENABLE: u16 = 4;
    pub const BANK: u16 = 5;
    pub const VIRT_WIDTH: u16 = 6;
    pub const VIRT_HEIGHT: u16 = 7;
    pub const X_OFFSET: u16 = 8;
    pub const Y_OFFSET: u16 = 9;
    pub const VIDEO_MEMORY_64K: u16 = 10;
}

pub mod enable {
    pub const ENABLED: u16 = 0x01;
    pub const GETCAPS: u16 = 0x02;
    pub const LFB: u16 = 0x40;
    pub const NOCLEARMEM: u16 = 0x80;
}

const DISPI: usize = 0x500;
const QEMU_EXT: usize = 0x600;
const EDID_SIZE: usize = 0x400;
/// Poradi bajtu framebufferu little-endian (rozsireni QEMU).
const FB_LE: u32 = 0x1e1e_1e1e;

/// Seznam rezimu bez EDID (QEMU s `edid=off`).
const FALLBACK: [(u32, u32); 6] = [(640, 480), (800, 600), (1024, 768), (1280, 720), (1280, 800), (1920, 1080)];

pub struct Bochs {
    regs: Mapping,
    vram: Mapping,
    vram_size: u64,
    max: (u32, u32),
    edid: Option<Edid>,
    monitor: String,
    /// Aktualni rezim (zapnuto).
    mode: Option<(u32, u32, u32)>,
    buffers: usize,
    clock: VblankClock,
    flip_at: Option<u64>,
    /// Po padu: jen rezim firmwaru, bez zmen.
    safe: bool,
    /// Zaznam firmwaru ulozen (pred prvnim zapisem).
    saved: bool,
}

impl Bochs {
    pub fn new(regs: Mapping, vram: Mapping, vram_size: u64, safe: bool) -> Result<Self, Error> {
        let mut b = Bochs {
            regs,
            vram,
            vram_size,
            max: (0, 0),
            edid: None,
            monitor: String::new(),
            mode: None,
            buffers: 1,
            clock: VblankClock::new(libos::time::now(), 60_000),
            flip_at: None,
            safe,
            saved: false,
        };
        let id = b.read(reg::ID);
        if !(0xB0C0..=0xB0CF).contains(&id) {
            return Err(Error::NotSupported);
        }
        // Nejvetsi rozliseni (GETCAPS) - cteni bez zmeny obrazu.
        let en = b.read(reg::ENABLE);
        b.write(reg::ENABLE, en | enable::GETCAPS);
        b.max = (b.read(reg::XRES) as u32, b.read(reg::YRES) as u32);
        b.write(reg::ENABLE, en);
        if en & enable::ENABLED != 0 {
            let (w, h) = (b.read(reg::XRES) as u32, b.read(reg::YRES) as u32);
            b.mode = Some((w, h, 60_000));
            b.buffers = (b.read(reg::VIRT_HEIGHT) as u32 / h.max(1)).clamp(1, 2) as usize;
        }
        // Zakladni blok + jedno rozsireni (QEMU vic negeneruje).
        let mut raw = vec![0u8; 256.min(EDID_SIZE)];
        for (i, v) in raw.iter_mut().enumerate() {
            // SAFETY: EDID je v prvnich 1 KiB namapovaneho BAR2.
            *v = unsafe { core::ptr::read_volatile((b.regs.addr() as *const u8).add(i)) };
        }
        b.edid = Edid::parse(&raw).ok();
        if let Some(e) = &b.edid {
            b.monitor = e.display_name();
        }
        // Framebuffer little-endian (na big-endian hostiteli by QEMU
        // jinak prohodilo bajty).
        if b.ext_size() >= 8 {
            b.write_ext(4, FB_LE);
        }
        Ok(b)
    }

    pub fn edid_raw_ok(&self) -> bool {
        self.edid.is_some()
    }

    fn read(&self, r: u16) -> u16 {
        // SAFETY: DISPI registry lezi v namapovanem BAR2 (4 KiB).
        unsafe { core::ptr::read_volatile((self.regs.addr() as *const u8).add(DISPI + r as usize * 2) as *const u16) }
    }

    fn write(&mut self, r: u16, v: u16) {
        // SAFETY: jako `read`.
        unsafe { core::ptr::write_volatile((self.regs.addr() as *mut u8).add(DISPI + r as usize * 2) as *mut u16, v) }
    }

    fn ext_size(&self) -> u32 {
        // SAFETY: oblast rozsireni QEMU v BAR2.
        unsafe { core::ptr::read_volatile((self.regs.addr() as *const u8).add(QEMU_EXT) as *const u32) }
    }

    fn write_ext(&mut self, off: usize, v: u32) {
        // SAFETY: jako `ext_size`, offset overen velikosti oblasti.
        unsafe { core::ptr::write_volatile((self.regs.addr() as *mut u8).add(QEMU_EXT + off) as *mut u32, v) }
    }

    /// Stav firmwaru (DISPI registry) pro navrat po padu.
    pub fn snapshot(&self) -> Snapshot {
        let mut regs = Vec::new();
        for r in [reg::BPP, reg::XRES, reg::YRES, reg::BANK, reg::VIRT_WIDTH, reg::VIRT_HEIGHT, reg::X_OFFSET, reg::Y_OFFSET] {
            regs.push((r as u32, self.read(r) as u32));
        }
        // ENABLE posledni (zapne obraz s obnovenymi hodnotami, bez mazani).
        regs.push((reg::ENABLE as u32, (self.read(reg::ENABLE) | enable::NOCLEARMEM) as u32));
        let (w, h) = self.mode.map_or((0, 0), |m| (m.0, m.1));
        Snapshot {
            driver: driver::BOCHS,
            w,
            h,
            stride: w * 4,
            fb_offset: 0,
            regs,
        }
    }

    /// Zapise registry ze zaznamu (obnova po padu).
    pub fn restore(&mut self, s: &Snapshot) {
        self.write(reg::ENABLE, 0);
        for &(r, v) in &s.regs {
            if r <= reg::VIDEO_MEMORY_64K as u32 && r != reg::ID as u32 {
                self.write(r as u16, v as u16);
            }
        }
        self.mode = if self.read(reg::ENABLE) & enable::ENABLED != 0 {
            Some((self.read(reg::XRES) as u32, self.read(reg::YRES) as u32, 60_000))
        } else {
            None
        };
        // Obraz firmwaru je od Y_OFFSET 0 (ostatni vynulovany zaznamem).
        self.buffers = 1;
    }

    /// Ulozi zaznam firmwaru pred prvni zmenou (vola ovladac).
    pub fn mark_saved(&mut self) {
        self.saved = true;
    }

    fn mode_list(&self) -> Vec<(u32, u32, u32, u32, bool)> {
        let fits = |w: u32, h: u32| w <= self.max.0 && h <= self.max.1 && (w as u64 * h as u64 * 4) <= self.vram_size;
        let mut out = Vec::new();
        if let Some(e) = &self.edid {
            let lim = Limits::new(u32::MAX, self.max.0 as u16, self.max.1 as u16);
            for m in modes(e, &lim) {
                let t = m.timing;
                if fits(t.hactive as u32, t.vactive as u32) {
                    out.push((t.hactive as u32, t.vactive as u32, t.refresh_mhz(), t.clock_khz, t.preferred()));
                }
            }
        }
        if out.is_empty() {
            for (i, &(w, h)) in FALLBACK.iter().enumerate() {
                if fits(w, h) {
                    out.push((w, h, 60_000, 0, i == 2));
                }
            }
        }
        out
    }
}

impl Backend for Bochs {
    fn caps(&self) -> u32 {
        if self.safe { caps::VSYNC } else { caps::MODESET | caps::VSYNC }
    }

    fn connectors(&mut self) -> Vec<Connector> {
        let mut modes: Vec<Mode> = Vec::new();
        let mut current_marked = false;
        let list = if self.safe {
            self.mode.map(|(w, h, r)| vec![(w, h, r, 0, true)]).unwrap_or_default()
        } else {
            self.mode_list()
        };
        for (w, h, r, clk, pref) in list {
            let mut flags = if pref { mode_flags::PREFERRED } else { 0 };
            if !current_marked && self.mode.is_some_and(|m| (m.0, m.1) == (w, h)) {
                flags |= mode_flags::CURRENT;
                current_marked = true;
            }
            modes.push(Mode { w, h, refresh_mhz: r, clock_khz: clk, flags });
        }
        // Rezim nastaveny firmwarem mimo seznam (napr. 800x600 od OVMF).
        if !current_marked && let Some((w, h, r)) = self.mode {
            modes.insert(0, Mode { w, h, refresh_mhz: r, clock_khz: 0, flags: mode_flags::CURRENT });
        }
        let size_mm = self.edid.as_ref().map_or((0, 0), |e| {
            let s = e.size_mm();
            (s.0 as u32, s.1 as u32)
        });
        vec![Connector {
            output: 0,
            kind: kind::VIRTUAL,
            connected: true,
            monitor: self.monitor.clone(),
            size_mm,
            modes,
        }]
    }

    fn size(&self, output: u32) -> Option<(u32, u32)> {
        if output != 0 {
            return None;
        }
        self.mode.map(|m| (m.0, m.1))
    }

    fn set_mode(&mut self, output: u32, w: u32, h: u32, refresh_mhz: u32) -> Result<(u32, u32, u32), Error> {
        if output != 0 {
            return Err(Error::NotFound);
        }
        let list = self.connectors().remove(0).modes;
        let m = guiproto::modes::find_mode(
            &Connector { output: 0, kind: 0, connected: true, monitor: String::new(), size_mm: (0, 0), modes: list },
            w,
            h,
            refresh_mhz,
        )
        .copied()
        .ok_or(Error::NotSupported)?;
        if self.safe && self.mode.map(|x| (x.0, x.1)) != Some((w, h)) {
            return Err(Error::NotSupported);
        }
        if !self.saved {
            // Ovladac musi zaznam firmwaru ulozit driv nez prvni zapis.
            return Err(Error::BadState);
        }
        let frame = w as u64 * h as u64 * 4;
        let buffers = if self.safe { 1 } else if frame * 2 <= self.vram_size { 2 } else { 1 };
        let prev = self.snapshot();
        self.write(reg::ENABLE, 0);
        self.write(reg::BPP, 32);
        self.write(reg::XRES, w as u16);
        self.write(reg::YRES, h as u16);
        self.write(reg::BANK, 0);
        self.write(reg::VIRT_WIDTH, w as u16);
        self.write(reg::VIRT_HEIGHT, (h as usize * buffers) as u16);
        self.write(reg::X_OFFSET, 0);
        self.write(reg::Y_OFFSET, 0);
        self.write(reg::ENABLE, enable::ENABLED | enable::LFB);
        // Zarizeni muze rezim odmitnout (nastavi jine hodnoty): vratit puvodni.
        if self.read(reg::XRES) as u32 != w || self.read(reg::YRES) as u32 != h || self.read(reg::BPP) != 32 {
            self.restore(&prev);
            return Err(Error::NotSupported);
        }
        self.mode = Some((w, h, m.refresh_mhz));
        self.buffers = buffers;
        self.clock = VblankClock::new(libos::time::now(), m.refresh_mhz);
        self.flip_at = None;
        Ok((w, h, m.refresh_mhz))
    }

    fn buffers(&self, _output: u32) -> usize {
        self.buffers
    }

    fn buffer(&mut self, output: u32, index: usize) -> Option<Scanout> {
        let (w, h, _) = self.mode?;
        if output != 0 || index >= self.buffers {
            return None;
        }
        let off = index as u64 * w as u64 * h as u64 * 4;
        Some(Scanout {
            surface: Surface {
                base: (self.vram.addr() + off) as *mut u8,
                width: w,
                height: h,
                stride: w * 4,
            },
            format: Format::Bgr,
        })
    }

    fn flip(&mut self, output: u32, index: usize, now: u64) -> Result<(), Error> {
        let (_, h, _) = self.mode.ok_or(Error::BadState)?;
        if output != 0 || index >= self.buffers {
            return Err(Error::InvalidArgs);
        }
        self.write(reg::Y_OFFSET, (index as u32 * h) as u16);
        self.flip_at = Some(self.clock.next(now + 1));
        Ok(())
    }

    fn flip_done(&mut self, _output: u32, now: u64) -> Option<u64> {
        match self.flip_at {
            Some(t) if now >= t => {
                self.flip_at = None;
                Some(t)
            }
            _ => None,
        }
    }
}
