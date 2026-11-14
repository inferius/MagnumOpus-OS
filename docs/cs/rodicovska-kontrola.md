# Rodičovská kontrola

Jak může rodič (nebo jiný dospělý s rolí správy) omezit účet dítěte:
časové limity, filtr webu, schvalování aplikací a přehled aktivity. Celá
stránka je **[navrženo]** (schváleno uživatelem, kód zatím není).

Je to **tentýž mechanismus jako [Správa organizace](sprava-organizace.md)**
v malém: politika účtu, kterou nastaví role správy úkonem správy
([Správa počítače](ucty.md#správa-počítače)), a žádosti, které jdou
**správci, ne dítěti**. Právo instalovat aplikace (úrovně Z katalogu / Žádné)
je popsané v [Správě organizace](sprava-organizace.md#právo-instalovat-aplikace);
tady je zbytek.

## Principy

1. **Únava z dotazů je bezpečnostní problém** ([rozpočet dotazů](architektura.md#bezpečnost)).
   Dítě se nemá ptát, co smí; systém mu to řekne předem a při překročení
   nabídne **žádost rodiči**.
2. **Žádosti jdou rodiči, ne dítěti.** Dítě nikdy nedostane dialog „povolit?“,
   který by mohlo samo odklikat.
3. **Průhlednost.** Dítě vidí, že je účet pod rodičovskou kontrolou a **co se
   sleduje**; skrytá kontrola neexistuje.
4. **Žádný záznam obrazovky ani obsahu zpráv.** Sleduje se čas, aplikace
   a blokované pokusy, nic víc.
5. **Odolnost proti obejití dítětem**, ale s upřímným vyjmenováním mezí
   ([Rizika](#soukromí-a-rizika)).

## Společný mechanismus

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Politika uctu a zadosti">
<defs><marker id="rk1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker><marker id="rk1a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="warn" x="10" y="20" width="200" height="100" rx="6"/><text class="t b" x="110" y="45" text-anchor="middle">rodič (role správy)</text><text class="m" x="110" y="66" text-anchor="middle">nastaví politiku účtu:</text><text class="m" x="110" y="82" text-anchor="middle">čas, web, aplikace</text><text class="m" x="110" y="102" text-anchor="middle">úkon správy (ELEVATE)</text>
<rect class="acc" x="290" y="20" width="200" height="100" rx="6"/><text class="t b" x="390" y="45" text-anchor="middle">accountd</text><text class="m" x="390" y="66" text-anchor="middle">politika účtu dítěte</text><text class="m" x="390" y="82" text-anchor="middle">rozpočet času, žádosti,</text><text class="m" x="390" y="98" text-anchor="middle">audit</text>
<rect class="ok" x="570" y="20" width="180" height="100" rx="6"/><text class="t b" x="660" y="45" text-anchor="middle">vynucení</text><text class="m" x="660" y="66" text-anchor="middle">správce sezení (čas)</text><text class="m" x="660" y="82" text-anchor="middle">netd / fwd / dnsd (web)</text><text class="m" x="660" y="98" text-anchor="middle">FS a balíčky (aplikace)</text>
<path class="ln" d="M210 70 L288 70" marker-end="url(#rk1)"/>
<path class="ln" d="M490 70 L568 70" marker-end="url(#rk1)"/>
<rect class="box" x="10" y="200" width="200" height="100" rx="6"/><text class="t b" x="110" y="225" text-anchor="middle">dítě</text><text class="m" x="110" y="246" text-anchor="middle">vidí: účet je pod kontrolou</text><text class="m" x="110" y="262" text-anchor="middle">a co se sleduje; zbývající</text><text class="m" x="110" y="278" text-anchor="middle">čas; „Požádat o ...“</text>
<rect class="ok" x="290" y="200" width="200" height="100" rx="6"/><text class="t b" x="390" y="225" text-anchor="middle">žádost</text><text class="m" x="390" y="246" text-anchor="middle">„další čas“, „povolit web“,</text><text class="m" x="390" y="262" text-anchor="middle">„nainstalovat aplikaci“</text><text class="m" x="390" y="282" text-anchor="middle">jde rodiči, ne dítěti</text>
<path class="ln-acc" d="M210 250 L288 250" marker-end="url(#rk1a)"/>
<path class="ln-acc" d="M390 200 L390 122" marker-end="url(#rk1a)"/>
<path class="ln-acc" d="M290 90 L215 190" marker-end="url(#rk1a)"/><text class="m" x="224" y="150" text-anchor="start">oznámení v relaci rodiče</text>
</svg>
<figcaption>Rodič nastaví politiku účtu, dítě při překročení nabídne žádost a ta jde rodiči.</figcaption>
</figure>

- **Politika účtu** patří `accountd` a mění se jen úkonem správy
  (`ELEVATE`, potvrzení osobou s rolí). Dítě ji nemůže změnit ani
  přeinstalováním aplikace.
- **Žádosti** („další čas“, „povolit stránku“, „nainstalovat aplikaci X“)
  se ukládají u `accountd` a rodič je vidí jako **oznámení ve své relaci**
  (při přihlášení nebo hned, běží-li). **Oznámení na jiné zařízení**
  **[navrženo]** později. Schválení vyřídí žádost sama (čas se prodlouží,
  doména se povolí, aplikace se nainstaluje).
- **Bez organizace** je to jediný mechanismus správy cizího účtu; **v
  organizaci** stejnou politiku nastaví správa organizace.

## Časové limity

<figure class="diagram">
<svg viewBox="0 0 760 270" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Casovy rozpocet a upozorneni">
<defs><marker id="rk2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="30" width="740" height="40" rx="6"/><text class="m" x="20" y="54">povolené okno (např. 7:00 až 20:00)</text>
<rect class="ok" x="10" y="100" width="420" height="40" rx="6"/><text class="t b" x="220" y="125" text-anchor="middle">denní rozpočet: zbývá čas</text>
<rect class="warn" x="430" y="100" width="90" height="40" rx="6"/><text class="m" x="475" y="125" text-anchor="middle">10 min</text>
<rect class="warn" x="520" y="100" width="60" height="40" rx="6"/><text class="m" x="550" y="125" text-anchor="middle">1 min</text>
<rect class="acc" x="580" y="100" width="170" height="40" rx="6"/><text class="t b" x="665" y="125" text-anchor="middle">zamčení relace</text>
<path class="ln" d="M430 100 L430 82" marker-end="url(#rk2)"/><text class="m" x="440" y="90">upozornění předem</text>
<rect class="ok" x="10" y="180" width="740" height="70" rx="6"/><text class="t b" x="380" y="203" text-anchor="middle">zamčení nabídne „Požádat o další čas“ -> žádost rodiči</text><text class="m" x="380" y="223" text-anchor="middle">měří se aktivní čas relace (nečinnost se nepočítá); rozpočet drží accountd / správce sezení, ne aplikace</text><text class="m" x="380" y="239" text-anchor="middle">výjimky: škola a komunikace s rodiči jsou povolené vždy</text>
</svg>
<figcaption>Denní rozpočet v povoleném okně, upozornění předem, po vypršení zamčení a žádost o další čas.</figcaption>
</figure>

- **Denní rozpočet času** po dnech v týdnu (jiný pro školní den a víkend).
- **Povolené časové okno** (např. 7 až 20 h): mimo ně se relace nespustí
  ani neodemkne.
- **Limity po aplikacích a kategoriích** (hry, video, sociální sítě)
  vedle celkového rozpočtu; kategorie plyne z manifestu aplikace.
- **Výjimky:** školní aplikace a **komunikace s rodiči je vždy povolená**
  (rozpočet se pro ně nespotřebovává).
- **Upozornění předem** po 10 minutách a po 1 minutě před koncem. Po
  vypršení se relace **zamkne** (stav aplikací zůstane, dítě o práci
  nepřijde) a zamčená obrazovka nabídne **„Požádat o další čas“**; žádost
  jde rodiči.
- **Měření:** počítá se **aktivní čas relace**, nečinnost (zamčená
  obrazovka, žádný vstup) se nepočítá. **Rozpočet drží `accountd` a správce
  sezení v initu, ne aplikace**, takže ho aplikace nemůže zdržet ani
  vynulovat.
- **Odolnost proti změně času:** dítě nic nezíská přestavením hodin.
  Rozpočet se měří **monotonním časem** (`CLOCK_MONOTONIC`, jádro), povolené
  okno se vyhodnocuje podle reálného času ze služby `timed` a **změna
  systémového času je úkon správy** ([Správa počítače](ucty.md#správa-počítače)),
  takže ji dítě bez rodiče neprovede ([Reálný čas](cas.md#nastavení-hodin-clock_set)).

## Filtr webu

- **Kde:** v síťové vrstvě, kterou nejde obejít z aplikace: `netd` (jediný
  síťový zásobník) s pravidly `fwd` ([Firewall](firewall.md#filtr-dns))
  a `dnsd` (šifrované DNS, [Síť](sit.md#šifrované-dns-a-dnssec-dnsd)).
  Nově **pravidla podle uživatele** (dnes jsou podle aplikace a profilu
  sítě) **[navrženo]**.
- **Kategorie** (dospělé, hazard, ...) ze seznamu, **vlastní povolené
  a zakázané domény**, **bezpečné vyhledávání vynucené na úrovni DNS**
  (vyhledávač se přesměruje na jeho bezpečnou variantu).
- **Blokovaná stránka** ukáže **srozumitelnou hlášku** („Tuhle stránku
  rodič zakázal“) a **„Požádat o povolení“**, které jde rodiči. Ne tichý
  odmítnutý překlad.
- ⚠️ **Omezení, která je třeba říct výslovně:**
  - Filtr podle domén **nevidí obsah HTTPS**: blokuje jméno, ne stránku.
    Neškodná doména může obsahovat nevhodný obsah.
  - **VPN, proxy a vlastní DoH aplikace** ho mohou obejít. Politika proto
    **zakáže cizí DNS a VPN pro účet** (firewall pravidla podle uživatele:
    jen systémový resolver `dnsd`, žádné tunely).
  - **Aplikace s vlastním prohlížečem** (vlastní DNS, vlastní síťový kód)
    musí jít přes stejná pravidla, jinak by filtr obešla; takové aplikace
    spadají pod právo instalovat
    ([Z katalogu / Žádné](sprava-organizace.md#právo-instalovat-aplikace)).
  - Žádný filtr není dokonalý; rodičovská kontrola doplňuje rozhovor,
    nenahrazuje ho.

## Přehled aktivity pro rodiče

- **Rodič vidí:** **čas po aplikacích** (den, týden), **žádosti** (čekající
  a vyřízené) a **blokované pokusy** (doména, aplikace, čas).
- **Dítě vidí:** že je účet pod rodičovskou kontrolou, **co se sleduje**
  (čas, aplikace, blokované pokusy) a vlastní přehled spotřeby; zbývající
  čas je vidět na liště.
- **Nesleduje se:** **záznam obrazovky**, stisky kláves, **obsah zpráv**
  a obsah stránek. Pro rodičovskou kontrolu tyto kategorie neexistují
  (na rozdíl od [správy organizace](sprava-organizace.md#záznam-obrazovky-a-kláves),
  kde jde o výslovně ohlášenou volbu).

## Soukromí a rizika

- **Data o aktivitě jsou jen lokální** a viditelná jen **roli správy**
  (a dítěti o něm samém); nikam se neposílají. **Uchování je omezené**
  (např. 30 dní), pak se mažou.
- ⚠️ **Obejití dítětem:** jiný počítač nebo telefon mimo systém; VPN nebo
  vlastní DNS, pokud pravidla neplatí; fyzický přístup (přeinstalace systému nebo vyjmutí disku). Kontrola
  nechrání před tím, co leží mimo zařízení.
- ⚠️ **Zneužití rodičem:** silná role nad cizím účtem. Proto je politika
  viditelná dítěti a změny jdou do auditu účtu; skrytá kontrola neexistuje.
- ⚠️ **Falešné pocity bezpečí:** filtr domén a časový limit nenahrazují
  rozhovor s dítětem; stránka to říká výslovně.
- **Právní a věkové otázky** (souhlas, věk plnoletosti) závisí na
  zemi; stránka není právní poradenství.

## Pořadí zavedení a závislosti

Začíná **až po aktuálním pořadí** převzetí cizího software
([std, `cstd`, ...](architektura.md#převzetí-cizího-software-pořadí)) a po
správě organizace, se kterou sdílí politiku účtu.

| Část | Potřebuje |
|---|---|
| politika účtu a žádosti | `accountd` (role, `ELEVATE`, audit), `fsd`, `compd` (oznámení, zamčená obrazovka) |
| časové limity | správce sezení v initu, `timed` (reálný čas), jádro (monotonní čas), `compd` (upozornění na liště) |
| filtr webu | `netd`, `fwd`, `dnsd`, pravidla podle uživatele v `netproto` / `netfilter` |
| přehled aktivity | `accountd`, měření času po aplikacích (kandidát `taskd`, [přehled úloh](architektura.md#přehled-úloh-odsouhlaseno)), `logd` (uchování) |
| právo instalovat | [Správa organizace](sprava-organizace.md#právo-instalovat-aplikace), katalog a balíčky |

## Stav zavedení

| Část | Stav |
|---|---|
| politika účtu a žádosti rodiči | **[navrženo]** |
| časové limity, upozornění, zamčení a žádost o další čas | **[navrženo]** |
| filtr webu (kategorie, vlastní seznamy, bezpečné vyhledávání, pravidla podle uživatele) | **[navrženo]** |
| přehled aktivity a průhlednost pro dítě | **[navrženo]** |
