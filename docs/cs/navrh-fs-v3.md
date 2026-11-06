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
  snapshoty (XChaCha20-Poly1305 nebo AES-256-GCM podle CPU; klíč z hesla
  přes Argon2id, nebo z biometrie/klíče přes systémovou službu).
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
- **Volitelný audit** přístupů k citlivým datasetům.
- **Přehled místa** včetně toho, kolik uvolní smazání každého snapshotu.

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
| 1 - formát | uzly 16 KiB, superblok/ukazatel/záznam v3, příznaky vlastností, datasety a strom datasetů, jména a symlinky, opravy z review, protokol v2 s handly, oprávnění (capabilities, granty, viditelnost, profily) |
| 2 - verze | commity, větve, klony, diff, merge, žurnál změn, automatické snapshoty, koš, historie verzí, vrácení operací |
| 3 - data | komprese, okamžité kopie, deduplikace, automatické díry, metadata 2x a samoléčení, scrub na pozadí |
| 4 - ochrana | šifrované trezory, neměnné snapshoty, kvóty, audit, ověřený systém |
| 5 - ostatní | indexy a štítky, send/receive, transakce, ARC a přednačítání, změna velikosti, vrstva pro git |

Každá fáze končí testy (jednotkové, model, výpadky proudu, v systému),
dokumentací v cs i en a review.

## Otevřené otázky

- Zálohy do cloudu (šifrované, přes send/receive) - až bude síť.
- Víc disků (zrcadlo, RAID): formát připravený, implementace až po fázi 5.
