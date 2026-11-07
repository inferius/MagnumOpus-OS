# USB

USB obsluhují tři druhy procesů, které spolu mluví kanály:

- **`xhci`**: ovladač řadiče xHCI a jádro USB. Nastavuje zařízení
  (adresa, deskriptory, konfigurace, endpointy) a provádí přenosy.
  Jako jediný má přístup k řadiči.
- **`usb-hub`**: ovladač třídy hub. Obsluhuje kořenové porty řadiče
  (virtuální root hub) i externí huby.
- **`usb-hid`**: ovladač klávesnice a myši. Posílá události do
  [služby vstupu](vstup.md) stejně jako virtio-input nebo PS/2.

Třídní ovladače nemají žádný Resource. Dostanou jen kanál svého rozhraní
a nevidí řadič, paměť ani jiná zařízení. Chyba nebo zlý úmysl v ovladači
jedné třídy tak neohrozí ostatní.

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Procesy USB">
<defs><marker id="us1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="160" height="70" rx="6"/><text class="t b" x="90" y="45" text-anchor="middle">řadič xHCI</text><text class="m" x="90" y="64" text-anchor="middle">PCI 0C/03/30</text><text class="m" x="90" y="80" text-anchor="middle">porty, DMA, MSI-X</text>
<rect class="acc" x="220" y="20" width="200" height="90" rx="6"/><text class="t b" x="320" y="45" text-anchor="middle">xhci (proces)</text><text class="m" x="320" y="64" text-anchor="middle">ringy, sloty, enumerace</text><text class="m" x="320" y="80" text-anchor="middle">virtuální root hub</text><text class="m" x="320" y="96" text-anchor="middle">Resource jen tohoto řadiče</text>
<path class="ln" d="M170 55 L218 55" marker-end="url(#us1)"/>
<rect class="box" x="480" y="20" width="130" height="56" rx="6"/><text class="t b" x="545" y="43" text-anchor="middle">devmgr</text><text class="m" x="545" y="62" text-anchor="middle">spouští ovladače</text>
<path class="ln" d="M420 45 L478 45" marker-end="url(#us1)"/><text class="m" x="424" y="38">usb-interface</text>
<rect class="box" x="220" y="180" width="160" height="70" rx="6"/><text class="t b" x="300" y="205" text-anchor="middle">usb-hub</text><text class="m" x="300" y="224" text-anchor="middle">root hub i externí</text><text class="m" x="300" y="240" text-anchor="middle">reset portu, hotplug</text>
<rect class="box" x="430" y="180" width="160" height="70" rx="6"/><text class="t b" x="510" y="205" text-anchor="middle">usb-hid</text><text class="m" x="510" y="224" text-anchor="middle">boot protokol</text><text class="m" x="510" y="240" text-anchor="middle">klávesnice, myš</text>
<rect class="ok" x="630" y="180" width="120" height="70" rx="6"/><text class="t b" x="690" y="210" text-anchor="middle">inputd</text><text class="m" x="690" y="229" text-anchor="middle">IDEV / IEVT</text>
<path class="ln" d="M300 112 L300 178" marker-end="url(#us1)"/><text class="m" x="306" y="150">usbproto (kanál hubu)</text>
<path class="ln" d="M380 112 L480 178" marker-end="url(#us1)"/><text class="m" x="430" y="140">usbproto</text>
<path class="ln" d="M545 78 L520 176" marker-end="url(#us1)"/><path class="ln" d="M545 78 L320 176" marker-end="url(#us1)"/>
<path class="ln" d="M590 215 L628 215" marker-end="url(#us1)"/>
<text class="m" x="10" y="290">Třídní ovladače mají jen kanál svého rozhraní: žádný Resource, žádná DMA, žádná jiná zařízení.</text>
<text class="m" x="10" y="310">Kořenové porty řadiče vidí usb-hub jako obyčejný hub, takže hledání zařízení i hotplug fungují všude stejně.</text>
</svg>
<figcaption>Procesy USB: řadič, jádro USB, třídní ovladače a služba vstupu.</figcaption>
</figure>

## Spuštění a vyhledání zařízení

1. `devmgr` najde na PCI řadič třídy 0C/03/30 a spustí `xhci`. Dá mu
   omezený Resource (BARy, konfiguraci, přerušení, DMA) a kanál zpět.
2. `xhci` převezme řadič od firmwaru (USB Legacy Support, firmware tím
   přestane emulovat PS/2), resetuje ho a nastaví kontexty, frontu příkazů
   a frontu událostí. Pak ohlásí **virtuální root hub** zprávou
   `usb-interface 09 00 00 0000 0000` s kanálem.
3. `devmgr` spustí `usb-hub`. Ten zapne napájení portů, projde je a na
   portu s připojeným zařízením udělá reset (port USB 3 se povolí sám).
   Pak řekne `xhci`, ať zařízení nastaví (`ATTACH`).
4. `xhci` nastaví zařízení:
   - Enable Slot a Address Device,
   - prvních 8 B deskriptoru zařízení (velikost paketu EP0, případně
     Evaluate Context), celý deskriptor a celou konfiguraci,
   - Configure Endpoint pro interrupt endpointy a teprve pak
     SET_CONFIGURATION (xHCI 4.3.5: řadič má prostředky pro endpointy dřív,
     než je zařízení začne používat). U hubu pak deskriptor hubu a další
     Configure Endpoint s údaji hubu v kontextu slotu (počet portů, TT,
     MTT).

   Každé rozhraní pak ohlásí: `usb-interface <třída> <podtřída>
   <protokol> <vid> <pid>`.
5. `devmgr` podle třídy spustí `usb-hub` (09) nebo `usb-hid` (03).
   `usb-hid` dostane i kanál ke službě vstupu, jehož druhý konec jde initu
   zprávou `input`.
6. Po prvním průchodu porty pošle `usb-hub` `SCAN_DONE`, `xhci` pošle
   `usb-ready` a `devmgr` teprve potom initu `done`. Zařízení připojená při
   startu tak jsou ve službě vstupu dřív, než init řekne `settled`.

Log při startu:

```text
xhci 00:08.0: 8 portu (4 USB 3), 64 slotu, kontext 32 B
usb-hub: root hub, 8 portu
xhci: port 5: zarizeni 0627:0001 (high speed), slot 1
usb-hid: USB klavesnice 0627:0001 (port [5], high speed)
inputd: zarizeni 'USB klavesnice 0627:0001' pripojeno
```

## Virtuální root hub

`xhci` předvádí kořenové porty jako hub USB 2.0. Odpovídá na požadavky
třídy hub a převádí je na registry `PORTSC`:

| Požadavek | Co udělá |
|---|---|
| `GET_DESCRIPTOR(HUB)` | počet portů řadiče, napájení po portech |
| `GET_STATUS` portu | `wPortStatus` z `PORTSC`: CCS, PED, OCA, PR, PP, rychlost (low / high; super speed v rezervovaném bitu 15); `wPortChange` z CSC, PEC, OCC, PRC+WRC |
| `SET_FEATURE(PORT_POWER / PORT_RESET)` | nastaví PP / PR |
| `CLEAR_FEATURE(C_PORT_*)` | smaže změnu (RW1C), ostatní bity zachová |
| `CLEAR_FEATURE(PORT_ENABLE)` | vypne port (PED) |
| `INTERRUPT_IN` na 0x81 | čeká na změnu portu, odpoví bitmapou (bit N = port N) |

Proto má `usb-hub` jen jeden kód pro kořenové i externí huby. Hotplug
funguje stejně: změna na portu ukončí čekání na 0x81 a ovladač hubu port
projde znovu.

## Protokol `usbproto`

Požadavek: hlavička 16 B (`op`, `id`, `a`, `b`, vše u32) a data.
Odpověď: `id`, `status` (0 nebo kód [chyby](chyby.md)), `completion` (kód
dokončení xHCI), `actual` (přenesené bajty) a data. Vše little-endian. Na
každý požadavek přijde jedna odpověď se stejným `id`, ale pořadí odpovědí
se může lišit od pořadí požadavků. Na jednom endpointu smí čekat nejvýš
jeden požadavek, jinak `ALREADY_EXISTS`.

| `op` | Kdo | Význam |
|---|---|---|
| 1 `INFO` | všichni | rychlost, číslo rozhraní, cesta porty, deskriptor zařízení, celá konfigurace |
| 2 `CONTROL` | všichni | kontrolní přenos na EP0: data = setup 8 B + data OUT; odpověď = data IN. `SET_ADDRESS` a `SET_CONFIGURATION` = `ACCESS_DENIED` (řídí je řadič) |
| 3 `INTERRUPT_IN` | všichni | `a` = endpoint (např. 0x81), `b` = nejvýš bajtů; odpověď až přijdou data. Jen endpointy vlastního rozhraní, jinak `ACCESS_DENIED` |
| 4 `INTERRUPT_OUT` | všichni | `a` = endpoint, data = obsah |
| 16 `ATTACH` | hub | na port `a` se připojilo zařízení (port je po resetu povolený); odpověď `actual` = slot |
| 17 `DETACH` | hub | zařízení z portu `a` zmizelo: řadič zavře kanály jeho rozhraní a zruší slot |
| 18 `SCAN_DONE` | hub | první průchod porty dokončen |

Chyba přenosu je `status = IO_ERROR` a `completion` říká proč, např.
`6 = STALL`, když zařízení požadavek nepodporuje. Zastavený endpoint
`xhci` sám obnoví (Reset Endpoint, Set TR Dequeue). V třídních ovladačích
požadavky obaluje knihovna `usbclient`:

```rust
let mut usb = UsbClient::new(channel);
let info = usb.info()?;
usb.control(hid::set_boot_protocol(info.interface), &[])?;
let report = usb.call(op::INTERRUPT_IN, 0x81, 8, &[], None)?;
```

## HID: klávesnice, myš, tablet, multimediální klávesy

`usb-hid` nejdřív přečte **deskriptor reportu** rozhraní
(`GET_DESCRIPTOR(0x22)`). Když mu rozumí, čte reporty v **report
protokolu**, který je výchozí po resetu zařízení. Jinak přepne rozhraní do
**boot protokolu** (`SET_PROTOCOL(0)`), pokud ho rozhraní nabízí. Vždy vypne
opakování reportů (`SET_IDLE(0)`), protože klávesy opakuje služba vstupu.
STALL u těchto požadavků nevadí.

**Parser deskriptoru** (`usbproto::report`) projde položky (main, global,
local; long items přeskočí) a sestaví vstupní pole: id reportu, bitovou
pozici, velikost a počet prvků, logický rozsah a usage (seznam nebo
rozsah). Je obranný: nejvýš 256 polí, 1024 usage na pole, 8 kbit na report,
vnoření kolekcí do 32 a zásobník Push/Pop do 16. Poškozený deskriptor
vrátí `None` a ovladač zkusí boot protokol.

<figure class="diagram">
<svg viewBox="0 0 760 200" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Report HID na udalosti">
<defs><marker id="us3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="200" height="70" rx="6"/><text class="t b" x="110" y="45" text-anchor="middle">deskriptor reportu</text><text class="m" x="110" y="64" text-anchor="middle">Usage Page, Usage, Report</text><text class="m" x="110" y="80" text-anchor="middle">Size/Count, Input ...</text>
<rect class="acc" x="270" y="20" width="200" height="70" rx="6"/><text class="t b" x="370" y="45" text-anchor="middle">pole (report::Field)</text><text class="m" x="370" y="64" text-anchor="middle">id, bity, rozsah, usage,</text><text class="m" x="370" y="80" text-anchor="middle">proměnná / pole indexů</text>
<rect class="ok" x="530" y="20" width="220" height="70" rx="6"/><text class="t b" x="640" y="45" text-anchor="middle">události evdev</text><text class="m" x="640" y="64" text-anchor="middle">EV_KEY, EV_REL, EV_ABS,</text><text class="m" x="640" y="80" text-anchor="middle">MSC_SCAN = usage</text>
<path class="ln" d="M210 55 L268 55" marker-end="url(#us3)"/><path class="ln" d="M470 55 L528 55" marker-end="url(#us3)"/>
<rect class="box" x="270" y="120" width="200" height="56" rx="6"/><text class="t b" x="370" y="143" text-anchor="middle">report ze zařízení</text><text class="m" x="370" y="162" text-anchor="middle">bajty (s id reportu)</text>
<path class="ln" d="M470 148 L560 92" marker-end="url(#us3)"/>
<text class="m" x="10" y="115">parse()</text><text class="m" x="490" y="140">Decoder::report()</text>
</svg>
<figcaption>Deskriptor reportu popisuje bity reportu, dekodér z nich dělá události.</figcaption>
</figure>

**Význam polí** (`usbproto::hid_report`):

| Strana | Usage | Událost |
|---|---|---|
| 0x01 Generic Desktop | X, Y relativní | `REL_X`, `REL_Y` (myš) |
| 0x01 | X, Y absolutní | `ABS_X`, `ABS_Y` s rozsahem z logického min/max (tablet) |
| 0x01 | Wheel | `REL_WHEEL` |
| 0x07 Keyboard | 0x04-0x65, 0xE0-0xE7 | klávesy (proměnná = bitmapa, pole = až N stisknutých) |
| 0x09 Button | 1-5 (6-15) | `BTN_LEFT` ... `BTN_EXTRA` (`BTN_MISC`+) |
| 0x0C Consumer | hlasitost, ztlumení, přehrávání, jas, pošta, kalkulačka, hledání; AC Pan | multimediální klávesy; `REL_HWHEEL` |
| 0x0D Digitizer | Tip Switch, Barrel, In Range | `BTN_TOUCH`, `BTN_STYLUS`, `BTN_TOOL_PEN` |

- Proměnná pole dávají událost při změně, relativní osy při každé nenulové
  hodnotě.
- Pole indexů (stisknuté klávesy) se porovnají s minulým reportem: nejdřív
  uvolnění, pak stisky. Report „rollover“ (usage 1-3) se ignoruje.
- `MSC_SCAN` = usage (strana << 16 | id), takže `u` má scan kód `0x70018`
  a hlasitost+ `0xC00E9`.
- Jeden report je jeden snímek.
- **Schopnosti** zdroje určí dekodér: `KEYBOARD` (strana Keyboard),
  `MEDIA` (jen klávesy strany Consumer, třeba ovládání hlasitosti na
  sluchátkách), `POINTER`, nebo `ABSOLUTE`. Rozsahy absolutních os ovladač pošle službě zprávou `IAXS`
  a služba je převede na `Absolute { x, y }` v rozsahu 0-65535.
- **Boot protokol** (`usbproto::hid`): klávesnice 8 B (modifikátory,
  rezerva, 6 kláves), myš 3-4 B. Používá se jen tehdy, když rozhraní nemá
  použitelný deskriptor.
- **Odpojení:** kanál rozhraní se zavře, ovladač pošle uvolnění všech
  držených kláves a tlačítek a skončí. Služba vstupu pak zdroj odebere.

## Ověření nového vstupního zařízení (proti BadUSB)

Zařízení USB se může vydávat za klávesnici a samo psát příkazy (BadUSB,
„Rubber Ducky“), nebo za myš a klikat do dialogů. **Volba zabezpečení**
„ověřovat nová vstupní zařízení“ to zastaví:

<figure class="diagram">
<svg viewBox="0 0 760 210" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Overeni nove klavesnice">
<defs><marker id="us2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="40" width="170" height="70" rx="6"/><text class="t b" x="95" y="65" text-anchor="middle">nová klávesnice</text><text class="m" x="95" y="84" text-anchor="middle">připojená po startu</text><text class="m" x="95" y="100" text-anchor="middle">(settled)</text>
<rect class="warn" x="250" y="40" width="200" height="70" rx="6"/><text class="t b" x="350" y="65" text-anchor="middle">čeká na ověření</text><text class="m" x="350" y="84" text-anchor="middle">vstup jen na kanál secure</text><text class="m" x="350" y="100" text-anchor="middle">bez zkratky, bez opakování</text>
<rect class="ok" x="540" y="10" width="210" height="56" rx="6"/><text class="t b" x="645" y="33" text-anchor="middle">ověřená (trust)</text><text class="m" x="645" y="52" text-anchor="middle">vstup vlastníkovi fokusu</text>
<rect class="box" x="540" y="90" width="210" height="56" rx="6"/><text class="t b" x="645" y="113" text-anchor="middle">odmítnutá (reject)</text><text class="m" x="645" y="132" text-anchor="middle">vstup se zahazuje</text>
<path class="ln" d="M180 75 L248 75" marker-end="url(#us2)"/>
<path class="ln" d="M450 65 L538 40" marker-end="url(#us2)"/>
<path class="ln" d="M450 90 L538 115" marker-end="url(#us2)"/>
<text class="m" x="10" y="180">Systémový dialog ukáže kód; uživatel ho napíše na nové klávesnici (vstup vidí jen systém) a pak dialog pošle trust.</text>
</svg>
<figcaption>Nová klávesnice dostane slovo až po ověření systémem.</figcaption>
</figure>

1. Init po startu pošle `inputd` příkaz `settled`. Klávesnice připojené do
   té chvíle jsou důvěryhodné, protože je uživatel měl připojené už při
   zapnutí.
2. Se zapnutou volbou (`policy verify-new-devices on`) dostane každá
   další **fyzická** klávesnice nebo myš stav `pending`. Vlastník fokusu
   i kanál `secure` dostanou `NewDevice { source, Pending }`. Klient může
   ukázat třeba „Připojena nová klávesnice“.
3. Všechny události toho zařízení jdou **jen na kanál `secure`**, ne
   vlastníkovi fokusu. Bezpečná zkratka z něj nefunguje, opakování kláves
   také ne, a jeho modifikátory (Ctrl, Shift) se nepřidávají k událostem
   myši.
4. Systém ukáže svůj dialog (kreslí ho kompozitor ve vrstvě, kterou
   aplikace nepřekreslí):
   - **u klávesnice** s náhodným kódem, který uživatel napíše na nové
     klávesnici. Dialog to vidí, protože vstup jde na `secure`;
   - **u myši, tabletu a multimediálních kláves** (`MEDIA`: hlasitost,
     přehrávání; nepíšou text, takže kód opsat nejde) s dotazem, který
     uživatel potvrdí **na už ověřené klávesnici**, protože jejich vlastní
     vstup by mohl být podvržený.

   Pak dialog pošle `trust <id>`. Při nesouhlasu pošle `reject <id>`
   a vstup zařízení se pak zahazuje.
5. Po rozhodnutí přijde `NewDevice { source, Trusted / Rejected }`.

Zařízení, které jen předstírá klávesnici, kód nezná. Než by ho uhodlo,
nic nenapíše. Podvržená myš nic neodklikne, dokud uživatel neodpoví na
své klávesnici. Volba patří do [profilů zabezpečení](navrh-fs-v3.md#výchozí-stav-a-profily):

| Profil | Výchozí stav |
|---|---|
| Přísný | zapnuto |
| Standardní | volba (výchozí vypnuto) |
| Volný | vypnuto |

Jde zapnout i samostatně ve vlastní volbě zabezpečení. Mechanismus je hotový
v `inputd`. Dialog s kódem přijde s GUI (6c). Do té doby ověřuje test
přímo zprávou `trust`.

## Testy

| Kde | Co |
|---|---|
| `cargo test -p usbproto` | deskriptory (konfigurace, alternativní nastavení, poškozené délky), setup pakety, zprávy, hub (deskriptor, stav portu, rychlost, route string a jeho meze), HID boot (rozdíl reportů, rollover, tabulka usage, myš), parser reportu (deskriptory QEMU klávesnice, myši a tabletu, multimédia s id reportu, hodnoty se znaménkem, poškozené deskriptory) |
| `cargo xtask test`, sada `usb` | klávesnice USB ze startu je důvěryhodná; klávesy z QEMU až do služby (zdroj, scan kód HID); hotplug myši (připojení, pohyb, kolečko, odpojení); nová klávesnice a nová myš čekají na ověření (vstup jen na `secure`, po `trust` vlastníkovi fokusu); externí hub s klávesnicí za ním; tablet (absolutní poloha přes report protokol) |

V QEMU je `qemu-xhci` s klávesnicí `usb-kbd` navázanou na konzoli druhé
grafiky (`secondary-vga,id=video1`). Xtask jí přes QMP posílá události
s `device=video1`. Myš a druhou klávesnici test připojí za běhu
(`device_add`, `device_del`). Myš USB v QEMU nejde navázat na konzoli
a brala by si události, které má dostat PS/2.

## Omezení a další fáze

| Fáze | Stav | Co |
|---|---|---|
| 1 | hotovo | řadič xHCI: převzetí od firmwaru, reset, kontexty, fronty, MSI-X, kořenové porty, kontrolní a interrupt přenosy |
| 2 | hotovo | HID boot protokol (klávesnice, myš), hotplug, ověření nového vstupního zařízení |
| 3 | hotovo | externí huby: route string, příznak hubu, počet portů, MTT (Configure Endpoint s endpointy alternativy 1 a pak SET_INTERFACE; hub, který ji odmítne, zůstane s jedním TT) a TT think time v kontextu slotu, TT pro low/full speed za high-speed hubem; odpojení hubu odpojí i zařízení za ním (nejvýš 5 úrovní hubů; port hubu nad 15 nejde adresovat: `usb-hub` ho neresetuje a `ATTACH` by vrátil `NOT_SUPPORTED`; po neúspěšném `ATTACH` ovladač hubu port vypne, aby nenastavené zařízení neodpovídalo na adrese 0) |
| 4 | hotovo | parser deskriptoru reportu HID: multimediální klávesy, myši s více tlačítky a vodorovným kolečkem, tablety a pera (`EV_ABS`, událost `Absolute`) |
| 5 | plán | I2C-HID (touchpady notebooků: potřebuje AML a ovladač I2C) |

Další omezení první verze:

- hub USB 3 (deskriptor 0x2A, jiný formát stavu portu) zatím ne, za
  řadičem USB 3 funguje zařízení USB 3 jen na kořenovém portu,
- dotyk více prsty (Multi-touch, kolekce Finger) a gesta zatím ne, jen
  jedno pero nebo jeden bod,
- bulk a izochronní endpointy zatím ne (disky, zvuk), jeden interrupter,
- jen stránky 4 KiB, cíl přerušení je bootovací CPU,
- kontrolky klávesnice USB (`SET_REPORT`) zatím nesvítí,
- zařízení připojená za běhu předává službě vstupu supervisor initu
  ([Init](init.md)). Disky připojené za běhu zatím jen zaloguje.
