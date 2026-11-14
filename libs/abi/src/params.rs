//! Blok parametru procesu: argumenty, promenne prostredi a pocatecni
//! pracovni adresar.
//!
//! Rodic ho preda objektem pameti jen pro cteni ve startovni zprave
//! (`bootstrap::kind::PARAMS`). Objekt pameti misto dat zpravy: velikost
//! neomezuje limit zpravy kanalu. Format (little-endian):
//!
//! ```text
//! 0   magic   u32  "OSPA"
//! 4   version u32  2
//! 8   size    u32  velikost celeho bloku (objekt pameti muze byt vetsi)
//! 12  flags   u32  [`flags`]
//! 16  argc    u32
//! 20  envc    u32
//! 24  retezce: cwd, argv[0..argc], env[0..envc]; kazdy = delka u32 + bajty
//! ```
//!
//! Verze 2 pridala `flags` (verze 1 se uz necte).
//!
//! Retezce jsou bajty (v systemu UTF-8, ale format to nevynucuje); polozka
//! prostredi je `JMENO=HODNOTA`. Standardni handly (stdin, stdout, relace
//! FS...) jsou ve stejne startovni zprave jako `kind::USER` se stitky
//! z [`tag`].

/// "OSPA" v ASCII.
pub const MAGIC: u32 = u32::from_le_bytes(*b"OSPA");
pub const VERSION: u32 = 2;
pub const HEADER_SIZE: usize = 24;
/// Nejvetsi blok, ktery ctenar prijme (ochrana pred nesmyslnou velikosti).
pub const MAX_SIZE: usize = 16 * 1024 * 1024;

/// Priznaky bloku (`flags`).
pub mod flags {
    /// Standardni vystup zahodit (`Stdio::null`): bez kanalu `STDOUT` by
    /// vystup sel do logu jadra.
    pub const STDOUT_NULL: u32 = 1 << 0;
    /// Chybovy vystup zahodit.
    pub const STDERR_NULL: u32 = 1 << 1;
    pub const ALL: u32 = STDOUT_NULL | STDERR_NULL;
}

/// Stitky standardnich handlu ve startovni zprave (`kind::USER`, `arg`).
/// Rozsah `0x7FFF_0000..` je vyhrazeny pro system.
pub mod tag {
    /// Kanal proudu bajtu: vstup (cte proces).
    pub const STDIN: u32 = 0x7FFF_0000;
    /// Kanal proudu bajtu: vystup (zapisuje proces).
    pub const STDOUT: u32 = 0x7FFF_0001;
    /// Kanal proudu bajtu: chybovy vystup.
    pub const STDERR: u32 = 0x7FFF_0002;
    /// Relace souboroveho systemu (fsproto).
    pub const FS: u32 = 0x7FFF_0003;
    /// Relace site (netproto).
    pub const NET: u32 = 0x7FFF_0004;
    /// Kanal ke spousteci sluzbe launchd (`launchproto`).
    pub const LAUNCH: u32 = 0x7FFF_0005;
}

/// Precteny blok (odkazuje do bufferu).
#[derive(Debug, Clone, Copy)]
pub struct Params<'a> {
    pub cwd: &'a [u8],
    /// Priznaky ([`flags`]); nezname bity se ignoruji.
    pub flags: u32,
    argc: u32,
    envc: u32,
    /// Retezce za `cwd` (argumenty, pak prostredi).
    rest: &'a [u8],
}

fn u32_at(data: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(off..off + 4)?.try_into().ok()?))
}

/// Jeden retezec z `data`: (retezec, zbytek).
fn take(data: &[u8]) -> Option<(&[u8], &[u8])> {
    let len = u32_at(data, 0)? as usize;
    let s = data.get(4..4usize.checked_add(len)?)?;
    Some((s, &data[4 + len..]))
}

impl<'a> Params<'a> {
    /// Velikost bloku z hlavicky (pro cteni z objektu pameti: nejdriv
    /// hlavicka, pak zbytek). `None` = neplatna hlavicka.
    pub fn size(header: &[u8]) -> Option<usize> {
        if u32_at(header, 0)? != MAGIC || u32_at(header, 4)? != VERSION {
            return None;
        }
        let size = u32_at(header, 8)? as usize;
        (HEADER_SIZE..=MAX_SIZE).contains(&size).then_some(size)
    }

    /// Precte a zkontroluje blok (vsechny retezce musi byt cele uvnitr
    /// velikosti z hlavicky; data za ni se ignoruji).
    pub fn parse(data: &'a [u8]) -> Option<Self> {
        let data = data.get(..Self::size(data)?)?;
        let flags = u32_at(data, 12)?;
        let argc = u32_at(data, 16)?;
        let envc = u32_at(data, 20)?;
        let (cwd, rest) = take(data.get(HEADER_SIZE..)?)?;
        let mut check = rest;
        for _ in 0..u64::from(argc) + u64::from(envc) {
            check = take(check)?.1;
        }
        Some(Self { cwd, flags, argc, envc, rest })
    }

    /// Argumenty (prvni je obvykle jmeno programu).
    pub fn args(&self) -> Strings<'a> {
        Strings { data: self.rest, left: self.argc }
    }

    /// Polozky prostredi `JMENO=HODNOTA`.
    pub fn env(&self) -> Strings<'a> {
        let mut data = self.rest;
        for _ in 0..self.argc {
            // Platnost overil `parse`.
            data = take(data).map_or(&[][..], |(_, r)| r);
        }
        Strings { data, left: self.envc }
    }
}

/// Iterator retezcu bloku.
#[derive(Debug, Clone)]
pub struct Strings<'a> {
    data: &'a [u8],
    left: u32,
}

impl<'a> Iterator for Strings<'a> {
    type Item = &'a [u8];
    fn next(&mut self) -> Option<&'a [u8]> {
        if self.left == 0 {
            return None;
        }
        let (s, rest) = take(self.data)?;
        self.data = rest;
        self.left -= 1;
        Some(s)
    }
}

/// Velikost bloku pro dane retezce.
pub fn encoded_len<'s>(cwd: &[u8], args: impl IntoIterator<Item = &'s [u8]>, env: impl IntoIterator<Item = &'s [u8]>) -> usize {
    HEADER_SIZE + 4 + cwd.len() + args.into_iter().chain(env).map(|s| 4 + s.len()).sum::<usize>()
}

/// Zapise blok do `out` (velikost presne [`encoded_len`]); `None` =
/// spatna velikost nebo prilis mnoho polozek.
pub fn encode<'s>(
    out: &mut [u8],
    flags: u32,
    cwd: &'s [u8],
    args: impl IntoIterator<Item = &'s [u8]> + Clone,
    env: impl IntoIterator<Item = &'s [u8]> + Clone,
) -> Option<()> {
    if out.len() != encoded_len(cwd, args.clone(), env.clone()) || out.len() > MAX_SIZE {
        return None;
    }
    let argc = u32::try_from(args.clone().into_iter().count()).ok()?;
    let envc = u32::try_from(env.clone().into_iter().count()).ok()?;
    out[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    out[4..8].copy_from_slice(&VERSION.to_le_bytes());
    let size = u32::try_from(out.len()).ok()?;
    out[8..12].copy_from_slice(&size.to_le_bytes());
    out[12..16].copy_from_slice(&flags.to_le_bytes());
    out[16..20].copy_from_slice(&argc.to_le_bytes());
    out[20..24].copy_from_slice(&envc.to_le_bytes());
    let mut at = HEADER_SIZE;
    for s in core::iter::once(cwd).chain(args).chain(env) {
        out[at..at + 4].copy_from_slice(&u32::try_from(s.len()).ok()?.to_le_bytes());
        out[at + 4..at + 4 + s.len()].copy_from_slice(s);
        at += 4 + s.len();
    }
    Some(())
}
