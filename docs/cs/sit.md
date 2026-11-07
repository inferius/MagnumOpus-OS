# Síť

První část milníku 6 („síť“). Hotové jsou tyto části:

- ovladač karty **virtio-net** v userspace,
- služba sítě **`netd`** se stackem TCP/IP (ARP, IPv4, ICMP, UDP, TCP),
  klientem DHCP a resolverem DNS,
- protokol **`netproto`** s přístupem přes capability a omezením cílů,
- klient **NTP**, který přes službu času seřídí hodiny.

TLS a HTTP přijdou později. Rozhraní na ně je připravené (viz
[Příprava na TLS a HTTP](#příprava-na-tls-a-http)).

| Část | Kde | Co dělá |
|---|---|---|
| ovladač | `user/virtio-net` | fronty virtio, DMA přes IOMMU, rámce Ethernetu ↔ kanál |
| služba | `user/netd` | stack TCP/IP (smoltcp), DHCP, DNS, oprávnění, sockety |
| protokol | `libs/netproto` | zprávy, oprávnění, formáty DNS a NTP, klientská knihovna |
| správce zařízení | `user/devmgr` | najde kartu na PCI, spustí ovladač |
| init | `user/init/src/system.rs` | spustí `netd`, drží kořenovou relaci, NTP |
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
<rect class="ok" x="300" y="80" width="200" height="110" rx="6"/><text class="t b" x="400" y="105" text-anchor="middle">netd</text><text class="m" x="400" y="125" text-anchor="middle">oprávnění, sockety = kanály</text><text class="m" x="400" y="143" text-anchor="middle">smoltcp: ARP IPv4 ICMP UDP TCP</text><text class="m" x="400" y="161" text-anchor="middle">DHCP, vlastní resolver DNS</text><text class="m" x="400" y="179" text-anchor="middle">1 vlákno, OBJECT_WAIT_MANY</text>
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
  Na jednom kanálu je vždy nejvýš jeden požadavek. Další zprávu služba
  přečte až po odpovědi, a to funguje jako přirozený protitlak.
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

**Fronty:** 0 = příjem, 1 = odesílání, každá nejvýš 64 bufferů po 2 KiB.
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
| `INFO` (1) | ovladač → netd, první zpráva | MAC (6 B), MTU u16, příznaky u8 (bit 0 = linka připojena) |
| `FRAME` (2) | oba směry | rámec Ethernetu bez FCS (14 až 1514 B) |
| `LINK` (3) | ovladač → netd | příznaky u8 při změně stavu linky |

Rámec, který se nevejde do fronty druhé strany, se zahodí jako na
skutečné síti. Spolehlivost řeší TCP.

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
2. kanál **kořenové relace**, kterou drží `init`.

Služba počká na `INFO` od ovladače, odpoví rodiči `ok` a spustí DHCP.
Pak v jednom vlákně dokola opakuje:

1. `poll` stacku: přijaté rámce, časovače TCP, DHCP,
2. posun rozpracovaných požadavků (spojení, čtení, dotazy DNS, ping),
3. `OBJECT_WAIT_MANY` na kanál ovladače a na kanály klientů, s termínem
   podle nejbližšího časovače. Kanál s rozpracovaným požadavkem se nečte,
   hlídá se jen jeho zavření.

| Vlastnost | Hodnota |
|---|---|
| buffery TCP | 64 KiB příjem + 64 KiB odesílání na spojení |
| keep-alive TCP | po 60 s nečinnosti, spojení bez odezvy zrušeno po 120 s |
| navázání spojení bez limitu | vzdá se po 75 s (`TIMED_OUT`) |
| zavření spojení | FIN, úklid po doběhnutí nebo nejpozději po 30 s |
| fronta naslouchání | 1 až 8 spojení |
| dočasné porty | 49152 až 65535, začátek náhodný |
| datagram UDP | nejvýš 8 KiB; fronta 32 datagramů / 64 KiB |
| DNS | dotaz A (a AAAA), opakování po 1 s se střídáním serverů, nejvýš 30 s |

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

⚠️ **Omezení počtu socketů:** služba čeká jedním `OBJECT_WAIT_MANY`, který
bere nejvýš 64 objektů. Relací a socketů tedy může být najednou nejvýš
63. Další dostanou `NO_RESOURCES`. Na první programy to stačí, odstraní
to objekt jádra pro čekání na libovolně mnoho kanálů (port).

⚠️ **Náhodnost:** čísla TCP, dočasné porty a ID dotazů DNS pocházejí
z generátoru se semenem z RDRAND (když ho procesor má), času a MAC.
Kryptografický generátor to **není**. Bez RDRAND jsou hodnoty odhadnutelné
a útočník na stejné síti by mohl podvrhnout odpověď DNS. Služba to
zaloguje. Patří sem zdroj entropie od jádra (TODO).

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
přes `SET_DNS`). Služba si překlad zapamatuje (nejvýš 256 záznamů na
relaci). Odpověď od serveru, který si program zvolil sám (`resolve_with`
se serverem), se nepočítá. Jinak by si program mohl povolené jméno
„přeložit“ na libovolnou adresu.

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

| Metoda | Právo | Chyby navíc |
|---|---|---|
| `Net::info`, `wait_configured` | `INFO` | `TIMED_OUT` |
| `Net::tcp_connect`, `tcp_connect_host` | `TCP_CONNECT` + cíl | `CONNECTION_REFUSED`, `UNREACHABLE`, `TIMED_OUT` |
| `Net::tcp_listen` | `TCP_LISTEN` + `bind` | `ADDRESS_IN_USE` |
| `Net::udp_bind` | `UDP` + `bind` | `ADDRESS_IN_USE` |
| `Net::resolve`, `resolve_with` | `RESOLVE` (+ cíl UDP u vlastního serveru) | `NOT_FOUND`, `TIMED_OUT`, `UNREACHABLE` |
| `Net::ping` | `PING` + cíl ICMP | `TIMED_OUT`, `UNREACHABLE` |
| `Net::restrict` | `RESTRICT` | `INVALID_ARGS` (neplatná pravidla) |
| `Net::set_dns` | `CONFIGURE` | |
| `Net::ntp_query` | `UDP` + cíl | `TIMED_OUT`, `BAD_STATE` (server nesynchronizovaný) |
| `TcpStream::read`, `write` | | `CONNECTION_RESET`, `TIMED_OUT` |
| `UdpSocket::send_to` | cíl | `OUT_OF_RANGE` (nad 8 KiB) |

Chybějící právo nebo cíl mimo omezení vrací `ACCESS_DENIED`. Nové kódy
chyb popisuje [chyby.md](chyby.md).

`TcpStream::read` vrací `Ok(0)` na konci proudu (druhá strana ukončila
zápis). `CONNECTION_RESET` znamená, že spojení přerušil RST nebo že druhá
strana přestala odpovídat. Spojení lze předat jinému procesu:
`into_channel()` u odesílatele, `TcpStream::from_channel()` u příjemce.

## Protokol netproto

Požadavek: `op` u32 a parametry. Odpověď: `status` i32 (0 nebo kód
chyby), rezerva u32 a výsledek. Nový socket nebo relace jde v handlech
odpovědi. Kódování: čísla little-endian, adresa IP = rodina u8 (4/6)
+ 4/16 B, `socket_addr` = adresa + port u16, řetězec = délka u16 + UTF-8,
data = zbytek zprávy. Timeout je u64 v ms: 0 = nečekat,
`u64::MAX` = bez limitu.

| Kanál | Op | Parametry → výsledek |
|---|---|---|
| relace | `INFO` (1) | → Info |
| relace | `WAIT_CONFIGURED` (2) | timeout → Info |
| relace | `TCP_CONNECT` (3) | cíl, timeout → místní adresa + kanál spojení |
| relace | `TCP_LISTEN` (4) | port u16, fronta u16 → místní adresa + kanál naslouchání |
| relace | `UDP_BIND` (5) | port u16 → místní adresa + kanál socketu |
| relace | `RESOLVE` (6) | jméno, rodina u8 (0/4/6), má server u8 [server], timeout → počet u16, adresy, TTL u32 |
| relace | `PING` (7) | adresa, pořadí u16, délka u16, timeout → odezva ns u64 |
| relace | `RESTRICT` (8) | práva u32, Policy → kanál relace |
| relace | `SET_DNS` (9) | počet u8, servery → () |
| spojení | `SEND` (20) | timeout, data → přijato u32 |
| spojení | `RECV` (21) | nejvýš u32, timeout → data (prázdná = konec proudu) |
| spojení | `SHUTDOWN` (22) | → () (FIN, čtení pokračuje) |
| spojení | `ADDRS` (23) | → místní, vzdálená adresa |
| naslouchání | `ACCEPT` (30) | timeout → vzdálená adresa + kanál spojení |
| naslouchání, UDP | `LOCAL_ADDR` (31) | → místní adresa |
| UDP | `SEND_TO` (40) | cíl, data → () |
| UDP | `RECV_FROM` (41) | nejvýš u32, timeout → odesílatel, data |

**Info:** MAC (6 B), MTU u16, příznaky u8 (bit 0 linka, bit 1 nastaveno),
má adresu u8 [adresa, prefix u8], má bránu u8 [brána], počet DNS u8
[adresy], práva relace u32.

**Policy:** má connect u8 [počet u32, pravidla], má bind u8 [počet u32,
(od u16, do u16)...]. Pravidlo = protokoly u8, druh cíle u8
(0 libovolný, 1 síť: adresa + prefix u8, 2 jméno: řetězec), port od u16,
port do u16. Služba odmítne (`INVALID_ARGS`) prázdné protokoly, prefix
delší než adresa, neplatné jméno a víc než 256 pravidel nebo 16 vrstev.

## DNS

Resolver je ve službě (`user/netd/src/resolve.rs`), formát zpráv
v `netproto::dns`, kde ho používá i testovací server.

- Pro rodinu 0 se posílají dotazy A i AAAA. Stačí, když odpoví jeden.
  Prázdná odpověď na AAAA nevadí.
- Odpověď se páruje podle náhodného ID dotazu a adresy serveru.
- Komprese jmen se rozbaluje s ochranou proti smyčce (nejvýš 32 skoků).
- NXDOMAIN = `NOT_FOUND`, jiná chyba serveru = `IO_ERROR`.
- Adresa místo jména (`"10.0.2.2"`) se vrátí hned bez dotazu.

TODO: mezipaměť podle TTL, dotaz přes TCP u zkrácené odpovědi, DoT/DoH
(až s TLS).

## NTP a hodiny

`Net::ntp_query` pošle dotaz NTP v4 (`netproto::ntp`) a vrátí opravený
čas, posun hodin a zpoždění cesty. Odpověď se páruje podle času odeslání
(pole „originate“). Server bez synchronizace (stratum 0, leap 3) vrátí
`BAD_STATE`.

Hodiny smí nastavit **jen služba času** `timed` (právo `SET_CLOCK`, viz
[cas.md](cas.md)). Proto `init` čas jen zjistí a pošle ho službě času
zprávou `set`:

<figure class="diagram">
<svg viewBox="0 0 760 170" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Serizeni hodin podle NTP">
<defs><marker id="sa3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="50" width="150" height="60" rx="6"/><text class="t b" x="85" y="75" text-anchor="middle">server NTP</text><text class="m" x="85" y="93" text-anchor="middle">UDP 123</text>
<path class="ln" d="M238 80 L162 80" marker-end="url(#sa3)"/>
<text class="m" x="170" y="70">t1 → t2, t3 → t4</text>
<rect class="box" x="240" y="50" width="120" height="60" rx="6"/><text class="t b" x="300" y="75" text-anchor="middle">netd</text><text class="m" x="300" y="93" text-anchor="middle">socket UDP</text>
<path class="ln" d="M438 80 L362 80" marker-end="url(#sa3)"/>
<rect class="acc" x="440" y="50" width="130" height="60" rx="6"/><text class="t b" x="505" y="75" text-anchor="middle">init</text><text class="m" x="505" y="93" text-anchor="middle">ntp_query</text>
<path class="ln-acc" d="M570 80 L618 80" marker-end="url(#sa3)"/>
<rect class="ok" x="620" y="50" width="130" height="60" rx="6"/><text class="t b" x="685" y="75" text-anchor="middle">timed</text><text class="m" x="685" y="93" text-anchor="middle">CLOCK_SET</text>
<text class="m" x="10" y="150">posun = ((t2 - t1) + (t3 - t4)) / 2; nový čas = t4 + posun. Do RTC se zatím nezapisuje.</text>
</svg>
<figcaption>Síť čas jen zjistí; nastavit ho smí jen služba času.</figcaption>
</figure>

```rust
// init (user/init/src/system.rs)
let offset_ns = system::sync_time_ntp(&net_root, &time_service, server)?;
```

Zatím se hodiny seřídí jednou a jen v testech proti serveru hostitele.
TODO: server z konfigurace nebo z DHCP (volba 42), periodické seřizování
a plynulé dorovnání místo skoku.

## Příprava na TLS a HTTP

- **Proudy:** `netproto::io::{Read, Write}` jsou obecné rozhraní pro proud
  bajtů a `TcpStream` je implementuje. TLS bude obal
  `TlsStream<S: Read + Write>`, který implementuje totéž. Klient HTTP bude
  pracovat s jakýmkoli `Read + Write`, nad holým TCP i nad TLS stejně.
- **TLS poběží v procesu programu** (knihovna), ne ve službě sítě. Klíče
  relací tak zůstanou v programu, který je potřebuje. Úložiště důvěryhodných
  certifikátů bude systémová služba nebo soubor s právem jen pro čtení.
- **Oprávnění podle jmen:** pravidla `Target::Host` už teď počítají
  s aplikacemi, které smí jen na své API (`*.example.com:443`). TLS
  k tomu přidá ověření, že server jméno opravdu vlastní.
- **Náhodnost:** TLS potřebuje kryptografický generátor. Bez zdroje
  entropie od jádra (TODO výše) TLS nevznikne.

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
| TCP echo | 256 KiB tam a zpět, místní a vzdálená adresa, `shutdown_write` a konec proudu |
| TCP odmítnutí | zavřený port = `CONNECTION_REFUSED` (na Windows `TIMED_OUT`, viz výše), port 0 = `INVALID_ARGS` |
| TCP přerušení | hostitel zavře spojení s nepřečtenými daty (RST): čtení i zápis = `CONNECTION_RESET` |
| TCP naslouchání | hostitel se připojí přes `hostfwd`, `ADDRESS_IN_USE`, vypršení `accept` |
| UDP echo | datagramy tam a zpět, dočasný port, vypršení |
| oprávnění | chybějící práva, cíle mimo omezení, předání spojení jinému kanálu |
| jméno hostitele | pravidlo `*.magnumopus` platí po systémovém překladu, ne po překladu vlastním serverem |
| NTP | dotaz na server hostitele, nastavení přes `timed`, zdroj času `SET` |

DNS přes 10.0.2.3 (slirp předává dotazy DNS hostitele) se netestuje, protože
by test potřeboval internet. Bez pomocných serverů (`cargo xtask run`)
se testy, které je potřebují, přeskočí. V logu je pak
`[preskoceno] sit: ...`.

Formáty a oprávnění testuje i hostitel: `cargo test -p netproto
--features std` (součást `cargo xtask test`).

## Plán

- **Vlastní stack:** nahradí smoltcp za modulem `stack` ve službě. Pořadí:
  ARP + IPv4 + ICMP, UDP, pak TCP (řízení zahlcení, SACK, okna). smoltcp
  (licence 0BSD) poslouží jako vzor a jeho testy jako sada pro porovnání.
  Protokol a programy se nezmění.
- **IPv6:** SLAAC, ICMPv6, AAAA už resolver umí. Protokol je připravený
  (adresy `IpAddr`).
- **Objekt jádra pro čekání** na libovolný počet kanálů: zruší limit 63
  socketů.
- **Zdroj entropie v jádře** (syscall): pro TCP, DNS a hlavně TLS.
- **Sdílená paměť pro data** místo kopírování zprávami (jako u disku).
- **Víc karet**, statická konfigurace, mezipaměť DNS, NTP periodicky.
- **TLS 1.3 a klient HTTP** jako knihovny nad `Read + Write`.
