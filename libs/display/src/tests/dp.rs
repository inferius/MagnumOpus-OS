//! DisplayPort: AUX, DPCD, navazani linky a MST proti simulovanemu
//! prijemci (pamet DPCD, I2C EDID, chovani pri treninku).

use alloc::vec::Vec;

use super::data::AOC_2243W;
use crate::dp::auxch::{self as aux, Aux, Msg, Reply, cmd, decode_reply};
use crate::dp::train::{Drive, Pattern, Phy, Status, train};
use crate::dp::{self, Caps, HBR, HBR2, LinkConfig, RBR, bits, choose_link, dpcd, fallback, mst};
use crate::timing::Timing;
use crate::{Error, dmt};

/// Simulovany prijemce DP a zdroj (PHY) zaroven.
struct Sink {
    dpcd: Vec<u8>,
    edid: Vec<u8>,
    segment: u8,
    offset: usize,
    /// Kolikrat odpovedet DEFER pred dalsi odpovedi.
    defers: u32,
    /// Prijemce vrati pri I2C cteni jen tolik bajtu (kratka odpoved).
    short_i2c: Option<usize>,
    /// Potrebne buzeni pro CR a EQ.
    need: Drive,
    eq_need_preemph: u8,
    /// CR se nikdy nepodari (vadny kabel).
    broken: bool,
    /// Nejvyssi rychlost, pri ktere linka funguje.
    max_ok_rate: u32,
    // Stav zdroje.
    pattern: Pattern,
    drive: Drive,
    log: Vec<&'static str>,
    delays: u64,
}

impl Sink {
    fn new() -> Self {
        let mut d = alloc::vec![0u8; 0x2300];
        d[0x000] = 0x12; // DPCD 1.2
        d[0x001] = 0x14; // HBR2
        d[0x002] = 0x84 | bits::TPS3_SUPPORTED; // 4 lane, enhanced frame, TPS3
        d[0x00E] = 0x00;
        d[0x021] = 0x01; // MST
        Sink {
            dpcd: d,
            edid: Vec::from(&AOC_2243W[..]),
            segment: 0,
            offset: 0,
            defers: 0,
            short_i2c: None,
            need: Drive { vswing: 2, preemph: 0 },
            eq_need_preemph: 1,
            broken: false,
            max_ok_rate: HBR2,
            pattern: Pattern::None,
            drive: Drive::default(),
            log: Vec::new(),
            delays: 0,
        }
    }

    fn lanes(&self) -> usize {
        (self.dpcd[dpcd::LANE_COUNT_SET as usize] & 0x1F) as usize
    }

    fn rate(&self) -> u32 {
        dp::rate_of_bw_code(self.dpcd[dpcd::LINK_BW_SET as usize]).unwrap_or(0)
    }

    /// Prepocet stavu lane po zmene vzoru nebo buzeni.
    fn update_status(&mut self) {
        let set = self.dpcd[dpcd::TRAINING_LANE0_SET as usize];
        let v = set & 3;
        let p = (set >> 3) & 3;
        let rate_ok = self.rate() <= self.max_ok_rate;
        let cr = !self.broken && rate_ok && v >= self.need.vswing && self.drive.vswing == v;
        let eq = cr && matches!(self.pattern, Pattern::Tps2 | Pattern::Tps3 | Pattern::Tps4) && p >= self.eq_need_preemph;
        let lane = (cr as u8) | (eq as u8) << 1 | (eq as u8) << 2;
        let both = lane | lane << 4;
        self.dpcd[0x202] = both;
        self.dpcd[0x203] = if self.lanes() == 4 { both } else { 0 };
        self.dpcd[0x204] = eq as u8;
        // Pozadavek: potrebne napeti a predzdurazneni (az ve fazi EQ).
        let want_p = if self.pattern == Pattern::Tps1 { 0 } else { self.eq_need_preemph };
        let req = self.need.vswing | want_p << 2;
        self.dpcd[0x206] = req | req << 4;
        self.dpcd[0x207] = req | req << 4;
    }
}

impl Aux for Sink {
    fn transfer(&mut self, tx: &[u8], rx: &mut [u8; 20]) -> crate::Result<usize> {
        if self.defers > 0 {
            self.defers -= 1;
            rx[0] = 0x20; // nativni DEFER
            return Ok(1);
        }
        let (m, data) = Msg::decode(tx).ok_or(Error::Io)?;
        if m.is_native() {
            let a = m.address as usize;
            if m.is_read() {
                rx[0] = 0;
                rx[1..1 + m.len].copy_from_slice(&self.dpcd[a..a + m.len]);
                return Ok(1 + m.len);
            }
            self.dpcd[a..a + m.len].copy_from_slice(data);
            if (0x102..=0x106).contains(&a) {
                self.update_status();
            }
            rx[0] = 0;
            return Ok(1);
        }
        // I2C pres AUX.
        let read = m.cmd & 1 != 0;
        match (m.address, read) {
            (0x30, false) => self.segment = data[0],
            (0x50, false) => self.offset = self.segment as usize * 256 + data[0] as usize,
            (0x50, true) if m.len > 0 => {
                let n = self.short_i2c.unwrap_or(m.len).min(m.len);
                let src = self.edid.get(self.offset..self.offset + n).ok_or(Error::Io)?;
                rx[1..1 + n].copy_from_slice(src);
                self.offset += n;
                rx[0] = 0;
                return Ok(1 + n);
            }
            (0x50, true) => {}
            _ => {
                rx[0] = 0x10 << 2; // I2C NACK
                return Ok(1);
            }
        }
        rx[0] = 0;
        Ok(1)
    }

    fn delay_us(&mut self, us: u32) {
        self.delays += us as u64;
    }
}

impl Phy for Sink {
    fn set_pattern(&mut self, p: Pattern) -> crate::Result<()> {
        self.pattern = p;
        self.log.push(match p {
            Pattern::None => "video",
            Pattern::Tps1 => "tps1",
            Pattern::Tps2 => "tps2",
            Pattern::Tps3 => "tps3",
            Pattern::Tps4 => "tps4",
        });
        Ok(())
    }

    fn set_drive(&mut self, d: Drive, _lanes: u8) -> crate::Result<()> {
        self.drive = d;
        Ok(())
    }
}

fn caps(s: &mut Sink) -> Caps {
    let mut base = [0u8; 16];
    aux::dpcd_read(s, 0, &mut base).unwrap();
    Caps::parse(&base, None, s.dpcd[0x21]).unwrap()
}

fn mode_1080p() -> Timing {
    dmt::find(1920, 1080, 60, None).unwrap().timing
}

#[test]
fn aux_header_encoding() {
    let m = Msg { cmd: cmd::NATIVE_READ, address: 0x12345, len: 16 };
    let mut out = [0u8; 20];
    assert_eq!(m.encode(&[], &mut out), 4);
    assert_eq!(&out[..4], &[0x91, 0x23, 0x45, 0x0F]);
    let w = Msg { cmd: cmd::NATIVE_WRITE, address: 0x100, len: 2 };
    assert_eq!(w.encode(&[0x14, 0x84], &mut out), 6);
    assert_eq!(&out[..6], &[0x80, 0x01, 0x00, 0x01, 0x14, 0x84]);
    // Jen adresa (I2C STOP).
    let s = Msg { cmd: cmd::I2C_READ, address: 0x50, len: 0 };
    assert_eq!(s.encode(&[], &mut out), 3);
    assert_eq!(Msg::decode(&out[..3]).unwrap().0, s);
    // Kody odpovedi.
    assert_eq!(decode_reply(0x00, true), Reply::Ack);
    assert_eq!(decode_reply(0x10, true), Reply::Nack);
    assert_eq!(decode_reply(0x20, true), Reply::Defer);
    assert_eq!(decode_reply(0x40, false), Reply::Nack);
    assert_eq!(decode_reply(0x80, false), Reply::Defer);
    assert_eq!(decode_reply(0x20, false), Reply::Defer);
}

#[test]
fn dpcd_rw_with_defer() {
    let mut s = Sink::new();
    s.defers = 3;
    let mut b = [0u8; 3];
    aux::dpcd_read(&mut s, 0, &mut b).unwrap();
    assert_eq!(b, [0x12, 0x14, 0xC4]);
    assert!(s.delays > 0);
    // Prilis mnoho DEFER = timeout.
    s.defers = 100;
    assert_eq!(aux::dpcd_read(&mut s, 0, &mut b), Err(Error::Timeout));
    s.defers = 0;
    // Zapis delsi nez 16 B se rozdeli.
    let data: Vec<u8> = (0..40).collect();
    aux::dpcd_write(&mut s, 0x1000, &data).unwrap();
    assert_eq!(&s.dpcd[0x1000..0x1028], &data[..]);
}

#[test]
fn edid_over_aux() {
    let mut s = Sink::new();
    assert_eq!(aux::read_edid(&mut s).unwrap(), AOC_2243W);
    // Kratke odpovedi I2C (prijemce vraci po 5 B) se poskladaji.
    let mut s = Sink::new();
    s.short_i2c = Some(5);
    assert_eq!(aux::read_edid(&mut s).unwrap(), AOC_2243W);
}

#[test]
fn caps_parse() {
    let mut s = Sink::new();
    let c = caps(&mut s);
    assert_eq!(c.rev, 0x12);
    assert_eq!(c.rates, [RBR, HBR, HBR2]);
    assert_eq!(c.max_lanes, 4);
    assert!(c.enhanced_frame && c.tps3 && !c.tps4 && c.mst);
    assert_eq!(c.eq_interval_us, 400);
    // Nic pripojeno: DPCD_REV 0.
    assert_eq!(Caps::parse(&[0u8; 16], None, 0), Err(Error::Invalid));
    // eDP 1.4: tabulka rychlosti (200 kHz jednotky): 1.62, 2.16, 2.7 Gb/s.
    let mut base = [0u8; 16];
    base[0] = 0x14;
    base[2] = 0x82;
    base[0x0E] = 0x01;
    let table = [0xa4_u8, 0x1f, 0x30, 0x2a, 0xbc, 0x34, 0, 0];
    let c = Caps::parse(&base, Some(&table), 0).unwrap();
    assert!(c.edp_table);
    assert_eq!(c.rates, [162_000, 216_000, 270_000]);
    assert_eq!(c.eq_interval_us, 4000);
}

#[test]
fn link_config_choice_and_fallback() {
    let mut s = Sink::new();
    let c = caps(&mut s);
    let t = mode_1080p();
    // 148.5 MHz * 24 bit = 445500 kB/s: RBR x 4 lane (648000) staci;
    // nejdriv se zkousi mene lane pri nizsi rychlosti.
    let l = choose_link(&c, &t, 24, HBR2, 4).unwrap();
    assert_eq!((l.rate, l.lanes), (RBR, 4));
    let l2 = choose_link(&c, &t, 24, HBR2, 2).unwrap();
    assert_eq!((l2.rate, l2.lanes), (HBR, 2));
    // 4K60 (533 MHz RB v2) se na 2 lane HBR2 nevejde.
    let big = crate::cvt::cvt(3840, 2160, 60_000, crate::cvt::Blanking::ReducedV2).unwrap();
    assert_eq!(choose_link(&c, &big, 24, HBR2, 2), Err(Error::Bandwidth));
    // Nahradni konfigurace: nizsi rychlost, pak mene lane.
    let top = LinkConfig { rate: HBR2, lanes: 4, enhanced_frame: true, edp_rate_index: None };
    let f1 = fallback(&c, &top).unwrap();
    assert_eq!((f1.rate, f1.lanes), (HBR, 4));
    let f3 = fallback(&c, &LinkConfig { rate: RBR, ..top }).unwrap();
    assert_eq!((f3.rate, f3.lanes), (HBR2, 2));
    assert!(fallback(&c, &LinkConfig { rate: RBR, lanes: 1, ..top }).is_none());
}

#[test]
fn m_n_values() {
    // 148.5 MHz, 24 bpp, 4 lane, HBR (270 MHz): data M/N = 3564/8640.
    let mn = dp::compute_m_n(24, 4, 148_500, 270_000);
    assert_eq!(mn.data_n, 0x80_0000);
    assert_eq!(mn.data_m as u64, 148_500u64 * 24 * 0x80_0000 / (270_000 * 4 * 8));
    assert_eq!(mn.link_m as u64, 148_500u64 * 0x80_0000 / 270_000);
}

#[test]
fn training_succeeds_with_adjustments() {
    let mut s = Sink::new();
    let c = caps(&mut s);
    let cfg = LinkConfig { rate: HBR2, lanes: 4, enhanced_frame: true, edp_rate_index: None };
    let r = train(&mut s, &c, &cfg).unwrap();
    assert_eq!(r.drive, Drive { vswing: 2, preemph: 1 });
    assert_eq!(r.pattern, Pattern::Tps3);
    assert_eq!(r.cr_rounds, 2);
    assert_eq!(r.eq_rounds, 2);
    // Konfigurace v prijemci a konec treninku.
    assert_eq!(s.dpcd[0x100], 0x14);
    assert_eq!(s.dpcd[0x101], 0x84);
    assert_eq!(s.dpcd[0x102], 0);
    assert_eq!(s.log, ["tps1", "tps3", "video"]);
}

#[test]
fn training_uses_tps2_without_tps3() {
    let mut s = Sink::new();
    s.dpcd[2] &= !bits::TPS3_SUPPORTED;
    let c = caps(&mut s);
    let cfg = LinkConfig { rate: HBR, lanes: 2, enhanced_frame: false, edp_rate_index: None };
    assert_eq!(train(&mut s, &c, &cfg).unwrap().pattern, Pattern::Tps2);
    assert_eq!(s.dpcd[0x101], 0x02);
}

#[test]
fn training_failure_and_fallback() {
    // Vadny kabel: CR se nepodari, vzor se presto vypne.
    let mut s = Sink::new();
    s.broken = true;
    let c = caps(&mut s);
    let cfg = LinkConfig { rate: HBR2, lanes: 4, enhanced_frame: true, edp_rate_index: None };
    assert_eq!(train(&mut s, &c, &cfg), Err(Error::ClockRecovery));
    assert_eq!(s.dpcd[0x102], 0);
    assert_eq!(s.log.last(), Some(&"video"));
    // Linka zvladne jen HBR: z HBR2 se spadne na HBR (jako ovladac).
    let mut s = Sink::new();
    s.max_ok_rate = HBR;
    let c = caps(&mut s);
    let mut cfg = cfg;
    let mut tries = 0;
    let ok = loop {
        tries += 1;
        match train(&mut s, &c, &cfg) {
            Ok(_) => break cfg,
            Err(_) => cfg = fallback(&c, &cfg).unwrap(),
        }
    };
    assert_eq!((ok.rate, ok.lanes, tries), (HBR, 4, 2));
}

#[test]
fn lane_status_bits() {
    let s = Status([0x77, 0x77, 0x01, 0, 0x26, 0x00]);
    assert!(s.cr_ok(4) && s.eq_ok(4));
    let s = Status([0x77, 0x17, 0x01, 0, 0, 0]);
    assert!(s.cr_ok(4) && !s.eq_ok(4) && s.eq_ok(2));
    // Pozadavek: maximum pres lane (lane 1: vswing 2, preemph 1).
    let s = Status([0, 0, 0, 0, 0x61, 0x00]);
    assert_eq!(s.requested(2), Drive { vswing: 2, preemph: 1 });
    assert_eq!(s.requested(1), Drive { vswing: 1, preemph: 0 });
}

#[test]
fn mst_sideband_roundtrip() {
    let msg = mst::link_address(1);
    let (h, body) = mst::decode_message(&msg).unwrap();
    assert_eq!(h.lct, 1);
    assert!(h.somt && h.eomt);
    assert_eq!(h.seqno, 1);
    assert_eq!(body, [mst::req::LINK_ADDRESS]);
    // Hlavicka s cestou pres dva vetvice (LCT 3, RAD 2 a 5).
    let mut h2 = mst::Header::direct(0, 0);
    h2.lct = 3;
    h2.lcr = 2;
    h2.rad[0] = 2;
    h2.rad[1] = 5;
    let mut out = Vec::new();
    mst::encode_message(&h2, &[0x20, 0x10, 0x00, 0x00, 0x10], &mut out).unwrap();
    let (back, body) = mst::decode_message(&out).unwrap();
    assert_eq!((back.lct, back.lcr, back.rad[0], back.rad[1]), (3, 2, 2, 5));
    assert_eq!(body.len(), 5);
    // Poskozeni hlavicky i tela se pozna.
    let mut bad = out.clone();
    bad[1] ^= 0x10;
    assert_eq!(mst::decode_message(&bad).map(|_| ()), Err(Error::BadChecksum));
    let mut bad = out.clone();
    let n = bad.len();
    bad[n - 2] ^= 1;
    assert_eq!(mst::decode_message(&bad).map(|_| ()), Err(Error::BadChecksum));
}

#[test]
fn mst_crc_known_values() {
    // CRC4 nuloveho vstupu je 0; CRC8 jednoho bajtu 0x01 = polynom 0xD5.
    assert_eq!(mst::header_crc4(&[0, 0], 3), 0);
    assert_eq!(mst::body_crc8(&[0x01]), 0xD5);
    assert_eq!(mst::body_crc8(&[]), 0);
}
