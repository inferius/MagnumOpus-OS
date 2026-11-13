# Live USB: trying the system on a real computer

MagnumOpus can run from a USB stick on a real computer without
installation. The system boots from the stick straight into the desktop
with a terminal (a development profile with automatic sign-in, see the
warning below); it stores its data (and later logs and crash records) on the
**stick's data partition**. It sees the computer's disks, but **read-only**
(see [Disks](disks.md#risk-and-foreign-disk-protection)).

Step-by-step guide:

1. [What you need](#what-you-need)
2. [Creating the image](#creating-the-image)
3. [Writing the stick](#writing-the-stick) (xtask, Rufus, dd)
4. [Booting from the stick](#booting-from-the-stick)
5. [Diagnostics](#diagnostics): logs, crash records, the developer bundle
6. [When something goes wrong](#when-something-goes-wrong)

## What you need

- A computer with **UEFI** (x86_64; aarch64 with UEFI, e.g. a dev kit). An
  old BIOS without UEFI does not work.
- **Secure Boot off** (the loader is not signed yet).
- A USB stick of at least **2 GB**. All of its contents are erased.
- On the computer where you create the image: the repository and tools
  as in [Building](building.md).

> **Risks on a real computer - read before the first boot.**
>
> - **Writing the stick overwrites the whole stick.** Picking the wrong
>   disk is the most common way to lose data. `cargo xtask usb --write`
>   therefore writes only to a removable disk (USB, SD), never to the
>   system disk, and only after you type a confirmation. Rufus and `dd`
>   have no such check.
> - The system sees the **computer's disks** (Windows, Linux, backups)
>   read-only (`diskguard`). It writes only to a partition with our GPT
>   type - on the stick.
> - **DMA without an IOMMU.** Most computers have the IOMMU (Intel VT-d,
>   AMD-Vi) turned off in the firmware settings. Without it, a bug in a
>   device driver (network, USB, graphics) can overwrite any memory of the
>   computer - the system crashes, in the worse case it corrupts data it is
>   writing at that moment. The computer's disks are not directly at risk
>   (the disk driver refuses writes to them), but **turn on VT-d / AMD-Vi
>   (IOMMU) in the firmware** if you can. Without an IOMMU the kernel logs
>   the warning `IOMMU neni: zarizeni smi pres DMA zapisovat kamkoli do
>   pameti` at boot.
> - The system is under development: expect crashes. A crash does not
>   damage the computer's disks (they are read-only), at most the stick's
>   data partition.

## Creating the image

```bash
cargo xtask usb --release
```

This creates `target/usb-x86_64-release.img` (1 GiB). For aarch64 use
`--arch aarch64`, a different size `--size <MiB>` (at least 258 MiB), a
different path `--out <file>`. A debug build (without `--release`) is
larger and slower but has more precise stack traces.

Image contents:

<figure class="diagram">
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Layout of the live USB image">
<rect class="box" x="10" y="30" width="70" height="80" rx="4"/><text class="t b" x="45" y="62" text-anchor="middle">GPT</text><text class="m" x="45" y="80" text-anchor="middle">MBR,</text><text class="m" x="45" y="95" text-anchor="middle">table</text>
<rect class="acc" x="80" y="30" width="230" height="80" rx="4"/><text class="t b" x="195" y="55" text-anchor="middle">ESP (FAT32), 128 MiB</text><text class="mono m" x="195" y="74" text-anchor="middle">EFI/BOOT/BOOTX64.EFI</text><text class="mono m" x="195" y="89" text-anchor="middle">os/kernel.elf, os/initfs.img</text><text class="mono m" x="195" y="104" text-anchor="middle">CTI-MNE.txt</text>
<rect class="warn" x="310" y="30" width="110" height="80" rx="4"/><text class="t b" x="365" y="55" text-anchor="middle">MO-LOGY</text><text class="m" x="365" y="74" text-anchor="middle">FAT32, 64 MiB</text><text class="m" x="365" y="91" text-anchor="middle">logs for Windows</text><rect class="ok" x="420" y="30" width="270" height="80" rx="4"/><text class="t b" x="555" y="55" text-anchor="middle">data partition "MagnumOpus"</text><text class="m" x="555" y="74" text-anchor="middle">type C123ADA0-...-C2CE6B2B3581</text><text class="m" x="555" y="91" text-anchor="middle">our FS (fsd), to the end of the image</text>
<rect class="box" x="690" y="30" width="60" height="80" rx="4"/><text class="t b" x="720" y="62" text-anchor="middle">GPT</text><text class="m" x="720" y="80" text-anchor="middle">backup</text>
<text class="m" x="10" y="20">0</text><text class="m" x="80" y="20">1 MiB</text><text class="m" x="310" y="20">129 MiB</text><text class="m" x="420" y="20">193 MiB</text><text class="m" x="690" y="20">end of image</text>
<text class="m" x="10" y="140">The firmware finds the loader on the ESP; the loader loads the kernel and initfs (files through UEFI).</text>
<text class="m" x="10" y="160">After boot the usb-storage driver exposes the stick; diskguard allows reads and writes only to the data partition.</text>
<text class="m" x="10" y="180">Init finds the file system there and starts fsd. The system does not change the ESP while running.</text>
<text class="m" x="10" y="200">Windows shows the ESP and MO-LOGY (logs, crash records, bundle); it cannot read the data partition.</text>
<text class="m" x="10" y="220">The rest of a larger stick stays unused (growing the partition is not there yet).</text>
</svg>
<figcaption>Layout of the live USB image (512 B sectors).</figcaption>
</figure>

## Writing the stick

### A. xtask (Windows and Linux, recommended)

```bash
cargo xtask usb --list
```

lists the computer's disks, for example:

```text
disky tohoto pocitace (zapsat jde jen na vyjimatelny, ne systemovy):
  0          Samsung SSD 980                   931.5 GiB  NVMe   SYSTEMOVY  pripojeno: C:
  2          SanDisk Ultra                      28.6 GiB  USB    vyjimatelny  pripojeno: E:
```

(`SYSTEMOVY` = system disk, `vyjimatelny` = removable, `pripojeno` =
mounted.) Then write (Windows: **a terminal started as administrator**;
Linux: `sudo` or write access to `/dev/sdX`):

```bash
cargo xtask usb --release --write 2
```

On Linux the disk is a path, e.g. `--write /dev/sdb`. xtask checks the disk
**before** and **after** building the image, shows the model, size and bus
and waits for you to type `ANO 2` ("ANO" = yes). It refuses:

| case | why |
|---|---|
| the system or boot disk (Windows `IsSystem`/`IsBoot`; Linux mounted `/`, `/boot`, `/boot/efi`, `/usr`, `/var`, `/home`, swap - also through LVM) | it would destroy the running system |
| a disk outside USB, SD and MMC (and on Linux without the `removable` flag) | xtask does not overwrite internal disks (installing to an internal disk will come as an installer in the system) |
| a disk smaller than the image | the image would not fit |
| Linux: a disk with a mounted partition | `umount` first |

The write: on Windows the stick's partitions are removed first
(`Clear-Disk`, otherwise Windows blocks writes into a mounted volume).
Then xtask clears the stick's last MiB (an old backup GPT would confuse
the firmware), writes the image in 1 MiB chunks and **the first MiB (the
partition table) last**: an interrupted write does not leave a stick that
looks like a valid system.

### B. Rufus (Windows)

1. Start [Rufus](https://rufus.ie), select the stick under *Device*.
2. *Boot selection* -> *SELECT* -> the file `target/usb-x86_64-release.img`.
3. Rufus asks for the write mode: choose **write in DD image mode** (not
   ISO).
4. *START*. Careful: Rufus does not check that you picked the right disk.

balenaEtcher or Win32 Disk Imager work similarly.

### C. dd (Linux, macOS)

```bash
lsblk -o NAME,SIZE,MODEL,TRAN,MOUNTPOINTS   # find the stick, e.g. sdb
sudo umount /dev/sdb?*                       # unmount its partitions
sudo dd if=target/usb-x86_64-release.img of=/dev/sdb bs=4M conv=fsync status=progress
```

`of=` must be the **whole disk** (`/dev/sdb`), not a partition
(`/dev/sdb1`). A wrong letter destroys another disk. On macOS the disk is
`/dev/rdiskN` (`diskutil list`, `diskutil unmountDisk`).

## Booting from the stick

1. Plug in the stick and turn on the computer.
2. Open the firmware **boot menu** (depending on the vendor F12, F11, F8,
   F10 or Esc; on a Mac Option). Pick the stick marked **UEFI**.
3. If nothing happens: turn off **Secure Boot** in the firmware (and CSM /
   Legacy if only legacy USB boot is enabled).
4. The system boots into the desktop with a terminal. USB keyboard and mouse
   work right away (PS/2 too on x86). The network works on Intel (e1000e,
   igc) and Realtek (r8169) cards - see [Network cards](network-cards.md).

⚠️ **Live USB = development profile** **[implemented]**: the stick image
(`cargo xtask usb` and `cargo xtask usb --release`) contains the
`accounts-dev` file, so the [account](accounts.md#automatic-sign-in-development-only)
service creates the `vyvojar` account **without a password** and signs it
in right away; the boot ends in the desktop with a terminal. **The data
partition on the stick is not encrypted:** whoever has the stick can read
it. The stick is for trying hardware and VMware, not for personal data.
`xtask usb` and `xtask vmware` (the VM disk is the same image) print this
as a warning. The image for users (`cargo xtask image`) has no automatic
sign-in; without an account the sign-in screen says that accounts are
created in the installer (`design/instalace.html`, **[designed]**).

Verified in QEMU (`cargo xtask usb --run` with a window, `cargo xtask usb
--test` without one; both use a machine as close to a real PC as possible:
the stick on an xHCI controller, USB keyboard and tablet, an Intel card, no
virtio, no IOMMU). Not verified on real hardware yet - hence this guide and
the diagnostics.

## Diagnostics

The system itself stores everything a developer needs:

| what | where (data partition) | who |
|---|---|---|
| kernel and service log, each boot separately | `/system/log/boot-000042.log` | the `logd` service (last 20 boots, each at most 32 MiB) |
| kernel crash record | `/system/pady/000041-jadro-MO-7F3A-21C9.crash` | the kernel into reserved RAM, `logd` to disk on the next boot |
| service crash (CPU exception, panic) | `/system/pady/000042-fsd-1.txt` | init's supervisor, `logd` |
| machine information | in the boot log: `stroj:` lines (CPU, memory map, GOP), `acpi:` (tables), `devmgr: pci` (PCI devices), disk, network and USB drivers | kernel, services |
| developer bundle | `/system/log/report-000042-1.txt` | the `report` command |

(`pady` = crashes, `jadro` = kernel, `stroj` = machine.)

<figure class="diagram">
<svg viewBox="0 0 760 230" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Path of the log and the crash record">
<defs><marker id="lu1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="150" height="70" rx="6"/><text class="t b" x="85" y="44" text-anchor="middle">kernel and services</text><text class="m" x="85" y="62" text-anchor="middle">log -&gt; ring buffer</text><text class="m" x="85" y="78" text-anchor="middle">512 KiB in the kernel</text>
<rect class="acc" x="230" y="20" width="160" height="70" rx="6"/><text class="t b" x="310" y="44" text-anchor="middle">logd</text><text class="m" x="310" y="62" text-anchor="middle">LOG_READ every 1 s</text><text class="m" x="310" y="78" text-anchor="middle">queue in RAM (4 MiB)</text>
<rect class="ok" x="460" y="20" width="160" height="70" rx="6"/><text class="t b" x="540" y="44" text-anchor="middle">data partition</text><text class="m" x="540" y="62" text-anchor="middle">/system/log</text><text class="m" x="540" y="78" text-anchor="middle">/system/pady</text>
<path class="ln" d="M160 55 L228 55" marker-end="url(#lu1)"/><path class="ln" d="M390 55 L458 55" marker-end="url(#lu1)"/>
<rect class="warn" x="10" y="130" width="150" height="70" rx="6"/><text class="t b" x="85" y="154" text-anchor="middle">kernel crash</text><text class="m" x="85" y="172" text-anchor="middle">record into RAM (pstore)</text><text class="m" x="85" y="188" text-anchor="middle">screen, reboot in 60 s</text>
<rect class="box" x="230" y="130" width="160" height="70" rx="6"/><text class="t b" x="310" y="154" text-anchor="middle">next boot</text><text class="m" x="310" y="172" text-anchor="middle">CRASH_RECORD READ</text><text class="m" x="310" y="188" text-anchor="middle">save, then CLEAR</text>
<path class="ln" d="M160 165 L228 165" marker-end="url(#lu1)"/><path class="ln" d="M390 165 L540 92" marker-end="url(#lu1)"/>
<text class="m" x="640" y="150">When the FS is down (fsd</text><text class="m" x="640" y="166">crashed), logd keeps the</text><text class="m" x="640" y="182">log in RAM and writes it later.</text>
</svg>
<figcaption>Path of the log and the crash record to the data partition.</figcaption>
</figure>

**Kernel crash.** The screen shows a code (e.g. `KERNEL_PANIC ·
#MO-7F3A-21C9`), a QR code with a summary, the end of the log with
registers and the stack, and reboots after 60 s. The record sits in
reserved RAM (64 KiB at a fixed address) that a warm reset does not erase;
on the next boot `logd` writes it to `/system/pady`. **Photograph the
screen and the QR code with a phone** - after a power-off (not a reboot)
the record in RAM is gone. A computer with a native graphics driver
(Intel, virtio-gpu) may no longer show the firmware image; the record is
then stored only in RAM and on the serial line.

**Developer bundle.** Type `report` in the terminal. It prints the path
and a preview of the contents:

```text
balicek pro vyvojare: /system/log/report-000042-1.txt
  zaznam padu 000041-jadro-MO-7F3A-21C9.crash (21044 B)
  log tohoto bootu boot-000042.log (180312 B)
  konec logu minuleho bootu (262144 B)
  celkem 463900 B
```

The bundle contains only system logs and crash records (process and device
names, addresses, service messages). Your file contents and passwords do
not go into the log. On the stick there is also a copy on the MO-LOGY partition
(`MAGNUMOS\REPORT.TXT`) that Windows reads without any extra tools.

**For developers:** `cargo xtask symbolize <file>` translates addresses in
the crash record and in the bundle to function names (with the same
`--arch` and `--release` as the build the system ran).

**Copy on the stick for Windows.** When running from the stick, the log
service also copies logs, crash records and the bundle to a small
**MO-LOGY** partition (FAT32) that Windows shows as an ordinary drive:
folder `MAGNUMOS` - `LOGxx.TXT` (boot log, the last 16 boots in rotation,
each at most 2 MiB, copied every 30 s), `PADxx.TXT` (crash records),
`REPORT.TXT` (the latest bundle). **Sending `REPORT.TXT` (or the whole
folder) to the developer is enough.** The system writes only into its own
folder (creating and overwriting files, no deleting) and only on a stick
with our partition: a foreign FAT partition named MO-LOGY on another disk
stays read-only.

**"Pull logs" mode** - when the installed system on this computer does not
boot (or you just want its logs):

1. Plug in the stick and boot from it (firmware boot menu).
2. When the loader prints "Stiskni L = vytahnout logy..." ("press L to pull
   logs"), press **L** within 2 s.
3. The system does not start the graphics: it finds disks with our file
   system other than the stick, mounts them **read-only** (the block device
   refuses writes too), copies `/system/log` and `/system/pady` into the
   folder `ZACHRANA` on the MO-LOGY partition and prints **HOTOVO** (done)
   and the number of files.
4. Turn the computer off and plug the stick into a Windows computer:
   `ZACHRANA\SEZNAM.TXT` says which file is which.

**Log levels and flooding.** The kernel writes at most 10 records per
second from one place (file and line, for services the process and the
text without digits); further ones are only counted, followed by "...
previous message suppressed N times". The log service also writes identical
consecutive lines (without the time) at most three times, then "...
previous line repeated N more times". If the kernel ring buffer (512 KiB)
overflows anyway, the log shows "N lines lost". Levels at run time:

```text
loglevel                      # list overrides and the suppressed count
loglevel virtio-blk warn      # a service: only warnings and errors
loglevel sched debug          # a kernel module: debug messages too
loglevel sched vychozi        # remove the override ("vychozi" = default)
```

Permanently in the file `/system/log/urovne.conf` (`module = level`, the log
service reads it at boot). Default suppressed sections (only `info` and
above): `syscall`, `object::process`, `smp::clock_sync`.

## When something goes wrong

| symptom | what to try |
|---|---|
| the stick is missing from the boot menu | another USB port (USB 2 instead of USB 3), enable USB boot in the firmware, check the write (`--write` again) |
| it returns to the menu right after selecting it | turn Secure Boot off; computers without UEFI (BIOS only) are not supported |
| black screen | wait 30 s (USB and disk drivers start); otherwise take a photo of the screen and use the serial console (below) |
| the system crashes | photograph the screen (code and trace); more in [Log messages](messages.md) |

**Serial console.** On x86_64 the kernel and service log goes from boot to
the serial port COM1 (port 0x3F8, 115200 Bd, 8N1). Connect a computer with
a COM connector (or a board with a COM header) to another computer with a
null-modem cable and capture the log (PuTTY,
`picocom -b 115200 /dev/ttyUSB0`). A USB-serial adapter (CDC-ACM, FTDI) in
the computer running MagnumOpus does not work yet: the kernel log could go
out only after the USB drivers start, i.e. not for crashes early in boot.
It will come later as a log mirror from the log service. A backup of the
crash record in a UEFI variable (it would survive a power-off) is not there
yet either: the kernel would have to call UEFI runtime services from the
crash handler.

## For developers

- The USB disk is handled by the `usb-storage` driver (SCSI over Bulk-Only
  Transport), see [USB](usb.md#usb-disks-usb-storage).
- `cargo xtask test` also runs the USB boot after the main run;
  `--only zive-usb` runs only that. FS tests on the stick:
  `cargo xtask test --data-disk usb --only "souborovy system,data FS"`.
- Installing from the stick to an internal disk: [Installation](installation.md)
  (`--only instalace`).
- Sources: `xtask/src/usb/` (image, listing and writing disks, boot in
  QEMU), `user/usb-storage/`, `libs/usbproto/src/storage.rs`.
