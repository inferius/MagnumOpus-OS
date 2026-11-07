# Reálný čas

Systém má dvoje hodiny:

| Hodiny | Volání | Na co | Vlastnosti |
|---|---|---|---|
| monotónní | `CLOCK_MONOTONIC`, `libos::time::now()` | měření, termíny, timeouty | ns od startu jádra, nikdy necouvne, nemůže selhat |
| reálné („na zdi“) | `CLOCK_REALTIME`, `libos::time::wall()` | časy souborů, zobrazení, logy | sekundy + ns od 1970 UTC, **může skočit** (nastavení hodin) |

Termín čekání se vždy počítá z monotónních hodin. Reálný čas se mezitím
může posunout třeba o hodinu (oprava podle sítě) a timeout by pak vypršel
okamžitě nebo nikdy.

## Odkud jádro reálný čas bere

<figure class="diagram">
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Zdroje realneho casu">
<defs><marker id="tm1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="200" height="70" rx="6"/><text class="t b" x="110" y="45" text-anchor="middle">loader (UEFI)</text><text class="m" x="110" y="64" text-anchor="middle">GetTime + TSC těsně před</text><text class="m" x="110" y="80" text-anchor="middle">ukončením boot services</text>
<path class="ln" d="M210 55 L278 55" marker-end="url(#tm1)"/>
<rect class="acc" x="280" y="20" width="220" height="70" rx="6"/><text class="t b" x="390" y="45" text-anchor="middle">BootInfo.firmware_time</text><text class="m" x="390" y="64" text-anchor="middle">sekundy od 1970, ns, pásmo,</text><text class="m" x="390" y="80" text-anchor="middle">TSC v okamžiku čtení</text>
<path class="ln" d="M500 55 L568 90" marker-end="url(#tm1)"/>
<rect class="box" x="10" y="140" width="200" height="70" rx="6"/><text class="t b" x="110" y="165" text-anchor="middle">CMOS RTC (porty 70h/71h)</text><text class="m" x="110" y="184" text-anchor="middle">záloha, když UEFI čas nedal;</text><text class="m" x="110" y="200" text-anchor="middle">století z ACPI FADT</text>
<path class="ln" d="M210 175 L568 125" marker-end="url(#tm1)"/>
<rect class="ok" x="570" y="70" width="180" height="80" rx="6"/><text class="t b" x="660" y="95" text-anchor="middle">jádro: počátek</text><text class="m" x="660" y="114" text-anchor="middle">reálný čas v okamžiku</text><text class="m" x="660" y="130" text-anchor="middle">0 monotónních hodin</text>
<text class="m" x="10" y="238">CLOCK_REALTIME = počátek + CLOCK_MONOTONIC. Posun mezi čtením v loaderu a startem hodin jádra se dopočítá z TSC.</text>
</svg>
<figcaption>Zdroj času při startu: UEFI, jinak CMOS RTC, jinak je čas neznámý.</figcaption>
</figure>

1. **UEFI `GetTime`.** Loader přečte čas těsně před `ExitBootServices`
   a zároveň hodnotu TSC. Jádro po kalibraci TSC dopočítá, kolik času
   uběhlo do startu jeho hodin, a uloží počátek.
2. **Hardwarové hodiny** (`arch::hardware_clock`, na x86 CMOS RTC; aarch64
   zatím žádné). Když firmware čas nedal (chyba, neplatné datum), jádro
   přečte hodiny MC146818 samo:
   - Čte dvakrát po sobě, dokud se čtení neshodují. Během aktualizace by
     se jinak smíchaly dvě sekundy.
   - Umí BCD i binární zápis a 12- i 24hodinový režim.
   - Století bere z registru uvedeného v ACPI FADT, jinak předpokládá 20xx.
   - Když FADT hlásí, že RTC chybí, hodiny se nečtou.
3. **Nic.** `CLOCK_REALTIME` vrací `NOT_SUPPORTED` a `fsd` použije čas od
   startu. Soubory pak vypadají jako z roku 1970.

Log při startu ukazuje výsledek:

```text
loader: cas firmwaru: 2026-11-07 14:03:12 (pasmo neuvedeno, beru jako UTC)
kernel: realny cas: 2026-11-07 14:03:13 UTC (zdroj UEFI, pasmo neuvedeno - bran jako UTC)
```

⚠️ **Časové pásmo:** hodiny PC pásmo neznají a UEFI ho téměř vždy hlásí
jako „neuvedeno“. Jádro pak čas bere jako **UTC** a nastaví příznak
`ZONE_ASSUMED_UTC`.

Linux má v RTC obvykle UTC, **Windows místní čas**. Na počítači
s Windows je pak reálný čas posunutý o pásmo (v ČR o 1 až 2 hodiny). Opraví
ho služba času, když má v nastavení `rtc = local`, viz
[RTC v místním čase](#nastavení-systému-a-rtc-v-místním-čase-dual-boot-s-windows).

Když firmware pásmo uvede, platí UTC = místní − pásmo. To je konvence
UEFI 2.x; jiné verze specifikace ji uvádějí s opačným znaménkem.

⚠️ **Přesnost:** firmware i RTC mají rozlišení 1 s, počátek je tedy přesný
na sekundu. Seřízení podle sítě (NTP) umí `init` přes službu času, viz
[sit.md](sit.md#ntp-a-hodiny). Nastavit hodiny už jde
(`CLOCK_SET`, viz níže).

## Nastavení hodin (`CLOCK_SET`)

Reálný čas za běhu nastavuje **služba času** `timed`. Je to jediný proces
kromě `init`, který na to má oprávnění.

<figure class="diagram">
<svg viewBox="0 0 760 240" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Opravneni k nastaveni hodin">
<defs><marker id="tm2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="85" width="170" height="70" rx="6"/><text class="t b" x="95" y="110" text-anchor="middle">jádro</text><text class="m" x="95" y="128" text-anchor="middle">kořenový Resource:</text><text class="m" x="95" y="144" text-anchor="middle">MANAGE + SET_CLOCK</text>
<path class="ln" d="M180 120 L248 120" marker-end="url(#tm2)"/>
<rect class="box" x="250" y="85" width="150" height="70" rx="6"/><text class="t b" x="325" y="110" text-anchor="middle">init</text><text class="m" x="325" y="128" text-anchor="middle">drží kořen,</text><text class="m" x="325" y="144" text-anchor="middle">rozdělí práva</text>
<path class="ln" d="M400 105 L498 55" marker-end="url(#tm2)"/>
<path class="ln" d="M400 135 L498 185" marker-end="url(#tm2)"/>
<rect class="ok" x="500" y="20" width="250" height="70" rx="6"/><text class="t b" x="625" y="45" text-anchor="middle">timed (služba času)</text><text class="m" x="625" y="64" text-anchor="middle">Resource jen SET_CLOCK</text><text class="m" x="625" y="80" text-anchor="middle">bez MANAGE = bez hardwaru</text>
<rect class="box" x="500" y="150" width="250" height="70" rx="6"/><text class="t b" x="625" y="175" text-anchor="middle">devmgr → ovladače</text><text class="m" x="625" y="194" text-anchor="middle">kořen bez SET_CLOCK; omezené</text><text class="m" x="625" y="210" text-anchor="middle">prostředky ho nemají nikdy</text>
</svg>
<figcaption>Právo SET_CLOCK má jen kořenový Resource; init ho předá jen službě času.</figcaption>
</figure>

**Oprávnění** je právo `SET_CLOCK` na handlu typu Resource:

- Má ho jen **kořenový** Resource, který jádro dá procesu `init`.
- `init` službě času předá duplikát s právy `BASIC | SET_CLOCK`. Bez
  `MANAGE` s ním služba nesmí na hardware (okna registrů, DMA, přerušení).
- Správce zařízení dostane kořen s právy `BASIC | MANAGE`, tedy bez
  `SET_CLOCK`. Práva jdou při duplikaci jen ubírat, takže si ho nevrátí.
- Omezené prostředky ovladačů (`RESOURCE_CREATE`) `SET_CLOCK` nemají nikdy.

Bez práva vrátí `CLOCK_SET` chybu `ACCESS_DENIED`. Po nastavení má čas zdroj
`source::SET` a příznak `ZONE_ASSUMED_UTC` zmizí.

```rust
use libos::time::{self, zone::Settings, WallTime};

// V timed: clock = Resource s pravem SET_CLOCK.
let t = WallTime { secs: 1_794_000_000, nanos: 0 };
let hw = time::set(&clock, t, &settings, true)?; // true = zapsat i do RTC
```

**Zápis do hardwarových hodin** (`set_flags::WRITE_HARDWARE`) uloží čas
i do CMOS RTC, aby platil po restartu:

- Jádro pásma nezná. Do RTC zapíše `UTC + rtc_offset`, kde `rtc_offset`
  pošle služba času. Pro RTC v UTC je 0, pro RTC v místním čase je to
  posun pásma v daném okamžiku (Praha v létě +7200 s). Obal
  `time::set` ho spočítá z nastavení sám.
- Formát zápisu (BCD nebo binárně, 12 nebo 24 h) jádro převezme z registru
  B hodin. Během zápisu drží bit SET, aby se hodiny mezitím neaktualizovaly.
- **Zlomek sekundy:** RTC má rozlišení 1 s. Jádro proto během zápisu drží
  dělič hodin (registr A) v resetu. Po jeho uvolnění přijde první sekunda
  za 0,5 s, takže jádro zapisuje přesně v polovině sekundy
  (`arch::HARDWARE_CLOCK_PHASE_NS`). Hodiny pak tikají na hranici sekund.
  Bez toho by byly po každém zápisu až o 1 s pozadu. Syscall s
  `WRITE_HARDWARE` proto může **spát až 1 s**.
- Zapisuje se čas systému **v okamžiku zápisu**, ne čas z požadavku. Zápis
  je pod zámkem, takže i souběžná nastavení nechají v RTC nejnovější čas
  systému.
- Platný vstup: čas v letech 1970–9999 (`SetRequest::MIN_SECS` až
  `MAX_SECS`), nanosekundy pod 10⁹ a `|rtc_offset|` nejvýš 18 h
  (`abi::time::MAX_ZONE_OFFSET`). Jinak `INVALID_ARGS`.
- Bez registru století v ACPI FADT umí hodiny jen roky 2000–2099.
- aarch64 zatím hardwarové hodiny nemá (`set_hardware_clock` vrací
  `false`). Nastaví se jen čas systému a syscall vrátí 0.

⚠️ **Rizika nastavení hodin:**

- Reálný čas **skočí** dopředu i dozadu. Kdo podle něj měří dobu, dostane
  nesmysl. Termíny a timeouty proto vždy počítejte z `CLOCK_MONOTONIC`,
  ty se nastavením nezmění.
- Čas souborů zapsaných před nastavením zůstane podle starého času.
- Když je RTC v místním čase a běží zároveň Windows, zapisujte do RTC jen
  místní čas (`rtc = local`). Zápis UTC by Windows posunul o pásmo.

## Plynulá korekce (`CLOCK_ADJUST`)

Skok hodin zpět rozbije pořadí: soubor uložený později má starší čas,
logy jdou mimo pořadí, certifikáty TLS se můžou na chvíli jevit jako
neplatné. Proto se za běhu hodiny jen **plynule dolaďují**.

<figure class="diagram">
<svg viewBox="0 0 760 220" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Plynula korekce casu">
<defs><marker id="tm7" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<path class="ln" d="M60 190 L740 190" marker-end="url(#tm7)"/><text class="m" x="680" y="210">monotonní čas</text>
<path class="ln" d="M60 190 L60 20" marker-end="url(#tm7)"/><text class="m" x="66" y="30">reálný čas</text>
<path class="ln" d="M60 170 L720 40"/><text class="m" x="600" y="80">skutečný čas</text>
<path class="ln-acc" d="M60 140 L300 92 L300 123 L720 40"/>
<path class="ln-acc" d="M300 92 L560 72"/>
<text class="m" x="310" y="145">skok (CLOCK_SET): čas jde zpět</text>
<text class="m" x="250" y="60">plynule (CLOCK_ADJUST): jen pomaleji</text>
</svg>
<figcaption>Hodiny o 30 ms napřed: skok vrátí čas zpět, plynulá korekce jen na chvíli zpomalí.</figcaption>
</figure>

Jádro počítá reálný čas od bodu, kde se naposledy změnil. Přičte k tomu
uplynulý monotónní čas a dvě opravy:

- **frekvence**: trvalá oprava rychlosti krystalu (hodiny počítače
  trvale předbíhají nebo se zpožďují, typicky do 100 ppm),
- **slew**: dočasné zrychlení nebo zpomalení (nejvýš 500 ppm), dokud se
  nedorovná zadaný posun. Pak samo skončí.

Rychlost reálného času je tak vždy 1 ± 1000 ppm, nikdy záporná ani nulová.
Syscall: [`CLOCK_ADJUST`](syscally.md#clock_adjust), obal
`libos::time::adjust(clock, posun, frekvence)`.

```rust
use libos::time;
// V timed: hodiny jdou o 20 ms napřed, krystal předbíhá o 35 ppm.
let st = time::adjust(&clock, Some(-20_000_000), Some(-35_000))?;
// st.remaining_ns = -20 ms, za 40 s je dorovnáno.
```

## Synchronizace podle sítě (`ntpd`)

<figure class="diagram">
<svg viewBox="0 0 760 170" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="ntpd a timed">
<defs><marker id="tm8" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="warn" x="10" y="50" width="150" height="70" rx="6"/><text class="t b" x="85" y="78" text-anchor="middle">servery NTP</text><text class="m" x="85" y="96" text-anchor="middle">UDP 123</text>
<path class="ln" d="M160 85 L228 85" marker-end="url(#tm8)"/>
<rect class="box" x="230" y="40" width="200" height="90" rx="6"/><text class="t b" x="330" y="65" text-anchor="middle">ntpd</text><text class="m" x="330" y="84" text-anchor="middle">relace: jen tyto servery</text><text class="m" x="330" y="100" text-anchor="middle">filtr, median, frekvence</text><text class="m" x="330" y="116" text-anchor="middle">bez práva k hodinám</text>
<path class="ln-acc" d="M430 85 L498 85" marker-end="url(#tm8)"/><text class="m" x="440" y="76">sync</text>
<rect class="ok" x="500" y="40" width="250" height="90" rx="6"/><text class="t b" x="625" y="65" text-anchor="middle">timed</text><text class="m" x="625" y="84" text-anchor="middle">ověří a omezí návrh</text><text class="m" x="625" y="100" text-anchor="middle">CLOCK_ADJUST (SET_CLOCK)</text><text class="m" x="625" y="116" text-anchor="middle">skok jen při první synchronizaci</text>
</svg>
<figcaption>Proces, který zpracovává data ze sítě, hodiny přímo měnit nesmí.</figcaption>
</figure>

**`ntpd`** se ptá serverů NTP a jen navrhuje korekce. Init ho spustí po
službě sítě:

- **Relace sítě** podle manifestu `ntpd.manifest.toml` (práva `tcp`,
  `udp`, `dns`) a navíc jen cíle serverů z `ntpd.toml` (viz
  [NTS](#nts-ověřený-čas)). Nikam jinam nesmí.
- **Kanál k `timed`** dostane jen pro zprávy `sync` (návrh korekce).
- Každé kolo: z každého serveru dávka měření (`samples`), z ní vzorek
  s **nejmenším zpožděním** (nejméně ho zkreslí fronty a nesymetrická
  cesta), ze serverů **medián** (jeden špatný server výsledek nestáhne).
- **Frekvence:** z posunu mezi koly (minus část předchozí korekce, kterou
  jádro ještě nestihlo dorovnat) odhadne drift krystalu. Navrhne polovinu
  chyby, aby šum měření nerozkmital smyčku (`netproto::ntp::Discipline`).
- **Interval** začíná na `min_poll` a při srovnaných hodinách (posun pod
  2 ms) se zdvojnásobuje až na `max_poll`. Kolo bez měření (při startu
  ještě nejede síť nebo DNS, server neodpověděl) se opakuje za 2, 4, 8, …
  s, nejvýš za `min_poll`: hodiny se nastaví pár sekund po startu sítě,
  ne až za `min_poll` (64 s).

```toml
# ntpd.toml (initfs)
servers = ["nts://time.cloudflare.com", "nts://nts.netnod.se", "nts://ptbtime1.ptb.de", "pool.ntp.org"]
unauthenticated = "profile" # neověřené servery: never / fallback / profile
min_poll = 64
max_poll = 1024
samples = 4
```

**`timed`** návrh ověří a provede, protože jen on má právo k hodinám:

| Situace | Co `timed` udělá |
|---|---|
| první synchronizace, posun nad 128 ms | skok (`CLOCK_SET`): hodiny z RTC můžou být o minuty vedle a dorovnání by trvalo dny |
| jinak | plynulé dorovnání (`CLOCK_ADJUST`), nikdy skok zpět |
| po první synchronizaci posun nad 60 s | odmítne (`OUT_OF_RANGE`), hodiny se tolik samy nerozejdou, spíš jde o podvržení |
| frekvence | nejvýš ±500 ppm a jedna změna nejvýš o 100 ppm |

„První synchronizace“ si `timed` nepamatuje jen v paměti: jádro po ní hlásí
zdroj času 4 (`source::NETWORK`, nastaví ho `CLOCK_SET` s příznakem
`NETWORK` i `CLOCK_ADJUST` s posunem). Nový `timed` po restartu
supervisorem podle něj pozná, že skok už proběhl, a další skok nepovolí.

> **Změna proti první verzi:** stav byl jen v paměti `timed`, takže po
> jeho restartu šlo hodiny skokem posunout znovu. **Proč:** pravidlo
> „jediný skok“ musí platit po celou dobu běhu systému.

Stav: zpráva `status` na kanálu initu k `timed` (`synced=1 updates=…
last_offset_ns=… remaining_ns=… freq_ppb=… slew_ppb=…`) a k `ntpd`
(`rounds=… offset_ns=… delay_ns=… freq_ppb=… poll_s=… error=… timed=…
auth=nts|plain|- policy=… nts_error=…`).

### NTS (ověřený čas)

<figure class="diagram">
<svg viewBox="0 0 760 230" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="NTS">
<defs><marker id="tm9" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="20" width="160" height="190" rx="6"/><text class="t b" x="90" y="45" text-anchor="middle">ntpd</text>
<text class="m" x="20" y="70">klíče C2S, S2C</text><text class="m" x="20" y="88">z TLS exportu</text><text class="m" x="20" y="106">8 cookies</text>
<rect class="ok" x="590" y="20" width="160" height="80" rx="6"/><text class="t b" x="670" y="45" text-anchor="middle">NTS-KE</text><text class="m" x="670" y="64" text-anchor="middle">TLS 1.3, TCP 4460</text><text class="m" x="670" y="82" text-anchor="middle">ALPN ntske/1</text>
<rect class="ok" x="590" y="130" width="160" height="80" rx="6"/><text class="t b" x="670" y="155" text-anchor="middle">server NTP</text><text class="m" x="670" y="174" text-anchor="middle">UDP 123</text><text class="m" x="670" y="192" text-anchor="middle">AES-SIV-CMAC-256</text>
<path class="ln" d="M170 50 L588 50" marker-end="url(#tm9)"/><text class="m" x="190" y="44">ClientHello, NTPv4 + AES-SIV, END</text>
<path class="ln" d="M588 75 L172 75" marker-end="url(#tm9)"/><text class="m" x="190" y="69">certifikát; 8 cookies, port NTP, END</text>
<path class="ln-acc" d="M170 150 L588 150" marker-end="url(#tm9)"/><text class="m" x="190" y="144">NTP + UID + cookie + zástupci + autentizátor (C2S)</text>
<path class="ln-acc" d="M588 180 L172 180" marker-end="url(#tm9)"/><text class="m" x="190" y="174">NTP + UID + autentizátor (S2C) s novými cookies</text>
</svg>
<figcaption>Klíče se dohodnou přes TLS (ověřený server), každý dotaz NTP je podepsaný a jednorázový.</figcaption>
</figure>

**NTS** (Network Time Security, RFC 8915) brání podvržení času:

1. **NTS-KE:** TLS 1.3 na port 4460 s ALPN `ntske/1`. Certifikát serveru
   se ověří jako u HTTPS (kořeny z úložiště, odvolání přes `revocd`).
   Klient pošle NTPv4 a AEAD 15 (AES-SIV-CMAC-256), server vrátí sadu
   8 **cookies** a případně jiný server nebo port pro NTP. Klíče C2S a S2C
   (32 B) se spočítají z TLS exportu (`EXPORTER-network-time-security`).
2. **NTP s rozšířeními:** dotaz nese náhodný identifikátor (32 B), jednu
   cookie, zástupce pro doplnění cookies a **autentizátor** AES-SIV přes
   hlavičku a předchozí rozšíření (klíč C2S). Odpověď se ověří klíčem S2C
   a v zašifrované části nese nové cookies.
3. Každá cookie se použije **jednou** (dotazy nejdou spojit). Když dojdou
   nebo server pošle NTSN („kiss of death“), proběhne nové NTS-KE.

Podvržená odpověď (jiný identifikátor, změněný bajt, chybějící
autentizátor) se zahodí a ntpd čeká dál na pravou; útočník tak může
synchronizaci jen zablokovat, ne posunout.

| `servers` | Význam |
|---|---|
| `nts://jméno[:port]` | NTS, NTS-KE na portu (výchozí 4460) |
| `jméno`, `adresa[:port]` | neověřené NTP (výchozí port 123) |

| `unauthenticated` | Kdy se použijí neověřené servery |
|---|---|
| `never` | nikdy; bez odpovědi serveru NTS se hodiny nekorigují |
| `fallback` | jen když v kole neodpoví žádný server NTS |
| `profile` (výchozí) | podle profilu sítě (`unauthenticated` ve [firewallu](firewall.md#profily-sítí); veřejná síť = `never`), čte se každé kolo z `Net::info()` |

Zpráva `policy never|fallback|profile` pravidlo změní za běhu. Relace sítě `ntpd` pro `nts://` server: TCP na NTS-KE, UDP na
server NTS-KE (libovolný port) a UDP 123 kamkoli (server NTP určuje
NTS-KE).

> ⚠️ **Hodiny a certifikát se potřebují navzájem:** platnost certifikátu
> serveru NTS-KE se kontroluje podle hodin. Když jsou hodiny z RTC mimo
> platnost certifikátu (vybitá baterie, rok 1970), NTS-KE selže a čas se
> nesynchronizuje, dokud ho nenastaví uživatel nebo neověřený server
> (`fallback`).

> **Změna proti první verzi:** jen neověřené NTP. **Proč:** kdo ovládá
> síť, mohl čas posunout a tím třeba nechat projít propadlý nebo
> odvolaný certifikát.

⚠️ **Neověřené NTP** (servery bez `nts://`, pravidlo `fallback`) může
podvrhnout kdo ovládá síť. Omezení v `timed` útok zpomalí (nejvýš 500 ppm,
tedy 43 s za den), ale nezastaví; proto je výchozí `never`.

## Časová pásma

Jádro drží **jen UTC**. Pásmo je nastavení systému, které čte služba času
a programy, ne jádro. Převod dělá `libos::time::zone` (crate `libs/tz`):

- Nemá tzdata. Pásma popisuje kompaktní tabulka se **současnými** pravidly.
- Každé pásmo má standardní posun od UTC a nejvýš jedno pravidlo letního
  času. Začátek a konec jsou zadané jako „n-tá / poslední neděle v měsíci
  v hodinu H“, vztažená k UTC, standardnímu nebo místnímu času.
- Pevný posun jde zapsat i bez tabulky: `UTC+2`, `UTC-03:30`, `GMT+05:45`.

| Pravidlo | Začátek letního času | Konec | Pásma (výběr) |
|---|---|---|---|
| EU | poslední neděle v březnu 01:00 UTC | poslední neděle v říjnu 01:00 UTC | Europe/Prague, Bratislava, Berlin, London, Lisbon, Helsinki … |
| USA | 2. neděle v březnu 02:00 místního | 1. neděle v listopadu 02:00 místního | America/New_York, Chicago, Denver, Los_Angeles |
| Austrálie | 1. neděle v říjnu 02:00 standardního | 1. neděle v dubnu 02:00 standardního | Australia/Sydney |
| bez letního času | – | – | UTC, Asia/Tokyo, Asia/Kolkata (+5:30), Europe/Moscow … |

Úplný seznam je v `libs/tz/src/table.rs`. Nové pásmo = jeden řádek.

<figure class="diagram">
<svg viewBox="0 0 760 210" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Prechody letniho casu v Praze 2026">
<rect class="box" x="10" y="60" width="180" height="50" rx="6"/><text class="t b" x="100" y="82" text-anchor="middle">SEČ (CET)</text><text class="m" x="100" y="100" text-anchor="middle">UTC+1</text>
<rect class="ok" x="250" y="60" width="260" height="50" rx="6"/><text class="t b" x="380" y="82" text-anchor="middle">SELČ (CEST)</text><text class="m" x="380" y="100" text-anchor="middle">UTC+2</text>
<rect class="box" x="570" y="60" width="180" height="50" rx="6"/><text class="t b" x="660" y="82" text-anchor="middle">SEČ (CET)</text><text class="m" x="660" y="100" text-anchor="middle">UTC+1</text>
<path class="ln" d="M220 30 L220 140"/><path class="ln" d="M540 30 L540 140"/>
<text class="t" x="220" y="22" text-anchor="middle">29. 3. 01:00 UTC</text><text class="t" x="540" y="22" text-anchor="middle">25. 10. 01:00 UTC</text>
<text class="m" x="220" y="160" text-anchor="middle">02:00 → 03:00 místního</text><text class="m" x="220" y="176" text-anchor="middle">02:30 neexistuje (Gap)</text>
<text class="m" x="540" y="160" text-anchor="middle">03:00 → 02:00 místního</text><text class="m" x="540" y="176" text-anchor="middle">02:30 nastane 2× (Ambiguous)</text>
</svg>
<figcaption>Rok 2026 v Praze: na jaře se hodina přeskočí, na podzim se opakuje.</figcaption>
</figure>

```rust
use libos::time::{self, zone::{Zone, LocalResult}};

let praha = Zone::by_name("Europe/Prague").unwrap();
if let Some(l) = time::local(&praha) {
    let c = l.civil;
    println!("{:02}:{:02} {}", c.hour, c.minute, l.offset.abbrev); // 14:03 CET
}

// Místní čas -> UTC: na přechodech nemusí být jednoznačný.
match praha.local_to_utc(&civil).unwrap() {
    LocalResult::Single(utc) => { /* běžný případ */ }
    LocalResult::Ambiguous { earlier, later } => { /* podzim: hodina 2× */ }
    LocalResult::Gap(utc) => { /* jaro: hodina neexistovala */ }
}
```

`LocalResult::resolve()` vybere jednu hodnotu: při dvojznačnosti dřívější
okamžik (ještě letní čas), při mezeře převod posunem platným před ní
(02:30 → 03:30 SELČ). Chová se stejně jako `mktime` s `tm_isdst = -1`.

⚠️ **Omezení tabulky:**

- Historie se nepočítá, pravidla platí pro všechny roky. V Česku platí
  pravidla EU od roku 1996. Pro starší data (a před rokem 1979, kdy se
  letní čas nepoužíval) vyjde místní čas o hodinu jinak, než skutečně byl.
- Až stát pravidla změní, je třeba upravit tabulku.
- Pásma s víc než dvěma přechody za rok nebo se záporným letním časem
  (Irsko v tzdata) tabulka neumí. Posun od UTC je ale správný.

### Nastavení systému a RTC v místním čase (dual-boot s Windows)

Nastavení času je text `klíč = hodnota`:

```text
zone = Europe/Prague   # jméno z tabulky, nebo UTC+2, UTC-03:30
rtc = local            # utc (výchozí, Linux) | local (Windows)
```

Zatím je pevně v `init` (`system::TIME_SETTINGS`, pro QEMU `rtc = utc`).
Až bude konfigurace systému na disku, načte se odtud. `init` ho při startu
pošle službě času jako první zprávu.

Windows drží v RTC **místní čas**. Jádro ho bez údaje o pásmu bere jako UTC,
takže je reálný čas posunutý o pásmo. Při `rtc = local` to služba času
hned po startu opraví, ještě před spuštěním ostatních služeb:

<figure class="diagram">
<svg viewBox="0 0 760 200" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Oprava RTC v mistnim case">
<defs><marker id="tm3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="40" width="170" height="80" rx="6"/><text class="t b" x="95" y="65" text-anchor="middle">RTC (Windows)</text><text class="m" x="95" y="84" text-anchor="middle">14:00 místního</text><text class="m" x="95" y="100" text-anchor="middle">(léto, UTC+2)</text>
<path class="ln" d="M180 80 L248 80" marker-end="url(#tm3)"/>
<rect class="acc" x="250" y="40" width="210" height="80" rx="6"/><text class="t b" x="355" y="65" text-anchor="middle">jádro při startu</text><text class="m" x="355" y="84" text-anchor="middle">14:00 „UTC“, zdroj RTC/UEFI,</text><text class="m" x="355" y="100" text-anchor="middle">ZONE_ASSUMED_UTC</text>
<path class="ln" d="M460 80 L528 80" marker-end="url(#tm3)"/>
<rect class="ok" x="530" y="40" width="220" height="80" rx="6"/><text class="t b" x="640" y="65" text-anchor="middle">timed: rtc = local</text><text class="m" x="640" y="84" text-anchor="middle">14:00 − 2 h = 12:00 UTC</text><text class="m" x="640" y="100" text-anchor="middle">CLOCK_SET (RTC beze změny)</text>
<text class="m" x="10" y="160">Oprava jen jednou: zdroj musí být UEFI nebo RTC a mít ZONE_ASSUMED_UTC. Po CLOCK_SET je zdroj SET,</text>
<text class="m" x="10" y="178">takže restart služby čas neposune podruhé. Když firmware pásmo uvedl, jádro už převedlo samo.</text>
</svg>
<figcaption>RTC v místním čase: jádro čte „UTC“, služba času odečte posun pásma.</figcaption>
</figure>

Funkce: `zone::rtc_local_correction(&settings, &reading)` (čistý výpočet,
testovaný na hostiteli) a `time::apply_rtc_local(&clock, &settings)`
(přečte, opraví, nastaví).

⚠️ **Opakovaná hodina:** při konci letního času (v Praze poslední neděle
v říjnu 02:00–03:00) nejde z místního času v RTC poznat, která z obou hodin
to je. Bere se dřívější (letní čas). Pokud systém startuje právě v té
hodině, je čas o hodinu napřed, dokud ho neopraví síť.

⚠️ **Přechod během vypnutí:** Windows při prvním startu po změně letního
času posune RTC samo. Když po přechodu nastartuje dřív náš systém, RTC ještě
drží starý posun. Výsledek je pak o hodinu vedle, dokud čas neopraví síť
nebo Windows.

## Formát

```rust
pub struct WallTime {
    pub secs: i64,  // sekundy od 1970-01-01 00:00:00 UTC, záporné = dřív
    pub nanos: u32, // 0..1_000_000_000
}
```

- **Rozsah:** ±292 miliard let, tedy i data před rokem 1970 a daleko za
  rokem 2262 (hranice u64 nanosekund).
- **Přestupné sekundy:** nepočítají se (jako v POSIX), každý den má
  86 400 s.
- **Převod na kalendář:** proleptický gregoriánský kalendář přes `to_civil`
  a `from_civil` v `abi::time` (algoritmus „days from civil“). Testy
  pokrývají vybrané roky od −4713 do 10⁹ a každý sedmý den v rozsahu
  ±2 000 let.

```rust
use libos::time;

if let Some(t) = time::wall() {
    let c = t.to_civil();
    println!("{:04}-{:02}-{:02} {:02}:{:02} UTC", c.year, c.month, c.day, c.hour, c.minute);
}
let r = time::wall_reading()?;            // i se zdrojem a příznaky
if r.flags & time::flags::ZONE_ASSUMED_UTC != 0 { /* pásmo jen odhadnuté */ }
```

## Časy v souborovém systému

`fsd` nastavuje čas změn ve FS z reálných hodin. FS ukládá časy ve stejném
tvaru (sekundy i64 + nanosekundy u32), takže uloží i data před rokem 1970
a bez hranice 2262. Svazky ze starší verze FS mají časy u64 ns od 1970
a čas před 1970 se v nich ořízne. Viz [Časy objektů](souborovy-system.md#časy-objektů).
