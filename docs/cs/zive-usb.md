# Živé USB: zkoušení na skutečném počítači

MagnumOpus jde spustit z flashky na skutečném počítači bez instalace.
Systém z flashky nastartuje rovnou do plochy s terminálem (vývojový
profil s automatickým přihlášením, viz varování níže); svá data (a později logy
a záznamy pádů) ukládá na **datový oddíl flashky**. Disky počítače vidí,
ale **jen pro čtení** (viz [Disky](disky.md#riziko-a-ochrana-cizích-disků)).

Návod krok za krokem:

1. [Co je potřeba](#co-je-potřeba)
2. [Vytvoření obrazu](#vytvoření-obrazu)
3. [Zápis na flashku](#zápis-na-flashku) (xtask, Rufus, dd)
4. [Start z flashky](#start-z-flashky)
5. [Diagnostika](#diagnostika): logy, záznamy pádů, balíček pro vývojáře
6. [Co dělat při problému](#co-dělat-při-problému)

## Co je potřeba

- Počítač s **UEFI** (x86_64; aarch64 s UEFI, např. dev kit). Starý BIOS
  bez UEFI nejde.
- **Secure Boot vypnutý** (zavaděč zatím není podepsaný).
- Flashka aspoň **2 GB**. Celý její obsah se smaže.
- Na počítači, kde obraz vytváříte: repozitář a nástroje podle
  [Sestavení](sestaveni.md).

> **Rizika na skutečném počítači – přečtěte si před prvním startem.**
>
> - **Zápis na flashku přepíše celou flashku.** Splést si disk je nejčastější
>   způsob, jak přijít o data. `cargo xtask usb --write` proto zapíše jen na
>   vyjímatelný disk (USB, SD), nikdy na systémový disk a jen po opsání
>   potvrzení. Rufus a `dd` takovou kontrolu nemají.
> - **Disky počítače** (Windows, Linux, zálohy) systém vidí jen pro čtení
>   (`diskguard`). Zapisuje jen na oddíl s naším typem GPT – na flashce.
> - **DMA bez IOMMU.** Většina počítačů má IOMMU (Intel VT-d, AMD-Vi)
>   v nastavení firmwaru vypnutou. Bez ní může chyba v ovladači zařízení
>   (síť, USB, grafika) přepsat libovolnou paměť počítače – systém spadne,
>   v horším případě poškodí data, která právě zapisuje. Disky počítače to
>   neohrozí přímo (zápis na ně odmítá ovladač disku), ale **zapněte VT-d /
>   AMD-Vi (IOMMU) ve firmwaru**, pokud to jde. Bez IOMMU zapíše jádro při
>   startu do logu varování `IOMMU neni: zarizeni smi pres DMA zapisovat
>   kamkoli do pameti`.
> - Systém je ve vývoji: počítejte s pádem. Pád nepoškodí disky počítače
>   (jsou jen pro čtení), nanejvýš datový oddíl flashky.

## Vytvoření obrazu

```bash
cargo xtask usb --release
```

Vznikne `target/usb-x86_64-release.img` (1 GiB). Pro aarch64
`--arch aarch64`, jinou velikost `--size <MiB>`, jinou cestu `--out <soubor>`.
Ladicí build (bez `--release`) je větší a pomalejší, ale má přesnější výpisy
zásobníku.

Velikost ESP se počítá ze souborů, které na ni patří (zavaděč, jádro, živý
a instalační initfs): jejich součet + 25 % + 16 MiB rezervy, zaokrouhleno na
8 MiB, aspoň 64 MiB. Ladicí build aarch64 má tak ESP kolem 180 MiB. Obraz musí
mít aspoň ESP + 130 MiB (logy 64 MiB, data 64 MiB, GPT); menší `--size` xtask
zvětší a vypíše to.

Obsah obrazu:

<figure class="diagram">
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Rozložení obrazu živého USB">
<rect class="box" x="10" y="30" width="70" height="80" rx="4"/><text class="t b" x="45" y="62" text-anchor="middle">GPT</text><text class="m" x="45" y="80" text-anchor="middle">MBR,</text><text class="m" x="45" y="95" text-anchor="middle">tabulka</text>
<rect class="acc" x="80" y="30" width="230" height="80" rx="4"/><text class="t b" x="195" y="55" text-anchor="middle">ESP (FAT32), podle souborů</text><text class="mono m" x="195" y="74" text-anchor="middle">EFI/BOOT/BOOTX64.EFI</text><text class="mono m" x="195" y="89" text-anchor="middle">os/kernel.elf, os/initfs.img</text><text class="mono m" x="195" y="104" text-anchor="middle">CTI-MNE.txt</text>
<rect class="warn" x="310" y="30" width="110" height="80" rx="4"/><text class="t b" x="365" y="55" text-anchor="middle">MO-LOGY</text><text class="m" x="365" y="74" text-anchor="middle">FAT32, 64 MiB</text><text class="m" x="365" y="91" text-anchor="middle">logy pro Windows</text><rect class="ok" x="420" y="30" width="270" height="80" rx="4"/><text class="t b" x="555" y="55" text-anchor="middle">datový oddíl „MagnumOpus“</text><text class="m" x="555" y="74" text-anchor="middle">typ C123ADA0-...-C2CE6B2B3581</text><text class="m" x="555" y="91" text-anchor="middle">náš FS (fsd), do konce obrazu</text>
<rect class="box" x="690" y="30" width="60" height="80" rx="4"/><text class="t b" x="720" y="62" text-anchor="middle">GPT</text><text class="m" x="720" y="80" text-anchor="middle">záloha</text>
<text class="m" x="10" y="20">0</text><text class="m" x="80" y="20">1 MiB</text><text class="m" x="310" y="20">1 MiB + ESP</text><text class="m" x="420" y="20">+ 64 MiB</text><text class="m" x="690" y="20">konec obrazu</text>
<text class="m" x="10" y="140">Firmware najde zavaděč na ESP, zavaděč načte jádro a initfs (soubory přes UEFI).</text>
<text class="m" x="10" y="160">Po startu ovladač usb-storage zpřístupní flashku; diskguard pustí ke čtení i zápisu jen datový oddíl.</text>
<text class="m" x="10" y="180">Init na něm najde souborový systém a spustí fsd. ESP systém za běhu nemění.</text>
<text class="m" x="10" y="200">Windows ukáže ESP a MO-LOGY (logy, záznamy pádů, balíček); datový oddíl nepřečte.</text>
<text class="m" x="10" y="220">Zbytek větší flashky zůstane nevyužitý (zvětšení oddílu zatím není).</text>
</svg>
<figcaption>Rozložení obrazu živého USB (sektor 512 B).</figcaption>
</figure>

## Zápis na flashku

### A. xtask (Windows i Linux, doporučeno)

```bash
cargo xtask usb --list
```

vypíše disky počítače, např.:

```text
disky tohoto pocitace (zapsat jde jen na vyjimatelny, ne systemovy):
  0          Samsung SSD 980                   931.5 GiB  NVMe   SYSTEMOVY  pripojeno: C:
  2          SanDisk Ultra                      28.6 GiB  USB    vyjimatelny  pripojeno: E:
```

Pak zápis (Windows: **terminál spuštěný jako správce**; Linux: `sudo` nebo
právo zápisu do `/dev/sdX`):

```bash
cargo xtask usb --release --write 2
```

Na Linuxu je disk cesta, např. `--write /dev/sdb`. xtask disk zkontroluje
**před** i **po** sestavení obrazu, ukáže model, velikost a sběrnici
a čeká na opsání textu `ANO 2`. Odmítne:

| případ | proč |
|---|---|
| systémový nebo zaváděcí disk (Windows `IsSystem`/`IsBoot`; Linux připojené `/`, `/boot`, `/boot/efi`, `/usr`, `/var`, `/home`, swap – i přes LVM) | zničil by běžící systém |
| disk mimo USB, SD a MMC (a na Linuxu bez příznaku `removable`) | interní disky xtask nepřepisuje (instalace na interní disk přijde jako instalátor v systému) |
| disk menší než obraz | obraz by se nevešel |
| Linux: disk s připojeným oddílem | nejdřív `umount` |

Zápis: Windows nejdřív odstraní oddíly flashky (`Clear-Disk`, jinak Windows
zápis do připojeného svazku zablokuje). Pak xtask smaže poslední MiB
flashky (stará záložní tabulka GPT by zmátla firmware), zapíše obraz po
1 MiB a **první MiB (tabulku oddílů) až nakonec**: přerušený zápis
nenechá flashku, která vypadá jako platný systém.

### B. Rufus (Windows)

1. Spusťte [Rufus](https://rufus.ie), vyberte flashku v poli *Zařízení*.
2. *Výběr spouštění* → *VYBRAT* → soubor `target/usb-x86_64-release.img`.
3. Rufus se zeptá na režim zápisu: zvolte **zápis v režimu obrazu DD**
   (ne ISO).
4. *START*. Pozor: Rufus nekontroluje, jestli jste vybrali správný disk.

Podobně funguje balenaEtcher nebo Win32 Disk Imager.

### C. dd (Linux, macOS)

```bash
lsblk -o NAME,SIZE,MODEL,TRAN,MOUNTPOINTS   # najděte flashku, např. sdb
sudo umount /dev/sdb?*                       # odpojte její oddíly
sudo dd if=target/usb-x86_64-release.img of=/dev/sdb bs=4M conv=fsync status=progress
```

`of=` musí být **celý disk** (`/dev/sdb`), ne oddíl (`/dev/sdb1`). Chyba
v písmenu zničí jiný disk. Na macOS je disk `/dev/rdiskN` (`diskutil list`,
`diskutil unmountDisk`).

## Start z flashky

1. Zasuňte flashku a zapněte počítač.
2. Otevřete **nabídku zavádění** firmwaru (podle výrobce F12, F11, F8, F10
   nebo Esc; na Macu Option). Vyberte flashku s označením **UEFI**.
3. Když se nic nestane: ve firmwaru vypněte **Secure Boot** (a CSM /
   Legacy, je-li zapnutý jen legacy start z USB).
4. Systém nastartuje do plochy s terminálem. Klávesnice a myš USB fungují
   hned (i PS/2 na x86). Síť funguje na kartách Intel (e1000e, igc) a Realtek (r8169)
   – viz [Síťové karty](sitove-karty.md).

⚠️ **Živé USB = vývojový profil** **[implementováno]**: obraz flashky
(`cargo xtask usb` i `cargo xtask usb --release`) má soubor `accounts-dev`,
takže služba [účtů](ucty.md#automatické-přihlášení-jen-vývoj) založí účet
`vyvojar` **bez hesla** a hned ho přihlásí; start skončí v ploše
s terminálem. **Datový oddíl na flashce není šifrovaný:** kdo má flashku,
přečte ho. Flashka slouží ke zkoušení hardwaru a VMware, ne pro osobní
data. `xtask usb` a `xtask vmware` (disk VM je tentýž obraz) to vypíší
jako varování. Obraz pro uživatele (`cargo xtask image`) automatické
přihlášení nemá; bez účtu ukáže přihlašovací obrazovka zprávu „účet se
zakládá v instalátoru“ (instalátor, `design/instalace.html`,
**[navrženo]**).

Ověřené v QEMU (`cargo xtask usb --run` s oknem, `cargo xtask usb --test`
bez okna; obojí stroj co nejbližší skutečnému PC: flashka na řadiči xHCI,
klávesnice a tablet USB, karta Intel, žádné virtio, bez IOMMU). Na
skutečném hardwaru zatím ověřeno není – proto tato příručka a diagnostika.

## Diagnostika

Systém sám ukládá všechno, co vývojář potřebuje:

| co | kde (datový oddíl) | kdo |
|---|---|---|
| log jádra a všech služeb, každý start zvlášť | `/system/log/boot-000042.log` | služba `logd` (posledních 20 startů, každý nejvýš 32 MiB) |
| záznam pádu jádra | `/system/pady/000041-jadro-MO-7F3A-21C9.crash` | jádro do vyhrazené RAM, `logd` při dalším startu na disk |
| pád služby (výjimka CPU, panika) | `/system/pady/000042-fsd-1.txt` | supervisor initu, `logd` |
| informace o stroji | v logu startu: řádky `stroj:` (CPU, mapa paměti, GOP), `acpi:` (tabulky), `devmgr: pci` (zařízení PCI), ovladače disků, sítě a USB | jádro, služby |
| balíček pro vývojáře | `/system/log/report-000042-1.txt` | příkaz `report` |

<figure class="diagram">
<svg viewBox="0 0 760 230" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Cesta logu a záznamu pádu">
<defs><marker id="zu1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="150" height="70" rx="6"/><text class="t b" x="85" y="44" text-anchor="middle">jádro a služby</text><text class="m" x="85" y="62" text-anchor="middle">log -&gt; kruhový buffer</text><text class="m" x="85" y="78" text-anchor="middle">512 KiB v jádře</text>
<rect class="acc" x="230" y="20" width="160" height="70" rx="6"/><text class="t b" x="310" y="44" text-anchor="middle">logd</text><text class="m" x="310" y="62" text-anchor="middle">LOG_READ každou 1 s</text><text class="m" x="310" y="78" text-anchor="middle">fronta v RAM (4 MiB)</text>
<rect class="ok" x="460" y="20" width="160" height="70" rx="6"/><text class="t b" x="540" y="44" text-anchor="middle">datový oddíl</text><text class="m" x="540" y="62" text-anchor="middle">/system/log</text><text class="m" x="540" y="78" text-anchor="middle">/system/pady</text>
<path class="ln" d="M160 55 L228 55" marker-end="url(#zu1)"/><path class="ln" d="M390 55 L458 55" marker-end="url(#zu1)"/>
<rect class="warn" x="10" y="130" width="150" height="70" rx="6"/><text class="t b" x="85" y="154" text-anchor="middle">pád jádra</text><text class="m" x="85" y="172" text-anchor="middle">záznam do RAM (pstore)</text><text class="m" x="85" y="188" text-anchor="middle">obrazovka, restart 60 s</text>
<rect class="box" x="230" y="130" width="160" height="70" rx="6"/><text class="t b" x="310" y="154" text-anchor="middle">další start</text><text class="m" x="310" y="172" text-anchor="middle">CRASH_RECORD READ</text><text class="m" x="310" y="188" text-anchor="middle">uložit, pak CLEAR</text>
<path class="ln" d="M160 165 L228 165" marker-end="url(#zu1)"/><path class="ln" d="M390 165 L540 92" marker-end="url(#zu1)"/>
<text class="m" x="640" y="150">Když FS nejde (pád fsd),</text><text class="m" x="640" y="166">logd drží log v RAM</text><text class="m" x="640" y="182">a zapíše ho později.</text>
</svg>
<figcaption>Cesta logu a záznamu pádu na datový oddíl.</figcaption>
</figure>

**Pád jádra.** Obrazovka ukáže kód (např. `KERNEL_PANIC · #MO-7F3A-21C9`),
kód QR se souhrnem, konec logu s registry a zásobníkem a po 60 s počítač
restartuje. Záznam leží ve vyhrazené RAM (64 KiB na pevné adrese), kterou
teplý restart nesmaže; při dalším startu ho `logd` zapíše do `/system/pady`.
**Vyfoťte obrazovku i QR telefonem** - po vypnutí napájení (ne restartu)
záznam v RAM zmizí. Počítač s nativním ovladačem grafiky (Intel, virtio-gpu)
už obraz firmwaru nemusí ukazovat; záznam se pak uloží jen do RAM a na
sériovou linku.

**Balíček pro vývojáře.** V terminálu napište `report`. Vypíše cestu
a náhled obsahu:

```text
balicek pro vyvojare: /system/log/report-000042-1.txt
  zaznam padu 000041-jadro-MO-7F3A-21C9.crash (21044 B)
  log tohoto bootu boot-000042.log (180312 B)
  konec logu minuleho bootu (262144 B)
  celkem 463900 B
```

Balíček obsahuje jen logy systému a záznamy pádů (jména procesů a zařízení,
adresy, hlášení služeb). Obsah vašich souborů ani hesla do logu nejdou.
Na flashce je kopie i na oddílu MO-LOGY (`MAGNUMOS\REPORT.TXT`), který
Windows přečte bez dalších nástrojů.

**Pro vývojáře:** adresy v záznamu pádu i v balíčku přeloží na jména funkcí
`cargo xtask symbolize <soubor>` (se stejným `--arch` a `--release` jako
build, ze kterého systém běžel).

**Kopie na flashce pro Windows.** Při běhu z flashky služba logu kopíruje
logy, záznamy pádů a balíček i na malý oddíl **MO-LOGY** (FAT32), který
Windows ukáže jako běžný disk: složka `MAGNUMOS` – `LOGxx.TXT` (log startu,
posledních 16 startů dokola, každý nejvýš 2 MiB, kopie každých 30 s),
`PADxx.TXT` (záznamy pádů), `REPORT.TXT` (poslední balíček). **Vývojáři
stačí poslat `REPORT.TXT`** (nebo celou složku). Systém do oddílu zapisuje
jen do své složky (vytvoření a přepsání souborů, žádné mazání) a jen na
flashce s naším oddílem: cizí FAT oddíl jménem MO-LOGY na jiném disku
zůstane jen pro čtení.

**Režim „vytáhni logy“** – když nainstalovaný systém na tomto počítači
nenastartuje (nebo jen chcete jeho logy):

1. Zasuňte flashku a nabootujte z ní (nabídka zavádění firmwaru).
2. Když zavaděč napíše „Stiskni L = vytáhnout logy…“, stiskněte do 2 s **L**.
3. Systém nespustí grafiku: najde disky s naším souborovým systémem mimo
   flashku, připojí je **jen pro čtení** (zápis odmítne i blokové zařízení),
   zkopíruje `/system/log` a `/system/pady` do složky `ZACHRANA` na oddíle
   MO-LOGY a napíše **HOTOVO** a počet souborů.
4. Vypněte počítač a flashku zasuňte do počítače s Windows: `ZACHRANA\SEZNAM.TXT`
   říká, který soubor je který.

**Úrovně logu a zahlcení.** Jádro z jednoho místa (soubor a řádek, u služeb
proces a text bez číslic) zapíše nejvýš 10 záznamů za sekundu; další jen
spočítá a pak zapíše „… předchozí hláška potlačena N×“. Služba logu navíc
stejné řádky za sebou (bez času) zapíše nejvýš třikrát a pak
„… předchozí řádek opakován ještě N×“. Když přesto přeteče kruhový buffer
jádra (512 KiB), objeví se v logu „ztraceno N řádků“. Úrovně za běhu:

```text
loglevel                      # výpis přepsání a počtu potlačených hlášek
loglevel virtio-blk warn      # služba: jen varování a chyby
loglevel sched debug          # modul jádra: i ladicí hlášky
loglevel sched vychozi        # zrušit přepsání
```

Trvale v souboru `/system/log/urovne.conf` (`modul = úroveň`, služba logu
ho načte při startu). Výchozí potlačené sekce (jen `info` a výš):
`syscall`, `object::process`, `smp::clock_sync`.

## Co dělat při problému

| příznak | co zkusit |
|---|---|
| flashka v nabídce zavádění chybí | jiný port USB (USB 2 místo USB 3), zapnout podporu startu z USB ve firmwaru, ověřit zápis (`--write` znovu) |
| hned po výběru se vrátí do nabídky | Secure Boot vypnout; počítač bez UEFI (jen BIOS) nepodporujeme |
| černá obrazovka | počkat 30 s (start ovladačů USB a disků); jinak fotka obrazovky a sériová konzole (viz níže) |
| systém spadne | vyfotit obrazovku (kód a výpis); více v [Hlášky v logu](hlasky.md) |

**Sériová konzole.** Na x86_64 jde log jádra a služeb od startu na
sériovou linku COM1 (port 0x3F8, 115200 Bd, 8N1). Počítač s konektorem COM
(nebo deskou s hlavičkou COM) připojte nulmodemovým kabelem k jinému
počítači a log zachyťte (PuTTY, `picocom -b 115200 /dev/ttyUSB0`).
Převodník USB-sériová linka (CDC-ACM, FTDI) v počítači s MagnumOpus zatím
nefunguje: log z jádra by šel ven až po startu ovladačů USB, tedy právě ne
při pádech v začátku startu. Přidá se později jako zrcadlo logu ze služby
logu. Záloha záznamu pádu do proměnné UEFI (přežije i vypnutí) také zatím
není: jádro by muselo volat běhové služby UEFI z obsluhy pádu.

## Pro vývojáře

- Disk USB obsluhuje ovladač `usb-storage` (SCSI přes Bulk-Only
  Transport), viz [USB](usb.md#disky-usb-usb-storage).
- `cargo xtask test` spouští po hlavním běhu i start z USB;
  `--only zive-usb` jen tento běh. Testy FS nad flashkou:
  `cargo xtask test --data-disk usb --only "souborovy system,data FS"`.
- Instalace z flashky na interní disk: [Instalace](instalace.md)
  (`--only instalace`).
- Zdrojáky: `xtask/src/usb/` (obraz, výpis a zápis disků, start v QEMU),
  `user/usb-storage/`, `libs/usbproto/src/storage.rs`.
