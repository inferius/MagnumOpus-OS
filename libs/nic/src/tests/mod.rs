//! Testy ovladacu nad simulovanymi registry a DMA (`sim`, modely cipu).
//! Spolecne scenare (`scenarios!`) bezi pro kazdy cip stejne; zvlastnosti
//! cipu (NVM, PHY, chyby ramcu) maji vlastni testy v `chips.rs`.

mod chips;
mod model_e1000e;
mod model_igc;
mod model_r8169;
mod sim;

use std::boxed::Box;
use std::vec::Vec;

use crate::{HwError, MIN_FRAME, Nic, RING};
use sim::{Control, Handle, Model, frame};

/// Vytvori simulaci a ovladac nad ni.
type Make<M> = fn() -> (Handle<M>, Box<dyn Nic>);

fn rx_all(nic: &mut dyn Nic) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    nic.receive(&mut |f| out.push(f.to_vec())).unwrap();
    out
}

/// Ocekavany ramec na drate: kratky doplneny nulami.
fn on_wire(f: &[u8]) -> Vec<u8> {
    let mut v = f.to_vec();
    if v.len() < MIN_FRAME {
        v.resize(MIN_FRAME, 0);
    }
    v
}

pub fn init_and_link<M: Model + Control>(make: Make<M>) {
    let (sim, nic) = make();
    let l = nic.link().unwrap();
    assert!(l.up && l.speed_mbps >= 1000 && l.full_duplex, "{l:?}");
    assert!(crate::valid_mac(&nic.mac()));
    assert!(sim.take_sent().is_empty(), "karta neco poslala pri inicializaci");
}

pub fn tx_frames<M: Model + Control>(make: Make<M>) {
    let (sim, mut nic) = make();
    let frames: Vec<Vec<u8>> = [14, 59, 60, 61, 1000, 1514].iter().enumerate().map(|(i, &l)| frame(i as u32, l)).collect();
    for f in &frames {
        nic.send(f);
    }
    assert!(sim.take_sent().is_empty(), "odeslano pred kick");
    nic.kick();
    let sent = sim.take_sent();
    assert_eq!(sent.len(), frames.len());
    for (s, f) in sent.iter().zip(&frames) {
        assert_eq!(s, &on_wire(f));
    }
    nic.reclaim().unwrap();
    assert_eq!(nic.tx_space(), RING - 1);
}

pub fn tx_ring_wraps<M: Model + Control>(make: Make<M>) {
    let (sim, mut nic) = make();
    let mut next = 0u32;
    let mut got = Vec::new();
    // Dav ruzne velikosti, aby se konec kruhu trefil do ruznych mist.
    for batch in [1usize, 7, 255, 100, 3, 254, 300, 80].iter().cycle().take(20) {
        nic.reclaim().unwrap();
        let n = (*batch).min(nic.tx_space());
        assert!(n > 0, "kruh odesilani se neuvolnil");
        for _ in 0..n {
            nic.send(&frame(next, 64 + (next as usize % 1400)));
            next += 1;
        }
        nic.kick();
        got.extend(sim.take_sent());
    }
    assert_eq!(got.len(), next as usize);
    for (i, f) in got.iter().enumerate() {
        assert_eq!(f, &frame(i as u32, 64 + (i % 1400)), "ramec {i}");
    }
    nic.reclaim().unwrap();
    assert_eq!(nic.tx_space(), RING - 1);
    assert_eq!(nic.stats().tx_frames, u64::from(next));
}

pub fn tx_backpressure<M: Model + Control>(make: Make<M>) {
    let (_sim, mut nic) = make();
    // Bez kick karta nic neodesle: kruh se zaplni a tx_space klesne na 0.
    while nic.tx_space() > 0 {
        nic.send(&frame(1, 100));
    }
    nic.reclaim().unwrap();
    assert_eq!(nic.tx_space(), 0, "neodeslane deskriptory se uvolnily");
    nic.kick();
    nic.reclaim().unwrap();
    assert_eq!(nic.tx_space(), RING - 1);
}

pub fn rx_ring_wraps<M: Model + Control>(make: Make<M>) {
    let (sim, mut nic) = make();
    let mut got = Vec::new();
    for i in 0..1000u32 {
        assert!(sim.deliver(&frame(i, 60 + (i as usize * 7) % 1455)), "karta zahodila ramec {i}");
        if i % 13 == 0 {
            got.extend(rx_all(&mut *nic));
        }
    }
    got.extend(rx_all(&mut *nic));
    assert_eq!(got.len(), 1000);
    for (i, f) in got.iter().enumerate() {
        assert_eq!(f, &frame(i as u32, 60 + (i * 7) % 1455), "ramec {i}");
    }
    assert_eq!(nic.stats().rx_frames, 1000);
    assert_eq!(nic.stats().rx_errors, 0);
}

pub fn rx_overflow_recovers<M: Model + Control>(make: Make<M>) {
    let (sim, mut nic) = make();
    let accepted = (0..300u32).filter(|&i| sim.deliver(&frame(i, 100))).count();
    // Intel: jeden deskriptor drzi ovladac (RDT); Realtek: cely kruh.
    assert!((RING - 1..=RING).contains(&accepted), "prijato {accepted}");
    let got = rx_all(&mut *nic);
    assert_eq!(got.len(), accepted);
    for (i, f) in got.iter().enumerate() {
        assert_eq!(f, &frame(i as u32, 100));
    }
    // Po vyprazdneni prijima dal.
    assert!(sim.deliver(&frame(999, 100)));
    assert_eq!(rx_all(&mut *nic), [frame(999, 100)]);
}

pub fn interrupts_and_link_change<M: Model + Control>(make: Make<M>) {
    let (sim, mut nic) = make();
    nic.interrupt().unwrap();
    let before = sim.interrupts();
    sim.deliver(&frame(1, 64));
    assert!(sim.interrupts() > before, "prijem nevyvolal preruseni");
    let ev = nic.interrupt().unwrap();
    assert!(!ev.link && !ev.fatal);
    rx_all(&mut *nic);
    sim.set_link(false);
    assert!(nic.interrupt().unwrap().link, "zmena linky bez udalosti");
    let l = nic.link().unwrap();
    assert!(!l.up && l.speed_mbps == 0, "{l:?}");
    assert!(!sim.deliver(&frame(2, 64)), "odpojena linka prijima");
    sim.set_link(true);
    assert!(nic.interrupt().unwrap().link);
    assert!(nic.link().unwrap().up);
}

pub fn unplugged<M: Model + Control>(make: Make<M>) {
    let (sim, mut nic) = make();
    sim.unplug();
    assert_eq!(nic.link().unwrap_err(), HwError::Gone);
    assert_eq!(nic.interrupt().unwrap_err(), HwError::Gone);
}

pub fn stop_halts_dma<M: Model + Control>(make: Make<M>) {
    let (sim, mut nic) = make();
    nic.stop();
    assert!(!sim.deliver(&frame(1, 64)), "zastavena karta prijima");
    nic.send(&frame(2, 64));
    nic.kick();
    assert!(sim.take_sent().is_empty(), "zastavena karta odesila");
}

/// Vsechny spolecne scenare pro jeden cip.
macro_rules! scenarios {
    ($module:ident, $make:path) => {
        mod $module {
            #[test]
            fn init_and_link() {
                crate::tests::init_and_link($make)
            }
            #[test]
            fn tx_frames() {
                crate::tests::tx_frames($make)
            }
            #[test]
            fn tx_ring_wraps() {
                crate::tests::tx_ring_wraps($make)
            }
            #[test]
            fn tx_backpressure() {
                crate::tests::tx_backpressure($make)
            }
            #[test]
            fn rx_ring_wraps() {
                crate::tests::rx_ring_wraps($make)
            }
            #[test]
            fn rx_overflow_recovers() {
                crate::tests::rx_overflow_recovers($make)
            }
            #[test]
            fn interrupts_and_link_change() {
                crate::tests::interrupts_and_link_change($make)
            }
            #[test]
            fn unplugged() {
                crate::tests::unplugged($make)
            }
            #[test]
            fn stop_halts_dma() {
                crate::tests::stop_halts_dma($make)
            }
        }
    };
}

scenarios!(e1000e_82574, crate::tests::chips::make_82574);
scenarios!(e1000e_pch, crate::tests::chips::make_i219);
scenarios!(igc_i226, crate::tests::chips::make_i226);
scenarios!(r8169_rtl8168, crate::tests::chips::make_rtl8168);
