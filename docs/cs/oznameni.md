# Oznámení: služba notifyd a vyskakovací karty

Aplikace a služby posílají oznámení („Stahování dokončeno“, „Flash disk
připojen“, „Baterie 5 %“) službě `notifyd`. Kompozitor je ukáže jako
vyskakovací kartu vpravo nahoře a uloží do centra oznámení (zvonek
v liště). Aplikace nemůže podvrhnout, kdo oznámení poslal, ani zahltit
obrazovku.

Vlastnická stránka pro: protokol `libs/notifyproto`, službu `user/notifyd`
(logika `libs/notifycore`), vyskakovací karty v kompozitoru
(`libs/comp/src/toast.rs`). Vzhled karet je v [Vzhled](vzhled.md#oznámení),
centrum oznámení v panelu zvonku patří [GUI](gui.md).

## Přehled

<svg viewBox="0 0 760 260" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Cesta oznameni">
<defs><marker id="on1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="200" height="80" rx="6"/><text class="t b" x="110" y="45" text-anchor="middle">aplikace, služba</text><text class="m" x="110" y="64" text-anchor="middle">kanál odesílatele (štítek</text><text class="m" x="110" y="80" text-anchor="middle">NOTIFY) s identitou od initu</text>
<path class="ln" d="M210 60 L288 60" marker-end="url(#on1)"/><text class="m" x="222" y="52">Send</text>
<rect class="acc" x="290" y="20" width="200" height="80" rx="6"/><text class="t b" x="390" y="45" text-anchor="middle">notifyd</text><text class="m" x="390" y="64" text-anchor="middle">identita, limity, Nerušit,</text><text class="m" x="390" y="80" text-anchor="middle">viditelnost podle uživatele</text>
<path class="ln" d="M490 60 L568 60" marker-end="url(#on1)"/><text class="m" x="500" y="52">ViewReply</text>
<rect class="ok" x="570" y="20" width="180" height="80" rx="6"/><text class="t b" x="660" y="45" text-anchor="middle">compd (comp::toast)</text><text class="m" x="660" y="64" text-anchor="middle">karta vpravo nahoře,</text><text class="m" x="660" y="80" text-anchor="middle">centrum oznámení</text>
<path class="ln" d="M660 100 L660 150 L390 150 L390 102" marker-end="url(#on1)"/><text class="m" x="430" y="142">klik, křížek, Seen (View)</text>
<path class="ln" d="M290 90 L212 90" marker-end="url(#on1)"/><text class="m" x="222" y="110">akce, zavřeno</text>
<rect class="box" x="290" y="180" width="200" height="60" rx="6"/><text class="t b" x="390" y="205" text-anchor="middle">init</text><text class="m" x="390" y="224" text-anchor="middle">jediný razí kanály s identitou</text>
<path class="ln" d="M390 180 L390 102" marker-end="url(#on1)"/>
</svg>

- **Odesílatel** dostane kanál od initu: aplikace při spuštění se štítkem
  `abi::params::tag::NOTIFY` s identitou z manifestu (id, jméno) a uživatelem
  relace; systémové služby identitu `system` (uživatel 0)
  **[ověřeno v QEMU]** (test `oznameni: protokol: identita, viditelnost,
  akce, nahrazeni, Nerusit`).
- **Prohlížeč** (kompozitor pro každou relaci příkazem `notify <uid>`,
  testy) vidí oznámení svého uživatele a systémová **[ověřeno v QEMU]**.
- Řídicí příkazy notifyd (`sender`, `viewer`, `status`) přijímá jen od
  initu na bootstrap kanálu.

## Protokol notifyproto

Zpráva = kód operace `u32` + pole (`libs/wire`). Neplatná zpráva vrátí
`INVALID_ARGS`, služba běží dál **[ověřeno v QEMU]**.

| Strana | Zpráva | Pole | Odpověď |
|---|---|---|---|
| odesílatel | `NOTIFY` (1) | klíč u64, titulek, tělo, priorita, akce (nejvýš 3: id, popisek) | `SENT` (klíč, vyskočí) nebo `RESULT` s chybou |
| odesílatel | `CLOSE` (2) | klíč | - |
| odesílateli | `ACTION` (102) | klíč, id akce (prázdné = klik na oznámení) | - |
| odesílateli | `CLOSED` (103) | klíč, důvod (uživatel, odesílatel, limit) | - |
| prohlížeč | `SUBSCRIBE` (10) | - | `SNAPSHOT` (110, po 32 oznámeních) a `DND` (113), pak změny |
| prohlížeč | `DISMISS` (11), `INVOKE` (12), `DISMISS_ALL` (13) | číslo oznámení, akce | `RESULT` (100) |
| prohlížeč | `SET_DND` (14) | konec (reálný čas ns, 0 = vypnout, `u64::MAX` = do vypnutí) | `RESULT`, `DND` všem prohlížečům uživatele |
| prohlížeč | `SEEN` (15) | číslo | `RESULT`: uživatel oznámení viděl (karta zmizela do centra, otevřené centrum); znovu nevyskočí, v oznámení `seen` |
| prohlížeči | `ADDED` (111), `REMOVED` (112) | oznámení, nebo číslo a důvod | - |

- **Klíč** oznámení je v prostoru identity: stejný klíč nahradí oznámení
  na místě (průběh stahování, `replaced`, karta se jen překreslí) i po
  restartu aplikace **[ověřeno v QEMU]**.
- **Priorita:** `Low` jen do centra, `Normal` vyskočí na 6 s, `High`
  zůstane do zavření, `Urgent` projde i Nerušit a smí ji jen systémová
  identita (aplikaci se sníží na `High`) **[ověřeno v QEMU]**.
- **Texty** se zkrátí (titulek 120 znaků, tělo 1000, popisek akce 40)
  a řídicí znaky se odstraní; prázdný titulek nebo dvě akce se stejným id
  = `INVALID_ARGS` **[implementováno]** (`cargo test -p notifycore`).

## Limity a Nerušit

- Nejvýš **20 oznámení na identitu** a **200 na uživatele**; nejstarší
  vytlačí nové (`CLOSED` s důvodem limit) **[implementováno]**.
- **Rychlost vyskakování:** zásobník 5 karet na identitu, jedna se doplní
  za 3 s. Nad limitem jde oznámení potichu do centra (spam nezaplní
  obrazovku) **[implementováno]** (`cargo test -p notifycore`).
- **Nerušit** je na uživatele (`SET_DND`): oznámení jdou jen do centra
  a nevyskočí ani po skončení Nerušit; `Urgent` ano **[ověřeno v QEMU]**.
- **Celá obrazovka** (automatické Nerušit kompozitoru, [GUI](gui.md)):
  kompozitor kartu neukáže a pošle `SEEN`, kromě `Urgent`
  **[implementováno]** (test `fullscreen_dnd_keeps_quiet_except_urgent`).
- Přihlašovací a zamčená obrazovka karty nekreslí (soukromí).

## Vyskakovací karty

`libs/comp/src/toast.rs` **[implementováno]** (`cargo test -p comp toast`):

- karta šířky `size.notification_w` z tokenů vpravo nahoře, vyjede zprava
  (posun 24 px za 240 milisekund, zpomalení), nejvýš 3 karty pod sebou, další čekají;
- obsah: jméno aplikace, titulek, nejvýš 2 řádky těla, tlačítka akcí
  (první zvýrazněné), křížek; karta agenta AI má levý pruh v barvě `ai`;
- `Normal` po 6 s zmizí do centra (`SEEN`), `High` a `Urgent` zůstanou;
- klik na kartu = `INVOKE` bez akce, na tlačítko = `INVOKE` s akcí, křížek
  = `DISMISS`. **Jen fyzický vstup**: umělý klik (`SYNTHETIC`) akci
  oznámení neprovede (test `synthetic_click_does_not_invoke`).

API pro centrum oznámení (panel zvonku, GUI): `Comp::notifications`,
`notify_dismiss`, `notify_invoke`, `notify_dismiss_all`, `notify_set_dnd`,
`notify_dnd`, `notify_unseen` (počet neviděných pro odznak) a
`notify_mark_seen` (otevřené centrum) **[implementováno]** (test
`unseen_count_and_mark_seen`). Příznak `seen` drží notifyd, takže přežije
restart kompozitoru.

## Omezení

- Stav notifyd je jen v paměti: pád služby oznámení ztratí (supervisor ji
  hlídá, restart **[navrženo]**); kompozitor po novém odběru stav nahradí.
- Ikony aplikací na kartě, zvuk oznámení, seskupení podle aplikace
  a historie po restartu **[navrženo]**.
- Čtečka obrazovky (oblast `status`) **[navrženo]**.
