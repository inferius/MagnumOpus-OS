//! Blok parametru procesu: argumenty, promenne prostredi a pocatecni
//! pracovni adresar.
//!
//! Rodic ho preda objektem pameti jen pro cteni ve startovni zprave
//! (`bootstrap::kind::PARAMS`). Objekt pameti misto dat zpravy: velikost
//! neomezuje limit zpravy kanalu. Format (little-endian):
//!
//! ```text
//! 0   magic   u32  "OSPA"
//! 4   version u32  3
//! 8   size    u32  velikost celeho bloku (objekt pameti muze byt vetsi)
//! 12  flags   u32  [`flags`]
//! 16  argc    u32
//! 20  envc    u32
//! 24  fdc     u32  pocet zdedenych fd
//! 28  retezce: cwd, argv[0..argc], env[0..envc]; kazdy = delka u32 + bajty
//!     fd[0..fdc]: cislo u32, druh u8, stav u32, data = delka u32 + bajty
//! ```
//!
//! Verze 2 pridala `flags`, verze 3 zdedene fd ([`Fd`], `posix_spawn`);
//! starsi verze se uz nectou.
//!
//! Retezce jsou bajty (v systemu UTF-8, ale format to nevynucuje); polozka
//! prostredi je `JMENO=HODNOTA`. Standardni handly (stdin, stdout, relace
//! FS...) jsou ve stejne startovni zprave jako `kind::USER` se stitky
//! z [`tag`].

/// "OSPA" v ASCII.
pub const MAGIC: u32 = u32::from_le_bytes(*b"OSPA");
pub const VERSION: u32 = 3;
pub const HEADER_SIZE: usize = 28;
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
    /// Relace zvukove sluzby s pravy aplikace (`audioproto::session`).
    pub const AUDIO: u32 = 0x7FFF_0006;
    /// Listek k obnove relace `AUDIO` po restartu `audiod`.
    pub const AUDIO_LEASE: u32 = 0x7FFF_0007;
    /// Ridici kanal terminalu (`ttyproto`: rezim termios, velikost okna);
    /// jen proces spusteny v terminalu.
    pub const TTY: u32 = 0x7FFF_0008;
    /// Handly zdedenych fd ([`super::Fd`]): ve stejnem poradi jako polozky
    /// bloku, ktere handle maji ([`super::fd_kind::has_handle`]).
    pub const FD: u32 = 0x7FFF_0009;
}

/// Nejvic zdedenych fd v jednom bloku (handly jdou ve startovni zprave
/// s limitem `CHANNEL_MAX_HANDLES` spolu se stdio, relacemi a blokem).
pub const MAX_FDS: usize = 48;

/// Druhy zdedenych fd (`Fd::kind`) a jejich data. Data zapisuje a cte
/// runtime (`osrt::inherit`), format je soucasti ABI.
pub mod fd_kind {
    /// Kanal proudu bajtu (konec roury, stdio rodice pod jinym cislem);
    /// handle `tag::FD`, data prazdna.
    pub const PIPE: u8 = 1;
    /// Pametovy fd (`memfd_create`): handle objektu pameti `tag::FD`;
    /// data `zapisovatelny u8, pecete u32, pozice u64`.
    pub const MEMORY: u8 = 2;
    /// Soubor nebo adresar relace FS, bez handlu: potomek ho otevre ve sve
    /// relaci operaci `REOPEN` (fsproto v2). Data `dataset u64, oid u64,
    /// generace u64, pristup u8, sdileni u8, pozice u64, pripojovat u8,
    /// cesta (zbytek)`.
    pub const FILE: u8 = 3;
    /// Socket `AF_UNIX`: handle kanalu spojeni nebo posluchace `tag::FD`
    /// a za nim handly zprav, ktere uz byly vyctene z fronty (pocet v datech);
    /// data `stav u8` (1 spojeny, 2 naslouchajici, 3 jen jmeno), u spojeneho
    /// `uzivatel u64, skupina u64, pid u32, jmeno druhe strany, EOF u8,
    /// rozectena data (delka u32 + bajty), rozectene fd (pocet u8 + polozky
    /// jako `SCM_RIGHTS`)`, pak vlastni jmeno a fronta (pocet u32 + zprava:
    /// delka u32 + bajty + pocet handlu u8); jmeno = `delka u32 + bajty`
    /// (0 = bez jmena). Handly rozectenych fd jdou pred handly fronty.
    pub const UNIX: u8 = 4;
    /// Prazdny fd jako `/dev/null` (cteni = konec, zapis se zahodi), bez
    /// handlu a dat: stdio rodice bez kanalu (`Stdio::null`) a `/dev/null`.
    pub const NULL: u8 = 5;
    /// Procesovy fd (`mo_self_process_fd`, Windows vrstva): handle procesu
    /// jen s pravy pameti (`READ_MEMORY`, `WRITE_MEMORY`) `tag::FD`, data
    /// prazdna.
    pub const PROCESS: u8 = 6;

    /// Ma polozka handle ve startovni zprave?
    pub const fn has_handle(kind: u8) -> bool {
        matches!(kind, PIPE | MEMORY | UNIX | PROCESS)
    }
}

/// Zdedeny fd: cislo v potomkovi, druh ([`fd_kind`]), stav popisu
/// otevreni (`fcntl(F_GETFL)`, hodnoty POSIX) a data druhu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fd<'a> {
    pub fd: u32,
    pub kind: u8,
    pub status: u32,
    pub data: &'a [u8],
}

impl Fd<'_> {
    fn encoded_len(&self) -> usize {
        4 + 1 + 4 + 4 + self.data.len()
    }
}

/// Precteny blok (odkazuje do bufferu).
#[derive(Debug, Clone, Copy)]
pub struct Params<'a> {
    pub cwd: &'a [u8],
    /// Priznaky ([`flags`]); nezname bity se ignoruji.
    pub flags: u32,
    argc: u32,
    envc: u32,
    fdc: u32,
    /// Retezce za `cwd` (argumenty, prostredi, pak fd).
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

/// Jedna polozka fd z `data`: (polozka, zbytek).
fn take_fd(data: &[u8]) -> Option<(Fd<'_>, &[u8])> {
    let fd = u32_at(data, 0)?;
    let kind = *data.get(4)?;
    let status = u32_at(data, 5)?;
    let (d, rest) = take(data.get(9..)?)?;
    Some((Fd { fd, kind, status, data: d }, rest))
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
        let fdc = u32_at(data, 24)?;
        if fdc as usize > MAX_FDS {
            return None;
        }
        let (cwd, rest) = take(data.get(HEADER_SIZE..)?)?;
        let mut check = rest;
        for _ in 0..u64::from(argc) + u64::from(envc) {
            check = take(check)?.1;
        }
        for _ in 0..fdc {
            check = take_fd(check)?.1;
        }
        Some(Self { cwd, flags, argc, envc, fdc, rest })
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

    /// Zdedene fd (`posix_spawn`).
    pub fn fds(&self) -> Fds<'a> {
        let mut data = self.rest;
        for _ in 0..u64::from(self.argc) + u64::from(self.envc) {
            // Platnost overil `parse`.
            data = take(data).map_or(&[][..], |(_, r)| r);
        }
        Fds { data, left: self.fdc }
    }
}

/// Iterator zdedenych fd bloku.
#[derive(Debug, Clone)]
pub struct Fds<'a> {
    data: &'a [u8],
    left: u32,
}

impl<'a> Iterator for Fds<'a> {
    type Item = Fd<'a>;
    fn next(&mut self) -> Option<Fd<'a>> {
        if self.left == 0 {
            return None;
        }
        let (f, rest) = take_fd(self.data)?;
        self.data = rest;
        self.left -= 1;
        Some(f)
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

/// Velikost bloku pro dane retezce a fd.
pub fn encoded_len<'s>(
    cwd: &[u8],
    args: impl IntoIterator<Item = &'s [u8]>,
    env: impl IntoIterator<Item = &'s [u8]>,
    fds: &[Fd<'_>],
) -> usize {
    HEADER_SIZE
        + 4
        + cwd.len()
        + args.into_iter().chain(env).map(|s| 4 + s.len()).sum::<usize>()
        + fds.iter().map(Fd::encoded_len).sum::<usize>()
}

/// Zapise blok do `out` (velikost presne [`encoded_len`]); `None` =
/// spatna velikost nebo prilis mnoho polozek (fd nejvic [`MAX_FDS`]).
pub fn encode<'s>(
    out: &mut [u8],
    flags: u32,
    cwd: &'s [u8],
    args: impl IntoIterator<Item = &'s [u8]> + Clone,
    env: impl IntoIterator<Item = &'s [u8]> + Clone,
    fds: &[Fd<'_>],
) -> Option<()> {
    if out.len() != encoded_len(cwd, args.clone(), env.clone(), fds) || out.len() > MAX_SIZE || fds.len() > MAX_FDS {
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
    out[24..28].copy_from_slice(&(fds.len() as u32).to_le_bytes());
    let mut at = HEADER_SIZE;
    let put = |out: &mut [u8], at: &mut usize, s: &[u8]| -> Option<()> {
        out[*at..*at + 4].copy_from_slice(&u32::try_from(s.len()).ok()?.to_le_bytes());
        out[*at + 4..*at + 4 + s.len()].copy_from_slice(s);
        *at += 4 + s.len();
        Some(())
    };
    for s in core::iter::once(cwd).chain(args).chain(env) {
        put(out, &mut at, s)?;
    }
    for f in fds {
        out[at..at + 4].copy_from_slice(&f.fd.to_le_bytes());
        out[at + 4] = f.kind;
        out[at + 5..at + 9].copy_from_slice(&f.status.to_le_bytes());
        at += 9;
        put(out, &mut at, f.data)?;
    }
    Some(())
}
