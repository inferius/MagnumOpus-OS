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

Linux usually keeps UTC in the RTC; **Windows keeps local time**. On a
Windows machine the real time will be off by the zone (1 to 2 hours in
Central Europe) until zone settings and network sync arrive.

When the firmware does give a zone, UTC = local − zone. This is the UEFI
2.x convention; other versions of the specification state it with the
opposite sign.

⚠️ **Accuracy:** firmware and RTC have a 1 s resolution, so the origin is
accurate to a second. Refining and setting the clock come with networking
(source `source::SET`). Until then, `CLOCK_REALTIME` cannot be set.

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
