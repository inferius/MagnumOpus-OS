# Instalace na interní disk

Instalátor zapíše MagnumOpus z [živého USB](zive-usb.md) na interní disk
počítače (NVMe, SATA, disk USB): vlastní oddíl EFI, oddíl s naším
souborovým systémem, zavaděč, nastavení a první účet. Má dvě podoby se
stejnými kroky: textovou (příkaz `install` v terminálu) a grafickou
(okno „Nainstalovat MagnumOpus“ na živém USB, `design/instalace.html`).

Obsah:

1. [Kroky průvodce](#kroky-průvodce)
   - [Grafický průvodce](#grafický-průvodce)
2. [Kam nainstalovat](#kam-nainstalovat): celý disk, volné místo, přeinstalace
3. [Ochrana cizích oddílů](#ochrana-cizích-oddílů): raw kanál a diskguard
4. [Co se zapíše](#co-se-zapíše)
5. [Nabídka firmwaru (NVRAM)](#nabídka-firmwaru-nvram)
6. [Nastavení systému](#nastavení-systému): `system.toml`, klávesnice, pásmo
7. [Instalace bez dotazů](#instalace-bez-dotazů) (`--odpovedi`)
8. [Testy](#testy)
9. [Omezení](#omezení)

## Kroky průvodce

Příkaz `install` v terminálu (spuštěném ze živého USB) vede uživatele
kroky z návrhu **[ověřeno v QEMU]** (x86_64 i aarch64, `--only instalace`, režim
bez dotazů; interaktivní dotazy jen ručně):

1. **Jazyk a klávesnice** - čeština; víc rozložení klávesnice (např.
   `cz` a `us`), u každého náhled řad kláves. Přepínání rozložení
   **levý Alt + levý Shift**. Seznam rozložení dává služba vstupu
   (`libs/keymap`, jeden zdroj).
2. **Kontrola počítače** - seznam disků (model, velikost, co na nich je).
3. **Kam nainstalovat** - volby podle disku, viz níže.
4. **Náhled změn** - každý oddíl s označením *zůstane* / *nový* /
   *smaže se*.
5. **Tvůj účet** - jméno, zobrazované jméno, heslo (dvakrát), volitelně
   PIN. Heslo se zadává skrytě (terminál maskuje znaky, zpráva `Ask` se
   `secret = true`).
6. **Zabezpečení** - vždy text „Disk nebude šifrovaný - šifrování disku
   se připravuje“. Šifrování ani obnovovací klíč (patří k šifrování)
   instalátor zatím nenabízí.
7. **Souhrn** - u celého disku opsat název disku, jinak potvrdit `ano`.
   Je-li na disku jiný systém, otázka, který systém startuje jako výchozí.
8. **Průběh instalace** - osm kroků (čtení systému ze živého USB, tabulka
   oddílů, oddíl EFI, zavaděč a systém, souborový systém, nastavení, účet,
   ověření). Čtení ze flashky je první a nejdelší, než se cokoli zapíše
   (čísla v [Rychlost instalace](#rychlost-instalace)).

<figure class="diagram">
<svg viewBox="0 0 760 210" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Cesta od příkazu install k disku">
<rect class="box" x="10" y="30" width="120" height="60" rx="4"/><text class="t b" x="70" y="55" text-anchor="middle">install</text><text class="m" x="70" y="73" text-anchor="middle">shell (shsess)</text>
<rect class="box" x="170" y="30" width="120" height="60" rx="4"/><text class="t b" x="230" y="55" text-anchor="middle">init</text><text class="m" x="230" y="73" text-anchor="middle">OpenInstaller</text>
<rect class="acc" x="330" y="30" width="140" height="60" rx="4"/><text class="t b" x="400" y="55" text-anchor="middle">installd</text><text class="m" x="400" y="73" text-anchor="middle">plán, zápis, ověření</text>
<rect class="warn" x="510" y="30" width="110" height="60" rx="4"/><text class="t b" x="565" y="55" text-anchor="middle">ovladač disku</text><text class="m" x="565" y="73" text-anchor="middle">raw_write_allowed</text>
<rect class="ok" x="660" y="30" width="90" height="60" rx="4"/><text class="t b" x="705" y="55" text-anchor="middle">disk</text><text class="m" x="705" y="73" text-anchor="middle">GPT + naše</text>
<line x1="130" y1="60" x2="170" y2="60" class="line"/><line x1="290" y1="60" x2="330" y2="60" class="line"/><line x1="470" y1="60" x2="510" y2="60" class="line"/><line x1="620" y1="60" x2="660" y2="60" class="line"/>
<text class="m" x="10" y="125">1. Shell požádá init (powerproto OpenInstaller); init spustí installd při první žádosti a předá mu raw kanály disků.</text>
<text class="m" x="10" y="145">2. Shell mluví s installd protokolem installplan::proto (Disks, Keyboards, Preview, Install -> Progress, Done).</text>
<text class="m" x="10" y="165">3. installd zapisuje přes raw kanál; ovladač pustí jen zápis do GPT a do oddílů instalátoru.</text>
<text class="m" x="10" y="185">4. Rozhodnutí (volby, plán, náhled) jsou v knihovně installplan, testované na hostiteli.</text>
</svg>
<figcaption>Od příkazu <code>install</code> k zápisu na disk.</figcaption>
</figure>

## Grafický průvodce

Na živém USB je v liště a ve spouštěči aplikace **Nainstalovat
MagnumOpus** (`user/gui-install`, ikona `download`). Po přihlášení se
okno otevře samo, stejně jako ostatní aplikace plochy. Na nainstalovaném
systému aplikace v seznamu není (init ji přidá jen při startu ze živé
flashky). Stav: **[ověřeno v QEMU]** (x86_64, `--only instalace-gui`).

- **Stejné kroky jako `install`.** Průvodce (`libs/installview`) se
  službou installd mluví stejným protokolem `installplan::proto`. Plán
  disku, volby, náhled, pravidla jména a hesla jsou v `installplan`
  a v installd. Okno drží jen stav kroků a nic nerozhoduje samo.
- **Krok 1, klávesnice.** Pole časového pásma a tabulka rozložení
  s náhledem řad kláves vybraného rozložení. Tlačítka *Přidat*,
  *Odebrat* a *Nastavit jako hlavní* (první je hlavní). Data dává installd
  z `libs/keymap`, stejně jako službě vstupu, včetně tabulky kláves (kód
  evdev a znak na čtyřech úrovních). Podle ní okno překládá stisky na
  vybrané rozložení (`translate_keys`, Caps Lock jako
  `keymap::Keyboard`) v poli „Vyzkoušej klávesnici“ (háček `key_map`
  pole v `libs/ui`). Živé rozložení systému se nemění.
- **Krok 4, náhled změn.** Mapa disku *Teď* a *Po instalaci*
  (`installplan::map`): úseky v pořadí na disku včetně volného místa,
  nové oddíly označené. Pod mapou je seznam změn *zůstane* / *nový* /
  *smaže se* a potvrzení: u celého disku opsat název disku, jinak
  zaškrtnout „Rozumím“. Graf je pruh `segments` z `libs/ui`:
  MagnumOpus akcentem, cizí systém tónem *Other*, EFI tlumeně, volné
  místo šrafovaně a nové úseky mají v popisku „nový“ (ne jen barvu).
  Celý popisek každého úseku je ve stromu přístupnosti.
- **Krok 6, zabezpečení.** Přepínač *Šifrovat disk* je zašedlý
  a pod ním je vysvětlení (`NO_ENCRYPTION`).
- **Krok 8, průběh.** Pruh průběhu, osm kroků a výsledek.
  installd navíc hlásí instalaci do přehledu úloh (`taskd`) jako úlohu
  „Instalace MagnumOpus na ...“ s průběhem. Je tak vidět ve Správci úloh
  a v terminálu i po zavření okna (zápis běží v installd dál).
- **Úzké okno.** Ve třídě šířky `compact` (pod 600 px) zmizí seznam
  kroků vlevo a nahoře je jen „Krok N z 8: ...“.

Test v QEMU (`cargo xtask test --only instalace-gui`) má dvě části.
Nejdřív **skutečný vstup**: živé USB se souborem odpovědí jen
s `rozhrani = "gui-vstup"` (instalátor běží bez autopilota). Okno se po
přihlášení otevře samo a QMP pošle skutečné události přes `usb-tablet`
a `usb-kbd`. Test klikne do pole časového pásma, stiskne Ctrl+A, napíše
`utc` a klikne na *Pokračovat*. Ověří zalogované pásmo a druhý krok;
snímky jsou `vstup-1.png` a `vstup-2.png`. Kam klikat, zjistí init
inspekcí kompozitoru (poloha okna a strom přístupnosti) a zaloguje
střed pole a tlačítka na obrazovce. Rozlišení se totiž liší: x86_64 má
1280 × 800, aarch64 z živého USB jen ramfb 800 × 600. Rozměr obrazovky
pro souřadnice tabletu xtask čte ze snímku `vstup-1.png`.

Pak **celý průchod**: soubor odpovědí
s `rozhrani = "gui"` dostane okno instalátoru místo textové cesty.
Autopilot (`Installer::autopilot`) vyplní krok podle odpovědí a každých
8 s pokročí o krok (první krok po 12 s, okno se teprve otevírá). U každého
kroku vypíše značku `gui-install: krok N`. Debug build pod TCG kreslí okno
několik sekund, proto xtask uloží snímek kroku N přes QMP až při značce
kroku N+1. Snímky jsou v `target/shots/instalace-gui-<arch>/`:
`krok-N.png`, krok 8 uprostřed zápisu a `hotovo.png` po dokončení. Výsledek
se ověří stejně jako u textové instalace: start bez flashky a kontrola
disku na hostiteli.

## Kam nainstalovat

Volby počítá `installplan::choices` z tabulky GPT disku **[implementováno]**
(host testy `cargo test -p installplan`):

| volba | kdy | co se stane |
|---|---|---|
| Celý disk pro MagnumOpus | disk aspoň 8,5 GiB | nová GPT, ESP 512 MiB + náš oddíl do konce; u disku s daty opsat jeho název |
| Vedle jiného systému (volné místo) | na disku je cizí oddíl a souvislé volné místo aspoň 8,5 GiB | do volného místa přibude naše ESP a náš oddíl; cizí oddíly a jejich položky v GPT zůstanou bajt po bajtu |
| Zmenšit oddíl Windows | vždy **šedá** | „zatím není podporováno - zmenši C: ve Správě disků Windows“ |
| Přeinstalovat systém, data ponechat | na disku už je MagnumOpus | nový zavaděč a systém na naši ESP, oddíl s daty a účty zůstane |
| Smazat celý disk a nainstalovat jen MagnumOpus | disk s jiným systémem nebo starou instalací | jako „celý disk“, vždy s opsáním názvu disku |

Velikosti: ESP **512 MiB** (místo pro sloty A/B síťových aktualizací),
náš oddíl zbytek, aspoň **8 GiB**; oddíly zarovnané na 1 MiB. Cizí ESP
(např. Windows) instalátor **nikdy nezapisuje** - MagnumOpus má vždy vlastní.

## Ochrana cizích oddílů

[diskguard](disky.md#riziko-a-ochrana-cizích-disků) zůstává: cizí disk je
pro běžný systém jen pro čtení. Instalátor dostane od ovladače disku
zvláštní **raw kanál** (celý disk, zpráva `block-raw <velikost sektoru>
<model>`), ale každý zápis kontroluje **ovladač**, ne instalátor
(`diskguard::install::raw_write_allowed`) **[ověřeno v QEMU]**
(`--only instalace`, běh vedle Windows):

- zápis do oblasti GPT (LBA 0-33 a záložní tabulka na konci) je povolený,
- zápis jinam jen do oddílu, který patří instalátoru: typ náš + jméno
  `MagnumOpus`, nebo typ ESP + jméno `MagnumOpus EFI`,
- tabulku si ovladač drží v paměti a po každém zápisu do oblasti GPT ji
  načte znovu.

Chyba v instalátoru tak nepřepíše obsah cizího oddílu. Přepsat celou
tabulku (volba „celý disk“) je jediný krok, který cizí oddíly zruší, a ten
vyžaduje opsání názvu disku.

Pořadí zápisu GPT (`diskguard::install::add_partitions`): nejdřív záložní
tabulka na konci disku, pak hlavní. Zůstane GUID disku, ochranný MBR i
všechny cizí položky; naše položky jdou jen do volných slotů. Přerušení
mezi oběma zápisy nechá platnou aspoň jednu kopii.

## Co se zapíše

<figure class="diagram">
<svg viewBox="0 0 760 200" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Rozložení disku po instalaci vedle Windows">
<rect class="box" x="10" y="30" width="50" height="70" rx="4"/><text class="t b" x="35" y="70" text-anchor="middle">GPT</text>
<rect class="box" x="60" y="30" width="110" height="70" rx="4"/><text class="t b" x="115" y="58" text-anchor="middle">ESP Windows</text><text class="m" x="115" y="78" text-anchor="middle">nezměněno</text>
<rect class="box" x="170" y="30" width="200" height="70" rx="4"/><text class="t b" x="270" y="58" text-anchor="middle">C: (Windows)</text><text class="m" x="270" y="78" text-anchor="middle">nezměněno</text>
<rect class="acc" x="370" y="30" width="120" height="70" rx="4"/><text class="t b" x="430" y="52" text-anchor="middle">MagnumOpus EFI</text><text class="m" x="430" y="70" text-anchor="middle">FAT32, 512 MiB</text><text class="mono m" x="430" y="88" text-anchor="middle">EFI/BOOT, os/</text>
<rect class="ok" x="490" y="30" width="210" height="70" rx="4"/><text class="t b" x="595" y="52" text-anchor="middle">MagnumOpus</text><text class="m" x="595" y="70" text-anchor="middle">náš FS, aspoň 8 GiB</text><text class="mono m" x="595" y="88" text-anchor="middle">/home /system/config /system/log</text>
<rect class="box" x="700" y="30" width="50" height="70" rx="4"/><text class="t b" x="725" y="70" text-anchor="middle">GPT</text>
<text class="m" x="10" y="130">Bývalé volné místo dostane dvě nové položky GPT; ostatní položky, MBR a GUID disku zůstanou.</text>
<text class="m" x="10" y="150">Na ESP: EFI/BOOT/BOOTX64.EFI (BOOTAA64.EFI), os/kernel.elf, os/initfs.img, os/boot.cfg.</text>
<text class="m" x="10" y="170">Initfs instalace nemá vývojový profil (automatické přihlášení) ani soubor odpovědí testu.</text>
</svg>
<figcaption>Disk po instalaci vedle Windows (do volného místa).</figcaption>
</figure>

Postup služby `installd` **[ověřeno v QEMU]** (x86_64 i aarch64, `--only instalace`):

1. tabulka oddílů (nová, nebo přidání do volných slotů),
2. formát ESP (FAT32, `libs/fatw`, jmenovka `MAGNUMOS`; jména souborů 8.3),
3. kopie zavaděče, jádra a initfs z ESP živé flashky; initfs je zvláštní
   soubor `os/instfs.img` bez vývojového profilu,
4. formát našeho oddílu (knihovna `fs`), datasety `/home`,
   `/system/config`, `/system/log`,
5. `/system/config/system.toml`,
6. první účet: stejná knihovna `accounts` jako služba účtů (Argon2id,
   soubor `/system/accounts/users/<uid>`), domovská složka jako
   dataset; práva souborů znovu nastaví init při startu,
7. ověření: tabulka znovu přes diskguard, soubory na ESP přečtené zpět
   z cílového disku a porovnané s otiskem zdroje (délka a XXH3-128
   spočtené hned po přečtení z flashky; zdroj se z paměti uvolní po
   zápisu), náš FS připojený jen pro čtení.

### Rychlost instalace

Systém se z flashky čte jednou (zavaděč, jádro, `os/instfs.img`) a stejná
data jdou na cílovou ESP; ověření čte zpět jen to, co se zapsalo.
**[ověřeno v QEMU]** (x86_64 i aarch64, `--only instalace-gui`):

- čtení z flashky po 1 MiB: `BlockClient::read` pošle 32 požadavků bez
  čekání a `usb-storage` je spojí do jednoho příkazu READ ([Disky](disky.md),
  [USB](usb.md)); přenos dat USB po 32 KiB,
- zápis na cíl bez čekání na každou odpověď (`write_queued`, chyba se
  ohlásí u `flush`, který FAT i FS po zápisu volají),
- ověření otiskem místo druhé kopie v paměti (špička paměti o velikost
  systému nižší).

Změřeno 15. 11. 2026 pod TCG, debug build, stejný obraz (zdroj 108-114 MiB),
revize `ccfdb23` (před) a `ccfdb23` + tato změna (po), čas kroků z logu installd:

| krok | x86_64 před | x86_64 po | aarch64 před | aarch64 po |
|---|---|---|---|---|
| čtení z flashky | 104 s | 61 s | 154 s | 92 s |
| kopie na ESP | 43 s | 23 s | 57 s | 30 s |
| ověření | 51 s | 24 s | 63 s | 34 s |
| instalace celkem | 201 s | 110 s | 277 s | 157 s |

Podmínky: QEMU TCG na hostiteli Windows, 4 vCPU, flashka `usb-storage`
(super speed) na xHCI, cíl NVMe. Pod TCG stojí každý přenos USB
kolem 13 ms a každý příkaz NVMe (32 KiB) kolem 7 ms (debug, `ccfdb23*`)
bez ohledu na velikost dat, proto rozhoduje počet požadavků, ne objem. Další zrychlení
by daly víc přenosů na endpoint najednou v `xhci` a větší příkazy NVMe.

## Nabídka firmwaru (NVRAM)

Instalátor zapíše do `\os\boot.cfg` na naší ESP `nvram = "zapsat"`
a výchozí systém (`vychozi = "magnumopus"` nebo `"jiny"`). Při prvním
startu z interního disku zavaděč (`loader/src/nvram.rs`) **[ověřeno
v QEMU]** (x86_64 OVMF i aarch64 AAVMF, `--only instalace`):

1. když už položka „MagnumOpus“ existuje, nic nepřidá,
2. ověří volné místo v proměnných UEFI (`QueryVariableInfo`, aspoň
   16 KiB) - plná NVRAM umí desku zablokovat,
3. přidá `Boot####` na první volné číslo s cestou vlastního zavaděče,
4. číslo vloží do `BootOrder` na začátek (výchozí MagnumOpus) nebo na
   konec (výchozí jiný systém),
5. v `boot.cfg` přepíše `nvram` na `hotovo` - **jediný pokus**, i když
   zápis selhal (start přes záložní cestu `EFI/BOOT` funguje dál).

Cizí položky `Boot####` se nikdy nemažou ani nepřepisují.

## Nastavení systému

`/system/config/system.toml` zapisuje instalátor a čte init při každém
startu (`installplan::config`) **[ověřeno v QEMU]** (x86_64 i aarch64):

```toml
jazyk = "cs"
klavesnice = ["cz", "us"]
pasmo = "Europe/Prague"
rtc = "utc"
vychozi-system = "magnumopus"
```

Init předá pásmo a hodiny službě času (zpráva `settings`) a seznam
rozložení službě vstupu (řídicí zpráva `layouts cz us`). Chybí-li soubor
(živé USB), platí výchozí hodnoty. Při instalaci vedle jiného systému se
průvodce zeptá, jestli je to Windows; pak zapíše `rtc = "local"` (Windows
drží hodiny v místním čase).

## Instalace bez dotazů

```
install --odpovedi /cesta/odpovedi.toml
```

Soubor má formát `system.toml` a navíc klíče `disk` (model nebo číslo),
`zpusob` (`cely-disk`, `volne-misto`, `preinstalovat`), `potvrzeni`,
`jmeno`, `zobrazovane`, `heslo`, `pin`. Heslo v souboru je jen pro testy
a skripty. Popis klíčů: `libs/installplan/src/answers.rs`.

## Testy

- Host: `cargo test -p installplan` (volby, plán, náhled, nastavení,
  protokol, odpovědi), `cargo test -p diskguard` (`tests/install.rs`:
  pravidlo zápisu, přidání oddílů zachová cizí položky), `cargo test -p
  fatw` (formát FAT32 a čtení zpět).
- Host: `cargo test -p installview` (průvodce v okně s falešnou službou
  installd: celý průchod, klávesnice, úzké okno, chyba, autopilot).
- QEMU: `cargo xtask test --only instalace-gui` - grafický průvodce, viz
  [Grafický průvodce](#grafický-průvodce).
- QEMU: `cargo xtask test --only instalace` (`xtask/src/usb/install_test.rs`):
  1. **prázdný disk** - živé USB + prázdný 12 GiB disk NVMe, instalace
     podle souboru odpovědí, pak start **bez flashky** jen z NVMe:
     zavaděč zapíše NVRAM, fsd připojí náš oddíl, služba účtů načte jeden
     účet a kompozitor ukáže přihlašovací obrazovku. Hostitel pak ověří
     GPT, soubory na ESP, `system.toml` a že heslo účtu sedí
     (`accounts::kdf`),
  2. **vedle Windows** - disk s ESP a oddílem „Windows“ se značkami,
     instalace do volného místa; oddíly Windows i MBR zůstanou bajt po
     bajtu stejné.

Přihlášení vytvořeným účtem se ověřuje na úrovni knihovny účtů na
hostiteli, ne klikáním v GUI.

## Omezení

- Disky se sektorem 4 KiB instalátor zatím odmítne.
- Disky virtio nemají raw kanál (v QEMU testu je cílem NVMe).
- Přerušení instalace se neobnovuje od místa přerušení; bezpečné je jen
  pořadí zápisu GPT. Po přerušení zvolte znovu „celý disk“ nebo
  „přeinstalovat“.
- Šifrování disku a obnovovací klíč zatím nejsou.
- Zmenšení cizího oddílu (NTFS) není; uvolněte místo v druhém systému.
- Instalátor potřebuje náhodu (GUID oddílů, sůl hesla). Počítač bez
  generátoru (aarch64 bez TRNG a RNDR, v QEMU bez virtio-rng) čeká na
  entropii; test v QEMU proto na aarch64 přidává `virtio-rng-pci`.
- Zkušební pole klávesnice nesloží mrtvé klávesy (píše samotné
  znaménko).
- Celý průchod ověřuje autopilot; skutečný vstup (klik, psaní, tlačítko)
  ověřuje krátký test jen v kroku 1.
