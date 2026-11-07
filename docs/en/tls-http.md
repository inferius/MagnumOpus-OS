# TLS and HTTP

The second part of milestone 6 ("network"). Done:

- the **`libs/tls`** library: a TLS 1.3 client (rustls) with our own
  cryptography provider on top of RustCrypto, server certificate
  verification, ALPN, session resumption,
- the **system root certificate store** `/system/config/certs` (the
  Mozilla set, custom CAs, a list of distrusted CAs),
- the **`libs/httpc`** library: an HTTP/1.1 and HTTP/2 client (HPACK, flow
  control), redirects and limits,
- the **`fetch`** program: downloads an `http://` or `https://` address; it
  gets everything as capabilities (a network session from its manifest,
  an FS session with only the store, read-only).

Time synchronization (`ntpd`, smooth clock correction) is described in
[Real time](time.md#synchronization-over-the-network-ntpd). Certificate
validity is checked against real time, so TLS depends on it.

| Part | Where | What it does |
|---|---|---|
| TLS | `libs/tls` | rustls without `std`, cryptography provider (`provider/`), `TlsStream`, roots (`roots.rs`) |
| HTTP | `libs/httpc` | URL, HTTP/1.1 (`h1.rs`), HTTP/2 (`h2/`), client with redirects |
| revocation | `user/revocd`, `libs/tls/src/{revocation,ocsp,x509}.rs` | CRLs on demand with a cache, OneCRL, OCSP stapling |
| connections | `libs/web` | TCP and TLS over a network session, `httpc::Connector`, roots from the store |
| program | `user/fetch` | connections via `netproto`, TLS, HTTP; commands on the bootstrap channel |
| store | `user/init/src/certs.rs` | prepares `/system/config/certs`, updates the set, read-only sessions |
| CA set | `xtask/src/webhelper.rs` | the Mozilla set from the `webpki-root-certs` crate into initfs (`certs.pem`) |
| tests | `libs/tls/src/tests.rs`, `libs/httpc/src/tests/`, `user/init/src/tests/web.rs` | host: against rustls+ring and hyper; QEMU: `fetch` against host servers |

> ⚠ **Risks (read before use):**
> - **The RustCrypto cryptography is not audited as a whole.** Some crates
>   (AES-GCM, ChaCha20-Poly1305, P-256) had partial audits, not all of
>   them. ring (audited, with assembly) cannot be built for our targets:
>   it needs a C compiler and, on aarch64, NEON, which programs without an
>   FPU do not have.
> - **Software AES** (no AES-NI) is slower and constant-time only through
>   bitslicing; on a shared CPU ChaCha20-Poly1305 is the more robust choice.
> - **Revocation is "soft-fail" only:** when the revocation status cannot
>   be determined (the CRL cannot be downloaded, a server without OCSP
>   stapling), the connection passes - as in browsers. An attacker who
>   blocks the CRL download can push a revoked certificate through. See
>   [Certificate revocation](#certificate-revocation).
> - **Certificate validity depends on the clock.** Time is authenticated
>   with NTS (see [Real time](time.md#nts-authenticated-time));
>   unauthenticated NTP is used only as a fallback depending on the network
>   profile. In fallback an attacker can shift the time (`timed` limits it
>   to 500 ppm and rejects jumps over 60 s).
> - **DNS is not authenticated** (no DNSSEC or DoH): a spoofed address
>   fails certificate verification, but the attacker learns where the
>   program connects.

## Architecture

<figure class="diagram">
<svg viewBox="0 0 760 290" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="TLS and HTTP architecture">
<defs><marker id="th1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="20" width="330" height="250" rx="8"/>
<text class="t b" x="175" y="44" text-anchor="middle">program process (fetch)</text>
<rect class="box" x="30" y="60" width="290" height="40" rx="6"/><text class="t" x="175" y="85" text-anchor="middle">httpc: HTTP/1.1, HTTP/2, redirects</text>
<rect class="box" x="30" y="115" width="290" height="40" rx="6"/><text class="t" x="175" y="140" text-anchor="middle">tls: TLS 1.3, verification, ALPN</text>
<rect class="box" x="30" y="170" width="140" height="40" rx="6"/><text class="t" x="100" y="195" text-anchor="middle">RustCrypto</text>
<rect class="box" x="180" y="170" width="140" height="40" rx="6"/><text class="t" x="250" y="195" text-anchor="middle">TcpStream</text>
<rect class="box" x="30" y="222" width="290" height="36" rx="6"/><text class="m" x="175" y="245" text-anchor="middle">root CAs from /certs (read-only)</text>
<path class="ln" d="M175 100 L175 113" marker-end="url(#th1)"/>
<path class="ln" d="M120 155 L105 168" marker-end="url(#th1)"/>
<path class="ln" d="M230 155 L245 168" marker-end="url(#th1)"/>
<rect class="box" x="400" y="150" width="150" height="60" rx="6"/><text class="t b" x="475" y="175" text-anchor="middle">netd + fwd</text><text class="m" x="475" y="194" text-anchor="middle">session, firewall</text>
<rect class="box" x="400" y="222" width="150" height="48" rx="6"/><text class="t b" x="475" y="243" text-anchor="middle">fsd</text><text class="m" x="475" y="260" text-anchor="middle">/system/config/certs</text>
<rect class="ok" x="600" y="150" width="150" height="60" rx="6"/><text class="t b" x="675" y="175" text-anchor="middle">server</text><text class="m" x="675" y="194" text-anchor="middle">TCP 443</text>
<path class="ln" d="M320 190 L398 190" marker-end="url(#th1)"/>
<path class="ln" d="M320 240 L398 246" marker-end="url(#th1)"/>
<path class="ln-acc" d="M550 180 L598 180" marker-end="url(#th1)"/>
<text class="m" x="400" y="60">encryption, keys and verification</text>
<text class="m" x="400" y="78">only in the program process;</text>
<text class="m" x="400" y="96">netd sees an encrypted stream</text>
<text class="m" x="400" y="114">and checks targets (manifest, firewall)</text>
</svg>
<figcaption>TLS and HTTP are libraries in the program; services only hand out capabilities (network, files).</figcaption>
</figure>

Why a library in the program and not a TLS service: session keys and
plaintext are seen only by the program they belong to (a bug in one
program does not leak another program's data), the network service does
not need to understand TLS, and the firewall keeps working with names and
ports. The price is program size (~1 MB of cryptography code) and that a
TLS fix reaches a program only with a new build.

## The `tls` library

### What it supports

| Feature | Value |
|---|---|
| Protocol | TLS 1.3 only (RFC 8446); TLS 1.2 and older are not offered |
| Cipher suites | `TLS13_AES_128_GCM_SHA256`, `TLS13_AES_256_GCM_SHA384`, `TLS13_CHACHA20_POLY1305_SHA256` |
| Key exchange | X25519 (preferred), secp256r1, secp384r1; other groups = HelloRetryRequest |
| Server signature | ECDSA P-256/P-384 (SHA-256/384), Ed25519, RSA PSS and PKCS#1 v1.5 (SHA-256/384/512), RSA from 2048 bits |
| Verification | chain to a root CA from the store, server name or IP address (SAN), validity against real time |
| ALPN | list of offered protocols; `alpn_protocol()` returns the selected one |
| Session resumption | TLS 1.3 tickets in program memory (PSK); no 0-RTT |
| Client certificate | no |

The protocol is handled by **rustls** (0.23, without `std`, the
`UnbufferedClientConnection` interface). Cryptography comes from our own
provider `tls::provider` on top of the RustCrypto crates: pure Rust, no
assembly and no SIMD.

### Cryptography provider

| File | Contents |
|---|---|
| `provider/mod.rs` | `Platform` (randomness and time from the system), cipher suites, `provider()` |
| `provider/hash.rs` | SHA-256/384 and HMAC (for HKDF and the transcript) |
| `provider/aead.rs` | AES-GCM and ChaCha20-Poly1305 for TLS 1.3 records (nonce = IV ⊕ record number, AAD = header) |
| `provider/kx.rs` | ECDHE: X25519 (rejects an all-zero shared secret), P-256/P-384 (uncompressed point only, on-curve check) |
| `provider/verify.rs` | signature verification for webpki and the TLS scheme mapping |

Programs run without SSE and NEON. `xtask` therefore builds RustCrypto in
software only (`--cfg aes_force_soft`, `polyval_force_soft`,
`poly1305_force_soft`, `chacha20_force_soft`,
`curve25519_dalek_backend="serial"`, the `force-soft` feature of `sha2`):
run-time AES-NI detection would otherwise execute SIMD instructions. The
debug build optimizes cryptography (`opt-level = 3` in `Cargo.toml`);
otherwise a handshake in QEMU would take tens of seconds.

### Randomness and time

```rust
pub struct Platform {
    pub random: fn(&mut [u8]) -> bool, // false = no randomness -> no connection
    pub now: fn() -> Option<u64>,      // seconds since 1970 UTC; None = unknown
}
pub static LIBOS: Platform; // feature "libos": fill_seeded and the kernel's real time
```

`LIBOS.random` calls `libos::random::fill_seeded`: the kernel returns
randomness only from a seeded generator (`REQUIRE_SEEDED`). Without
randomness no key is generated (`FailedToGetRandomBytes`); a weak key
would be worse than an error. Without a known time the certificate
validity cannot be checked and the connection fails.

### Usage

```rust
let roots = tls::roots::Roots::from_files(files.iter().map(|(n, d)| (n.as_str(), &d[..])));
let client = tls::Client::new(&tls::LIBOS, &roots, &[b"h2", b"http/1.1"])?;
let tcp = net.tcp_connect_host("example.com", 443, Some(timeout))?;
let mut s = client.connect("example.com", tcp)?;   // handshake + verification
assert_eq!(s.alpn_protocol(), Some(&b"h2"[..]));
s.write_all(b"...")?;                              // netproto::io::Write
let n = s.read(&mut buf)?;                         // 0 = server close_notify
s.close()?;                                        // client close_notify
```

There is one `Client` per program (configuration, roots, session cache);
`connect` creates a new connection. `TlsStream<S>` works over any
`netproto::io::Read + Write`.

### Errors

| `tls::Error` | When | Code (`code()`) |
|---|---|---|
| `Certificate(text)` | the chain does not lead to a trusted CA (`UnknownIssuer`), the name does not match (`NotValidForName`), expired (`Expired`), bad signature (`BadSignature`) | `ACCESS_DENIED` |
| `Protocol(text)` | TLS error (server alert, damaged record, unsupported group) | `CORRUPTED` |
| `Truncated` | TCP ended without `close_notify`: an attacker could have truncated the response | `CONNECTION_RESET` |
| `Io(code)` | TCP error under TLS | TCP code |

After an error the connection is unusable and further calls return the
same error.

### Known limitation

If after session resumption the server switches to a cipher suite with a
different hash and at the same time forces another group
(HelloRetryRequest), the rustls client drops the ticket from the second
ClientHello and a rustls 0.23.46 server rejects the connection
(`MissingExtension`). It only happens when the server configuration
changes between connections; a new connection (a new `Client`) works.

## Root certificate store

<figure class="diagram">
<svg viewBox="0 0 760 220" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Root certificate store">
<defs><marker id="th2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="40" width="150" height="70" rx="6"/><text class="t b" x="85" y="65" text-anchor="middle">initfs</text><text class="m" x="85" y="84" text-anchor="middle">certs.pem</text><text class="m" x="85" y="100" text-anchor="middle">(net-test-ca.pem)</text>
<path class="ln" d="M160 75 L218 75" marker-end="url(#th2)"/>
<text class="m" x="166" y="66">init</text>
<rect class="acc" x="220" y="20" width="300" height="130" rx="6"/><text class="t b" x="370" y="44" text-anchor="middle">/system/config/certs</text>
<text class="m" x="240" y="68">mozilla.pem   managed by the system</text>
<text class="m" x="240" y="88">company.pem   custom CA (admin)</text>
<text class="m" x="240" y="108">distrust.txt  removed CAs (SHA-256)</text>
<text class="m" x="240" y="128">owner system, EVERYONE: read only</text>
<path class="ln-acc" d="M520 85 L578 85" marker-end="url(#th2)"/>
<rect class="ok" x="580" y="40" width="170" height="90" rx="6"/><text class="t b" x="665" y="65" text-anchor="middle">program</text><text class="m" x="665" y="85" text-anchor="middle">FS session: /certs only</text><text class="m" x="665" y="103" text-anchor="middle">READ_ONLY</text><text class="m" x="665" y="121" text-anchor="middle">Roots::from_files</text>
<text class="m" x="10" y="185">On every start init compares mozilla.pem with initfs and rewrites it atomically if it changed (save_if).</text>
<text class="m" x="10" y="205">Other files are left alone: custom CAs and distrust.txt survive a system update.</text>
</svg>
<figcaption>The CA set is updated with the system; local changes live in separate files.</figcaption>
</figure>

| File | Written by | Contents |
|---|---|---|
| `mozilla.pem` | init (from initfs `certs.pem`) | Mozilla root CAs (CCADB) from the `webpki-root-certs` crate; the version is set by `Cargo.lock` |
| `*.pem`, `*.crt` | administrator | one or more `CERTIFICATE` blocks; other blocks (keys) are skipped |
| `*.der`, `*.cer` | administrator | one DER certificate |
| `distrust.txt` | administrator | a certificate SHA-256 fingerprint (hex, with or without colons) per line, `#` = comment |
| `test-ca.pem` | init in a test run | the CA of the host test servers |

Other files are ignored. A damaged block or a certificate webpki does not
accept as a root is counted in `Roots::rejected` and loading continues.
A CA listed in `distrust.txt` is removed even if it is in `mozilla.pem`
(`Roots::distrusted`).

**Permissions:** the directory belongs to the system. Init grants
`EVERYONE` traverse on `/`, `/system`, `/system/config` and `VIEW` (list,
traverse, read) with inheritance on the store directory. A program gets an
FS session of the `NOBODY` account (65534) with a single mount `/certs` →
`/system/config/certs` with the `READ_ONLY` flag (`certs::session`): it
sees nothing else and cannot write.

```rust
// init: session for a program (user/init/src/certs.rs)
let fs = certs::session(devices)?;   // /certs only, read-only
spawn(.., vec![net_session, fs.into_channel()]);
```

The message `init: korenove certifikaty aktualizovany (N kB)` means the
set in the store changed (first start or a new system version). Without
an FS (`init: uloziste certifikatu nelze pripravit: ...`) programs have
no CAs and every `https` connection fails with `UnknownIssuer`.

## Certificate revocation

A CA can invalidate a certificate before it expires (stolen key, wrongly
issued). The client learns about it only from extra data; TLS checks
three sources:

<figure class="diagram">
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Revocation checking">
<defs><marker id="th4" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="20" width="250" height="210" rx="6"/><text class="t b" x="135" y="44" text-anchor="middle">program (tls)</text>
<text class="m" x="25" y="70">1. webpki: chain, name, time</text>
<text class="m" x="25" y="95">2. OCSP from the handshake</text>
<text class="m" x="25" y="120">3. OneCRL: revoked CAs</text>
<text class="m" x="25" y="145">4. CRL from the certificate</text>
<text class="m" x="25" y="170">   (webpki checks its signature)</text>
<text class="m" x="25" y="205">unknown status = passes</text>
<path class="ln-acc" d="M260 120 L338 120" marker-end="url(#th4)"/><text class="m" x="268" y="112">channel</text>
<rect class="ok" x="340" y="60" width="180" height="120" rx="6"/><text class="t b" x="430" y="85" text-anchor="middle">revocd</text><text class="m" x="430" y="108" text-anchor="middle">CRL cache</text><text class="m" x="430" y="126" text-anchor="middle">OneCRL (24 h)</text><text class="m" x="430" y="144" text-anchor="middle">failure: 10 min pause</text>
<path class="ln" d="M520 100 L598 80" marker-end="url(#th4)"/>
<path class="ln" d="M520 140 L598 160" marker-end="url(#th4)"/>
<rect class="box" x="600" y="45" width="150" height="60" rx="6"/><text class="t b" x="675" y="70" text-anchor="middle">CA (http)</text><text class="m" x="675" y="88" text-anchor="middle">CRL</text>
<rect class="box" x="600" y="135" width="150" height="60" rx="6"/><text class="t b" x="675" y="160" text-anchor="middle">Mozilla (https)</text><text class="m" x="675" y="178" text-anchor="middle">OneCRL</text>
<path class="ln" d="M430 180 L430 213" marker-end="url(#th4)"/><text class="m" x="440" y="232">/system/config/certs/crl</text>
</svg>
<figcaption>The program asks revocd; it verifies CRL and OCSP signatures itself.</figcaption>
</figure>

| Source | What it covers | When it rejects |
|---|---|---|
| **OCSP stapling** (`tls::ocsp`) | the end-entity certificate, when the server sends an OCSP response in the handshake | status `revoked`; a bad signature = `InvalidOcspResponse` (spoofed response). An expired response = unknown status |
| **OneCRL** (Mozilla) | revoked intermediate CAs (hundreds of entries, as in Firefox) | a chain certificate on the list (issuer + serial, or subject + SHA-256 of the key) |
| **CRL** | certificates with an `http://...` distribution point (almost all public CAs) | the certificate serial in a CRL signed by its issuer; an expired CRL is not used |

**Unknown status does not stop the connection** (revocd not running, the
CRL cannot be downloaded, a server without stapling). Browsers behave the
same way: a hard failure would let an attacker who blocks CRLs turn it
into a broken network. A revoked certificate
(`Error::Certificate("Revoked")`, code `ACCESS_DENIED`) is always rejected.

### The `revocd` service

Programs ask over a channel that init hands them
(`revocation::client()`); revocd only sees the queries, not the
connection data.

| Message | Reply |
|---|---|
| `crl <url>` | `ok` + CRL (DER) or `none` |
| `ca` + 4 fields with a u32 LE length (issuer, serial, subject, key SHA-256) | `revoked` or `ok` |
| bootstrap: `client` + handle | `ok` (a new program) |
| bootstrap: `status` | `ok:clients=… crls=… fetched=… onecrl=…` |
| bootstrap: `refresh` | `ok:onecrl=…` (OneCRL again now) |

- **CRL:** from memory (64 most recent), from the FS
  (`/crl/<SHA-256 of the URL>.crl`, valid until `nextUpdate`, a CRL
  without it 24 h after download), otherwise downloaded (HTTP/HTTPS, at
  most 16 MiB, 15 s limit). After a failure the URL is not retried for
  10 minutes. revocd does not verify the signature, webpki in the program
  does: a spoofed CRL from revocd achieves nothing.
- **OneCRL:** downloaded on the first query and then every 24 h from the
  URL in `revocd.toml`; the last valid version is kept in
  `/crl/onecrl.json`. Entries with `"enabled": false` are skipped.
- **Permissions:** network session TCP 80/443 anywhere (CAs choose the CRL
  URLs), FS session of the `REVOCD_USER` account (901): `/certs`
  read-only, `/crl` writable. The cache inherits read access for everyone
  (CRLs are public).

```toml
# revocd.toml
onecrl = "https://firefox.settings.services.mozilla.com/v1/buckets/security-state/collections/onecrl/changeset?_expected=0"
onecrl_refresh_hours = 24
crl_default_hours = 24
retry_minutes = 10
```

```rust
// program: revocation checking through revocd (channel from init)
let source = Arc::new(tls::revocation::ChannelSource::new(channel, Duration::from_secs(20)));
let client = tls::Client::with_revocation(&tls::LIBOS, &roots, httpc::ALPN, Some(source))?;
```

> **Change from the first version:** revocation was not checked at all.
> **Why:** a certificate with a stolen key would otherwise stay valid
> until it expires (months for public CAs).

## The `httpc` library

### HTTP/1.1

- Request: `Host`, `User-Agent: MagnumOpus-httpc/0.1`,
  `Accept-Encoding: identity` (no compression), the program's headers,
  `Content-Length` (always for POST/PUT/PATCH), `Connection: close`.
- Response: status line `HTTP/1.0` or `1.1`, 1xx interim responses are
  skipped, `101` is an error.
- Body per RFC 9112 6.3: HEAD, 204 and 304 have none;
  `Transfer-Encoding` wins (`chunked` as the last coding, otherwise until
  the connection ends); otherwise `Content-Length`; otherwise until the
  connection ends. Trailers are read and dropped.
- **Strict parsing** (against request smuggling): continuation lines
  (obs-fold), a space before the colon, an invalid header name and
  differing `Content-Length` values are errors.

### HTTP/2

<figure class="diagram">
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="HTTP/2 request">
<defs><marker id="th3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="10" width="140" height="230" rx="6"/><text class="t b" x="80" y="35" text-anchor="middle">client</text>
<rect class="ok" x="610" y="10" width="140" height="230" rx="6"/><text class="t b" x="680" y="35" text-anchor="middle">server</text>
<path class="ln" d="M150 60 L608 60" marker-end="url(#th3)"/><text class="m" x="170" y="54">preface, SETTINGS (push 0, window 1 MiB), WINDOW_UPDATE</text>
<path class="ln" d="M150 90 L608 90" marker-end="url(#th3)"/><text class="m" x="170" y="84">HEADERS stream 1 (:method :scheme :authority :path) [+ DATA]</text>
<path class="ln" d="M608 120 L152 120" marker-end="url(#th3)"/><text class="m" x="170" y="114">SETTINGS, SETTINGS ACK</text>
<path class="ln" d="M150 145 L608 145" marker-end="url(#th3)"/><text class="m" x="170" y="139">SETTINGS ACK</text>
<path class="ln" d="M608 175 L152 175" marker-end="url(#th3)"/><text class="m" x="170" y="169">HEADERS (:status) [+ CONTINUATION], DATA ... END_STREAM</text>
<path class="ln" d="M150 200 L608 200" marker-end="url(#th3)"/><text class="m" x="170" y="194">WINDOW_UPDATE for every DATA (connection and stream)</text>
<path class="ln" d="M150 225 L608 225" marker-end="url(#th3)"/><text class="m" x="170" y="219">GOAWAY (NO_ERROR or an error code)</text>
</svg>
<figcaption>One request per connection (stream 1); TLS with ALPN "h2".</figcaption>
</figure>

| Area | Behaviour |
|---|---|
| Client settings | `ENABLE_PUSH = 0`, `INITIAL_WINDOW_SIZE = 1 MiB`, `MAX_HEADER_LIST_SIZE` = header limit, `HEADER_TABLE_SIZE = 4096`, `MAX_CONCURRENT_STREAMS = 0` |
| Frames | at most 16 KiB (not raised); larger = `FRAME_SIZE_ERROR`; unknown types are ignored |
| Flow control | the request body only within the server's window (connection and stream, including a negative window after `SETTINGS`); received data is acknowledged at once |
| HPACK | decoder: all representations, dynamic table, Huffman (RFC 7541); encoder: literals without indexing |
| Response checks | `:status` first and the only pseudo-header, lower case, no connection headers, `content-length` = data length, trailers only with `END_STREAM` |
| Server push | disabled; `PUSH_PROMISE` = `PROTOCOL_ERROR` |
| End | `RST_STREAM` = `Error::Reset(code)`; `GOAWAY` before our stream = `Reset(REFUSED_STREAM)` (can be retried) |
| HEAD | data from the server is dropped (hyper sends it) |

### Limits and redirects

```rust
pub struct Limits {
    pub max_header_bytes: usize, // 64 KiB: HTTP/1.1 lines, decoded HTTP/2 list
    pub max_headers: usize,      // 128
    pub max_body: usize,         // 16 MiB
    pub max_redirects: u8,       // 5
}
```

For HTTP/2 the header limit covers both the encoded block (endless
`CONTINUATION`) and the decoded size (a short reference to a large table
entry repeated many times, the "HPACK bomb").

`Client::request` follows redirects 301, 302, 303, 307 and 308:

- 303 (and 301/302 for a method other than GET/HEAD) = a following GET
  without a body; 307/308 = the same method and body,
- `Location` can be absolute, `//server`, `/path`, a relative path (with
  `.` and `..`) or `?query`,
- **`https` → `http` is rejected** (`Error::InsecureRedirect`, code
  `ACCESS_DENIED`),
- `Authorization`, `Cookie` and `Proxy-Authorization` are not sent to a
  different server (scheme, name, port).

### Addresses

`Url::parse` accepts `http://` and `https://`, a server name (lower-cased),
IPv4 or `[IPv6]`, a port and a path with a query. It rejects credentials
in the address (`user:password@` - they would end up in history and
logs), names with non-ASCII letters (no IDN yet), port 0 and control
characters. A space and non-ASCII bytes in the path are encoded (`%20`,
`%C4%8D`), the `#...` fragment is dropped.

### Usage

```rust
struct Web<'a> { net: &'a Net, tls: &'a tls::Client }

impl httpc::Connector for Web<'_> {
    type Stream = Conn; // TcpStream or TlsStream<TcpStream>
    fn connect(&mut self, url: &Url) -> httpc::Result<(Conn, Version)> {
        // TCP; for https TLS with ALPN httpc::ALPN, version from ALPN
    }
}

let mut client = httpc::Client::new(Web { net: &net, tls: &tls }, httpc::Limits::default());
let resp = client.get("https://example.com/")?;
println!("{} {} {} B", resp.status, resp.version.as_str(), resp.body.len());
```

Program headers must have a lower-case token name and a value without CR,
LF and NUL (otherwise another header could be injected). `Host`,
`Content-Length`, `Connection`, `Transfer-Encoding`, `Upgrade`, `TE` and
`Keep-Alive` are added by the library and rejected from the program
(`Error::Request`).

### Errors

| `httpc::Error` | When | Code |
|---|---|---|
| `Io(code)` | stream error during the request | stream code |
| `Transport(code, text)` | the connection cannot be established (TCP, certificate) | from the `Connector` (certificate = `ACCESS_DENIED`) |
| `Url(text)` | invalid address | `INVALID_ARGS` |
| `Request(text)` | invalid program header | `INVALID_ARGS` |
| `Protocol(text)` | the server broke HTTP | `CORRUPTED` |
| `Reset(code)` | HTTP/2 `RST_STREAM` / `GOAWAY` | `CORRUPTED` |
| `TooLarge(what)` | a limit was exceeded | `OUT_OF_RANGE` |
| `TooManyRedirects` | more than `max_redirects` | `OUT_OF_RANGE` |
| `InsecureRedirect` | redirect from `https` to `http` | `ACCESS_DENIED` |

## The `fetch` program

Manifest `user/fetch/fetch.manifest.toml`:

```toml
[app]
id = "org.magnumopus.fetch"
[limits]
memory = "48M"            # cryptography, body (up to 16 MiB), buffers
[network]
rights = ["tcp", "dns"]
ask = true                # a port other than 80/443 = ask the user
[[network.connect]]
proto = ["tcp"]
to = "*"
ports = "80"              # and another rule for 443
```

Handles at start: 1. network session, 2. store FS session (optional;
without it `fetch` trusts no CA), 3. channel to `revocd` (optional;
without it only OCSP stapling). Commands on the bootstrap channel:

| Command | Reply |
|---|---|
| `get <url>`, `head <url>`, `post <url> <body>` | `ok:<status> <version> <body length> <FNV-1a 64 hex> <final address>` |
| `body <url>` | `ok:<body as text, at most 4 KiB>` |
| `roots` | `ok:<CA count> rejected=<n> distrusted=<n>` |
| error | `err:<CODE_NAME> <text>`, e.g. `err:ACCESS_DENIED spojeni nelze navazat: neplatny certifikat serveru: UnknownIssuer` |

TCP connections and reads time out after 30 s.

## Tests

**Host** (`cargo xtask test` runs `cargo test -p tls -p httpc`):

| Test | Checks |
|---|---|
| `tls`: suites × groups | all 9 combinations against a rustls+ring server (HelloRetryRequest for P-256/P-384) |
| `tls`: key types | ECDSA P-256, P-384, Ed25519, RSA 2048 (CA and handshake signatures) |
| `tls`: large transfer | 300 kB up, 1 MiB down (many records both ways) |
| `tls`: ALPN, IP address | selected protocol, server without ALPN, certificate for 127.0.0.1 |
| `tls`: rejections | foreign CA, wrong name, expired certificate (time 2100), no randomness |
| `tls`: truncation, resumption | TCP end without `close_notify` = `Truncated`; second connection over PSK |
| `tls`: roots | PEM with comments, DER, damaged blocks, `distrust.txt`, base64 |
| `tls`: revocation | CRL of the end-entity and intermediate certificate, expired and spoofed CRL, OneCRL, OCSP good / revoked / expired / spoofed, KeyUpdate |
| `httpc`: HPACK | RFC 7541 examples C.1, C.4 (Huffman, dynamic table), C.5 (eviction), Huffman of all bytes, bad padding, "HPACK bomb" |
| `httpc`: scripted HTTP/1.1 | chunked with extensions and trailers, TE vs. CL, 1xx, 304, byte-by-byte data, 15 bad responses, limits |
| `httpc`: scripted HTTP/2 | PING, padding, CONTINUATION, 1xx, trailers, 17 bad responses, request body flow control, body limit |
| `httpc`: against hyper | HTTP/1.1 and HTTP/2: text, 3 MB stream, 200 kB POST, headers, 60 headers (HPACK with a table), HEAD, redirects, loop, `https` → `http`, credential headers |

**QEMU** (the `TLS a HTTP` suite, `user/init/src/tests/web.rs`): `fetch`
against host servers (`xtask/src/webhelper.rs`: rustls+ring, hyper, a test
CA generated on every run):

| Test | Checks |
|---|---|
| store | `mozilla.pem` = initfs, `test-ca.pem`, a repeated seed does not rewrite, the program session only reads and sees nothing outside `/certs` |
| roots | `fetch roots`: more than 100 CAs, none rejected |
| HTTP/1.1 | `http://10.0.2.2:<port>/hello` |
| HTTPS | HTTP/2 via ALPN, HTTP/1.1 with a server without `h2`, a 1 MB body over both, POST |
| rejections | foreign CA (`UnknownIssuer`), wrong name (`NotValidForName`), TLS to a non-TLS port |
| redirects | within `https` yes, to `http` = `ACCESS_DENIED` |
| revocation | certificate in a CRL (revocd downloads and stores it; a non-revoked one with the same CRL passes), OCSP "revoked" in stapling, intermediate CA in OneCRL (an entry with `enabled: false` does not count) |

## Plan

- **CRLite** (a compact filter of all revocations) instead of downloading
  CRLs, and CA set updates outside system releases; revocd serving several
  programs concurrently (queries are sequential now).
- **Connection reuse** (HTTP/1.1 keep-alive, several HTTP/2 streams) and
  compression (gzip, brotli).
- **NTS** (RFC 8915) for authenticated time; DoH/DoT for authenticated DNS.
- **ring or our own assembly** for AES-GCM with AES-NI/ARMv8 Crypto, once
  programs may use SIMD.
- **Client certificates** and a TLS server (for services in the system).
