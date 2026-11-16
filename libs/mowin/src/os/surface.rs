//! Dvojice bufferu jedne vrstvy (obsah okna nebo popup): kresli se do
//! bufferu, ktery kompozitor prave necte (`RELEASED`), snimek se kopiruje
//! cely (`copy_frame`).

use guiproto::Rect;
use guiproto::window::{ClientMsg, LayerProps, prop};
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::{Handle, Mapping, Result, Rights, Vmo};

use crate::event::copy_frame;

pub(super) struct Buf {
    pub id: u32,
    pub w: u32,
    pub h: u32,
    vmo: Vmo,
    map: Mapping,
    pub busy: bool,
}

impl Buf {
    /// Novy buffer `w` x `h`; vraci ho se zpravou `BUFFER_CREATE`
    /// a handlem VMO jen pro cteni.
    pub fn new(id: u32, w: u32, h: u32) -> Result<(Self, ClientMsg, Handle)> {
        let size = (w as u64 * h as u64 * 4).max(4);
        let vmo = Vmo::create(size)?;
        let map = Mapping::new(&vmo, 0, size, MAP_READ | MAP_WRITE)?;
        let ro = vmo.handle().duplicate(Some(Rights::READ | Rights::MAP | Rights::BASIC))?;
        Ok((Self { id, w, h, vmo, map, busy: false }, ClientMsg::BufferCreate { buffer: id, w, h, stride: w }, ro))
    }

    /// Handle VMO pro kompozitor (jen cteni; obnova po padu).
    pub fn readonly(&self) -> Result<Handle> {
        self.vmo.handle().duplicate(Some(Rights::READ | Rights::MAP | Rights::BASIC))
    }

    /// Zkopiruje snimek (B G R A, krok `pitch`) do bufferu.
    pub fn fill(&mut self, src: &[u8], pitch: u32, sw: u32, sh: u32) -> bool {
        // SAFETY: mapovani `w * h` pixelu (cteni a zapis) zije s bufferem;
        // kompozitor ho cte jen mezi COMMIT a RELEASED (`busy`).
        let px = unsafe { core::slice::from_raw_parts_mut(self.map.addr() as *mut u32, (self.w * self.h) as usize) };
        let _ = &self.vmo;
        copy_frame(px, self.w, self.h, src, pitch, sw, sh)
    }
}

/// Vrstva s dvojici bufferu.
pub(super) struct Surface {
    pub layer: u32,
    pub bufs: [Option<Buf>; 2],
    pub front: usize,
}

impl Surface {
    pub fn new(layer: u32) -> Self {
        Self { layer, bufs: [None, None], front: 1 }
    }

    /// Index zadniho bufferu a co je treba poslat, kdyz ma jinou velikost
    /// (zruseni stareho, vytvoreni noveho).
    pub fn back(&self) -> usize {
        1 - self.front
    }

    /// Vlastnosti vrstvy ukazujici buffer `id` v obdelniku `rect`.
    pub fn props(id: u32, rect: Rect, popover: bool) -> LayerProps {
        let mut mask = prop::BUFFER | prop::RECT;
        if popover {
            mask |= prop::POPOVER | prop::VISIBLE;
        }
        LayerProps { mask, rect, buffer: id, src: Rect::new(0, 0, rect.w, rect.h), popover, visible: true, ..Default::default() }
    }

    /// Id bufferu (pro `BUFFER_DESTROY` pri zaniku vrstvy).
    pub fn buffer_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.bufs.iter().flatten().map(|b| b.id)
    }
}
