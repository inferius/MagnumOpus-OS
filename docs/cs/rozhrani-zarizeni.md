# Jednotná rozhraní zařízení a služeb

**Stav: [navrženo]** (2026-11-08). Nic z této stránky zatím není
implementované, pokud to není výslovně označené. Vychází z existující
sady [driverkit](ovladace.md) a protokolů zařízení.

Cíl: ovladače jsou oddělené od zbytku systému jednotným rozhraním podle
třídy zařízení a aplikace se k hardwaru nikdy nedostanou přímo, jen přes
systémové služby se standardním API. Uživatel tak dostane stejné chování
bez ohledu na výrobce hardwaru i na aplikaci (hovor je hovor, ať přijde
odkudkoli; kamera má vždy stejné povolení a indikátor).

## Tři vrstvy

<figure class="diagram">
<svg viewBox="0 0 780 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tři vrstvy rozhraní">
<defs><marker id="rz1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="10" width="760" height="60" rx="6"/>
<text class="t b" x="390" y="34" text-anchor="middle">aplikace</text>
<text class="m" x="390" y="54" text-anchor="middle">jen API služeb + capability z manifestu; žádný přístup k ovladačům</text>
<rect class="acc" x="10" y="110" width="760" height="70" rx="6"/>
<text class="t b" x="390" y="134" text-anchor="middle">systémové služby (vrstva 2)</text>
<text class="m" x="390" y="154" text-anchor="middle">kamera · mikrofon · hovory · poloha · přehrávání · oznámení · výběr souboru · sdílení obrazovky</text>
<text class="m" x="390" y="170" text-anchor="middle">povolení, indikátor, souběh, poskytovatelé (aplikace se registrují jako zdroj hovorů, médií...)</text>
<rect class="ok" x="10" y="220" width="760" height="50" rx="6"/>
<text class="t b" x="390" y="242" text-anchor="middle">rozhraní tříd zařízení (vrstva 1)</text>
<text class="m" x="390" y="260" text-anchor="middle">displej · blokové úložiště · síť · zvuk · vstup · kamera · ...: stejné pro všechny výrobce</text>
<rect class="box" x="10" y="290" width="240" height="34" rx="6"/><text class="m" x="130" y="312" text-anchor="middle">naše ovladače v Rustu</text>
<rect class="box" x="270" y="290" width="240" height="34" rx="6"/><text class="m" x="390" y="312" text-anchor="middle">převzaté C (LinuxKPI, izolovaně)</text>
<rect class="box" x="530" y="290" width="240" height="34" rx="6"/><text class="m" x="650" y="312" text-anchor="middle">virtuální (QEMU, VMware)</text>
<line class="ln" x1="390" y1="70" x2="390" y2="106" marker-end="url(#rz1)"/>
<line class="ln" x1="390" y1="180" x2="390" y2="216" marker-end="url(#rz1)"/>
</svg>
<figcaption>Aplikace mluví se službou, služba s rozhraním třídy, třída s ovladačem.</figcaption>
</figure>

## Vrstva 1: jednotný model ovladačů

Navazuje na `driverkit`; nový framework nevzniká, rozšiřuje se stávající.

1. **Společný životní cyklus**: nalezení, spuštění, připojení klienta,
   zastavení, odpojení, reset, obnova po pádu. Framework uvolní prostředky
   (DMA, přerušení, mapování) vždy, i po chybě.
   Dnes: `probe -> start -> události -> stop` a restart přes devmgr
   **[implementováno]**; na driverkit jsou převedené jen `virtio-rng`,
   `virtio-input` a `edu`.
2. **Rozhraní podle třídy**: displej, blokové úložiště, síť, zvuk, vstup
   (později kamera, senzory, tiskárna). Služby mluví s třídou, ne
   s výrobcem. Dnes: protokoly `blockproto`, `netproto`, `audioproto`,
   `inputproto` a protokol displeje existují **[implementováno]**, každý
   ale jinak.
3. **Asynchronní požadavky a události** se společnou hlavičkou: id
   požadavku, **generace spojení** (zvýší se s každým restartem ovladače),
   třída, verze, výsledek. Výsledek rozlišuje **dokončeno**, **neprovedeno**
   a **výsledek neznámý** (ovladač mohl operaci provést a spadnout před
   potvrzením). Každá operace v protokolu třídy určuje, zda je **bezpečně
   opakovatelná** (idempotentní, např. čtení bloku) a zda jde **její
   výsledek dohledat** (např. stav zápisu podle ID). Klient po restartu
   (nová generace) opakuje jen opakovatelné operace, ostatní dohledá,
   a co dohledat nejde, vrátí volajícímu jako „výsledek neznámý“ - nikdy
   tiché opakování s rizikem dvojího účinku.
4. **Vyjednání schopností a verzí**: ovladač ohlásí funkce, formáty
   a limity. Rozšíření mají vlastní ID a verzi; nepodporované rozšíření
   má jednoznačný výsledek (`NOT_SUPPORTED`), ne tiché ignorování.
   Dnes: jen bity schopností displeje (`VSYNC`, `HOTPLUG`, `MULTI`).
5. **Datová cesta bez zbytečných kopií**: velká data přes sdílené buffery
   a handly. Rozhraní definuje **celý životní cyklus bufferu**: kdo ho
   přiděluje (a z jaké paměti), kdo do něj smí zapisovat a kdy, jak se
   označí dokončení zápisu (plot), kdy ho smí převzít zařízení (displej,
   disk), kdy je bezpečné ho znovu použít (plot uvolnění), a jeho formát,
   rozložení v paměti (řádek, zarovnání, dlaždice) a kompatibilitu mezi
   zařízením, které kreslí, a zařízením, které zobrazuje. Dnes: disky
   kopírují data zprávou kanálu (zapsaná odchylka od návrhu).
6. **Připojení manifestem**: běžný ovladač = kód + manifest + testy, bez
   úprav jádra a bez zvláštních větví ve službách. Dnes: manifest
   **[implementováno]**, ale devmgr má vestavěnou tabulku ovladačů a init
   zvláštní volbu displeje.
7. **Společné testy shody** pro každou třídu (protokol, životní cyklus,
   chybové stavy, limity prostředků, obnova), spustitelné v simulaci
   hardwaru na hostiteli. Autor ovladače doplní jen testy svého hardwaru.

Převzaté velké ovladače (grafika AMD/NVIDIA, Wi-Fi) musí poskytovat stejné
rozhraní třídy jako naše; služby rozdíl nepoznají.

**Pořadí převodu**: blokové úložiště (největší zisk: konec kopírování
zprávou), displej, síť, zvuk, vstup. Každá třída zvlášť, starý protokol
přes adaptér do převedení všech ovladačů.

## Displej bez zbytečné kopie

Dnes **[implementováno]**: kompozitor skládá do své paměti (stín) a ovladač
při `PRESENT` kopíruje poškozené obdélníky do zadního bufferu, navíc
i obdélníky předchozího snímku (zadní buffer je o snímek pozadu).

Úplně odstranit kopii nejde všude: softwarové skládání **čte** cílové
pixely (průhlednost, stíny) a čtení z paměti karty přes BAR
(write-combining nebo bez cache) je o řád až dva pomalejší než z RAM.
Proto podle druhu paměti:

| Kde je obraz | Cesta | Kopie |
|---|---|---|
| systémová RAM, kterou karta umí zobrazit (Intel integrovaná přes GGTT, virtio-gpu `SET_SCANOUT_BLOB`) | kompozitor kreslí přímo do bufferu ovladače, `PRESENT` = přepnutí | žádná |
| paměť karty (bochs-display, Intel DG2, `fbdisplay`) | skládání v RAM, kopie poškozených obdélníků | jedna kopie; viz stáří bufferu níže |
| skládání na GPU (`libs/vkcomp`) | GPU kreslí do bufferu způsobilého k zobrazení, který displej právě nečte; po plotu dokončení se přepne | žádná |

**Stáří bufferu (buffer age)** samo opravu staršího obsahu neodstraní:
při střídání dvou bufferů obsahuje právě získaný zadní buffer obraz starý
dva snímky. Musí se do něj promítnout **všechny oblasti změněné od jeho
posledního použití** (sjednocení poškození posledních N snímků, kde N je
stáří) - překreslením nebo kopií. Příklad: snímek 1 změní levou část,
snímek 2 pravou; do bufferu pro snímek 2 musí jít levá i pravá, jinak se
levá vrátí do starého stavu. Stáří jen určuje, kolik snímků poškození
sjednotit (neznámé stáří = celý buffer); úspora je v tom, že se
nekopíruje nic, co se nezměnilo.

Ovladač nabídne kompozitoru seznam svých bufferů s druhem paměti
a kompozitor zvolí cestu. Rozhraní displeje tím zůstane jedno pro CPU
i GPU skládání.

### Videorovina (navrženo)

**[navrženo]** Video se nemá skládat jako obyčejná vrstva: dekódovaný snímek 4K by se
každý snímek kopíroval a skládal procesorem. Kompozitor proto umí
**videorovinu**: buffer z hardwarového dekodéru jde přímo do roviny
displeje (overlay), pokud ho displej umí zobrazit, jinak do skládání
na GPU. Bez obou cest se video zmenší na rozlišení, které procesor
stihne, a uživatel to vidí v informacích o přehrávání.

| Co | Pravidlo |
|---|---|
| zdroj | buffer dekodéru ve formátu YUV (NV12, P010), bez převodu na RGB v procesoru |
| HDR | metadata (PQ, HLG, jas) jdou s bufferem; kompozitor přepne výstup do HDR, jen když je video v popředí a displej to umí |
| obnovovací frekvence | displej se může přepnout na násobek snímkové frekvence videa (24, 25, 50 Hz), jen na celé obrazovce |
| titulky a ovládání | kreslí se jako obyčejná vrstva nad videorovinou |
| ochrana obsahu | zabezpečená cesta (DRM) se nenavrhuje: certifikaci nový systém nezíská |

Souvisí s [vzhledem pro televizi](vzhled.md#tablet-a-televize-návrh),
ale potřebuje ho každé přehrávání videa na počítači i telefonu.

## Vrstva 2: systémové služby pro aplikace

Aplikace nikdy nedostane capability k ovladači (registry, neomezené DMA).
Dostane kanál ke službě podle oprávnění v manifestu a souhlasu uživatele.
**Služba nesmí být úzké hrdlo**: ověří oprávnění a pak aplikaci předá
**omezenou relaci** - sdílené buffery a frontu příkazů s pevnými mezemi
(formát, velikost, rychlost) - a datová cesta jde bez kopírování přes
službu (grafika, video, zvuk). Služba drží jen řízení (otevření, změna
parametrů, odebrání oprávnění = zneplatnění relace). Služba:

- **rozhoduje o povolení** (jednou, vždy, jen teď) a ukazuje **indikátor**
  (svítí kamera, běží mikrofon, poloha),
- **řeší souběh** (kdo smí mluvit do reproduktoru, hovor ztlumí hudbu),
- **dává jednotné UI** (okno hovoru, ovládání přehrávání v liště, výběr
  souboru),
- **přijímá poskytovatele**: aplikace se registruje jako zdroj (hovorů,
  médií, polohy, tiskárny) a systém s ní zachází stejně jako s vestavěným.

| Služba | Co aplikace dostane | Poskytovatelé |
|---|---|---|
| kamera | proud snímků ve sdílených bufferech, výběr kamery, rozlišení | ovladače kamer (UVC), virtuální kamera aplikace |
| mikrofon a zvuk | proud vzorků přes `audiod`, ztlumení podle priority | zvukové ovladače |
| hovory | systémové okno hovoru, historie, přednost zvuku, odmítnutí | telefon, aplikace VoIP (jako CallKit) |
| poloha | poloha s přesností podle povolení (přibližná / přesná) | GPS, Wi-Fi, ruční |
| přehrávání médií | ovládání v liště a klávesami médií, informace o skladbě | přehrávače |
| oznámení | systémové oznámení, odložení, režim nerušit | každá aplikace |
| výběr souboru (portál) | jen soubory, které uživatel vybral, ne celý disk | FS, cloudové úložiště |
| sdílení obrazovky | jen okno nebo obrazovka, kterou uživatel vybral, s indikátorem | kompozitor |
| schránka, kontakty, tisk | data jen na akci uživatele (vložit, vybrat kontakt) | služby a aplikace |

### Mediální akce a relace přehrávání (navrženo)

**[navrženo]** Služba přehrávání médií drží **relaci přehrávání** každého přehrávače:
co hraje (název, obrázek, délka), pozici, stav a seznam akcí, které
přehrávač umí. Ovládání v liště, klávesy médií, sluchátka, zamykací
obrazovka, dálkový ovladač, doprovodný telefon i hlas jsou jen klienti
téže relace.

Ovládání jde přes **sémantické akce**, ne jako klávesy nebo kliknutí:

| Akce | Parametry |
|---|---|
| `media.play`, `media.pause`, `media.toggle`, `media.stop` | - |
| `media.seek` | pozice nebo posun v ms |
| `media.next`, `media.previous` | - |
| `media.subtitles` | jazyk nebo vypnuto |
| `media.audio_track` | stopa |
| `media.info` | - (ukáže podrobnosti) |

Hlasitost a vypnutí zpracuje systém; aplikace je nedostane. Přehrávač
v manifestu uvede, které akce umí; neznámé akce systém neposílá. Když
hraje víc přehrávačů, akce jde relaci, která naposledy začala hrát
(stejné pravidlo jako souběh zvuku výše).

**Cena**: aplikace mají méně volnosti než na Androidu. Proto musí mít
každá služba rozšíření s verzí (vrstva 1, bod 4), jinak by ji vývojáři
obcházeli.

### Relace okna pro potomky: riziko

**[ověřeno v QEMU]** x86_64. Aplikace smí svým potomkům předat relaci okna
u kompozitoru (role `Gui`, [GUI](gui.md#relace-okna-pro-potomky-role-gui)).
Aplikace tím dál nedostává capability k ovladači displeje ani vstupu:
dostane jen relaci ke službě `compd` se svou identitou.

- ⚠️ **Co tím potomek může:** otevírat okna, číst vstup ve svých oknech
  a používat schránku **pod identitou rodiče** (odznak důvěry ukazuje
  rodičovskou aplikaci). Stejné riziko jako proces, který na Linuxu zdědí
  `DISPLAY` nebo `WAYLAND_DISPLAY`.
- **Co nemůže:** získat širší identitu nebo práva (zúžení je jen prázdné,
  recept ověřuje supervisor), obejít limity oken a bufferů (počítají se na
  identitu) ani přežít odvolání (zruší se celý strom lístků a kompozitor
  zavře všechna spojení identity).
- **Jak dítě bez GUI:** prostředí potomka bez `MO_GUI`.

## Další krok

Podrobný návrh protokolu vrstvy 1 (hlavička, schopnosti, buffery,
testy shody) a převod blokového úložiště; služby vrstvy 2 postupně podle
potřeby aplikací (první: výběr souboru, oznámení, přehrávání médií).
