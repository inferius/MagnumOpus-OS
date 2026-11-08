# Audio

Audio is handled by two processes and one protocol library:

- **`hda`**: the Intel High Definition Audio controller driver (PCI class
  04:03). It resets the controller, finds the codecs and the path to an
  output, and runs DMA. It is the only process with access to the
  controller.
- **`audiod`**: the audio service. It mixes application streams,
  converts formats and sample rates, and controls master volume and mute.
- **`libs/audioproto`**: the protocol between applications, the service
  and the driver. Samples do not travel over a channel but through shared
  memory (VMO).

The hardware-free logic lives in libraries tested on the host:
`libs/hdaspec` (registers, verbs, widget graph, path search) and
`libs/audiomix` (format conversion, sample rate conversion, mixing,
tone, WAV).

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Audio path">
<defs><marker id="au1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="150" height="56" rx="6"/><text class="t b" x="85" y="43" text-anchor="middle">application A</text><text class="m" x="85" y="62" text-anchor="middle">44.1 kHz mono</text>
<rect class="box" x="10" y="100" width="150" height="56" rx="6"/><text class="t b" x="85" y="123" text-anchor="middle">shell (audio)</text><text class="m" x="85" y="142" text-anchor="middle">48 kHz, WAV</text>
<rect class="ok" x="190" y="20" width="140" height="136" rx="6"/><text class="t b" x="260" y="45" text-anchor="middle">ring buffers</text><text class="m" x="260" y="64" text-anchor="middle">one VMO per stream</text><text class="m" x="260" y="80" text-anchor="middle">write_pos / read_pos</text><text class="m" x="260" y="96" text-anchor="middle">no copy via kernel</text><text class="m" x="260" y="112" text-anchor="middle">"space" event</text>
<path class="ln" d="M160 48 L188 48" marker-end="url(#au1)"/><path class="ln" d="M160 128 L188 128" marker-end="url(#au1)"/>
<rect class="acc" x="360" y="20" width="170" height="136" rx="6"/><text class="t b" x="445" y="45" text-anchor="middle">audiod</text><text class="m" x="445" y="64" text-anchor="middle">format conversion</text><text class="m" x="445" y="80" text-anchor="middle">rate conversion</text><text class="m" x="445" y="96" text-anchor="middle">mix, volume</text><text class="m" x="445" y="112" text-anchor="middle">2 periods ahead of DMA</text>
<path class="ln" d="M330 88 L358 88" marker-end="url(#au1)"/>
<rect class="ok" x="560" y="20" width="190" height="70" rx="6"/><text class="t b" x="655" y="45" text-anchor="middle">DMA buffer</text><text class="m" x="655" y="64" text-anchor="middle">4 periods of 5.33 ms</text><text class="m" x="655" y="80" text-anchor="middle">s16 stereo 48 kHz</text>
<path class="ln" d="M530 55 L558 55" marker-end="url(#au1)"/>
<rect class="box" x="560" y="110" width="190" height="56" rx="6"/><text class="t b" x="655" y="133" text-anchor="middle">hda (driver)</text><text class="m" x="655" y="152" text-anchor="middle">BDL, CORB/RIRB, MSI</text>
<path class="ln" d="M655 166 L655 198" marker-end="url(#au1)"/>
<path class="ln" d="M560 138 L532 120" marker-end="url(#au1)"/><text class="m" x="470" y="182">DMA position + period event</text>
<rect class="box" x="560" y="200" width="190" height="70" rx="6"/><text class="t b" x="655" y="225" text-anchor="middle">HDA controller and codec</text><text class="m" x="655" y="244" text-anchor="middle">DAC, mixer, pin</text><text class="m" x="655" y="260" text-anchor="middle">line out, headphones</text>
<path class="ln" d="M700 92 L700 198" marker-end="url(#au1)"/><text class="m" x="706" y="190">reads DMA</text>
<text class="m" x="10" y="300">Channels carry only commands (OPEN, START, DRAIN, volume); samples live in shared memory.</text>
<text class="m" x="10" y="318">audiod never sees controller registers or the BDL, only the stream DMA buffer and the position page.</text>
</svg>
<figcaption>Audio path from an application to the speaker.</figcaption>
</figure>

## Start-up

1. `devmgr` finds a PCI function of class 04:03 (any vendor: Intel, AMD,
   VMware, QEMU) and starts `hda` with a restricted Resource (BARs,
   configuration, 2 interrupts, 1 MiB DMA). It sends init an `audio`
   message with the channel to the driver.
2. init starts `audiod` and hands it the channels of all drivers. The
   service asks each for its device (`INFO`) and uses the first one with
   an analog output; others (e.g. graphics card HDMI audio) are skipped.
3. init keeps the service's bootstrap channel; further clients are
   created with `CONNECT` (termd sends it for each shell session, tests
   for themselves).

## Protocol (`libs/audioproto`)

### Formats

A PCM format = sample rate (8-192 kHz), channel count (1-8) and sample
type: `u8`, `s16`, `s24` (24 bits in a 32-bit word), `s32`, `f32`.
Channel samples are interleaved. The device plays 48 kHz (or 44.1 kHz
if the codec lacks 48 kHz), 16-bit stereo; the service converts.

### Shared ring buffer

The stream buffer is created by the **service** (so the client does not
decide its size or layout) and handed to the client as a VMO. Both map it:

| offset | field | written by |
|---|---|---|
| 0 | magic `AUR1`, version | service |
| 8 | capacity (frames), bytes per frame | service |
| 64 | `write_pos` (u64, own cache line) | producer |
| 128 | `read_pos` (u64) | consumer |
| 192 | underruns, overruns | service |
| 208 | played frames, latency (us) | service |
| 4096 | data | producer |

Positions are monotonic frame counts (index = position mod capacity).
The producer writes data, then `write_pos` (Release); the consumer reads
`write_pos` (Acquire), the data, and advances `read_pos`. The other side
is not trusted: an out-of-range position (more than the capacity ahead,
or backwards) means "nothing to read" and the service resyncs; it never
reads outside the buffer.

After each period the service sets the stream's **event** (a handle in
the `OPEN` reply). The client waits on it, not on the channel: one write
into the buffer and one wait per period, no message per block of data.

### Messages

| message | meaning |
|---|---|
| `OPEN` (direction, format, latency) | new stream; reply `Opened` + [buffer VMO, event] |
| `START`, `STOP` | start; stop and drop unplayed data |
| `DRAIN` | replies once everything written has played (stream then stops) |
| `CLOSE` | destroys the stream (closing the channel destroys all its streams) |
| `STREAM_VOLUME`, `MASTER_VOLUME` | volume 0-100 % (curve percent³) |
| `MUTE` | mutes the whole output |
| `STATUS`, `STREAM_INFO` | device and service state, stream state |
| `CONNECT` | new channel to the service |

Volume 50 % is about -18 dB, 10 % -60 dB (a cubic curve follows perceived
loudness better than a linear one).

**Recording** (`Direction::Input`) has the same shape in the protocol
with roles swapped: the service writes, the client reads, the event means
"new data". The driver cannot record yet; `OPEN` with direction `Input`
returns `NOT_SUPPORTED`.

### Driver protocol

`audiod` sends the driver `OPEN_OUTPUT` (format, period, number of
periods) and gets three handles: the **DMA buffer** (the service mixes
directly into it, no extra copy), the **status page** (DMA position in
frames, interrupt count, errors) and the **period event**. The service
never sees the BDL, CORB, RIRB or controller registers. `START` and
`STOP` start and stop DMA; `OPEN_INPUT` is reserved for recording.

## The `hda` driver

### Controller

1. PCI configuration: "no snoop" disabled in PCIe Device Control (DMA
   memory is cached, the device must be coherent), traffic class TC0 on
   Intel (register 0x44), snoop enabled on AMD/ATI (register 0x42). Same
   as Linux `snd_hda_intel`.
2. Stop DMA left by firmware, reset (`GCTL.CRST` 0 and back to 1), 1 ms
   for codecs to announce themselves (`STATESTS`).
3. CORB (commands) and RIRB (responses) in one DMA page, the largest size
   the controller supports (usually 256 entries). Commands are
   synchronous with busy-waiting and a 20 ms deadline (codecs answer in
   microseconds). If CORB/RIRB does not start, the immediate command
   interface (`ICOI`/`ICII`) is used. A codec that does not answer is
   skipped from then on.
4. MSI interrupts (not MSI-X) via `libs/pci::msi`. Without MSI the driver
   reads the position on a timer twice per period.

### Codecs and paths

The driver reads all widgets of each audio function group:
capabilities, connection lists (short and long form, ranges), and for
pins their capabilities and **default configuration** (how the board
vendor wired the jack: line out, speaker, headphones, HDMI; association
and sequence). The decisions live in `hdaspec::graph`:

<figure class="diagram">
<svg viewBox="0 0 760 230" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Codec paths">
<defs><marker id="au2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="30" width="110" height="46" rx="6"/><text class="t b" x="65" y="52" text-anchor="middle">DAC 0x02</text><text class="m" x="65" y="68" text-anchor="middle">stream 1</text>
<rect class="acc" x="10" y="120" width="110" height="46" rx="6"/><text class="t b" x="65" y="142" text-anchor="middle">DAC 0x03</text><text class="m" x="65" y="158" text-anchor="middle">stream 1</text>
<rect class="box" x="200" y="30" width="120" height="46" rx="6"/><text class="t b" x="260" y="52" text-anchor="middle">mixer 0x0C</text><text class="m" x="260" y="68" text-anchor="middle">DAC input only</text>
<rect class="box" x="200" y="120" width="120" height="46" rx="6"/><text class="t b" x="260" y="142" text-anchor="middle">mixer 0x0D</text><text class="m" x="260" y="158" text-anchor="middle">DAC input only</text>
<path class="ln" d="M120 53 L198 53" marker-end="url(#au2)"/><path class="ln" d="M120 143 L198 143" marker-end="url(#au2)"/>
<rect class="ok" x="420" y="10" width="160" height="46" rx="6"/><text class="t b" x="500" y="32" text-anchor="middle">pin 0x14 speaker</text><text class="m" x="500" y="48" text-anchor="middle">EAPD, auto-mute</text>
<rect class="ok" x="420" y="80" width="160" height="46" rx="6"/><text class="t b" x="500" y="102" text-anchor="middle">pin 0x1B line out</text><text class="m" x="500" y="118" text-anchor="middle">rear panel</text>
<rect class="ok" x="420" y="150" width="160" height="46" rx="6"/><text class="t b" x="500" y="172" text-anchor="middle">pin 0x15 headphones</text><text class="m" x="500" y="188" text-anchor="middle">detection, HP amp</text>
<path class="ln" d="M320 50 L418 33" marker-end="url(#au2)"/><path class="ln" d="M320 140 L418 103" marker-end="url(#au2)"/><path class="ln" d="M320 60 L418 168" marker-end="url(#au2)"/>
<text class="m" x="600" y="40">1st pin: free DAC</text><text class="m" x="600" y="110">2nd pin: next free</text><text class="m" x="600" y="180">3rd pin: shares DAC</text>
<text class="m" x="10" y="218">All DACs get the same stream tag: one DMA stream plays on all outputs at once.</text>
</svg>
<figcaption>A typical codec (Realtek ALC): three outputs, two converters.</figcaption>
</figure>

1. Output pins from the default configuration (connected, output
   capable), sorted by association and sequence. HDMI/DP and S/PDIF
   separately.
2. For each pin the shortest path upstream (pin -> mixers/selectors ->
   DAC, at most 6 nodes): a free DAC first, otherwise a DAC already in
   use. A selector can have only one selection, so a path that would
   change another path's selection is rejected.
3. All DACs get the same stream: line out, headphones and speaker play at
   the same time. **Auto-mute:** when jack detection reports headphones,
   the speakers are switched off (pin without `OUT`). Detection is read
   every 500 ms (no unsolicited responses yet).
4. Path setup: power D0, selector and pin selections, amplifiers at 0 dB
   (the service does volume in software), mixers pass only the path's
   input (others, e.g. a microphone loopback, are muted), pin control
   (`OUT`, plus `HP` for headphones), EAPD.
5. If the default configuration lists no analog output (a badly filled
   BIOS), the driver tries every output-capable pin as a line out.

**HDMI/DP** is only found and reported (`INFO`: output `hdmi`, inactive).
Reading the ELD (what the monitor supports), infoframes and digital
converter setup are missing; that is the next step.

### Stream and DMA

The output stream is the first output descriptor (after the input ones).
The DMA buffer is a ring of 4 periods; each period is one BDL entry with
IOC, so the controller raises an interrupt after every period. Entry
address and length are 128-byte aligned (a 256-frame period = 1024 B).
DMA memory comes from the driver's Resource, so with an IOMMU (VT-d,
AMD-Vi, SMMUv3) the device can reach it and nothing else. The position
(`LPIB`) is converted to a monotonic frame count in the status page. A
FIFO or descriptor error stops the stream and is reported.

## The `audiod` service

### Timing and latency budget

The service waits for client channels and for the driver's period event.
After each period it reads the DMA position and mixes data so that it is
two periods ahead of it. Behind the mixed data it always zeroes one
period: if the service falls behind, silence plays (and
`device_underruns` is counted), not stale data from the previous lap.

| part (48 kHz, period 256 frames = 5.33 ms) | time |
|---|---|
| mixed ahead of the DMA position (2 periods) | 5.3-10.7 ms |
| controller and codec FIFO | < 0.5 ms |
| client buffer (min. 5 ms, test 50 ms, shell 100 ms) | per client |
| **from a client write with a 10 ms buffer to the speaker** | **about 11-21 ms** |

The DMA buffer has 4 periods (21.3 ms): two with data, one with silence
and one spare that the device is reading. The service wakes 187 times a
second; mixing one period for two streams takes tens of microseconds.
When no stream has played for 2 s the service stops DMA (the device does
not play silence forever) and on the next `START` zeroes the buffer and
starts again.

### Format and rate conversion

Each stream's samples are converted to internal stereo Q23 (24 bits in
i32): mono to both channels, more channels = average of even ones to the
left and odd ones to the right. Sample rate conversion is **cubic
Catmull-Rom interpolation** with a Q32 phase (exactly the input at equal
rates). Mixing sums in i64 with the stream gain, then master volume and
clipping to 16 bits (only the sum is clipped, so two loud streams in
opposite phase do not distort). Everything is integer: programs on
aarch64 run without an FPU.

## Shell command `audio`

```shell-b
audio tone 1000 --duration 2s --level 30
audio play /music/sample.wav
audio volume 60
audio mute
audio status
```

`audio volume` without a number returns the volume, `audio unmute`
unmutes. `tone` and `play` return a record with the played duration and
the underrun count. The WAV file is read by the interpreter through the
session FS (same rights as `open`): PCM 8/16/32-bit, 24-bit in a 32-bit
word and float 32, any rate and channel count. The command waits until
the sound has played.

## Tests

- **On the host** (`cargo xtask test`): `audioproto` (messages, the
  buffer including hostile positions), `audiomix` (conversions, Goertzel
  on a tone after 44.1 -> 48 kHz conversion, mixing, WAV), `hdaspec`
  (verbs, formats, connection lists, QEMU hda-duplex, Realtek ALC,
  selector, HDMI and cycle graphs), the `audio` command in `shlang`,
  tone detection in xtask.
- **In QEMU**, suite `zvuk`: device and output, the stream runs and DMA
  advances the position (at least by the played data), interrupts
  arrive, two clients mixed (48 and 22.05 kHz), volume and mute, invalid
  requests, `audio tone`/`volume`/`status` through termd and a session.
- **Recorded sound:** QEMU has `-device intel-hda -device hda-duplex`
  with `-audiodev wav` (48 kHz, s16, stereo). The test `ton 1 kHz do WAV`
  plays 1.5 s of a 1 kHz tone (mono 44.1 kHz, i.e. through rate
  conversion); after QEMU exits, xtask looks for 50 ms windows with a
  clean 1 kHz tone in the file and requires at least 500 ms.

With sound interactively: `cargo xtask run --audio dsound` (Windows),
`--audio pa` or `alsa` (Linux), `coreaudio` (macOS).

## Hardware and risks

- **QEMU** (`intel-hda` on q35 and on the PCIe of the aarch64 virt
  machine) and **VMware** (HDA with a Realtek-type codec) use the same
  paths as real boards.
- **Intel and AMD boards** with an analog codec (Realtek, Conexant, VIA)
  should play, but **this has not been verified on real hardware yet.**
  Some codecs need vendor-specific fixups (Realtek coefficients, GPIO
  for the amplifier on laptops) that Linux keeps in fixup tables; they
  are not here. The result can be silent laptop speakers while line out
  plays.
- **Intel laptops with a DSP (SOF, digital microphones)** route audio
  through the DSP and may not show it in HDA mode at all. Unsupported.
- **HDMI/DP**: only prepared (see above).
- Rate conversion when **downsampling** (e.g. 96 -> 48 kHz) does not
  filter the band above the new Nyquist frequency: content above 24 kHz
  can fold back (aliasing). Common sources (44.1 and 48 kHz) are not
  affected.
- Without unsolicited responses, switching to headphones takes up to
  500 ms.
- `audio play` and `audio tone` cannot be interrupted with Ctrl-C yet.
