# Vyměnitelná média: flash disky s FAT, exFAT a NTFS

Flash disk nebo karta s cizím souborovým systémem se po zasunutí objeví
v `/media/<jmenovka>` a programy s ním pracují stejně jako se soubory
v domovské složce. Cizí souborový systém přitom nikdy nerozebírá služba
`fsd` ani jádro: každé médium má vlastní proces `volfs`, který vidí jen
kanál k oddílu média.

Vlastnická stránka pro: proces média `volfs`, protokol `volproto`, operace
`MEDIA_*` ve `fsd`, pravidla zápisu na média (`diskguard::media`), testy
médií. Ochrana cizích disků obecně je v [Disky](disky.md#riziko-a-ochrana-cizích-disků),
protokol `fsproto` v [Souborový systém](souborovy-system.md#protokol-fsproto).

## Přehled

<svg viewBox="0 0 760 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Cesta dat vymenneho media">
<defs><marker id="md1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="170" height="70" rx="6"/><text class="t b" x="95" y="45" text-anchor="middle">aplikace, Soubory</text><text class="m" x="95" y="64" text-anchor="middle">relace fsd s cestou</text><text class="m" x="95" y="80" text-anchor="middle">/media (uživatel)</text>
<path class="ln" d="M180 55 L248 55" marker-end="url(#md1)"/><text class="m" x="190" y="46">fsproto</text>
<rect class="acc" x="250" y="20" width="200" height="70" rx="6"/><text class="t b" x="350" y="45" text-anchor="middle">fsd</text><text class="m" x="350" y="64" text-anchor="middle">oprávnění, handly, /media;</text><text class="m" x="350" y="80" text-anchor="middle">cizí FS nerozebírá</text>
<path class="ln" d="M450 55 L538 55" marker-end="url(#md1)"/><text class="m" x="462" y="46">volproto</text>
<rect class="warn" x="540" y="20" width="210" height="70" rx="6"/><text class="t b" x="645" y="45" text-anchor="middle">volfs (1 na médium)</text><text class="m" x="645" y="64" text-anchor="middle">FAT, exFAT, NTFS;</text><text class="m" x="645" y="80" text-anchor="middle">jen kanál oddílu</text>
<path class="ln" d="M645 90 L645 158" marker-end="url(#md1)"/><text class="m" x="652" y="130">blockproto</text>
<rect class="ok" x="540" y="160" width="210" height="80" rx="6"/><text class="t b" x="645" y="185" text-anchor="middle">usb-storage</text><text class="m" x="645" y="204" text-anchor="middle">pohled na oddíl (diskguard):</text><text class="m" x="645" y="220" text-anchor="middle">rozsah a zápis hlídá ovladač</text>
<path class="ln" d="M540 200 L452 200" marker-end="url(#md1)"/><text class="m" x="460" y="192">block-media</text>
<rect class="box" x="250" y="160" width="200" height="80" rx="6"/><text class="t b" x="350" y="185" text-anchor="middle">init</text><text class="m" x="350" y="204" text-anchor="middle">spustí volfs pro oddíl,</text><text class="m" x="350" y="220" text-anchor="middle">předá ho fsd (MEDIA_ADD)</text>
<path class="ln" d="M350 160 L350 92" marker-end="url(#md1)"/>
<text class="m" x="10" y="275">Pád volfs (poškozený obraz) = médium zmizí z /media, fsd i systém běží dál.</text>
</svg>

1. Ovladač disku (dnes `usb-storage`) při hlášení disků najde na
   vyměnitelném médiu oddíly s FAT, exFAT nebo NTFS (`diskguard::media`)
   a pro každý pošle správci zařízení kanál `block-media <fs> <index>
   <model>` **[ověřeno v QEMU]**.
2. init pro každý oddíl spustí proces `volfs <fs> <ro|rw>` jen se dvěma
   handly (kanál oddílu, kanál k fsd) a fsd ho předá operací `MEDIA_ADD`
   **[ověřeno v QEMU]**.
3. fsd médium pojmenuje podle jmenovky svazku (bez jmenovky „USB disk“,
   shoda jmen = „ (2)“) a cesty `/media/<jméno>/...` přeposílá procesu
   média **[ověřeno v QEMU]**.

Test: `cargo xtask test --only "vymenna media"` (sada `vymenna media`,
`user/init/src/tests/removable.rs`).

## Izolace a pád procesu média

- `volfs` dostane jen kanál oddílu (`blockproto`) a kanál k fsd. Nemá
  síť, FS, okna ani jiné disky; rozsah sektorů a povolení zápisu hlídá
  ovladač (pohled `Kind::Media`), ne `volfs` **[ověřeno v QEMU]**.
- Data z média jsou nedůvěryhodná. Chyba parseru je chyba operace, pád
  procesu je ztráta média: fsd odpoví čekajícím relacím chybou a médium
  odebere **[implementováno]** (fuzz poškozených obrazů v host testech
  `cargo test -p volfs`).
- Proces, který neodpoví do 30 s, fsd odpojí **[ověřeno v QEMU]**
  (chyba dvou disků stejného modelu se projevila právě tak).

## Kdy se na médium smí zapisovat

Zapisovat lze jen na **vyměnitelné USB médium** (bit RMB z SCSI INQUIRY)
a jen na oddíl s **FAT nebo exFAT** (typ oddílu i boot sektor sedí)
**[ověřeno v QEMU]**. Vše ostatní je jen pro čtení:

| Médium | Zápis |
|---|---|
| vyměnitelné USB, FAT12/16/32 nebo exFAT | ano **[ověřeno v QEMU]** |
| vyměnitelné USB, NTFS | ne (NTFS jen čtení) **[ověřeno v QEMU]** |
| pevný disk přes USB (bez RMB), interní disk | ne |
| oddíl ESP, rozšířený oddíl MBR, překryvy oddílů | médium se nenabídne |
| disk s oddílem MagnumOpus | médium se nenabídne (vlastní disk) |

Host testy `cargo test -p diskguard --test media` pokrývají interní FAT,
ESP a MBR jen pro čtení.

**[navrženo]** Přísný profil (správa organizace): zápis na médium povolí
uživatel pro každé médium zvlášť (varianta B); výchozí je dnešní pravidlo.

## Vlastník a oprávnění

FAT a exFAT oprávnění nemají, ACL v NTFS se ignorují. Všechny soubory na
médiu patří **přihlášenému uživateli** relace (`Stat.owner` = identita
relace) **[ověřeno v QEMU]**. Je to dočasné řešení odpovídající
„Pracovním složkám“ (médium je místo, kam uživatel ukládá vlastní data);
přesné mapování vlastníků a práv je v odchylkách v
[Architektura](architektura.md#aktuální-odchylky-od-návrhu).

Atribut „jen pro čtení“ souboru se hlásí v `Stat`, rozšířené atributy
(např. původ `sys.origin` stažených souborů, [Stahování](stahovani.md#původ-souboru-sysorigin))
FAT, exFAT ani NTFS neuchovají: kopie z domova na médium původ ztratí
a files-copy to zapíše do logu **[implementováno]**.

## Souborové systémy

| FS | Čtení | Zápis | Implementace |
|---|---|---|---|
| FAT12/16/32, dlouhá jména | ano | ano | crate `fatfs` 0.3.6 (MIT) **[ověřeno v QEMU]** |
| exFAT | ano | ano | vlastní (`user/volfs/src/exfat/`) **[ověřeno v QEMU]** |
| NTFS | ano | ne | crate `ntfs` 0.4.0 (MIT/Apache-2.0) **[ověřeno v QEMU]** |

Licence a patenty: soubor `THIRD-PARTY.md` v kořeni repozitáře (sekce
Souborové systémy vyměnitelných médií). exFAT: specifikace je
veřejná, licence patentů v OIN kryje jen „Linux System“, odhad vypršení
2029-2030; podporu hned rozhodl uživatel 2026-11-16.

### exFAT

Crates exFAT v Rustu nebyly zralé (bez zkrácení souboru, čtení za
`ValidDataLength` nevracelo nuly), proto vlastní implementace ve stejném
procesu média:

- **Spouštěcí oblast:** ověří se kontrolní součet (sektory 0-10 proti
  sektoru 11) a všechna pole proti velikosti oddílu. Poškozená hlavní
  oblast = záložní (sektory 12-23) a svazek jen pro čtení; obě poškozené =
  svazek se nepřipojí **[implementováno]**.
- **Jména:** sady položek (File, Stream Extension, 1-17 File Name)
  s kontrolním součtem a hashem jména; porovnání bez ohledu na velikost
  písmen podle tabulky velkých písmen z média (i s diakritikou). Poškozená
  sada se v seznamu vynechá a její místo zůstane obsazené. Nová jména bez
  znaků zakázaných ve Windows a bez tečky nebo mezery na konci
  **[implementováno]**.
- **Alokace:** bitmapa v paměti (změny hned do sektoru), nové soubory
  souvisle (`NoFatChain`), při nesouvislém růstu převod na řetěz FAT.
  `ValidDataLength`: zvětšení souboru jen nastaví délku, bajty za platnými
  daty se čtou jako nuly; zápis za ně nejdřív doplní nuly
  **[implementováno]**.
- **Bezpečné odpojení:** první změna nastaví příznak „špinavý svazek“,
  odpojení ho smaže a zapíše procento využití. Vytažené médium bez
  odpojení Windows nabídne zkontrolovat **[ověřeno v QEMU]**.
- **Jen pro čtení:** TexFAT (dvě FAT), příznak chyby média, chybějící nebo
  poškozená tabulka velkých písmen; soubory s neznámými nedůležitými
  sekundárními položkami se nemění (`NOT_SUPPORTED`), aby se položky
  neztratily **[implementováno]**.

Host testy `cargo test -p volfs exfat` formátují obraz nezávislým crate
`exfat-fs` (MIT, jen testy) a po zápisu ho jím i čtou: dlouhá česká
jména, vnořené složky, fragmentované soubory, díry, 300 souborů ve
složce (růst složky), přejmenování, plné médium, příznak špinavého
svazku, poškozená spouštěcí oblast a sada položek a 300 náhodně
poškozených obrazů bez pádu. `exfat-fs` neumí přečíst prázdný soubor
s prvním clusterem 0 (specifikace ho dovoluje, Windows ho tak vytváří),
testy proto ověřují jen soubory s daty.

## Zápis a vyrovnávací paměť

`volfs` drží sektory v paměti a na médium je zapíše na konci **každé**
měnící operace; po odpovědi fsd jsou data na médiu. Vytažení flash disku
po dokončené kopii je bezpečné, během kopie ne (FS může zůstat nekonzistentní
jako ve Windows) **[ověřeno v QEMU]**. Chyba uprostřed operace rozpracované
změny zahodí.

## Bezpečné odpojení a vytažení

- **Odpojit** (Soubory, `FsClient::media_remove`): fsd pošle `UNMOUNT`,
  proces zapíše data a označí svazek jako čistý, médium zmizí z `/media`
  **[ověřeno v QEMU]**. Smí ho volat relace s `/media` připojeným pro zápis
  nebo správce.
- **Vytažení bez odpojení:** ovladač `usb-storage` při zavření kanálu USB
  zařízení skončí (`blockproto::server::Disks::gone`), `volfs` skončí
  s kódem 4 a fsd médium odebere; otevřené handly vrací `NOT_FOUND`
  **[ověřeno v QEMU]**.
- **Zasunutí za běhu:** správce zařízení předá nový `block-media` initu,
  ten spustí `volfs` jako při startu **[ověřeno v QEMU]**.

## Protokol volproto

`libs/volproto`: mezi fsd a procesem média, **podle cest a bez stavu**
(otevřené soubory drží fsd jako handly `1 << 62` a výš, proces dostává
cesty). Zpráva = `id u64`, `op u32`, pole; odpověď = `id u64`, stav
`i32`, pole. fsd smí mít rozpracovaných víc požadavků různých relací
a páruje je podle `id`; další požadavek stejné relace čeká na odpověď
(pořadí). Data `READ`/`WRITE` nejvýš 32 KiB.

| Op | Číslo | Pole -> odpověď |
|---|---|---|
| `INFO` | 1 | -> verze, FS, jmenovka, velikost, volno, jen pro čtení |
| `STAT` | 2 | cesta -> `Stat` |
| `LIST` | 3 | cesta -> počet, (jméno, `Stat`)... |
| `READ` | 4 | cesta, offset, délka -> data |
| `WRITE` | 5 | cesta, offset, data -> zapsáno |
| `CREATE` | 6 | cesta, druh, exkluzivně -> () |
| `REMOVE` | 7 | cesta -> () |
| `RENAME` | 8 | z, do, nahradit -> () (v rámci média) |
| `SET_LEN` | 9 | cesta, délka -> () |
| `SYNC` | 10 | -> () |
| `UNMOUNT` | 11 | -> (), proces skončí |
| `OPEN` | 12 | cesta, příznaky -> `Stat` |

Přejmenování mezi médiem a domovem vrací `NOT_SUPPORTED`; Soubory ho
dělají kopií **[ověřeno v QEMU]**.

## Operace fsd

V `fsproto` (podrobně [Souborový systém](souborovy-system.md#protokol-fsproto)):

| Op | Číslo | Kdo | Co |
|---|---|---|---|
| `MEDIA_ADD` | 134 | init (řídicí relace, systém) | jméno, jen pro čtení, kanál procesu média |
| `MEDIA_REMOVE` | 135 | správce, relace s `/media` pro zápis | bezpečné odpojení |
| `MEDIA_LIST` | 136 | každá relace | jméno, jmenovka, FS, jen pro čtení, velikost, volno |

Relace vidí `/media`, jen když ho má v připojených cestách
(`Profile::user_mounts` ho dává uživatelským relacím) **[ověřeno v QEMU]**.

## Testy

- `cargo xtask test --only "vymenna media"`: xtask připraví flash disky
  (`xtask/src/mediadisks.rs`): FAT32 s MBR a českým jménem souboru, exFAT
  s MBR (příprava vlastní implementací), NTFS z obrazu `testfs1`
  a superfloppy FAT pro zasunutí za běhu (QMP `device_add`, `device_del`).
  Test čte, zapisuje (dočasné jméno a přejmenování), ověří NTFS jen pro
  čtení, odpojí, zasune a vytáhne médium; xtask pak na hostiteli ověří, že
  zápis na FAT přečte `fatfs` a zápis na exFAT nezávislý `exfat-fs`
  **[ověřeno v QEMU]** (x86_64).
- `cargo test -p volfs`, `cargo test -p diskguard --test media`,
  `cargo test -p volproto`.

## Omezení a další kroky

- **Vytažení během zápisu** může nechat FAT/exFAT nekonzistentní (stejně
  jako jinde); kontrola a oprava (`chkdsk`) zatím není.
- Velké složky se při každé operaci čtou celé (žádná mezipaměť složek):
  tisíce souborů v jedné složce zpomalí kopírování **[implementováno]**.
- Čas FAT je bez pásma a bere se jako UTC (Windows píše místní čas);
  exFAT pásmo ukládá a čte.
- **[navrženo]** Média v Místech Souborů, Odpojit z GUI, oznámení
  o připojení.

## Složka hostitele jako flash disk

`cargo xtask run --share <složka>` připojí složku hostitele jako
vyměnitelný flash disk USB (QEMU vvfat: FAT16 s MBR, nejvýš 504 MB dat)
**jen pro čtení**: disk má `readonly=on`, ovladač hlásí ochranu proti
zápisu a médium je v `/media` jen pro čtení **[implementováno]**.
`--share-rw <složka>` povolí zápis; xtask při startu vypíše varování, že
zápis vvfat je v QEMU experimentální a může poškodit soubory ve složce
(jen zálohovaná složka) **[implementováno]**.
