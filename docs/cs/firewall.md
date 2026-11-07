# Firewall

Firewall je zabudovaný do služby sítě. `netd` je jediný síťový stack
v systému, takže ho nejde obejít: program nemá jinou cestu do sítě než
přes kanál relace a každý přijatý rámec projde filtrem dřív, než ho
dostane stack.

Má šest vrstev:

| Vrstva | Kde | Co dělá |
|---|---|---|
| **1. Manifest aplikace** | `<program>.manifest.toml`, `libs/manifest`, init | identita aplikace, rozpočet jobu, práva a povolené cíle v síti |
| **2. Dotaz uživateli** | `netd` → `fwd` → agent (UI) | cíl mimo pravidla manifestu: povolit / zakázat jednou nebo vždy |
| **3. Paketový filtr** | `libs/netfilter` v `netd` | příchozí spojení podle profilu, blokované sítě, omezení rychlosti |
| **4. Profily sítí** | `fwd` | domácí / veřejná / pracovní síť, poznaná podle MAC brány |
| **5. Audit** | `netd` (`AUDIT`, `STATS`) | kdo, kam, kdy; zamítnutí; provoz podle aplikací |
| **6. Filtr DNS** | resolver v `netd` | blokované domény (`ACCESS_DENIED` místo překladu) |

Konfiguraci spravuje služba **`fwd`** (`user/fwd`). Ukládá ji do souboru
TOML na souborovém systému, takže se verzuje se snapshotem FS.

<figure class="diagram">
<svg viewBox="0 0 760 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Firewall: netd, fwd, agent">
<defs><marker id="fw1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="170" height="70" rx="6"/><text class="t b" x="95" y="45" text-anchor="middle">aplikace</text><text class="m" x="95" y="63" text-anchor="middle">relace podle manifestu</text><text class="m" x="95" y="79" text-anchor="middle">(ask = true)</text>
<path class="ln" d="M180 55 L278 95" marker-end="url(#fw1)"/>
<text class="m" x="190" y="60">connect</text>
<rect class="ok" x="280" y="60" width="210" height="150" rx="6"/><text class="t b" x="385" y="85" text-anchor="middle">netd</text><text class="m" x="385" y="105" text-anchor="middle">vrstvy relace + rozhodnutí</text><text class="m" x="385" y="122" text-anchor="middle">blokované sítě, filtr DNS</text><text class="m" x="385" y="139" text-anchor="middle">příchozí filtr (profil)</text><text class="m" x="385" y="156" text-anchor="middle">audit, provoz aplikací</text><text class="m" x="385" y="190" text-anchor="middle">ASK →   ← ANSWER, CONFIG</text>
<path class="ln-acc" d="M490 120 L568 120" marker-end="url(#fw1)"/>
<path class="ln-acc" d="M568 150 L492 150" marker-end="url(#fw1)"/>
<rect class="acc" x="570" y="60" width="180" height="120" rx="6"/><text class="t b" x="660" y="85" text-anchor="middle">fwd</text><text class="m" x="660" y="105" text-anchor="middle">konfigurace, profily</text><text class="m" x="660" y="122" text-anchor="middle">trvalá rozhodnutí</text><text class="m" x="660" y="139" text-anchor="middle">/config/firewall.toml</text><text class="m" x="660" y="156" text-anchor="middle">(relace FS)</text>
<path class="ln" d="M660 180 L660 228" marker-end="url(#fw1)"/>
<rect class="warn" x="570" y="230" width="180" height="55" rx="6"/><text class="t b" x="660" y="253" text-anchor="middle">agent uživatele</text><text class="m" x="660" y="271" text-anchor="middle">UI: „Povolit?“, profil sítě</text>
<path class="ln" d="M385 210 L385 248" marker-end="url(#fw1)"/>
<rect class="box" x="280" y="250" width="210" height="40" rx="6"/><text class="t b" x="385" y="275" text-anchor="middle">karta (rámce)</text>
</svg>
<figcaption>netd rozhoduje o každém požadavku a paketu, fwd drží konfiguraci a ptá se uživatele.</figcaption>
</figure>

## Manifest aplikace

Init spouští programy podle manifestu `<program>.manifest.toml`, který leží
v initfs vedle programu. Později se přesune do balíčku beze změny formátu
(viz [Aplikace](architektura.md#aplikace-instalace-aktualizace-odinstalace-návrh)).

```toml
[app]
id = "org.magnumopus.netprobe"   # povinné: a-z A-Z 0-9 . - _
name = "Síťová sonda"            # pro dotazy uživateli

[limits]                         # rozpočet jobu (bez sekce = bez limitu)
memory = "16M"                   # uživatelská paměť
kernel_memory = "4M"             # paměť jádra

[network]                        # bez sekce = žádná síť
rights = ["info", "tcp", "udp", "dns", "ping"]
ask = true                       # cíl mimo pravidla: zeptat se uživatele
sockets = 32                     # nejvíc socketů najednou
buffers = "4M"                   # nejvíc paměti bufferů socketů

[[network.connect]]
proto = ["udp"]                  # tcp, udp, icmp (výchozí všechny)
to = "10.0.2.3"                  # "*", síť "10.0.0.0/8", adresa, jméno, "*.jméno"
ports = "53"                     # "80-90", "*" (výchozí všechny)

[[network.listen]]
ports = "8080"
```

- **Job:** program běží ve vlastním jobu s limity z `[limits]` (viz
  [Joby](syscally.md#joby-rozpočty-prostředků)). Program bez manifestu
  běží v podřízeném jobu bez limitu a síť nedostane.
- **Relace sítě:** init z kořenové relace vytvoří užší (`RESTRICT`)
  s právy, pravidly, identitou aplikace, příznakem `ask` a rozpočtem
  socketů. Program dostane jen její kanál.
- **Práva** mají jména `info`, `tcp`, `listen`, `udp`, `dns`, `ping`,
  `restrict`, `configure`, `audit` (viz [Oprávnění](sit.md#oprávnění)).

## Dotaz uživateli

Vrstva relace s příznakem `ask` není pevná hranice: cíl mimo její pravidla
rozhodne uživatel. Vrstva bez `ask` je pevná. Rodič tak může dítě uzavřít
a uživatel ji nepřebije.

<figure class="diagram">
<svg viewBox="0 0 760 210" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Prubeh dotazu">
<defs><marker id="fw2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="130" height="170" rx="6"/><text class="t b" x="75" y="45" text-anchor="middle">aplikace</text>
<rect class="ok" x="215" y="20" width="130" height="170" rx="6"/><text class="t b" x="280" y="45" text-anchor="middle">netd</text>
<rect class="acc" x="420" y="20" width="130" height="170" rx="6"/><text class="t b" x="485" y="45" text-anchor="middle">fwd</text>
<rect class="warn" x="625" y="20" width="125" height="170" rx="6"/><text class="t b" x="687" y="45" text-anchor="middle">agent</text>
<path class="ln" d="M140 70 L213 70" marker-end="url(#fw2)"/><text class="m" x="145" y="64">TCP_CONNECT</text>
<path class="ln" d="M345 90 L418 90" marker-end="url(#fw2)"/><text class="m" x="352" y="84">ASK</text>
<path class="ln" d="M550 110 L623 110" marker-end="url(#fw2)"/><text class="m" x="556" y="104">QUESTION</text>
<path class="ln" d="M623 135 L552 135" marker-end="url(#fw2)"/><text class="m" x="560" y="129">DECIDE</text>
<path class="ln" d="M418 155 L347 155" marker-end="url(#fw2)"/><text class="m" x="352" y="149">CONFIG, ANSWER</text>
<path class="ln" d="M213 175 L142 175" marker-end="url(#fw2)"/><text class="m" x="150" y="169">spojení / ACCESS_DENIED</text>
</svg>
<figcaption>Požadavek čeká na kanálu aplikace; ostatní sockety a aplikace běží dál.</figcaption>
</figure>

| Volba (`fw::choice`) | Výsledek |
|---|---|
| `DENY_ONCE` | tento požadavek `ACCESS_DENIED`, příště se zeptá znovu |
| `ALLOW_ONCE` | jen tento požadavek (u UDP pro tento socket a cíl do jeho zavření) |
| `ALLOW_ALWAYS` | fwd uloží pravidlo `allow` pro aplikaci, další požadavky bez dotazu |
| `DENY_ALWAYS` | fwd uloží pravidlo `deny`, další požadavky rovnou `ACCESS_DENIED` |

- **Trvalé pravidlo** má protokol a port dotazu a cíl. Cílem je jméno
  hostitele, když ho relace pro adresu dostala od systémového resolveru,
  jinak přesná adresa. Zákaz má přednost před povolením.
- **Bez agenta** nebo po `ask_timeout` (výchozí 60 s) platí
  `ask_default` (výchozí `deny`). Bez služby `fwd` netd dotaz rovnou
  odmítne.
- **Příchozí provoz se nenabízí:** datagram od odesílatele mimo pravidla
  se zahodí bez dotazu.
- Dotaz se týká TCP spojení, datagramu UDP, pingu a dotazu DNS na vlastní
  server. Do auditu jde `ASK_ALLOWED` nebo `ASK_DENIED`.

## Konfigurace

Výchozí konfiguraci (`user/fwd/firewall.toml`) předá init službě `fwd`
z initfs. Při prvním startu ji `fwd` uloží na FS do
`/system/config/firewall/firewall.toml` a dál čte a zapisuje jen tam.
`fwd` dostane relaci FS s jedinou připojenou cestou `/config`, takže
jinam na disku nevidí.

```toml
[firewall]
ask_default = "deny"        # bez agenta / po vypršení dotazu: deny | allow
ask_timeout = 60            # s
unknown_network = "public"  # profil neznámé sítě
audit_entries = 4096

[profiles.public]
inbound = "deny"            # deny | listeners
ping = false
stealth = true

[profiles.home]
inbound = "listeners"
ping = true
stealth = false
[[profiles.home.allow_in]]  # výjimky i v režimu deny
proto = ["tcp"]
from = "10.0.0.0/8"
ports = "22"

[rate]
syn_per_sec = 100
syn_burst = 200
icmp_per_sec = 10
icmp_burst = 20

[block]
networks = ["192.0.2.0/24"]
domains = ["ads.example", "*.tracker.example"]

[[apps]]                    # rozhodnutí uživatele (zapisuje fwd)
id = "org.magnumopus.netprobe"
[[apps.rule]]
action = "allow"
proto = ["tcp"]
to = "api.example.com"
ports = "443"

[[networks]]                # známé sítě (zapisuje fwd)
id = "52:54:00:12:35:02"
gateway = "10.0.2.2"
profile = "home"
```

- **Ukládání je podmíněné revizí souboru** (`save_if`). Když soubor
  mezitím změnil někdo jiný (uživatel v editoru), `fwd` ho znovu načte,
  změnu přidá k nové verzi a uloží znovu. Cizí úpravy se tak neztratí.
- Neplatný soubor `fwd` nepřevezme: při startu použije výchozí konfiguraci
  a nová konfigurace od agenta (`CONFIG_SET`) skončí `INVALID_ARGS`.

## Profily sítí

Síť pozná `netd` podle **MAC brány** z odpovědi ARP. Když se pak změní,
ohlásí ji `fwd` zprávou `NETWORK`. `fwd` najde profil uložené sítě. Pro
neznámou síť platí `unknown_network` (výchozí `public`) a `fwd` se agenta
zeptá (`NETWORK_QUESTION`). Odpověď `CLASSIFY` uloží do `[[networks]]`.
Dokud `netd` konfiguraci nedostane, platí veřejný profil.

| Profil (výchozí) | Příchozí spojení | Ping | Stealth | Neověřené DNS a NTP |
|---|---|---|---|---|
| `public` | žádná | ne | ano | `never` |
| `home` | na porty, kde někdo naslouchá | ano | ne | `fallback` |
| `work` | žádná (jen `allow_in`) | ano | ano | `fallback` |

`unauthenticated` (v `[profiles.*]`) říká, co dělat, když ověřená cesta
nefunguje (šifrované DNS v `dnsd`, NTS v `ntpd` – síť blokuje porty):
`never` = nic, `fallback` = neověřený resolver z DHCP a neověřené NTP. Ve
veřejné síti by záloha dala útočníkovi v téže Wi-Fi možnost podvrhnout
odpovědi. `netd` pravidlo dostává s profilem a hlásí ho v `Net::info()`
(`unauthenticated_fallback`), odkud ho čte `ntpd`.

⚠️ **MAC brány jde podvrhnout.** Kdo je ve stejné síti (veřejná Wi-Fi),
může předstírat MAC domácího routeru. Systém by pak síť považoval za
domácí a otevřel porty, na kterých se naslouchá. MAC brány proto bere jen
jako nápovědu. Proti tomu pomůže:

- nechat výchozí `public`,
- v domácím profilu povolovat jen to, co je nutné,
- později ověřit síť silnější identitou: WPA2/3 Enterprise, certifikát
  serveru, potvrzení uživatelem při každé změně.

## Paketový filtr

Logika je v `libs/netfilter` (testy na hostiteli) a `netd` ji volá na
každý přijatý rámec, než ho předá smoltcp.

- **Nové příchozí spojení TCP** (SYN bez ACK) a **datagram UDP** mimo
  sledované toky: profil `deny` je propustí jen podle `allow_in`, profil
  `listeners` také na porty, kde někdo naslouchá (TCP) nebo přijímá (UDP).
- **Sledované toky UDP:** odpověď na datagram, který jsme poslali (místní
  port + vzdálená adresa a port, platnost 120 s), projde bez pravidla.
  Tak fungují DNS, NTP a další klienti i ve veřejné síti.
  Sledovaných toků je nejvýš 65 536, nejstarší vypadnou.
- **Ping** (ICMP echo na nás) podle profilu. Ostatní ICMP (odpovědi,
  chyby) prochází vždy.
- **Blokované sítě:** nic dovnitř ani ven, ani pro kořenovou relaci.
- **Omezení rychlosti** SYN a ICMP echo na zdroj (token bucket, nejvíc
  4096 sledovaných zdrojů) a celkově (desetinásobek).
- **Stealth:** filtrovaný paket se tiše zahodí. Bez stealth dostane stack
  SYN na port, kde nikdo nenaslouchá, a odpoví RST jako zavřený port.
  Na port s posluchačem se filtrovaný paket nepustí nikdy.
- DHCP (odpověď serveru na port 68) prochází vždy.

⚠️ **Jen IPv4.** IPv6 stack zatím neumí a rámce IPv6 zahodí. Až IPv6
přibude, musí přibýt i do filtru. Další fragmenty IPv4 (bez hlavičky
protokolu) filtr pustí, protože je stack bez skládání stejně zahodí.

## Filtr DNS

Jméno ze seznamu `block.domains` resolver nepřeloží a vrátí
`ACCESS_DENIED`. Aplikace se tak dozví, že jde o blokování, a ne že jméno
neexistuje. Vzor `ads.example` blokuje jméno i jeho poddomény,
`*.ads.example` jen poddomény. Platí to pro systémový resolver i pro
dotaz na vlastní server. Do auditu jde `DNS_BLOCKED`.

⚠️ **DNS není ověřené** (DNSSEC, DoT, DoH chybí). Pravidlo se jménem
hostitele (i trvalé rozhodnutí uživatele) platí pro adresy ze systémového
resolveru. Kdo ovládá síť, může jméno přeložit na svou adresu, a ta pak
pravidlem projde.

## Audit a provoz

`netd` drží kruhový seznam záznamů (`audit_entries`, výchozí 4096).
Relace s právem `AUDIT` je čte přes `Net::audit(od, nejvíc)`.

| Druh (`fw::audit`) | Kdy |
|---|---|
| `ALLOWED` | spojení, první datagram na cíl, ping povolený pravidly |
| `DENIED` | zamítnuto pravidly relace nebo trvalým zákazem |
| `ASK_ALLOWED` / `ASK_DENIED` | rozhodnutí uživatele (nebo vypršení) |
| `LISTEN` | naslouchání na portu |
| `INBOUND_DROP` | filtr zahodil příchozí paket (`detail` = důvod; stejný zdroj a důvod nejvýš jednou za 10 s) |
| `DNS_BLOCKED` | překlad blokovaného jména |
| `BLOCKED_NET` | cíl v blokované síti |
| `PROFILE` | změna profilu sítě |

`Net::stats()` vrátí aktivní profil a provoz podle aplikací: odeslané
a přijaté bajty a počet socketů. Relace bez identity aplikace se počítají
pod `-`.

```rust
let (records, next) = root.audit(0, 256)?;
for r in &records {
    println!("{} {} {:?} {}", r.seq, r.app, r.remote, r.detail);
}
let (profile, apps) = root.stats()?;
```

## Protokol

Zprávy jsou v `netproto::fw`. Kanál `netd` ↔ `fwd` vytvoří init. Zprávy
na něm nemají odpovědi, každá stojí sama za sebe.

| Zpráva | Směr | Obsah |
|---|---|---|
| `ASK` (100) | netd → fwd | id, aplikace, protokol, cíl, jméno hostitele |
| `NETWORK` (101) | netd → fwd | brána, MAC brány |
| `ANSWER` (110) | fwd → netd | id, povolit (jen tento požadavek) |
| `CONFIG` (111) | fwd → netd | `FwConfig`: profil, filtr, blokované domény, rozhodnutí uživatele, kapacita auditu |
| `QUESTION` (120) | fwd → agent | id, aplikace, jméno aplikace, protokol, cíl, jméno hostitele |
| `DECIDE` (121) | agent → fwd | id, volba |
| `NETWORK_QUESTION` (122) | fwd → agent | id, id sítě, brána |
| `CLASSIFY` (123) | agent → fwd | id, profil |
| `CONFIG_GET` (130) | agent → fwd | → konfigurace TOML |
| `CONFIG_SET` (131) | agent → fwd | konfigurace TOML → () |
| `PROFILE_SET` (132) | agent → fwd | profil aktuální sítě → () |

Na požadavky agenta (`CONFIG_*`, `PROFILE_SET`) odpoví `fwd` ve formátu
`netproto::reply`. Odpověď začíná statusem ≤ 0, zpráva `fwd` kódem
operace > 0, takže je agent rozliší.

⚠️ **Agent musí být důvěryhodné UI.** Kdo drží kanál agenta, rozhoduje za
uživatele. Dnes ho drží init (testy). Dostat ho má jen systémové UI sezení
se zabezpečeným vstupem, ne aplikace. Jinak by si aplikace mohla povolit
cokoli sama.

## Testy

`user/init/src/tests/firewall.rs` (init hraje agenta uživatele, aplikace
`netprobe` má manifest s `ask = true`):

| Test | Ověří |
|---|---|
| síť | `NETWORK_QUESTION` po ARP brány, neznámá síť = `public`, `CLASSIFY home` uloží síť a přepne profil |
| manifest | aplikace v jobu s limity z manifestu, cíl z manifestu bez dotazu |
| dotazy | zamítnout jednou, povolit jednou (příště znovu dotaz), povolit vždy (bez dotazu, uloženo v konfiguraci i na FS), zakázat vždy |
| vypršení | bez odpovědi platí `ask_default` po `ask_timeout`, pozdní odpověď nic nezmění |
| blokování | doména i poddoména `ACCESS_DENIED`, blokovaná síť i pro kořenovou relaci |
| příchozí | veřejný profil zahodí spojení z hostitele přes `hostfwd`, domácí ho pustí |
| audit | všechny druhy záznamů, rostoucí pořadí, provoz aplikace, bez práva `AUDIT` nic |

Logiku filtru, konfigurace a manifestu testuje i hostitel
(`cargo test -p netfilter -p conf -p manifest`).

## Plán

- **UI agenta** v sezení (dialog „Povolit?“ se jménem aplikace z manifestu,
  přehled auditu a provozu).
- **IPv6** ve filtru, až ho bude umět stack.
- **Silnější identita sítě** než MAC brány (viz výše).
- **Brána spuštění a antivir** (odloženo): ověření podpisu programu před
  spuštěním, s volnějším režimem podle nastavení. Skenování souborů
  v izolovaných procesech bez sítě.
