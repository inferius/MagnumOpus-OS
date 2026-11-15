//! Zaznam platneho stavu okna pro obnovu po padu kompozitoru.
//!
//! Klient ([`Replay::record`]) zaznamenava vse, co kompozitoru posila.
//! Zpravy, ktere kompozitor odklada do `COMMIT` ([`ClientMsg::is_staged`]),
//! se do stavu zapocitaji az s `COMMIT` - stejne jako v kompozitoru. Po
//! padu kompozitoru tak [`Replay::replay`] vrati zpravy, ktere na nove
//! instanci postavi **posledni potvrzeny** stav: okna, buffery, strom
//! vrstev s vlastnostmi, oblasti titulku a strom pristupnosti.
//!
//! Rozpracovana transakce (zpravy od posledniho `COMMIT`) se neobnovi:
//! nevime, kolik z ni stara instance prijala. [`Replay::lost_staged`] rekne,
//! jestli nejaka byla - klient pak vrati `SERVICE_RESTARTED` a aplikace
//! nakresli okno znovu (`docs/cs/obnova-sluzeb.md`).
//!
//! Neobnovi se: probihajici animace (vrstva skonci rovnou v cilove
//! hodnote), skupiny transakci (`SYNC_GROUP` - token patri stare instanci)
//! a volani akci (`ACTION_RESULT` - volajici patril stare instanci).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;

use crate::a11y::Node;
use crate::window::{AnimProp, ClientMsg, LayerKind, LayerProps, Mode, a11y_chunks, prop};

/// Platny stav jednoho okna.
#[derive(Debug, Clone)]
struct Win {
    mode: Mode,
    w: u32,
    h: u32,
    title: String,
    /// Posledni `WINDOW_SET` (cela zprava, nese vsechna pole).
    set: Option<ClientMsg>,
    regions: Option<ClientMsg>,
    /// Uzly stromu pristupnosti podle id (poradi = poradi id).
    a11y: BTreeMap<u32, Node>,
    /// Vlastnik okna (`WINDOW_OWNER`), 0 = zadny.
    owner: u32,
}

/// Platny stav jedne vrstvy.
#[derive(Debug, Clone)]
struct Layer {
    /// Poradi vytvoreni (rodic se vytvori pred ditetem).
    seq: u64,
    window: u32,
    parent: u32,
    kind: LayerKind,
    /// Vlastnosti; `mask` = ktera pole kdy klient nastavil.
    props: LayerProps,
}

/// Zaznam stavu oken jednoho klienta.
#[derive(Debug, Clone, Default)]
pub struct Replay {
    windows: BTreeMap<u32, Win>,
    /// Buffer -> (sirka, vyska, stride). Buffery plati hned (necekaji na
    /// `COMMIT`).
    buffers: BTreeMap<u32, (u32, u32, u32)>,
    layers: BTreeMap<u32, Layer>,
    next_seq: u64,
    /// Zpravy od posledniho `COMMIT`.
    staged: Vec<ClientMsg>,
}

impl Replay {
    pub fn new() -> Self {
        Self::default()
    }

    /// Zaznamena zpravu odeslanou kompozitoru.
    pub fn record(&mut self, m: &ClientMsg) {
        if m.is_staged() {
            self.staged.push(m.clone());
            return;
        }
        match m {
            ClientMsg::WindowCreate { window, mode, w, h, title } => {
                self.windows.insert(
                    *window,
                    Win {
                        mode: *mode,
                        w: *w,
                        h: *h,
                        title: title.clone(),
                        set: None,
                        regions: None,
                        a11y: BTreeMap::new(),
                        owner: 0,
                    },
                );
            }
            ClientMsg::WindowOwner { window, owner } => {
                if let Some(w) = self.windows.get_mut(window) {
                    w.owner = *owner;
                }
            }
            ClientMsg::WindowDestroy { window } => {
                self.windows.remove(window);
                for w in self.windows.values_mut().filter(|w| w.owner == *window) {
                    w.owner = 0;
                }
                self.layers.retain(|_, l| l.window != *window);
            }
            ClientMsg::BufferCreate { buffer, w, h, stride } => {
                self.buffers.insert(*buffer, (*w, *h, *stride));
            }
            ClientMsg::BufferDestroy { buffer } => {
                self.buffers.remove(buffer);
            }
            ClientMsg::Commit { .. } => {
                for s in core::mem::take(&mut self.staged) {
                    self.apply(s);
                }
            }
            _ => {}
        }
    }

    /// Kompozitor potvrdil novou velikost okna (`CONFIGURE`): obnova ho
    /// vytvori uz v ni.
    pub fn configured(&mut self, window: u32, w: u32, h: u32) {
        if let Some(win) = self.windows.get_mut(&window) {
            win.w = w;
            win.h = h;
        }
    }

    /// Je rozpracovana transakce (zpravy bez `COMMIT`)?
    pub fn lost_staged(&self) -> bool {
        !self.staged.is_empty()
    }

    /// Zahodi rozpracovanou transakci (po obnove; aplikace ji posle znovu).
    pub fn drop_staged(&mut self) {
        self.staged.clear();
    }

    /// Okna v zaznamu.
    pub fn windows(&self) -> impl Iterator<Item = u32> + '_ {
        self.windows.keys().copied()
    }

    /// Buffer, ktery vrstva `layer` v platnem stavu ukazuje.
    pub fn layer_buffer(&self, layer: u32) -> Option<u32> {
        let l = self.layers.get(&layer)?;
        (l.props.mask & prop::BUFFER != 0 && l.props.buffer != 0).then_some(l.props.buffer)
    }

    fn apply(&mut self, m: ClientMsg) {
        match m {
            ClientMsg::WindowSet { window, ref title, .. } => {
                if let Some(w) = self.windows.get_mut(&window) {
                    w.title = title.clone();
                    w.set = Some(m);
                }
            }
            ClientMsg::LayerCreate { layer, window, parent, kind } => {
                // Rodic musi existovat (jinak ho kompozitor odmitl).
                if !self.windows.contains_key(&window) || (parent != 0 && !self.layers.contains_key(&parent)) {
                    return;
                }
                let seq = self.next_seq;
                self.next_seq += 1;
                self.layers.insert(
                    layer,
                    Layer {
                        seq,
                        window,
                        parent,
                        kind,
                        props: LayerProps::default(),
                    },
                );
            }
            ClientMsg::LayerSet { layer, props } => {
                if let Some(l) = self.layers.get_mut(&layer) {
                    merge(&mut l.props, &props);
                }
            }
            ClientMsg::LayerDestroy { layer } => self.destroy_layer(layer),
            ClientMsg::Animate { layer, prop: p, target, .. } => {
                // Animace se neobnovuje: vrstva rovnou v cilove hodnote.
                if let Some(l) = self.layers.get_mut(&layer) {
                    let v = &mut l.props;
                    match p {
                        AnimProp::Opacity => {
                            v.opacity = target.clamp(0, 255) as u8;
                            v.mask |= prop::OPACITY;
                        }
                        AnimProp::X => {
                            v.rect.x = target;
                            v.mask |= prop::RECT;
                        }
                        AnimProp::Y => {
                            v.rect.y = target;
                            v.mask |= prop::RECT;
                        }
                        AnimProp::ScrollX => {
                            v.scroll.0 = target;
                            v.mask |= prop::SCROLL;
                        }
                        AnimProp::ScrollY => {
                            v.scroll.1 = target;
                            v.mask |= prop::SCROLL;
                        }
                    }
                }
            }
            ClientMsg::A11yUpdate { window, nodes } => {
                if let Some(w) = self.windows.get_mut(&window) {
                    for n in nodes {
                        w.a11y.insert(n.id, n);
                    }
                }
            }
            ClientMsg::A11yRemove { window, ids } => {
                if let Some(w) = self.windows.get_mut(&window) {
                    for id in ids {
                        w.a11y.remove(&id);
                    }
                }
            }
            ClientMsg::Regions { window, .. } => {
                if let Some(w) = self.windows.get_mut(&window) {
                    w.regions = Some(m);
                }
            }
            _ => {}
        }
    }

    /// Odstrani vrstvu i s potomky.
    fn destroy_layer(&mut self, layer: u32) {
        let mut gone = BTreeSet::new();
        gone.insert(layer);
        loop {
            let more: Vec<u32> = self
                .layers
                .iter()
                .filter(|(id, l)| gone.contains(&l.parent) && !gone.contains(*id))
                .map(|(id, _)| *id)
                .collect();
            if more.is_empty() {
                break;
            }
            gone.extend(more);
        }
        self.layers.retain(|id, _| !gone.contains(id));
    }

    /// Zpravy, ktere na nove instanci kompozitoru postavi platny stav (bez
    /// `HELLO` a bez `COMMIT`). Ke kazdemu `BUFFER_CREATE` patri handle
    /// VMO bufferu - pripoji ho klient (VMO drzi on). Po nich klient pocka
    /// na `CONFIGURE` a posle `COMMIT`.
    pub fn replay(&self) -> Vec<ClientMsg> {
        let mut out = Vec::new();
        for (&window, w) in &self.windows {
            out.push(ClientMsg::WindowCreate {
                window,
                mode: w.mode,
                w: w.w,
                h: w.h,
                title: w.title.clone(),
            });
        }
        // Vlastnici az po vytvoreni vsech oken.
        for (&window, w) in self.windows.iter().filter(|(_, w)| w.owner != 0) {
            out.push(ClientMsg::WindowOwner { window, owner: w.owner });
        }
        for (&buffer, &(w, h, stride)) in &self.buffers {
            out.push(ClientMsg::BufferCreate { buffer, w, h, stride });
        }
        // Rodice pred detmi: v poradi vytvoreni.
        let mut layers: Vec<(&u32, &Layer)> = self.layers.iter().collect();
        layers.sort_by_key(|(_, l)| l.seq);
        for &(&layer, l) in &layers {
            out.push(ClientMsg::LayerCreate {
                layer,
                window: l.window,
                parent: l.parent,
                kind: l.kind,
            });
        }
        for &(&layer, l) in &layers {
            let mut props = l.props;
            // Buffer, ktery uz neexistuje, by kompozitor odmitl.
            if props.mask & prop::BUFFER != 0 && !self.buffers.contains_key(&props.buffer) {
                props.mask &= !prop::BUFFER;
            }
            if props.mask != 0 {
                out.push(ClientMsg::LayerSet { layer, props });
            }
        }
        for (&window, w) in &self.windows {
            out.extend(w.set.iter().cloned());
            out.extend(w.regions.iter().cloned());
            let nodes: Vec<Node> = w.a11y.values().cloned().collect();
            out.extend(a11y_chunks(window, &nodes));
        }
        out
    }
}

/// Slouci zmenu vlastnosti (`LAYER_SET` plati jen pro pole z `mask`).
pub fn merge(into: &mut LayerProps, change: &LayerProps) {
    let m = change.mask;
    if m & prop::RECT != 0 {
        into.rect = change.rect;
    }
    if m & prop::ORDER != 0 {
        into.order = change.order;
    }
    if m & prop::OPACITY != 0 {
        into.opacity = change.opacity;
    }
    if m & prop::VISIBLE != 0 {
        into.visible = change.visible;
    }
    if m & prop::CLIP != 0 {
        into.clip = change.clip;
    }
    if m & prop::SCROLL != 0 {
        into.scroll = change.scroll;
    }
    if m & prop::BUFFER != 0 {
        into.buffer = change.buffer;
        into.src = change.src;
    }
    if m & prop::COLOR != 0 {
        into.color = change.color;
    }
    into.mask |= m & prop::ALL;
}
