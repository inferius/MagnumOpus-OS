# Zvuk

Zvuk obsluhují dva procesy a jedna knihovna protokolu:

- **`hda`**: ovladač řadiče Intel High Definition Audio (PCI třída
  04:03). Resetuje řadič, najde kodeky a cestu k výstupu, spouští DMA.
  Jako jediný má přístup k řadiči.
- **`audiod`**: zvuková služba. Míchá streamy aplikací, rozvádí
  nahrávaný zvuk, převádí formáty a vzorkovací frekvence, řídí hlavní
  hlasitost, ztlumení a výběr zvukové karty.
- **`libs/audioproto`**: protokol mezi aplikacemi, službou a ovladačem.
  Vzorky nejdou přes kanál, ale přes sdílenou paměť (VMO).

Logika bez hardwaru je v knihovnách testovaných na hostiteli:
`libs/hdaspec` (registry, verby, graf widgetů, cesty k výstupům i ke
vstupu, HDMI, opravy desek) a `libs/audiomix` (převod formátů, převod
frekvence s antialiasingovým filtrem, mix, tón, WAV).

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Cesta zvuku">
<defs><marker id="au1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="150" height="56" rx="6"/><text class="t b" x="85" y="43" text-anchor="middle">aplikace A</text><text class="m" x="85" y="62" text-anchor="middle">44,1 kHz mono</text>
<rect class="box" x="10" y="100" width="150" height="56" rx="6"/><text class="t b" x="85" y="123" text-anchor="middle">shell (audio)</text><text class="m" x="85" y="142" text-anchor="middle">48 kHz, WAV</text>
<rect class="ok" x="190" y="20" width="140" height="136" rx="6"/><text class="t b" x="260" y="45" text-anchor="middle">kruhové buffery</text><text class="m" x="260" y="64" text-anchor="middle">VMO na stream</text><text class="m" x="260" y="80" text-anchor="middle">write_pos / read_pos</text><text class="m" x="260" y="96" text-anchor="middle">bez kopie přes jádro</text><text class="m" x="260" y="112" text-anchor="middle">událost "místo"</text>
<path class="ln" d="M160 48 L188 48" marker-end="url(#au1)"/><path class="ln" d="M160 128 L188 128" marker-end="url(#au1)"/>
<rect class="acc" x="360" y="20" width="170" height="136" rx="6"/><text class="t b" x="445" y="45" text-anchor="middle">audiod</text><text class="m" x="445" y="64" text-anchor="middle">převod formátu</text><text class="m" x="445" y="80" text-anchor="middle">převod frekvence</text><text class="m" x="445" y="96" text-anchor="middle">mix, hlasitost</text><text class="m" x="445" y="112" text-anchor="middle">2 periody před DMA</text>
<path class="ln" d="M330 88 L358 88" marker-end="url(#au1)"/>
<rect class="ok" x="560" y="20" width="190" height="70" rx="6"/><text class="t b" x="655" y="45" text-anchor="middle">DMA buffer</text><text class="m" x="655" y="64" text-anchor="middle">16 period po 5,33 ms</text><text class="m" x="655" y="80" text-anchor="middle">s16 stereo 48 kHz</text>
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
2. init spustí `audiod` pod supervisorem a předá mu kanály všech
   ovladačů. Služba se každého zeptá na zařízení (`INFO`) a jako aktivní
   použije první s analogovým výstupem; ostatní (např. HDMI zvuk grafické
   karty) jsou v seznamu (`DEVICES`) a dají se vybrat (`SELECT`).
   Nahrává se z aktivní karty, když má vstup, jinak z první jiné karty
   se vstupem (otevře se zvlášť, její výstup se nespouští).
3. Bootstrap kanál služby si nechá init (jen přes něj jde přidat ovladač
   za běhu). Klienti dostávají relace přes `CONNECT`: termd s lístkem od
   supervisoru (`session("audiod")`) a každá relace shellu vlastní relaci
   s lístkem (`narrow`), testy bez lístku.

### Obnova po pádu

- **Pád `audiod`:** supervisor ho spustí znovu. Kanály k ovladačům drží
  init (`Kept`), takže ovladače nic nepoznají a nová instance je dostane
  hned; stream u ovladače si otevře znovu. Klienti s lístkem
  (`svcclient::Conn`) se při dalším volání připojí sami; otevřené streamy
  restart nepřežijí (volání vrátí `SERVICE_RESTARTED` nebo `NOT_FOUND`,
  program otevře nový).
- **Pád `hda`:** devmgr ho ve vlastním vlákně spustí znovu (nejvýš 5×
  za minutu) a pošle initu novou zprávu `audio`. Init kanál uschová
  a předá běžící službě (`device`); služba starý ovladač vyřadí (vidí
  zavřený kanál) a přejde na nový.

Obojí ověřují testy „zabij audiod“ a „zabij ovladač hda“ (sada
`obnova zvuku`).

## Protokol (`libs/audioproto`)

### Formáty

Formát PCM = vzorkovací frekvence (8-192 kHz), počet kanálů (1-8)
a typ vzorku: `u8`, `s16`, `s24` (24 bitů v 32bitovém slově), `s32`,
`f32`. Vzorky kanálů jsou prokládané. Zařízení hraje 48 kHz (nebo
44,1 kHz, když kodek 48 kHz neumí), stereo, 16 bitů, nebo 24 bitů ve
32bitovém slově, když to umí všechny kodeky s výstupem. Nahrává 48 kHz,
stereo, 16 bitů. Převod na formát klienta dělá služba.

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
| `DEVICES`, `SELECT` | seznam zvukových karet, přepnutí (streamy hrají dál na nové) |

Hlasitost 50 % je asi -18 dB, 10 % -60 dB (kubická křivka se blíží
vnímání hlasitosti lépe než lineární).

**Nahrávání** (`Direction::Input`) má stejný tvar s opačnými rolemi:
služba píše, klient čte, událost znamená "nová data". Když klient
nestíhá číst, služba zbytek zahodí a počítá přetečení. Zařízení bez
vstupu vrací `NOT_SUPPORTED`.

Klient `audioproto::conn::AudioClient` stojí na `svcclient::Conn`:
s lístkem přežije restart služby, bez něj vrátí `SERVICE_UNAVAILABLE`.

Data do kruhu smí psát i jiný proces než vlastník streamu:
`Stream::producer_handles` vrátí kopie VMO kruhu a události periody
(dekodér médií, [multimedia.md](multimedia.md)). Řízení (start, stop,
zavření) zůstává vlastníkovi; na událost pak čeká jen producent.

### Protokol ovladače

`audiod` pošle ovladači `OPEN_OUTPUT` (formát, perioda, počet period)
a dostane tři handly: **DMA buffer** (služba do něj míchá přímo, žádná
další kopie), **stránku stavu** (pozice DMA v snímcích, počet přerušení,
chyby) a **událost periody**. BDL, CORB, RIRB ani registry řadiče služba
nevidí. `START` a `STOP` spouští a zastavují DMA. Nahrávání má
`OPEN_INPUT`, `START_INPUT`, `STOP_INPUT`, `CLOSE_INPUT`; pozice ve
stránce stavu jsou snímky, které zařízení do bufferu už zapsalo.
`TEST_EXIT` ukončí ovladač jako po pádu (posílá ho jen `audiod` na
pokyn initu v testu obnovy).

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

Ovladač přečte z každého kodeku na lince (všechny adresy, které se
ohlásily) všechny widgety audio funkční skupiny:
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
3. Všechny DAC všech kodeků dostanou stejný stream: linkový výstup,
   sluchátka i reproduktor hrají současně. **Auto-mute:** když detekce
   zásunutí hlásí sluchátka, reproduktory se vypnou (pin bez `OUT`).
   Konektory hlásí změnu **nevyžádanou odpovědí** (značka podle pinu,
   přerušení RIRB); záloha je čtení každé 3 s (bez MSI každých 500 ms).
4. Nastavení cesty: napájení D0, výběry selektorů a pinů, zesilovače na
   0 dB (hlasitost řeší služba softwarově), u mixerů jen vstup z cesty
   (ostatní, např. smyčka z mikrofonu, se ztlumí), řízení pinu
   (`OUT`, u sluchátek `HP`), EAPD.
5. Když výchozí konfigurace žádný analogový výstup neuvádí (špatně
   vyplněný BIOS), ovladač zkusí všechny piny s výstupem jako linkový
   výstup.

**HDMI/DP** (`hdaspec::hdmi`): ovladač připraví digitální cestu
(napájení, pin `OUT`, digitální převodník, 2 kanály, sloty) a čte
připojení monitoru. Když je monitor připojený a ELD platné (verb
`GET_HDMI_ELDD` po bajtech: jméno monitoru, HDMI nebo DP, kolik kanálů
LPCM umí), zapíše audio infoframe (HDMI typ 0x84 se součtem, DP
sekundární paket) a převodník dostane stejný stream jako analogové
výstupy. Odpojení monitoru převodník odpojí. Vícekanálový zvuk,
mapování kanálů a DP MST zatím nejsou.

### Nahrávání

`hdaspec::input` najde cestu od převodníku A/D (ADC) proti proudu ke
vstupnímu pinu v pořadí: mikrofon v konektoru, vestavěný mikrofon,
linkový vstup, aux. Nastaví výběry, vstupní zesilovače na 0 dB (u mixeru
jen vstup z cesty), pin jako vstup a u mikrofonu napájení 80 %, když ho
pin umí. Vstupní stream je první vstupní deskriptor (číslo streamu 2).

### Opravy desek

Některé desky mají chybnou výchozí konfiguraci pinů, nebo potřebují
GPIO (zesilovač reproduktorů notebooku) či koeficienty kodeku.
`hdaspec::fixup` má tabulku podle ID kodeku a ID subsystému (výrobce
desky, verb `GET_SUBSYSTEM_ID`): přepis konfigurace pinů před plánováním
cest, GPIO a koeficienty po nastavení. Údaje jsou fakta o hardwaru
(převzít je smíme i z veřejných tabulek oprav, ne jejich kód). **Tabulka
je zatím prázdná**: položka se přidá, až se ověří na skutečném počítači.
Obecná oprava bez tabulky: u kodeků Realtek se zapne EAPD na všech
pinech, které ho mají.

### Stream a DMA

Výstupní stream je první výstupní deskriptor (za vstupními). DMA
buffer je kruh 4 period; každá perioda je jedna položka BDL s IOC,
takže řadič po každé periodě vyvolá přerušení. Adresa i délka položky
jsou zarovnané na 128 bajtů (perioda 256 snímků = 1024 B). Paměť DMA
vzniká přes Resource ovladače, takže s IOMMU (VT-d, AMD-Vi, SMMUv3) do
ní zařízení smí a jinam ne. Pozice (`LPIB`) se převádí na monotónní
počet snímků ve stránce stavu. Chyba FIFO nebo deskriptoru stream
zastaví a nahlásí.

## Relace aplikací a nahrávání

**[ověřeno v QEMU]** x86_64 i aarch64 (test `sdl: zvuk - prehravani,
nahravani s povolenim, odvolani (QMP)`, host testy `cargo test -p
audioproto session`, `-p accountproto perm`).

Každá aplikace s oknem dostane při spuštění **relaci zvukové služby
s právy aplikace**: init ji vytvoří přes supervisor
(`Supervisor::session("audiod", recept)`) a předá jako handle se štítkem
`abi::params::tag::AUDIO`, lístek k obnově jako `AUDIO_LEASE`. Recept
(`audioproto::session::Recipe`) nese identitu a jméno z manifestu,
uživatele relace a nahrávání podle manifestu; po restartu `audiod` ho
supervisor pošle znovu, takže práva obnovu přežijí. Prázdný recept je
systémová relace (lišta, shell, init) jako dosud.

| Smí relace aplikace | Ne |
|---|---|
| `OPEN`, `START`, `STOP`, `CLOSE`, `DRAIN` vlastních streamů, `STREAM_VOLUME`, `STREAM_INFO`, `STATUS`, `DEVICES` | `MASTER_VOLUME`, `MUTE`, `SELECT`, `CONNECT` (`ACCESS_DENIED`; mění zvuk celého systému, patří liště a nastavení) |

**Nahrávání** (`OPEN` vstupu):

- bez `[audio] capture = true` v manifestu `ACCESS_DENIED` hned, bez
  dotazu;
- s ním se `audiod` při prvním `OPEN` vstupu v relaci zeptá služby účtů
  (kanál `permissions`, zprávy `accountproto::perm`) a odpoví aplikaci,
  až přijde rozhodnutí: uložené hned, jinak po systémovém dialogu
  Povolení (Povolit / Jen tentokrát / Nepovolit, Esc = Nepovolit). Volání
  aplikace do té doby čeká;
- rozhodnutí platí po zbytek relace; „Povolit“ a „Nepovolit“ se ukládají
  trvale po uživateli a aplikaci ([Účty](ucty.md#povolení-aplikací)),
  odvolání příkazem shellu `permissions revoke <aplikace>`;
- bez služby účtů (nebo bez kompozitoru pro dialog) nahrávání pro
  aplikace není.

Po dobu nahrávání ukazuje lišta místo ikony hlasitosti **mikrofon
v barvě nebezpečí**; bublina vypíše, které aplikace nahrávají (`STATUS`
pole `capturing`). Uživatel tak vždy vidí, že se nahrává.

### Relace zvuku pro potomky (role `Audio`)

**[ověřeno v QEMU]** x86_64 i aarch64 (sada `sdl`, testy `role Audio: ...`, program
`user/gui-probe`). Aplikace, která spouští další procesy se zvukem (Wine:
`wine` -> `notepad`, hra z `cmd`), jim předá relaci zvuku se **stejnou
identitou** jako relaci okna ([role `Gui`](gui.md#relace-okna-pro-potomky-role-gui)):

- launchd zná role `Audio` a `AudioLease` (`launchproto`), potomek dostane
  relaci a lístek se štítky `abi::params::tag::AUDIO` a `AUDIO_LEASE`
  (stejné jako aplikace od initu). Runtime (`osrt::audio`) je předá
  knihovně zvuku (`take_client_raw`): ta s lístkem dělá obnovu po restartu
  `audiod` a runtime dá háček pro relace dalších potomků.
- Relace potomka vzniká `NARROW` s prázdným zúžením (adaptér `audiod`
  v initu jiné zúžení odmítne): **stejný recept**, tedy stejná identita
  i pravidla nahrávání. Lístek je svázaný s rodičovským, limit živých
  a rychlosti jako u `ADOPT`.
- **Nahrávání nikdy automaticky z manifestu:** i s rolí platí jen
  `Capture::Ask` (dialog Povolení mikrofonu, uložené rozhodnutí) nebo
  `Capture::No`; zúžení nic nerozšíří. Po dobu nahrávání je v liště
  mikrofon (indikátor výše).
- **Opt-out:** runtime při startu nastaví `MO_AUDIO=1`; potomek bez
  `MO_AUDIO` v prostředí relaci nedostane.
- **Odvolání:** `audio::revoke(id)` v initu zruší lístky identity
  u `audiod` i lístky z nich odvozené a `audiod` (zpráva `revoke` na
  řídicím kanálu) zavře všechny relace identity: streamy hned zmlknou,
  obnova přes lístek je odmítnutá.
- **Limity na identitu:** relace (`MAX_IDENTITY_SESSIONS` 64) a streamy
  (`MAX_IDENTITY_STREAMS` 32) se sčítají přes všechny relace stejné
  identity (`user/audiod/src/perm.rs`), procesy je neobejdou.
- Relace bez lístku (testy) jde knihovně taky, ale bez obnovy a bez
  relací pro potomky.

**Společné jádro `moaudio`** (`libs/mowin/src/os/audio.rs`, hlavička
`include/moaudio.h`, část `libc.a`): streamy přes relaci od runtime.
`mosdl_audio_*` (SDL) jsou jen obálky nad ním a zvukový ovladač Wine
(`winemoaudio.drv`, [Windows vrstva](windows-vrstva.md#zvuk-schránka-a-obnova-oken-w4))
volá `moaudio_*`. Zápis a čekání na místo běží bez zámku knihovny;
`moaudio_put` zapíše jen to, co se vejde, bez čekání a bez volání služby
(pro volající, kteří píší pod svým zámkem a stream spouštějí mimo něj).

## Služba `audiod`

### Takt a rozpočet latence

Služba čeká na kanály klientů a na událost periody od ovladače. Po
každé periodě přečte pozici DMA a namíchá data tak, aby byla o předstih
(zpočátku dvě periody) před ní. Za namíchanými daty vždy vynuluje jednu periodu:
když služba nestihne, zazní ticho (a počítá se `device_underruns`), ne
stará data z minulého kola.

| část (48 kHz, perioda 256 snímků = 5,33 ms) | doba |
|---|---|
| mix před pozicí DMA (předstih 2 periody, adaptivně až 12) | 5,3-10,7 ms (až 64 ms) |
| FIFO řadiče a kodeku | < 0,5 ms |
| buffer klienta (min. 5 ms, test 50 ms, shell 100 ms) | podle klienta |
| **od zápisu klienta s 10 ms bufferem po reproduktor** | **asi 11-21 ms** |

**Adaptivní předstih:** když služba nestihne (pozice DMA předběhne
namíchaná data), předstih se zvětší o periodu, nejvýš na 12 period
(64 ms); po 10 s bez podtečení se zase o periodu zmenší. Na rychlém
stroji tak latence zůstane 11-21 ms, na zatíženém (QEMU na vytíženém
hostiteli, který hostovi na desítky ms sebere procesor) se zvětší
místo opakovaných mezer ve zvuku. DMA buffer má proto 16 period
(85 ms): předstih, jedna perioda ticha a rezerva, kterou zařízení
právě čte. Služba se probouzí 187krát
za sekundu; mix jedné periody pro dva streamy je desítky mikrosekund.
Když 2 s nehraje žádný stream, služba DMA zastaví (zařízení nehraje
ticho do nekonečna) a při dalším `START` buffer vynuluje a spustí znovu.
Vstup se zastaví hned, jak nebeží žádný nahrávací stream.

Nahrávání: po každé periodě vstupu služba přečte nová data z DMA
bufferu, převede je na stereo Q23 a každému nahrávacímu streamu přes
jeho převodník frekvence zapíše do bufferu v jeho formátu (s hlasitostí
streamu).

### Převod formátu a frekvence

Vzorky každého streamu se převedou na vnitřní stereo Q23 (24 bitů
v i32): mono do obou kanálů, víc kanálů **downmix** podle rozložení
WAV/HDA (3 až 8 kanálů: přední plně, střed a zadní/boční -3 dB, LFE se
vynechá, váhy normované, takže nic neořízne). Převod frekvence je
**kubická interpolace Catmull-Rom** s fází v Q32 (při stejné frekvenci
přesně vstup). Při **snižování** frekvence jde vstup nejdřív přes FIR
dolní propust (47 koeficientů, okénkovaný sinc s Hannovým oknem, mez
0,45 × nová frekvence): obsah nad novou Nyquistovou frekvencí je
potlačený aspoň o 40 dB. Na konci `DRAIN` se historie převodníku
vytlačí tichem, takže zazní i poslední vzorky. Mix sčítá v i64 se
zesílením streamu, pak hlavní hlasitost a ořez na 16 nebo 24 bitů
(ořezává se až součet). Vše v celých číslech (koeficienty filtru se
spočítají jednou při otevření) - napsáno v době, kdy programy na aarch64
běžely bez FPU; celá čísla dávají stejný výsledek na obou architekturách.

## Příkaz shellu `audio`

```shell-b
audio tone 1000 --duration 2s --level 30
audio play /hudba/ukazka.wav
audio volume 60
audio mute
audio status
audio devices
audio device 1
```

`audio volume` bez čísla vrátí hlasitost, `audio unmute` zruší ztlumení,
`audio devices` vypíše zvukové karty a `audio device N` přepne.
`tone` a `play` vrátí záznam s délkou přehrání a počtem podtečení.
Zvuk jde do relace po kouscích 100 ms: soubor WAV čte interpret po
částech přes FS relace (stejná práva jako `open`, velikost
neomezená), mezi kousky kontroluje **Ctrl-C** (pak stream zastaví a vrátí
`SH_INTERRUPTED`). Formáty WAV: PCM 8/16/32 bitů, 24 bitů ve 32bitovém
slově a float 32, libovolná frekvence a počet kanálů.

## Testy

- **Na hostiteli** (`cargo xtask test`): `audioproto` (zprávy, buffer
  i se zlými pozicemi), `audiomix` (převody tam i zpět, downmix, Goertzel
  na tónu po převodu 44,1 -> 48 kHz, potlačení aliasu 96 -> 48 kHz, mix
  16 i 24 bitů, WAV), `hdaspec` (verby, formáty, seznamy spojení, grafy
  QEMU hda-duplex, Realtek ALC, selektor, HDMI, cykly, cesta ke vstupu,
  ELD a infoframe, tabulka oprav), příkaz `audio` v `shlang` (kousky,
  čtení po částech, Ctrl-C), detekce tónu v xtask.
- **V QEMU** sada `zvuk`: zařízení a výstup, stream běží a DMA posouvá
  pozici, přerušení chodí, mix dvou klientů (48 a 22,05 kHz), hlasitost
  a ztlumení, neplatné požadavky, `audio tone`/`volume`/`status` přes
  termd a relaci, nahrávání (16 kHz mono přes převod dolů) z druhé
  karty, seznam dvou karet a přepnutí výstupu. Sada `obnova zvuku`: „zabij
  audiod“ (nová instance má hned zařízení, klient s lístkem se připojí
  sám) a „zabij ovladač hda“ (devmgr ho spustí znovu, služba přejde na
  nový).
- **Zapsaný zvuk:** QEMU má dva řadiče `intel-hda`: první s kodekem
  `hda-output` do `-audiodev wav` (48 kHz, s16, stereo), druhý
  s `hda-duplex` a backendem `none` (umí i vstup pro test nahrávání;
  backend wav nahrávat neumí). Dva kodeky na jedné lince QEMU nemodeluje
  správně (každý kodek si z DMA bere vlastní data, na skutečném hardwaru
  se stream vysílá všem), proto dva řadiče. Test `ton 1 kHz do WAV` přehraje 1,5 s tónu 1 kHz (mono 44,1 kHz, tedy přes převod
  frekvence). Že zazněl celý, ověří test v QEMU počítadlem audiod (všech
  66 150 snímků). Soubor WAV je doplňková kontrola celé cesty: po konci
  QEMU xtask hledá okna 50 ms s čistým tónem 1 kHz a vyžaduje aspoň
  500 ms. Když tón chybí a audiod během něj hlásil podtečení zařízení
  (hostitel hostovi bral procesor), je to varování s vysvětlením; bez
  podtečení je to chyba cesty k výstupu. xtask navíc vypíše časovou osu
  souboru po 100 ms (`.` ticho, `t` tón, `?` jiný zvuk).

Se zvukem interaktivně: `cargo xtask run --audio dsound` (Windows),
`--audio pa` nebo `alsa` (Linux), `coreaudio` (macOS).

## Hardware a rizika

- **QEMU** (`intel-hda` na q35 i na PCIe stroje virt pro aarch64)
  a **VMware** (HDA s kodekem typu Realtek) používají stejné cesty jako
  skutečné desky.
- **Desky Intel a AMD** s analogovým kodekem (Realtek, Conexant, VIA)
  by měly hrát, ale **na skutečném hardwaru to zatím ověřené není.**
  Tabulka oprav desek je prázdná: notebook, který potřebuje GPIO nebo
  koeficienty pro zesilovač, může mít tiché reproduktory, zatímco
  linkový výstup hraje. Položka se přidá po ověření na daném počítači.
- **Notebooky Intel s DSP (SOF, digitální mikrofony)** mají zvuk za DSP
  a v režimu HDA ho nemusí ukázat vůbec. Nepodporováno.
- **HDMI/DP** je napsané podle specifikace, ale QEMU kodek HDMI nemá,
  takže to v testech neprochází: ověřit na skutečné grafické kartě.
  Jen stereo; DP MST (víc monitorů na jednom konektoru) chybí.
- **24 bitů** se použije, jen když ho hlásí všechny kodeky s výstupem;
  QEMU umí jen 16 bitů, takže 24bitová cesta je ověřená jen testy na
  hostiteli.
- Nahrávání bere jediný vstup (aktivní karta, když ho má, jinak první
  jiná se vstupem; na kartě první vstup podle pořadí); ruční výběr vstupu
  zatím není.
- Víc kodeků s výstupem na jednom řadiči hraje tentýž stream; v QEMU to
  ověřit nejde (viz Testy), jen vyčet obou kodeků.
