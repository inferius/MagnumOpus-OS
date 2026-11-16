//! Protokol blokoveho zarizeni (verze 1): zpravy po kanalu mezi klientem
//! (napr. souborovy system) a ovladacem disku.
//!
//! Pozadavek: hlavicka 16 B (`op` u32, `count` u32, `lba` u64) a u zapisu
//! data. Odpoved: hlavicka 16 B (`status` i32 = 0 nebo kod chyby, rezerva,
//! `value` u64) a u cteni data. Sektor ma 512 B, jeden pozadavek prenese
//! nejvyse `MAX_SECTORS` sektoru (data se vejdou do zpravy kanalu); vyjimkou
//! je `DISCARD`, ktery nenese data a `count` muze byt libovolny.
//!
//! Zapis a DISCARD na disk, ktery neni nas, vraci `READ_ONLY` (ochrana
//! cizich disku, `server` a knihovna `diskguard`).
//!
//! Verze 1 kopiruje data zpravou. Ovladac zpracovava pozadavky po jednom
//! v poradi prichodu a odpovida ve stejnem poradi, takze klient smi poslat
//! vic zapisu bez cekani (`BlockClient::write_queued`): odpovedi vybere
//! pred dalsim pozadavkem jineho druhu (cteni, flush, ...). Cteni po zapisu
//! tak vidi zapsana data a chyba zapisu se ohlasi nejpozdeji u `flush`.
//! Stejne tak dlouhe cteni (`BlockClient::read`) posle vic pozadavku
//! najednou; ovladac s pomalym prikazem (USB) navazujici cteni spoji
//! (`server::Disks::max_read`).
//! TODO: sdilena pamet a fronty pozadavku (asynchronni I/O podle navrhu).

#![no_std]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use libos::{Channel, Error, Result};

pub mod server;

pub const SECTOR: usize = 512;
/// Jak dlouho klient ceka na odpoved ovladace (pak `TIMED_OUT`).
pub const REPLY_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(30);
pub const MAX_SECTORS: u32 = 64;
/// Nejvic zapisu na ceste bez odpovedi (32 x 32 KiB = 1 MiB, hluboko pod
/// limitem fronty kanalu).
pub const MAX_QUEUED_WRITES: u32 = 32;
/// Nejvic cteni na ceste bez odpovedi v jednom `BlockClient::read`
/// (32 x 32 KiB = 1 MiB odpovedi ve fronte kanalu).
pub const READ_AHEAD: usize = 32;
pub const HEADER: usize = 16;

/// Kody operaci.
pub mod op {
    /// Informace o zarizeni: `value` = pocet sektoru.
    pub const INFO: u32 = 0;
    pub const READ: u32 = 1;
    pub const WRITE: u32 = 2;
    /// Zapsat vse z mezipameti zarizeni na medium.
    pub const FLUSH: u32 = 3;
    /// Sektory `lba..lba+count` uz nejsou pouzite (TRIM/UNMAP). Jen
    /// napoveda: obsah je pak nedefinovany. Bez podpory `NOT_SUPPORTED`.
    pub const DISCARD: u32 = 4;
    /// Vlastnosti zarizeni: `value` = bity z `feature`. Starsi ovladac
    /// vraci `NOT_SUPPORTED` (= zadne vlastnosti, tedy bez zaruky
    /// trvanlivosti).
    pub const FEATURES: u32 = 5;
    /// Synchronizace: `value` = `lba` pozadavku (ozvena). Ovladac zpracovava
    /// pozadavky po jednom v poradi, takze vse pred odpovedi na `SYNC` jsou
    /// pozdni odpovedi na starsi pozadavky - nova instance klienta (restart
    /// sluzby po padu, docs/cs/obnova-sluzeb.md) je zahodi.
    pub const SYNC: u32 = 6;
    /// Test ochrany DMA (IOMMU): ovladac necha zarizeni zapsat do sve
    /// stranky, ktera neni DMA pamet. `value` = 1 zapis zablokovan, 0 ne.
    pub const TEST_DMA_PROTECTION: u32 = 100;
}

/// Bity odpovedi na `op::FEATURES`.
pub mod feature {
    /// `FLUSH` opravdu ulozi data z mezipameti zarizeni na medium.
    pub const FLUSH: u64 = 1 << 0;
    /// Zarizeni nema mezipamet zapisu: dokonceny zapis je trvaly.
    pub const WRITE_THROUGH: u64 = 1 << 1;
    /// Zarizeni umi `DISCARD`.
    pub const DISCARD: u64 = 1 << 2;
    /// Zarizeni je jen pro cteni.
    pub const READ_ONLY: u64 = 1 << 3;
}

/// Rozebrany pozadavek.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    pub op: u32,
    pub count: u32,
    pub lba: u64,
}

impl Request {
    pub fn encode(&self, data: &[u8]) -> Vec<u8> {
        let mut m = Vec::with_capacity(HEADER + data.len());
        m.extend_from_slice(&self.op.to_le_bytes());
        m.extend_from_slice(&self.count.to_le_bytes());
        m.extend_from_slice(&self.lba.to_le_bytes());
        m.extend_from_slice(data);
        m
    }

    pub fn decode(m: &[u8]) -> Option<(Self, &[u8])> {
        let h = m.get(..HEADER)?;
        let r = Self {
            op: u32::from_le_bytes(h[0..4].try_into().ok()?),
            count: u32::from_le_bytes(h[4..8].try_into().ok()?),
            lba: u64::from_le_bytes(h[8..16].try_into().ok()?),
        };
        Some((r, &m[HEADER..]))
    }
}

/// Odpoved ovladace.
pub fn encode_reply(result: Result<u64>, data: &[u8]) -> Vec<u8> {
    let (status, value) = match result {
        Ok(v) => (0i32, v),
        Err(e) => (e.code(), 0),
    };
    let mut m = Vec::with_capacity(HEADER + data.len());
    m.extend_from_slice(&status.to_le_bytes());
    m.extend_from_slice(&0u32.to_le_bytes());
    m.extend_from_slice(&value.to_le_bytes());
    m.extend_from_slice(data);
    m
}

fn decode_reply(m: &[u8]) -> Result<(u64, &[u8])> {
    let h = m.get(..HEADER).ok_or(Error::Corrupted)?;
    let status = i32::from_le_bytes(h[0..4].try_into().unwrap());
    if status != 0 {
        return Err(Error::from_code(status as i64).unwrap_or(Error::Corrupted));
    }
    Ok((
        u64::from_le_bytes(h[8..16].try_into().unwrap()),
        &m[HEADER..],
    ))
}

/// Klient blokoveho zarizeni.
pub struct BlockClient {
    channel: Channel,
    sectors: u64,
    /// Odpoved neprisla vcas: pozdni odpoved by se mohla priradit dalsimu
    /// pozadavku (zpravy nemaji cislo), proto uz zadny dalsi pozadavek.
    failed: core::sync::atomic::AtomicBool,
    /// Zapisy odeslane bez cekani na odpoved (`write_queued`).
    queued: core::sync::atomic::AtomicU32,
    /// Prvni chyba odpovedi na odeslany zapis (kod, 0 = zadna).
    queued_error: core::sync::atomic::AtomicI64,
}

impl BlockClient {
    /// Handle kanalu (cekani na zavreni: medium vytazeno).
    pub fn handle(&self) -> &libos::Handle {
        self.channel.handle()
    }

    pub fn new(channel: Channel) -> Result<Self> {
        let mut c = Self {
            channel,
            sectors: 0,
            failed: core::sync::atomic::AtomicBool::new(false),
            queued: core::sync::atomic::AtomicU32::new(0),
            queued_error: core::sync::atomic::AtomicI64::new(0),
        };
        c.sectors = c
            .call(
                Request {
                    op: op::INFO,
                    count: 0,
                    lba: 0,
                },
                &[],
            )?
            .0;
        Ok(c)
    }

    /// Jako [`Self::new`], ale nejdriv zahodi pozdni odpovedi predchozi
    /// instance klienta na tomtez kanalu (konec drzel supervisor, viz
    /// `op::SYNC`). `nonce` = libovolne cislo, ktere stara instance
    /// nepouzila (napr. cas).
    pub fn new_synced(channel: Channel, nonce: u64) -> Result<Self> {
        let req = Request {
            op: op::SYNC,
            count: 0,
            lba: nonce,
        };
        channel.write(&req.encode(&[]), Vec::new())?;
        let deadline = libos::time::deadline_after(REPLY_TIMEOUT);
        loop {
            let m = channel.read(Some(deadline))?;
            // Odpoved na SYNC: stav 0 a ozvena; cokoli jineho je pozdni
            // odpoved stare instance.
            if let Ok((value, _)) = decode_reply(&m.data)
                && value == nonce
                && m.data.len() == HEADER
            {
                break;
            }
        }
        Self::new(channel)
    }

    /// Velikost zarizeni v sektorech po 512 B.
    pub fn sectors(&self) -> u64 {
        self.sectors
    }

    /// Precte jednu odpoved (s limitem; vyprseni = klient uz nic neposle).
    fn reply(&self) -> Result<libos::Message> {
        use core::sync::atomic::Ordering;
        // Ovladac, ktery neodpovi, nesmi zaseknout klienta navzdy.
        let deadline = libos::time::deadline_after(REPLY_TIMEOUT);
        self.channel.read(Some(deadline)).inspect_err(|_| self.failed.store(true, Ordering::Release))
    }

    /// Vybere odpovedi na odeslane zapisy; vrati prvni chybu (jednou).
    pub fn drain(&self) -> Result<()> {
        use core::sync::atomic::Ordering;
        while self.queued.load(Ordering::Acquire) > 0 {
            let m = self.reply()?;
            self.queued.fetch_sub(1, Ordering::AcqRel);
            if let Err(e) = decode_reply(&m.data) {
                let _ = self.queued_error.compare_exchange(0, e.code() as i64, Ordering::AcqRel, Ordering::Acquire);
            }
        }
        match self.queued_error.swap(0, Ordering::AcqRel) {
            0 => Ok(()),
            c => Err(Error::from_code(c).unwrap_or(Error::IoError)),
        }
    }

    fn call(&self, req: Request, data: &[u8]) -> Result<(u64, Vec<u8>)> {
        use core::sync::atomic::Ordering;
        if self.failed.load(Ordering::Acquire) {
            return Err(Error::IoError);
        }
        // Odpovedi chodi v poradi: nejdriv ty na odeslane zapisy.
        self.drain()?;
        self.channel.write(&req.encode(data), Vec::new())?;
        let reply = self.reply()?;
        let (value, payload) = decode_reply(&reply.data)?;
        Ok((value, payload.to_vec()))
    }

    /// Precte sektory od `lba` do `buf` (delka = nasobek 512). Delsi cteni
    /// posle az [`READ_AHEAD`] pozadavku bez cekani (odpovedi chodi
    /// v poradi): klient neceka na kazdou zpravu zvlast a ovladac muze
    /// navazujici cteni spojit (`server::Disks::max_read`).
    pub fn read(&self, lba: u64, buf: &mut [u8]) -> Result<()> {
        use core::sync::atomic::Ordering;
        if !buf.len().is_multiple_of(SECTOR) {
            return Err(Error::InvalidArgs);
        }
        if self.failed.load(Ordering::Acquire) {
            return Err(Error::IoError);
        }
        self.drain()?;
        const CHUNK: usize = MAX_SECTORS as usize * SECTOR;
        let n = buf.len().div_ceil(CHUNK);
        let (mut sent, mut got) = (0usize, 0usize);
        let mut first_err = None;
        while got < n {
            while sent < n && sent - got < READ_AHEAD && first_err.is_none() {
                let count = ((buf.len() - sent * CHUNK).min(CHUNK) / SECTOR) as u32;
                let req = Request { op: op::READ, count, lba: lba + (sent * MAX_SECTORS as usize) as u64 };
                match self.channel.write(&req.encode(&[]), Vec::new()) {
                    Ok(()) => sent += 1,
                    Err(e) => first_err = Some(e),
                }
            }
            if got == sent {
                break;
            }
            // Odpovedi na vsechna odeslana cteni vybrat i po chybe (jinak
            // by je dostal dalsi pozadavek).
            let m = self.reply()?;
            let end = ((got + 1) * CHUNK).min(buf.len());
            let chunk = &mut buf[got * CHUNK..end];
            got += 1;
            match decode_reply(&m.data) {
                Ok((_, data)) if data.len() == chunk.len() => chunk.copy_from_slice(data),
                Ok(_) => _ = first_err.get_or_insert(Error::Corrupted),
                Err(e) => _ = first_err.get_or_insert(e),
            }
        }
        first_err.map_or(Ok(()), Err)
    }

    /// Zapise sektory od `lba` (delka = nasobek 512).
    pub fn write(&self, lba: u64, data: &[u8]) -> Result<()> {
        if !data.len().is_multiple_of(SECTOR) {
            return Err(Error::InvalidArgs);
        }
        for (i, chunk) in data.chunks(MAX_SECTORS as usize * SECTOR).enumerate() {
            let count = (chunk.len() / SECTOR) as u32;
            let lba = lba + (i * MAX_SECTORS as usize) as u64;
            self.call(
                Request {
                    op: op::WRITE,
                    count,
                    lba,
                },
                chunk,
            )?;
        }
        Ok(())
    }

    /// Zapise sektory od `lba` bez cekani na odpoved ovladace (jako
    /// `write`, ale odpovedi se vyberou az pred dalsim pozadavkem jineho
    /// druhu nebo `drain`). Chyba zapisu se ohlasi tam. Rada zapisu tak
    /// stoji jednu cestu tam a zpet misto jedne za kazdou zpravu.
    pub fn write_queued(&self, lba: u64, data: &[u8]) -> Result<()> {
        use core::sync::atomic::Ordering;
        if !data.len().is_multiple_of(SECTOR) {
            return Err(Error::InvalidArgs);
        }
        if self.failed.load(Ordering::Acquire) {
            return Err(Error::IoError);
        }
        for (i, chunk) in data.chunks(MAX_SECTORS as usize * SECTOR).enumerate() {
            if self.queued.load(Ordering::Acquire) >= MAX_QUEUED_WRITES {
                self.drain()?;
            }
            let req = Request { op: op::WRITE, count: (chunk.len() / SECTOR) as u32, lba: lba + (i * MAX_SECTORS as usize) as u64 };
            self.channel.write(&req.encode(chunk), Vec::new())?;
            self.queued.fetch_add(1, Ordering::AcqRel);
        }
        Ok(())
    }

    /// Oznaci sektory jako nepouzite (TRIM). `NOT_SUPPORTED`, kdyz to
    /// zarizeni neumi.
    pub fn discard(&self, lba: u64, sectors: u64) -> Result<()> {
        let mut done = 0;
        while done < sectors {
            let count = (sectors - done).min(u32::MAX as u64) as u32;
            self.call(
                Request {
                    op: op::DISCARD,
                    count,
                    lba: lba + done,
                },
                &[],
            )?;
            done += count as u64;
        }
        Ok(())
    }

    /// Test ochrany DMA: `true` = zarizeni nemohlo zapsat mimo pridelenou
    /// pamet (IOMMU), `false` = mohlo.
    pub fn test_dma_protection(&self) -> Result<bool> {
        let req = Request {
            op: op::TEST_DMA_PROTECTION,
            count: 0,
            lba: 0,
        };
        self.call(req, &[]).map(|(v, _)| v == 1)
    }

    /// Vlastnosti zarizeni (bity `feature`); starsi ovladac = 0.
    pub fn features(&self) -> Result<u64> {
        let req = Request {
            op: op::FEATURES,
            count: 0,
            lba: 0,
        };
        match self.call(req, &[]) {
            Ok((v, _)) => Ok(v),
            Err(Error::NotSupported) => Ok(0),
            Err(e) => Err(e),
        }
    }

    /// Kanal k ovladaci (predani klienta jinemu procesu). Odpovedi na
    /// odeslane zapisy se nejdriv vyberou (jinak by je dostal novy klient).
    pub fn into_channel(self) -> Channel {
        let _ = self.drain();
        self.channel
    }

    pub fn flush(&self) -> Result<()> {
        self.call(
            Request {
                op: op::FLUSH,
                count: 0,
                lba: 0,
            },
            &[],
        )
        .map(|_| ())
    }
}

/// Buffer pro odpoved na cteni (pomocna funkce pro ovladace).
pub fn zeroed_sectors(count: u32) -> Vec<u8> {
    vec![0; count as usize * SECTOR]
}
