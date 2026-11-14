# Síť

První část milníku 6 („síť“). Hotové jsou tyto části:

- ovladač karty **virtio-net** v userspace,
- ovladače karet **Intel e1000e, igc a Realtek r8169** (viz
  [Síťové karty](sitove-karty.md)),
- služba sítě **`netd`** se stackem TCP/IP (ARP, IPv4, ICMP, UDP, TCP),
  klientem DHCP a resolverem DNS,
- protokol **`netproto`** s přístupem přes capability a omezením cílů,
- synchronizace času službou **`ntpd`** (plynulá korekce přes službu času),
- **firewall**: manifest aplikace, dotazy uživateli, paketový filtr,
  profily sítí, audit a filtr DNS (viz [firewall.md](firewall.md)).

- **TLS 1.3 a HTTP/1.1, HTTP/2** jako knihovny v programech, úložiště
  kořenových certifikátů a program `fetch` (viz [TLS a HTTP](tls-http.md)).

| Část | Kde | Co dělá |
|---|---|---|
| ovladač | `user/virtio-net` | fronty virtio, DMA přes IOMMU, rámce Ethernetu ↔ kanál |
| služba | `user/netd` | stack TCP/IP (smoltcp), DHCP, DNS, oprávnění, sockety |
| protokol | `libs/netproto` | zprávy, oprávnění, formáty DNS a NTP, klientská knihovna |
| správce zařízení | `user/devmgr` | najde kartu na PCI, spustí ovladač |
| firewall | `user/fwd`, `libs/netfilter`, `libs/manifest` | konfigurace, dotazy uživateli, filtr paketů, manifesty |
| init | `user/init/src/system.rs`, `apps.rs` | spustí `netd` a `fwd` v jobech podle manifestů, drží kořenovou relaci, spustí `ntpd` |
| testy | `user/init/src/tests/net.rs`, `xtask/src/nethelper.rs` | testy v QEMU přes slirp proti serverům hostitele |

## Architektura

<figure class="diagram">
<svg viewBox="0 0 760 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Architektura site">
<defs><marker id="sa1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="160" height="60" rx="6"/><text class="t b" x="90" y="45" text-anchor="middle">program A</text><text class="m" x="90" y="63" text-anchor="middle">relace: jen TCP :443</text>
<rect class="box" x="10" y="100" width="160" height="60" rx="6"/><text class="t b" x="90" y="125" text-anchor="middle">program B</text><text class="m" x="90" y="143" text-anchor="middle">relace: UDP + DNS</text>
<rect class="acc" x="10" y="200" width="160" height="60" rx="6"/><text class="t b" x="90" y="225" text-anchor="middle">init</text><text class="m" x="90" y="243" text-anchor="middle">kořenová relace</text>
<path class="ln" d="M170 50 L298 110" marker-end="url(#sa1)"/>
<path class="ln" d="M170 130 L298 135" marker-end="url(#sa1)"/>
<path class="ln-acc" d="M170 230 L298 160" marker-end="url(#sa1)"/>
<text class="m" x="200" y="200">RESTRICT: uzší relace</text>
<rect class="ok" x="300" y="80" width="200" height="110" rx="6"/><text class="t b" x="400" y="105" text-anchor="middle">netd</text><text class="m" x="400" y="125" text-anchor="middle">oprávnění, sockety = kanály</text><text class="m" x="400" y="143" text-anchor="middle">smoltcp: ARP IPv4 ICMP UDP TCP</text><text class="m" x="400" y="161" text-anchor="middle">DHCP, vlastní resolver DNS</text><text class="m" x="400" y="179" text-anchor="middle">1 vlákno, port jádra</text>
<path class="ln" d="M500 135 L568 135" marker-end="url(#sa1)"/>
<text class="m" x="505" y="125">rámce</text>
<rect class="box" x="570" y="95" width="180" height="80" rx="6"/><text class="t b" x="660" y="120" text-anchor="middle">virtio-net</text><text class="m" x="660" y="139" text-anchor="middle">fronty RX/TX, 2 KiB buffery</text><text class="m" x="660" y="157" text-anchor="middle">DMA v doméně IOMMU</text>
<path class="ln" d="M660 175 L660 223" marker-end="url(#sa1)"/>
<rect class="warn" x="570" y="225" width="180" height="50" rx="6"/><text class="t b" x="660" y="248" text-anchor="middle">karta (QEMU slirp)</text><text class="m" x="660" y="265" text-anchor="middle">10.0.2.0/24</text>
</svg>
<figcaption>Programy mají jen kanály relací. Každý socket je další kanál od netd.</figcaption>
</figure>

Hlavní rozhodnutí:

- **Kanály jsou capability.** Program na síť nemůže, dokud mu někdo
  nepředá kanál *relace*. Dnes to dělá `init`, později správce sezení
  podle manifestu aplikace.
- **Socket je kanál.** Spojení TCP, naslouchání i socket UDP jsou
  samostatné kanály. Socket lze předat jinému procesu, například server
  předá přijaté spojení pracovnímu procesu. Zavření kanálu zavře socket.
  Na jednom kanálu může být víc dotazů najednou: každý nese číslo
  a termín a odpověď patří k číslu (viz [Protokol](#protokol-netproto)).
  Čtení a zápis jednoho spojení z různých vláken si nepřekáží, dlouhý
  connect nebrzdí ostatní dotazy relace **[ověřeno v QEMU]** (sady
  `sit soubeh` a `firewall`). Protitlak dávají fronty směrů a limit
  dotazů relace.
- **API není kopie BSD socketů.** Typy jsou ve stylu Rustu: `Net`,
  `TcpStream`, `TcpListener`, `UdpSocket`. Vrstva POSIX pro porty
  C programů je možná později nad nimi.
- **Stack je zatím smoltcp** (licence 0BSD), schovaný za modulem `stack`
  ve službě. Protokol `netproto`, ovladač ani programy na něm nezávisí,
  takže výměna za vlastní stack nic venku nezmění. Resolver DNS je už
  vlastní, viz [Plán](#plán).
- **Adresy jsou `core::net::IpAddr`**, tedy IPv4 i IPv6. Služba zatím umí
  jen IPv4 a pro IPv6 vrací `UNREACHABLE`. Protokol se přidáním IPv6
  nezmění.

## Ovladač virtio-net

Spouští ho správce zařízení `devmgr` stejně jako ovladač disku: zařízení
PCI `1AF4:1041` (moderní), případně `1AF4:1000` (přechodné). Ovladač
dostane:

1. omezený Resource: jen BARy a konfiguraci této funkce, kvóty přerušení
   a DMA, s IOMMU vlastní doménu DMA,
2. stránku konfiguračního prostoru PCI,
3. kanál ke službě sítě (`netproto::link`).

**Vyjednané vlastnosti:** `VERSION_1`, `MAC`, `STATUS`,
`ACCESS_PLATFORM`. Offload kontrolních součtů ani GSO se nepoužívají
(kontrolní součty počítá stack), stejně jako `MRG_RXBUF` (rámec se vždy
vejde do jednoho bufferu).

**Fronty:** 0 = příjem, 1 = odesílání, každá nejvýš 256 bufferů po 2 KiB
(kolik zařízení nabídne; QEMU 256). Paměť pro DMA a přerušení dostane
ovladač od `devmgr` podle druhu zařízení (síť: 2 MiB, 4 přerušení).
Buffer obsahuje hlavičku virtio-net (12 B) a rámec. Obě fronty mají
jeden vektor MSI-X. Ovladač čeká jedním `OBJECT_WAIT_MANY` na přerušení
a na zprávy od služby:

- **Přerušení:** potvrdí ho (`Interrupt::ack`) ještě před obsluhou,
  přijaté rámce pošle službě a buffery vrátí zařízení.
- **Odesílání:** volné buffery vrátí mezi volné. Když volný buffer není,
  kanál od služby nečte (protitlak).

Pro tento režim má `libs/virtio` nové metody `Virtqueue::post` a
`take_used_any`. Každý deskriptor je samostatný buffer s vlastním číslem.
Synchronní režim disku (`submit` + `take_used`) se nezměnil.

**Protokol linky** (`netproto::link`) je obousměrný, bez odpovědí:

| Typ | Směr | Obsah |
|---|---|---|
| `INFO` (1) | ovladač → netd, první zpráva | MAC (6 B), MTU u16, příznaky u8, rychlost u32 (Mb/s, 0 = neznámá) |
| `FRAME` (2) | oba směry | rámec Ethernetu bez FCS (14 až 1514 B) |
| `LINK` (3) | ovladač → netd | příznaky u8, rychlost u32 při změně stavu linky |

Příznaky: bit 0 = linka připojena, bit 1 = plný duplex. Rámec, který se
nevejde do fronty druhé strany, se zahodí jako na skutečné síti.
Spolehlivost řeší TCP. Když se linka znovu připojí (`LINK` s bitem 0),
`netd` začne DHCP znovu: za kabelem může být jiná síť.

> **Změna proti první verzi:** `INFO` a `LINK` neměly rychlost a duplex
> a `netd` po znovupřipojení linky DHCP neopakoval. **Proč:** ovladače
> skutečných karet ([síťové karty](sitove-karty.md)) linku hlásí a kabel
> se dá přepojit. Kratší zprávy (`INFO` 10 B, `LINK` 2 B) se čtou dál.
> Offloady protokol záměrně nenese: smoltcp ověřuje součty pro celé
> zařízení, ne pro jednotlivý rámec.

Ovladače karet Intel e1000e a igc a Realtek r8169 mluví s `netd` stejným
protokolem, viz [Síťové karty](sitove-karty.md).

⚠️ **DMA bez IOMMU:** ovladač dostává buffery jen z paměti domény
zařízení. Na stroji bez IOMMU ale karta přes DMA zapíše **kamkoli do
paměti**. Vadná nebo škodlivá karta (či její firmware) tak může přepsat
jádro. Jádro chybějící IOMMU zaloguje. Na skutečném hardwaru pouštějte
síť jen s IOMMU.

⚠️ **Vadné zařízení:** když zařízení vrátí nesmyslný index fronty nebo
buffer, který nemělo (`CORRUPTED`), ovladač ho resetuje, aby přestalo
používat DMA, a skončí. Služba sítě pak skončí také (restart ovladačů
zatím chybí).

## Služba netd

Start (handly `USER` v pořadí):

1. kanál k ovladači karty,
2. kanál **kořenové relace**, kterou drží `init`,
3. (volitelně) kanál ke službě firewallu `fwd` (viz [firewall.md](firewall.md)).
   Bez něj platí veřejný profil a dotazy aplikací se odmítnou.

Služba počká na `INFO` od ovladače, odpoví rodiči `ok` a spustí DHCP.
Pak v jednom vlákně dokola opakuje:

1. `poll` stacku: přijaté rámce, časovače TCP, DHCP,
2. posun rozpracovaných požadavků (spojení, čtení, dotazy DNS, ping),
3. `PORT_WAIT` na **portu jádra** (viz [Porty](syscally.md#porty)),
   s termínem podle nejbližšího časovače a termínů dotazů. Každý kanál má
   na portu dvě jednorázové registrace: „ke čtení“ (obnoví se po přečtení
   dávky nejvýš 32 zpráv, i když jsou dotazy rozpracované) a „zavřeno“. Klíč paketu nese číslo
   slotu a jeho generaci, takže starý paket po uvolnění slotu nic nespletě.
   Práce v každém kole je úměrná aktivním objektům (rozpracované
   požadavky, přijaté pakety), ne počtu všech socketů.

| Vlastnost | Hodnota |
|---|---|
| buffery TCP | výchozí 64 KiB příjem + 64 KiB odesílání na spojení; program může zvolit 4 KiB až 16 MiB (`Buffers`, jako `SO_RCVBUF`) |
| keep-alive TCP | po 60 s nečinnosti, spojení bez odezvy zrušeno po 120 s |
| navázání spojení bez limitu | vzdá se po 75 s (`TIMED_OUT`) |
| zavření spojení | FIN, úklid po doběhnutí nebo nejpozději po 30 s |
| fronta naslouchání | 1 až 4096 spojení (jako `SOMAXCONN`); 16 souběžných navázání mezi dvěma koly služby |
| dočasné porty | 49152 až 65535, začátek náhodný; posledních 4096 uvolněných se znovu nepřidělí (stará relace v NAT/slirp) |
| souběžné dotazy na socketu | v každém směru (příjem: `RECV`, `RECV_FROM`; odesílání: `SEND`, `SHUTDOWN`, `SEND_TO`; `ACCEPT`) rozpracovaný nejvýš jeden, další čekají ve frontě FIFO, dohromady nejvýš 8 (`DIRECTION_QUEUE`), další `NO_RESOURCES` |
| souběžné dotazy na relaci | nejvýš `Quota::ops` (výchozí 32, meze 1 až 1024, v manifestu `ops`) jen pro tuto relaci. Další dotazy služba odloží (nejvýš 32 zpráv, FIFO) a spustí po uvolnění místa; pak relaci přestane číst a zprávy čekají v kanálu v jádře (protitlak, bez chyby). `CANCEL` platí i pro odložené dotazy |
| datagram UDP | nejvýš 65 507 B (největší obsah datagramu IPv4; fragmentace a skládání IPv4) a nejvýš velikost fronty socketu (výchozí 64 KiB) |
| DNS | dotaz A (a AAAA) s EDNS0 (1232 B), zkrácená odpověď přes TCP, opakování po 1 s se střídáním serverů, nejvýš 30 s |

**DHCP** nastaví adresu, výchozí bránu a servery DNS. Ztráta pronájmu
adresu zruší a spojení pak dostanou `UNREACHABLE`. Ručně nastavené servery
DNS (`SET_DNS`, právo `CONFIGURE`) mají přednost před servery z DHCP.

**Chyby ICMP:** smoltcp u TCP ignoruje ICMP „cíl nedosažitelný“. Služba ho
při navazování spojení zachytí (RFC 1122 4.2.3.9) a spojení ukončí
s `CONNECTION_REFUSED` (port nedosažitelný) nebo `UNREACHABLE`, místo aby
čekala na vypršení.

⚠️ **slirp na Windows odmítnutí nehlásí:** spojení z hosta na zavřený port
hostitele nedostane RST ani chybu ICMP. SYN zůstane bez odpovědi
a navazování skončí `TIMED_OUT`. Na Linuxu slirp pošle RST
(`CONNECTION_REFUSED`). Test ověří to chování, které na hostiteli platí
(`refuses` v `net-test`).

**Počet socketů:** díky portu cena čekání nezávisí na počtu kanálů.
Pevný počet není: každý socket (i relace) se strhává z **rozpočtu své
relace** (počet socketů a bajty bufferů, viz [Oprávnění](#oprávnění)).
Kořenová relace dostane 3/4 uživatelské paměti, kterou job služby ještě
smí dostat (limit z manifestu `netd.manifest.toml`). Kanály a registrace
na portu hlídá rozpočet jádra jobu. Test otevře 200 socketů najednou.

> **Změna proti druhé verzi:** pevné meze služby byly 16 384 objektů,
> fronta naslouchání 8, datagram 8 KiB, buffery 64 KiB pro všechny,
> 256 zapamatovaných jmen na relaci a dotaz DNS jen přes UDP s 512 B.
> Velký server by na ně narazil, a přitom nechránily rozumně paměť
> služby. Teď platí rozpočty relací a jobu a meze dané protokoly.

> **Změna proti první verzi:** služba čekala jedním `OBJECT_WAIT_MANY`
> (nejvýš 64 objektů), takže socketů mohlo být nejvýš 63. Teď čeká na
> portu jádra (`PORT_WAIT`).

**Náhodnost:** čísla TCP (semeno smoltcp), dočasné porty, ID dotazů DNS
a identifikátory pingu jdou přímo z generátoru jádra (`RANDOM_GET`,
ChaCha20, viz [Náhodná čísla](syscally.md#náhodná-čísla)). Služba nedrží
vlastní stav, ze kterého by šlo odhadnout další hodnoty. Když generátor
jádra ještě není nasazený (bez RDRAND a bez virtio-rng), služba to
zaloguje.

> **Změna proti první verzi:** služba měla vlastní nekryptografický
> generátor se semenem z RDRAND, času a MAC. Bez RDRAND byla ID dotazů DNS
> odhadnutelná.

⚠️ **DNS bez ověření:** odpovědi nejsou podepsané (DNSSEC) ani šifrované
(DoT, DoH). Kdo ovládá síť, může jméno přeložit na jinou adresu. Ochranu
obsahu přinese až TLS s ověřením certifikátu.

## Oprávnění

Relace má **práva** (co smí) a **vrstvy omezení** (kam smí). Drží je
služba, klient je nevidí a nemůže je změnit.

| Právo | Bit | Dovoluje |
|---|---|---|
| `INFO` | 0 | stav rozhraní (`info`, `wait_configured`) |
| `TCP_CONNECT` | 1 | odchozí spojení TCP |
| `TCP_LISTEN` | 2 | naslouchání TCP |
| `UDP` | 3 | sockety UDP |
| `RESOLVE` | 4 | překlad jmen |
| `PING` | 5 | ICMP echo |
| `RESTRICT` | 6 | vytvářet uzší relace pro jiné programy |
| `CONFIGURE` | 7 | nastavení rozhraní (zatím servery DNS) |
| `AUDIT` | 8 | audit firewallu a provoz aplikací (`audit`, `stats`) |

`rights::CLIENT` = `INFO | TCP_CONNECT | UDP | RESOLVE` je běžný klient.

**Vrstva omezení** (`Policy`) má dvě části. `None` v kterékoli z nich
znamená bez omezení:

- `connect`: pravidla cílů (`Rule`): protokoly (TCP, UDP, ICMP), cíl a
  rozsah portů. Cíl je *libovolný*, *síť* (`10.0.2.0/24`) nebo *jméno
  hostitele* (`api.example.com`, `*.example.com` = všechny poddomény bez
  samotného `example.com`).
- `bind`: rozsahy místních portů pro naslouchání a pro socket UDP na
  zvoleném portu. Port 0 (přidělí služba) je povolený vždy.

<figure class="diagram">
<svg viewBox="0 0 760 230" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Vrstvy omezeni relace">
<defs><marker id="sa2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="80" width="150" height="70" rx="6"/><text class="t b" x="85" y="105" text-anchor="middle">kořen (init)</text><text class="m" x="85" y="123" text-anchor="middle">všechna práva</text><text class="m" x="85" y="139" text-anchor="middle">bez vrstev</text>
<path class="ln" d="M160 115 L238 115" marker-end="url(#sa2)"/>
<rect class="box" x="240" y="70" width="220" height="90" rx="6"/><text class="t b" x="350" y="95" text-anchor="middle">relace aplikace</text><text class="m" x="350" y="114" text-anchor="middle">práva CLIENT</text><text class="m" x="350" y="131" text-anchor="middle">vrstva 1: TCP *.example.com:443</text><text class="m" x="350" y="148" text-anchor="middle">+ UDP 10.0.0.0/8:53</text>
<path class="ln" d="M460 115 L538 115" marker-end="url(#sa2)"/>
<rect class="box" x="540" y="70" width="210" height="90" rx="6"/><text class="t b" x="645" y="95" text-anchor="middle">relace pluginu</text><text class="m" x="645" y="114" text-anchor="middle">práva ∩ (nejde přidat)</text><text class="m" x="645" y="131" text-anchor="middle">vrstva 1 + vrstva 2:</text><text class="m" x="645" y="148" text-anchor="middle">jen TCP :443</text>
<text class="m" x="10" y="195">Cíl musí projít všemi vrstvami. Odvozená relace nikdy nesmí víc než ta, ze které vznikla.</text>
<text class="m" x="10" y="215">Jméno hostitele platí pro adresy, které relace dostala od systémového resolveru.</text>
</svg>
<figcaption>Každé zúžení přidá vrstvu; práva jdou jen ubírat.</figcaption>
</figure>

**Pravidlo se jménem hostitele** platí pro adresy, které relace pro to
jméno dostala od **systémového** resolveru (servery z DHCP nebo nastavené
přes `SET_DNS`). Služba si překlad zapamatuje s platností podle TTL
(nejméně 5 minut, protože program se po překladu připojuje později
a krátké TTL CDN by spojení rozbilo; nejvýš den). Na relaci je nejvýš
4096 záznamů: nejdřív vypadnou vypršené, pak nejstarší. Odpověď od
serveru, který si program zvolil sám (`resolve_with` se serverem), se
nepočítá. Jinak by si program mohl povolené jméno „přeložit“ na
libovolnou adresu.

**Rozpočet relace** (`Quota`): nejvíc socketů a bajtů bufferů. Každý
socket (spojení, naslouchání, socket UDP, rozpracovaný dotaz DNS a ping)
a každá podřízená relace (2 KiB) se strhne z rozpočtu relace i všech
nadřazených, stejně jako u jobů jádra. Překročení = `NO_RESOURCES`.

**Identita aplikace a dotaz:** zúžení (`RESTRICT`) může nést identitu
aplikace (z manifestu) a příznak `ask`. Cíl mimo pravidla takové vrstvy
pak rozhodne uživatel přes firewall (viz [Dotaz uživateli](firewall.md#dotaz-uživateli)).
Vrstva bez `ask` je pevná hranice.

**Socket UDP** podléhá omezení i při příjmu: datagram od odesílatele mimo
povolené cíle se zahodí. Naslouchání TCP omezuje jen `bind` a příchozí
spojení se podle `connect` nefiltrují.

Příklad: `init` vytvoří relaci pro aplikaci, která smí jen HTTPS na své
API a překládat jména.

```rust
use netproto::{Policy, Rule, Target, proto, rights};

let policy = Policy::only(vec![Rule::port(
    proto::TCP,
    Target::Host("*.example.com".into()),
    443,
)]);
let app_net = root.restrict(rights::INFO | rights::TCP_CONNECT | rights::RESOLVE, &policy)?;
// app_net.into_channel() se předá aplikaci ve startovní zprávě.
```

## API pro programy

Klientská knihovna je v `netproto` (feature `client`). Všechny metody
blokují. Timeout `None` znamená bez limitu.

**Vlákna** **[ověřeno v QEMU]**: `Net`, `TcpStream`, `TcpListener`
a `UdpSocket` jsou `Send + Sync` a metody berou `&self`. Na jednom kanálu
může čekat víc vláken najednou. `try_clone()` vrátí další handle na
stejný socket (sdílí kanál i limity `set_read_timeout`
a `set_write_timeout`, jako `SO_RCVTIMEO`), `Net` je `Clone`. Odpovědi
rozděluje sdílený `Mux` podle čísla dotazu. Vlákno, které čeká a nikdo
zrovna nečte, čte z kanálu samo, cizí odpovědi uloží a probudí jejich
vlákna (futex). Jedno vlákno tedy nemá žádný syscall navíc.

**Termín a zrušení** **[ověřeno v QEMU]**: varianty `*_ctx`
(`Net::tcp_connect_ctx`, `resolve_ctx`, `TcpStream::recv_ctx`,
`send_ctx`, `TcpListener::accept_ctx`, `UdpSocket::recv_from_ctx`) berou
absolutní termín `Deadline` (`Deadline::after(Option<Duration>)`,
`Deadline::NONE`, `Deadline::TRY` = jen zkusit) a volitelný
`CancelToken`. `CancelToken::cancel()` z jiného vlákna zruší rozpracované
dotazy s tímto tokenem (vrátí `CANCELED`) a každý další dotaz s ním vrátí
`CANCELED` hned. U spojení se uvolní socket i port, u dotazu firewallu
čekání na uživatele. Čas dotazu na uživatele se do termínu nepočítá.
`Deadline::TRY` je neblokující režim: `recv_ctx`, `send_ctx`, `accept_ctx`
a `recv_from_ctx` bez dat, místa nebo spojení vrátí `SHOULD_WAIT` (ne
`TIMED_OUT`) **[ověřeno v QEMU]**. Kdy to zkusit znovu, řekne
[připravenost socketu](#připravenost-socketů).

**Pojistka v klientovi:** limity operací hlídá `netd`, klient má navíc
vlastní: termín operace + 10 s, operace bez limitu (stav, relace, socket)
30 s, operace s možným dotazem firewallu (spojení, datagram na nový cíl,
ping, překlad vlastním serverem) navíc 300 s. Když `netd` neodpoví,
program dostane `TIMED_OUT`, klient pošle `CANCEL` a v logu je
`netproto: sluzba site neodpovedela na dotaz <op> (id N) vcas, rusim ho`.
Kanál jde používat dál: pozdní odpověď má číslo, na které už nikdo
nečeká, a zahodí se i s handly.

> **Změna proti první verzi:** na kanálu byl nejvýš jeden požadavek
> a odpověď neměla číslo. Dvě vlákna na jednom spojení si prohodila
> odpovědi, zápis čekal za blokovaným `RECV` a connect čekající na dotaz
> firewallu (až 300 s) zablokoval celou relaci. Kanál po vypršení
> pojistky nešel použít. **Proč:** std (`TcpStream` je `Read` i `Write`,
> `try_clone`, vlákno čtení a vlákno zápisu) a async runtimy potřebují
> souběh. Hlavička s číslem, termínem a zrušením je zároveň kontext
> operace budoucího IDL ([architektura](architektura.md)). Starý formát
> zprávy se nečte: převedli se všichni klienti najednou.

```rust
use core::time::Duration;
use netproto::io::{Read, Write};
use netproto::Net;

fn main(startup: libos::Startup) -> i64 {
    // Kanál relace dostal program od init (např. první handle USER).
    let net = Net::new(libos::Channel::from_handle(startup.handles.into_iter().next().unwrap()));
    net.wait_configured(Some(Duration::from_secs(10))).unwrap();

    // TCP
    let mut s = net.tcp_connect_host("example.com", 80, Some(Duration::from_secs(5))).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5)));
    s.write_all(b"GET / HTTP/1.0\r\nHost: example.com\r\n\r\n").unwrap();
    let mut page = Vec::new();
    s.read_to_end(&mut page).unwrap();

    // UDP
    let u = net.udp_bind(0).unwrap();
    u.send_to(b"ahoj", "10.0.2.2:7".parse().unwrap()).unwrap();
    let mut buf = [0u8; 1500];
    let (n, from) = u.recv_from(&mut buf, Some(Duration::from_secs(1))).unwrap();

    // Server
    let l = net.tcp_listen(8080, 4).unwrap();
    let (conn, peer) = l.accept().unwrap();
    0
}
```

Čtení a zápis z různých vláken a zrušení:

```rust
use netproto::{CancelToken, Deadline};

let s = net.tcp_connect(addr, Some(Duration::from_secs(5)))?;
let reader = s.try_clone()?;
let stop = CancelToken::new();
let token = stop.clone();
libos::thread::spawn("cteni", move || {
    let mut buf = [0u8; 4096];
    // Čeká bez limitu, dokud nepřijdou data nebo `stop.cancel()`.
    while let Ok(n) = reader.recv_ctx(&mut buf, Deadline::NONE, Some(&token)) {
        if n == 0 { break; }
        // ...
    }
})?;
s.write(b"ahoj")?;      // neblokuje se za čtením
stop.cancel();          // čtení skončí s CANCELED
```

| Metoda | Právo | Chyby navíc |
|---|---|---|
| `Net::info`, `wait_configured` | `INFO` | `TIMED_OUT` |
| `Net::tcp_connect`, `tcp_connect_host` | `TCP_CONNECT` + cíl | `CONNECTION_REFUSED`, `UNREACHABLE`, `TIMED_OUT` |
| `Net::tcp_listen` | `TCP_LISTEN` + `bind` | `ADDRESS_IN_USE` |
| `Net::udp_bind` | `UDP` + `bind` | `ADDRESS_IN_USE` |
| `Net::resolve`, `resolve_with` | `RESOLVE` (+ cíl UDP u vlastního serveru) | `NOT_FOUND`, `TIMED_OUT`, `UNREACHABLE` |
| `Net::ping` | `PING` + cíl ICMP | `TIMED_OUT`, `UNREACHABLE` |
| `Net::restrict`, `restrict_with` | `RESTRICT` | `INVALID_ARGS` (neplatná pravidla, `ask` bez identity), `NO_RESOURCES` |
| `Net::audit`, `stats` | `AUDIT` | |
| `Net::tcp_connect_with`, `tcp_listen_with`, `udp_bind_with` | jako bez `_with` | `NO_RESOURCES` (rozpočet relace) |
| varianty `*_ctx` | jako bez `_ctx` | `CANCELED` (zrušeno tokenem), `TIMED_OUT` (termín) |
| každý dotaz na socketu | | `NO_RESOURCES` (fronta směru plná, víc než 8 dotazů jednoho směru) |
| `Net::tcp_listen_addr`, `udp_bind_addr` | jako `tcp_listen`, `udp_bind` | `ADDRESS_NOT_AVAILABLE` (adresa není `0.0.0.0` ani adresa rozhraní; i `127.0.0.1`, loopback zatím chybí) |
| `Net::set_dns` | `CONFIGURE` | |
| `Net::ntp_query` | `UDP` + cíl | `TIMED_OUT`, `BAD_STATE` (server nesynchronizovaný) |
| `TcpStream::read`, `write` | | `CONNECTION_RESET`, `TIMED_OUT` |
| `UdpSocket::send_to` | cíl | `OUT_OF_RANGE` (nad 65 507 B nebo nad frontu socketu) |

Chybějící právo nebo cíl mimo omezení vrací `ACCESS_DENIED`. Nové kódy
chyb popisuje [chyby.md](chyby.md).

`UdpSocket::local_addr` a `TcpListener::local_addr` se ptají služby
(vracejí `Result`): socket vytvořený před DHCP má adresu 0.0.0.0 jen do
nastavení rozhraní. Adresa místo jména (i IPv6, `"::1"`) se v `resolve`
vrátí hned bez dotazu.

`TcpStream::read` vrací `Ok(0)` na konci proudu (druhá strana ukončila
zápis). `CONNECTION_RESET` znamená, že spojení přerušil RST nebo že druhá
strana přestala odpovídat. Spojení lze předat jinému procesu:
`into_channel()` u odesílatele (`BAD_STATE`, když spojení drží klon nebo
na něm běží dotaz), `TcpStream::from_channel()` u příjemce.

## Připravenost socketů

Neblokující smyčky (backend mio, a nad ním tokio) potřebují čekat na mnoho
socketů najednou a pak operaci zkusit bez čekání **[ověřeno v QEMU]**
(sada „sit pripravenost“).

- **`READINESS`** na kanálu socketu (spojení, naslouchání, UDP) vrátí
  **event** v handlech odpovědi. Klient dostane duplikát jen s právem
  `WAIT` (a `DUPLICATE`, `TRANSFER`), `netd` si drží `SIGNAL`. Bity jsou
  uživatelské bity eventu ([`OBJECT_SIGNAL`](syscally.md#object_signal)),
  takže na ně jde čekat portem jádra jako na jakýkoli jiný objekt.
- **Bity** (`netproto::ready`):

| Bit | Signál | Kdy |
|---|---|---|
| `READABLE` | `USER_0` | spojení: data, konec proudu nebo chyba (čtení neblokuje); naslouchání: spojení čeká na `accept`; UDP: datagram |
| `WRITABLE` | `USER_1` | místo ve frontě odesílání (nebo chyba) |
| `HUP` | `USER_2` | druhá strana ukončila zápis (FIN) nebo je spojení zavřené |
| `ERROR` | `USER_3` | spojení přerušeno (RST, druhá strana přestala odpovídat) |

- **Stav je úrovňový:** `netd` po každém kole přepočítá bity sledovaných
  socketů (těch, pro které někdo zavolal `READINESS`) a do eventu zapíše
  jen změnu. Práce navíc je úměrná počtu sledovaných socketů. Bit
  nastavený už při registraci se na portu ohlásí hned.
- **Smyčka jako v mio:** `Port::wait_async(readiness.handle(), token,
  maska)` = `Registry::register`; `Port::wait` = `Poll::poll`; po události
  operace s `Deadline::TRY` (`SHOULD_WAIT` = už není připravený, což se
  může stát, protože stav platí k poslednímu kolu `netd`) a nová
  registrace (jednorázová jako epoll s `ONESHOT`). Probuzení z jiného
  vlákna (`Waker`) = event se `SIGNALED` na stejném portu.

```rust
use netproto::{Deadline, ready};

let port = libos::Port::create()?;
let r = stream.readiness()?;
port.wait_async(r.handle(), TOKEN, ready::READABLE)?;
loop {
    let ev = port.wait(None)?;
    let mut buf = [0u8; 4096];
    match stream.recv_ctx(&mut buf, Deadline::TRY, None) {
        Ok(n) => { /* data */ }
        Err(libos::Error::ShouldWait) => {} // zkusit po další události
        Err(e) => return Err(e),
    }
    port.wait_async(r.handle(), TOKEN, ready::READABLE)?;
}
```

> **Změna proti první verzi:** připravenost neexistovala, neblokující
> smyčku šlo udělat jen vláknem na socket. **Proč:** mio/tokio a servery
> s tisíci spojení. Jádro na to dostalo uživatelské bity eventu (malá
> změna `OBJECT_SIGNAL`), `netd` jimi stav ukazuje bez zpráv navíc.

## Protokol netproto

Verze 2 **[ověřeno v QEMU]**. Každá zpráva má hlavičku s kontextem
operace (`netproto::msg`):

| Zpráva | Hlavička | Tělo |
|---|---|---|
| dotaz | `op` u32, `id` u32, termín u64 (16 B) | parametry |
| odpověď | `id` u32, `status` i32 (8 B) | výsledek (jen při `status` 0) |

- **`id`** volí klient. Je jedinečné mezi rozpracovanými dotazy na kanálu
  a nikdy není 0. Odpověď nese stejné `id`, pořadí odpovědí není dané.
  Dotaz s `id`, které už čeká, dostane `ALREADY_EXISTS`.
- **Termín** je absolutní čas monotónních hodin jádra v ns (stejné hodiny
  ve všech procesech). `u64::MAX` = bez limitu, 0 (nebo minulost) = jen
  zkusit. Vypršení = `TIMED_OUT`, u „jen zkusit“ (0) `SHOULD_WAIT`
  (`RECV`, `SEND`, `ACCEPT`, `RECV_FROM`, `SEND_TO`). Doba čekání ve
  frontě se tak nepočítá dvakrát.
- **`CANCEL`** (op `0xFFFF_FFFF`, na každém kanálu): `id` v hlavičce je
  číslo rušeného dotazu, tělo je prázdné a vlastní odpověď nemá. Rušený
  dotaz (rozpracovaný i ve frontě) odpoví `CANCELED`; `SEND`, ze kterého
  už část prošla, odpoví počtem přijatých bajtů. Neznámé nebo hotové
  `id` služba ignoruje.
- Zprávu bez platné hlavičky služba zahodí (nemá číslo, kam odpovědět).

Nový socket nebo relace jde v handlech odpovědi. Kódování: čísla
little-endian, adresa IP = rodina u8 (4/6) + 4/16 B, `socket_addr` =
adresa + port u16, řetězec = délka u16 + UTF-8, data = zbytek zprávy.
Kanál mezi `fwd` a agentem uživatele má vlastní formát odpovědi
(`fw::agent_reply`), viz [firewall.md](firewall.md).

| Kanál | Op | Parametry → výsledek |
|---|---|---|
| relace | `INFO` (1) | → Info |
| relace | `WAIT_CONFIGURED` (2) | → Info (do termínu) |
| relace | `TCP_CONNECT` (3) | cíl, Buffers → místní adresa + kanál spojení |
| relace | `TCP_LISTEN` (4) | místní adresa (`0.0.0.0` nebo adresa rozhraní), fronta u16, Buffers → místní adresa + kanál naslouchání |
| relace | `UDP_BIND` (5) | místní adresa, Buffers → místní adresa + kanál socketu |
| relace | `RESOLVE` (6) | jméno, rodina u8 (0/4/6), má server u8 [server] → počet u16, adresy, TTL u32 |
| relace | `PING` (7) | adresa, pořadí u16, délka u16 → odezva ns u64 |
| relace | `RESTRICT` (8) | Restriction → kanál relace |
| relace | `SET_DNS` (9) | počet u8, servery → () |
| relace | `AUDIT` (10) | od u64, nejvíc u16 → další u64, počet u16, záznamy |
| relace | `STATS` (11) | → profil, počet u16, provoz aplikací |
| spojení | `SEND` (20) | data → přijato u32 |
| spojení | `RECV` (21) | nejvýš u32 → data (prázdná = konec proudu) |
| spojení | `SHUTDOWN` (22) | → () (FIN, čtení pokračuje) |
| spojení | `ADDRS` (23) | → místní, vzdálená adresa |
| naslouchání | `ACCEPT` (30) | → vzdálená adresa, místní adresa + kanál spojení |
| naslouchání, UDP | `LOCAL_ADDR` (31) | → místní adresa (podle aktuální adresy rozhraní) |
| UDP | `SEND_TO` (40) | cíl, data → () (na místo ve frontě nejvýš 5 s) |
| každý socket | `READINESS` (50) | → () + event připravenosti v handlech (viz [Připravenost socketů](#připravenost-socketů)) |
| UDP | `RECV_FROM` (41) | nejvýš u32 → odesílatel, data |

**Info:** MAC (6 B), MTU u16, příznaky u8 (bit 0 linka, bit 1 nastaveno),
má adresu u8 [adresa, prefix u8], má bránu u8 [brána], počet DNS u8
[adresy], práva relace u32.

**Buffers:** příjem u32, odesílání u32 (0 = výchozí 64 KiB, jinak
4 KiB až 16 MiB).

**Restriction:** práva u32, Policy, příznaky u8 (bit 0 = `ask`), aplikace
(řetězec, nejvýš 128 znaků `a-z A-Z 0-9 . - _`), sockety u32, bajty u64
(`u32::MAX` / `u64::MAX` = bez limitu), souběžné dotazy u32 (meze 1 až
1024; init dává 32).

**Policy:** má connect u8 [počet u32, pravidla], má bind u8 [počet u32,
(od u16, do u16)...]. Pravidlo = protokoly u8, druh cíle u8
(0 libovolný, 1 síť: adresa + prefix u8, 2 jméno: řetězec), port od u16,
port do u16. Služba odmítne (`INVALID_ARGS`) prázdné protokoly, prefix
delší než adresa, neplatné jméno a víc než 256 pravidel nebo 16 vrstev.

## DNS

Systémový překlad jde přes `dnsd` (viz níže). Neověřený resolver ve
službě (`user/netd/src/resolve.rs`, formát `netproto::dns`) slouží
dotazům na vlastní server a jako záloha podle profilu sítě:

- Pro rodinu 0 se posílají dotazy A i AAAA. Stačí, když odpoví jeden.
  Prázdná odpověď na AAAA nevadí.
- Odpověď se páruje podle náhodného ID dotazu a adresy serveru.
- Komprese jmen se rozbaluje s ochranou proti smyčce (nejvýš 32 skoků).
- NXDOMAIN = `NOT_FOUND`, jiná chyba serveru = `IO_ERROR`.
- Adresa místo jména (`"10.0.2.2"`) se vrátí hned bez dotazu.
- **EDNS0:** dotaz nabízí odpověď až 1232 B přes UDP (DNS Flag Day 2020:
  vejde se do jednoho paketu i přes IPv6 s MTU 1280, bez fragmentace).
- **Zkrácená odpověď** (bit TC): resolver se zeptá přes TCP na stejný
  server a port (RFC 7766, zpráva s délkou u16). U vlastního serveru
  stačí, že relace smí na server UDP.
- **Uvázlý tok:** opakování přes UDP jde po každých 3 pokusech z nového
  místního portu a spojení TCP, které se nenaváže do 3 s, se zahodí
  a naváže znovu (nejvýš 3×). Slirp na Windows pro tok drží jeden socket
  hostitele; když se pokazí, ztratil by se každý další pokus ze stejného
  portu (hláška `netd: DNS pres TCP: spojeni se nenavazalo do 3 s, novy
  pokus (n)`).
- **Blokované domény** firewallu vrátí `ACCESS_DENIED` bez dotazu (viz
  [Filtr DNS](firewall.md#filtr-dns)).

### Šifrované DNS a DNSSEC (`dnsd`)

<figure class="diagram">
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Sifrovane DNS">
<defs><marker id="sd1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="130" height="60" rx="6"/><text class="t b" x="75" y="45" text-anchor="middle">program</text><text class="m" x="75" y="64" text-anchor="middle">resolve()</text>
<path class="ln" d="M140 50 L198 50" marker-end="url(#sd1)"/>
<rect class="acc" x="200" y="20" width="160" height="110" rx="6"/><text class="t b" x="280" y="45" text-anchor="middle">netd</text><text class="m" x="280" y="64" text-anchor="middle">blokované domény</text><text class="m" x="280" y="82" text-anchor="middle">→ dnsd (kanál)</text><text class="m" x="280" y="100" text-anchor="middle">učí se pravidla</text><text class="m" x="280" y="118" text-anchor="middle">se jmény</text>
<path class="ln-acc" d="M360 50 L418 50" marker-end="url(#sd1)"/>
<rect class="ok" x="420" y="20" width="170" height="110" rx="6"/><text class="t b" x="505" y="45" text-anchor="middle">dnsd</text><text class="m" x="505" y="64" text-anchor="middle">DO + CD, DNSSEC</text><text class="m" x="505" y="82" text-anchor="middle">řetěz od kotvy IANA</text><text class="m" x="505" y="100" text-anchor="middle">mezipaměť (TTL)</text><text class="m" x="505" y="118" text-anchor="middle">OK/NX/BOGUS/…</text>
<path class="ln" d="M590 50 L638 50" marker-end="url(#sd1)"/>
<rect class="box" x="640" y="20" width="110" height="60" rx="6"/><text class="t b" x="695" y="45" text-anchor="middle">resolver</text><text class="m" x="695" y="64" text-anchor="middle">DoH / DoT</text>
<path class="ln" d="M280 130 L280 168" marker-end="url(#sd1)"/>
<rect class="warn" x="200" y="170" width="160" height="60" rx="6"/><text class="t b" x="280" y="195" text-anchor="middle">záloha UDP/TCP</text><text class="m" x="280" y="214" text-anchor="middle">servery z DHCP</text>
<text class="m" x="380" y="190">jen když dnsd odpoví UNAVAILABLE</text>
<text class="m" x="380" y="208">a profil sítě má</text>
<text class="m" x="380" y="226">unauthenticated = "fallback"</text>
</svg>
<figcaption>Systémový překlad: šifrovaně a ověřeně; neověřená záloha jen podle profilu sítě.</figcaption>
</figure>

Systémový překlad (relace bez vlastního serveru) jde ze `netd` do služby
**`dnsd`**. Ta se ptá **šifrovaně** (DoH, RFC 8484, nebo DoT, RFC 7858)
a odpovědi **ověřuje sama** (DNSSEC): řetěz DS od kořenové kotvy IANA, podpisy
RRSIG, důkazy neexistence NSEC a NSEC3. Resolveru se nevěří – lhát může
jen tak, že zónu prohlásí za nepodepsanou, a to důkaz z nadřazené zóny
nepustí.

| Výsledek `dnsd` | Pro program |
|---|---|
| adresy (ověřené, nebo zóna nepodepsaná) | adresy; `netd` se z nich učí pravidla se jménem |
| NXDOMAIN, NODATA (ověřené) | `NOT_FOUND` |
| **BOGUS** (podpis nebo důkaz neprošel) | `CORRUPTED` – útok, záloha se nespustí |
| UNAVAILABLE (žádný resolver neodpověděl) | podle profilu sítě: neověřený resolver (UDP/TCP na servery z DHCP nebo `SET_DNS`), nebo `UNREACHABLE` |

- **Kanál** `netd` ↔ `dnsd` vzniká před startem `netd` (4. handle); dotazy
  čekají v kanálu, než `dnsd` naběhne. Když `dnsd` skončí, `netd` překládá
  jen podle pravidla profilu.
- **Resolvery** v `dnsd.toml` mají adresu (`doh:https://jméno/cesta@adresa`,
  `dot:jméno@adresa[:port]`): `dnsd` jména resolverů nepřekládá. Relace
  sítě `dnsd` smí jen TCP na tyto adresy. Po chybě zkusí další a drží se
  posledního funkčního.
- **Kotvy:** DS kořene podle IANA `root-anchors.xml`: KSK-2017 (20326)
  a KSK-2024 (38696, podepisuje kořen od 11. 10. 2026). Při další výměně
  klíče kořene je potřeba `trust_anchors` aktualizovat (RFC 5011 zatím
  ne).
- **Algoritmy:** RSASHA256, RSASHA512, ECDSA P-256/P-384, Ed25519; otisky
  SHA-256 a SHA-384. Zóna s jiným algoritmem nebo s NSEC3 nad 100 iterací
  (RFC 9276) se bere jako nepodepsaná.
- **Mezipaměť** podle TTL (záporné odpovědi podle SOA), 1 024 záznamů,
  nejvýš 1 den; stavy zón (klíče) zvlášť, nejvýš 1 h. Podvržené odpovědi se
  neukládají.
- **Bez kontroly odvolání** certifikátu resolveru (jen OCSP stapling):
  `revocd` překládá jména přes `dnsd`, kontrola by se zacyklila.
- Bootstrap příkazy: `status` (`queries upstream cache_hits secure
  insecure bogus unavailable dnssec server error`), `flush`, `prefer <n>`.

```toml
# dnsd.toml
servers = ["doh:https://cloudflare-dns.com/dns-query@1.1.1.1", "dot:one.one.one.one@1.0.0.1"]
dnssec = true
trust_anchors = ["20326 8 2 E06D44B8...", "38696 8 2 683D2D0A..."]
```

> **Změna proti první verzi:** `netd` se ptal serverů z DHCP nešifrovaně
> a bez ověření. **Proč:** síť (Wi-Fi, poskytovatel) viděla, kam se
> připojujete, a mohla odpovědi podvrhnout. TLS sice podvržený server
> odhalí, ale DNSSEC a šifrování chrání i jména bez TLS a soukromí.


## NTP a hodiny

`Net::ntp_query` pošle dotaz NTP v4 (`netproto::ntp`) a vrátí opravený
čas, posun hodin a zpoždění cesty. Odpověď se páruje podle času odeslání
(pole „originate“). Server bez synchronizace (stratum 0, leap 3) vrátí
`BAD_STATE`.

Hodiny podle sítě seřizuje služba **`ntpd`**: měří posun proti
serverům, filtruje měření a navrhuje korekce službě času `timed`, která
jako jediná smí hodiny měnit (právo `SET_CLOCK`). Po první synchronizaci se
čas jen plynule dorovnává (`CLOCK_ADJUST`), nikdy neskočí zpět. Podrobně
v [Reálný čas](cas.md#synchronizace-podle-sítě-ntpd).

> **Změna proti první verzi:** init se jednou zeptal serveru NTP a čas
> nastavil skokem (`sync_time_ntp`, jen v testech). **Proč:** skok zpět
> rozbije programy, které čas porovnávají (certifikáty, soubory, logy),
> a jedno měření bez filtru je snadno podvrhnutelné. `ntpd` běží trvale,
> s relací sítě jen na nakonfigurované servery.

## TLS a HTTP

TLS 1.3 a klient HTTP/1.1 a HTTP/2 jsou knihovny v procesu programu
(`libs/tls`, `libs/httpc`) nad obecným proudem `netproto::io::{Read,
Write}`, takže fungují nad holým TCP i nad TLS stejně. Kořenové
certifikáty jsou v systémovém úložišti `/system/config/certs`, které
program dostane jako relaci FS jen pro čtení. Vše popisuje kapitola
[TLS a HTTP](tls-http.md).

> **Změna proti první verzi:** úložiště certifikátů mělo být služba nebo
> soubor; je to adresář na FS (sada Mozilla + vlastní CA + `distrust.txt`).
> **Proč:** správce přidá firemní CA souborem, aktualizace systému jeho
> soubory nepřepíše a práva hlídá FS jako u ostatních dat.

## Testy

`cargo xtask test` spustí QEMU s kartou `virtio-net-pci` v user-mode síti
(slirp) a na hostiteli pomocné servery (`xtask/src/nethelper.rs`). Servery
poslouchají jen na `127.0.0.1` na volných portech, takže souběžné běhy
testů si nevadí. Porty dostane host v souboru `net-test` v initfs. Slirp
překládá adresu 10.0.2.2 na 127.0.0.1 hostitele.

<figure class="diagram">
<svg viewBox="0 0 760 210" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Testy site pres slirp">
<defs><marker id="sa4" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="30" width="230" height="150" rx="6"/><text class="t b" x="125" y="55" text-anchor="middle">host (QEMU) 10.0.2.15</text><text class="m" x="125" y="78" text-anchor="middle">init: testy sit</text><text class="m" x="125" y="96" text-anchor="middle">netd + virtio-net</text><text class="m" x="125" y="114" text-anchor="middle">naslouchá :8080</text>
<rect class="warn" x="290" y="30" width="180" height="150" rx="6"/><text class="t b" x="380" y="55" text-anchor="middle">slirp</text><text class="m" x="380" y="78" text-anchor="middle">DHCP, brána 10.0.2.2</text><text class="m" x="380" y="96" text-anchor="middle">DNS 10.0.2.3</text><text class="m" x="380" y="114" text-anchor="middle">10.0.2.2 → 127.0.0.1</text><text class="m" x="380" y="132" text-anchor="middle">hostfwd → :8080</text>
<rect class="ok" x="520" y="30" width="230" height="150" rx="6"/><text class="t b" x="635" y="55" text-anchor="middle">xtask (127.0.0.1)</text><text class="m" x="635" y="78" text-anchor="middle">echo TCP, echo UDP</text><text class="m" x="635" y="96" text-anchor="middle">DNS (test.magnumopus)</text><text class="m" x="635" y="114" text-anchor="middle">NTP, zavřený port, RST</text><text class="m" x="635" y="132" text-anchor="middle">callback → hostfwd</text>
<path class="ln" d="M240 105 L288 105" marker-end="url(#sa4)"/>
<path class="ln" d="M470 105 L518 105" marker-end="url(#sa4)"/>
<path class="ln" d="M518 150 L472 150" marker-end="url(#sa4)"/>
<path class="ln" d="M288 150 L242 150" marker-end="url(#sa4)"/>
</svg>
<figcaption>Testy nepotřebují internet: všechny servery běží na hostiteli.</figcaption>
</figure>

| Test | Ověří |
|---|---|
| DHCP | adresa 10.0.2.15/24, brána 10.0.2.2, DNS 10.0.2.3, MTU, linka |
| ping | ICMP echo na bránu (slirp odpovídá sám), IPv6 = `UNREACHABLE` |
| DNS | dotaz přes slirp na server hostitele: A, A+AAAA, NXDOMAIN, neplatné jméno, adresa místo jména |
| DNS velká odpověď | 100 adres (víc než 512 B), odpověď zkrácená přes UDP → dotaz přes TCP |
| TCP echo | 256 KiB tam a zpět, místní a vzdálená adresa, `shutdown_write` a konec proudu |
| TCP odmítnutí | zavřený port = `CONNECTION_REFUSED` (na Windows `TIMED_OUT`, viz výše), port 0 = `INVALID_ARGS` |
| TCP přerušení | hostitel zavře spojení s nepřečtenými daty (RST): čtení i zápis = `CONNECTION_RESET` |
| TCP naslouchání | hostitel se připojí přes `hostfwd`, `ADDRESS_IN_USE`, vypršení `accept` |
| UDP echo | datagramy tam a zpět, dočasný port, vypršení |
| UDP velký datagram | 60 000 B tam a zpět (fragmentace a skládání IPv4) |
| oprávnění | chybějící práva, cíle mimo omezení, předání spojení jinému kanálu |
| jméno hostitele | pravidlo `*.magnumopus` platí po systémovém překladu, ne po překladu vlastním serverem |
| šifrované DNS | sada „sifrovane DNS“: ověřené A, CNAME, wildcard, AAAA; NXDOMAIN (NSEC i NSEC3), NODATA; nepodepsaná a podvržená zóna; přechod z nefunkčního resolveru, DoT, mezipaměť; pravidlo profilu |
| souběh (sada „sit soubeh“) | čtení a zápis jednoho spojení ze dvou vláken (zápis nečeká za čtením), dva čtenáři jednoho spojení (fronta FIFO), „jen zkusit“ = `SHOULD_WAIT` a termín 300 ms, zrušení čtení z jiného vlákna a token zrušený předem, místní adresa naslouchání a UDP (`ADDRESS_NOT_AVAILABLE` pro cizí adresu a 127.0.0.1) |
| plný limit relace (sada „firewall“) | relace s `ops = 1`: druhý dotaz se odloží (nečeká chybou), jeho zrušení přijde do 2 s, i když první dotaz dál čeká na uživatele |
| zrušení connectu (sada „firewall“) | connect čeká na dotaz uživatele, relace mezitím odpovídá na `INFO`, `CancelToken` ho zruší do 2 s (`CANCELED`), pozdní odpověď uživatele nic nezmění |
| NTP | `ntpd` proti serveru hostitele (sada „synchronizace casu“, viz [cas.md](cas.md#synchronizace-podle-sítě-ntpd)) |

**DNS v testech jen lokálně:** všechny dotazy jdou na pomocný server
hostitele z `nethelper` nebo na testovací DoH/DoT; init v testovacím běhu
nastaví i neověřenou zálohu systémového resolveru na pomocný server
(`set_dns`), takže ani výpadek `dnsd` nespadne na skutečné DNS. Limity
dotazů DNS v testech jsou 30 s (běh pod TCG a zátěží). Resolver hostitele
(10.0.2.3, slirp předává dotazy DNS Windows/Linuxu) testuje jen
`cargo xtask test --net-online` – závisí na síti hostitele a internetu. Bez pomocných serverů (`cargo xtask run`)
se testy, které je potřebují, přeskočí. V logu je pak
`[preskoceno] sit: ...`.

**Porty pomocných serverů:** žádný není pevný ani uhodnutelný. Každý
server si vezme dočasný port od jádra hostitele (bind na port 0, při chybě
jako Windows 10013 až 20 pokusů) a xtask porty předá hostu v initfs
(`net-test`). Port pro test odmítnutí (`closed`) xtask drží po celý běh
jako socket navázaný bez `listen`: spojení na něj jádro odmítne a žádný
jiný proces ho mezitím neobsadí. Port pro `hostfwd` drží xtask obsazený až
do startu QEMU. Když QEMU hned po startu skončí s hláškou o obsazeném portu
(QMP nebo `hostfwd`, souběžné běhy testů), xtask ho spustí znovu s novými
porty (nejvýš 3 pokusy, hláška `xtask: QEMU nenastartovalo (...), novy
pokus s jinymi porty`).

**Zátěž (víc QEMU naraz):** UDP testy posílají každý další pokus z nového
socketu (nový tok ve slirp; pozdní odpověď na starý pokus nepřijde),
`dnsd` čeká na navázání spojení s resolverem 5 s (ztracený SYN se opakuje).

Formáty a oprávnění testuje i hostitel: `cargo test -p netproto
--features std` (součást `cargo xtask test`). Testy firewallu popisuje
[firewall.md](firewall.md#testy).

## Plán

- **Vlastní stack:** nahradí smoltcp za modulem `stack` ve službě. Pořadí:
  ARP + IPv4 + ICMP, UDP, pak TCP (řízení zahlcení, SACK, okna). smoltcp
  (licence 0BSD) poslouží jako vzor a jeho testy jako sada pro porovnání.
  Protokol a programy se nezmění.
- **IPv6:** SLAAC, ICMPv6, AAAA už resolver umí. Protokol je připravený
  (adresy `IpAddr`).
- **Sdílená paměť pro data** místo kopírování zprávami (jako u disku).
- **Další operace socketu** (kódy rezervované v `netproto::op`): `RECV`
  s příznakem `PEEK` (i `RECV_FROM`), `SHUTDOWN_READ`, `SET_OPTION`
  a `GET_OPTION` (nodelay, TTL, keep-alive, linger, broadcast), počet
  bajtů k přečtení (`FIONREAD`) **[navrženo]**.
- **Loopback** (127.0.0.1) ve službě **[navrženo]**.
- **Víc karet**, statická konfigurace, mezipaměť DNS, servery NTP z DHCP
  (volba 42).
