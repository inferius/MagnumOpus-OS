//! Pool bufferu videa: VMO od aplikace (pres `mediad`) a plot snimku.

use alloc::vec::Vec;

use bufdesc::{BufDesc, format};
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::{Error, Fence, Handle, Mapping, Result, Vmo};
use mediaproto::testmedia::{self, Header};

struct Buf {
    map: Mapping,
    desc: BufDesc,
    /// Buffer je u dekoderu (aplikace ho vratila `RELEASE`).
    free: bool,
}

pub struct Pool {
    bufs: Vec<Buf>,
    fence: Fence,
    /// Hodnota plotu posledniho dokonceneho snimku.
    serial: u64,
}

impl Pool {
    /// Handly: VMO v poradi popisu, nakonec plot snimku.
    pub fn new(descs: Vec<BufDesc>, mut hs: Vec<Handle>) -> Result<Self> {
        if hs.len() != descs.len() + 1 {
            return Err(Error::InvalidArgs);
        }
        let fence = Fence::from_handle(hs.pop().ok_or(Error::InvalidArgs)?);
        let serial = fence.query()?.value;
        let mut bufs = Vec::new();
        for (desc, h) in descs.into_iter().zip(hs) {
            if desc.fourcc != format::NV12 || desc.planes.len() != 2 {
                return Err(Error::NotSupported);
            }
            let vmo = Vmo::from_handle(h);
            let size = desc.min_size().map_err(|_| Error::InvalidArgs)?;
            // Velikost VMO overi mapovani: za koncem VMO mapovat nejde.
            let map = Mapping::new(&vmo, 0, size, MAP_READ | MAP_WRITE)?;
            desc.validate(map.len()).map_err(|_| Error::InvalidArgs)?;
            bufs.push(Buf { map, desc, free: true });
        }
        Ok(Self { bufs, fence, serial })
    }

    pub fn free_slot(&self) -> Option<u8> {
        self.bufs.iter().position(|b| b.free).map(|i| i as u8)
    }

    pub fn release(&mut self, slot: u8) -> Result<()> {
        let b = self.bufs.get_mut(slot as usize).ok_or(Error::InvalidArgs)?;
        if b.free {
            return Err(Error::BadState);
        }
        b.free = true;
        Ok(())
    }

    /// Vykresli snimek do bufferu `slot` a posune plot; vrati jeho hodnotu.
    pub fn fill(&mut self, slot: u8, h: &Header, seed: u32) -> Result<u64> {
        let b = self.bufs.get_mut(slot as usize).ok_or(Error::InvalidArgs)?;
        if b.desc.width != h.width || b.desc.height != h.height {
            return Err(Error::InvalidArgs);
        }
        let (y, uv) = (b.desc.planes[0], b.desc.planes[1]);
        // SAFETY: mapovani je cele nase a zapisovatelne (MAP_WRITE), delka
        // `map.len()` je namapovana po celou dobu zivota `Mapping`; buffer
        // je volny (aplikace ho vratila), takze ho nikdo jiny necte.
        let mem = unsafe { core::slice::from_raw_parts_mut(b.map.addr() as *mut u8, b.map.len() as usize) };
        // `validate` overil, ze roviny lezi v mapovani; Y je pred UV.
        let (ys, uvs) = mem.split_at_mut(uv.offset as usize);
        testmedia::fill_nv12(h.width, h.height, &mut ys[y.offset as usize..], y.pitch as usize, uvs, uv.pitch as usize, seed);
        b.free = false;
        self.serial += 1;
        self.fence.signal(self.serial)?;
        Ok(self.serial)
    }
}
