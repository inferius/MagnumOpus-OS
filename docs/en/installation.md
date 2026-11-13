# Installing to an internal disk

The installer writes MagnumOpus from the [live USB](live-usb.md) onto an
internal disk of the computer (NVMe, SATA, USB disk): its own EFI
partition, a partition with our file system, the boot loader, settings and
the first account. It has two forms with the same steps: a text one (the
`install` command in the terminal) and a graphical one (the "Nainstalovat
MagnumOpus" window on the live USB, `design/instalace.html`).

Contents:

1. [Wizard steps](#wizard-steps)
   - [Graphical wizard](#graphical-wizard)
2. [Where to install](#where-to-install): whole disk, free space, reinstall
3. [Protecting foreign partitions](#protecting-foreign-partitions): raw channel and diskguard
4. [What gets written](#what-gets-written)
5. [Firmware menu (NVRAM)](#firmware-menu-nvram)
6. [System settings](#system-settings): `system.toml`, keyboard, time zone
7. [Unattended install](#unattended-install) (`--odpovedi`)
8. [Tests](#tests)
9. [Limitations](#limitations)

## Wizard steps

The `install` command in the terminal (started from the live USB) walks
the user through the steps of the design **[verified in QEMU]** (x86_64 and aarch64,
`--only instalace`, unattended mode; the interactive prompts only by hand):

1. **Language and keyboard** - Czech; several keyboard layouts (e.g. `cz`
   and `us`), each with a preview of its key rows. Switching layouts with
   **left Alt + left Shift**. The layout list comes from the input service
   (`libs/keymap`, single source).
2. **Computer check** - list of disks (model, size, what is on them).
3. **Where to install** - choices depending on the disk, see below.
4. **Preview of changes** - every partition marked *stays* / *new* /
   *will be erased*.
5. **Your account** - name, display name, password (twice), optional PIN.
   The password is entered hidden (the terminal masks characters, the
   `Ask` message has `secret = true`).
6. **Security** - always the text "Disk nebude šifrovaný - šifrování disku
   se připravuje" (the disk will not be encrypted, encryption is being
   prepared). The installer offers neither encryption nor the recovery
   key (which belongs to encryption) yet.
7. **Summary** - for the whole disk, type the disk name, otherwise confirm
   with `ano`. If another OS is on the disk, a question which system boots
   by default.
8. **Installation progress** - eight steps (reading the system from the
   live USB, partition table, EFI partition, loader and system, file
   system, settings, account, verification). Reading from the stick comes
   first and takes longest: a debug build under TCG needs ~50 s before
   anything is written.

<figure class="diagram">
<svg viewBox="0 0 760 210" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Path from the install command to the disk">
<rect class="box" x="10" y="30" width="120" height="60" rx="4"/><text class="t b" x="70" y="55" text-anchor="middle">install</text><text class="m" x="70" y="73" text-anchor="middle">shell (shsess)</text>
<rect class="box" x="170" y="30" width="120" height="60" rx="4"/><text class="t b" x="230" y="55" text-anchor="middle">init</text><text class="m" x="230" y="73" text-anchor="middle">OpenInstaller</text>
<rect class="acc" x="330" y="30" width="140" height="60" rx="4"/><text class="t b" x="400" y="55" text-anchor="middle">installd</text><text class="m" x="400" y="73" text-anchor="middle">plan, write, verify</text>
<rect class="warn" x="510" y="30" width="110" height="60" rx="4"/><text class="t b" x="565" y="55" text-anchor="middle">disk driver</text><text class="m" x="565" y="73" text-anchor="middle">raw_write_allowed</text>
<rect class="ok" x="660" y="30" width="90" height="60" rx="4"/><text class="t b" x="705" y="55" text-anchor="middle">disk</text><text class="m" x="705" y="73" text-anchor="middle">GPT + ours</text>
<line x1="130" y1="60" x2="170" y2="60" class="line"/><line x1="290" y1="60" x2="330" y2="60" class="line"/><line x1="470" y1="60" x2="510" y2="60" class="line"/><line x1="620" y1="60" x2="660" y2="60" class="line"/>
<text class="m" x="10" y="125">1. The shell asks init (powerproto OpenInstaller); init starts installd on the first request and hands it the raw disk channels.</text>
<text class="m" x="10" y="145">2. The shell talks to installd with installplan::proto (Disks, Keyboards, Preview, Install -> Progress, Done).</text>
<text class="m" x="10" y="165">3. installd writes through the raw channel; the driver allows writes only to the GPT and to installer partitions.</text>
<text class="m" x="10" y="185">4. Decisions (choices, plan, preview) live in the installplan library, tested on the host.</text>
</svg>
<figcaption>From the <code>install</code> command to the disk write.</figcaption>
</figure>

## Graphical wizard

On the live USB, the bar and the launcher contain the app **Nainstalovat
MagnumOpus** (Install MagnumOpus; `user/gui-install`, icon `download`).
After login its window opens by itself, like the other desktop apps. On
the installed system the app is not listed (init adds it only when
started from the live stick). Status: **[verified in QEMU]** (x86_64,
`--only instalace-gui`).

- **Same steps as `install`.** The wizard (`libs/installview`) talks to
  installd with the same `installplan::proto` protocol. The disk plan,
  choices, preview, and name and password rules live in `installplan`
  and installd. The window only holds the step state and decides nothing
  on its own.
- **Step 1, keyboard.** A table of layouts with a preview of the key rows
  of the selected layout. *Add*, *Remove* and *Make main* buttons (the
  first layout is the main one) and a time zone field. installd supplies
  the data from `libs/keymap`, the same source the input service uses.
- **Step 4, change preview.** A disk map *Now* and *After installation*
  (`installplan::map`): segments in disk order including free space, new
  partitions marked. Below the map are the changes *keep* / *new* /
  *erase* and the confirmation: type the disk name for the whole disk,
  otherwise tick "I understand". For now the segments are drawn as text.
  A `libs/ui` component will add the bar chart (requested from the 6c
  thread).
- **Step 6, security.** The *Encrypt disk* switch is greyed out, with an
  explanation below it (`NO_ENCRYPTION`).
- **Step 8, progress.** A progress bar, the eight steps and the
  result. installd also reports the installation to the task overview
  (`taskd`) as the task "Instalace MagnumOpus na ..." with progress. It
  is therefore visible in the Task Manager and the terminal even after
  the window is closed (the write keeps running in installd).
- **Narrow window.** In the `compact` width class (below 600 px) the step
  list on the left disappears and only "Krok N z 8: ..." (step N of 8)
  stays at the top.

QEMU test (`cargo xtask test --only instalace-gui`): an answers file with
`rozhrani = "gui"` goes to the installer window instead of the text path.
The autopilot (`Installer::autopilot`) fills each step from the answers
and moves one step forward every 8 s (the first step after 12 s, while the
window is still opening). At each step it prints the marker
`gui-install: krok N`. A debug build under TCG takes several seconds to
draw the window, so xtask saves the screenshot of step N over QMP only at
the marker of step N+1. The screenshots are in
`target/shots/instalace-gui-<arch>/`: `krok-N.png`, step 8 in the middle
of writing, and `hotovo.png` after completion. The result
is checked the same way as for the text install: boot without the stick
and a check of the disk on the host.

## Where to install

The choices are computed by `installplan::choices` from the disk's GPT
**[implemented]** (host tests `cargo test -p installplan`):

| choice | when | what happens |
|---|---|---|
| Whole disk for MagnumOpus | disk at least 8.5 GiB | new GPT, 512 MiB ESP + our partition to the end; a disk with data requires typing its name |
| Next to another system (free space) | the disk has a foreign partition and contiguous free space of at least 8.5 GiB | our ESP and our partition are added into the free space; foreign partitions and their GPT entries stay byte for byte |
| Shrink the Windows partition | always **greyed out** | "not supported yet - shrink C: in Windows Disk Management" |
| Reinstall the system, keep data | MagnumOpus is already on the disk | new loader and system on our ESP, the partition with data and accounts stays |
| Erase the whole disk and install only MagnumOpus | disk with another system or an old installation | like "whole disk", always with typing the disk name |

Sizes: ESP **512 MiB** (room for A/B slots of network updates), our
partition the rest, at least **8 GiB**; partitions aligned to 1 MiB. A
foreign ESP (e.g. Windows) is **never written** - MagnumOpus always has
its own.

## Protecting foreign partitions

[diskguard](disks.md#risk-and-foreign-disk-protection) stays: a
foreign disk is read-only for the normal system. The installer gets a
special **raw channel** from the disk driver (whole disk, message
`block-raw <sector size> <model>`), but every write is checked by the
**driver**, not by the installer (`diskguard::install::raw_write_allowed`)
**[verified in QEMU]** (`--only instalace`, next-to-Windows run):

- writes to the GPT area (LBA 0-33 and the backup table at the end) are
  allowed,
- writes elsewhere only into a partition owned by the installer: our type
  + name `MagnumOpus`, or ESP type + name `MagnumOpus EFI`,
- the driver keeps the table in memory and re-reads it after every write
  to the GPT area.

A bug in the installer therefore cannot overwrite the contents of a
foreign partition. Rewriting the whole table (the "whole disk" choice) is
the only step that removes foreign partitions, and it requires typing the
disk name.

GPT write order (`diskguard::install::add_partitions`): the backup table
at the end of the disk first, then the primary one. The disk GUID, the
protective MBR and all foreign entries stay; our entries go only into free
slots. An interruption between the two writes leaves at least one valid
copy.

## What gets written

<figure class="diagram">
<svg viewBox="0 0 760 200" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Disk layout after installing next to Windows">
<rect class="box" x="10" y="30" width="50" height="70" rx="4"/><text class="t b" x="35" y="70" text-anchor="middle">GPT</text>
<rect class="box" x="60" y="30" width="110" height="70" rx="4"/><text class="t b" x="115" y="58" text-anchor="middle">Windows ESP</text><text class="m" x="115" y="78" text-anchor="middle">unchanged</text>
<rect class="box" x="170" y="30" width="200" height="70" rx="4"/><text class="t b" x="270" y="58" text-anchor="middle">C: (Windows)</text><text class="m" x="270" y="78" text-anchor="middle">unchanged</text>
<rect class="acc" x="370" y="30" width="120" height="70" rx="4"/><text class="t b" x="430" y="52" text-anchor="middle">MagnumOpus EFI</text><text class="m" x="430" y="70" text-anchor="middle">FAT32, 512 MiB</text><text class="mono m" x="430" y="88" text-anchor="middle">EFI/BOOT, os/</text>
<rect class="ok" x="490" y="30" width="210" height="70" rx="4"/><text class="t b" x="595" y="52" text-anchor="middle">MagnumOpus</text><text class="m" x="595" y="70" text-anchor="middle">our FS, at least 8 GiB</text><text class="mono m" x="595" y="88" text-anchor="middle">/home /system/config /system/log</text>
<rect class="box" x="700" y="30" width="50" height="70" rx="4"/><text class="t b" x="725" y="70" text-anchor="middle">GPT</text>
<text class="m" x="10" y="130">The former free space gets two new GPT entries; the other entries, the MBR and the disk GUID stay.</text>
<text class="m" x="10" y="150">On the ESP: EFI/BOOT/BOOTX64.EFI (BOOTAA64.EFI), os/kernel.elf, os/initfs.img, os/boot.cfg.</text>
<text class="m" x="10" y="170">The installed initfs has neither the developer profile (auto login) nor the test answers file.</text>
</svg>
<figcaption>Disk after installing next to Windows (into free space).</figcaption>
</figure>

Steps of the `installd` service **[verified in QEMU]** (x86_64 and aarch64,
`--only instalace`):

1. partition table (new, or adding into free slots),
2. ESP format (FAT32, `libs/fatw`, label `MAGNUMOS`; 8.3 file names),
3. copy of the loader, kernel and initfs from the live stick's ESP; the
   initfs is the separate file `os/instfs.img` without the
   developer profile,
4. format of our partition (the `fs` library), datasets `/home`,
   `/system/config`, `/system/log`,
5. `/system/config/system.toml`,
6. first account: the same `accounts` library as the account service
   (Argon2id, file `/system/accounts/users/<uid>`), home folder as a
   dataset; init re-applies file permissions at boot,
7. verification: the table again through diskguard, ESP files read back
   and compared, our FS mounted read-only.

## Firmware menu (NVRAM)

The installer writes `nvram = "zapsat"` and the default system
(`vychozi = "magnumopus"` or `"jiny"`) into `\os\boot.cfg` on our ESP. On
the first boot from the internal disk the loader (`loader/src/nvram.rs`)
**[verified in QEMU]** (x86_64 OVMF and aarch64 AAVMF, `--only instalace`):

1. adds nothing if a "MagnumOpus" entry already exists,
2. checks the free space of UEFI variables (`QueryVariableInfo`, at least
   16 KiB) - a full NVRAM can brick a board,
3. adds `Boot####` at the first free number with the path of its own
   loader,
4. puts the number at the start of `BootOrder` (MagnumOpus default) or at
   the end (other system default),
5. rewrites `nvram` in `boot.cfg` to `hotovo` - **a single attempt**, even
   if the write failed (booting via the fallback path `EFI/BOOT` still
   works).

Foreign `Boot####` entries are never deleted or overwritten.

## System settings

`/system/config/system.toml` is written by the installer and read by init
on every boot (`installplan::config`) **[verified in QEMU]** (x86_64 and aarch64):

```toml
jazyk = "cs"
klavesnice = ["cz", "us"]
pasmo = "Europe/Prague"
rtc = "utc"
vychozi-system = "magnumopus"
```

Init passes the time zone and clock to the time service (message
`settings`) and the list of layouts to the input service (control message
`layouts cz us`). Without the file (live USB) defaults apply. When
installing next to another system the wizard asks whether it is Windows;
then it writes `rtc = "local"` (Windows keeps the clock in local time).

## Unattended install

```
install --odpovedi /path/answers.toml
```

The file has the `system.toml` format plus the keys `disk` (model or
number), `zpusob` (`cely-disk`, `volne-misto`, `preinstalovat`),
`potvrzeni`, `jmeno`, `zobrazovane`, `heslo`, `pin`. A password in the
file is for tests and scripts only. Key reference:
`libs/installplan/src/answers.rs`.

## Tests

- Host: `cargo test -p installplan` (choices, plan, preview, settings,
  protocol, answers), `cargo test -p diskguard` (`tests/install.rs`: write
  rule, adding partitions keeps foreign entries), `cargo test -p fatw`
  (FAT32 format and read back).
- Host: `cargo test -p installview` (the wizard window with a fake
  installd: full run, keyboard, narrow window, failure, autopilot).
- QEMU: `cargo xtask test --only instalace-gui` - the graphical wizard,
  see [Graphical wizard](#graphical-wizard).
- QEMU: `cargo xtask test --only instalace` (`xtask/src/usb/install_test.rs`):
  1. **empty disk** - live USB + an empty 12 GiB NVMe disk, install from
     an answers file, then boot **without the stick** from the NVMe only:
     the loader writes the NVRAM entry, fsd mounts our partition, the
     account service loads one account and the compositor shows the login
     screen. The host then checks the GPT, the ESP files, `system.toml` and
     that the account password matches (`accounts::kdf`),
  2. **next to Windows** - a disk with an ESP and a "Windows" partition
     with markers, install into free space; the Windows partitions and the
     MBR stay byte for byte identical.

Logging in with the created account is verified at the accounts library
level on the host, not by clicking in the GUI.

## Limitations

- Disks with 4 KiB sectors are rejected by the installer for now.
- virtio disks have no raw channel (the QEMU test targets NVMe).
- An interrupted installation does not resume where it stopped; only the
  GPT write order is safe. After an interruption choose "whole disk" or
  "reinstall" again.
- No disk encryption and no recovery key yet.
- No shrinking of a foreign (NTFS) partition; free space in the other
  system.
- The installer needs randomness (partition GUIDs, password salt). A
  computer without a generator (aarch64 without TRNG and RNDR, QEMU without
  virtio-rng) waits for entropy; the QEMU test therefore adds
  `virtio-rng-pci` on aarch64.
- The partition chart in the graphical wizard is drawn as text for now;
  the bar chart waits for a `libs/ui` component.
- The design's "try the keyboard" field is missing: the window has no
  right to switch the live layout of the input service.
- The graphical wizard is verified only by the autopilot (filled from the
  answers), not by mouse clicks; mouse and keyboard handling is covered by
  the `libs/ui` tests.
