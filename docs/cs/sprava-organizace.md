# Správa organizace

Jak může firma nebo škola spravovat **své** počítače, aniž by systém
přestal chránit lidi, kteří je používají: zápis počítače do správy
organizace, firemní záloha klíče domova, sledování koncových bodů (EDR)
a výslovná rizika. Celá stránka je **[navrženo]** (schváleno uživatelem,
kód zatím není).

Navazuje na [Uživatelské účty](ucty.md) (správa počítače bez roota, šifrování
domova), na model [Bezpečnosti](architektura.md#bezpečnost) a na
[Obnovu služeb](obnova-sluzeb.md) (supervisor restartuje agenta).

## Principy

1. **Osobní počítač nikdy.** Funkce z této stránky má jen počítač, který
   byl do správy organizace výslovně **zapsán**. Žádná z nich není ve
   výchozím systému zapnutá a žádná se nedá zapnout vzdáleně na počítači,
   který zapsaný není.
2. **Uživatel vždy vidí, co se děje.** Zapsaný počítač to ukazuje
   v nastavení („Počítač spravuje organizace X, sleduje: ...“) a trvalým
   indikátorem na liště. Skrytá správa neexistuje.
3. **Žádný kód organizace v jádře.** Agent organizace je obyčejný proces
   v userspace s capability; jádro ani ovladač od třetí strany se
   nepřidává (poučení z [CrowdStrike 2024](#sledování-koncových-bodů-edr)).
4. **Silná oprávnění jsou zaznamenaná a viditelná.** Odběr událostí,
   obnova klíče domova i změna politiky jdou do auditu a uživatel je vidí.
5. **Soukromé klíče organizace nejsou v počítači.** Kradež počítače nebo
   databáze účtů organizaci ani útočníkovi nic nedá.

## Zápis počítače do správy organizace

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Zapis pocitace do spravy organizace">
<defs><marker id="so1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker><marker id="so1a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="210" height="110" rx="6"/><text class="t b" x="115" y="45" text-anchor="middle">organizace X</text><text class="m" x="115" y="66" text-anchor="middle">veřejný klíč organizace</text><text class="m" x="115" y="84" text-anchor="middle">politiky (podepsané)</text><text class="m" x="115" y="102" text-anchor="middle">jednorázový zápisový kód</text><text class="m" x="115" y="120" text-anchor="middle">soukromý klíč: mimo síť</text>
<rect class="warn" x="290" y="20" width="200" height="110" rx="6"/><text class="t b" x="390" y="45" text-anchor="middle">systémový dialog</text><text class="m" x="390" y="66" text-anchor="middle">„Zapsat počítač do správy</text><text class="m" x="390" y="82" text-anchor="middle">organizace X?“ + co bude</text><text class="m" x="390" y="98" text-anchor="middle">sledováno; potvrdí osoba</text><text class="m" x="390" y="114" text-anchor="middle">s rolí správy (ELEVATE)</text>
<rect class="acc" x="560" y="20" width="190" height="110" rx="6"/><text class="t b" x="655" y="45" text-anchor="middle">/system/org</text><text class="m" x="655" y="66" text-anchor="middle">záznam o zápisu</text><text class="m" x="655" y="84" text-anchor="middle">veřejný klíč (kotva důvěry)</text><text class="m" x="655" y="102" text-anchor="middle">politiky, ověřené podpisem</text><text class="m" x="655" y="120" text-anchor="middle">relace FS jen pro tuto službu</text>
<path class="ln" d="M220 75 L288 75" marker-end="url(#so1)"/>
<path class="ln" d="M490 75 L558 75" marker-end="url(#so1)"/>
<rect class="ok" x="10" y="190" width="230" height="110" rx="6"/><text class="t b" x="125" y="215" text-anchor="middle">nastavení uživatele</text><text class="m" x="125" y="236" text-anchor="middle">„Počítač spravuje</text><text class="m" x="125" y="252" text-anchor="middle">organizace X, sleduje: ...“</text><text class="m" x="125" y="274" text-anchor="middle">seznam politik, audit</text>
<rect class="ok" x="290" y="190" width="200" height="110" rx="6"/><text class="t b" x="390" y="215" text-anchor="middle">lišta (compd)</text><text class="m" x="390" y="236" text-anchor="middle">trvalý indikátor</text><text class="m" x="390" y="252" text-anchor="middle">„spravováno“; klik ukáže</text><text class="m" x="390" y="268" text-anchor="middle">co a kým se sleduje</text>
<rect class="box" x="560" y="190" width="190" height="110" rx="6"/><text class="t b" x="655" y="215" text-anchor="middle">osobní počítač</text><text class="m" x="655" y="236" text-anchor="middle">bez záznamu: žádná z těchto</text><text class="m" x="655" y="252" text-anchor="middle">funkcí, capability k odběru</text><text class="m" x="655" y="268" text-anchor="middle">události nikdo nevydá</text>
<path class="ln-acc" d="M655 130 L655 160 L125 160 L125 188" marker-end="url(#so1a)"/>
<path class="ln-acc" d="M655 160 L390 160 L390 188" marker-end="url(#so1a)"/>
<text class="m" x="520" y="154" text-anchor="middle">čte accountd / compd</text>
</svg>
<figcaption>Zápis potvrdí člověk u počítače; od té chvíle ho uživatel vidí v nastavení i na liště.</figcaption>
</figure>

- **Zápis (enrollment)** je úkon pro celý počítač: jde přes `ELEVATE`
  a systémový dialog stejně jako ostatní správa
  ([Správa počítače](ucty.md#správa-počítače)). Dialog ukáže organizaci
  a **seznam toho, co se bude sledovat**; aplikace ani malware ho
  nepodvrhne. Zápisový kód je jednorázový a časově omezený.
- **Záznam o zápisu** (`/system/org`) obsahuje veřejný klíč organizace
  jako kotvu důvěry a politiky podepsané soukromým klíčem organizace.
  Počítač politiky přijme, jen když podpis sedí. Jméno organizace
  a seznam sledovaného bere nastavení i lišta **z tohoto záznamu**, ne
  z agenta, takže agent nemůže zobrazit méně, než sleduje.
- **Politiky organizace** jsou data: které kategorie událostí se
  sledují, výchozí chování `AUTH` při nedostupném agentu (fail-open /
  fail-closed), zda se zálohuje klíč domova, povolené aplikace.
  Změna politiky jde do auditu a uživatel ji vidí.
- **Identita organizace je její veřejný klíč.** Politika, escrow i povolení
  odběru jsou **podepsané tímto klíčem**; počítač přijme jen podepsanou
  aktualizaci politiky, nepodepsanou zahodí a zaloguje.
- **Tři cesty zápisu** **[navrženo]**:
  1. **V instalátoru:** volba „Tento počítač patří organizaci“. Zadá se
     adresa organizace nebo kód či QR od IT, počítač stáhne podepsanou
     politiku, **ukáže rozsah** (co organizace uvidí a smí) a zápis se
     potvrdí **před založením prvního účtu**.
  2. **Později v Nastavení:** „Připojit k organizaci“ je úkon správy přes
     `ELEVATE`. Ostatní uživatelé počítače se to **dozvědí při dalším
     přihlášení**.
  3. **Zero-touch:** IT si vyrobí vlastní instalační obraz nebo USB
     s vloženým profilem organizace (obdoba Apple ADE nebo Windows
     Autopilot, ale bez databáze výrobců: důvěra je jen v klíč
     organizace vložený do obrazu).
- **Odpojení** je úkon `ELEVATE` s potvrzením. Podle politiky může
  vyžadovat **podpis organizace** (otevřená otázka 1). Odpojení **smaže
  firemní obal klíčů a vypne odběr událostí**.
- ⚠️ **Upřímné omezení:** kdo má k počítači fyzický přístup a roli správy,
  může systém **přeinstalovat** a tím správu zrušit. **Firemní data zůstanou
  chráněná**: šifrovaný domov a firemní úložiště přeinstalací
  nezískáte, protože klíče drží organizace.
- **Osobní počítač** záznam nemá: capability pro odběr událostí ani
  firemní obal klíče nikdo nevydá a služby, které by je používaly, se
  nespustí.

## Firemní záloha klíče domova (escrow)

Klíč domova je už dnes náhodný (256 bitů) a má **obal pro každou metodu**
ověření ([Šifrování domova](ucty.md#šifrování-domova)). Firemní záloha
přidá **další obal**: klíč domova zabalený **veřejným klíčem organizace**
(asymetricky, např. HPKE s X25519, RFC 9180).

<figure class="diagram">
<svg viewBox="0 0 760 340" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Firemni obal klice domova">
<defs><marker id="so2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker><marker id="so2a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="ok" x="10" y="20" width="170" height="80" rx="6"/><text class="t b" x="95" y="45" text-anchor="middle">klíč domova</text><text class="m" x="95" y="64" text-anchor="middle">256 bitů, náhodný</text><text class="m" x="95" y="80" text-anchor="middle">(jak dnes)</text>
<rect class="box" x="250" y="20" width="210" height="110" rx="6"/><text class="t b" x="355" y="43" text-anchor="middle">obaly uživatele (v DB)</text><text class="m" x="355" y="64" text-anchor="middle">heslo, PIN, bezpečnostní klíč,</text><text class="m" x="355" y="80" text-anchor="middle">obnovovací klíč</text><text class="m" x="355" y="102" text-anchor="middle">beze změny</text>
<rect class="acc" x="250" y="150" width="210" height="80" rx="6"/><text class="t b" x="355" y="173" text-anchor="middle">obal organizace (nový)</text><text class="m" x="355" y="193" text-anchor="middle">HPKE: veřejný klíč organizace</text><text class="m" x="355" y="209" text-anchor="middle">jen se zapsaným počítačem</text>
<path class="ln" d="M180 50 L248 55" marker-end="url(#so2)"/>
<path class="ln-acc" d="M120 100 L120 190 L248 190" marker-end="url(#so2a)"/>
<rect class="warn" x="530" y="20" width="220" height="210" rx="6"/><text class="t b" x="640" y="43" text-anchor="middle">soukromá část: nikdy v PC</text><text class="m" x="640" y="68" text-anchor="middle">A  offline (sejf)</text><text class="m" x="640" y="88" text-anchor="middle">B  HSM</text><text class="m" x="640" y="108" text-anchor="middle">C  YubiKey</text><text class="m" x="640" y="128" text-anchor="middle">D  Shamir k z n mezi</text><text class="m" x="640" y="144" text-anchor="middle">více lidmi</text><text class="m" x="640" y="176" text-anchor="middle">krádež PC nebo DB účtů</text><text class="m" x="640" y="192" text-anchor="middle">nedá nic</text>
<path class="ln-acc" d="M460 190 L528 190" marker-end="url(#so2a)"/><text class="m" x="494" y="182" text-anchor="middle">obnova</text>
<rect class="ok" x="10" y="270" width="740" height="56" rx="6"/><text class="t b" x="380" y="292" text-anchor="middle">Obnova = záznam v auditu + oznámení majiteli při dalším přihlášení</text><text class="m" x="380" y="312" text-anchor="middle">majitel se dozví, že se jeho domov odemykal firemním klíčem, i když nic neudělal</text>
</svg>
<figcaption>Další obal klíče domova je zabalený veřejným klíčem organizace, soukromá část leží mimo počítač.</figcaption>
</figure>

- **Soukromá část klíče organizace není nikdy v počítači ani v databázi
  účtů.** Možnosti: offline uložení (sejf), HSM, YubiKey, nebo **Shamirovo
  sdílení k z n** mezi více lidmi, takže obnova potřebuje několik osob.
  Počítač zná jen veřejný klíč; **krádež počítače nebo databáze účtů
  nedá nic**, ani útočníkovi, ani organizaci.
- **Obnova** je výjimečná událost: odemkne domov jednoho účtu, zapíše se
  do auditu (kdo, kdy, proč) a majitel dostane **oznámení při dalším
  přihlášení**. Obnova neumožňuje číst data skrytě.
- **Porovnání s existujícími řešeními.** FileVault na macOS má
  „institucionální obnovovací klíč“ s veřejným certifikátem organizace
  (soukromá část mimo počítač): stejná myšlenka jako tady. BitLocker
  s úschovou do adresáře ukládá obnovovací klíče na server organizace; je
  slabší, protože **kompromitace adresáře vyzradí všechny klíče**.
  Rozhodnutí: použít asymetrický obal.
- **Doporučení:** firemní data patří hlavně na **firemní úložiště**
  (server, sdílené svazky); escrow je nouzová cesta, ne hlavní způsob
  práce s daty.
- Escrow závisí na šifrování domova, které zatím čeká na šifrované
  trezory FS (fáze 4, [Šifrované trezory](navrh-fs-v3.md#šifrované-trezory));
  dokud domov není šifrovaný, escrow nemá co zálohovat.
- **Zálohu klíče domova** zapíná politika; v nastavení se ukáže, že je
  zapnutá a pro koho.

## Sledování koncových bodů (EDR)

Organizace chtějí vidět, co se na jejich počítačích děje (jako nástroje
typu CrowdStrike). Systém to nabídne jako **vrstvu systému**, ne jako
kód v jádře.

> **Poučení z července 2024:** chybná aktualizace obsahu, kterou načítal
> ovladač CrowdStrike v jádře Windows, shodila zhruba 8,5 milionu počítačů
> do smyčky restartů. Kód třetí strany v jádře znamená, že jeho chyba je
> chyba celého systému. Proto zde agent v jádře ani jako ovladač být
> **nesmí**. Vzorem je **Endpoint Security framework** od Applu: systém
> dodává události a agent je obyčejný proces mimo jádro.

<figure class="diagram">
<svg viewBox="0 0 760 360" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Sluzba udalosti a agent organizace">
<defs><marker id="so3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker><marker id="so3a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="190" height="200" rx="6"/><text class="t b" x="105" y="43" text-anchor="middle">jádro a služby</text><text class="m" x="105" y="68" text-anchor="middle">procesy: start, konec,</text><text class="m" x="105" y="84" text-anchor="middle">exec, výjimka</text><text class="m" x="105" y="108" text-anchor="middle">soubory: open, zápis,</text><text class="m" x="105" y="124" text-anchor="middle">smazání, spuštění</text><text class="m" x="105" y="148" text-anchor="middle">síť: spojení, DNS</text><text class="m" x="105" y="172" text-anchor="middle">přihlášení, ELEVATE,</text><text class="m" x="105" y="188" text-anchor="middle">USB, změny pravidel</text>
<rect class="acc" x="280" y="20" width="200" height="200" rx="6"/><text class="t b" x="380" y="43" text-anchor="middle">auditd (služba událostí)</text><text class="m" x="380" y="68" text-anchor="middle">strukturované události</text><text class="m" x="380" y="88" text-anchor="middle">odběr NOTIFY: jen kopie</text><text class="m" x="380" y="108" text-anchor="middle">odběr AUTH: povolit / zakázat</text><text class="m" x="380" y="124" text-anchor="middle">s termínem</text><text class="m" x="380" y="148" text-anchor="middle">cache verdiktů</text><text class="m" x="380" y="172" text-anchor="middle">výchozí politika při</text><text class="m" x="380" y="188" text-anchor="middle">nestihnutí / pádu agenta</text>
<rect class="warn" x="560" y="20" width="190" height="200" rx="6"/><text class="t b" x="655" y="43" text-anchor="middle">agent organizace</text><text class="m" x="655" y="68" text-anchor="middle">obyčejný proces</text><text class="m" x="655" y="84" text-anchor="middle">v userspace</text><text class="m" x="655" y="108" text-anchor="middle">capability k odběru</text><text class="m" x="655" y="124" text-anchor="middle">(jen zapsaný počítač)</text><text class="m" x="655" y="148" text-anchor="middle">žádný přístup k paměti</text><text class="m" x="655" y="164" text-anchor="middle">cizích procesů</text><text class="m" x="655" y="188" text-anchor="middle">posílá data organizaci</text>
<path class="ln" d="M200 120 L278 120" marker-end="url(#so3)"/>
<path class="ln-acc" d="M480 100 L558 100" marker-end="url(#so3a)"/><text class="m" x="519" y="92" text-anchor="middle">událost</text>
<path class="ln-acc" d="M558 150 L482 150" marker-end="url(#so3a)"/><text class="m" x="519" y="168" text-anchor="middle">verdikt</text>
<rect class="ok" x="10" y="260" width="740" height="84" rx="6"/><text class="t b" x="380" y="282" text-anchor="middle">Agent spadl nebo nestihl termín? Systém běží dál.</text><text class="m" x="380" y="302" text-anchor="middle">platí výchozí politika (fail-open: povolit a zapsat; fail-closed: zakázat jen vybrané kategorie)</text><text class="m" x="380" y="320" text-anchor="middle">supervisor (init) agenta restartuje; odběr se obnoví přes lístek</text>
</svg>
<figcaption>Systém dodává události, agent je obyčejný proces; jeho pád systém neshodí.</figcaption>
</figure>

- **Služba událostí** (pracovní jméno `auditd`) dostává **strukturované
  události** z jádra a ze služeb:
  - **procesy:** start, konec, `exec`, výjimka (jádro má už dnes
    [kanály výjimek](syscally.md#exception_channel), na kterých to lze
    postavit),
  - **soubory:** `open`, zápis, smazání, spuštění (hlásí `fsd`),
  - **síť:** spojení a DNS (hlásí `netd`, navazuje na [audit firewallu](firewall.md)),
  - **přihlášení, `ELEVATE`, USB, změny pravidel** (hlásí `accountd`,
    `devmgr`, `fwd`).
- **Dva druhy odběru:**
  - `NOTIFY`: agent dostane kopii události, nic nerozhoduje.
  - `AUTH`: agent událost **povolí nebo zakáže**, vždy **s termínem**.
    Nestihne-li termín nebo agent spadl, platí **výchozí politika**
    (fail-open nebo fail-closed podle politiky organizace) a **systém
    běží dál**. Verdikty se **cachují** (stejná operace stejného programu
    se neptá znovu).
- **Agent je obyčejný proces** userspace s capability k odběru: nikdy kód
  v jádře, nikdy ovladač, **žádný přístup k paměti cizích procesů**.
  Supervisor ho po pádu restartuje ([Obnova služeb](obnova-sluzeb.md)).
- **Capability k odběru vydá jen zapsaný počítač** (init po ověření
  záznamu v `/system/org`). Odběr je **silné oprávnění**: je zaznamenaný
  v auditu a viditelný v nastavení i na liště.
- Rozsah událostí a kategorie, které se smí sledovat, vymezuje politika;
  co není v politice, agent nedostane.
- **Nezapsaný počítač nemá žádnou režii:** bez odběratele místo emise
  události jen **atomicky přečte příznak „má odběr“** (jednotky ns)
  a pokračuje. Nevzniká událost, kopie ani žádná služba navíc.
  **Podmínka zavedení:** tvrzení ověří budoucí test a měření
  **[navrženo]**.
- **Zapsaný počítač:** `NOTIFY` jde **asynchronně do fronty** (levné);
  `AUTH` je **jedna výměna zpráv s agentem**, odhad **desítky µs** na
  sledovaný `exec` nebo `open` plus cache verdiktu. Odhad se ověří
  měřením (viz [Rizika](#rizika)).

## Záznam obrazovky a kláves

**Není ve výchozím rozsahu sledování.** Záznam obrazovky nebo stisků
kláves je **samostatná, výslovně ohlášená kategorie**: musí být
v politice, uživatel ji vidí v nastavení při zápisu a po celou dobu ji
ukazuje **viditelný indikátor** (lišta). Bez zapnutí kategorie se pro ni
capability nevydá. Vstup do systémových dialogů a polí s heslem se
nezaznamenává nikdy.

## Rizika

- ⚠️ **Právní.** Sledování zaměstnanců řeší GDPR (zákonný důvod,
  přiměřenost, informování) a v ČR zákoník práce, § 316: zaměstnavatel
  smí sledovat zaměstnance jen se **závažným důvodem** a po jejich
  **informování**. Systém pomáhá **průhledností** (uživatel vidí, co se
  sleduje, a dostane oznámení o obnově klíče), ale **právní odpovědnost
  zůstává na organizaci**. Stránka není právní poradenství.
- ⚠️ **Výkon.** `AUTH` zpomaluje sledované operace: každá nezkešovaná
  operace čeká na verdikt agenta. Návrh to omezuje **termínem** (po něm
  platí výchozí politika) a **cache verdiktů** (opakovaná operace se
  neptá). Odhad: zásah do cache je levný (bez komunikace s agentem),
  chybějící položka stojí jednu výměnu zpráv s agentem plus jeho práci.
  Konkrétní termín a dopad se **změří** při zavedení **[navrženo]**;
  výchozí rozsah `AUTH` je proto úzký (typicky spuštění programů), ne
  každý `open`.
- ⚠️ **Zneužití.** Odběr událostí a obnova klíče jsou silná oprávnění:
  kompromitovaný agent nebo správce organizace vidí hodně. Proti tomu
  stojí: capability jen pro zapsaný počítač, audit a viditelnost pro
  uživatele, politiky podepsané klíčem mimo počítač, Shamirovo sdílení
  u obnovy klíče a žádný přístup agenta k paměti cizích procesů.
- ⚠️ **Obchůzka u nešifrovaného domova.** Role správy může účtu nastavit
  nové heslo a přihlásit se jako on; u **nešifrovaného** domova tak získá
  jeho data. U šifrovaného domova to nejde (klíč domova nové heslo
  nezíská). Zapsáno jako známé omezení v [Účtech](ucty.md#rizika-a-omezení).

## Pořadí zavedení a závislosti

Implementace začíná **až po aktuálním pořadí** převzetí cizího software
([std, `cstd`, správa paměti, dynamické linkování, ...](architektura.md#převzetí-cizího-software-pořadí));
tahle stránka ho nemění. Co musí být před každou částí:

| Část | Potřebuje |
|---|---|
| zápis do správy (`/system/org`, dialog) | `accountd` (`ELEVATE`, role, audit), `fsd` (relace pro `/system/org`), `compd` (dialog, indikátor na liště) |
| firemní obal klíče domova | šifrované trezory FS (fáze 4), `accountd` (obaly), kryptografie HPKE (RustCrypto) |
| služba událostí `auditd` | `logd` (uložení a dohledání), `fsd` a `netd` jako zdroje událostí, jádro (kanály výjimek, události procesů), init (capability, supervisor) |
| agent a odběr `AUTH` | `auditd`, init (restart přes lístky), politiky ze záznamu o zápisu |
| záznam obrazovky a kláves | `compd` (indikátor), vstupní vrstva (`inputd`), politika |

## Otevřené otázky

1. Smí uživatel odepsat firemní počítač z evidence sám, nebo jen
   s organizací (politika)? Varianty: A) vždy smí (ztratí firemní
   funkce), B) jen s organizací, C) určí politika.
2. Kdo smí potvrdit zápis na sdíleném počítači s více účty (jen role
   správy, nebo i každý majitel účtu pro svůj domov)?
3. Jaký formát politik a jak je organizace doručí (pull při každém
   spojení, nebo push)?

## Stav zavedení

| Část | Stav |
|---|---|
| zápis do správy organizace, politiky, indikátor | **[navrženo]** |
| firemní obal klíče domova (escrow) | **[navrženo]** (po šifrovaných trezorech FS) |
| služba událostí `auditd`, agent, odběry `NOTIFY` a `AUTH` | **[navrženo]** |
| záznam obrazovky a kláves | **[navrženo]** |
