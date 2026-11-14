//! Povrchy videa: buffery od aplikace (VMO + `bufdesc`, typicky NV12 od
//! dekoderu medii) ve vlastni vrstve nad oknem, s ploty ziskani
//! a uvolneni (6a-4). Snimek se ukaze bez prekresleni okna: zmena vrstvy
//! (buffer, obdelnik) a body synchronizace v transakci s commitem.
//!
//! Kompozitor dostane buffery jen pro cteni. Po padu kompozitoru se
//! povrch pri dalsim snimku vytvori znovu (zaznam obnovy okna buffery
//! s popisem nezna).

use alloc::vec::Vec;

use bufdesc::BufDesc;
use guiproto::Rect;
use guiproto::window::{ClientMsg, LayerKind, LayerProps, prop};
use libos::{Error, Fence, Result, Rights, Vmo};

use super::Window;
use crate::MAX_VIDEOS;

/// Vrstvy ploch videa: `VIDEO_LAYER_BASE + id` (pod `SCROLL_BASE`).
const VIDEO_LAYER_BASE: u32 = 900;
/// Buffery povrchu: horni bit + id plochy + cislo bufferu.
const VIDEO_BUFFER_BASE: u32 = 0x8000_0000;
/// Poradi vrstvy videa nad obsahem okna (pod popoverem a kurzorem).
const VIDEO_ORDER: i32 = 1;

/// Povrch videa plochy `id` (`node::video`): buffery a ploty. Plot
/// ziskani posouva producent (dekoder), plot uvolneni kompozitor.
pub struct VideoSurface {
    pub id: u32,
    pub descs: Vec<BufDesc>,
    pub vmos: Vec<Vmo>,
    /// Plot ziskani (staci pravo cekat).
    pub acquire: Fence,
    /// Plot uvolneni (kompozitor dostane pravo ho posouvat).
    pub release: Fence,
}

pub(crate) struct Video {
    surf: VideoSurface,
    /// Generace kompozitoru, na ktere povrch existuje.
    generation: Option<u64>,
    /// Posledni pouzita hodnota plotu uvolneni.
    release: u64,
    /// Obdelnik a buffer vrstvy, jak je zna kompozitor.
    rect: Option<Rect>,
    slot: Option<u8>,
}

fn buffer_id(id: u32, slot: u8) -> u32 {
    VIDEO_BUFFER_BASE | (id << 8) | slot as u32
}

impl Video {
    pub(crate) fn new(surf: VideoSurface) -> Result<Self> {
        if surf.id >= MAX_VIDEOS || surf.descs.is_empty() || surf.descs.len() != surf.vmos.len() || surf.descs.len() > 255 {
            return Err(Error::InvalidArgs);
        }
        let release = surf.release.query()?.value;
        Ok(Self { surf, generation: None, release, rect: None, slot: None })
    }

    pub(crate) fn id(&self) -> u32 {
        self.surf.id
    }
}

impl Window {
    /// Vytvori vrstvu a buffery povrchu na soucasne instanci kompozitoru.
    fn video_attach(&mut self, v: &mut Video) -> Result<()> {
        let layer = VIDEO_LAYER_BASE + v.surf.id;
        self.send(ClientMsg::LayerCreate { layer, window: 1, parent: 0, kind: LayerKind::Buffer })?;
        self.send(ClientMsg::LayerSet { layer, props: LayerProps { mask: prop::ORDER, order: VIDEO_ORDER, ..Default::default() } })?;
        for (i, (d, vmo)) in v.surf.descs.iter().zip(&v.surf.vmos).enumerate() {
            let ro = vmo.handle().duplicate(Some(Rights::READ | Rights::MAP | Rights::BASIC))?;
            self.send_with(ClientMsg::BufferCreateDesc { buffer: buffer_id(v.surf.id, i as u8), desc: d.clone() }, alloc::vec![ro])?;
        }
        v.generation = Some(self.generation());
        v.rect = None;
        v.slot = None;
        Ok(())
    }

    /// Ukaze buffer `slot` v `rect`, az plot ziskani dosahne `acquire`.
    /// Vrati hodnotu plotu uvolneni tohoto snimku. Plati s pristim commitem.
    pub(crate) fn video_show(&mut self, v: &mut Video, slot: u8, acquire: u64, rect: Rect) -> Result<u64> {
        if slot as usize >= v.surf.descs.len() {
            return Err(Error::InvalidArgs);
        }
        if v.generation != Some(self.generation()) {
            self.video_attach(v)?;
        }
        let layer = VIDEO_LAYER_BASE + v.surf.id;
        let props = LayerProps {
            mask: prop::BUFFER | prop::RECT | prop::VISIBLE,
            rect,
            buffer: buffer_id(v.surf.id, slot),
            visible: true,
            ..Default::default()
        };
        self.send(ClientMsg::LayerSet { layer, props })?;
        v.release += 1;
        let hs = alloc::vec![v.surf.acquire.for_waiter()?.into_handle(), v.surf.release.handle().duplicate(None)?];
        self.send_with(ClientMsg::SyncPoints { window: 1, acquire: Some(acquire), release: Some(v.release) }, hs)?;
        v.rect = Some(rect);
        v.slot = Some(slot);
        Ok(v.release)
    }

    /// Po zmene rozvrzeni: vrstva na novy obdelnik (nebo skryt, kdyz plocha
    /// zmizela). Vrati, zda se neco poslalo (pak je potreba commit).
    pub(crate) fn video_place(&mut self, v: &mut Video, rect: Option<Rect>) -> Result<bool> {
        if v.slot.is_none() || v.generation != Some(self.generation()) || v.rect == rect {
            return Ok(false);
        }
        let layer = VIDEO_LAYER_BASE + v.surf.id;
        let props = match rect {
            Some(r) => LayerProps { mask: prop::RECT | prop::VISIBLE, rect: r, visible: true, ..Default::default() },
            None => LayerProps { mask: prop::VISIBLE, visible: false, ..Default::default() },
        };
        self.send(ClientMsg::LayerSet { layer, props })?;
        v.rect = rect;
        Ok(true)
    }
}
