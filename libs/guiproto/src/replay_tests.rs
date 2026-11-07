//! Testy zaznamu stavu okna (`replay`).

use alloc::vec;
use alloc::vec::Vec;

use crate::Rect;
use crate::a11y::{Node, Role};
use crate::replay::Replay;
use crate::window::{AnimProp, ClientMsg, Curve, LayerKind, LayerProps, Mode, RegionKind, prop};

fn commit() -> ClientMsg {
    ClientMsg::Commit { txn: 1, flags: 0, window: 1, serial: 0, token: 0 }
}

fn set(layer: u32, props: LayerProps) -> ClientMsg {
    ClientMsg::LayerSet { layer, props }
}

/// Okno s korenovou vrstvou a bufferem 7, potvrzene.
fn base() -> Replay {
    let mut r = Replay::new();
    r.record(&ClientMsg::Hello { version: 1 });
    r.record(&ClientMsg::WindowCreate { window: 1, mode: Mode::Standard, w: 300, h: 200, title: "Okno".into() });
    r.record(&ClientMsg::BufferCreate { buffer: 7, w: 300, h: 200, stride: 300 });
    r.record(&ClientMsg::LayerCreate { layer: 1, window: 1, parent: 0, kind: LayerKind::Buffer });
    r.record(&set(1, LayerProps { mask: prop::BUFFER | prop::RECT, rect: Rect::new(0, 0, 300, 200), buffer: 7, src: Rect::new(0, 0, 300, 200), ..Default::default() }));
    r.record(&ClientMsg::A11yUpdate { window: 1, nodes: vec![Node::new(1, 0, Role::Window, "Okno"), Node::new(2, 1, Role::Button, "OK")] });
    r.record(&commit());
    r
}

#[test]
fn replay_rebuilds_committed_state() {
    let r = base();
    assert!(!r.lost_staged());
    assert_eq!(r.layer_buffer(1), Some(7));
    let m = r.replay();
    assert!(matches!(&m[0], ClientMsg::WindowCreate { window: 1, w: 300, h: 200, .. }));
    assert!(matches!(&m[1], ClientMsg::BufferCreate { buffer: 7, .. }));
    assert!(matches!(&m[2], ClientMsg::LayerCreate { layer: 1, parent: 0, .. }));
    match &m[3] {
        ClientMsg::LayerSet { layer: 1, props } => {
            assert_eq!(props.buffer, 7);
            assert_eq!(props.rect, Rect::new(0, 0, 300, 200));
        }
        o => panic!("{o:?}"),
    }
    match &m[4] {
        ClientMsg::A11yUpdate { window: 1, nodes } => assert_eq!(nodes.len(), 2),
        o => panic!("{o:?}"),
    }
    assert_eq!(m.len(), 5);
}

#[test]
fn staged_changes_wait_for_commit() {
    let mut r = base();
    r.record(&set(1, LayerProps { mask: prop::OPACITY, opacity: 10, ..Default::default() }));
    r.record(&ClientMsg::A11yRemove { window: 1, ids: vec![2] });
    assert!(r.lost_staged());
    // Bez COMMIT se zmena do obnovy nedostane.
    let m = r.replay();
    assert!(m.iter().any(|x| matches!(x, ClientMsg::LayerSet { props, .. } if props.opacity == 255)));
    assert!(m.iter().any(|x| matches!(x, ClientMsg::A11yUpdate { nodes, .. } if nodes.len() == 2)));
    r.drop_staged();
    assert!(!r.lost_staged());
    // Po COMMIT plati (a sloucena s predchozimi vlastnostmi).
    r.record(&set(1, LayerProps { mask: prop::OPACITY, opacity: 10, ..Default::default() }));
    r.record(&commit());
    let m = r.replay();
    match m.iter().find(|x| matches!(x, ClientMsg::LayerSet { .. })) {
        Some(ClientMsg::LayerSet { props, .. }) => {
            assert_eq!(props.opacity, 10);
            assert_eq!(props.buffer, 7, "buffer zustal");
            assert_eq!(props.mask, prop::OPACITY | prop::BUFFER | prop::RECT);
        }
        o => panic!("{o:?}"),
    }
}

#[test]
fn layers_children_after_parents_and_destroy_recursive() {
    let mut r = base();
    r.record(&ClientMsg::LayerCreate { layer: 10, window: 1, parent: 0, kind: LayerKind::Group });
    r.record(&ClientMsg::LayerCreate { layer: 11, window: 1, parent: 10, kind: LayerKind::Buffer });
    r.record(&set(10, LayerProps { mask: prop::SCROLL | prop::CLIP, scroll: (0, -40), clip: true, ..Default::default() }));
    r.record(&ClientMsg::Animate { layer: 10, prop: AnimProp::ScrollY, target: -80, ms: 100, curve: Curve::Linear });
    // Vrstva s neexistujicim rodicem se nezaznamena (kompozitor ji odmitne).
    r.record(&ClientMsg::LayerCreate { layer: 50, window: 1, parent: 49, kind: LayerKind::Buffer });
    r.record(&commit());
    let m = r.replay();
    let creates: Vec<u32> = m.iter().filter_map(|x| if let ClientMsg::LayerCreate { layer, .. } = x { Some(*layer) } else { None }).collect();
    assert_eq!(creates, vec![1, 10, 11]);
    assert!(m.iter().any(|x| matches!(x, ClientMsg::LayerSet { layer: 10, props } if props.scroll == (0, -80))), "animace v cilove hodnote");
    r.record(&ClientMsg::LayerDestroy { layer: 10 });
    r.record(&commit());
    let m = r.replay();
    assert!(!m.iter().any(|x| matches!(x, ClientMsg::LayerCreate { layer: 10 | 11, .. })), "skupina i s detmi pryc");
}

#[test]
fn destroyed_buffer_not_referenced() {
    let mut r = base();
    r.record(&ClientMsg::BufferDestroy { buffer: 7 });
    let m = r.replay();
    assert!(!m.iter().any(|x| matches!(x, ClientMsg::BufferCreate { .. })));
    assert!(m.iter().any(|x| matches!(x, ClientMsg::LayerSet { props, .. } if props.mask & prop::BUFFER == 0)));
}

#[test]
fn window_set_regions_and_configure() {
    let mut r = base();
    let ws = ClientMsg::WindowSet { window: 1, title: "Nový".into(), min_w: 10, min_h: 10, side: crate::window::ButtonSide::Right, button_color: 0 };
    let rg = ClientMsg::Regions { window: 1, regions: vec![(Rect::new(0, 0, 300, 30), RegionKind::Drag)] };
    r.record(&ws);
    r.record(&rg);
    r.record(&commit());
    r.configured(1, 640, 480);
    let m = r.replay();
    assert!(matches!(&m[0], ClientMsg::WindowCreate { w: 640, h: 480, title, .. } if title == "Nový"));
    assert!(m.contains(&ws) && m.contains(&rg));
    r.record(&ClientMsg::WindowDestroy { window: 1 });
    assert!(r.replay().iter().all(|x| matches!(x, ClientMsg::BufferCreate { .. })), "jen buffery zustaly");
}
