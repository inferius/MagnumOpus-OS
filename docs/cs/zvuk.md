# Zvuk

Zvuk obsluhují dva procesy a jedna knihovna protokolu:

- **`hda`**: ovladač řadiče Intel High Definition Audio (PCI třída
  04:03). Resetuje řadič, najde kodeky a cestu k výstupu, spouští DMA.
  Jako jediný má přístup k řadiči.
- **`audiod`**: zvuková služba. Míchá streamy aplikací, převádí formáty
  a vzorkovací frekvence, řídí hlavní hlasitost a ztlumení.
- **`libs/audioproto`**: protokol mezi aplikacemi, službou a ovladačem.
  Vzorky nejdou přes kanál, ale přes sdílenou paměť (VMO).

Logika bez hardwaru je v knihovnách testovaných na hostiteli:
`libs/hdaspec` (registry, verby, graf widgetů, hledání cest) a
`libs/audiomix` (převod formátů, převod frekvence, mix, tón, WAV).

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Cesta zvuku">
<defs><marker id="au1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="150" height="56" rx="6"/><text class="t b" x="85" y="43" text-anchor="middle">aplikace A</text><text class="m" x="85" y="62" text-anchor="middle">44,1 kHz mono</text>
<rect class="box" x="10" y="100" width="150" height="56" rx="6"/><text class="t b" x="85" y="123" text-anchor="middle">shell (audio)</text><text class="m" x="85" y="142" text-anchor="middle">48 kHz, WAV</text>
<rect class="ok" x="190" y="20" width="140" height="136" rx="6"/><text class="t b" x="260" y="45" text-anchor="middle">kruhové buffery</text><text class="m" x="260" y="64" text-anchor="middle">VMO na stream</text><text class="m" x="260" y="80" text-anchor="middle">write_pos / read_pos</text><text class="m" x="260" y="96" text-anchor="middle">bez kopie přes jádro</text><text class="m" x="260" y="112" text-anchor="middle">událost "místo"</text>
<path class="ln" d="M160 48 L188 48" marker-end="url(#au1)"/><path class="ln" d="M160 128 L188 128" marker-end="url(#au1)"/>
<rect class="acc" x="360" y="20" width="170" height="136" rx="6"/><text class="t b" x="445" y="45" text-anchor="middle">audiod</text><text class="m" x="445" y="64" text-anchor="middle">převod formátu</text><text class="m" x="445" y="80" text-anchor="middle">převod frekvence</text><text class="m" x="445" y="96" text-anchor="middle">mix, hlasitost</text><text class="m" x="445" y="112" text-anchor="middle">2 periody před DMA</text>
<path class="ln" d="M330 88 L358 88" marker-end="url(#au1)"/>
<rect class="ok" x="560" y="20" width="190" height="70" rx="6"/><text class="t b" x="655" y="45" text-anchor="middle">DMA buffer</text><text class="m" x="655" y="64" text-anchor="middle">4 periody po 5,33 ms</text><text class="m" x="655" y="80" text-anchor="middle">s16 stereo 48 kHz</text>
<path class="ln" d="M530 55 L558 55" marker-end="url(#au1)"/>
<rect class="box" x="560" y="110" width="190" height="56" rx="6"/><text class="t b" x="655" y="133" text-anchor="middle">hda (ovladač)</text><text class="m" x="655" y="152" text-anchor="middle">BDL, CORB/RIRB, MSI</text>
<path class="ln" d="M655 166 L655 198" marker-end="url(#au1)"/>
<path class="ln" d="M560 138 L532 120" marker-end="url(#au1)"/><text class="m" x="470" y="182">pozice DMA + událost periody</text>
<rect class="box" x="560" y="200" width="190" height="70" rx="6"/><text class="t b" x="655" y="225" text-anchor="middle">řadič a kodek HDA</text><text class="m" x="655" y="244" text-anchor="middle">DAC, mixer, pin</text><text class="m" x="655" y="260" text-anchor="middle">linkový výstup, sluchátka</text>
<path class="ln" d="M700 92 L700 198" marker-end="url(#au1)"/><text class="m" x="706" y="190">čte DMA</text>
<text class="m" x="10" y="300">Kanály nesou jen příkazy (OPEN, START, DRAIN, hlasitost); vzorky leží ve sdílené paměti.</text>
<text class="m" x="10" y="318">audiod nevidí registry ani BDL řadiče, jen DMA buffer streamu a stránku s pozicí.</text>
</svg>
<figcaption>Cesta zvuku od aplikace k reproduktoru.</figcaption>
</figure>

## Spuštění

1. `devmgr` najde funkci PCI třídy 04:03 (libovolný výrobce: Intel,
   AMD, VMware, QEMU) a spustí `hda` s omezeným Resource (BARy,
   konfigurace, 2 přerušení, 1 MiB DMA). Initu pošle zprávu `audio`
   s kanálem k ovladači.
2. init spustí `audiod` a předá mu kanály všech ovladačů. Služba se
   každého zeptá na zařízení (`INFO`) a použije první s analogovým
   výstupem; ostatní (např. HDMI zvuk grafické karty) přeskočí.
3. Bootstrap kanál služby si nechá init; další klienti vznikají zprávou
   `CONNECT` (termd ho pošle pro každou relaci shellu, testy pro sebe).

## Protokol (`libs/audioproto`)

### Formáty

Formát PCM = vzorkovací frekvence (8-192 kHz), počet kanálů (1-8)
a typ vzorku: `u8`, `s16`, `s24` (24 bitů v 32bitovém slově), `s32`,
`f32`. Vzorky kanálů jsou prokládané. Zařízení hraje 48 kHz (nebo
44,1 kHz, když kodek 48 kHz neumí), 16 bitů, stereo; převod dělá služba.

### Sdílený kruhový buffer

Buffer streamu vytváří **služba** (velikost a rozložení tedy neurčuje
klient) a předá ho klientovi jako VMO. Obě strany ho mají namapovaný:

| offset | pole | píše |
|---|---|---|
| 0 | magic `AUR1`, verze | služba |
| 8 | kapacita (snímky), bajtů na snímek | služba |
| 64 | `write_pos` (u64, vlastní cache line) | producent |
| 128 | `read_pos` (u64) | konzument |
| 192 | podtečení, přetečení | služba |
| 208 | přehrané snímky, latence (us) | služba |
| 4096 | data | producent |

Pozice jsou monotónní počty snímků (index = pozice mod kapacita).
Producent zapíše data a pak `write_pos` (Release), konzument přečte
`write_pos` (Acquire), data a posune `read_pos`. Druhá strana není
důvěryhodná: pozice mimo meze (víc než kapacita napřed, nebo zpět)
znamená "nic ke čtení" a služba buffer srovná; nikdy se nečte mimo něj.

Po každé periodě služba nastaví **událost** streamu (handle v odpovědi
na `OPEN`). Klient čeká na ni, ne na kanál; jeden zápis do bufferu
a jedno čekání na periodu, žádná zpráva na blok dat.

### Zprávy

| zpráva | význam |
|---|---|
| `OPEN` (směr, formát, latence) | nový stream; odpověď `Opened` + [VMO bufferu, událost] |
| `START`, `STOP` | spustí; zastaví a zahodí nepřehraná data |
| `DRAIN` | odpoví, až vše zapsané zaznělo (pak stream stojí) |
| `CLOSE` | zruší stream (zavření kanálu zruší všechny streamy klienta) |
| `STREAM_VOLUME`, `MASTER_VOLUME` | hlasitost 0-100 % (křivka procenta³) |
| `MUTE` | ztlumení celého výstupu |
| `STATUS`, `STREAM_INFO` | stav zařízení a služby, stav streamu |
| `CONNECT` | nový kanál ke službě |

Hlasitost 50 % je asi -18 dB, 10 % -60 dB (kubická křivka se blíží
vnímání hlasitosti lépe než lineární).

**Nahrávání** (`Direction::Input`) má v protokolu stejný tvar
s opačnými rolemi: služba píše, klient čte, událost znamená "nová data".
Ovladač ho zatím neumí; `OPEN` se směrem `Input` vrací `NOT_SUPPORTED`.

### Protokol ovladače

`audiod` pošle ovladači `OPEN_OUTPUT` (formát, perioda, počet period)
a dostane tři handly: **DMA buffer** (služba do něj míchá přímo, žádná
další kopie), **stránku stavu** (pozice DMA v snímcích, počet přerušení,
chyby) a **událost periody**. BDL, CORB, RIRB ani registry řadiče služba
nevidí. `START` a `STOP` spouští a zastavují DMA, `OPEN_INPUT` je
připravený pro nahrávání.

## Ovladač `hda`

### Řadič

1. Konfigurace PCI: zákaz "no snoop" v PCIe Device Control (paměť DMA
   je v cache, zařízení musí být koherentní), u Intelu třída provozu TC0
   (registr 0x44), u AMD/ATI zapnutý snoop (registr 0x42). Jako Linux
   `snd_hda_intel`.
2. Zastavení DMA po firmwaru, reset (`GCTL.CRST` 0 a zpět 1), 1 ms na
   ohlášení kodeků (`STATESTS`).
3. CORB (příkazy) a RIRB (odpovědi) v jedné stránce DMA; největší
   velikost, kterou řadič umí (obvykle 256 položek). Příkazy jsou
   synchronní s aktivním čekáním a termínem 20 ms (kodek odpovídá
   v mikrosekundách). Když CORB/RIRB nenaběhne, použije se okamžité
   rozhraní (`ICOI`/`ICII`). Kodek, který neodpoví, se dál vynechává.
4. Přerušení MSI (bez X) z `libs/pci::msi`. Bez MSI ovladač čte pozici
   časovačem dvakrát za periodu.

### Kodeky a cesty

Ovladač přečte z každé audio funkční skupiny všechny widgety:
schopnosti, seznam spojení (krátký i dlouhý tvar, rozsahy), u pinů
schopnosti a **výchozí konfiguraci** (jak konektor zapojil výrobce desky:
linkový výstup, reproduktor, sluchátka, HDMI; asociace a pořadí).
Rozhodování je v `hdaspec::graph`:

<figure class="diagram">
<svg viewBox="0 0 760 230" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Cesty kodeku">
<defs><marker id="au2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="30" width="110" height="46" rx="6"/><text class="t b" x="65" y="52" text-anchor="middle">DAC 0x02</text><text class="m" x="65" y="68" text-anchor="middle">stream 1</text>
<rect class="acc" x="10" y="120" width="110" height="46" rx="6"/><text class="t b" x="65" y="142" text-anchor="middle">DAC 0x03</text><text class="m" x="65" y="158" text-anchor="middle">stream 1</text>
<rect class="box" x="200" y="30" width="120" height="46" rx="6"/><text class="t b" x="260" y="52" text-anchor="middle">mixer 0x0C</text><text class="m" x="260" y="68" text-anchor="middle">jen vstup DAC</text>
<rect class="box" x="200" y="120" width="120" height="46" rx="6"/><text class="t b" x="260" y="142" text-anchor="middle">mixer 0x0D</text><text class="m" x="260" y="158" text-anchor="middle">jen vstup DAC</text>
<path class="ln" d="M120 53 L198 53" marker-end="url(#au2)"/><path class="ln" d="M120 143 L198 143" marker-end="url(#au2)"/>
<rect class="ok" x="420" y="10" width="160" height="46" rx="6"/><text class="t b" x="500" y="32" text-anchor="middle">pin 0x14 reproduktor</text><text class="m" x="500" y="48" text-anchor="middle">EAPD, auto-mute</text>
<rect class="ok" x="420" y="80" width="160" height="46" rx="6"/><text class="t b" x="500" y="102" text-anchor="middle">pin 0x1B linkový</text><text class="m" x="500" y="118" text-anchor="middle">zadní panel</text>
<rect class="ok" x="420" y="150" width="160" height="46" rx="6"/><text class="t b" x="500" y="172" text-anchor="middle">pin 0x15 sluchátka</text><text class="m" x="500" y="188" text-anchor="middle">detekce, HP zesilovač</text>
<path class="ln" d="M320 50 L418 33" marker-end="url(#au2)"/><path class="ln" d="M320 140 L418 103" marker-end="url(#au2)"/><path class="ln" d="M320 60 L418 168" marker-end="url(#au2)"/>
<text class="m" x="600" y="40">1. pin: volný DAC</text><text class="m" x="600" y="110">2. pin: další volný</text><text class="m" x="600" y="180">3. pin: sdílí DAC</text>
<text class="m" x="10" y="218">Všechny DAC mají stejné číslo streamu: jeden DMA stream hraje na všech výstupech současně.</text>
</svg>
<figcaption>Typický kodek (Realtek ALC): tři výstupy, dva převodníky.</figcaption>
</figure>

1. Výstupní piny podle výchozí konfigurace (zapojené, s výstupem),
   seřazené podle asociace a pořadí. HDMI/DP a S/PDIF zvlášť.
2. Pro každý pin nejkratší cesta proti proudu (pin -> mixery/selektory
   -> DAC, nejvýš 6 uzlů): nejdřív k volnému DAC, jinak ke DAC už
   použitému. Selektor smí mít jen jeden výběr, takže cesta, která by
   změnila výběr jiné cesty, se odmítne.
3. Všechny DAC dostanou stejný stream: linkový výstup, sluchátka
   i reproduktor hrají současně. **Auto-mute:** když detekce zásunutí
   hlásí sluchátka, reproduktory se vypnou (pin bez `OUT`). Detekce se čte
   každých 500 ms (nevyžádané odpovědi zatím ne).
4. Nastavení cesty: napájení D0, výběry selektorů a pinů, zesilovače na
   0 dB (hlasitost řeší služba softwarově), u mixerů jen vstup z cesty
   (ostatní, např. smyčka z mikrofonu, se ztlumí), řízení pinu
   (`OUT`, u sluchátek `HP`), EAPD.
5. Když výchozí konfigurace žádný analogový výstup neuvádí (špatně
   vyplněný BIOS), ovladač zkusí všechny piny s výstupem jako linkový
   výstup.

**HDMI/DP** se jen najde a nahlásí (`INFO`: výstup `hdmi`, neaktivní).
Chybí čtení ELD (co umí monitor), infoframe a nastavení digitálního
převodníku; to je další krok.

### Stream a DMA

Výstupní stream je první výstupní deskriptor (za vstupními). DMA
buffer je kruh 4 period; každá perioda je jedna položka BDL s IOC,
takže řadič po každé periodě vyvolá přerušení. Adresa i délka položky
jsou zarovnané na 128 bajtů (perioda 256 snímků = 1024 B). Paměť DMA
vzniká přes Resource ovladače, takže s IOMMU (VT-d, AMD-Vi, SMMUv3) do
ní zařízení smí a jinam ne. Pozice (`LPIB`) se převádí na monotónní
počet snímků ve stránce stavu. Chyba FIFO nebo deskriptoru stream
zastaví a nahlásí.

## Služba `audiod`

### Takt a rozpočet latence

Služba čeká na kanály klientů a na událost periody od ovladače. Po
každé periodě přečte pozici DMA a namíchá data tak, aby byla dvě
periody před ní. Za namíchanými daty vždy vynuluje jednu periodu:
když služba nestihne, zazní ticho (a počítá se `device_underruns`), ne
stará data z minulého kola.

| část (48 kHz, perioda 256 snímků = 5,33 ms) | doba |
|---|---|
| mix před pozicí DMA (2 periody) | 5,3-10,7 ms |
| FIFO řadiče a kodeku | < 0,5 ms |
| buffer klienta (min. 5 ms, test 50 ms, shell 100 ms) | podle klienta |
| **od zápisu klienta s 10 ms bufferem po reproduktor** | **asi 11-21 ms** |

DMA buffer má 4 periody (21,3 ms): dvě s daty, jedna s tichem
a jedna rezerva, kterou zařízení právě čte. Služba se probouzí 187krát
za sekundu; mix jedné periody pro dva streamy je desítky mikrosekund.
Když 2 s nehraje žádný stream, služba DMA zastaví (zařízení nehraje
ticho do nekonečna) a při dalším `START` buffer vynuluje a spustí znovu.

### Převod formátu a frekvence

Vzorky každého streamu se převedou na vnitřní stereo Q23 (24 bitů
v i32): mono do obou kanálů, víc kanálů = průměr sudých do levého
a lichých do pravého. Převod frekvence je **kubická interpolace
Catmull-Rom** s fází v Q32 (při stejné frekvenci přesně vstup). Mix
sčítá v i64 se zesílením streamu, pak hlavní hlasitost a ořez na 16 bitů
(ořezává se až součet, takže dva hlasité streamy v protifázi
nezkreslí). Vše v celých číslech: programy na aarch64 běží bez FPU.

## Příkaz shellu `audio`

```shell-b
audio tone 1000 --duration 2s --level 30
audio play /hudba/ukazka.wav
audio volume 60
audio mute
audio status
```

`audio volume` bez čísla vrátí hlasitost, `audio unmute` zruší ztlumení.
`tone` a `play` vrátí záznam s délkou přehrání a počtem podtečení.
Soubor WAV čte interpret přes FS relace (stejná práva jako `open`):
PCM 8/16/32 bitů, 24 bitů ve 32bitovém slově a float 32, libovolná
frekvence a počet kanálů. Příkaz čeká, než zvuk dohraje.

## Testy

- **Na hostiteli** (`cargo xtask test`): `audioproto` (zprávy, buffer
  i se zlými pozicemi), `audiomix` (převody, Goertzel na tónu po převodu
  44,1 -> 48 kHz, mix, WAV), `hdaspec` (verby, formáty, seznamy spojení,
  grafy QEMU hda-duplex, Realtek ALC, selektor, HDMI, cykly), příkaz
  `audio` v `shlang`, detekce tónu v xtask.
- **V QEMU** sada `zvuk`: zařízení a výstup, stream běží a DMA posouvá
  pozici (aspoň o přehraná data), přerušení chodí, mix dvou klientů
  (48 a 22,05 kHz), hlasitost a ztlumení, neplatné požadavky,
  `audio tone`/`volume`/`status` přes termd a relaci.
- **Zapsaný zvuk:** QEMU má `-device intel-hda -device hda-duplex`
  s `-audiodev wav` (48 kHz, s16, stereo). Test `ton 1 kHz do WAV`
  přehraje 1,5 s tónu 1 kHz (mono 44,1 kHz, tedy přes převod
  frekvence); po konci QEMU xtask v souboru hledá okna 50 ms s čistým
  tónem 1 kHz a vyžaduje aspoň 500 ms.

Se zvukem interaktivně: `cargo xtask run --audio dsound` (Windows),
`--audio pa` nebo `alsa` (Linux), `coreaudio` (macOS).

## Hardware a rizika

- **QEMU** (`intel-hda` na q35 i na PCIe stroje virt pro aarch64)
  a **VMware** (HDA s kodekem typu Realtek) používají stejné cesty jako
  skutečné desky.
- **Desky Intel a AMD** s analogovým kodekem (Realtek, Conexant, VIA)
  by měly hrát, ale **na skutečném hardwaru to zatím ověřené není.**
  Některé kodeky potřebují úpravy specifické pro výrobce (koeficienty
  Realtek, GPIO pro zesilovač u notebooků), které Linux má v tabulkách
  oprav; ty tu nejsou. Výsledkem může být ticho na reproduktorech
  notebooku, zatímco linkový výstup hraje.
- **Notebooky Intel s DSP (SOF, digitální mikrofony)** mají zvuk za DSP
  a v režimu HDA ho nemusí ukázat vůbec. Nepodporováno.
- **HDMI/DP**: jen připraveno (viz výše).
- Převod frekvence při **snižování** (např. 96 -> 48 kHz) nefiltruje
  pásmo nad novou Nyquistovou frekvencí: obsah nad 24 kHz se může složit
  zpět (aliasing). Běžné zdroje (44,1 a 48 kHz) to nepostihuje.
- Bez nevyžádaných odpovědí trvá přepnutí na sluchátka až 500 ms.
- `audio play` a `audio tone` zatím nejde přerušit Ctrl-C.
