# Návrh: souborový systém v3

**Stav: návrh k odsouhlasení.** Popisuje, co přibude a změní se proti
současnému formátu v2 ([souborovy-system.md](souborovy-system.md)). Po
schválení se implementuje po fázích (viz [Pořadí implementace](#pořadí-implementace))
a hotové části se přesunou do hlavní kapitoly o FS.

Zásady (odsouhlasené):

- **Dělat věci správně, ne podle Windows nebo Linuxu.** Překlad pro cizí
  systémy řeší ovladače (Linux přes FUSE, Windows přes WinFsp nad `libs/fs`).
- **Ve výchozím stavu naprosto jednoduché** pro uživatele; přísnější nebo
  volnější chování jde nastavit v nastavení systému (profily).
- **Formát se teď smí měnit zadarmo**, proto v něm hned je místo pro vše
  níže, i pro věci implementované později (příznaky vlastností).

## Přehled změn

| Oblast | v2 | v3 |
|---|---|---|
| Uzel stromu | 4 KiB | **16 KiB** (delší jména, mělčí strom, víc extentů v listu) |
| Jednotka verzování | celý svazek | **dataset (subvolume)** = repozitář, trezor, systém |
| Verze | snapshoty jen pro čtení | **commity, větve, klony, diff, merge, žurnál změn, vrstva pro git** |
| Jména | 255 bajtů | **1024 znaků** (max. 4 KiB UTF-8), zakázané i řídicí znaky směru textu |
| Záznam objektu | 64 B | **128 B** (verze, příznaky, čas vytvoření, generace, vlastník, oprávnění) |
| Typy objektů | soubor, adresář | + **symbolický odkaz**, odkaz na dataset |
| Data | extenty | + **komprese**, **sdílené bloky** (okamžité kopie, deduplikace), automatické díry |
| Odolnost | Merkleův strom, návrat o generaci | + **metadata 2x** a samoléčení, scrub na pozadí, formát připravený na víc disků |
| Oprávnění | žádná | **capabilities + granty**, viditelnost, profily |
| Šifrování | žádné | **šifrované trezory** (datasety) |
| Snapshoty | ruční | + **automatické**, **koš**, **neměnné** s nastavitelným zámkem |
| Vyhledávání | - | **indexy**: jména, štítky, metadata, hash obsahu; fulltext jako služba |
| Ostatní | - | kvóty, transakce pro aplikace, send/receive, ověřený systém, změna velikosti za běhu |
| Aplikace | - | **zámky souborů a rozsahů, předalokace, sledování změn bez ztrát, mapování do paměti**, rozlišení velikosti písmen pro dataset, zdraví disku |
| Bezpečnost | - | **audit s hashovým řetězem, původ souboru, detekce ransomwaru, návnady**, expirace souborů, skutečné časy, zákaz spouštění na datech, kvóty aplikací |
| Cloud | - | **šifrovaná záloha, soubory na vyžádání, sync mezi zařízeními, sdílení** (formát rezervovaný, viz [Cloud a síť](#cloud-a-síť)) |
| Protokol | v1: cesty, jeden klient | **v2: handly s právy, víc klientů, stránkování** |

## Formát na disku

- **Blok 4 KiB** (data, metadata místa, log), **uzel stromu 16 KiB** (4
  bloky souvisle). Hodnota položky až ~8 KiB.
- **Superblok v3** přibírá: příznaky vlastností ve třech skupinách (jako
  ext4: *compat* - starší ovladač smí připojit; *ro_compat* - jen pro čtení;
  *incompat* - nesmí), UUID svazku, ukazatel na strom datasetů, seznam
  zařízení svazku, nastavení (komprese, deduplikace), podpis kořene
  (ověřený systém).
- **Ukazatel v3** (40 B): zařízení (16 bitů) + blok (48 bitů), generace
  vzniku, 128bitový součet, délka v blocích, příznaky (komprimováno,
  šifrováno, sdíleno). Číslo zařízení je připravené pro víc disků.
- **Záznam objektu (128 B):** druh, příznaky, velikost, časy (změna obsahu,
  metadat, vytvoření, přístup volitelně), rodič, generace (pro handly),
  souhrny podstromu, číslo popisu oprávnění, vlastník, počet odkazů na
  rozšířené atributy, rezerva.

## Datasety (subvolumy)

<figure class="diagram">
<svg viewBox="0 0 760 210" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Datasety">
<defs><marker id="ds1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="80" width="130" height="44" rx="6"/><text class="t b" x="75" y="107" text-anchor="middle">superblok</text>
<path class="ln" d="M140 102 L188 102" marker-end="url(#ds1)"/>
<rect class="box" x="190" y="70" width="170" height="64" rx="6"/><text class="t b" x="275" y="95" text-anchor="middle">strom datasetů</text><text class="m" x="275" y="114" text-anchor="middle">záznamy, commity, větve</text>
<path class="ln" d="M360 90 L428 40" marker-end="url(#ds1)"/><path class="ln" d="M360 102 L428 102" marker-end="url(#ds1)"/><path class="ln" d="M360 114 L428 164" marker-end="url(#ds1)"/>
<rect class="ok" x="430" y="15" width="320" height="46" rx="6"/><text class="t" x="590" y="36" text-anchor="middle">systém (jen pro čtení, A/B, podepsaný kořen)</text><text class="m" x="590" y="52" text-anchor="middle">vlastní strom metadat</text>
<rect class="box" x="430" y="79" width="320" height="46" rx="6"/><text class="t" x="590" y="100" text-anchor="middle">domov uživatele, projekty (repozitáře)</text><text class="m" x="590" y="116" text-anchor="middle">vlastní strom, historie, větve</text>
<rect class="warn" x="430" y="143" width="320" height="46" rx="6"/><text class="t" x="590" y="164" text-anchor="middle">trezor (šifrovaný, zamčený)</text><text class="m" x="590" y="180" text-anchor="middle">klíč jen v paměti po odemčení</text>
</svg>
<figcaption>Každý dataset má vlastní strom; snapshot, větev i klon je jen další kořen.</figcaption>
</figure>

- **Strom datasetů** (jako root tree v btrfs): pro každý dataset záznam s
  kořenem jeho stromu, původem (z jakého snapshotu je klon), generací
  posledního snapshotu, vlastním deadlistem, nastavením (komprese, kvóta,
  automatické snapshoty, šifrování) a jménem.
- V adresářovém stromu je dataset **položka druhu „dataset“**: cesta do něj
  prostě přejde. Pro uživatele je to obyčejná složka (ikona repozitáře nebo
  zámku).
- **Snapshot, klon, větev:** nový záznam ukazující na existující kořen -
  okamžitě, bez kopírování. Každý dataset má vlastní hranici vlastnictví
  bloků a vlastní deadlisty (model datasetů ZFS), takže zapisovatelné klony
  i nezávislé historie fungují.
- **Převod adresáře na dataset** („udělat z této složky repozitář/trezor“):
  přesunou se položky stromu, data zůstanou na místě. U trezoru se data při
  převodu přešifrují (na pozadí, s průběhem).

## Nativní verzování

<figure class="diagram">
<svg viewBox="0 0 760 170" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Historie commitu">
<defs><marker id="vc1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<circle class="acc" cx="60" cy="70" r="16"/><text class="t" x="60" y="75" text-anchor="middle">A</text>
<circle class="acc" cx="180" cy="70" r="16"/><text class="t" x="180" y="75" text-anchor="middle">B</text>
<circle class="acc" cx="300" cy="40" r="16"/><text class="t" x="300" y="45" text-anchor="middle">C</text>
<circle class="acc" cx="300" cy="110" r="16"/><text class="t" x="300" y="115" text-anchor="middle">D</text>
<circle class="ok" cx="440" cy="70" r="16"/><text class="t" x="440" y="75" text-anchor="middle">M</text>
<path class="ln" d="M164 70 L78 70" marker-end="url(#vc1)"/><path class="ln" d="M285 46 L196 66" marker-end="url(#vc1)"/><path class="ln" d="M285 104 L196 74" marker-end="url(#vc1)"/>
<path class="ln" d="M424 64 L316 44" marker-end="url(#vc1)"/><path class="ln" d="M424 76 L316 106" marker-end="url(#vc1)"/>
<rect class="box" x="520" y="20" width="230" height="40" rx="6"/><text class="t" x="635" y="45" text-anchor="middle">větev main → M</text>
<rect class="warn" x="520" y="80" width="230" height="40" rx="6"/><text class="t" x="635" y="105" text-anchor="middle">pracovní stav = klon M</text>
<text class="m" x="10" y="155">Commit = neměnný kořen + autor, zpráva, čas, rodiče; ID = hash. Šipky vedou k rodičům.</text>
</svg>
<figcaption>Historie je DAG commitů jako v gitu; pracovní stav je zapisovatelný klon.</figcaption>
</figure>

- **Commit** = snapshot datasetu + autor, zpráva, čas, rodiče. ID commitu
  = hash (BLAKE3 nebo XXH3 podle svazku) ze součtu kořene, rodičů a
  metadat - obsahově adresovaný DAG.
- **Větve a tagy** = pojmenované reference na commit. **Pracovní stav** =
  zapisovatelný klon posledního commitu; `checkout` je okamžitý.
- **Diff dvou verzí:** souběžný průchod oběma stromy; fyzicky stejný uzel
  (stejný blok) se přeskočí celý. Cena odpovídá změně, ne velikosti.
  Výsledek: seznam změněných, nových, smazaných a přesunutých objektů (oid
  je stabilní, takže přesun se pozná přesně, ne heuristikou jako v gitu).
- **Merge:** třícestný na úrovni souborů a adresářů (společný předek z DAG);
  textové soubory po řádcích, konflikty jako soubory s označením.
- **Žurnál změn** datasetu: uspořádaný proud (objekt, operace, čas, commit),
  s volitelnou retencí - pro sledování změn, indexování, synchronizaci a
  `status` bez procházení disku.
- **Vrstva pro git** (služba nad FS): (1) ID objektů gitu (SHA-1 a SHA-256)
  se počítají líně a ukládají jako rozšířený atribut; import a export
  repozitáře; (2) push/pull přes síť, až bude TCP/IP. Pro nástroje adaptér
  úložiště objektů (např. backend libgit2).

## Jména, typy, velikosti

- Jméno až **1024 znaků Unicode** (max. 4 KiB UTF-8), NFC, porovnání bez
  ohledu na velikost písmen (beze změny proti v2).
- Zakázané: `/`, řídicí znaky, `.` a `..`, **řídicí znaky směru textu**
  (U+202A-202E, U+2066-2069; jinak jde `faktura[U+202E]fdp.exe` zobrazit jako
  `fakturaexe.pdf`).
- **Symbolické odkazy** (cíl jako text, max. 4 KiB) - potřebují je git
  repozitáře.
- Soubor nejvýš **2^63−1 B**; veškerá aritmetika kontrolovaná, souhrny
  adresářů saturující (řídké soubory mohou mít obří logickou velikost).

## Data: komprese, sdílené bloky, díry

- **Komprese po extentech:** LZ4 (rychlá, výchozí) nebo Zstd (lepší
  poměr), nastavení na dataset. Extent, který se nezmenší aspoň o 1/8,
  zůstane nekomprimovaný (fotky, video). Součet se počítá z dat na disku,
  takže scrub nemusí rozbalovat.
- **Okamžité kopie (reflink):** kopie ukazuje na stejné bloky. Sdílené
  bloky mají počítadlo v **tabulce sdílení** (jen sdílené bloky, jako BRT
  v ZFS). Zápis do kopie jde jako vždy do nového bloku; starému se jen sníží
  počítadlo. Blok se uvolní při nule. Vše v jednom atomickém commitu.
- **Deduplikace:** index „hash obsahu → blok“. S XXH3 se při shodě bloky
  porovnají bajt po bajtu (ochrana proti záměrné kolizi), s BLAKE3 ne.
  Výchozí **na pozadí** (neblokuje zápis, málo paměti); okamžitá při zápisu
  jako volba datasetu. V trezorech jen uvnitř trezoru (jinak by šlo
  odhadnout obsah).
- **Automatické díry:** bloky ze samých nul se neukládají.

## Odolnost

- **Metadata dvakrát** na různých místech disku (výchozí; data volitelně).
  Při chybě součtu FS přečte kopii a poškozené místo přepíše (samoléčení).
- **Scrub na pozadí** s nízkou prioritou I/O; výsledek v přehledu stavu disku.
- **Víc disků:** formát (číslo zařízení v ukazateli, seznam zařízení
  v superbloku) je připravený na zrcadlo a později paritní RAID.
- **Opravy z review:** návrat o generaci jen při `CORRUPTED` a zneplatnění
  novějších slotů; nečitelný slot superbloku = neplatný slot; zapisovatelný
  svazek se zárukou trvanlivosti jen na zařízení se skutečným flush (jinak
  jen pro čtení); kontrola všech hodnot z disku (rozsahy, hloubka stromu,
  cykly rodičů, délky); operace, která selže uprostřed, se vrátí celá.
- **fsync a commit atomické:** dávka logu se odebere až po potvrzení
  zápisu; po nejistém zápisu instance nic dalšího nepotvrdí, dokud neproběhne
  obnova.

## Snapshoty, koš, ochrana proti ransomwaru

- **Automatické snapshoty** podle plánu datasetu (např. hodinové 24 h,
  denní 30 dní, týdenní 6 měsíců) - výchozí pro domov uživatele.
- **Koš:** smazání nic nekopíruje, objekt se přesune do koše datasetu;
  vysypání podle stáří nebo při nedostatku místa.
- **Historie verzí souboru** a **vrácení operací** (smazání, přesun,
  přejmenování) z automatických snapshotů a žurnálu změn.
- **Neměnné snapshoty** - nesmí je smazat ani správce z běžícího systému.
  Režimy (nastavitelné, kombinovatelné):

| Režim | Chování |
|---|---|
| časový zámek | nejde smazat dřív než za N dní |
| limit místa | chrání se nejvýš X % disku; nad limitem systém upozorní a nejstarší chráněné snapshoty jde smazat jen s potvrzením (níže) |
| počet | chrání se posledních N snapshotů |
| potvrzení při startu | předčasné smazání se provede až po restartu a potvrzení v zavaděči (kam malware nedosáhne) |
| bezpečnostní klíč | předčasné smazání po dotyku klíče FIDO2 |

  Výchozí: časový zámek 7 dní + limit 15 % disku + potvrzení při startu
  pro předčasné smazání. Místo se tak nikdy nezablokuje natrvalo.

## Oprávnění a viditelnost

- **Capabilities jako základ:** proces přistupuje jen k tomu, co dostal
  (handle adresáře nebo souboru s právy). Práva se ověří jednou při
  otevření, handle je nese; další operace už nic nekontrolují.
- **Granty pro sdílení mezi uživateli:** vlastník + seznam „uživatel nebo
  skupina → úroveň“, dědičnost do podadresářů, **jen povolení** (žádné
  zákazy). Popisy oprávnění jsou uložené jednou a objekty na ně odkazují
  číslem (tisíce souborů se stejnými právy nestojí nic navíc).
- **Práva:** *Vypsat* (jména v adresáři), *Projít* (k položce se známým
  jménem), *Číst*, *Zapsat*, *Vytvořit*, *Smazat*, *Spustit*, *Spravovat*
  (měnit oprávnění). Uživatelské úrovně: **Zobrazit**, **Upravovat**,
  **Spouštět**, **Spravovat**.
- **Viditelnost:** položka se ve výpisu objeví, jen když k ní uživatel má
  aspoň nějaké právo; cizí soubory nevidí ani jménem a nepočítají se mu do
  souhrnů velikosti.
- **Aplikace:** vlastní adresář + soubory, které uživatel vybere v dialogu
  (powerbox) - bez dotazů na práva.
- **Profily v nastavení systému:**

| Profil | Pro koho | Chování |
|---|---|---|
| **Standardní** (výchozí) | běžné používání | vlastní soubory bez omezení a dotazů; aplikace vidí dokumenty uživatele, systémové a cizí soubory ne |
| **Přísný** | citlivá data, firmy | aplikace v sandboxu (jen dialog výběru), trezory se zamykají při uspání, audit přístupů |
| **Volný** | vývojáři | aplikace vidí celý domov uživatele, méně potvrzování |

## Šifrované trezory

- Trezor = **dataset se šifrováním**: data, jména, metadata i jeho
  snapshoty (XChaCha20-Poly1305 nebo AES-256-GCM podle CPU). Hlavní klíč
  je náhodný a odemykají ho „ochránci“: heslo (Argon2id), povinný
  obnovovací klíč, volitelně FIDO2 nebo TPM (viz
  [Další pojistky](#další-pojistky); TPM nikdy jako jediný).
- Součty v ukazatelích jsou nad zašifrovanými daty + autentizační značka
  šifry, takže scrub a samoléčení fungují bez klíče.
- **Zamčený trezor** je vidět jako složka se zámkem; obsah ani počet
  souborů nejsou vidět. Klíč je jen v paměti po odemčení.
- **Okamžité bezpečné smazání:** smazání klíče = data nevratně nečitelná.
- Pořadí: komprese před šifrováním; deduplikace jen uvnitř trezoru.

## Indexy, štítky, vyhledávání

- **Indexy ve stromu na jednom místě**, aktualizované v témže commitu jako
  data (nikdy zastaralé): jméno → objekty, **štítek → objekty**, druh/čas/
  velikost → objekty, **hash obsahu → objekty** (zároveň tabulka
  deduplikace: „které soubory mají stejný obsah“ okamžitě).
- **Štítky a vlastní metadata** (barva, hodnocení, poznámka) jako rozšířené
  atributy.
- **Fulltext v obsahu** jako služba nad žurnálem změn (ne v jádru FS).

## Další vlastnosti

- **Kvóty** na dataset a uživatele (místo i počet objektů).
- **Transakce pro aplikace:** více zápisů a přejmenování atomicky (vše nebo nic).
- **Send/receive:** rozdíl dvou snapshotů jako proud dat - zálohy a
  synchronizace mezi vlastními zařízeními bez procházení souborů.
- **Ověřený systém:** kořen systémového datasetu podepsaný; zavaděč ho při
  startu ověří (jako dm-verity) - navazuje na A/B aktualizace.
- **Změna velikosti svazku za běhu** (zvětšení i zmenšení).
- **Audit přístupů** - viz [Bezpečnost a audit](#bezpečnost-a-audit).
- **Přehled místa** včetně toho, kolik uvolní smazání každého snapshotu.

## Aplikace a kompatibilita

**Stav: odsouhlaseno.** Věci, které jiné systémy mají a aplikace na ně
spoléhají.

- **Zamykání souborů a rozsahů** (fáze 1e, protokol v2): sdílený zámek
  (čtení) a výhradní zámek (zápis) na celý soubor nebo rozsah bajtů. Zámek
  patří handlu; zavřením handlu nebo pádem procesu zmizí sám.
  - Výchozí **poradní** zámky (jako POSIX `fcntl`): kdo je nekontroluje,
    smí zapisovat.
  - **Povinné** zámky jen pro vrstvu Windows (`LockFileEx`, sdílecí režimy
    `CreateFile`), protože na nich Windows aplikace závisí.
  - Čekání na zámek má časový limit a služba hlídá smyčky čekání (deadlock
    se ohlásí chybou, nezasekne se).
- **Předalokace místa** (fáze 3): `allocate(soubor, od, délka)` rezervuje
  souvislé bloky předem (databáze, stahování, video). Extent má příznak
  **nezapsaný** - čte se jako nuly, dokud se do něj nezapíše; první zápis
  příznak jen shodí. Formát je rezervovaný už teď (příznaky extentu).
- **Sledování změn** (fáze 2): aplikace se přihlásí k odběru adresáře nebo
  datasetu a dostává události (vytvořeno, změněno, smazáno, přesunuto)
  přes kanál. Zdrojem je žurnál změn, takže události **se neztratí** ani
  při přetečení fronty: aplikace se doptá „od verze N“ (na rozdíl od
  inotify, kde přetečení = ztráta událostí).
- **Mapování souborů do paměti** (fáze 5 + jádro): objekt paměti (VMO)
  napojený na soubor, stránky se načítají při přístupu a sdílí se jedna
  cache stránek pro FS i mapování. Potřebují ho spouštění programů (PE/ELF)
  a databáze. Je to hlavně práce v jádře (stránkování na vyžádání).
- **Rozlišování velikosti písmen pro dataset** (fáze 1c): volba datasetu,
  pro git repozitáře a zdrojáky z Linuxu, kde vedle sebe existují `Foo`
  i `foo`. Výchozí zůstává bez rozlišení; změnit jde jen u prázdného
  datasetu.
- **Zdraví disku** (fáze 4): služba čte SMART (SATA) a health log (NVMe),
  sleduje opotřebení SSD, přemapované sektory a chyby z scrubu a varuje
  dřív, než disk selže. Výsledek je v přehledu stavu disku.
- **Pevné hardlinky nebudou:** každý objekt má jednoho rodiče (díky tomu
  fungují souhrny podstromu a viditelnost). Místo nich jsou okamžité kopie
  (sdílené bloky); ovladače pro Linux a Windows hlásí hardlinky jako
  nepodporované.

## Bezpečnost a audit

**Stav: odsouhlaseno, fáze 4** (kromě uvedených výjimek).

### Záznam přístupů (audit)

| Co | Jak |
|---|---|
| Události | otevření, čtení, zápis, vytvoření, smazání, přesun, změna oprávnění, **odepřený pokus** |
| Kdo | aplikace (identita z podpisu), uživatel, proces, handle |
| Úrovně | vypnuto / **jen změny** (výchozí) / i čtení - nastavitelné na dataset; profil Přísný zapne čtení pro citlivé datasety |
| Objem | opakované čtení téhož souboru stejnou aplikací v jedné relaci = jeden záznam |
| Úložiště | zvláštní dataset jen pro připisování, uchování podle dnů nebo místa |
| Odolnost proti úpravám | záznamy tvoří **hashový řetěz** (každý obsahuje hash předchozího) a konec řetězu je v záznamu datasetu, tedy v Merkleově stromu commitu. Smazat log jde jen celý a to je vidět; tiše upravit nejde. |

- **„Kdo četl můj soubor“** přímo ve vlastnostech souboru: posledních
  N přístupů z auditu, srozumitelně („Fotky - čteno aplikací Prohlížeč,
  včera 18:02“).
- Čtení auditu je samo právo *Spravovat* datasetu a samo se zaznamená.

### Původ souboru

- Každý nový soubor dostane rozšířený atribut **`sys.origin`**: odkud přišel
  (URL stažení, aplikace, která ho vytvořila, zdroj kopie, přenosný disk).
  Kopie a přesun ho přenesou; přejmenování ho nezmění.
- **Spustitelný soubor s původem „internet“ nebo „přenosný disk“** se před
  prvním spuštěním zeptá (jako Mark of the Web, ale nejde obejít
  přejmenováním ani kopií uvnitř systému).
- Atribut smí nastavit jen FS a systémové služby, ne aplikace.

### Ochrana proti ransomwaru (detekce chování)

- Služba sleduje žurnál změn: když jedna aplikace během krátké doby
  přepíše nebo přejmenuje mnoho souborů a nový obsah má vysokou entropii
  (vypadá zašifrovaně), FS:
  1. hned udělá **snapshot** dotčených datasetů,
  2. aplikaci **pozastaví** (zápisy čekají),
  3. zeptá se uživatele: pokračovat / zastavit a vrátit změny ze snapshotu.
- **Návnady:** skryté soubory v domově uživatele, na které nic legitimního
  nesahá. Zápis do nich spustí totéž okamžitě.
- Aplikace, které to dělají legitimně (archivátory, šifrování, export
  videa), jde po potvrzení vyjmout.

### Další pojistky

- **Datum expirace souboru** (fáze 2): atribut `sys.expires` - po datu se
  soubor přesune do koše (nebo archivuje). Pro soukromí a dočasné soubory.
- **Skutečné časy:** čas vzniku a generaci nastavuje jen FS. Aplikace smí
  změnit zobrazovaný čas změny (kvůli kompatibilitě), ale skutečné časy
  zůstanou v auditu a v záznamu objektu - podvržení časů je vidět.
- **Zákaz spouštění na datasetu** (fáze 1d, oprávnění): datasety s daty
  (stažené soubory, dokumenty, přenosné disky) mají ve výchozím stavu
  právo *Spustit* vypnuté; spouštět jde jen z datasetů aplikací.
- **Kvóty pro aplikace** (s kvótami): aplikace nezaplní disk (ani omylem
  logy); po limitu dostává `NO_SPACE`, uživatel a systém mají místo dál.
- **Odemykání trezorů - víc „ochránců“ téhož klíče:** data trezoru šifruje
  náhodný hlavní klíč a ten je uložený několikrát, pokaždé zašifrovaný jinak:
  heslem (Argon2id), **obnovovacím klíčem** (vždy, povinně), volitelně
  klíčem FIDO2 a **volitelně TPM** (pohodlné odemčení bez hesla na tomto
  stroji s neporušeným zavaděčem, případně s PINem).
  - TPM je **jen doplněk, nikdy jediný ochránce** - ve výchozím stavu
    vypnutý. Disk přenesený do jiného stroje (nebo nový procesor s jiným
    integrovaným TPM) se odemkne heslem nebo obnovovacím klíčem a systém
    nabídne přidat TPM nového stroje; starý záznam se smaže.
  - Přidání nebo odebrání ochránce nepřešifrovává data, mění jen malý
    záznam s obalenými klíči.

### Rizika (vědomě přijatá)

- **Audit čtení stojí výkon a je citlivý.** Log sám prozrazuje, co kdo
  četl - proto je výchozí „jen změny“, log je chráněný právem *Spravovat*
  a čtení logu se zaznamenává.
- **Detekce ransomwaru má falešné poplachy** (komprese, šifrování, export
  videa). Proto pozastavení a dotaz, nikdy tiché zablokování; výjimky po
  potvrzení. Chytrý útočník může šifrovat pomalu - proti tomu jsou hlavní
  obranou neměnné snapshoty, detekce je jen včasné varování.
- **Původ se ztratí** při kopii přes cizí FS (FAT na flashce). Ovladač pro
  Windows ho přenese jako `Zone.Identifier`, kde to jde.
- **TPM:** aktualizace firmwaru nebo výměna hardwaru změní měření a TPM
  trezor neodemkne. Proto je TPM jen volitelný doplněk a vždy funguje heslo
  a **obnovovací klíč**.
- **Povinné zámky** umí zablokovat jiné aplikace (DoS). Proto jen pro vrstvu
  Windows, vázané na handle a s časovým limitem čekání.

## Cloud a síť

**Stav: odsouhlaseno, formát rezervovaný ve fázi 1, implementace ve fázi 6
(po síťovém stacku).**

Zásady:

- **Pravda je lokálně.** Cloud je další úložiště, ne náhrada FS: bez sítě
  všechno lokální funguje dál, sync dožene změny později.
- **Šifrování u klienta (end-to-end).** Poskytovatel vidí jen zašifrované
  balíky stejné velikosti; jména, struktura ani obsah se k němu nedostanou.
- **Nezávislost na poskytovateli.** Úložiště se připojuje přes adaptér:
  S3 kompatibilní (AWS, Backblaze, MinIO, vlastní server), WebDAV, jiné
  naše zařízení v síti. Adaptér umí jen `put/get/list/delete` balíku.

<figure class="diagram">
<svg viewBox="0 0 760 230" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Cloud a sit">
<defs><marker id="cl1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="30" width="150" height="60" rx="6"/><text class="t b" x="85" y="56" text-anchor="middle">aplikace</text><text class="m" x="85" y="75" text-anchor="middle">čte soubor</text>
<rect class="acc" x="200" y="30" width="150" height="60" rx="6"/><text class="t b" x="275" y="56" text-anchor="middle">fsd + libs/fs</text><text class="m" x="275" y="75" text-anchor="middle">datasety, verze</text>
<rect class="ok" x="390" y="30" width="170" height="60" rx="6"/><text class="t b" x="475" y="56" text-anchor="middle">služba sync</text><text class="m" x="475" y="75" text-anchor="middle">balíky, šifrování</text>
<rect class="warn" x="600" y="30" width="150" height="60" rx="6"/><text class="t b" x="675" y="56" text-anchor="middle">adaptér</text><text class="m" x="675" y="75" text-anchor="middle">S3 / WebDAV / zařízení</text>
<path class="ln" d="M160 60 L198 60" marker-end="url(#cl1)"/>
<path class="ln" d="M350 60 L388 60" marker-end="url(#cl1)"/>
<path class="ln" d="M560 60 L598 60" marker-end="url(#cl1)"/>
<rect class="box" x="200" y="130" width="360" height="70" rx="6"/><text class="t b" x="380" y="155" text-anchor="middle">síťový stack (služba, milník 6)</text><text class="m" x="380" y="175" text-anchor="middle">virtio-net, TCP/IP, DNS, TLS 1.3, HTTP</text>
<path class="ln" d="M475 90 L475 128" marker-end="url(#cl1)"/>
<path class="ln" d="M675 90 L560 150" marker-end="url(#cl1)"/>
<text class="m" x="10" y="220">Soubor na vyžádání: fsd pozná příznak REMOTE, požádá službu sync, ta balík stáhne, ověří a rozšifruje.</text>
</svg>
<figcaption>Cesta dat do cloudu: FS nic o síti neví, vše řeší služba sync a adaptér.</figcaption>
</figure>

### 1. Šifrovaná záloha

- Základ je **send/receive**: rozdíl dvou snapshotů se rozdělí do balíků
  po ~4 MiB, každý se zašifruje (XChaCha20-Poly1305) a pojmenuje **klíčovaným
  hashem BLAKE3** obsahu. Stejný obsah = stejné jméno = **deduplikace mezi
  zálohami i zařízeními**, aniž by poskytovatel poznal, co je uvnitř.
- Index záloh (také šifrovaný) umí **obnovit jeden soubor** bez stažení
  celé zálohy.
- **Pravidla uchování** (např. 7 denních, 4 týdenní, 12 měsíčních) a
  **ověření zálohy**: pravidelné stažení náhodného vzorku a kontrola součtů
  (záloha, která nejde obnovit, není záloha).
- **Neměnné zálohy:** u úložišť s object lock (S3) se balíky zamknou na dobu
  uchování - ransomware ani ukradené heslo k účtu je nesmaže.

### 2. Soubory na vyžádání (placeholdery)

- Objekt s příznakem **REMOTE**: záznam, jméno, velikost i náhled leží
  lokálně, data jen v cloudu (extenty s příznakem ukazatele `PTR_REMOTE`).
- Při čtení fsd požádá službu sync o stažení; aplikace jen čeká. Volba
  **„vždy offline“** (příznak PINNED) drží data lokálně.
- **Automatické uvolnění místa:** při nedostatku místa se z nesdílených,
  dlouho nepoužitých souborů s kopií v cloudu stanou placeholdery.
- Průzkumník ukazuje stav (lokálně / jen v cloudu / synchronizuje se).

### 3. Synchronizace mezi zařízeními

- Synchronizuje se **dataset** navázaný na úložiště. Každé zařízení dělá
  lokální commity (fáze 2) a sync si je vyměňuje jako git: změny, které se
  nepřekrývají, se sloučí samy.
- **Konflikt** (stejný soubor změněný na dvou místech) nikdy neztratí data:
  zůstanou obě verze („Zpráva (konflikt - notebook).docx“) a přijde
  upozornění. Smazání jde přes koš, takže se dá vrátit i na druhém zařízení.
- **Výběr složek** (příznak NOSYNC), omezení rychlosti a pauza na měřené
  (mobilní) síti.

### 4. Sdílení

- Každý dataset má vlastní klíč. Sdílení složky = předání klíče
  zašifrovaného veřejným klíčem příjemce; odebrání = nový klíč (rotace) pro
  další změny.
- Odkaz pro čtení (s platností a heslem) přes vlastní server nebo zařízení.

### 5. Cizí cloudy

- OneDrive, Google Drive, Dropbox přes jejich API jako **samostatné
  adaptéry** v režimu souborů (bez šifrování u klienta, protože soubory mají
  být vidět i na webu poskytovatele). Uživatel to v nastavení uvidí jasně
  označené.

### Co se rezervuje už teď (fáze 1)

| Co | Kde | Význam |
|---|---|---|
| příznak objektu `INODE_REMOTE` (1) | záznam objektu, `flags` | data jen v cloudu |
| příznak objektu `INODE_PINNED` (2) | záznam objektu, `flags` | držet vždy lokálně |
| příznak objektu `INODE_NOSYNC` (4) | záznam objektu, `flags` | nesynchronizovat (dědí se) |
| příznak ukazatele `PTR_REMOTE` (8) | `BlockPtr.flags` | blok není na disku, jen v cloudu |
| druh položky `SYNC` (8) | strom | stav synchronizace objektu: vzdálená verze, značka, čas |
| vlastnost `INCOMPAT_REMOTE` (bit 0) | superblok | svazek obsahuje placeholdery; starší ovladač ho nesmí připojit |
| vazba datasetu na úložiště | záznam datasetu (fáze 1c) | ID úložiště, ID klíče, pravidla |

### Síť

Síť zatím v milnících nebyla; přibyla do **milníku 6** jako služby
v userspace: ovladač virtio-net, TCP/IP stack, DNS, TLS 1.3, HTTP klient.
Aplikace dostanou síť jen přes capability (bez ní žádný socket), takže
firewall je přirozeně per aplikace.

### Rizika (vědomě přijatá, řešená návrhem)

- **Ztráta klíče = ztráta zálohy.** Šifrování u klienta znamená, že
  poskytovatel data neobnoví. Proto se při zapnutí vytvoří **obnovovací
  klíč** (vytisknout / uložit jinam) a bez jeho potvrzení se záloha
  nespustí.
- **Placeholder bez sítě nejde přečíst.** Čtení vrátí novou chybu
  `UNAVAILABLE` (doplní se do ABI ve fázi 6); UI předem ukazuje, co je jen
  v cloudu, a důležité složky jdou připnout.
- **Metadata o provozu:** i šifrovaný cloud vidí, kdy a kolik se nahrává.
  Balíky mají stejnou velikost (doplnění), ale časování skrýt nejde.
- **Cizí cloudy** (bod 5) šifrované nejsou - poskytovatel obsah vidí.

## Výkon

- **Adaptivní cache (ARC)** pro uzly a data místo „při zaplnění vše zahodit“.
- **Přednačítání:** sekvenční čtení se pozná a načítá dopředu; při startu
  systému se zaznamená, co se čte, a příště se načte předem.
- **Paralelní commit:** součty, komprese a šifrování na více jádrech.
- **Přímé I/O** pro databáze a obří soubory (bez mezipaměti).
- **Priority I/O:** čtení pro UI předběhne práci na pozadí (scrub,
  deduplikace, defragmentace).

## Protokol fsproto v2

- **Handly místo cest:** `OPEN(adresář, jméno, práva) -> handle`; handle
  nese oid + generaci a práva, takže otevřený soubor přežije přejmenování
  a operace nevyhodnocuje cestu znovu. Adresář je capability: kdo má jeho
  handle, smí jen pod něj.
- **Nahrazení přejmenováním** (atomické uložení), `O_EXCL` vytvoření.
- **Víc klientů:** každý klient vlastní kanál (`OPEN_SESSION`), čísla
  požadavků, stránkovaný výpis adresáře (kurzor), časové limity; služba
  nikdy nespadne kvůli jednomu klientovi.
- Později: sdílená paměť a fronty požadavků (asynchronní I/O).

## Pořadí implementace

| Fáze | Obsah |
|---|---|
| 1 - formát | uzly 16 KiB, superblok/ukazatel/záznam v3, příznaky vlastností, datasety a strom datasetů (i rozlišení velikosti písmen), jména a symlinky, opravy z review, protokol v2 s handly a zámky, oprávnění (capabilities, granty, viditelnost, profily, zákaz spouštění na datech) |
| 2 - verze | commity, větve, klony, diff, merge, žurnál změn, automatické snapshoty, koš, historie verzí, vrácení operací, sledování změn, expirace souborů |
| 3 - data | komprese, okamžité kopie, deduplikace, automatické díry, metadata 2x a samoléčení, scrub na pozadí, předalokace |
| 4 - ochrana | šifrované trezory (ochránci klíče, volitelně TPM), neměnné snapshoty, kvóty i pro aplikace, audit s hashovým řetězem, původ souboru, detekce ransomwaru a návnady, zdraví disku, ověřený systém |
| 5 - ostatní | indexy a štítky, send/receive, transakce, ARC a přednačítání, změna velikosti, vrstva pro git, mapování souborů do paměti (s jádrem) |
| 6 - cloud (po síti) | služba sync, adaptéry (S3, WebDAV, zařízení), šifrovaná záloha, soubory na vyžádání, sync mezi zařízeními, sdílení, cizí cloudy |

Každá fáze končí testy (jednotkové, model, výpadky proudu, v systému),
dokumentací v cs i en a review.

## Otevřené otázky

- Víc disků (zrcadlo, RAID): formát připravený, implementace až po fázi 5.
