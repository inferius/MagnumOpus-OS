# Real time

The system has two clocks:

| Clock | Call | For | Properties |
|---|---|---|---|
| monotonic | `CLOCK_MONOTONIC`, `libos::time::now()` | measuring, deadlines, timeouts | ns since kernel start, never goes back, cannot fail |
| real ("wall") | `CLOCK_REALTIME`, `libos::time::wall()` | file times, display, logs | seconds + ns since 1970 UTC, **can jump** (clock setting) |

A wait deadline is always computed from the monotonic clock. The real time
may meanwhile move by, say, an hour (a network correction), and a timeout
would then expire immediately or never.

## Where the kernel gets real time

<figure class="diagram">
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Real time sources">
<defs><marker id="tm1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="200" height="70" rx="6"/><text class="t b" x="110" y="45" text-anchor="middle">loader (UEFI)</text><text class="m" x="110" y="64" text-anchor="middle">GetTime + TSC right before</text><text class="m" x="110" y="80" text-anchor="middle">exiting boot services</text>
<path class="ln" d="M210 55 L278 55" marker-end="url(#tm1)"/>
<rect class="acc" x="280" y="20" width="220" height="70" rx="6"/><text class="t b" x="390" y="45" text-anchor="middle">BootInfo.firmware_time</text><text class="m" x="390" y="64" text-anchor="middle">seconds since 1970, ns, zone,</text><text class="m" x="390" y="80" text-anchor="middle">TSC at the moment of reading</text>
<path class="ln" d="M500 55 L568 90" marker-end="url(#tm1)"/>
<rect class="box" x="10" y="140" width="200" height="70" rx="6"/><text class="t b" x="110" y="165" text-anchor="middle">CMOS RTC (ports 70h/71h)</text><text class="m" x="110" y="184" text-anchor="middle">fallback when UEFI gave no time;</text><text class="m" x="110" y="200" text-anchor="middle">century from ACPI FADT</text>
<path class="ln" d="M210 175 L568 125" marker-end="url(#tm1)"/>
<rect class="ok" x="570" y="70" width="180" height="80" rx="6"/><text class="t b" x="660" y="95" text-anchor="middle">kernel: origin</text><text class="m" x="660" y="114" text-anchor="middle">real time at moment</text><text class="m" x="660" y="130" text-anchor="middle">0 of the monotonic clock</text>
<text class="m" x="10" y="238">CLOCK_REALTIME = origin + CLOCK_MONOTONIC. The gap between the loader's reading and the kernel clock start comes from the TSC.</text>
</svg>
<figcaption>Time source at boot: UEFI, else CMOS RTC, else the time is unknown.</figcaption>
</figure>

1. **UEFI `GetTime`.** The loader reads the time right before
   `ExitBootServices`, together with the TSC value. After calibrating the
   TSC, the kernel computes how much time passed until its clock started
   and stores the origin.
2. **Hardware clock** (`arch::hardware_clock`, on x86 the CMOS RTC; none
   on aarch64 yet). When the firmware gave no time (an error or an invalid
   date), the kernel reads the MC146818 clock itself:
   - It reads twice in a row until the readings match. During an update,
     two different seconds could otherwise mix.
   - It handles BCD and binary encoding and 12-hour and 24-hour mode.
   - The century comes from the register named in the ACPI FADT; otherwise
     20xx is assumed.
   - When the FADT says the RTC is absent, the clock is not read.
3. **Nothing.** `CLOCK_REALTIME` returns `NOT_SUPPORTED` and `fsd` uses the
   time since boot. Files then look as if they were from 1970.

The boot log shows the result:

```text
loader: cas firmwaru: 2026-11-07 14:03:12 (pasmo neuvedeno, beru jako UTC)
kernel: realny cas: 2026-11-07 14:03:13 UTC (zdroj UEFI, pasmo neuvedeno - bran jako UTC)
```

⚠️ **Time zone:** PC clocks do not know the zone, and UEFI almost always
reports it as "unspecified". The kernel then takes the time as **UTC** and
sets the `ZONE_ASSUMED_UTC` flag.

Linux usually keeps UTC in the RTC, **Windows keeps local time**. On a
machine with Windows real time is then off by the zone offset (1 to 2 hours
in Central Europe). The time service corrects it when its settings say
`rtc = local`, see
[RTC in local time](#system-settings-and-an-rtc-in-local-time-dual-boot-with-windows).

When the firmware does give a zone, UTC = local − zone. This is the UEFI
2.x convention; other versions of the specification state it with the
opposite sign.

⚠️ **Accuracy:** firmware and RTC have a 1 s resolution, so the origin is
accurate to a second. `init` can refine it from the network (NTP) through the time service, see
[network.md](network.md#ntp-and-the-clock). The clock
can already be set (`CLOCK_SET`, see below).

## Setting the clock (`CLOCK_SET`)

At run time the real time is set by the **time service** `timed`. Apart from
`init` it is the only process allowed to do so.

<figure class="diagram">
<svg viewBox="0 0 760 240" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Clock setting permission">
<defs><marker id="tm2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="85" width="170" height="70" rx="6"/><text class="t b" x="95" y="110" text-anchor="middle">kernel</text><text class="m" x="95" y="128" text-anchor="middle">root Resource:</text><text class="m" x="95" y="144" text-anchor="middle">MANAGE + SET_CLOCK</text>
<path class="ln" d="M180 120 L248 120" marker-end="url(#tm2)"/>
<rect class="box" x="250" y="85" width="150" height="70" rx="6"/><text class="t b" x="325" y="110" text-anchor="middle">init</text><text class="m" x="325" y="128" text-anchor="middle">holds the root,</text><text class="m" x="325" y="144" text-anchor="middle">splits the rights</text>
<path class="ln" d="M400 105 L498 55" marker-end="url(#tm2)"/>
<path class="ln" d="M400 135 L498 185" marker-end="url(#tm2)"/>
<rect class="ok" x="500" y="20" width="250" height="70" rx="6"/><text class="t b" x="625" y="45" text-anchor="middle">timed (time service)</text><text class="m" x="625" y="64" text-anchor="middle">Resource with SET_CLOCK only</text><text class="m" x="625" y="80" text-anchor="middle">no MANAGE = no hardware</text>
<rect class="box" x="500" y="150" width="250" height="70" rx="6"/><text class="t b" x="625" y="175" text-anchor="middle">devmgr → drivers</text><text class="m" x="625" y="194" text-anchor="middle">root without SET_CLOCK; restricted</text><text class="m" x="625" y="210" text-anchor="middle">resources never have it</text>
</svg>
<figcaption>Only the root Resource has SET_CLOCK; init hands it to the time service only.</figcaption>
</figure>

The **permission** is the `SET_CLOCK` right on a Resource handle:

- Only the **root** Resource, which the kernel gives to `init`, has it.
- `init` gives the time service a duplicate with rights
  `BASIC | SET_CLOCK`. Without `MANAGE` the service cannot touch hardware
  (register windows, DMA, interrupts).
- The device manager gets the root with `BASIC | MANAGE`, i.e. without
  `SET_CLOCK`. Duplication can only remove rights, so it cannot get it back.
- Restricted driver resources (`RESOURCE_CREATE`) never have `SET_CLOCK`.

Without the right `CLOCK_SET` returns `ACCESS_DENIED`. After setting, the
time has source `source::SET` and the `ZONE_ASSUMED_UTC` flag is cleared.

```rust
use libos::time::{self, zone::Settings, WallTime};

// In timed: clock = Resource with the SET_CLOCK right.
let t = WallTime { secs: 1_794_000_000, nanos: 0 };
let hw = time::set(&clock, t, &settings, true)?; // true = also write the RTC
```

**Writing the hardware clock** (`set_flags::WRITE_HARDWARE`) stores the time
in the CMOS RTC too, so it survives a reboot:

- The kernel knows no time zones. It writes `UTC + rtc_offset` to the RTC;
  the time service sends `rtc_offset`. For an RTC in UTC it is 0, for an RTC
  in local time it is the zone offset at that moment (Prague in summer
  +7200 s). The `time::set` wrapper computes it from the settings.
- The kernel takes the write format (BCD or binary, 12 or 24 h) from the
  clock's register B. During the write it holds the SET bit, so the clock
  does not update meanwhile.
- **Fraction of a second:** the RTC has a 1 s resolution. During the write
  the kernel therefore holds the clock divider (register A) in reset. After
  the reset is released the first second comes 0.5 s later, so the kernel
  writes exactly in the middle of a second
  (`arch::HARDWARE_CLOCK_PHASE_NS`). The clock then ticks on second
  boundaries. Without this it would be up to 1 s behind after every write.
  A syscall with `WRITE_HARDWARE` can therefore **sleep up to 1 s**.
- The system time **at the moment of the write** is written, not the time
  from the request. The write is under a lock, so even concurrent settings
  leave the newest system time in the RTC.
- Valid input: a time in the years 1970–9999 (`SetRequest::MIN_SECS` to
  `MAX_SECS`), nanoseconds below 10⁹ and `|rtc_offset|` at most 18 h
  (`abi::time::MAX_ZONE_OFFSET`). Otherwise `INVALID_ARGS`.
- Without a century register in the ACPI FADT the clock only covers
  2000–2099.
- aarch64 has no hardware clock yet (`set_hardware_clock` returns `false`).
  Only the system time is set and the syscall returns 0.

⚠️ **Risks of setting the clock:**

- Real time **jumps** forwards or backwards. Anyone measuring durations with
  it gets nonsense. Always compute deadlines and timeouts from
  `CLOCK_MONOTONIC`; setting the clock does not change it.
- Files written before the change keep times from the old clock.
- When the RTC holds local time and Windows runs on the machine too, only
  write local time to the RTC (`rtc = local`). Writing UTC would shift
  Windows by the zone offset.

## Smooth correction (`CLOCK_ADJUST`)

Stepping the clock backwards breaks ordering: a file saved later has an
older time, logs go out of order, TLS certificates may briefly look
invalid. At run time the clock is therefore only **adjusted smoothly**.

<figure class="diagram">
<svg viewBox="0 0 760 220" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Smooth time correction">
<defs><marker id="tm7" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<path class="ln" d="M60 190 L740 190" marker-end="url(#tm7)"/><text class="m" x="660" y="210">monotonic time</text>
<path class="ln" d="M60 190 L60 20" marker-end="url(#tm7)"/><text class="m" x="66" y="30">real time</text>
<path class="ln" d="M60 170 L720 40"/><text class="m" x="600" y="80">true time</text>
<path class="ln-acc" d="M60 140 L300 92 L300 123 L720 40"/>
<path class="ln-acc" d="M300 92 L560 72"/>
<text class="m" x="310" y="145">step (CLOCK_SET): time goes back</text>
<text class="m" x="250" y="60">smooth (CLOCK_ADJUST): only slower</text>
</svg>
<figcaption>A clock 30 ms ahead: a step moves time back, a smooth correction only slows it down for a while.</figcaption>
</figure>

The kernel computes real time from the point where it last changed. It
adds the elapsed monotonic time and two corrections:

- **frequency**: a permanent correction of the crystal rate (a computer
  clock is permanently fast or slow, typically under 100 ppm),
- **slew**: a temporary speed-up or slow-down (at most 500 ppm) until the
  given offset is made up. Then it ends by itself.

The rate of real time is thus always 1 ± 1000 ppm, never negative or
zero. Syscall: [`CLOCK_ADJUST`](syscalls.md#clock_adjust), wrapper
`libos::time::adjust(clock, offset, frequency)`.

```rust
use libos::time;
// In timed: the clock is 20 ms ahead, the crystal is 35 ppm fast.
let st = time::adjust(&clock, Some(-20_000_000), Some(-35_000))?;
// st.remaining_ns = -20 ms, made up within 40 s.
```

## Synchronization over the network (`ntpd`)

<figure class="diagram">
<svg viewBox="0 0 760 170" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="ntpd and timed">
<defs><marker id="tm8" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="warn" x="10" y="50" width="150" height="70" rx="6"/><text class="t b" x="85" y="78" text-anchor="middle">NTP servers</text><text class="m" x="85" y="96" text-anchor="middle">UDP 123</text>
<path class="ln" d="M160 85 L228 85" marker-end="url(#tm8)"/>
<rect class="box" x="230" y="40" width="200" height="90" rx="6"/><text class="t b" x="330" y="65" text-anchor="middle">ntpd</text><text class="m" x="330" y="84" text-anchor="middle">session: these servers only</text><text class="m" x="330" y="100" text-anchor="middle">filter, median, frequency</text><text class="m" x="330" y="116" text-anchor="middle">no right to the clock</text>
<path class="ln-acc" d="M430 85 L498 85" marker-end="url(#tm8)"/><text class="m" x="440" y="76">sync</text>
<rect class="ok" x="500" y="40" width="250" height="90" rx="6"/><text class="t b" x="625" y="65" text-anchor="middle">timed</text><text class="m" x="625" y="84" text-anchor="middle">verifies and limits</text><text class="m" x="625" y="100" text-anchor="middle">CLOCK_ADJUST (SET_CLOCK)</text><text class="m" x="625" y="116" text-anchor="middle">step only on first sync</text>
</svg>
<figcaption>A process handling data from the network must not change the clock directly.</figcaption>
</figure>

**`ntpd`** asks NTP servers and only proposes corrections. Init starts
it after the network service:

- **A network session** by the manifest `ntpd.manifest.toml` (rights
  `tcp`, `udp`, `dns`) plus only the targets of the servers from
  `ntpd.toml` (see [NTS](#nts-authenticated-time)). Nowhere else.
- **A channel to `timed`** only for `sync` messages (correction
  proposals).
- Every round: a batch of measurements from each server (`samples`), the
  sample with the **smallest delay** (least distorted by queues and an
  asymmetric path), and the **median** across servers (one bad server
  does not pull the result).
- **Frequency:** from the offset between rounds (minus the part of the
  previous correction the kernel has not made up yet) it estimates the
  crystal drift. It proposes half of the error so measurement noise does
  not make the loop oscillate (`netproto::ntp::Discipline`).
- **The interval** starts at `min_poll` and doubles up to `max_poll` while
  the clock is in sync (offset under 2 ms).

```toml
# ntpd.toml (initfs)
servers = ["nts://time.cloudflare.com", "nts://nts.netnod.se", "nts://ptbtime1.ptb.de", "pool.ntp.org"]
unauthenticated = "never"   # unauthenticated servers: never / fallback
min_poll = 64
max_poll = 1024
samples = 4
```

**`timed`** verifies and applies the proposal, because only it has the
right to the clock:

| Situation | What `timed` does |
|---|---|
| first synchronization, offset over 128 ms | step (`CLOCK_SET`): an RTC clock may be minutes off and slewing would take days |
| otherwise | smooth correction (`CLOCK_ADJUST`), never a step back |
| after the first synchronization, offset over 60 s | rejects it (`OUT_OF_RANGE`): a clock does not drift that much by itself, it is more likely spoofed |
| frequency | at most ±500 ppm and one change at most by 100 ppm |

`timed` does not keep "first synchronization" only in memory: afterwards
the kernel reports time source 4 (`source::NETWORK`, set by `CLOCK_SET`
with the `NETWORK` flag and by `CLOCK_ADJUST` with an offset). A new
`timed` restarted by the supervisor sees it and does not allow another
step.

> **Change from the first version:** the state lived only in `timed`'s
> memory, so after a restart the clock could be stepped again. **Why:** the
> "single step" rule must hold for the whole uptime.

Status: the `status` message on init's channel to `timed`
(`synced=1 updates=… last_offset_ns=… remaining_ns=… freq_ppb=…
slew_ppb=…`) and to `ntpd` (`rounds=… offset_ns=… delay_ns=… freq_ppb=…
poll_s=… error=… timed=… auth=nts|plain|- policy=… nts_error=…`).

### NTS (authenticated time)

<figure class="diagram">
<svg viewBox="0 0 760 230" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="NTS">
<defs><marker id="tm9" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="20" width="160" height="190" rx="6"/><text class="t b" x="90" y="45" text-anchor="middle">ntpd</text>
<text class="m" x="20" y="70">keys C2S, S2C</text><text class="m" x="20" y="88">from TLS export</text><text class="m" x="20" y="106">8 cookies</text>
<rect class="ok" x="590" y="20" width="160" height="80" rx="6"/><text class="t b" x="670" y="45" text-anchor="middle">NTS-KE</text><text class="m" x="670" y="64" text-anchor="middle">TLS 1.3, TCP 4460</text><text class="m" x="670" y="82" text-anchor="middle">ALPN ntske/1</text>
<rect class="ok" x="590" y="130" width="160" height="80" rx="6"/><text class="t b" x="670" y="155" text-anchor="middle">NTP server</text><text class="m" x="670" y="174" text-anchor="middle">UDP 123</text><text class="m" x="670" y="192" text-anchor="middle">AES-SIV-CMAC-256</text>
<path class="ln" d="M170 50 L588 50" marker-end="url(#tm9)"/><text class="m" x="190" y="44">ClientHello, NTPv4 + AES-SIV, END</text>
<path class="ln" d="M588 75 L172 75" marker-end="url(#tm9)"/><text class="m" x="190" y="69">certificate; 8 cookies, NTP port, END</text>
<path class="ln-acc" d="M170 150 L588 150" marker-end="url(#tm9)"/><text class="m" x="190" y="144">NTP + UID + cookie + placeholders + authenticator (C2S)</text>
<path class="ln-acc" d="M588 180 L172 180" marker-end="url(#tm9)"/><text class="m" x="190" y="174">NTP + UID + authenticator (S2C) with new cookies</text>
</svg>
<figcaption>Keys are agreed over TLS (authenticated server); every NTP query is authenticated and single-use.</figcaption>
</figure>

**NTS** (Network Time Security, RFC 8915) prevents time spoofing:

1. **NTS-KE:** TLS 1.3 to port 4460 with ALPN `ntske/1`. The server
   certificate is verified as for HTTPS (roots from the store, revocation
   through `revocd`). The client sends NTPv4 and AEAD 15
   (AES-SIV-CMAC-256), the server returns a set of 8 **cookies** and
   possibly another server or port for NTP. The C2S and S2C keys (32 B)
   come from the TLS exporter (`EXPORTER-network-time-security`).
2. **NTP with extensions:** a query carries a random identifier (32 B), one
   cookie, placeholders for new cookies and an AES-SIV **authenticator**
   over the header and the previous extensions (C2S key). The reply is
   verified with the S2C key and carries new cookies in its encrypted part.
3. Every cookie is used **once** (queries cannot be linked). When they run
   out or the server sends NTSN ("kiss of death"), NTS-KE runs again.

A spoofed reply (another identifier, a changed byte, a missing
authenticator) is dropped and ntpd keeps waiting for the real one; an
attacker can only block synchronization, not shift the time.

| `servers` | Meaning |
|---|---|
| `nts://name[:port]` | NTS, NTS-KE on the port (default 4460) |
| `name`, `address[:port]` | unauthenticated NTP (default port 123) |

| `unauthenticated` | When unauthenticated servers are used |
|---|---|
| `never` (default) | never; without a reply from an NTS server the clock is not corrected |
| `fallback` | only when no NTS server replies in a round |

Init changes the rule at run time with `policy never|fallback` (by network
profile). The `ntpd` network session for an `nts://` server: TCP to NTS-KE,
UDP to the NTS-KE server (any port) and UDP 123 anywhere (NTS-KE chooses
the NTP server).

> ⚠️ **The clock and the certificate need each other:** the NTS-KE server
> certificate is checked against the clock. When the RTC clock is outside
> the certificate validity (flat battery, year 1970), NTS-KE fails and the
> time is not synchronized until the user sets it or an unauthenticated
> server does (`fallback`).

> **Change from the first version:** unauthenticated NTP only. **Why:**
> whoever controls the network could shift the time and e.g. let an
> expired or revoked certificate pass.

⚠️ **Unauthenticated NTP** (servers without `nts://`, the `fallback` rule)
can be spoofed by whoever controls the network. The limits in `timed`
slow the attack down (at most 500 ppm, i.e. 43 s per day) but do not stop
it; that is why the default is `never`.

## Time zones

The kernel keeps **UTC only**. The zone is a system setting read by the
time service and programs, not by the kernel. `libos::time::zone` (crate
`libs/tz`) does the conversion:

- No tzdata. A compact table describes zones with their **current** rules.
- Each zone has a standard offset from UTC and at most one daylight saving
  rule. Start and end are given as "n-th / last Sunday of a month at hour
  H", relative to UTC, standard time or local wall time.
- A fixed offset works without the table: `UTC+2`, `UTC-03:30`, `GMT+05:45`.

| Rule | DST starts | DST ends | Zones (selection) |
|---|---|---|---|
| EU | last Sunday of March 01:00 UTC | last Sunday of October 01:00 UTC | Europe/Prague, Bratislava, Berlin, London, Lisbon, Helsinki … |
| USA | 2nd Sunday of March 02:00 local | 1st Sunday of November 02:00 local | America/New_York, Chicago, Denver, Los_Angeles |
| Australia | 1st Sunday of October 02:00 standard | 1st Sunday of April 02:00 standard | Australia/Sydney |
| no DST | – | – | UTC, Asia/Tokyo, Asia/Kolkata (+5:30), Europe/Moscow … |

The full list is in `libs/tz/src/table.rs`. A new zone is one line.

<figure class="diagram">
<svg viewBox="0 0 760 210" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="DST transitions in Prague 2026">
<rect class="box" x="10" y="60" width="180" height="50" rx="6"/><text class="t b" x="100" y="82" text-anchor="middle">CET</text><text class="m" x="100" y="100" text-anchor="middle">UTC+1</text>
<rect class="ok" x="250" y="60" width="260" height="50" rx="6"/><text class="t b" x="380" y="82" text-anchor="middle">CEST</text><text class="m" x="380" y="100" text-anchor="middle">UTC+2</text>
<rect class="box" x="570" y="60" width="180" height="50" rx="6"/><text class="t b" x="660" y="82" text-anchor="middle">CET</text><text class="m" x="660" y="100" text-anchor="middle">UTC+1</text>
<path class="ln" d="M220 30 L220 140"/><path class="ln" d="M540 30 L540 140"/>
<text class="t" x="220" y="22" text-anchor="middle">29 Mar 01:00 UTC</text><text class="t" x="540" y="22" text-anchor="middle">25 Oct 01:00 UTC</text>
<text class="m" x="220" y="160" text-anchor="middle">02:00 → 03:00 local</text><text class="m" x="220" y="176" text-anchor="middle">02:30 does not exist (Gap)</text>
<text class="m" x="540" y="160" text-anchor="middle">03:00 → 02:00 local</text><text class="m" x="540" y="176" text-anchor="middle">02:30 happens twice (Ambiguous)</text>
</svg>
<figcaption>2026 in Prague: an hour is skipped in spring and repeated in autumn.</figcaption>
</figure>

```rust
use libos::time::{self, zone::{Zone, LocalResult}};

let prague = Zone::by_name("Europe/Prague").unwrap();
if let Some(l) = time::local(&prague) {
    let c = l.civil;
    println!("{:02}:{:02} {}", c.hour, c.minute, l.offset.abbrev); // 14:03 CET
}

// Local time -> UTC: not unique around transitions.
match prague.local_to_utc(&civil).unwrap() {
    LocalResult::Single(utc) => { /* the usual case */ }
    LocalResult::Ambiguous { earlier, later } => { /* autumn: hour twice */ }
    LocalResult::Gap(utc) => { /* spring: hour did not exist */ }
}
```

`LocalResult::resolve()` picks one value: the earlier instant when ambiguous
(still DST), and for a gap the conversion with the offset valid before it
(02:30 → 03:30 CEST). This matches `mktime` with `tm_isdst = -1`.

⚠️ **Table limits:**

- No history: the rules apply to all years. The Czech Republic follows EU
  rules since 1996. For older dates (and before 1979, when there was no DST)
  the local time is off by an hour from what it really was.
- When a country changes its rules, the table must be updated.
- Zones with more than two transitions a year or negative DST (Ireland in
  tzdata) are not supported. The offset from UTC is still correct.

### System settings and an RTC in local time (dual boot with Windows)

Time settings are `key = value` text:

```text
zone = Europe/Prague   # table name, or UTC+2, UTC-03:30
rtc = local            # utc (default, Linux) | local (Windows)
```

For now they are fixed in `init` (`system::TIME_SETTINGS`, `rtc = utc` for
QEMU). Once there is a system configuration on disk they will be read from
there. At boot `init` sends them to the time service as its first message.

Windows keeps **local time** in the RTC. Without zone information the kernel
takes it as UTC, so real time is off by the zone offset. With `rtc = local`
the time service fixes this right after boot, before other services start:

<figure class="diagram">
<svg viewBox="0 0 760 200" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Correcting an RTC in local time">
<defs><marker id="tm3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="40" width="170" height="80" rx="6"/><text class="t b" x="95" y="65" text-anchor="middle">RTC (Windows)</text><text class="m" x="95" y="84" text-anchor="middle">14:00 local</text><text class="m" x="95" y="100" text-anchor="middle">(summer, UTC+2)</text>
<path class="ln" d="M180 80 L248 80" marker-end="url(#tm3)"/>
<rect class="acc" x="250" y="40" width="210" height="80" rx="6"/><text class="t b" x="355" y="65" text-anchor="middle">kernel at boot</text><text class="m" x="355" y="84" text-anchor="middle">14:00 "UTC", source RTC/UEFI,</text><text class="m" x="355" y="100" text-anchor="middle">ZONE_ASSUMED_UTC</text>
<path class="ln" d="M460 80 L528 80" marker-end="url(#tm3)"/>
<rect class="ok" x="530" y="40" width="220" height="80" rx="6"/><text class="t b" x="640" y="65" text-anchor="middle">timed: rtc = local</text><text class="m" x="640" y="84" text-anchor="middle">14:00 − 2 h = 12:00 UTC</text><text class="m" x="640" y="100" text-anchor="middle">CLOCK_SET (RTC unchanged)</text>
<text class="m" x="10" y="160">Corrected once only: the source must be UEFI or RTC with ZONE_ASSUMED_UTC. After CLOCK_SET the source</text>
<text class="m" x="10" y="178">is SET, so restarting the service does not shift twice. If the firmware gave a zone, the kernel converted already.</text>
</svg>
<figcaption>RTC in local time: the kernel reads "UTC", the time service subtracts the zone offset.</figcaption>
</figure>

Functions: `zone::rtc_local_correction(&settings, &reading)` (pure
computation, tested on the host) and `time::apply_rtc_local(&clock,
&settings)` (reads, corrects, sets).

⚠️ **Repeated hour:** at the end of DST (Prague: last Sunday of October
02:00–03:00) the local time in the RTC cannot tell which of the two hours
it is. The earlier one (DST) is taken. If the system boots in exactly that
hour, the time is an hour ahead until the network corrects it.

⚠️ **Transition while powered off:** Windows shifts the RTC itself on its
first boot after a DST change. If our system boots first after the change,
the RTC still has the old offset and the time is an hour off until the
network or Windows corrects it.

## Format

```rust
pub struct WallTime {
    pub secs: i64,  // seconds since 1970-01-01 00:00:00 UTC, negative = earlier
    pub nanos: u32, // 0..1_000_000_000
}
```

- **Range:** ±292 billion years, so dates before 1970 and far beyond 2262
  (the limit of u64 nanoseconds) both fit.
- **Leap seconds:** not counted (as in POSIX); every day has 86,400 s.
- **Calendar conversion:** proleptic Gregorian, via `to_civil` and
  `from_civil` in `abi::time` (the "days from civil" algorithm). Tests cover
  selected years from −4713 to 10⁹ and every seventh day across ±2,000
  years.

```rust
use libos::time;

if let Some(t) = time::wall() {
    let c = t.to_civil();
    println!("{:04}-{:02}-{:02} {:02}:{:02} UTC", c.year, c.month, c.day, c.hour, c.minute);
}
let r = time::wall_reading()?;            // with source and flags
if r.flags & time::flags::ZONE_ASSUMED_UTC != 0 { /* zone only assumed */ }
```

## Times in the file system

`fsd` sets the change time in the FS from the real clock. The FS stores
times in the same form (seconds i64 + nanoseconds u32), so it keeps dates
before 1970 and has no 2262 limit. Volumes from an older FS version store
u64 ns since 1970, and a time before 1970 is clamped there. See
[Object times](file-system.md#object-times).
