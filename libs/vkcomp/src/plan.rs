//! Prevod operace kresleni na parametry shaderu (bez GPU, testy na
//! hostiteli): orez, vyrez zdroje, push constants, pocet skupin.
//!
//! Semantika je stejna jako `gfx::Canvas::draw_image`: vyrez `src` obrazu
//! se nakresli na `dst` (skalovani nejblizsim sousedem, kdyz se rozmery
//! lisi), jen uvnitr `clip` a snimku; vyrez mimo obraz zdroje se orizne.

use guiproto::Rect;
use spirv::blend::{LOCAL, Push};

/// Jedna operace skladani.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Draw {
    /// Zdrojovy povrch.
    pub src: u32,
    /// Vyrez zdroje (pixely povrchu).
    pub src_rect: Rect,
    /// Cil ve snimku.
    pub dst: Rect,
    /// Orez (napr. poskozena oblast).
    pub clip: Rect,
    pub opacity: u8,
    /// Zdroj je nepruhledny (pri `opacity` 255 kopie bez prolinani).
    pub opaque: bool,
}

/// Parametry dispatche: push constants a pocet skupin (x, y).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dispatch {
    pub push: Push,
    pub groups: (u32, u32),
}

/// Spocita dispatch pro `d` do snimku `fw` x `fh`; povrch zdroje ma
/// `sw` x `sh` pixelu a krok `stride` pixelu. `None` = nic se nekresli.
pub fn plan(d: &Draw, fw: u32, fh: u32, sw: u32, sh: u32, stride: u32) -> Option<Dispatch> {
    let sr = d.src_rect.intersect(&Rect::new(0, 0, sw, sh));
    if sr.is_empty() || d.dst.is_empty() || d.opacity == 0 || d.src_rect.is_empty() {
        return None;
    }
    let same = d.src_rect.w == d.dst.w && d.src_rect.h == d.dst.h;
    // Cast cile, kterou pokryva oriznuty vyrez zdroje (u stejne velikosti
    // presne; u skalovani cely cil - orez zdroje mimo obraz je vzacny
    // a skaluje se z puvodniho vyrezu jako v gfx).
    let covered = if same {
        Rect::new(d.dst.x + (sr.x - d.src_rect.x), d.dst.y + (sr.y - d.src_rect.y), sr.w, sr.h)
    } else if sr == d.src_rect {
        d.dst
    } else {
        // Skalovani s vyrezem mimo obraz: ponechat na CPU (gfx).
        return None;
    };
    let c = covered.intersect(&d.clip).intersect(&Rect::new(0, 0, fw, fh));
    if c.is_empty() {
        return None;
    }
    // Shader pocita s cilovym obdelnikem bez zapornych souradnic: posunout
    // pocatek vrstvy, kdyz lezi vlevo nebo nahore mimo snimek (stejna
    // velikost; u skalovani se nezaporny orez resi `lx = px - dx`).
    let (dx, dy, src_x, src_y) = if same {
        (c.x, c.y, sr.x + (c.x - covered.x), sr.y + (c.y - covered.y))
    } else {
        (d.dst.x, d.dst.y, d.src_rect.x, d.src_rect.y)
    };
    // Zaporny pocatek (jen u skalovani, vrstva presahuje vlevo nebo nahore):
    // shader pocita `lx = px - dx` v aritmetice modulo 2^32, takze staci
    // predat bity cisla se znamenkem.
    let (dw, dh, sw2, sh2) = if same { (c.w, c.h, c.w, c.h) } else { (d.dst.w, d.dst.h, d.src_rect.w, d.src_rect.h) };
    let push = Push {
        dx: dx as u32,
        dy: dy as u32,
        dw,
        dh,
        cx: c.x as u32,
        cy: c.y as u32,
        cw: c.w,
        ch: c.h,
        dst_stride: fw,
        src_off: src_y as u32 * stride + src_x as u32,
        src_stride: stride,
        sw: sw2,
        sh: sh2,
        opacity: d.opacity as u32,
        copy: (d.opaque && d.opacity == 255) as u32,
    };
    Some(Dispatch { push, groups: (c.w.div_ceil(LOCAL), c.h.div_ceil(LOCAL)) })
}

/// Referencni provedeni dispatche na CPU (pro testy a overeni GPU): stejne
/// vzorce jako shader.
pub fn run_reference(p: &Push, dst: &mut [u32], src: &[u32]) {
    for gy in 0..p.ch {
        for gx in 0..p.cw {
            let (px, py) = (p.cx + gx, p.cy + gy);
            let (lx, ly) = (px.wrapping_sub(p.dx), py.wrapping_sub(p.dy));
            let sx = lx * p.sw / p.dw;
            let sy = ly * p.sh / p.dh;
            let si = (p.src_off + sy * p.src_stride + sx) as usize;
            let di = (py * p.dst_stride + px) as usize;
            dst[di] = spirv::blend::reference(dst[di], src[si], p.opacity, p.copy == 1);
        }
    }
}
