//! Radic HDA: reset, prikazy kodekum pres CORB/RIRB (zalozne okamzite
//! rozhrani) a pristup k registrum.
//!
//! Pamet radice (jedna stranka DMA, jen pro ovladac):
//!
//! ```text
//! 0     CORB  256 polozek x 4 B (prikazy, pise ovladac)
//! 1024  RIRB  256 polozek x 8 B (odpovedi, pise radic)
//! ```
//!
//! Prikazy jsou synchronni (jeden prikaz, cekani na odpoved). Kodek
//! odpovida v mikrosekundach, takze se ceka aktivne s terminem; vycet
//! kodeku pri startu je stovky prikazu.

use core::time::Duration;

use hdaspec::regs::{self, Gcap};
use hdaspec::verb::{self, RESP_UNSOL};
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::hw::Resource;
use libos::{Error, Mapping, Result, Vmo, barrier, println, time};

const CORB_OFF: u64 = 0;
const RIRB_OFF: u64 = 1024;

/// Jak dlouho cekat na odpoved kodeku (spec.: kodek musi odpovedet do
/// jednoho ramce 21 us; rezerva na pomale kodeky a QEMU).
const RESPONSE_TIMEOUT: Duration = Duration::from_millis(20);
/// Jak dlouho cekat na zmenu bitu resetu.
const RESET_TIMEOUT: Duration = Duration::from_millis(100);
/// Po kolika neuspesnych prikazech v rade kodek vynechat (jeden vypadek,
/// napr. behem probouzeni z D3, ho nesmi vyradit natrvalo).
const DEAD_AFTER: u8 = 3;

pub struct Ctrl {
    mmio: Mapping,
    pub gcap: Gcap,
    _mem: Vmo,
    mem: Mapping,
    pub mem_phys: u64,
    corb_n: u32,
    rirb_n: u32,
    corb_wp: u32,
    rirb_rp: u32,
    /// CORB/RIRB nefunguje: okamzite rozhrani (ICOI/ICII).
    immediate: bool,
    /// Neuspesne prikazy kodeku v rade; po [`DEAD_AFTER`] se kodek vynechava.
    fails: [u8; 16],
    pub codecs: u16,
}

/// Aktivne cekat (s terminem), az `done()` plati.
fn wait_until(timeout: Duration, mut done: impl FnMut() -> bool) -> bool {
    let end = time::deadline_after(timeout);
    loop {
        if done() {
            return true;
        }
        if time::now() > end {
            return done();
        }
        core::hint::spin_loop();
    }
}

impl Ctrl {
    pub fn open(mmio: Mapping, res: &Resource) -> Result<Self> {
        let gcap = Gcap::decode(mmio.read::<u16>(regs::GCAP));
        let mem = res.dma_memory(4096)?;
        let mem_phys = res.phys_addr(&mem, 0)?;
        if !gcap.addr64 && mem_phys >> 32 != 0 {
            // Radic bez 64bitovych adres a pamet nad 4 GiB: DMA by sahlo jinam.
            return Err(Error::NotSupported);
        }
        let map = Mapping::new(&mem, 0, 4096, MAP_READ | MAP_WRITE)?;
        let mut c = Self {
            mmio,
            gcap,
            _mem: mem,
            mem: map,
            mem_phys,
            corb_n: 0,
            rirb_n: 0,
            corb_wp: 0,
            rirb_rp: 0,
            immediate: false,
            fails: [0; 16],
            codecs: 0,
        };
        c.reset()?;
        if let Err(e) = c.start_rings() {
            println!("hda: CORB/RIRB nejde spustit ({e}), okamzite rozhrani prikazu");
            c.immediate = true;
        }
        Ok(c)
    }

    pub fn r8(&self, off: u64) -> u8 {
        self.mmio.read(off)
    }
    pub fn r16(&self, off: u64) -> u16 {
        self.mmio.read(off)
    }
    pub fn r32(&self, off: u64) -> u32 {
        self.mmio.read(off)
    }
    pub fn w8(&self, off: u64, v: u8) {
        self.mmio.write(off, v)
    }
    pub fn w16(&self, off: u64, v: u16) {
        self.mmio.write(off, v)
    }
    pub fn w32(&self, off: u64, v: u32) {
        self.mmio.write(off, v)
    }

    /// Reset radice a vycet kodeku (STATESTS).
    fn reset(&mut self) -> Result<()> {
        // Nejdriv zastavit DMA prikazu a streamy (po firmware nebo po padu
        // predchoziho ovladace muzou bezet).
        self.w8(regs::CORBCTL, 0);
        self.w8(regs::RIRBCTL, 0);
        let streams = self.gcap.input_streams + self.gcap.output_streams + self.gcap.bidi_streams;
        for i in 0..streams.min(30) {
            let off = regs::sd_offset(i);
            self.w8(off + regs::sd::CTL, 0);
        }
        self.w32(regs::INTCTL, 0);
        self.w32(regs::DPLBASE, 0);
        self.w16(regs::STATESTS, 0x7FFF);
        let g = self.r32(regs::GCTL);
        self.w32(regs::GCTL, g & !regs::GCTL_CRST);
        if !wait_until(RESET_TIMEOUT, || self.r32(regs::GCTL) & regs::GCTL_CRST == 0) {
            return Err(Error::TimedOut);
        }
        time::sleep(Duration::from_micros(200));
        let g = self.r32(regs::GCTL);
        self.w32(regs::GCTL, g | regs::GCTL_CRST);
        if !wait_until(RESET_TIMEOUT, || self.r32(regs::GCTL) & regs::GCTL_CRST != 0) {
            return Err(Error::TimedOut);
        }
        // Kodeky se ohlasi do 521 us po resetu.
        time::sleep(Duration::from_millis(1));
        self.codecs = self.r16(regs::STATESTS) & 0x7FFF;
        self.w16(regs::STATESTS, 0x7FFF);
        Ok(())
    }

    fn start_rings(&mut self) -> Result<()> {
        self.w8(regs::CORBCTL, 0);
        self.w8(regs::RIRBCTL, 0);
        wait_until(RESET_TIMEOUT, || self.r8(regs::CORBCTL) & regs::CORBCTL_RUN == 0);
        let (code, n) = regs::ring_size(self.r8(regs::CORBSIZE));
        self.w8(regs::CORBSIZE, (self.r8(regs::CORBSIZE) & !3) | code);
        self.corb_n = n;
        let corb = self.mem_phys + CORB_OFF;
        self.w32(regs::CORBLBASE, corb as u32);
        self.w32(regs::CORBUBASE, (corb >> 32) as u32);
        self.w16(regs::CORBWP, 0);
        // Reset cteciho ukazatele: nektere radice potvrdi bit 15, jine (QEMU)
        // ne - proto jen kratke cekani bez chyby.
        self.w16(regs::CORBRP, regs::CORBRP_RST);
        wait_until(Duration::from_millis(1), || self.r16(regs::CORBRP) & regs::CORBRP_RST != 0);
        self.w16(regs::CORBRP, 0);
        if !wait_until(RESET_TIMEOUT, || self.r16(regs::CORBRP) & regs::CORBRP_RST == 0) {
            return Err(Error::TimedOut);
        }
        self.corb_wp = 0;

        let (code, n) = regs::ring_size(self.r8(regs::RIRBSIZE));
        self.w8(regs::RIRBSIZE, (self.r8(regs::RIRBSIZE) & !3) | code);
        self.rirb_n = n;
        let rirb = self.mem_phys + RIRB_OFF;
        self.w32(regs::RIRBLBASE, rirb as u32);
        self.w32(regs::RIRBUBASE, (rirb >> 32) as u32);
        self.w16(regs::RIRBWP, regs::RIRBWP_RST);
        // Preruseni po odpovedich se nepouziva (ceka se aktivne), ale citac
        // musi byt nenulovy - QEMU jinak zastavi zpracovani CORB.
        self.w16(regs::RINTCNT, 0xFF);
        self.rirb_rp = 0;
        self.w8(regs::RIRBCTL, regs::RIRBCTL_DMAEN);
        self.w8(regs::CORBCTL, regs::CORBCTL_RUN);
        if !wait_until(RESET_TIMEOUT, || self.r8(regs::CORBCTL) & regs::CORBCTL_RUN != 0) {
            return Err(Error::TimedOut);
        }
        Ok(())
    }

    /// Posle prikaz a vrati odpoved kodeku.
    pub fn command(&mut self, cmd: u32) -> Result<u32> {
        let codec = (cmd >> 28) as usize & 0xF;
        if self.fails[codec] >= DEAD_AFTER {
            return Err(Error::TimedOut);
        }
        let r = if self.immediate { self.command_immediate(cmd) } else { self.command_ring(cmd) };
        match r {
            Ok(_) => self.fails[codec] = 0,
            Err(_) => {
                self.fails[codec] += 1;
                if self.fails[codec] >= DEAD_AFTER {
                    println!("hda: kodek {codec} neodpovida (prikaz {cmd:08x}), dal ho vynechavam");
                } else {
                    // Odpoved mohla prijit pozde: dalsi prikaz ji nesmi vzit za svou.
                    self.resync_rirb();
                }
            }
        }
        r
    }

    fn command_ring(&mut self, cmd: u32) -> Result<u32> {
        let wp = (self.corb_wp + 1) % self.corb_n;
        self.mem.write::<u32>(CORB_OFF + wp as u64 * 4, cmd);
        self.corb_wp = wp;
        barrier::mmio_wmb();
        self.w16(regs::CORBWP, wp as u16);
        let end = time::deadline_after(RESPONSE_TIMEOUT);
        loop {
            let hw = self.r16(regs::RIRBWP) as u32 % self.rirb_n;
            if hw != self.rirb_rp {
                barrier::dma_rmb();
                self.rirb_rp = (self.rirb_rp + 1) % self.rirb_n;
                let off = RIRB_OFF + self.rirb_rp as u64 * 8;
                let resp = self.mem.read::<u32>(off);
                let ex = self.mem.read::<u32>(off + 4);
                // Potvrdit (QEMU jinak po RINTCNT odpovedich zastavi CORB).
                self.w8(regs::RIRBSTS, 0x05);
                if ex & RESP_UNSOL != 0 {
                    continue; // nevyzadana odpoved (konektor), neni nase
                }
                return Ok(resp);
            }
            if time::now() > end {
                return Err(Error::TimedOut);
            }
            core::hint::spin_loop();
        }
    }

    /// Preskoci odpovedi, ktere uz v RIRB jsou (pozdni odpoved na prikaz,
    /// kteremu vyprsel cas).
    fn resync_rirb(&mut self) {
        if !self.immediate && self.rirb_n != 0 {
            self.rirb_rp = self.r16(regs::RIRBWP) as u32 % self.rirb_n;
            self.w8(regs::RIRBSTS, 0x05);
        }
    }

    fn command_immediate(&mut self, cmd: u32) -> Result<u32> {
        if !wait_until(RESPONSE_TIMEOUT, || self.r16(regs::ICIS) & regs::ICIS_ICB == 0) {
            return Err(Error::TimedOut);
        }
        // Stara platna odpoved: smazat (zapis 1).
        self.w16(regs::ICIS, regs::ICIS_IRV);
        self.w32(regs::ICOI, cmd);
        self.w16(regs::ICIS, regs::ICIS_ICB);
        if !wait_until(RESPONSE_TIMEOUT, || self.r16(regs::ICIS) & regs::ICIS_IRV != 0) {
            return Err(Error::TimedOut);
        }
        Ok(self.r32(regs::ICII))
    }

    /// GET_PARAMETER.
    pub fn param(&mut self, codec: u8, nid: u16, p: u8) -> Result<u32> {
        self.command(verb::cmd12(codec, nid as u8, verb::GET_PARAMETER, p))
    }

    pub fn verb12(&mut self, codec: u8, nid: u16, v: u16, data: u8) -> Result<u32> {
        self.command(verb::cmd12(codec, nid as u8, v, data))
    }

    pub fn verb4(&mut self, codec: u8, nid: u16, v: u8, data: u16) -> Result<u32> {
        self.command(verb::cmd4(codec, nid as u8, v, data))
    }

    /// Zastavi DMA prikazu a radic (pred koncem ovladace).
    pub fn shutdown(&mut self) {
        self.w32(regs::INTCTL, 0);
        self.w8(regs::CORBCTL, 0);
        self.w8(regs::RIRBCTL, 0);
        self.w32(regs::DPLBASE, 0);
    }
}
