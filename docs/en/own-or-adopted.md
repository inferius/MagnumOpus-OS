# Own or adopted

What we want to write ourselves and what to take ready-made. We write our
own code where it defines the character of the system: the security model,
the look, the behaviour, the file system. We take existing code where it is
one well-bounded thing with a standard and a single correct answer (a
cipher, a codec, an image format). Writing it again would be reinventing
the wheel: a lot of work and more bugs than code that thousands of projects
have tested for years.

The rules for drivers (translate small ones to Rust from BSD/ISC/MIT, run
large ones as an isolated C process, GPL only in the separate `gpl/`) are in
[Drivers](drivers.md) (chapter Source and licence rules). Adopted code and
its licence are listed in `THIRD-PARTY.md`.

## How to decide

<figure class="diagram">
<svg viewBox="0 0 780 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Own or adopted decision">
<defs><marker id="oa1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="250" height="70" rx="6"/>
<text class="t b" x="135" y="46" text-anchor="middle">Does it define the system?</text>
<text class="m" x="135" y="66" text-anchor="middle">security, look, behaviour, FS,</text>
<text class="m" x="135" y="81" text-anchor="middle">service model, human interface</text>
<rect class="box" x="10" y="130" width="250" height="70" rx="6"/>
<text class="t b" x="135" y="156" text-anchor="middle">One thing with a standard?</text>
<text class="m" x="135" y="176" text-anchor="middle">cipher, codec, format, protocol,</text>
<text class="m" x="135" y="191" text-anchor="middle">Unicode, compression</text>
<rect class="box" x="10" y="230" width="250" height="60" rx="6"/>
<text class="t b" x="135" y="256" text-anchor="middle">Is the code large (100k+ lines)?</text>
<text class="m" x="135" y="276" text-anchor="middle">GPU, Wi-Fi, ACPI, browser</text>
<rect class="acc" x="440" y="20" width="330" height="70" rx="6"/>
<text class="t b" x="605" y="46" text-anchor="middle">Own</text>
<text class="m" x="605" y="66" text-anchor="middle">in Rust, to our design; foreign code</text>
<text class="m" x="605" y="81" text-anchor="middle">only as a reference or a library below it</text>
<rect class="ok" x="440" y="130" width="330" height="70" rx="6"/>
<text class="t b" x="605" y="156" text-anchor="middle">Adopt a Rust library</text>
<text class="m" x="605" y="176" text-anchor="middle">MIT/Apache/BSD crate, no_std preferred;</text>
<text class="m" x="605" y="191" text-anchor="middle">otherwise translate from BSD</text>
<rect class="warn" x="440" y="230" width="330" height="60" rx="6"/>
<text class="t b" x="605" y="256" text-anchor="middle">Adopt C as an isolated process</text>
<text class="m" x="605" y="276" text-anchor="middle">own capabilities only, a crash takes nothing down</text>
<line class="ln-acc" x1="260" y1="55" x2="436" y2="55" marker-end="url(#oa1)"/>
<text class="m" x="348" y="48" text-anchor="middle">yes</text>
<line class="ln" x1="135" y1="90" x2="135" y2="126" marker-end="url(#oa1)"/>
<text class="m" x="150" y="112">no</text>
<line class="ln" x1="260" y1="165" x2="436" y2="165" marker-end="url(#oa1)"/>
<text class="m" x="348" y="158" text-anchor="middle">yes, small</text>
<line class="ln" x1="135" y1="200" x2="135" y2="226" marker-end="url(#oa1)"/>
<text class="m" x="150" y="218">yes, large</text>
<line class="ln" x1="260" y1="260" x2="436" y2="260" marker-end="url(#oa1)"/>
<text class="m" x="348" y="253" text-anchor="middle">yes</text>
</svg>
<figcaption>Own code where it makes the system different; existing code where there is one correct answer.</figcaption>
</figure>

Other deciding questions:

- **Security.** We never write cryptography ourselves. A bug in it is
  invisible and ordinary tests do not catch it.
- **Maintenance.** Formats and codecs evolve (new profiles, vulnerability
  fixes). Adopted code gets updated; our own we would have to track alone.
- **Fit with the system.** When a ready-made thing assumes POSIX,
  unrestricted shared memory or root, write a thin layer of our own on top,
  do not rewrite it.
- **Licences.** We take MIT, Apache 2.0, BSD, ISC and zlib. LGPL only as a
  separate process or dynamic library. GPL only in the separate `gpl/` folder
  as a standalone driver process (see [Drivers](drivers.md#source-and-licence-rules)); we do not
  take AGPL.

## Own (now and in the future)

| Area | Why own | Adopted underneath |
|---|---|---|
| Kernel, IPC, capabilities, scheduler | core of the security model and performance; nothing existing fits | - |
| Service model, init, supervisor, crash recovery | decides how the system behaves on failure | - |
| Accounts, permissions, sessions, administration without root | security model and sign-in UX | Argon2id, FIDO2/CTAP2, ciphers (RustCrypto) |
| File system (format, versions, journal, trash, vaults) | the main difference of the system | hashes (XXH3, BLAKE3), compression (LZ4, zstd) |
| FS git layer | joins FS versions with git | `gix` (gitoxide) after the Rust std port |
| Compositor, UI library, design system, icons | look and responsiveness are our brand | tiny-skia (vectors), Inter and JetBrains Mono fonts |
| Shell, terminal, commands, disk manager | everyday tools tied to the FS and capabilities | - |
| System apps: files, settings, installer, task manager | must match the design and our FS | image decoders, previews |
| AI agent layer | own model of permissions and workspaces | - |
| Small drivers (network, disks, USB, audio, input) | small, security sensitive, we want Rust | datasheet facts, translation from BSD |
| The `cstd` C library, later a libc/POSIX layer | on top of capabilities, not Unix | - |
| Text editor | part of the system, not just an app | `ropey` (buffer), `tree-sitter` (highlighting) |
| Own compiler (long-term goal) | full control over the tools | until then Rust, clang |

## Adopted (it would be reinventing the wheel)

| Area | What to take | Note |
|---|---|---|
| Cryptography, TLS | RustCrypto, `rustls`, `ring`, `argon2`, `blake3` | never our own ciphers or protocol |
| Root certificates | Mozilla's list (`webpki-roots`) | data only |
| Compression | `lz4_flex`, `ruzstd`/zstd, `miniz_oxide` (deflate) | the formats are fixed |
| Hashes | `xxhash-rust`, `blake3`, `sha2` | |
| Unicode | normalization, segmentation, bidi, line breaking (`unicode-*`, ICU4X) | tables change with every Unicode version |
| Text shaping | `rustybuzz` (HarfBuzz in Rust) | Arabic, Indic scripts, ligatures |
| Vector graphics, SVG | `tiny-skia`, `usvg`/`resvg` | path rasterization, gradients |
| Images | `png`, `zune-jpeg`, `image-webp`, `gif` | decoders are a common attack target, we want proven ones |
| Audio and video | `symphonia`, `opus`, `dav1d`; FFmpeg only as a separate process | codecs are huge and patent sensitive |
| PDF | `pdfium` | not MuPDF (AGPL) |
| Browser (engine) | Servo or Ladybird | only the window and integration are ours |
| Windows apps | Wine + DXVK | own lower layer on top of capabilities |
| AMD, NVIDIA, newer Intel graphics | Linux DRM drivers through LinuxKPI, isolated | hundreds of thousands of lines |
| Wi-Fi | BSD/Linux drivers isolated, vendor firmware | |
| ACPI | ACPICA as the `acpid` process | already in the system |
| Git (objects, protocol) | `gix` | |
| Regular expressions | `regex` | |
| JSON, serialization | `serde`, `serde_json` | |
| Time zones | tzdata (IANA) | data only, we do the conversion (`libs/tz`) |
| Keyboard layouts | CLDR / xkeyboard-config data | data only |
| Spell-check dictionaries | Hunspell dictionaries | data only |
| Fonts | Inter, JetBrains Mono, Noto for other scripts | |

## Borderline cases

Things we have as our own because nothing existing fit (`no_std`, size,
integration), but which we may replace in the long run:

| Own code | Why own now | When to consider adopting |
|---|---|---|
| Font rasterizer (`libs/font`) | small, `no_std`, enough for Latin | adopt hinting and shaping (`rustybuzz`, `skrifa`) once more scripts are needed |
| Config reader (`libs/conf`) | TOML subset without extra allocations | consider `toml` after the Rust std port |
| FAT writer (`libs/fatw`) | only logs to the USB stick, a few hundred lines | take `fatfs` when full FAT read/write is needed |
| QR code (`libs/qr`) | crash screen, no allocation | stays, existing crates allocate |
| DNS (`dnsd`) | small, tied to `netd` | adopt for DNSSEC or DoH |

## How to adopt

1. Check the licence and record it in `THIRD-PARTY.md`.
2. Prefer a `no_std` + `alloc` crate; otherwise wait for the Rust std port
   or put the code in a separate process.
3. Write a thin layer of our own over the adopted library (our error type,
   capabilities, logs) so it can be swapped.
4. Large C code only as an isolated process with minimal capabilities.
