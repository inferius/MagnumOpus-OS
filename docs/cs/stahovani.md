# Stahování z webu do složky Stažené

Soubory mají příkaz **Stáhnout z adresy…** (menu Soubor, paleta Ctrl+K).
Soubor se uloží do `~/Stažené`, průběh je v Úlohách (zrušit jde ze
Správce úloh), na konci přijde oznámení. Každý stažený soubor si pamatuje,
odkud a kdy přišel (`sys.origin`), a tento údaj nemůže podvrhnout ani
smazat žádná aplikace ani uživatel.

Vlastnická stránka pro: příkaz `save` programu `fetch`, modul initu
`downloads.rs`, příznak připojení `ORIGIN` a atribut `sys.origin`,
proudový příjem těla v `libs/httpc` (`Sink`). Klient HTTP(S) obecně:
[TLS a HTTP](tls-http.md), oznámení: [Oznámení](oznameni.md).

## Přehled

<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Stahovani">
<defs><marker id="dl1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="170" height="70" rx="6"/><text class="t b" x="95" y="45" text-anchor="middle">Soubory</text><text class="m" x="95" y="64" text-anchor="middle">„Stáhnout z adresy…“</text><text class="m" x="95" y="80" text-anchor="middle">save-bg &lt;adresa&gt;</text>
<path class="ln" d="M180 55 L258 55" marker-end="url(#dl1)"/>
<rect class="warn" x="260" y="20" width="220" height="70" rx="6"/><text class="t b" x="370" y="45" text-anchor="middle">fetch (job relace)</text><text class="m" x="370" y="64" text-anchor="middle">síť jen 80/443 a DNS,</text><text class="m" x="370" y="80" text-anchor="middle">FS jen /Stazene (ORIGIN)</text>
<path class="ln" d="M480 40 L578 30" marker-end="url(#dl1)"/>
<path class="ln" d="M480 70 L578 100" marker-end="url(#dl1)"/>
<rect class="ok" x="580" y="10" width="170" height="50" rx="6"/><text class="t b" x="665" y="32" text-anchor="middle">~/Stažené</text><text class="m" x="665" y="50" text-anchor="middle">soubor + sys.origin</text>
<rect class="acc" x="580" y="80" width="170" height="50" rx="6"/><text class="t b" x="665" y="102" text-anchor="middle">taskd, notifyd</text><text class="m" x="665" y="120" text-anchor="middle">průběh, oznámení</text>
<rect class="box" x="260" y="160" width="220" height="70" rx="6"/><text class="t b" x="370" y="185" text-anchor="middle">init (downloads.rs)</text><text class="m" x="370" y="204" text-anchor="middle">spustí fetch pro relaci,</text><text class="m" x="370" y="220" text-anchor="middle">razí relace a kanály</text>
<path class="ln" d="M370 160 L370 92" marker-end="url(#dl1)"/>
</svg>

- **Kdo spouští fetch:** init, při prvním okně Souborů v relaci, v jobu
  relace (stahování přežije zavření okna, skončí s odhlášením)
  **[implementováno]**. Soubory samy relaci s `ORIGIN` ani síťovou relaci
  vyrobit nemůžou. Proto je odchylka od návrhu „Soubory spustí fetch“:
  okno dostane jen kanál pro příkazy.
- **Capability fetch:** síťová relace podle manifestu (jen TCP 80 a 443,
  DNS), `/certs` jen pro čtení, kanál k `revocd`, **relace FS s identitou
  uživatele, která vidí jen `/Stazene`** (`~/Stažené`, příznak
  `mount_flags::ORIGIN`), producent do taskd, odesílatel oznámení. Nic
  jiného z domova nevidí **[ověřeno v QEMU]** (test `stahovani: save:
  soubor, puvod, uloha, oznameni a chyby`).

## Příkaz save

`save <adresa> [jméno]` (odpověď `ok:<jméno> <bajtů>` nebo
`err:<KÓD> <popis pro uživatele>`) a `save-bg` (bez odpovědi; Soubory):

1. jméno z adresy (poslední část cesty, dekódované `%xx`, bez znaků
   zakázaných ve Windows), obsazené jméno = `jméno (2).přípona`; nikdy
   nepřepíše existující soubor **[ověřeno v QEMU]**;
2. tělo jde po částech (`httpc::Sink`, 256 KiB zápisy) do skrytého
   `.jméno.stahuje`, velikost není omezená pamětí procesu **[ověřeno
   v QEMU]** (1 MB přes HTTP/2 a TLS);
3. známá délka a málo místa (méně než velikost + 16 MiB) = chyba hned,
   ne v půlce **[implementováno]**;
4. na konci přejmenování a zápis `sys.origin` = `<adresa>\n<čas ns od
   1970>` **[ověřeno v QEMU]**;
5. chyba = dočasný soubor smazán, úloha selže s popisem, oznámení
   s prioritou High **[ověřeno v QEMU]**.

Srozumitelné chyby: 404 „Soubor na serveru neexistuje (404).“, 401/403,
5xx, „Zabezpečené spojení nejde ověřit: …“ (cizí CA, špatné jméno,
odvolaný certifikát), „Na disku není dost místa.“, „Server je
nedostupný.“, „Adresa serveru neexistuje (DNS).“, přesměrování
z https na http odmítnuto **[implementováno]** (404 a cizí CA
**[ověřeno v QEMU]**).

## Původ souboru sys.origin

- Jmenný prostor `sys.` zapisuje jen systém (`libs/fs`, relace bez
  identity). `sys.origin` navíc smí relace, která cestu vidí přes
  připojení s `mount_flags::ORIGIN` a má k souboru právo zápisu; fsd zápis
  provede jako systém (`serve_hist.rs`, `origin_allowed`)
  **[ověřeno v QEMU]**.
- Uživatel ani aplikace původ nepodvrhnou ani nesmažou (`ACCESS_DENIED`),
  vlastní atributy `user.*` zapisovat smí **[ověřeno v QEMU]**.
- **Kopie** souboru původ nepřenese (relace uživatele `sys.*` zapsat
  nesmí); přesun v rámci FS ho zachová. Kopie na vyměnitelné médium
  (FAT, exFAT, NTFS bez rozšířených atributů) zapíše do logu
  `files-copy: … puvod souboru (sys.origin) se do … neprenese`
  **[implementováno]**. Přenos původu při kopii službou **[navrženo]**.

## Proudový příjem v httpc

`httpc::Sink` dostane stav a hlavičky (`head`, s délkou z
`Content-Length`) a pak data po částech (`data`), u HTTP/1.1 (délka,
chunked, do konce spojení) i HTTP/2. Chyba přijímače (plný disk, zrušení)
ukončí požadavek. `Client::request_to` těla přesměrování přijímači
nepředá. Původní API (`send`, `request`) je obal s `VecSink` a limitem
`Limits::max_body` **[implementováno]** (`cargo test -p httpc sink`).

## Omezení

- Příkazy jednoho fetch jdou po řadě: druhé stahování čeká na první
  (souběžná stahování **[navrženo]**).
- Navázání přerušeného stahování (`Range`) **[navrženo]**.
- Ikona a akce v oznámení (Otevřít, Ukázat ve složce) **[navrženo]**.
