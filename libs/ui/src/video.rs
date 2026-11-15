//! Plocha videa (`node::video`): jadro jen rezervuje misto a spocita
//! obdelnik obrazu se zachovanim pomeru stran. Snimky ukazuje klient
//! (`client/video.rs`) ve vlastni vrstve z bufferu, ktere mu dala
//! aplikace - okno se pri novem snimku neprekresluje.

use alloc::vec::Vec;

use guiproto::Rect;

use crate::Ui;
use crate::layout::ElemKind;

/// Nejvic ploch videa v okne (vrstvy `VIDEO_LAYER_BASE + id`).
pub const MAX_VIDEOS: u32 = 8;

/// Snimek, ktery ma klient ukazat v plose `id`: buffer `slot` povrchu
/// videa, jakmile plot ziskani dosahne `acquire`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoShow {
    pub id: u32,
    pub slot: u8,
    pub acquire: u64,
}

/// Vysledek [`crate::App::video_frame`]: snimky k ukazani a zda se ma
/// prekreslit okno (napr. zmenil se cas na posuvniku).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VideoTick {
    pub shows: Vec<VideoShow>,
    pub redraw: bool,
}

/// Obdelnik obrazu `w` x `h` zmenseneho do `r` se zachovanim pomeru
/// stran, na stred (zbytek = pruhy pozadi okna).
pub fn fit(r: Rect, w: u32, h: u32) -> Rect {
    if w == 0 || h == 0 || r.w == 0 || r.h == 0 {
        return Rect::new(r.x, r.y, 0, 0);
    }
    let k = (r.w as u64 * h as u64).min(r.h as u64 * w as u64);
    let (dw, dh) = ((k / h as u64) as u32, (k / w as u64) as u32);
    Rect::new(r.x + (r.w - dw) as i32 / 2, r.y + (r.h - dh) as i32 / 2, dw, dh)
}

impl<M: Clone> Ui<M> {
    /// Obdelniky ploch videa v souradnicich okna (jen mimo rolovane
    /// oblasti; video v rolovani se zatim neukazuje).
    pub fn video_rects(&self) -> Vec<(u32, Rect)> {
        self.elems
            .iter()
            .filter(|e| e.space == 0)
            .filter_map(|e| match &e.kind {
                ElemKind::Video { id, w, h, .. } if *id < MAX_VIDEOS => Some((*id, fit(e.rect, *w, *h))),
                _ => None,
            })
            .collect()
    }
}
