# TLS a HTTP

Druhá část milníku 6 („síť“). Hotové jsou:

- knihovna **`libs/tls`**: klient TLS 1.3 (rustls) s vlastním
  poskytovatelem kryptografie nad RustCrypto, ověření certifikátu serveru,
  ALPN, obnovení relace,
- **systémové úložiště kořenových certifikátů** `/system/config/certs`
  (sada Mozilla, vlastní CA, seznam nedůvěryhodných CA),
- knihovna **`libs/httpc`**: klient HTTP/1.1 a HTTP/2 (HPACK, řízení
  toku), přesměrování a limity,
- program **`fetch`**: stáhne adresu `http://` nebo `https://`; vše dostává
  jako capability (relace sítě podle manifestu, relace FS jen
  s úložištěm jen pro čtení).

Synchronizaci času (`ntpd`, plynulá korekce hodin) popisuje
[Reálný čas](cas.md#synchronizace-podle-sítě-ntpd). Platnost certifikátu
se ověřuje podle reálného času, proto na ní TLS závisí.

| Část | Kde | Co dělá |
|---|---|---|
| TLS | `libs/tls` | rustls bez `std`, poskytovatel kryptografie (`provider/`), proud `TlsStream`, kořeny (`roots.rs`) |
| HTTP | `libs/httpc` | URL, HTTP/1.1 (`h1.rs`), HTTP/2 (`h2/`), klient s přesměrováním |
| odvolání | `user/revocd`, `libs/tls/src/{revocation,ocsp,x509}.rs` | CRL na vyžádání s mezipamětí, OneCRL, OCSP stapling |
| spojení | `libs/web` | TCP a TLS přes relaci sítě, `httpc::Connector`, kořeny z úložiště |
| program | `user/fetch` | spojení přes `netproto`, TLS, HTTP; příkazy na bootstrap kanálu |
| úložiště | `user/init/src/certs.rs` | připraví `/system/config/certs`, aktualizuje sadu, relace jen pro čtení |
| sada CA | `xtask/src/webhelper.rs` | sada Mozilla z crate `webpki-root-certs` do initfs (`certs.pem`) |
| testy | `libs/tls/src/tests.rs`, `libs/httpc/src/tests/`, `user/init/src/tests/web.rs` | hostitel: proti rustls+ring a hyper; QEMU: `fetch` proti serverům hostitele |

> ⚠ **Rizika (čtěte před použitím):**
> - **Kryptografie RustCrypto není auditovaná jako celek.** Jednotlivé
>   crate (AES-GCM, ChaCha20-Poly1305, P-256) prošly dílčími audity, ale
>   ne všechny. ring (auditovaný, s assemblerem) nejde přeložit pro naše
>   cíle: potřebuje překladač C pro naše cíle a assembler podle platformy
>   (NEON a SSE už programy od FPU-1 mají; převzetí je samostatná práce).
> - **Softwarové AES** (bez AES-NI) je pomalejší a jen „bitsliced“
>   konstantní v čase; na sdíleném CPU je odolnější ChaCha20-Poly1305.
> - **Odvolání jen „měkce“:** když stav odvolání nejde zjistit (CRL nejde
>   stáhnout, server bez OCSP staplingu), spojení projde – jako
>   v prohlížečích. Útočník, který stažení CRL zablokuje, tak odvolaný
>   certifikát protlačí. Viz [Odvolání certifikátů](#odvolání-certifikátů).
> - **Platnost certifikátu závisí na hodinách.** Čas se ověřuje přes NTS
>   (viz [Reálný čas](cas.md#nts-ověřený-čas)); neověřené NTP se podle
>   profilu sítě použije jen jako záloha. V záloze může útočník čas
>   posunout (`timed` to omezí na 500 ppm a odmítne skoky nad 60 s).
> - **DNS:** systémový překlad je šifrovaný (DoH/DoT) a ověřený (DNSSEC,
>   viz [Síť](sit.md#šifrované-dns-a-dnssec-dnsd)). Nepodepsané zóny
>   (většina domén) chrání jen šifrování k resolveru, ne podpis; resolver
>   tak vidí, kam se program připojuje.

## Architektura

<figure class="diagram">
<svg viewBox="0 0 760 290" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Architektura TLS a HTTP">
<defs><marker id="th1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="20" width="330" height="250" rx="8"/>
<text class="t b" x="175" y="44" text-anchor="middle">proces programu (fetch)</text>
<rect class="box" x="30" y="60" width="290" height="40" rx="6"/><text class="t" x="175" y="85" text-anchor="middle">httpc: HTTP/1.1, HTTP/2, přesměrování</text>
<rect class="box" x="30" y="115" width="290" height="40" rx="6"/><text class="t" x="175" y="140" text-anchor="middle">tls: TLS 1.3, ověření, ALPN</text>
<rect class="box" x="30" y="170" width="140" height="40" rx="6"/><text class="t" x="100" y="195" text-anchor="middle">RustCrypto</text>
<rect class="box" x="180" y="170" width="140" height="40" rx="6"/><text class="t" x="250" y="195" text-anchor="middle">TcpStream</text>
<rect class="box" x="30" y="222" width="290" height="36" rx="6"/><text class="m" x="175" y="245" text-anchor="middle">kořeny CA z /certs (jen čtení)</text>
<path class="ln" d="M175 100 L175 113" marker-end="url(#th1)"/>
<path class="ln" d="M120 155 L105 168" marker-end="url(#th1)"/>
<path class="ln" d="M230 155 L245 168" marker-end="url(#th1)"/>
<rect class="box" x="400" y="150" width="150" height="60" rx="6"/><text class="t b" x="475" y="175" text-anchor="middle">netd + fwd</text><text class="m" x="475" y="194" text-anchor="middle">relace, firewall</text>
<rect class="box" x="400" y="222" width="150" height="48" rx="6"/><text class="t b" x="475" y="243" text-anchor="middle">fsd</text><text class="m" x="475" y="260" text-anchor="middle">/system/config/certs</text>
<rect class="ok" x="600" y="150" width="150" height="60" rx="6"/><text class="t b" x="675" y="175" text-anchor="middle">server</text><text class="m" x="675" y="194" text-anchor="middle">TCP 443</text>
<path class="ln" d="M320 190 L398 190" marker-end="url(#th1)"/>
<path class="ln" d="M320 240 L398 246" marker-end="url(#th1)"/>
<path class="ln-acc" d="M550 180 L598 180" marker-end="url(#th1)"/>
<text class="m" x="400" y="60">šifrování, klíče a ověření</text>
<text class="m" x="400" y="78">jen v procesu programu;</text>
<text class="m" x="400" y="96">netd vidí jen šifrovaný proud</text>
<text class="m" x="400" y="114">a hlídá cíle (manifest, firewall)</text>
</svg>
<figcaption>TLS a HTTP jsou knihovny v programu; služby dávají jen capability (síť, soubory).</figcaption>
</figure>

Proč knihovna v programu, ne služba TLS: klíče relací a otevřená data
vidí jen program, kterému patří (chyba v jednom programu neprozradí data
jiného), služba sítě nemusí rozumět TLS a firewall dál pracuje se jmény
a porty. Cenou je velikost programu (kryptografie ~1 MB kódu) a to, že
opravu chyby v TLS dostane program až s novým sestavením.

## Knihovna `tls`

### Co umí

| Vlastnost | Hodnota |
|---|---|
| Protokol | jen TLS 1.3 (RFC 8446); TLS 1.2 a starší se nenabízí |
| Šifrovací sady | `TLS13_AES_128_GCM_SHA256`, `TLS13_AES_256_GCM_SHA384`, `TLS13_CHACHA20_POLY1305_SHA256` |
| Výměna klíčů | X25519 (preferovaná), secp256r1, secp384r1; jiné skupiny = HelloRetryRequest |
| Podpis serveru | ECDSA P-256/P-384 (SHA-256/384), Ed25519, RSA PSS a PKCS#1 v1.5 (SHA-256/384/512), RSA od 2048 bitů |
| Ověření | řetěz do kořenové CA z úložiště, jméno nebo adresa IP serveru (SAN), platnost podle reálného času |
| ALPN | seznam nabízených protokolů; `alpn_protocol()` vrátí vybraný |
| Obnovení relace | lístky TLS 1.3 v paměti programu (PSK); bez 0-RTT |
| Klientský certifikát | ne |

Protokol obstarává **rustls** (0.23, bez `std`, rozhraní
`UnbufferedClientConnection`). Kryptografii dodává vlastní poskytovatel
`tls::provider` nad crate RustCrypto: čistý Rust, bez assembleru a SIMD.

### Poskytovatel kryptografie

| Soubor | Obsah |
|---|---|
| `provider/mod.rs` | `Platform` (náhoda a čas od systému), šifrovací sady, `provider()` |
| `provider/hash.rs` | SHA-256/384 a HMAC (pro HKDF a transkript) |
| `provider/aead.rs` | AES-GCM a ChaCha20-Poly1305 pro záznamy TLS 1.3 (nonce = IV ⊕ číslo záznamu, AAD = hlavička) |
| `provider/kx.rs` | ECDHE: X25519 (odmítne nulové sdílené tajemství), P-256/P-384 (jen nekomprimovaný bod, kontrola, že leží na křivce) |
| `provider/verify.rs` | ověření podpisů pro webpki a mapování schémat TLS |

`xtask` překládá RustCrypto zatím jen softwarově (programy mají od FPU-1
SSE a NEON, ale detekce AES-NI/PCLMUL za běhu a AES na aarch64 přijdou
zvlášť s měřením; x86-64-v2 AES-NI nezaručuje) (`--cfg aes_force_soft`, `polyval_force_soft`,
`poly1305_force_soft`, `chacha20_force_soft`,
`curve25519_dalek_backend="serial"`, u `sha2` feature `force-soft`). Ladicí build
kryptografii optimalizuje (`opt-level = 3` v `Cargo.toml`), jinak by
handshake v QEMU trval desítky sekund.

### Náhoda a čas

```rust
pub struct Platform {
    pub random: fn(&mut [u8]) -> bool, // false = nahoda neni -> spojeni se nenavaze
    pub now: fn() -> Option<u64>,      // sekundy od 1970 UTC; None = cas neznamy
}
pub static LIBOS: Platform; // feature "libos": fill_seeded a realny cas jadra
```

`LIBOS.random` volá `libos::random::fill_seeded`: jádro vrátí náhodu jen
z nasazeného generátoru (`REQUIRE_SEEDED`). Bez náhody se nevygeneruje
žádný klíč (`FailedToGetRandomBytes`); slabý klíč by byl horší než
chyba. Bez známého času nejde ověřit platnost certifikátu a spojení
selže.

### Použití

```rust
let roots = tls::roots::Roots::from_files(files.iter().map(|(n, d)| (n.as_str(), &d[..])));
let client = tls::Client::new(&tls::LIBOS, &roots, &[b"h2", b"http/1.1"])?;
let tcp = net.tcp_connect_host("example.com", 443, Some(timeout))?;
let mut s = client.connect("example.com", tcp)?;   // handshake + overeni
assert_eq!(s.alpn_protocol(), Some(&b"h2"[..]));
s.write_all(b"...")?;                              // netproto::io::Write
let n = s.read(&mut buf)?;                         // 0 = close_notify serveru
s.close()?;                                        // close_notify klienta
```

`Client` je jeden na program (konfigurace, kořeny, mezipaměť relací);
`connect` vytvoří nové spojení. `TlsStream<S>` funguje nad jakýmkoli
`netproto::io::Read + Write`.

### Chyby

| `tls::Error` | Kdy | Kód (`code()`) |
|---|---|---|
| `Certificate(popis)` | řetěz nevede k důvěryhodné CA (`UnknownIssuer`), jméno nesedí (`NotValidForName`), propadlý (`Expired`), špatný podpis (`BadSignature`) | `ACCESS_DENIED` |
| `Protocol(popis)` | chyba TLS (alert serveru, poškozený záznam, nepodporovaná skupina) | `CORRUPTED` |
| `Truncated` | TCP skončilo bez `close_notify`: útočník mohl odpověď zkrátit | `CONNECTION_RESET` |
| `Io(kód)` | chyba TCP pod TLS | kód TCP |

Po chybě je spojení nepoužitelné a další volání vrátí stejnou chybu.

### Známé omezení

Když se po obnovení relace změní šifrovací sada serveru na sadu s jiným
hashem a zároveň server vynutí jinou skupinu (HelloRetryRequest), rustls
klient ve druhém ClientHello lístek vynechá a server rustls 0.23.46
spojení odmítne (`MissingExtension`). Nastane jen při změně konfigurace
serveru mezi spojeními; nové spojení (nový `Client`) projde.

## Úložiště kořenových certifikátů

<figure class="diagram">
<svg viewBox="0 0 760 220" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Uloziste korenovych certifikatu">
<defs><marker id="th2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="40" width="150" height="70" rx="6"/><text class="t b" x="85" y="65" text-anchor="middle">initfs</text><text class="m" x="85" y="84" text-anchor="middle">certs.pem</text><text class="m" x="85" y="100" text-anchor="middle">(net-test-ca.pem)</text>
<path class="ln" d="M160 75 L218 75" marker-end="url(#th2)"/>
<text class="m" x="166" y="66">init</text>
<rect class="acc" x="220" y="20" width="300" height="130" rx="6"/><text class="t b" x="370" y="44" text-anchor="middle">/system/config/certs</text>
<text class="m" x="240" y="68">mozilla.pem   spravuje systém</text>
<text class="m" x="240" y="88">firma.pem     vlastní CA (správce)</text>
<text class="m" x="240" y="108">distrust.txt  vyřazené CA (SHA-256)</text>
<text class="m" x="240" y="128">vlastník system, EVERYONE: jen čtení</text>
<path class="ln-acc" d="M520 85 L578 85" marker-end="url(#th2)"/>
<rect class="ok" x="580" y="40" width="170" height="90" rx="6"/><text class="t b" x="665" y="65" text-anchor="middle">program</text><text class="m" x="665" y="85" text-anchor="middle">relace FS: jen /certs</text><text class="m" x="665" y="103" text-anchor="middle">READ_ONLY</text><text class="m" x="665" y="121" text-anchor="middle">Roots::from_files</text>
<text class="m" x="10" y="185">Při každém startu init porovná mozilla.pem s initfs a při změně ho atomicky přepíše (save_if).</text>
<text class="m" x="10" y="205">Ostatní soubory nechá být: vlastní CA a distrust.txt přežijí aktualizaci systému.</text>
</svg>
<figcaption>Sada CA se aktualizuje se systémem; místní úpravy jsou v oddělených souborech.</figcaption>
</figure>

| Soubor | Kdo ho píše | Obsah |
|---|---|---|
| `mozilla.pem` | init (z initfs `certs.pem`) | kořenové CA Mozilla (CCADB) z crate `webpki-root-certs`; verzi určuje `Cargo.lock` |
| `*.pem`, `*.crt` | správce | jeden nebo víc bloků `CERTIFICATE`; jiné bloky (klíče) se přeskočí |
| `*.der`, `*.cer` | správce | jeden certifikát DER |
| `distrust.txt` | správce | otisk SHA-256 certifikátu (hex, s dvojtečkami nebo bez) na řádek, `#` = komentář |
| `test-ca.pem` | init v testovacím běhu | CA testovacích serverů hostitele |

Ostatní soubory se ignorují. Poškozený blok nebo certifikát, který
webpki nepřijme jako kořen, se započítá do `Roots::rejected` a ostatní
se načtou dál. CA z `distrust.txt` se vyřadí, i když je v `mozilla.pem`
(`Roots::distrusted`).

**Práva:** adresář patří systému. Init přidá právo průchodu pro
`EVERYONE` na `/`, `/system`, `/system/config` a pro adresář úložiště
`VIEW` (výpis, průchod, čtení) s dědičností. Program dostane relaci FS
účtu `NOBODY` (65534) s jediným připojením `/certs` →
`/system/config/certs` s příznakem `READ_ONLY` (`certs::session`):
nevidí nic jiného a nic nezapíše.

```rust
// init: relace pro program (user/init/src/certs.rs)
let fs = certs::session(devices)?;   // jen /certs, jen pro cteni
spawn(.., vec![net_session, fs.into_channel()]);
```

Hláška `init: korenove certifikaty aktualizovany (N kB)` znamená, že se
sada v úložišti změnila (první start nebo nová verze systému). Bez FS
(`init: uloziste certifikatu nelze pripravit: ...`) programy žádné CA
nemají a každé spojení `https` skončí chybou `UnknownIssuer`.

## Odvolání certifikátů

CA může certifikát zneplatnit před koncem platnosti (ukradený klíč,
chybně vydaný). Klient se to dozví jen z dalších dat; TLS kontroluje
tři zdroje:

<figure class="diagram">
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Kontrola odvolani">
<defs><marker id="th4" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="20" width="250" height="210" rx="6"/><text class="t b" x="135" y="44" text-anchor="middle">program (tls)</text>
<text class="m" x="25" y="70">1. webpki: řetěz, jméno, čas</text>
<text class="m" x="25" y="95">2. OCSP z handshaku</text>
<text class="m" x="25" y="120">3. OneCRL: odvolané CA</text>
<text class="m" x="25" y="145">4. CRL z certifikátu</text>
<text class="m" x="25" y="170">   (webpki ověří podpis CRL)</text>
<text class="m" x="25" y="205">neznámý stav = projde</text>
<path class="ln-acc" d="M260 120 L338 120" marker-end="url(#th4)"/><text class="m" x="268" y="112">kanál</text>
<rect class="ok" x="340" y="60" width="180" height="120" rx="6"/><text class="t b" x="430" y="85" text-anchor="middle">revocd</text><text class="m" x="430" y="108" text-anchor="middle">mezipaměť CRL</text><text class="m" x="430" y="126" text-anchor="middle">OneCRL (24 h)</text><text class="m" x="430" y="144" text-anchor="middle">neúspěch: 10 min pauza</text>
<path class="ln" d="M520 100 L598 80" marker-end="url(#th4)"/>
<path class="ln" d="M520 140 L598 160" marker-end="url(#th4)"/>
<rect class="box" x="600" y="45" width="150" height="60" rx="6"/><text class="t b" x="675" y="70" text-anchor="middle">CA (http)</text><text class="m" x="675" y="88" text-anchor="middle">CRL</text>
<rect class="box" x="600" y="135" width="150" height="60" rx="6"/><text class="t b" x="675" y="160" text-anchor="middle">Mozilla (https)</text><text class="m" x="675" y="178" text-anchor="middle">OneCRL</text>
<path class="ln" d="M430 180 L430 213" marker-end="url(#th4)"/><text class="m" x="440" y="232">/system/config/certs/crl</text>
</svg>
<figcaption>Program se ptá revocd; podpisy CRL a OCSP ověřuje sám.</figcaption>
</figure>

| Zdroj | Co pokryje | Kdy se odmítne |
|---|---|---|
| **OCSP stapling** (`tls::ocsp`) | koncový certifikát, když server odpověď OCSP pošle v handshaku | stav `revoked`; špatný podpis = `InvalidOcspResponse` (podvržená odpověď). Propadlá odpověď = stav neznámý |
| **OneCRL** (Mozilla) | odvolané mezilehlé CA (stovky záznamů, jako Firefox) | certifikát řetězu na seznamu (vystavitel + číslo, nebo subjekt + SHA-256 klíče) |
| **CRL** | certifikáty s distribučním bodem `http://...` (téměř všechny veřejné CA) | číslo certifikátu v CRL podepsané jeho vystavitelem; propadlá CRL se nepoužije |
| **CRL „jen CA“** (`tls::crl`) | CRL kořenů s rozsahem jen pro certifikáty CA (Issuing Distribution Point s `onlyContainsCACerts`, např. ISRG Root X1 a Root YR od Let's Encrypt) – webpki je neumí | číslo mezilehlé CA v CRL; podpis vydavatele CRL ověří `tls` sám |

**Neznámý stav spojení nezastaví** (revocd neběží, CRL nejde stáhnout,
server bez staplingu). Stejně se chovají prohlížeče: tvrdé odmítnutí by
útočník, který blokuje CRL, změnil v nefunkční síť. Odvolaný certifikát
(`Error::Certificate("Revoked")`, kód `ACCESS_DENIED`) se odmítne vždy.

**Každá CRL zvlášť:** CRL, kterou webpki přečte, ověří webpki; CRL „jen
CA“ ověří `tls::crl`. CRL, kterou nepřečte nikdo (delta CRL, nepřímá CRL,
`onlySomeReasons`, neznámé kritické rozšíření), nebo se špatným podpisem
se přeskočí s hláškou `tls: CRL <url> nejde pouzit (...)` (jednou pro
adresu) a stav z ní je neznámý. Dřív jedna taková CRL shodila celé
spojení chybou `unexpected error: provided CRL could not be parsed` – tak
selhávalo NTS-KE s každým serverem s certifikátem Let's Encrypt.

### Služba `revocd`

Programy se ptají kanálem, který jim předá init
(`revocation::client()`); revocd vidí jen dotazy, ne data spojení.

| Zpráva | Odpověď |
|---|---|
| `crl <url>` | `ok` + CRL (DER) nebo `none` |
| `ca` + 4 pole s délkou u32 LE (vystavitel, číslo, subjekt, SHA-256 klíče) | `revoked` nebo `ok` |
| bootstrap: `client` + handle | `ok` (nový program) |
| bootstrap: `status` | `ok:clients=… crls=… fetched=… onecrl=…` |
| bootstrap: `refresh` | `ok:onecrl=…` (OneCRL hned znovu) |

- **CRL:** z paměti (64 posledních), z FS (`/crl/<SHA-256 adresy>.crl`,
  platí do `nextUpdate`, CRL bez něj 24 h od stažení), jinak stažení
  (HTTP/HTTPS, nejvýš 16 MiB, limit 15 s). Po neúspěchu se adresa
  10 minut nezkouší. Podpis neověřuje revocd, ale webpki v programu:
  podvržená CRL od revocd nic nezpůsobí.
- **OneCRL:** stahuje se po prvním dotazu `ca` a pak každých 24 h
  z adresy v `revocd.toml`; poslední platná verze leží v
  `/crl/onecrl.json`. Stahuje se až v klidu (0,5 s bez dotazu): dotaz `ca`
  odpoví hned z posledního seznamu (při úplně prvním startu prázdného =
  stav neznámý), takže handshake programu (NTS-KE `ntpd` při startu) na
  megabajty OneCRL nečeká.
  Záznamy s `"enabled": false` se přeskočí.
- **Práva:** relace sítě TCP 80/443 kamkoli (adresy CRL určují CA),
  relace FS účtu `REVOCD_USER` (901): `/certs` jen pro čtení, `/crl`
  pro zápis. Mezipaměť dědí čtení pro všechny (CRL jsou veřejné).

```toml
# revocd.toml
onecrl = "https://firefox.settings.services.mozilla.com/v1/buckets/security-state/collections/onecrl/changeset?_expected=0"
onecrl_refresh_hours = 24
crl_default_hours = 24
retry_minutes = 10
```

```rust
// program: kontrola odvolani pres revocd (kanal od init)
let source = Arc::new(tls::revocation::ChannelSource::new(channel, Duration::from_secs(20)));
let client = tls::Client::with_revocation(&tls::LIBOS, &roots, httpc::ALPN, Some(source))?;
```

> **Změna proti první verzi:** odvolání se nekontrolovalo vůbec.
> **Proč:** certifikát s ukradeným klíčem by jinak platil až do konce
> platnosti (u veřejných CA měsíce).

## Knihovna `httpc`

### HTTP/1.1

- Požadavek: `Host`, `User-Agent: MagnumOpus-httpc/0.1`,
  `Accept-Encoding: identity` (bez komprese), hlavičky programu,
  `Content-Length` (u POST/PUT/PATCH vždy), `Connection: close`.
- Odpověď: stavový řádek `HTTP/1.0` i `1.1`, mezistavy 1xx se přeskočí,
  `101` je chyba.
- Tělo podle RFC 9112 6.3: HEAD, 204 a 304 bez těla; `Transfer-Encoding`
  má přednost (`chunked` jako poslední kódování, jinak do konce spojení);
  jinak `Content-Length`; jinak do konce spojení. Trailery se přečtou
  a zahodí.
- **Přísné čtení** (ochrana proti „request smuggling“): pokračovací
  řádky (obs-fold), mezera před dvojtečkou, neplatné jméno hlavičky
  a různé hodnoty `Content-Length` jsou chyba.

### HTTP/2

<figure class="diagram">
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Pozadavek HTTP/2">
<defs><marker id="th3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="10" width="140" height="230" rx="6"/><text class="t b" x="80" y="35" text-anchor="middle">klient</text>
<rect class="ok" x="610" y="10" width="140" height="230" rx="6"/><text class="t b" x="680" y="35" text-anchor="middle">server</text>
<path class="ln" d="M150 60 L608 60" marker-end="url(#th3)"/><text class="m" x="170" y="54">preface, SETTINGS (push 0, okno 1 MiB), WINDOW_UPDATE</text>
<path class="ln" d="M150 90 L608 90" marker-end="url(#th3)"/><text class="m" x="170" y="84">HEADERS proud 1 (:method :scheme :authority :path) [+ DATA]</text>
<path class="ln" d="M608 120 L152 120" marker-end="url(#th3)"/><text class="m" x="170" y="114">SETTINGS, SETTINGS ACK</text>
<path class="ln" d="M150 145 L608 145" marker-end="url(#th3)"/><text class="m" x="170" y="139">SETTINGS ACK</text>
<path class="ln" d="M608 175 L152 175" marker-end="url(#th3)"/><text class="m" x="170" y="169">HEADERS (:status) [+ CONTINUATION], DATA ... END_STREAM</text>
<path class="ln" d="M150 200 L608 200" marker-end="url(#th3)"/><text class="m" x="170" y="194">WINDOW_UPDATE za každý DATA (spojení i proud)</text>
<path class="ln" d="M150 225 L608 225" marker-end="url(#th3)"/><text class="m" x="170" y="219">GOAWAY (NO_ERROR nebo kód chyby)</text>
</svg>
<figcaption>Jeden požadavek na spojení (proud 1); TLS s ALPN „h2“.</figcaption>
</figure>

| Oblast | Chování |
|---|---|
| Nastavení klienta | `ENABLE_PUSH = 0`, `INITIAL_WINDOW_SIZE = 1 MiB`, `MAX_HEADER_LIST_SIZE` = limit hlaviček, `HEADER_TABLE_SIZE = 4096`, `MAX_CONCURRENT_STREAMS = 0` |
| Rámce | nejvýš 16 KiB (nezvětšujeme); větší = `FRAME_SIZE_ERROR`; neznámé typy se ignorují |
| Řízení toku | tělo požadavku jen do okna serveru (spojení i proud, i záporné okno po `SETTINGS`); přijatá data se hned potvrdí |
| HPACK | dekodér: všechny reprezentace, dynamická tabulka, Huffman (RFC 7541); kodér: literály bez indexování |
| Kontroly odpovědi | `:status` první a jediná pseudo-hlavička, malá písmena, bez hlaviček spojení, `content-length` = délka dat, trailery jen s `END_STREAM` |
| Server push | vypnutý; `PUSH_PROMISE` = `PROTOCOL_ERROR` |
| Konec | `RST_STREAM` = `Error::Reset(kód)`; `GOAWAY` před naším proudem = `Reset(REFUSED_STREAM)` (lze zopakovat) |
| HEAD | data od serveru se zahodí (hyper je posílá) |

### Limity a přesměrování

```rust
pub struct Limits {
    pub max_header_bytes: usize, // 64 KiB: radky HTTP/1.1, dekodovany seznam HTTP/2
    pub max_headers: usize,      // 128
    pub max_body: usize,         // 16 MiB
    pub max_redirects: u8,       // 5
}
```

Limit hlaviček u HTTP/2 hlídá i zakódovaný blok (nekonečné
`CONTINUATION`) a dekódovanou velikost (krátký odkaz na velkou položku
tabulky opakovaný mnohokrát, „HPACK bomb“).

`Client::request` sleduje přesměrování 301, 302, 303, 307 a 308:

- 303 (a 301/302 u jiné metody než GET/HEAD) = další požadavek GET bez
  těla; 307/308 = stejná metoda i tělo,
- `Location` může být absolutní, `//server`, `/cesta`, relativní cesta
  (s `.` a `..`) nebo `?dotaz`,
- **`https` → `http` se odmítne** (`Error::InsecureRedirect`, kód
  `ACCESS_DENIED`),
- na jiný server (schéma, jméno, port) se neposílají `Authorization`,
  `Cookie` ani `Proxy-Authorization`.

### Adresy

`Url::parse` přijme `http://` a `https://`, jméno serveru (malými
písmeny), IPv4 nebo `[IPv6]`, port a cestu s dotazem. Odmítne
přihlašovací údaje v adrese (`user:heslo@` – skončily by v historii
a logu), jména s diakritikou (IDN zatím ne), port 0 a řídicí znaky.
Mezera a ne-ASCII bajty v cestě se zakódují (`%20`, `%C4%8D`), fragment
`#...` se zahodí.

### Použití

```rust
struct Web<'a> { net: &'a Net, tls: &'a tls::Client }

impl httpc::Connector for Web<'_> {
    type Stream = Conn; // TcpStream nebo TlsStream<TcpStream>
    fn connect(&mut self, url: &Url) -> httpc::Result<(Conn, Version)> {
        // TCP; pro https TLS s ALPN httpc::ALPN, verze podle ALPN
    }
}

let mut client = httpc::Client::new(Web { net: &net, tls: &tls }, httpc::Limits::default());
let resp = client.get("https://example.com/")?;
println!("{} {} {} B", resp.status, resp.version.as_str(), resp.body.len());
```

Hlavičky programu musí mít jméno token malými písmeny a hodnotu bez CR,
LF a NUL (jinak by šlo vložit další hlavičku). `Host`, `Content-Length`,
`Connection`, `Transfer-Encoding`, `Upgrade`, `TE` a `Keep-Alive`
doplňuje knihovna a od programu je odmítne (`Error::Request`).

### Chyby

| `httpc::Error` | Kdy | Kód |
|---|---|---|
| `Io(kód)` | chyba proudu během požadavku | kód proudu |
| `Transport(kód, popis)` | spojení nejde navázat (TCP, certifikát) | od `Connector` (certifikát = `ACCESS_DENIED`) |
| `Url(popis)` | neplatná adresa | `INVALID_ARGS` |
| `Request(popis)` | neplatná hlavička programu | `INVALID_ARGS` |
| `Protocol(popis)` | server porušil HTTP | `CORRUPTED` |
| `Reset(kód)` | HTTP/2 `RST_STREAM` / `GOAWAY` | `CORRUPTED` |
| `TooLarge(co)` | překročený limit | `OUT_OF_RANGE` |
| `TooManyRedirects` | víc než `max_redirects` | `OUT_OF_RANGE` |
| `InsecureRedirect` | přesměrování z `https` na `http` | `ACCESS_DENIED` |

## Program `fetch`

Manifest `user/fetch/fetch.manifest.toml`:

```toml
[app]
id = "org.magnumopus.fetch"
[limits]
memory = "48M"            # kryptografie, telo (az 16 MiB), buffery
[network]
rights = ["tcp", "dns"]
ask = true                # jiny port nez 80/443 = dotaz uzivateli
[[network.connect]]
proto = ["tcp"]
to = "*"
ports = "80"              # a dalsi pravidlo pro 443
```

Handly při startu: 1. relace sítě, 2. relace FS úložiště (volitelná;
bez ní `fetch` nevěří žádné CA), 3. kanál k `revocd` (volitelný; bez něj
jen OCSP stapling). Příkazy na bootstrap kanálu:

| Příkaz | Odpověď |
|---|---|
| `get <url>`, `head <url>`, `post <url> <tělo>` | `ok:<stav> <verze> <délka těla> <FNV-1a 64 hex> <konečná adresa>` |
| `body <url>` | `ok:<tělo jako text, nejvýš 4 KiB>` |
| `roots` | `ok:<počet CA> rejected=<n> distrusted=<n>` |
| chyba | `err:<JMÉNO_KÓDU> <popis>`, např. `err:ACCESS_DENIED spojeni nelze navazat: neplatny certifikat serveru: UnknownIssuer` |

TCP spojení i čtení mají časový limit 30 s.

## Testy

**Hostitel** (`cargo xtask test` spustí `cargo test -p tls -p httpc`):

| Test | Ověří |
|---|---|
| `tls`: sady × skupiny | všech 9 kombinací proti serveru rustls+ring (HelloRetryRequest pro P-256/P-384) |
| `tls`: typy klíčů | ECDSA P-256, P-384, Ed25519, RSA 2048 (podpis CA i handshaku) |
| `tls`: velký přenos | 300 kB tam, 1 MiB zpět (víc záznamů v obou směrech) |
| `tls`: ALPN, IP adresa | vybraný protokol, server bez ALPN, certifikát na 127.0.0.1 |
| `tls`: odmítnutí | cizí CA, špatné jméno, propadlý certifikát (čas 2100), bez náhody |
| `tls`: zkrácení, obnovení | konec TCP bez `close_notify` = `Truncated`; druhé spojení přes PSK |
| `tls`: kořeny | PEM s komentáři, DER, poškozené bloky, `distrust.txt`, base64 |
| `tls`: odvolání | CRL koncového a mezilehlého certifikátu, propadlá a podvržená CRL, OneCRL, OCSP dobrá / odvolaná / propadlá / podvržená, KeyUpdate |
| `tls`: CRL „jen CA“ | skutečné CRL ISRG Root X1 a Root YR (`libs/tls/testdata/letsencrypt/`, webpki je odmítne, `tls::crl` je přečte a ověří podpis), skutečný řetěz `nts.netnod.se` s nimi projde; odvolaná mezilehlá CA v CRL „jen CA“, podvržená CRL spojení neshodí |
| `httpc`: HPACK | příklady RFC 7541 C.1, C.4 (Huffman, dynamická tabulka), C.5 (vyřazování), Huffman všech bajtů, chybná výplň, „HPACK bomb“ |
| `httpc`: HTTP/1.1 skriptem | chunked s rozšířením a trailery, TE vs. CL, 1xx, 304, data po bajtu, 15 chybných odpovědí, limity |
| `httpc`: HTTP/2 skriptem | PING, výplň, CONTINUATION, 1xx, trailery, 17 chybných odpovědí, řízení toku těla, limit těla |
| `httpc`: proti hyper | HTTP/1.1 i HTTP/2: text, 3 MB proud, POST 200 kB, hlavičky, 60 hlaviček (HPACK s tabulkou), HEAD, přesměrování, smyčka, `https` → `http`, přihlašovací hlavičky |

**QEMU** (sada `TLS a HTTP`, `user/init/src/tests/web.rs`): `fetch` proti
serverům hostitele (`xtask/src/webhelper.rs`: rustls+ring, hyper,
testovací CA vygenerovaná při každém běhu):

| Test | Ověří |
|---|---|
| úložiště | `mozilla.pem` = initfs, `test-ca.pem`, opakovaný seed nepřepíše, relace programu jen čte a nevidí nic mimo `/certs` |
| kořeny | `fetch roots`: víc než 100 CA, nic odmítnutého |
| HTTP/1.1 | `http://10.0.2.2:<port>/hello` |
| HTTPS | HTTP/2 přes ALPN, HTTP/1.1 u serveru bez `h2`, 1 MB tělo oběma, POST |
| odmítnutí | cizí CA (`UnknownIssuer`), špatné jméno (`NotValidForName`), TLS na port bez TLS |
| přesměrování | v rámci `https` ano, na `http` = `ACCESS_DENIED` |
| odvolání | certifikát v CRL (revocd ji stáhne a uloží; neodvolaný se stejnou CRL projde), OCSP „revoked“ ve staplingu, mezilehlá CA v OneCRL (záznam s `enabled: false` se nepočítá) |

## Plán

- **CRLite** (kompaktní filtr všech odvolání) místo stahování CRL
  a aktualizace sady CA mimo vydání systému; revocd souběžně pro víc
  programů (teď dotazy postupně).
- **Spojení znovu** (keep-alive HTTP/1.1, víc proudů HTTP/2) a komprese
  (gzip, brotli).
- **NTS** (RFC 8915) pro ověřený čas; DoH/DoT pro ověřené DNS.
- **Hardwarové AES a GHASH** (fronta, SIMD programy mají od FPU-1):
  zrušit `--cfg aes_force_soft` / `polyval_force_soft` a nechat
  RustCrypto zjistit AES-NI a PCLMULQDQ za běhu (CPUID; x86-64-v2 je
  nezaručuje). Na aarch64 potřebuje detekce ARMv8 Crypto informaci od
  jádra (obdoba HWCAP, `ID_AA64ISAR0_EL1` z EL0 nejde číst). Změřit
  handshake a propustnost a ověřit konstantní čas. Pak případně ring
  nebo vlastní assembler.
- **Klientské certifikáty** a TLS server (pro služby v systému).
