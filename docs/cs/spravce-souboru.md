# Správce souborů

Aplikace **Soubory** (`user/gui-files`) prochází souborový systém
uživatele a pracuje s ním: složky, datasety, koš, historie verzí. Je
postavená na [knihovně UI](gui.md#nativní-knihovna-ui-libsui) a na
protokolu [fsproto](souborovy-system.md#protokol-fsproto). Kopírování
a přesun dělá pomocný proces **`files-copy`**, takže úloha přežije
zavření okna, je vidět v [přehledu úloh](architektura.md) a po
přerušení pokračuje. Vzor vzhledu je prototyp `design/soubory.html`.

Kód:

| Část | Kde |
|---|---|
| model a rozhraní (testy na hostiteli) | `libs/filesview` |
| stroj kopírování, deník úlohy, protokol s `files-copy` | `libs/fileops` |
| asociace typů a zprávy „otevři cestu“ | `libs/openwith` |
| aplikace (backend nad `FsClient`) | `user/gui-files` |
| pomocný proces kopírování | `user/files-copy` |
| spouštění pro relaci, služba „otevři cestu“ | `user/init/src/files.rs`, `user/init/src/open.rs` |
| testy v QEMU | `user/init/src/tests/gui_files.rs` (sada `gui`) |

<figure class="diagram">
<svg viewBox="0 0 760 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Spravce souboru a jeho procesy">
<defs><marker id="sf1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker><marker id="sf1a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="20" width="210" height="110" rx="6"/><text class="t b" x="115" y="45" text-anchor="middle">gui-files (okno)</text><text class="m" x="115" y="66" text-anchor="middle">filesview: výpis, náhled</text><text class="m" x="115" y="84" text-anchor="middle">koš, verze, schránka okna</text><text class="m" x="115" y="102" text-anchor="middle">akce files.* (paleta, shell)</text>
<rect class="acc" x="280" y="20" width="200" height="110" rx="6"/><text class="t b" x="380" y="45" text-anchor="middle">files-copy</text><text class="m" x="380" y="66" text-anchor="middle">fileops: kopie po krocích</text><text class="m" x="380" y="84" text-anchor="middle">konflikty, obnova</text><text class="m" x="380" y="102" text-anchor="middle">stav v domově</text>
<rect class="box" x="550" y="20" width="200" height="50" rx="6"/><text class="t b" x="650" y="42" text-anchor="middle">taskd</text><text class="m" x="650" y="60" text-anchor="middle">přehled úloh</text>
<rect class="box" x="550" y="90" width="200" height="60" rx="6"/><text class="t b" x="650" y="112" text-anchor="middle">fsd</text><text class="m" x="650" y="130" text-anchor="middle">relace FS uživatele</text>
<rect class="ok" x="10" y="190" width="210" height="80" rx="6"/><text class="t b" x="115" y="214" text-anchor="middle">init</text><text class="m" x="115" y="234" text-anchor="middle">spouští pro relaci</text><text class="m" x="115" y="252" text-anchor="middle">služba „otevři cestu“</text>
<rect class="box" x="280" y="190" width="200" height="80" rx="6"/><text class="t b" x="380" y="214" text-anchor="middle">compd</text><text class="m" x="380" y="234" text-anchor="middle">okno, akce aplikací</text><text class="m" x="380" y="252" text-anchor="middle">přístupnost</text>
<path class="ln-acc" d="M220 75 L278 75" marker-end="url(#sf1a)"/><text class="m" x="249" y="67" text-anchor="middle">úloha</text>
<path class="ln" d="M480 45 L548 45" marker-end="url(#sf1)"/>
<path class="ln" d="M480 110 L548 115" marker-end="url(#sf1)"/>
<path class="ln" d="M220 30 L250 30 L250 10 L700 10 L700 88" marker-end="url(#sf1)"/><text class="m" x="470" y="8" text-anchor="middle">výpis, koš, verze</text>
<path class="ln" d="M115 188 L115 132" marker-end="url(#sf1)"/><text class="m" x="160" y="164" text-anchor="middle">otevři cestu</text>
<path class="ln" d="M220 230 L278 230" marker-end="url(#sf1)"/><text class="m" x="249" y="222" text-anchor="middle">akce</text>
<path class="ln" d="M380 188 L380 132" marker-end="url(#sf1)"/>
<text class="m" x="10" y="292">Okno i files-copy mají vlastní relaci FS s identitou uživatele; žádný z nich nedostane capability k ovladači.</text>
</svg>
<figcaption>Okno a pomocný proces kopírování; init je spouští pro relaci uživatele.</figcaption>
</figure>

## Co umí (v1)

| Vlastnost | Stav |
|---|---|
| postranní panel: Domov, datasety (`DATASETS`), Svazek, Koš | **[ověřeno v QEMU]** (test `gui: compd: spravce souboru...`) |
| drobečková navigace, Zpět, Vpřed, Nahoru (po návratu je vybraná složka, ze které se odešlo) | **[implementováno]** (host `navigate_into_and_up_reselects`) |
| tabulka Jméno, Změněno, Velikost, Druh; řazení klikem na záhlaví, složky vždy první, přirozené pořadí jmen (`2` před `10`) | **[ověřeno v QEMU]** |
| hledání v aktuální složce (bez rozlišení velikosti písmen) | **[implementováno]** (host `natural_sort_dirs_first_and_unknown_last`) |
| náhled: text (prvních 64 KiB, CRLF tolerantně), BMP (rozměry a hloubka), metadata | **[ověřeno v QEMU]** (text), BMP host `preview_text_bmp_binary` |
| nová složka, přejmenování (kontrola jména a kolize), do koše s „Vrátit“ | **[ověřeno v QEMU]** (koš), ostatní host |
| koš všech datasetů: obnovit, trvale smazat (s potvrzením) | **[implementováno]** (host `trash_undo_and_trash_view`) |
| historie verzí souboru: Obnovit, Kopie vedle | **[implementováno]** (host `preview_shows_three_states_and_versions`) |
| kopírovat / vyjmout / vložit přes schránku okna | **[ověřeno v QEMU]** |
| kopie a přesun jako úloha `files-copy` s průběhem, pozastavením a zrušením | **[ověřeno v QEMU]** |
| konflikt: přepsat / přeskočit / ponechat obě, „pro všechny“ | **[ověřeno v QEMU]** (ponechat obě), ostatní host `fileops` |
| obnova úlohy po přerušení (i po restartu) | **[ověřeno v QEMU]** (test `gui: files-copy: uloha s konfliktem prezije restart procesu`) |
| akce aplikace `files.*` (paleta příkazů, asistent, shell) | **[ověřeno v QEMU]** (`files.show`) |
| klávesy aplikace F2, Delete, Backspace, Ctrl+C/X/V | **[navrženo]** - čeká na klávesy aplikace v `libs/ui`; dnes přes paletu (Ctrl+K) a tlačítka |
| schránka mezi okny a aplikacemi (seznam cest) | **[navrženo]** - čeká na schránku s formátem v `libs/ui` |
| tažení myší (uvnitř okna, mezi okny) | **[navrženo]** - čeká na cíl puštění v `libs/ui` |
| pohled ikon (mřížka), miniatury | **[navrženo]** |
| „Zkusit bokem“, převzetí změn, Porovnat verze, úprava práv | **[navrženo]** (prototyp `design/soubory.html`) |
| dialog „Otevřít v…“ (`design/dialogy.html`) | **[navrženo]** - až budou systémové dialogy |

## Tři stavy hodnot

FS u každé hodnoty říká, jestli je **známá**, **skrytá** (existuje, ale
relace ji nesmí vidět, typicky souhrn složky bez práva `MANAGE`), nebo
**nedostupná** (FS ji nemá, např. souhrn na hranici datasetu); masky
`known`/`hidden`, bity `fsproto::avail`. Správce nikdy neukáže nulu
místo neznámé hodnoty:

- v tabulce je místo hodnoty „—“,
- náhled k ní napíše důvod: „Velikost: — (skryto: chybí právo
  Spravovat)“, „— (není k dispozici)“,
- řazení dává neznámé hodnoty vždy na konec (vzestupně i sestupně).

Velikost a počet položek složky jsou okamžité ze souhrnu podstromu
(`tree_bytes`, `tree_files`, `tree_dirs`), nic se neprochází.
**[ověřeno v QEMU]**, host test `preview_shows_three_states_and_versions`.

## Kopírování a přesun

Stroj je v `libs/fileops` (`Job`), běží v `files-copy`. Každý krok udělá
omezenou práci (jedna okamžitá kopie, jedno přejmenování, nebo blok
256 KiB), takže proces mezi kroky stihne zprávy (pozastavit, zrušit,
rozhodnutí konfliktu).

- **Okamžitá kopie FS** (`clone_path`, sdílení bloků) funguje v celém
  svazku i mezi datasety; kopie čtením a zápisem je jen náhrada pro
  `NOT_SUPPORTED` (šifrovaný trezor). **[ověřeno v QEMU]**
- Položka se skládá v dočasném jménu vedle cíle (`.<jméno>.kopie-<úloha>`,
  správce ho ve výpisu skryje) a hotová se přejmenuje: **cíl nikdy není
  napůl**.
- **Přepsat** nikdy nemaže: původní cíl jde do koše (lze vrátit).
- **Přesun** v jednom datasetu je přejmenování (okamžité); mezi datasety
  kopie a zdroj do koše.
- Složku nejde zkopírovat do sebe (chyba položky, ostatní pokračují).
- Kopie do stejné složky dostane volné jméno bez dotazu
  (`zprava (2).txt`).

**Obnova po přerušení.** Stav úlohy (`Job::encode`) se ukládá do
`<domov>/apps/org.magnumopus.files/ulohy/<id>.uloha` po každé položce
a nejpozději po 8 MiB dat; zápis přes dočasný soubor
a `RENAME_REPLACE`. Data i stav jdou přes tentýž FS s atomickými commity
v pořadí, takže stav po výpadku nikdy nepředběhne data. Při startu relace
`files-copy` nedokončené úlohy načte: okamžitá kopie se zopakuje, kopie
čtením pokračuje od uloženého místa, konflikt se zeptá znovu. Host test
`resume_after_interruption_*` přeruší úlohu po **každém** kroku
a ověří stejný výsledek jako bez přerušení. Okno s kraťoučkým výpadkem
mezi dokončením položky a uložením stavu se po obnově zeptá na konflikt
s vlastní kopií (vzácné, nic se neztratí).

**Přehled úloh.** `files-copy` je producent `taskd`: úloha
„Kopírování 3 položek do Projekty“ s průběhem, akce Zrušit, Pozastavit
a Pokračovat ze Správce úloh i z příkazu `tasks`.

## Otevírání souborů a asociace typů

Aplikace v manifestu deklaruje, co otevírá:

```toml
[open]
action = "editor.open"        # akce s povinným parametrem path typu path
extensions = ["txt", "md"]
folders = false               # otevírá složky (správce souborů)
reveal = false                # soubor bez aplikace ukáže (správce)
```

Služba **„otevři cestu“** v initu (`user/init/src/open.rs`) má pro
každou relaci vlastní kanál, takže cestu ověří identitou uživatele
(`STAT` přes jeho relaci FS) a aplikaci případně spustí v jeho relaci.
Postup **[ověřeno v QEMU]** (test spravce souboru, část „otevri cestu“):

1. žádost `open <cesta>` s kanálem pro odpověď;
2. složka → aplikace s `folders`, soubor → první aplikace s jeho
   příponou, jinak aplikace s `reveal` (správce soubor vybere ve složce);
3. init zavolá akci aplikace přes kompozitor; když aplikace neběží,
   spustí ji v relaci a volání zopakuje;
4. odpověď `ok <aplikace>`, `reveal <aplikace>`, nebo `err <důvod>`.

Enter nebo dvojklik na souboru ve správci posílá cestu službě; na
odpověď okno nečeká (služba může volat jeho vlastní akci).

### Rozhraní služby pro klienty

Klient (správce souborů, shell) mluví se službou takto **[ověřeno
v QEMU]** (část „otevri cestu“ testu správce souborů):

| Co | Jak |
|---|---|
| kanál | konec kanálu služby relace: init ho dá procesu ve startovní zprávě (`crate::open::endpoint(session)`; duplikát, každý proces relace má vlastní handle téhož kanálu) |
| žádost | `openwith::msg::request(cesta)` = text `open <absolutní cesta>` (UTF-8, mezery v cestě povolené) a **přesně jeden handle**: konec nového kanálu, na který přijde odpověď. `request_app(cesta)` = `open-app <cesta>`: jen aplikace s asociací, soubor bez ní se neukáže (holá cesta v shellu) |
| odpověď | `openwith::msg::Reply::decode`: `ok <id aplikace>` (otevřeno), `reveal <id aplikace>` (žádná aplikace typ neotevře, soubor ukázal správce), `none <přípona>` (žádná aplikace cestu neotevře: žádost `open-app` nebo chybí správce s `reveal`; nic se neotevřelo; přípona malými písmeny, u složky a souboru bez přípony prázdná), `err <důvod pro člověka>` (cesta neexistuje: `<cesta>: NOT_FOUND`, bez práva, aplikace nejde spustit, neodpověděla do 20 s) |
| identita | cestu ověří init relací FS uživatele, které kanál patří (relativní cestu si klient musí doplnit sám; relativní = `err`) |
| neplatná zpráva | bez handlu se zahodí; s handlem odpověď `err neplatná žádost` |

Shell **[ověřeno v QEMU]** (část „otevri cestu“ testu správce souborů,
`shell_start`): `open` zůstává čtení souboru. Příkaz `start <cesta>`
pošle `open` (jako dvojklik: soubor bez aplikace ukáže správce, složku
otevře správce). Holá cesta k souboru pošle `open-app`; při `none` ukáže
souhrn obsahu jako `open` a poznámku „Pro typ .xyz není aplikace.“
Terminál vypíše, kde se cesta otevřela. Kanál dostane jen okno terminálu
přihlášeného uživatele: init ho přidá termd (štítek
`term::ctl::tag::OPEN`) a termd pošle duplikát každé relaci shellu
(`SessMsg::Open`). Záložní konzole kanál nemá: `start` je tam
`NOT_SUPPORTED` a holá cesta ukáže souhrn. Omezení: kompozitor
vybírá aplikaci podle id, ne podle relace (více současně přihlášených
uživatelů zatím není).

## Výkon

Výpis je seřazený a filtrovaný jednou při změně (O(n log n)); texty
řádků (datum, velikost) se tvoří až pro viditelné řádky a tabulka čte
data líně. Strom přístupnosti nese jen viditelné řádky.

| Měření (složka s 10 000 soubory) | Výsledek |
|---|---|
| host, debug (`36c93f0`): načtení + řazení, přestavění rozhraní | 39 ms + 28 ms, 0,16 ms (test `ten_thousand_items_are_fast`) |
| QEMU TCG, release (`b0ccf9d*`): `LIST_STAT_PAGE` ve fsd | 0,56-0,8 s |
| QEMU TCG, release: akce `files.show` v aplikaci celkem | 0,63-0,87 s |
| QEMU TCG, debug: `LIST_STAT_PAGE` ve fsd | 3,2 s |

Nejdražší je výpis s metadaty ve fsd (každá položka `STAT` po cestě);
zrychlení je práce pro fsd. Překreslení po akci aplikace má dnes
v kompozitoru nebo `libs/ui` prodlevu ~2,6 s bez ohledu na velikost
složky (nahlášeno vláknu GUI); klik myší se překreslí hned.

## Bezpečnost

- Okno i `files-copy` dostanou relaci FS s identitou uživatele
  (`Profile::user_mounts`, celý strom, rozhodují oprávnění), ne
  capability k ovladači disku.
- Odznak „Systém“ kreslí kompozitor (manifest `system = true`).
- Obsah souborů (náhled, BMP) i uložený stav úlohy jsou nedůvěryhodný
  vstup: omezené rozměry, žádná panika, vadný stav se odloží jako
  `.vadna`.
