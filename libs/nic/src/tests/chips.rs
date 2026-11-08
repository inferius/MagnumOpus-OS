//! Zvlastnosti jednotlivych cipu: zdroj MAC, probuzeni PHY (PCH),
//! rychlosti, vadne a rozdelene ramce, adresy DMA nad 4 GiB.

use std::boxed::Box;
use std::vec::Vec;

use super::model_e1000e::E1000eModel;
use super::model_igc::IgcModel;
use super::model_r8169::R8169Model;
use super::rx_all;
use super::sim::{self, Handle, frame};
use crate::e1000e::{E1000e, RAH0, RAL0};
use crate::ids::{self, E1000eKind};
use crate::igc::Igc;
use crate::r8169::R8169;
use crate::{Hw, Nic};

/// Adresy DMA: pod 4 GiB a nad 4 GiB (overuje zapis horni poloviny adresy
/// a u Realteku PCIDAC).
const BASE_LOW: u64 = 0x7F00_0000;
const BASE_HIGH: u64 = 0x1_2340_0000;
const SEED: u64 = 0x1122_3344_5566_7788;

fn e1000e(model: E1000eModel, device: u16) -> (Handle<E1000eModel>, E1000e<Handle<E1000eModel>>) {
    let sim = sim::new(model, BASE_HIGH);
    let nic = E1000e::open(sim.clone(), device, SEED).unwrap();
    (sim, nic)
}

pub fn make_82574() -> (Handle<E1000eModel>, Box<dyn Nic>) {
    let (s, n) = e1000e(E1000eModel::new(E1000eKind::Discrete), 0x10D3);
    (s, Box::new(n))
}

pub fn make_i219() -> (Handle<E1000eModel>, Box<dyn Nic>) {
    let (s, n) = e1000e(E1000eModel::new(E1000eKind::Pch), 0x15BC);
    (s, Box::new(n))
}

pub fn make_i226() -> (Handle<IgcModel>, Box<dyn Nic>) {
    let sim = sim::new(IgcModel::new(), BASE_LOW);
    let nic = Igc::open(sim.clone(), 0x125C, SEED).unwrap();
    (sim, Box::new(nic))
}

pub fn make_rtl8168() -> (Handle<R8169Model>, Box<dyn Nic>) {
    let sim = sim::new(R8169Model::new(), BASE_HIGH);
    let nic = R8169::open(sim.clone(), 0x8168, SEED).unwrap();
    (sim, Box::new(nic))
}

#[test]
fn ids_select_program() {
    assert_eq!(ids::program_for(0x8086, 0x10D3), Some("e1000e"));
    assert_eq!(ids::program_for(0x8086, 0x15BC), Some("e1000e"));
    assert_eq!(ids::program_for(0x8086, 0x125C), Some("igc"));
    assert_eq!(ids::program_for(0x10EC, 0x8168), Some("r8169"));
    // igb (82576), rtl8139, RTL8125 a virtio nepatri nikomu z nich.
    for (v, d) in [(0x8086, 0x10C9), (0x10EC, 0x8139), (0x10EC, 0x8125), (0x1AF4, 0x1041)] {
        assert_eq!(ids::program_for(v, d), None, "{v:04x}:{d:04x}");
    }
    // Zadne ID ve dvou rodinach.
    let mut all: Vec<(u16, u16)> = ids::FAMILIES.iter().flat_map(|f| f.devices.iter().map(|&d| (f.vendor, d))).collect();
    let n = all.len();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), n, "duplicitni ID");
}

#[test]
fn e1000e_mac_from_nvm() {
    let (_, nic) = e1000e(E1000eModel::new(E1000eKind::Discrete), 0x10D3);
    assert_eq!(nic.mac(), [0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);
    assert_eq!(nic.model(), "82574L");
    assert!(nic.notes().is_empty(), "{:?}", nic.notes());
}

#[test]
fn e1000e_mac_from_eeprom_when_rar_empty() {
    let mut m = E1000eModel::new(E1000eKind::Discrete);
    m.nvm_mac = None;
    m.eeprom_mac = Some([0x00, 0x1B, 0x21, 0x01, 0x02, 0x03]);
    let (sim, nic) = e1000e(m, 0x10D3);
    assert_eq!(nic.mac(), [0x00, 0x1B, 0x21, 0x01, 0x02, 0x03]);
    // Zapsana do RAR0 jako platna (karta podle ni filtruje).
    assert_eq!(sim.read32(RAL0), 0x0121_1B00);
    assert_eq!(sim.read32(RAH0), 0x8000_0302);
}

#[test]
fn e1000e_local_mac_without_nvm() {
    let mut m = E1000eModel::new(E1000eKind::Pch);
    m.nvm_mac = None;
    let (_, nic) = e1000e(m, 0x15BC);
    let mac = nic.mac();
    assert_eq!(mac[0], 0x02, "mistne spravovana unicast");
    assert_eq!(mac, crate::local_mac(SEED));
    assert!(nic.notes().iter().any(|n| n.contains("nahodna")));
}

#[test]
fn e1000e_pch_wakes_phy_from_ulp() {
    let mut m = E1000eModel::new(E1000eKind::Pch);
    m.phy_awake = false;
    let (sim, nic) = e1000e(m, 0x15FA);
    assert_eq!(sim.model.borrow().lanphypc_toggles, 1);
    assert!(nic.notes().iter().any(|n| n.contains("probuzen")), "{:?}", nic.notes());
    // Semafor PHY po probuzeni uvolneny (firmware ME ho potrebuje).
    assert_eq!(sim.read32(crate::e1000e::EXTCNF_CTRL) & crate::e1000e::EXTCNF_SWFLAG, 0);
}

#[test]
fn e1000e_pch_respects_me_reset_block() {
    let mut m = E1000eModel::new(E1000eKind::Pch);
    m.phy_awake = false;
    m.phy_reset_blocked = true;
    let (sim, nic) = e1000e(m, 0x15FA);
    assert_eq!(sim.model.borrow().lanphypc_toggles, 0);
    assert!(nic.notes().iter().any(|n| n.contains("ME")), "{:?}", nic.notes());
}

#[test]
fn e1000e_discrete_never_toggles_phy() {
    let mut m = E1000eModel::new(E1000eKind::Discrete);
    m.phy_awake = false;
    let (sim, _) = e1000e(m, 0x10D3);
    assert_eq!(sim.model.borrow().lanphypc_toggles, 0);
    assert_eq!(sim.model.borrow().resets, 1);
}

#[test]
fn e1000e_drops_bad_frames() {
    let (sim, mut nic) = make_82574();
    sim.model.borrow_mut().next_rx_errors = 0x01; // CE = chyba CRC
    sim.deliver(&frame(1, 100));
    sim.deliver(&frame(2, 100));
    assert_eq!(rx_all(&mut *nic), [frame(2, 100)]);
    assert_eq!(nic.stats().rx_errors, 1);
}

#[test]
fn e1000e_unknown_id_rejected() {
    let sim = sim::new(E1000eModel::new(E1000eKind::Discrete), BASE_LOW);
    assert!(E1000e::open(sim, 0x100E, SEED).is_err());
}

#[test]
fn igc_speeds() {
    for (speed, want) in [(2500, 2500), (1000, 1000), (100, 100), (10, 10)] {
        let mut m = IgcModel::new();
        m.speed = speed;
        let sim = sim::new(m, BASE_LOW);
        let nic = Igc::open(sim, 0x15F3, SEED).unwrap();
        assert_eq!(nic.link().unwrap().speed_mbps, want);
        assert_eq!(nic.model(), "I225");
    }
}

#[test]
fn igc_blank_nvm_gets_local_mac() {
    let mut m = IgcModel::new();
    m.nvm_mac = None;
    let sim = sim::new(m, BASE_LOW);
    let nic = Igc::open(sim.clone(), 0x15FD, SEED).unwrap();
    assert_eq!(nic.mac(), crate::local_mac(SEED));
    assert_eq!(sim.read32(crate::igc::RAH0) >> 31, 1, "RAR0 neni platna");
}

#[test]
fn igc_drops_bad_frames() {
    let (sim, mut nic) = make_i226();
    sim.model.borrow_mut().next_rx_error = true;
    sim.deliver(&frame(1, 200));
    sim.deliver(&frame(2, 200));
    assert_eq!(rx_all(&mut *nic), [frame(2, 200)]);
    assert_eq!(nic.stats().rx_errors, 1);
}

#[test]
fn r8169_strips_fcs_and_drops_bad() {
    let (sim, mut nic) = make_rtl8168();
    // Rozdeleny ramec (2 deskriptory) i ramec s chybou zahodit, dalsi projde.
    sim.model.borrow_mut().next_split = true;
    sim.deliver(&frame(1, 300));
    sim.model.borrow_mut().next_rx_error = true;
    sim.deliver(&frame(2, 300));
    sim.deliver(&frame(3, 60));
    assert_eq!(rx_all(&mut *nic), [frame(3, 60)]);
    assert_eq!(nic.stats().rx_errors, 3);
}

#[test]
fn r8169_mac_written_under_unlock() {
    let mut m = R8169Model::new();
    m.mac = [0; 6];
    m.regs[..6].fill(0);
    let sim = sim::new(m, BASE_HIGH);
    let nic = R8169::open(sim.clone(), 0x8168, SEED).unwrap();
    let mac = crate::local_mac(SEED);
    assert_eq!(nic.mac(), mac);
    assert_eq!(&sim.model.borrow().regs[..6], &mac);
    // Konfigurace zase zamcena.
    assert_eq!(sim.read8(crate::r8169::CFG9346), crate::r8169::CFG9346_LOCK);
}

#[test]
fn r8169_low_dma_without_dac() {
    let sim = sim::new(R8169Model::new(), BASE_LOW);
    let mut nic = R8169::open(sim.clone(), 0x8168, SEED).unwrap();
    assert_eq!(sim.read16(crate::r8169::CPLUS_CMD) & crate::r8169::CPLUS_PCI_DAC, 0);
    nic.send(&frame(1, 64));
    nic.kick();
    assert_eq!(sim.take_sent().len(), 1);
}
