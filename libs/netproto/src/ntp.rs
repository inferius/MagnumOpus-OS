//! Format NTP v4 (RFC 5905), jen rezim klient - server.
//!
//! Klient posle pozadavek s vlastnim casem odeslani `t1`, server vrati cas
//! prijeti `t2` a odeslani `t3`, klient zaznamena prijeti odpovedi `t4`.
//! Posun hodin = `((t2 - t1) + (t3 - t4)) / 2`, zpozdeni cesty
//! `(t4 - t1) - (t3 - t2)`.
//!
//! Casova razitka NTP: sekundy od 1900-01-01 (u32) + zlomek sekundy (u32).
//! Sekundy pretecou v roce 2036 (era 1); hodnoty pod 2^31 se berou jako
//! era 1, takze prevod funguje pro roky 1968 az 2104.

use abi::time::WallTime;
use abi::{Error, Result};

/// Port serveru NTP.
pub const PORT: u16 = 123;
/// Delka zpravy bez rozsireni.
pub const LEN: usize = 48;
/// Sekundy mezi 1900-01-01 a 1970-01-01.
const UNIX_OFFSET: i64 = 2_208_988_800;

const MODE_CLIENT: u8 = 3;
const MODE_SERVER: u8 = 4;
const VERSION: u8 = 4;

/// Cas na 64bitove razitko NTP.
pub fn to_ntp(t: WallTime) -> u64 {
    let secs = (t.secs + UNIX_OFFSET) as u64 & 0xFFFF_FFFF;
    let frac = ((t.nanos as u64) << 32) / 1_000_000_000;
    secs << 32 | frac
}

/// Razitko NTP na cas (era podle rozsahu, viz modul).
pub fn from_ntp(ts: u64) -> WallTime {
    let mut secs = (ts >> 32) as i64;
    if secs < 0x8000_0000 {
        secs += 1 << 32;
    }
    let nanos = (((ts & 0xFFFF_FFFF) * 1_000_000_000) >> 32) as u32;
    WallTime {
        secs: secs - UNIX_OFFSET,
        nanos,
    }
}

/// Pozadavek klienta s casem odeslani `t1` (server ho vrati v poli
/// "originate" a klient podle nej pozna svou odpoved).
pub fn build_request(t1: WallTime) -> [u8; LEN] {
    let mut p = [0u8; LEN];
    p[0] = VERSION << 3 | MODE_CLIENT;
    p[40..48].copy_from_slice(&to_ntp(t1).to_be_bytes());
    p
}

/// Odpoved serveru.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reply {
    pub stratum: u8,
    /// Cas prijeti pozadavku serverem (t2).
    pub receive: WallTime,
    /// Cas odeslani odpovedi (t3).
    pub transmit: WallTime,
}

fn ts(d: &[u8], at: usize) -> u64 {
    u64::from_be_bytes(d[at..at + 8].try_into().unwrap())
}

/// Rozebere odpoved na pozadavek odeslany v `t1`. Cizi zprava (jiny rezim,
/// nesedi "originate") = `INVALID_ARGS`; server bez synchronizace (leap 3,
/// stratum 0 = "kiss of death" nebo > 15) = `BAD_STATE`.
pub fn parse_reply(t1: WallTime, d: &[u8]) -> Result<Reply> {
    if d.len() < LEN || d[0] & 7 != MODE_SERVER || ts(d, 24) != to_ntp(t1) {
        return Err(Error::InvalidArgs);
    }
    let leap = d[0] >> 6;
    let stratum = d[1];
    if leap == 3 || stratum == 0 || stratum > 15 {
        return Err(Error::BadState);
    }
    Ok(Reply {
        stratum,
        receive: from_ntp(ts(d, 32)),
        transmit: from_ntp(ts(d, 40)),
    })
}

/// Odpoved serveru (pro testovaci server): `receive` = kdy pozadavek prisel,
/// `transmit` = kdy se odpoved posila.
pub fn build_reply(request: &[u8], receive: WallTime, transmit: WallTime) -> Result<[u8; LEN]> {
    if request.len() < LEN || request[0] & 7 != MODE_CLIENT {
        return Err(Error::InvalidArgs);
    }
    let mut p = [0u8; LEN];
    p[0] = VERSION << 3 | MODE_SERVER;
    p[1] = 2; // stratum
    p[2] = request[2]; // poll
    p[3] = 0xEC; // precision ~ 2^-20 s
    p[12..16].copy_from_slice(b"LOCL"); // reference ID
    p[16..24].copy_from_slice(&to_ntp(receive).to_be_bytes()); // reference
    p[24..32].copy_from_slice(&request[40..48]); // originate = t1 klienta
    p[32..40].copy_from_slice(&to_ntp(receive).to_be_bytes());
    p[40..48].copy_from_slice(&to_ntp(transmit).to_be_bytes());
    Ok(p)
}

fn ns(t: WallTime) -> i128 {
    t.secs as i128 * 1_000_000_000 + t.nanos as i128
}

/// Posun mistnich hodin proti serveru v ns (kladny = mistni hodiny jdou
/// pozadu) a zpozdeni cesty v ns.
pub fn offset_delay(t1: WallTime, reply: &Reply, t4: WallTime) -> (i128, i128) {
    let (t1, t2, t3, t4) = (ns(t1), ns(reply.receive), ns(reply.transmit), ns(t4));
    (((t2 - t1) + (t3 - t4)) / 2, (t4 - t1) - (t3 - t2))
}

/// Opraveny cas v okamziku `t4`: `t4 + posun`.
pub fn corrected(t4: WallTime, offset_ns: i128) -> WallTime {
    let total = ns(t4) + offset_ns;
    WallTime {
        secs: total.div_euclid(1_000_000_000) as i64,
        nanos: total.rem_euclid(1_000_000_000) as u32,
    }
}

/// Vzorek mereni: posun a zpozdeni (ns).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    pub offset_ns: i128,
    pub delay_ns: i128,
}

/// Nejvetsi rozdil mezi uplynulym realnym a monotonnim casem behem jednoho
/// mereni (plynula korekce `timed` je radove ppm, tohle je skok hodin).
pub const STEP_TOLERANCE_NS: i128 = 100_000_000;
/// Nejdelsi verohodne zpozdeni cesty (limit dotazu je par sekund).
pub const MAX_DELAY_NS: i128 = 16_000_000_000;

/// Posunul nekdo realne hodiny mezi odeslanim (`t1`) a prijetim (`t4`)?
/// `mono_elapsed_ns` = cas mezi nimi podle monotonnich hodin. Takove
/// mereni nejde pouzit: `t1` a `t4` jsou z ruznych hodin (napr. test nebo
/// uzivatel nastavil cas o den a vratil ho: posun -12 h, zpozdeni -24 h).
pub fn clock_stepped(t1: WallTime, t4: WallTime, mono_elapsed_ns: u64) -> bool {
    (ns(t4) - ns(t1) - mono_elapsed_ns as i128).abs() > STEP_TOLERANCE_NS
}

/// Je vzorek verohodny? Zaporne zpozdeni nebo delsi nez [`MAX_DELAY_NS`]
/// = chyba v casovych znackach (serveru nebo vlastnich), vzorek zahodit.
pub fn plausible(s: &Sample) -> bool {
    (0..=MAX_DELAY_NS).contains(&s.delay_ns)
}

/// Z davky mereni jednoho serveru vybere verohodny vzorek s nejmensim
/// zpozdenim: u nej se nejmene projevi nesymetricka cesta a fronty (filtr
/// hodin NTP). Neverohodne vzorky se ignoruji - jinak by zaporne zpozdeni
/// vzdy vyhralo.
pub fn best_sample(samples: &[Sample]) -> Option<Sample> {
    samples.iter().copied().filter(plausible).min_by_key(|s| s.delay_ns)
}

/// Spoji posuny vic serveru: median (jeden spatny server vysledek nestahne).
pub fn combine(offsets: &mut [i128]) -> Option<i128> {
    if offsets.is_empty() {
        return None;
    }
    offsets.sort_unstable();
    let n = offsets.len();
    Some(if n % 2 == 1 {
        offsets[n / 2]
    } else {
        (offsets[n / 2 - 1] + offsets[n / 2]) / 2
    })
}

/// Rychlost dorovnavani jadra (500 ppm), aby slo odhadnout, kolik
/// z predchozi korekce uz probehlo.
pub const SLEW_PPB: i128 = 500_000;

/// Rizeni hodin klientem (zjednodusena smycka FLL): z posunu mezi
/// mericimi body odhadne, o kolik krystal trvale predbiha, a navrhne
/// opravu frekvence. Posun samotny dorovnava jadro (timed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discipline {
    /// Posledni korekce: (monotonni cas ns, posun ns).
    last: Option<(u64, i128)>,
    /// Navrzena celkova oprava frekvence (ppb).
    pub freq_ppb: i64,
    /// Interval dotazu (s).
    pub poll_secs: u64,
    pub min_poll: u64,
    pub max_poll: u64,
}

/// Nejkratsi interval, ze ktereho se odhaduje frekvence (kratsi = sum).
pub const FREQ_MIN_INTERVAL_NS: u64 = 16_000_000_000;
/// Posun, pod kterym se interval dotazu prodluzuje (hodiny jsou srovnane).
pub const STABLE_OFFSET_NS: i128 = 2_000_000;

impl Discipline {
    pub fn new(min_poll: u64, max_poll: u64) -> Self {
        Self {
            last: None,
            freq_ppb: 0,
            poll_secs: min_poll,
            min_poll,
            max_poll,
        }
    }

    /// Nove mereni posunu `offset` v okamziku `now` (ns). Vraci navrh
    /// frekvence (`None` = jeste neni z ceho odhadnout).
    pub fn update(&mut self, now: u64, offset: i128) -> Option<i64> {
        let freq = match self.last {
            Some((t, prev)) if now >= t + FREQ_MIN_INTERVAL_NS => {
                let dt = (now - t) as i128;
                // Cast predchozi korekce, kterou jadro jeste nestihlo dorovnat.
                let done = (dt * SLEW_PPB / 1_000_000_000).min(prev.abs());
                let residual = prev - prev.signum() * done;
                let drift = offset - residual;
                let err_ppb = drift * 1_000_000_000 / dt;
                // Polovinu chyby (tlumeni sumu), meze jako v jadru.
                let next = (self.freq_ppb as i128 + err_ppb / 2).clamp(-500_000, 500_000);
                self.freq_ppb = next as i64;
                Some(self.freq_ppb)
            }
            _ => None,
        };
        self.last = Some((now, offset));
        self.poll_secs = if offset.abs() < STABLE_OFFSET_NS {
            (self.poll_secs * 2).min(self.max_poll)
        } else {
            self.min_poll
        };
        freq
    }
}
