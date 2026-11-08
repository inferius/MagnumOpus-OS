# Síťové karty

Ovladače karet běžných počítačů a VMware vedle virtio-net. Všechny jsou
vlastní kód v Rustu.

**Původ:** z ovladačů Linuxu (GPL-2) a z datasheetů výrobců jsou
převzatá jen fakta: ID zařízení, offsety a bity registrů, pořadí
inicializace a chyby čipů. Žádný kód z nich přeložený není, takže kód
není odvozené dílo pod GPL. Kód z FreeBSD/OpenBSD (`if_em`, `if_igc`,
`if_re`, licence BSD/ISC) převzít smíme, ale zatím převzatý není. Až
bude, jeho původ a licence se uvedou v hlavičce souboru a v souboru
licencí třetích stran.

| Program | Karty | Ověřeno |
|---|---|---|
| `e1000e` | Intel 82574L/LA, 82583V, I217, I218, I219 (čipová sada PCH od Lynx Point) | 82574L v QEMU (`-device e1000e`) na x86_64 i aarch64, s IOMMU i bez ní; host testy nad simulací 82574 a I219 |
| `igc` | Intel I225, I226 (2,5 Gb/s), I220-V, I221-V | **jen host testy** nad simulovanými registry |
| `r8169` | Realtek RTL8111/8168 (ID `10EC:8168`, `8161`, `8162`) | **jen host testy** nad simulovanými registry |

⚠️ **Na skutečném hardwaru zatím neověřeno.** `igc` ani `r8169` QEMU
neumí: QEMU 11.1 nemá model I225 (`-device igb` je 82576, jiný čip)
a místo RTL8168 má jen starý `rtl8139`. Simulace v testech vychází
z dokumentace a ovladačů Linuxu, ne z chování skutečného křemíku. Na
skutečné kartě mohou chybět opravy, které Linux dělá pro jednotlivé
revize:

- **I217–I219:** opravy PHY (K1, jumbo rámce), vypnutí ULP přes firmware
  ME a vyprázdnění kruhů po zaseknutí (`FLUSH_DESC_REQUIRED` u I219).
  Ovladač umí jen probudit PHY přepnutím LANPHYPC. Když PHY neodpovídá
  a firmware ME blokuje jeho reset, jen to ohlásí.
- **I225/I226:** nastavení PHY a EEE. Ovladač spoléhá na výchozí stav
  PHY po resetu, který nabízí všechny rychlosti až do 2,5 Gb/s.
- **RTL8168:** nastavení PHY a firmware (`rtl_fw`) pro desítky revizí
  čipu, ASPM a EEE.

Wi-Fi zatím není.

| Část | Kde | Co dělá |
|---|---|---|
| logika čipů | `libs/nic/src/e1000e.rs`, `igc.rs`, `r8169.rs` | reset, MAC, kruhy, přerušení, linka nad rozhraním `Hw` |
| ID zařízení | `libs/nic/src/ids.rs` | tabulka vendor/device ID → program; čte ji i `devmgr` |
| smyčka ovladače | `libs/nic/src/driver.rs` (feature `libos`) | startovní zpráva, registry a DMA, přerušení, protokol `netproto::link` |
| programy | `user/e1000e`, `user/igc`, `user/r8169` | jen `main` volající `nic::driver::run` |
| přerušení | `libs/pci/src/irq.rs`, `msi.rs` | jedno přerušení přes MSI, nebo MSI-X, když MSI chybí |
| správce zařízení | `user/devmgr/src/nic.rs` | najde kartu podle ID, změří okno registrů, spustí ovladač |
| testy | `libs/nic/src/tests/`, `user/init/src/tests/nic.rs` | simulace registrů a DMA; v QEMU druhá karta e1000e |

## Architektura

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Ovladac sitove karty">
<defs><marker id="nk1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="170" height="70" rx="6"/><text class="t b" x="95" y="45" text-anchor="middle">devmgr</text><text class="m" x="95" y="64" text-anchor="middle">ID podle nic::ids</text><text class="m" x="95" y="80" text-anchor="middle">změří BAR registrů</text>
<path class="ln" d="M180 55 L248 55" marker-end="url(#nk1)"/><text class="m" x="186" y="47">spustí</text>
<rect class="acc" x="250" y="20" width="270" height="180" rx="6"/><text class="t b" x="385" y="45" text-anchor="middle">proces e1000e / igc / r8169</text>
<rect class="box" x="265" y="60" width="240" height="56" rx="4"/><text class="t" x="385" y="82" text-anchor="middle">nic::driver::run</text><text class="m" x="385" y="100" text-anchor="middle">smyčka: přerušení, kanál, linka 1 s</text>
<rect class="box" x="265" y="128" width="240" height="56" rx="4"/><text class="t" x="385" y="150" text-anchor="middle">logika čipu (trait Nic)</text><text class="m" x="385" y="168" text-anchor="middle">nad rozhraním Hw</text>
<path class="ln" d="M385 116 L385 126" marker-end="url(#nk1)"/>
<rect class="ok" x="590" y="70" width="160" height="70" rx="6"/><text class="t b" x="670" y="97" text-anchor="middle">netd</text><text class="m" x="670" y="116" text-anchor="middle">první karta = rozhraní</text>
<path class="ln" d="M520 95 L588 95" marker-end="url(#nk1)"/><path class="ln" d="M588 110 L522 110" marker-end="url(#nk1)"/><text class="m" x="526" y="88">INFO, LINK</text><text class="m" x="530" y="126">FRAME</text>
<rect class="warn" x="250" y="240" width="270" height="70" rx="6"/><text class="t b" x="385" y="263" text-anchor="middle">karta</text><text class="m" x="385" y="282" text-anchor="middle">registry (BAR) · kruhy RX/TX v DMA</text><text class="m" x="385" y="298" text-anchor="middle">MSI (MSI-X) · DMA v doméně IOMMU</text>
<path class="ln" d="M330 200 L330 238" marker-end="url(#nk1)"/><text class="m" x="200" y="224">MMIO, deskriptory</text>
<path class="ln" d="M440 238 L440 202" marker-end="url(#nk1)"/><text class="m" x="448" y="224">přerušení</text>
<rect class="box" x="10" y="240" width="200" height="70" rx="6"/><text class="t b" x="110" y="263" text-anchor="middle">testy na hostiteli</text><text class="m" x="110" y="282" text-anchor="middle">simulace registrů a DMA</text><text class="m" x="110" y="298" text-anchor="middle">místo Hw z libos</text>
<path class="ln dash" d="M210 270 L263 175" marker-end="url(#nk1)"/>
</svg>
<figcaption>Jeden proces na kartu. Logika čipu nezná libos, takže stejný kód běží v systému i v testech nad simulací.</figcaption>
</figure>

**Rozhraní `Hw`** (`libs/nic/src/lib.rs`) má tyto části:

- čtení a zápis registrů po 8, 16 a 32 bitech (Realtek má 8- a 16bitové
  registry na lichých offsetech, takže čtení celého slova by smazalo
  příznaky přerušení),
- DMA paměť (adresa pro zařízení, čtení a zápis, 32bitové slovo stavu
  deskriptoru jedním přístupem),
- tři bariéry (`dma_wmb`, `dma_rmb`, `mmio_wmb`, viz `libos::barrier`),
- čekání.

V systému ho plní `driver::Mmio` nad `Mapping`, v testech simulace.

**Rozhraní `Nic`** popisuje kartu po inicializaci: `link`, `interrupt`,
`receive`, `reclaim`, `send` + `kick`, `stop` a pro hlášení `notes`.
Smyčka ovladače je pro všechny čipy stejná.

## Start a smyčka

1. `devmgr` najde funkci PCI s ID z `nic::ids` (`program_for`). Ovladač
   dostane:
   - omezený Resource: jen BARy a konfiguraci této funkce, 4 přerušení,
     2 MiB DMA a s IOMMU vlastní doménu,
   - stránku konfigurace PCI,
   - kanál ke klientovi; jeho druhý konec jde initu zprávou `net`.

   Na bootstrap kanál pošle `pci <sběrnice> <zařízení> <funkce>`
   a `regs <adresa> <velikost>`, což je první paměťový BAR. Velikost změří
   správce, ovladač BARy nepřepisuje.
2. Ovladač zapne paměť a DMA v příkazovém registru PCI (INTx vypne),
   namapuje registry a vytvoří DMA paměť podle `Layout`. Pak kartu
   resetuje a nastaví (`open` čipu). Upozornění z inicializace vypíše:
   karta bez MAC, PHY neodpovídá a podobně.
3. Přerušení nastaví až po resetu karty: přednostně **MSI**, a když ho
   funkce nemá, **MSI-X** (`pci::irq::DeviceIrq`). I217–I219 MSI-X
   nemají. Jeden vektor stačí, protože fronta příjmu i odesílání je jen
   jedna.
4. Pošle `INFO` (MAC, MTU 1500, linka, rychlost) a obsluhuje:
   - **přerušení**: potvrdí ho před obsluhou, přečte příčiny (`ICR`,
     `IntrStatus`), vrátí odeslané buffery, přijaté rámce pošle `netd`,
   - **zprávy od netd**: rámce zařadí, dokud jsou volné deskriptory, pak
     jedním zápisem indexu oznámí kartě. Když volný deskriptor není,
     kanál nečte (protitlak).
   - **každou sekundu** zkontroluje linku pro případ, že by se přerušení
     o změně ztratilo. Změnu pošle zprávou `LINK`.
5. Konec (netd zavřel kanál, karta zmizela nebo hlásí chybu sběrnice):
   ovladač kartu zastaví a resetuje, aby přestala používat DMA paměť
   procesu, a vypne MSI.

**Která karta je rozhraní:** `netd` zatím používá jen první kartu, kterou
init dostane. `devmgr` spouští ovladače karet až po virtio, takže
v QEMU zůstává první kartou virtio-net. Na počítači bez virtio je první
nalezená karta. Více rozhraní a směrování mezi nimi zatím není.

## Paměť DMA a kruhy

Všechny tři čipy mají deskriptor o 16 B. Rozložení paměti (`Layout`) je
proto společné, dohromady 1,03 MiB:

| Offset | Obsah |
|---|---|
| 0 | kruh příjmu, 256 deskriptorů (4 KiB) |
| 4 KiB | kruh odesílání, 256 deskriptorů |
| 8 KiB | 256 bufferů příjmu po 2 KiB, za nimi 256 bufferů odesílání |

<figure class="diagram">
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Kruh prijmu">
<defs><marker id="nk2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<text class="t b" x="10" y="24">Intel (e1000e, igc): indexy v registrech</text>
<rect class="ok" x="10" y="40" width="60" height="34"/><rect class="ok" x="70" y="40" width="60" height="34"/><rect class="ok" x="130" y="40" width="60" height="34"/>
<rect class="box" x="190" y="40" width="60" height="34"/><rect class="box" x="250" y="40" width="60" height="34"/><rect class="box" x="310" y="40" width="60" height="34"/><rect class="box" x="370" y="40" width="60" height="34"/>
<rect class="warn" x="430" y="40" width="60" height="34"/>
<text class="m" x="100" y="62" text-anchor="middle">hotové (DD), čte ovladač</text>
<text class="m" x="310" y="62" text-anchor="middle">prázdné, patří kartě</text>
<text class="m" x="460" y="62" text-anchor="middle">volný</text>
<path class="ln" d="M40 100 L40 78" marker-end="url(#nk2)"/><text class="m" x="10" y="114">další ke čtení</text>
<path class="ln" d="M190 100 L190 78" marker-end="url(#nk2)"/><text class="m" x="160" y="114">RDH (karta)</text>
<path class="ln" d="M460 100 L460 78" marker-end="url(#nk2)"/><text class="m" x="430" y="114">RDT (ovladač)</text>
<text class="t b" x="10" y="150">Realtek (r8169): vlastník v deskriptoru</text>
<rect class="ok" x="10" y="166" width="60" height="34"/><rect class="ok" x="70" y="166" width="60" height="34"/>
<rect class="box" x="130" y="166" width="60" height="34"/><rect class="box" x="190" y="166" width="60" height="34"/><rect class="box" x="250" y="166" width="60" height="34"/><rect class="box" x="310" y="166" width="60" height="34"/><rect class="box" x="370" y="166" width="60" height="34"/><rect class="acc" x="430" y="166" width="60" height="34"/>
<text class="m" x="70" y="188" text-anchor="middle">OWN = 0</text><text class="m" x="280" y="188" text-anchor="middle">OWN = 1 (patří kartě)</text><text class="m" x="460" y="188" text-anchor="middle">RingEnd</text>
<text class="m" x="10" y="230">Intel: jeden deskriptor zůstává volný (RDT = poslední předaný), aby plný kruh nesplynul s prázdným.</text>
<text class="m" x="10" y="246">Realtek: karta má vlastní index od resetu a jde po bitech OWN; RingEnd ji vrátí na začátek.</text>
</svg>
<figcaption>Kruh příjmu. Odesílání je zrcadlové: ovladač plní deskriptory a posune ocas (Intel TDT, Realtek TxPoll).</figcaption>
</figure>

- **Příjem:** ovladač čte deskriptory, dokud jsou hotové (Intel bit DD,
  Realtek OWN = 0). Rámec s chybou (CRC, délka), rámec rozdělený do více
  bufferů (Realtek) nebo rámec mimo 14 až 1514 B zahodí a započítá.
  Realtek zapisuje délku i s FCS (4 B), ovladač ji odečte. Intel FCS
  odstraní sám (`RCTL.SECRC`).
- **Odesílání:** rámec se zkopíruje do bufferu deskriptoru. Krátký
  rámec ovladač doplní nulami na 60 B, protože některé revize Realteku
  to neumějí. Deskriptor má příkaz „konec rámce, přidat FCS, hlásit
  odeslání“. Volný se pozná podle zpětného zápisu karty: Intel DD,
  Realtek OWN = 0.
- **Bariéry:** před zápisem indexu do registru `dma_wmb` a `mmio_wmb`,
  po přečtení stavu deskriptoru `dma_rmb` před čtením dat. Realtek navíc
  zapíše bit OWN až po adrese bufferu.

⚠️ **DMA bez IOMMU:** jako u virtio-net. S IOMMU smí karta jen do paměti
své domény, bez IOMMU (`--iommu none`) kamkoli. Emulované karty Intel
v QEMU jdou přes IOMMU vždy, přepínač `iommu_platform` nemají.

## Čipy

### e1000e

Klasické deskriptory, registry podle datasheetu 82574. Inicializace:

1. Zakázat přerušení, zastavit příjem a odesílání, `GIO_MASTER_DISABLE`
   (dokončit rozpracované DMA).
2. Reset MAC (`CTRL.RST`). PHY se neresetuje: u PCH by se po resetu PHY
   musela znovu nahrát konfigurace OEM z NVM. U PCH jde reset za
   semaforem PHY (`EXTCNF_CTRL.SWFLAG`), protože PHY může zrovna používat
   firmware ME.
3. Počkat na NVM: 82574 `EECD.AUTO_RD`, PCH `STATUS.LAN_INIT_DONE`.
4. **PCH:** zkusit přečíst ID PHY přes `MDIC` (adresa 1 a 2). Když PHY
   neodpovídá (režim „ultra low power“, který nastaví Windows při
   vypnutí bez kabelu), zrušit vynucený SMBus a přepnout LANPHYPC
   (napájení PHY). Pokud reset PHY blokuje ME (`FWSM.RSPCIPHY`), jen to
   ohlásit.
5. MAC z `RAL0/RAH0`, kam ji karta při resetu nahraje z NVM. Když tam
   není: 82574 ji přečte z EEPROM (`EERD`), PCH (NVM ve flash BIOSu)
   dostane náhodnou místní adresu `02:...`, kterou ovladač zapíše do
   `RAR0`.
6. `CTRL.SLU`, kruhy, `RCTL` (příjem: buffer 2 KiB, všesměrové
   a skupinové rámce, bez FCS), `TCTL` (doplňovat krátké rámce),
   `TXDCTL` se zpětným zápisem po každém deskriptoru, `ITR` nejvýš ~20 000
   přerušení za sekundu, `IMS`.

### igc

Pokročilé deskriptory, fronta 0. Přerušení v režimu MSI přes `ICR`, bez
vektorů MSI-X pro fronty.

1. `GIO_MASTER_DISABLE`, zastavit, reset zařízení (`CTRL.DEV_RST`)
   a počkat na `EECD.AUTO_RD`.
2. MAC z `RAL0/RAH0`. Karta bez NVM (`..._BLANK_NVM`) dostane místní
   adresu.
3. Příjem:
   - `SRRCTL`: buffer 2 KiB, typ „advanced, one buffer“, zahodit při
     plné frontě,
   - `RXDCTL.ENABLE` a počkat, až se zapne,
   - teprve pak `RDT`.
4. Odesílání: `TXDCTL.ENABLE`, zpětný zápis po každém deskriptoru.
5. Rychlost z `STATUS`: 2,5 Gb/s hlásí bit 22 spolu s bitem 1 Gb/s.

### r8169

Registry 8, 16 i 32bitové, zámek konfigurace `Cfg9346`.

1. Zakázat přerušení, reset `ChipCmd.CmdReset` (čip ho sám shodí).
2. MAC z `MAC0..5`. Když tam není, zapíše místní adresu, ale jen pod
   odemčeným `Cfg9346` (horní část první).
3. Pod odemčeným `Cfg9346`:
   - `RxMaxSize` 2 KiB (delší rámec se zahodí, nerozdělí),
   - `CPlusCmd` bez offloadu a VLAN, `PCIDAC` pro DMA nad 4 GiB,
   - adresy kruhů (horní polovina první),
   - `ChipCmd` TE a RE, `RxConfig` (všesměrové, skupinové, vlastní
     adresa), `TxConfig`, všechny skupinové adresy v `MAR0`.
4. Zamknout konfiguraci, nastavit `IntrMask`. Přerušení se potvrzuje
   zápisem jedniček do `IntrStatus`.

RTL8101/8136 (jen 100 Mb/s), RTL8169 (PCI) a RTL8125/8126 (2,5/5 Gb/s,
jiné registry) ovladač neobsluhuje.

## Testy

**Na hostiteli** (`cargo test -p nic`, součást `cargo xtask test`)
modely registrů v `libs/nic/src/tests/` chodí po DMA paměti jako
skutečná karta. Čtou deskriptory podle adres, které do nich zapsal
ovladač, zapisují rámce a stav a vyvolávají přerušení. Přístup mimo DMA
paměť, špatný typ deskriptoru, nedoplněný krátký rámec nebo zápis do
zamčené konfigurace (Realtek) test shodí.

Společné scénáře běží pro 82574, I219, I226 a RTL8168:

- inicializace a linka,
- odesílání rámců 14 až 1514 B (krátké doplněné na 60 B),
- 1000 rámců oběma směry s různou délkou dávek (oba kruhy několikrát
  dokola),
- přetečení kruhu příjmu a zotavení,
- protitlak (bez `kick` se kruh nevyprázdní),
- přerušení a změna linky,
- vyjmutá karta (registry čtou samé jedničky → `Gone`),
- `stop` zastaví DMA.

Testy jednotlivých čipů:

- zdroj MAC (NVM, EEPROM, náhodná),
- probuzení PHY u PCH a respektování blokace ME,
- rychlosti igc až do 2,5 Gb/s,
- vadné a rozdělené rámce,
- odečtení FCS u Realteku,
- DMA nad 4 GiB (`PCIDAC`).

**V QEMU** (`cargo xtask test`, sada „sitove karty“) je druhá karta
`-device e1000e` (82574L) ve vlastní síti slirp. Test mluví s ovladačem
přímo protokolem `netproto::link`:

| Test | Co ověří |
|---|---|
| INFO, MAC z NVM a linka 1 Gb/s | ovladač přečetl MAC karty (QEMU `52:54:00:...`, ne místní), linka naběhne do 10 s, plný duplex |
| ARP na bránu slirp | rámec projde kartou ven a odpověď zpět |
| 640 rámců tam i zpět | 20 kol po 32 dotazech ARP, oba kruhy dvakrát dokola |

Sada běží na x86_64 (IOMMU Intel VT-d) i aarch64 (SMMUv3). Bez IOMMU se
dá pustit přes `--iommu none`.
