# Audio

Audio is handled by two processes and one protocol library:

- **`hda`**: the Intel High Definition Audio controller driver (PCI class
  04:03). It resets the controller, finds the codecs and the path to an
  output, and runs DMA. It is the only process with access to the
  controller.
- **`audiod`**: the audio service. It mixes application streams,
  distributes recorded audio, converts formats and sample rates, and
  controls master volume, mute and sound card selection.
- **`libs/audioproto`**: the protocol between applications, the service
  and the driver. Samples do not travel over a channel but through shared
  memory (VMO).

The hardware-free logic lives in libraries tested on the host:
`libs/hdaspec` (registers, verbs, widget graph, output and input paths,
HDMI, board fixups) and `libs/audiomix` (format conversion, sample rate
conversion with an anti-aliasing filter, mixing, tone, WAV).

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Audio path">
<defs><marker id="au1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="150" height="56" rx="6"/><text class="t b" x="85" y="43" text-anchor="middle">application A</text><text class="m" x="85" y="62" text-anchor="middle">44.1 kHz mono</text>
<rect class="box" x="10" y="100" width="150" height="56" rx="6"/><text class="t b" x="85" y="123" text-anchor="middle">shell (audio)</text><text class="m" x="85" y="142" text-anchor="middle">48 kHz, WAV</text>
<rect class="ok" x="190" y="20" width="140" height="136" rx="6"/><text class="t b" x="260" y="45" text-anchor="middle">ring buffers</text><text class="m" x="260" y="64" text-anchor="middle">one VMO per stream</text><text class="m" x="260" y="80" text-anchor="middle">write_pos / read_pos</text><text class="m" x="260" y="96" text-anchor="middle">no copy via kernel</text><text class="m" x="260" y="112" text-anchor="middle">"space" event</text>
<path class="ln" d="M160 48 L188 48" marker-end="url(#au1)"/><path class="ln" d="M160 128 L188 128" marker-end="url(#au1)"/>
<rect class="acc" x="360" y="20" width="170" height="136" rx="6"/><text class="t b" x="445" y="45" text-anchor="middle">audiod</text><text class="m" x="445" y="64" text-anchor="middle">format conversion</text><text class="m" x="445" y="80" text-anchor="middle">rate conversion</text><text class="m" x="445" y="96" text-anchor="middle">mix, volume</text><text class="m" x="445" y="112" text-anchor="middle">2 periods ahead of DMA</text>
<path class="ln" d="M330 88 L358 88" marker-end="url(#au1)"/>
<rect class="ok" x="560" y="20" width="190" height="70" rx="6"/><text class="t b" x="655" y="45" text-anchor="middle">DMA buffer</text><text class="m" x="655" y="64" text-anchor="middle">16 periods of 5.33 ms</text><text class="m" x="655" y="80" text-anchor="middle">s16 stereo 48 kHz</text>
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
2. init starts `audiod` under the supervisor and hands it the channels
   of all drivers. The service asks each for its device (`INFO`) and makes
   the first one with an analog output active; others (e.g. graphics card
   HDMI audio) are in the list (`DEVICES`) and can be selected (`SELECT`).
   Recording uses the active card if it has an input, otherwise the first
   other card with an input (opened separately, its output is not
   started).
3. init keeps the service's bootstrap channel (the only way to add a
   driver at run time). Clients get sessions via `CONNECT`: termd with a
   lease from the supervisor (`session("audiod")`) and each shell session
   its own leased session (`narrow`), tests without a lease.

### Recovery after a crash

- **`audiod` crashes:** the supervisor starts it again. init holds the
  channels to the drivers (`Kept`), so the drivers notice nothing and the
  new instance gets them right away; it reopens the stream at the
  driver. Clients with a lease (`svcclient::Conn`) reconnect on their next
  call; open streams do not survive the restart (calls return
  `SERVICE_RESTARTED` or `NOT_FOUND`, the program opens a new one).
- **`hda` crashes:** devmgr restarts it in its own thread (at most 5
  times a minute) and sends init a new `audio` message. init keeps the
  channel and hands it to the running service (`device`); the service
  drops the old driver (it sees a closed channel) and switches to the new
  one.

Both are covered by the tests "zabij audiod" and "zabij ovladac hda"
(suite `obnova zvuku`).

## Protocol (`libs/audioproto`)

### Formats

A PCM format = sample rate (8-192 kHz), channel count (1-8) and sample
type: `u8`, `s16`, `s24` (24 bits in a 32-bit word), `s32`, `f32`.
Channel samples are interleaved. The device plays 48 kHz (or 44.1 kHz
if the codec lacks 48 kHz), stereo, 16-bit, or 24-bit in a 32-bit word
when every output codec supports it. It records 48 kHz, stereo, 16-bit.
The service converts to the client's format.

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
| `DEVICES`, `SELECT` | list of sound cards, switching (streams keep playing on the new one) |

Volume 50 % is about -18 dB, 10 % -60 dB (a cubic curve follows perceived
loudness better than a linear one).

**Recording** (`Direction::Input`) has the same shape with roles
swapped: the service writes, the client reads, the event means "new
data". When the client does not read in time, the service drops the rest
and counts an overrun. A device without input returns `NOT_SUPPORTED`.

The client `audioproto::conn::AudioClient` is built on `svcclient::Conn`:
with a lease it survives a service restart, without one it returns
`SERVICE_UNAVAILABLE`.

### Driver protocol

`audiod` sends the driver `OPEN_OUTPUT` (format, period, number of
periods) and gets three handles: the **DMA buffer** (the service mixes
directly into it, no extra copy), the **status page** (DMA position in
frames, interrupt count, errors) and the **period event**. The service
never sees the BDL, CORB, RIRB or controller registers. `START` and
`STOP` start and stop DMA. Recording has `OPEN_INPUT`, `START_INPUT`,
`STOP_INPUT`, `CLOSE_INPUT`; positions in the status page are the frames
the device has already written into the buffer. `TEST_EXIT` ends the
driver as if it crashed (only `audiod` sends it, on init's request in the
recovery test).

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

The driver reads, from every codec on the link (all addresses that
announced themselves), all widgets of the audio function group:
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
3. All DACs of all codecs get the same stream: line out, headphones and
   speaker play at the same time. **Auto-mute:** when jack detection
   reports headphones, the speakers are switched off (pin without `OUT`).
   Jacks report changes with **unsolicited responses** (tag per pin, RIRB
   interrupt); the fallback is reading every 3 s (every 500 ms without
   MSI).
4. Path setup: power D0, selector and pin selections, amplifiers at 0 dB
   (the service does volume in software), mixers pass only the path's
   input (others, e.g. a microphone loopback, are muted), pin control
   (`OUT`, plus `HP` for headphones), EAPD.
5. If the default configuration lists no analog output (a badly filled
   BIOS), the driver tries every output-capable pin as a line out.

**HDMI/DP** (`hdaspec::hdmi`): the driver prepares the digital path
(power, pin `OUT`, digital converter, 2 channels, slots) and reads the
monitor connection. When a monitor is connected and the ELD is valid
(verb `GET_HDMI_ELDD` byte by byte: monitor name, HDMI or DP, how many
LPCM channels it accepts), it writes the audio infoframe (HDMI type 0x84
with checksum, DP secondary packet) and the converter gets the same
stream as the analog outputs. Unplugging the monitor disconnects the
converter. Multichannel audio, channel mapping and DP MST are not there
yet.

### Recording

`hdaspec::input` finds the path from an A/D converter (ADC) upstream to an
input pin in this order: microphone jack, built-in microphone, line in,
aux. It sets selections, input amplifiers at 0 dB (for a mixer only the
path's input), the pin as input and, for a microphone, 80 % bias when the
pin supports it. The input stream is the first input descriptor (stream
tag 2).

### Board fixups

Some boards have a wrong default pin configuration, or need GPIO (laptop
speaker amplifier) or codec coefficients. `hdaspec::fixup` has a table
keyed by codec ID and subsystem ID (the board vendor, verb
`GET_SUBSYSTEM_ID`): pin configuration overrides before path planning,
GPIO and coefficients after setup. The data are hardware facts (they may
be taken from public fixup tables, not their code). **The table is empty
for now**: an entry is added once verified on a real machine. A generic
fixup without the table: Realtek codecs get EAPD enabled on every pin
that has it.

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
the lead (initially two periods) ahead of it. Behind the mixed data it always zeroes one
period: if the service falls behind, silence plays (and
`device_underruns` is counted), not stale data from the previous lap.

| part (48 kHz, period 256 frames = 5.33 ms) | time |
|---|---|
| mixed ahead of the DMA position (lead 2 periods, adaptively up to 12) | 5.3-10.7 ms (up to 64 ms) |
| controller and codec FIFO | < 0.5 ms |
| client buffer (min. 5 ms, test 50 ms, shell 100 ms) | per client |
| **from a client write with a 10 ms buffer to the speaker** | **about 11-21 ms** |

**Adaptive lead:** when the service falls behind (the DMA position
overtakes the mixed data), the lead grows by a period, up to 12 periods
(64 ms); after 10 s without an underrun it shrinks by a period again. On
a fast machine latency stays at 11-21 ms; on a loaded one (QEMU on a busy
host that takes the CPU away from the guest for tens of ms) it grows
instead of repeated gaps in the sound. The DMA buffer therefore has 16
periods (85 ms): the lead, one period of silence and a spare that the
device is reading. The service wakes 187 times a
second; mixing one period for two streams takes tens of microseconds.
When no stream has played for 2 s the service stops DMA (the device does
not play silence forever) and on the next `START` zeroes the buffer and
starts again. Input stops as soon as no recording stream runs.

Recording: after each input period the service reads the new data from
the DMA buffer, converts it to stereo Q23 and, through each recording
stream's rate converter, writes it into that stream's buffer in its
format (with the stream volume).

### Format and rate conversion

Each stream's samples are converted to internal stereo Q23 (24 bits in
i32): mono to both channels, more channels are **downmixed** by the
WAV/HDA layout (3 to 8 channels: front at full level, centre and
rear/side at -3 dB, LFE dropped, weights normalised so nothing clips).
Sample rate conversion is **cubic Catmull-Rom interpolation** with a Q32
phase (exactly the input at equal rates). When **downsampling**, input
first goes through a FIR low-pass (47 taps, Hann-windowed sinc, cutoff
0.45 × the new rate): content above the new Nyquist frequency is
suppressed by at least 40 dB. At the end of `DRAIN` the converter's
history is flushed with silence, so the last samples play too. Mixing
sums in i64 with the stream gain, then master volume and clipping to 16
or 24 bits (only the sum is clipped). Everything is integer (filter
coefficients are computed once when the stream opens): programs on
aarch64 run without an FPU.

## Shell command `audio`

```shell-b
audio tone 1000 --duration 2s --level 30
audio play /music/sample.wav
audio volume 60
audio mute
audio status
audio devices
audio device 1
```

`audio volume` without a number returns the volume, `audio unmute`
unmutes, `audio devices` lists sound cards and `audio device N`
switches. `tone` and `play` return a record with the played duration and
the underrun count. Audio goes to the session in 100 ms chunks: the
interpreter reads the WAV file in parts through the session FS (same
rights as `open`, any size) and checks **Ctrl-C** between chunks (then it
stops the stream and returns `SH_INTERRUPTED`). WAV formats: PCM
8/16/32-bit, 24-bit in a 32-bit word and float 32, any rate and channel
count.

## Tests

- **On the host** (`cargo xtask test`): `audioproto` (messages, the
  buffer including hostile positions), `audiomix` (conversions both ways,
  downmix, Goertzel on a tone after 44.1 -> 48 kHz conversion, alias
  suppression 96 -> 48 kHz, 16- and 24-bit mixing, WAV), `hdaspec`
  (verbs, formats, connection lists, QEMU hda-duplex, Realtek ALC,
  selector, HDMI and cycle graphs, input path, ELD and infoframe, fixup
  table), the `audio` command in `shlang` (chunks, partial reads,
  Ctrl-C), tone detection in xtask.
- **In QEMU**, suite `zvuk`: device and output, the stream runs and DMA
  advances the position, interrupts arrive, two clients mixed (48 and
  22.05 kHz), volume and mute, invalid requests, `audio
  tone`/`volume`/`status` through termd and a session, recording (16 kHz
  mono through downsampling) from the second card, a list of two cards
  and switching the output. Suite `obnova zvuku`: "zabij audiod" (the new instance has
  the device at once, a leased client reconnects by itself) and "zabij
  ovladac hda" (devmgr restarts it, the service switches to it).
- **Recorded sound:** QEMU has two `intel-hda` controllers: the first
  with the `hda-output` codec into `-audiodev wav` (48 kHz, s16, stereo),
  the second with `hda-duplex` and the `none` backend (which also
  provides input for the recording test; the wav backend cannot record).
  QEMU does not model two codecs on one link correctly (each codec takes
  its own data from DMA, while real hardware broadcasts the stream), hence
  two controllers. The test `ton 1 kHz do WAV` plays 1.5 s of a 1 kHz
  tone (mono 44.1 kHz, i.e. through rate conversion). That it played in
  full is checked in QEMU by the audiod counter (all 66,150 frames). The
  WAV file is a supplementary check of the whole path: after QEMU exits,
  xtask looks for 50 ms windows with a clean 1 kHz tone and requires at
  least 500 ms. If the tone is missing and audiod reported device
  underruns during it (the host took the CPU away from the guest), it is
  a warning with an explanation; without underruns it is an output path
  error. xtask also prints the file's timeline in 100 ms steps (`.`
  silence, `t` tone, `?` other sound).

With sound interactively: `cargo xtask run --audio dsound` (Windows),
`--audio pa` or `alsa` (Linux), `coreaudio` (macOS).

## Hardware and risks

- **QEMU** (`intel-hda` on q35 and on the PCIe of the aarch64 virt
  machine) and **VMware** (HDA with a Realtek-type codec) use the same
  paths as real boards.
- **Intel and AMD boards** with an analog codec (Realtek, Conexant, VIA)
  should play, but **this has not been verified on real hardware yet.**
  The board fixup table is empty: a laptop that needs GPIO or
  coefficients for its amplifier may have silent speakers while line out
  plays. An entry is added once verified on that machine.
- **Intel laptops with a DSP (SOF, digital microphones)** route audio
  through the DSP and may not show it in HDA mode at all. Unsupported.
- **HDMI/DP** is written to the specification, but QEMU has no HDMI
  codec, so tests do not exercise it: verify on a real graphics card.
  Stereo only; DP MST (several monitors on one connector) is missing.
- **24-bit** is used only when every output codec reports it; QEMU
  supports only 16 bits, so the 24-bit path is verified only by host
  tests.
- Recording uses a single input (the active card if it has one,
  otherwise the first other card with input; on the card the first input
  by priority); manual input selection is not there yet.
- Several output codecs on one controller play the same stream; this
  cannot be verified in QEMU (see Tests), only the enumeration of both
  codecs.
