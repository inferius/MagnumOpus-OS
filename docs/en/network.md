# Network

The first part of milestone 6 ("network"). These parts are done:

- the **virtio-net** card driver in userspace,
- drivers for **Intel e1000e, igc and Realtek r8169** cards (see
  [Network cards](network-cards.md)),
- the network service **`netd`** with a TCP/IP stack (ARP, IPv4, ICMP,
  UDP, TCP), a DHCP client and a DNS resolver,
- the **`netproto`** protocol with access through a capability and
  destination restrictions,
- time synchronization by the **`ntpd`** service (smooth correction
  through the time service),
- a **firewall**: application manifest, asking the user, packet filter,
  network profiles, audit and DNS filter (see [firewall.md](firewall.md)).

- **TLS 1.3 and HTTP/1.1, HTTP/2** as libraries in programs, a root
  certificate store and the `fetch` program (see [TLS and HTTP](tls-http.md)).

| Part | Where | What it does |
|---|---|---|
| driver | `user/virtio-net` | virtio queues, DMA through the IOMMU, Ethernet frames ↔ channel |
| service | `user/netd` | TCP/IP stack (smoltcp), DHCP, DNS, permissions, sockets |
| protocol | `libs/netproto` | messages, permissions, DNS and NTP formats, client library |
| device manager | `user/devmgr` | finds the card on PCI, starts the driver |
| firewall | `user/fwd`, `libs/netfilter`, `libs/manifest` | configuration, asking the user, packet filter, manifests |
| init | `user/init/src/system.rs`, `apps.rs` | starts `netd` and `fwd` in jobs by their manifests, holds the root session, starts `ntpd` |
| tests | `user/init/src/tests/net.rs`, `xtask/src/nethelper.rs` | tests in QEMU through slirp against host servers |

## Architecture

<figure class="diagram">
<svg viewBox="0 0 760 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Network architecture">
<defs><marker id="na1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="160" height="60" rx="6"/><text class="t b" x="90" y="45" text-anchor="middle">program A</text><text class="m" x="90" y="63" text-anchor="middle">session: TCP :443 only</text>
<rect class="box" x="10" y="100" width="160" height="60" rx="6"/><text class="t b" x="90" y="125" text-anchor="middle">program B</text><text class="m" x="90" y="143" text-anchor="middle">session: UDP + DNS</text>
<rect class="acc" x="10" y="200" width="160" height="60" rx="6"/><text class="t b" x="90" y="225" text-anchor="middle">init</text><text class="m" x="90" y="243" text-anchor="middle">root session</text>
<path class="ln" d="M170 50 L298 110" marker-end="url(#na1)"/>
<path class="ln" d="M170 130 L298 135" marker-end="url(#na1)"/>
<path class="ln-acc" d="M170 230 L298 160" marker-end="url(#na1)"/>
<text class="m" x="200" y="200">RESTRICT: narrower session</text>
<rect class="ok" x="300" y="80" width="200" height="110" rx="6"/><text class="t b" x="400" y="105" text-anchor="middle">netd</text><text class="m" x="400" y="125" text-anchor="middle">permissions, sockets = channels</text><text class="m" x="400" y="143" text-anchor="middle">smoltcp: ARP IPv4 ICMP UDP TCP</text><text class="m" x="400" y="161" text-anchor="middle">DHCP, own DNS resolver</text><text class="m" x="400" y="179" text-anchor="middle">1 thread, kernel port</text>
<path class="ln" d="M500 135 L568 135" marker-end="url(#na1)"/>
<text class="m" x="505" y="125">frames</text>
<rect class="box" x="570" y="95" width="180" height="80" rx="6"/><text class="t b" x="660" y="120" text-anchor="middle">virtio-net</text><text class="m" x="660" y="139" text-anchor="middle">RX/TX queues, 2 KiB buffers</text><text class="m" x="660" y="157" text-anchor="middle">DMA in the IOMMU domain</text>
<path class="ln" d="M660 175 L660 223" marker-end="url(#na1)"/>
<rect class="warn" x="570" y="225" width="180" height="50" rx="6"/><text class="t b" x="660" y="248" text-anchor="middle">card (QEMU slirp)</text><text class="m" x="660" y="265" text-anchor="middle">10.0.2.0/24</text>
</svg>
<figcaption>Programs only hold session channels. Every socket is another channel from netd.</figcaption>
</figure>

Main decisions:

- **Channels are capabilities.** A program cannot use the network until
  someone hands it a *session* channel. Today that is `init`, later the
  session manager according to the application manifest.
- **A socket is a channel.** A TCP connection, a listener and a UDP socket
  are separate channels. A socket can be handed to another process, for
  example a server passes an accepted connection to a worker process.
  Closing the channel closes the socket. A channel can carry several
  requests at once: each has a number and a deadline, and the reply
  belongs to the number (see [Protocol](#the-netproto-protocol)). Reading
  and writing one connection from different threads do not block each
  other, and a long connect does not hold up the other requests of the
  session **[verified in QEMU]** (suites `sit soubeh` and `firewall`).
  Back-pressure comes from the direction queues and the session request
  limit.
- **The API is not a copy of BSD sockets.** The types are Rust-style:
  `Net`, `TcpStream`, `TcpListener`, `UdpSocket`. A POSIX layer for ported
  C programs can come later on top of them.
- **The stack is smoltcp for now** (0BSD licence), hidden behind the
  `stack` module of the service. The `netproto` protocol, the driver and
  programs do not depend on it, so replacing it with our own stack changes
  nothing outside. The DNS resolver is already our own, see [Plan](#plan).
- **Addresses are `core::net::IpAddr`**, so both IPv4 and IPv6. The
  service supports only IPv4 for now and returns `UNREACHABLE` for IPv6.
  Adding IPv6 will not change the protocol.

## The virtio-net driver

The device manager `devmgr` starts it the same way as the disk driver:
PCI device `1AF4:1041` (modern) or `1AF4:1000` (transitional). The driver
receives:

1. a restricted Resource: only the BARs and configuration of this
   function, interrupt and DMA quotas, with an IOMMU its own DMA domain,
2. the page of PCI configuration space,
3. a channel to the network service (`netproto::link`).

**Negotiated features:** `VERSION_1`, `MAC`, `STATUS`, `ACCESS_PLATFORM`.
Checksum offload and GSO are not used (the stack computes checksums), and
neither is `MRG_RXBUF` (a frame always fits in one buffer).

**Queues:** 0 = receive, 1 = transmit, each with at most 256 buffers of
2 KiB (as many as the device offers; QEMU 256). The driver gets DMA memory
and interrupts from `devmgr` by the device kind (network: 2 MiB, 4
interrupts). A buffer holds the virtio-net header (12 B) and the frame. Both
queues share one MSI-X vector. The driver waits with one
`OBJECT_WAIT_MANY` for the interrupt and for messages from the service:

- **Interrupt:** it acknowledges it (`Interrupt::ack`) before handling it,
  passes received frames to the service and returns the buffers to the
  device.
- **Transmit:** it returns sent buffers to the free list. When no free
  buffer is left, it does not read the channel from the service
  (back-pressure).

For this mode `libs/virtio` has new methods `Virtqueue::post` and
`take_used_any`. Every descriptor is a separate buffer with its own
number. The synchronous mode of the disk (`submit` + `take_used`) is
unchanged.

**Link protocol** (`netproto::link`) is bidirectional, without replies:

| Type | Direction | Content |
|---|---|---|
| `INFO` (1) | driver → netd, first message | MAC (6 B), MTU u16, flags u8, speed u32 (Mb/s, 0 = unknown) |
| `FRAME` (2) | both | Ethernet frame without FCS (14 to 1514 B) |
| `LINK` (3) | driver → netd | flags u8, speed u32 when the link state changes |

Flags: bit 0 = link up, bit 1 = full duplex. A frame that does not fit in
the other side's queue is dropped, as on a real network. TCP takes care
of reliability. When the link comes up again (`LINK` with bit 0), `netd`
starts DHCP again: there may be a different network behind the cable.

> **Change from the first version:** `INFO` and `LINK` had no speed and
> duplex, and `netd` did not repeat DHCP after the link came back.
> **Why:** drivers of real cards ([network cards](network-cards.md))
> report the link, and a cable can be moved. The shorter messages
> (`INFO` 10 B, `LINK` 2 B) are still read. The protocol deliberately
> carries no offloads: smoltcp verifies checksums for the whole device,
> not for a single frame.

The Intel e1000e and igc and Realtek r8169 card drivers talk to `netd`
with the same protocol, see [Network cards](network-cards.md).

⚠️ **DMA without an IOMMU:** the driver uses buffers only from the
device's domain memory. On a machine without an IOMMU, however, the card
can write **anywhere in memory** through DMA. A faulty or malicious card
(or its firmware) can overwrite the kernel. The kernel logs a missing
IOMMU. On real hardware run the network only with an IOMMU.

⚠️ **Faulty device:** when the device returns a nonsensical queue index
or a buffer it did not own (`CORRUPTED`), the driver resets it so it stops
using DMA, and exits. The network service then exits too (restarting
drivers is not implemented yet).

## The netd service

Start (`USER` handles in order):

1. channel to the card driver,
2. channel of the **root session**, held by `init`,
3. (optional) channel to the firewall service `fwd` (see [firewall.md](firewall.md)).
   Without it the public profile applies and application questions are denied.

The service waits for `INFO` from the driver, replies `ok` to its parent
and starts DHCP. Then a single thread repeats:

1. `poll` of the stack: received frames, TCP timers, DHCP,
2. progress of pending requests (connections, reads, DNS queries, ping),
3. `PORT_WAIT` on a **kernel port** (see [Ports](syscalls.md#ports)),
   with a deadline from the nearest timer and request deadlines. Every
   channel has two one-shot registrations on the port: "readable" (renewed
   after reading a batch of at most 32 messages, even while requests are
   pending) and "closed". The packet key carries
   the slot number and its generation, so an old packet after the slot is
   freed confuses nothing. The work in each round is proportional to the
   active objects (pending requests, received packets), not to the number
   of all sockets.

| Property | Value |
|---|---|
| TCP buffers | default 64 KiB receive + 64 KiB send per connection; a program may choose 4 KiB to 16 MiB (`Buffers`, like `SO_RCVBUF`) |
| TCP keep-alive | after 60 s of inactivity, an unresponsive connection is dropped after 120 s |
| connect without a limit | gives up after 75 s (`TIMED_OUT`) |
| closing a connection | FIN, cleanup when finished or after 30 s at the latest |
| listen backlog | 1 to 4096 connections (like `SOMAXCONN`); 16 concurrent handshakes between two rounds of the service |
| ephemeral ports | 49152 to 65535, random start; the last 4096 released ones are not handed out again (an old session in NAT/slirp) |
| concurrent requests on a socket | in each direction (receive: `RECV`, `RECV_FROM`; send: `SEND`, `SHUTDOWN`, `SEND_TO`; `ACCEPT`) at most one in progress, others wait in a FIFO queue, at most 8 in total (`DIRECTION_QUEUE`), further ones get `NO_RESOURCES` |
| concurrent requests on a session | at most `Quota::ops` (default 32, bounds 1 to 1024, `ops` in the manifest) for this session only. The service sets further requests aside (at most 32 messages, FIFO) and starts them when room frees up; then it stops reading the session and the messages wait in the channel in the kernel (back-pressure, no error). `CANCEL` also reaches requests set aside |
| UDP datagram | at most 65,507 B (the largest IPv4 datagram payload; IPv4 fragmentation and reassembly) and at most the socket queue size (default 64 KiB) |
| DNS | A (and AAAA) query with EDNS0 (1232 B), a truncated reply over TCP, retry after 1 s alternating servers, at most 30 s |

**DHCP** sets the address, the default gateway and the DNS servers. Losing
the lease removes the address and connections then get `UNREACHABLE`.
DNS servers set by hand (`SET_DNS`, the `CONFIGURE` right) take precedence
over the DHCP ones.

**ICMP errors:** smoltcp ignores ICMP "destination unreachable" for TCP.
The service catches it while connecting (RFC 1122 4.2.3.9) and ends the
connect with `CONNECTION_REFUSED` (port unreachable) or `UNREACHABLE`
instead of waiting for the timeout.

⚠️ **slirp on Windows does not report refusals:** a connection from the
guest to a closed host port gets neither an RST nor an ICMP error; the
SYN simply stays unanswered and the connect ends with `TIMED_OUT`. On
Linux slirp sends an RST (`CONNECTION_REFUSED`). The test checks
whichever behaviour applies on the host (`refuses` in `net-test`).

**Number of sockets:** thanks to the port the cost of waiting does not
depend on the number of channels. There is no fixed count: every socket
(and session) is charged to the **budget of its session** (number of
sockets and buffer bytes, see [Permissions](#permissions)). The root
session gets 3/4 of the user memory the service job may still get (the
limit from the `netd.manifest.toml` manifest). Channels and port
registrations are guarded by the kernel budget of the job. A test opens
200 sockets at once.

> **Change from the second version:** the fixed limits of the service were
> 16,384 objects, a listen backlog of 8, an 8 KiB datagram, 64 KiB buffers
> for everybody, 256 remembered names per session and DNS only over UDP
> with 512 B. A large server would hit them, and they did not protect the
> service memory sensibly. Session and job budgets and the limits given by
> the protocols apply now.

> **Change from the first version:** the service waited with a single
> `OBJECT_WAIT_MANY` (at most 64 objects), so there could be at most 63
> sockets. It now waits on a kernel port (`PORT_WAIT`).

**Randomness:** TCP numbers (the smoltcp seed), ephemeral ports, DNS query
IDs and ping identifiers come straight from the kernel generator
(`RANDOM_GET`, ChaCha20, see [Random numbers](syscalls.md#random-numbers)).
The service keeps no state of its own from which further values could be
guessed. When the kernel generator is not seeded yet (no RDRAND and no
virtio-rng), the service logs it.

> **Change from the first version:** the service had its own
> non-cryptographic generator seeded from RDRAND, the time and the MAC.
> Without RDRAND the DNS query IDs were predictable.

⚠️ **DNS without verification:** replies are neither signed (DNSSEC) nor
encrypted (DoT, DoH). Whoever controls the network can resolve a name to
another address. Content protection comes with TLS and certificate
verification.

## Permissions

A session has **rights** (what it may do) and **restriction layers**
(where it may go). The service holds them; the client cannot see or
change them.

| Right | Bit | Allows |
|---|---|---|
| `INFO` | 0 | interface state (`info`, `wait_configured`) |
| `TCP_CONNECT` | 1 | outgoing TCP connections |
| `TCP_LISTEN` | 2 | TCP listening |
| `UDP` | 3 | UDP sockets |
| `RESOLVE` | 4 | name resolution |
| `PING` | 5 | ICMP echo |
| `RESTRICT` | 6 | creating narrower sessions for other programs |
| `CONFIGURE` | 7 | interface configuration (DNS servers for now) |
| `AUDIT` | 8 | firewall audit and traffic per application (`audit`, `stats`) |

`rights::CLIENT` = `INFO | TCP_CONNECT | UDP | RESOLVE` is an ordinary
client.

A **restriction layer** (`Policy`) has two parts. `None` in either of them
means unrestricted:

- `connect`: destination rules (`Rule`): protocols (TCP, UDP, ICMP), a
  target and a port range. The target is *any*, a *network*
  (`10.0.2.0/24`) or a *host name* (`api.example.com`, `*.example.com` =
  all subdomains without `example.com` itself).
- `bind`: local port ranges for listening and for a UDP socket on a
  chosen port. Port 0 (assigned by the service) is always allowed.

<figure class="diagram">
<svg viewBox="0 0 760 230" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Session restriction layers">
<defs><marker id="na2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="80" width="150" height="70" rx="6"/><text class="t b" x="85" y="105" text-anchor="middle">root (init)</text><text class="m" x="85" y="123" text-anchor="middle">all rights</text><text class="m" x="85" y="139" text-anchor="middle">no layers</text>
<path class="ln" d="M160 115 L238 115" marker-end="url(#na2)"/>
<rect class="box" x="240" y="70" width="220" height="90" rx="6"/><text class="t b" x="350" y="95" text-anchor="middle">application session</text><text class="m" x="350" y="114" text-anchor="middle">rights CLIENT</text><text class="m" x="350" y="131" text-anchor="middle">layer 1: TCP *.example.com:443</text><text class="m" x="350" y="148" text-anchor="middle">+ UDP 10.0.0.0/8:53</text>
<path class="ln" d="M460 115 L538 115" marker-end="url(#na2)"/>
<rect class="box" x="540" y="70" width="210" height="90" rx="6"/><text class="t b" x="645" y="95" text-anchor="middle">plugin session</text><text class="m" x="645" y="114" text-anchor="middle">rights ∩ (cannot add)</text><text class="m" x="645" y="131" text-anchor="middle">layer 1 + layer 2:</text><text class="m" x="645" y="148" text-anchor="middle">TCP :443 only</text>
<text class="m" x="10" y="195">A destination must pass every layer. A derived session never may more than its parent.</text>
<text class="m" x="10" y="215">A host name applies to addresses the session got from the system resolver.</text>
</svg>
<figcaption>Each narrowing adds a layer; rights can only be removed.</figcaption>
</figure>

**A host-name rule** applies to addresses the session got for that name
from the **system** resolver (DHCP servers or the ones set with
`SET_DNS`). The service remembers the resolution with a validity from the
TTL (at least 5 minutes, because a program connects some time after the
resolution and a short CDN TTL would break it; at most a day). A session
holds at most 4096 entries: expired ones drop out first, then the
oldest. A reply from a server the program chose itself (`resolve_with`
with a server) does not count. Otherwise a program could "resolve" an
allowed name to any address.

**The session budget** (`Quota`): at most this many sockets and buffer
bytes. Every socket (connection, listener, UDP socket, pending DNS query
and ping) and every sub-session (2 KiB) is charged to the budget of the
session and all its parents, just like kernel jobs. Exceeding it =
`NO_RESOURCES`.

**Application identity and asking:** a narrowing (`RESTRICT`) can carry an
application identity (from the manifest) and the `ask` flag. The user
then decides about a target outside the rules of such a layer through the
firewall (see [Asking the user](firewall.md#asking-the-user)). A layer
without `ask` is a hard boundary.

**A UDP socket** is restricted on receive too: a datagram from a sender
outside the allowed destinations is dropped. TCP listening is restricted
only by `bind` and incoming connections are not filtered by `connect`.

Example: `init` creates a session for an application that may only use
HTTPS to its API and resolve names.

```rust
use netproto::{Policy, Rule, Target, proto, rights};

let policy = Policy::only(vec![Rule::port(
    proto::TCP,
    Target::Host("*.example.com".into()),
    443,
)]);
let app_net = root.restrict(rights::INFO | rights::TCP_CONNECT | rights::RESOLVE, &policy)?;
// app_net.into_channel() is handed to the application in its start message.
```

## API for programs

The client library is in `netproto` (feature `client`). All methods
block. A `None` timeout means no limit.

**Threads** **[verified in QEMU]**: `Net`, `TcpStream`, `TcpListener`
and `UdpSocket` are `Send + Sync` and their methods take `&self`. Several
threads can wait on one channel at once. `try_clone()` returns another
handle to the same socket (it shares the channel and the
`set_read_timeout` and `set_write_timeout` limits, like `SO_RCVTIMEO`);
`Net` is `Clone`. A shared `Mux` routes replies by request number. A
waiting thread reads the channel itself when nobody else is reading,
stores replies for others and wakes their threads (futex). A single thread
therefore makes no extra syscall.

**Deadline and cancellation** **[verified in QEMU]**: the `*_ctx` variants
(`Net::tcp_connect_ctx`, `resolve_ctx`, `TcpStream::recv_ctx`,
`send_ctx`, `TcpListener::accept_ctx`, `UdpSocket::recv_from_ctx`) take an
absolute `Deadline` (`Deadline::after(Option<Duration>)`,
`Deadline::NONE`, `Deadline::TRY` = only try) and an optional
`CancelToken`. `CancelToken::cancel()` from another thread cancels the
requests in progress with this token (they return `CANCELED`), and every
later request with it returns `CANCELED` at once. A connection releases
its socket and port, a firewall question stops waiting for the user. The
time of a question to the user does not count towards the deadline.
`Deadline::TRY` is the non-blocking mode: `recv_ctx`, `send_ctx`,
`accept_ctx` and `recv_from_ctx` without data, room or a connection return
`SHOULD_WAIT` (not `TIMED_OUT`) **[verified in QEMU]**. Readiness for
`poll`/mio (a signal on an object or a port) is **[designed]**.

**Client safeguard:** `netd` enforces operation limits, and the client has
its own as well: the deadline + 10 s, 30 s for operations without a limit
(status, session, socket), plus 300 s for operations that may wait for a
firewall question (connect, datagram to a new target, ping, resolution by
an own server). When `netd` does not answer, the program gets `TIMED_OUT`,
the client sends `CANCEL`, and the log shows `netproto: sluzba site
neodpovedela na dotaz <op> (id N) vcas, rusim ho` ("the network service
did not answer request <op> in time, cancelling it"). The channel stays
usable: a late reply carries a number nobody waits for and is dropped
together with its handles.

> **Change from the first version:** a channel carried at most one
> request and a reply had no number. Two threads on one connection swapped
> replies, a write waited behind a blocked `RECV`, and a connect waiting
> for a firewall question (up to 300 s) blocked the whole session. A
> channel was unusable after the safeguard expired. **Why:** std
> (`TcpStream` is both `Read` and `Write`, `try_clone`, a reader thread and
> a writer thread) and async runtimes need concurrency. The header with a
> number, a deadline and cancellation is also the operation context of
> the future IDL ([architecture](architecture.md)). The old message format
> is not read: all clients were converted at once.

```rust
use core::time::Duration;
use netproto::io::{Read, Write};
use netproto::Net;

fn main(startup: libos::Startup) -> i64 {
    // The program got the session channel from init (e.g. its first USER handle).
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
    u.send_to(b"hello", "10.0.2.2:7".parse().unwrap()).unwrap();
    let mut buf = [0u8; 1500];
    let (n, from) = u.recv_from(&mut buf, Some(Duration::from_secs(1))).unwrap();

    // Server
    let l = net.tcp_listen(8080, 4).unwrap();
    let (conn, peer) = l.accept().unwrap();
    0
}
```

Reading and writing from different threads, and cancellation:

```rust
use netproto::{CancelToken, Deadline};

let s = net.tcp_connect(addr, Some(Duration::from_secs(5)))?;
let reader = s.try_clone()?;
let stop = CancelToken::new();
let token = stop.clone();
libos::thread::spawn("cteni", move || {
    let mut buf = [0u8; 4096];
    // Waits without a limit until data arrives or `stop.cancel()`.
    while let Ok(n) = reader.recv_ctx(&mut buf, Deadline::NONE, Some(&token)) {
        if n == 0 { break; }
        // ...
    }
})?;
s.write(b"hello")?;     // does not block behind the read
stop.cancel();          // the read ends with CANCELED
```

| Method | Right | Additional errors |
|---|---|---|
| `Net::info`, `wait_configured` | `INFO` | `TIMED_OUT` |
| `Net::tcp_connect`, `tcp_connect_host` | `TCP_CONNECT` + destination | `CONNECTION_REFUSED`, `UNREACHABLE`, `TIMED_OUT` |
| `Net::tcp_listen` | `TCP_LISTEN` + `bind` | `ADDRESS_IN_USE` |
| `Net::udp_bind` | `UDP` + `bind` | `ADDRESS_IN_USE` |
| `Net::resolve`, `resolve_with` | `RESOLVE` (+ UDP destination for an own server) | `NOT_FOUND`, `TIMED_OUT`, `UNREACHABLE` |
| `Net::ping` | `PING` + ICMP destination | `TIMED_OUT`, `UNREACHABLE` |
| `Net::restrict`, `restrict_with` | `RESTRICT` | `INVALID_ARGS` (invalid rules, `ask` without an identity), `NO_RESOURCES` |
| `Net::audit`, `stats` | `AUDIT` | |
| `Net::tcp_connect_with`, `tcp_listen_with`, `udp_bind_with` | as without `_with` | `NO_RESOURCES` (session budget) |
| `*_ctx` variants | as without `_ctx` | `CANCELED` (cancelled by a token), `TIMED_OUT` (deadline) |
| every request on a socket | | `NO_RESOURCES` (the direction queue is full, more than 8 requests in one direction) |
| `Net::tcp_listen_addr`, `udp_bind_addr` | as `tcp_listen`, `udp_bind` | `ADDRESS_NOT_AVAILABLE` (the address is neither `0.0.0.0` nor the interface address; `127.0.0.1` too, there is no loopback yet) |
| `Net::set_dns` | `CONFIGURE` | |
| `Net::ntp_query` | `UDP` + destination | `TIMED_OUT`, `BAD_STATE` (unsynchronized server) |
| `TcpStream::read`, `write` | | `CONNECTION_RESET`, `TIMED_OUT` |
| `UdpSocket::send_to` | destination | `OUT_OF_RANGE` (over 65,507 B or over the socket queue) |

A missing right or a destination outside the restrictions returns
`ACCESS_DENIED`. The new error codes are described in
[errors.md](errors.md).

`UdpSocket::local_addr` and `TcpListener::local_addr` ask the service
(they return `Result`): a socket created before DHCP has the address
0.0.0.0 only until the interface is configured. An address instead of a
name (IPv6 too, `"::1"`) is returned by `resolve` at once without a query.

`TcpStream::read` returns `Ok(0)` at the end of the stream (the other side
finished writing). `CONNECTION_RESET` means the connection was broken by an
RST or the other side stopped responding. A connection can be handed to
another process: `into_channel()` at the sender (`BAD_STATE` when a
clone still holds the connection or a request is running on it),
`TcpStream::from_channel()` at the receiver.

## The netproto protocol

Version 2 **[verified in QEMU]**. Every message has a header with the
operation context (`netproto::msg`):

| Message | Header | Body |
|---|---|---|
| request | `op` u32, `id` u32, deadline u64 (16 B) | parameters |
| reply | `id` u32, `status` i32 (8 B) | result (only with `status` 0) |

- **`id`** is chosen by the client. It is unique among the requests in
  progress on the channel and never 0. The reply carries the same `id`;
  the order of replies is not fixed. A request whose `id` is already
  waiting gets `ALREADY_EXISTS`.
- **The deadline** is an absolute time of the kernel's monotonic clock in
  ns (the same clock in all processes). `u64::MAX` = no limit, 0 (or the
  past) = only try. Expiry = `TIMED_OUT`, for "only try" (0)
  `SHOULD_WAIT` (`RECV`, `SEND`, `ACCEPT`, `RECV_FROM`, `SEND_TO`). Time
  spent in a queue is thus not counted twice.
- **`CANCEL`** (op `0xFFFF_FFFF`, on every channel): the `id` in the
  header is the number of the request to cancel; the body is empty and it
  has no reply of its own. The cancelled request (in progress or queued)
  replies `CANCELED`; a `SEND` of which a part already went through
  replies with the number of accepted bytes. The service ignores an
  unknown or finished `id`.
- The service drops a message without a valid header (it has no number to
  reply to).

A new socket or session goes in the reply handles. Encoding: little-endian
numbers, IP address = family u8 (4/6) + 4/16 B, `socket_addr` = address +
port u16, string = length u16 + UTF-8, data = the rest of the message. The
channel between `fwd` and the user agent has its own reply format
(`fw::agent_reply`), see [firewall.md](firewall.md).

| Channel | Op | Parameters → result |
|---|---|---|
| session | `INFO` (1) | → Info |
| session | `WAIT_CONFIGURED` (2) | → Info (until the deadline) |
| session | `TCP_CONNECT` (3) | destination, Buffers → local address + connection channel |
| session | `TCP_LISTEN` (4) | local address (`0.0.0.0` or the interface address), backlog u16, Buffers → local address + listener channel |
| session | `UDP_BIND` (5) | local address, Buffers → local address + socket channel |
| session | `RESOLVE` (6) | name, family u8 (0/4/6), has server u8 [server] → count u16, addresses, TTL u32 |
| session | `PING` (7) | address, sequence u16, length u16 → round trip ns u64 |
| session | `RESTRICT` (8) | Restriction → session channel |
| session | `SET_DNS` (9) | count u8, servers → () |
| session | `AUDIT` (10) | from u64, max u16 → next u64, count u16, records |
| session | `STATS` (11) | → profile, count u16, traffic per application |
| connection | `SEND` (20) | data → accepted u32 |
| connection | `RECV` (21) | max u32 → data (empty = end of stream) |
| connection | `SHUTDOWN` (22) | → () (FIN, reading continues) |
| connection | `ADDRS` (23) | → local, remote address |
| listener | `ACCEPT` (30) | → remote address, local address + connection channel |
| listener, UDP | `LOCAL_ADDR` (31) | → local address (from the current interface address) |
| UDP | `SEND_TO` (40) | destination, data → () (at most 5 s for room in the queue) |
| UDP | `RECV_FROM` (41) | max u32 → sender, data |

**Info:** MAC (6 B), MTU u16, flags u8 (bit 0 link, bit 1 configured),
has address u8 [address, prefix u8], has gateway u8 [gateway], DNS count
u8 [addresses], session rights u32.

**Buffers:** receive u32, send u32 (0 = default 64 KiB, otherwise 4 KiB
to 16 MiB).

**Restriction:** rights u32, Policy, flags u8 (bit 0 = `ask`),
application (string, at most 128 characters `a-z A-Z 0-9 . - _`), sockets
u32, bytes u64 (`u32::MAX` / `u64::MAX` = no limit), concurrent requests
u32 (bounds 1 to 1024; init gives 32).

**Policy:** has connect u8 [count u32, rules], has bind u8 [count u32,
(from u16, to u16)...]. Rule = protocols u8, target kind u8 (0 any,
1 network: address + prefix u8, 2 name: string), port from u16, port to
u16. The service rejects (`INVALID_ARGS`) empty protocols, a prefix longer
than the address, an invalid name and more than 256 rules or 16 layers.

## DNS

System resolution goes through `dnsd` (see below). The unauthenticated
resolver in the service (`user/netd/src/resolve.rs`, format
`netproto::dns`) serves queries to an own server and the fallback by
network profile:

- For family 0 both A and AAAA queries are sent. One answer is enough,
  and an empty AAAA answer does not matter.
- Replies are matched by the random query ID and the server address.
- Name compression is expanded with loop protection (at most 32 jumps).
- NXDOMAIN = `NOT_FOUND`, another server error = `IO_ERROR`.
- An address instead of a name (`"10.0.2.2"`) is returned at once without
  a query.
- **EDNS0:** the query offers a reply of up to 1232 B over UDP (DNS Flag
  Day 2020: it fits one packet even over IPv6 with an MTU of 1280,
  without fragmentation).
- **A truncated reply** (the TC bit): the resolver asks over TCP on the
  same server and port (RFC 7766, a message with a u16 length). For an
  own server it is enough that the session may send UDP to the server.
- **A stuck flow:** UDP retries switch to a new local port every 3
  attempts, and a TCP connection that does not get established within 3 s
  is dropped and reopened (at most 3 times). Slirp on Windows keeps one host
  socket per flow; when it breaks, every further attempt from the same port
  would be lost (message `netd: DNS pres TCP: spojeni se nenavazalo do 3 s,
  novy pokus (n)` - "connection not established within 3 s, new attempt").
- **Blocked domains** of the firewall return `ACCESS_DENIED` without a
  query (see [DNS filter](firewall.md#dns-filter)).

### Encrypted DNS and DNSSEC (`dnsd`)

<figure class="diagram">
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Encrypted DNS">
<defs><marker id="sd1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="130" height="60" rx="6"/><text class="t b" x="75" y="45" text-anchor="middle">program</text><text class="m" x="75" y="64" text-anchor="middle">resolve()</text>
<path class="ln" d="M140 50 L198 50" marker-end="url(#sd1)"/>
<rect class="acc" x="200" y="20" width="160" height="110" rx="6"/><text class="t b" x="280" y="45" text-anchor="middle">netd</text><text class="m" x="280" y="64" text-anchor="middle">blocked domains</text><text class="m" x="280" y="82" text-anchor="middle">→ dnsd (channel)</text><text class="m" x="280" y="100" text-anchor="middle">learns name</text><text class="m" x="280" y="118" text-anchor="middle">rules</text>
<path class="ln-acc" d="M360 50 L418 50" marker-end="url(#sd1)"/>
<rect class="ok" x="420" y="20" width="170" height="110" rx="6"/><text class="t b" x="505" y="45" text-anchor="middle">dnsd</text><text class="m" x="505" y="64" text-anchor="middle">DO + CD, DNSSEC</text><text class="m" x="505" y="82" text-anchor="middle">chain from IANA anchor</text><text class="m" x="505" y="100" text-anchor="middle">cache (TTL)</text><text class="m" x="505" y="118" text-anchor="middle">OK/NX/BOGUS/…</text>
<path class="ln" d="M590 50 L638 50" marker-end="url(#sd1)"/>
<rect class="box" x="640" y="20" width="110" height="60" rx="6"/><text class="t b" x="695" y="45" text-anchor="middle">resolver</text><text class="m" x="695" y="64" text-anchor="middle">DoH / DoT</text>
<path class="ln" d="M280 130 L280 168" marker-end="url(#sd1)"/>
<rect class="warn" x="200" y="170" width="160" height="60" rx="6"/><text class="t b" x="280" y="195" text-anchor="middle">UDP/TCP fallback</text><text class="m" x="280" y="214" text-anchor="middle">DHCP servers</text>
<text class="m" x="380" y="190">only when dnsd replies UNAVAILABLE</text>
<text class="m" x="380" y="208">and the network profile has</text>
<text class="m" x="380" y="226">unauthenticated = "fallback"</text>
</svg>
<figcaption>System resolution: encrypted and validated; an unauthenticated fallback only by network profile.</figcaption>
</figure>

System resolution (sessions without an own server) goes from `netd` to the
**`dnsd`** service. It asks **encrypted** (DoH, RFC 8484, or DoT,
RFC 7858) and **validates** the answers itself (DNSSEC): the DS chain from
the IANA root anchor, RRSIG signatures, NSEC and NSEC3 denial proofs. The
resolver is not trusted - it can only lie by calling a zone unsigned, and a
proof from the parent zone does not let that through.

| `dnsd` result | For the program |
|---|---|
| addresses (validated, or an unsigned zone) | addresses; `netd` learns name rules from them |
| NXDOMAIN, NODATA (validated) | `NOT_FOUND` |
| **BOGUS** (a signature or proof failed) | `CORRUPTED` - an attack, no fallback |
| UNAVAILABLE (no resolver answered) | by network profile: the unauthenticated resolver (UDP/TCP to DHCP or `SET_DNS` servers), or `UNREACHABLE` |

- **The channel** `netd` ↔ `dnsd` is created before `netd` starts (4th
  handle); queries wait in it until `dnsd` is up. When `dnsd` exits, `netd`
  resolves only by the profile rule.
- **Resolvers** in `dnsd.toml` carry an address
  (`doh:https://name/path@address`, `dot:name@address[:port]`): `dnsd`
  does not resolve resolver names. The `dnsd` network session may only use
  TCP to these addresses. After an error it tries the next one and keeps
  the last working one.
- **Anchors:** the root DS per IANA `root-anchors.xml`: KSK-2017 (20326)
  and KSK-2024 (38696, signing the root since 2026-10-11). At the next root
  key rollover `trust_anchors` must be updated (no RFC 5011 yet).
- **Algorithms:** RSASHA256, RSASHA512, ECDSA P-256/P-384, Ed25519; SHA-256
  and SHA-384 digests. A zone with another algorithm or NSEC3 above 100
  iterations (RFC 9276) is treated as unsigned.
- **Cache** by TTL (negative answers by SOA), 1,024 entries, at most 1 day;
  zone states (keys) separately, at most 1 h. Bogus answers are not cached.
- **No revocation check** of the resolver certificate (OCSP stapling only):
  `revocd` resolves names through `dnsd`, the check would loop.
- Bootstrap commands: `status` (`queries upstream cache_hits secure
  insecure bogus unavailable dnssec server error`), `flush`, `prefer <n>`.

```toml
# dnsd.toml
servers = ["doh:https://cloudflare-dns.com/dns-query@1.1.1.1", "dot:one.one.one.one@1.0.0.1"]
dnssec = true
trust_anchors = ["20326 8 2 E06D44B8...", "38696 8 2 683D2D0A..."]
```

> **Change from the first version:** `netd` asked the DHCP servers
> unencrypted and unvalidated. **Why:** the network (Wi-Fi, ISP) saw where
> you connect and could spoof answers. TLS detects a spoofed server, but
> DNSSEC and encryption also protect names without TLS and privacy.


## NTP and the clock

`Net::ntp_query` sends an NTP v4 query (`netproto::ntp`) and returns the
corrected time, the clock offset and the round-trip delay. The reply is
matched by the send time (the "originate" field). An unsynchronized server
(stratum 0, leap 3) returns `BAD_STATE`.

The clock is synchronized over the network by the **`ntpd`** service: it
measures the offset against servers, filters the measurements and
proposes corrections to the time service `timed`, the only one allowed to
change the clock (the `SET_CLOCK` right). After the first synchronization
the time is only adjusted smoothly (`CLOCK_ADJUST`) and never jumps back.
Details in [Real time](time.md#synchronization-over-the-network-ntpd).

> **Change from the first version:** init asked an NTP server once and
> set the time with a jump (`sync_time_ntp`, tests only). **Why:** a jump
> back breaks programs that compare times (certificates, files, logs) and
> a single unfiltered measurement is easy to spoof. `ntpd` runs all the
> time, with a network session limited to the configured servers.

## TLS and HTTP

TLS 1.3 and the HTTP/1.1 and HTTP/2 client are libraries in the program's
process (`libs/tls`, `libs/httpc`) over the generic stream
`netproto::io::{Read, Write}`, so they work over plain TCP and over TLS
alike. Root certificates live in the system store `/system/config/certs`,
which a program gets as a read-only FS session. Everything is described
in the [TLS and HTTP](tls-http.md) chapter.

> **Change from the first version:** the certificate store was meant to
> be a service or a file; it is a directory on the FS (the Mozilla set +
> custom CAs + `distrust.txt`). **Why:** an administrator adds a company
> CA as a file, a system update does not overwrite their files, and the FS
> checks permissions as for any other data.

## Tests

`cargo xtask test` starts QEMU with a `virtio-net-pci` card in user-mode
networking (slirp) and helper servers on the host
(`xtask/src/nethelper.rs`). The servers listen only on `127.0.0.1` on free
ports, so concurrent test runs do not interfere. The guest gets the ports
in the `net-test` file in the initfs. slirp translates 10.0.2.2 to the
host's 127.0.0.1.

<figure class="diagram">
<svg viewBox="0 0 760 210" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Network tests through slirp">
<defs><marker id="na4" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="30" width="230" height="150" rx="6"/><text class="t b" x="125" y="55" text-anchor="middle">guest (QEMU) 10.0.2.15</text><text class="m" x="125" y="78" text-anchor="middle">init: network tests</text><text class="m" x="125" y="96" text-anchor="middle">netd + virtio-net</text><text class="m" x="125" y="114" text-anchor="middle">listens on :8080</text>
<rect class="warn" x="290" y="30" width="180" height="150" rx="6"/><text class="t b" x="380" y="55" text-anchor="middle">slirp</text><text class="m" x="380" y="78" text-anchor="middle">DHCP, gateway 10.0.2.2</text><text class="m" x="380" y="96" text-anchor="middle">DNS 10.0.2.3</text><text class="m" x="380" y="114" text-anchor="middle">10.0.2.2 → 127.0.0.1</text><text class="m" x="380" y="132" text-anchor="middle">hostfwd → :8080</text>
<rect class="ok" x="520" y="30" width="230" height="150" rx="6"/><text class="t b" x="635" y="55" text-anchor="middle">xtask (127.0.0.1)</text><text class="m" x="635" y="78" text-anchor="middle">TCP echo, UDP echo</text><text class="m" x="635" y="96" text-anchor="middle">DNS (test.magnumopus)</text><text class="m" x="635" y="114" text-anchor="middle">NTP, closed port, RST</text><text class="m" x="635" y="132" text-anchor="middle">callback → hostfwd</text>
<path class="ln" d="M240 105 L288 105" marker-end="url(#na4)"/>
<path class="ln" d="M470 105 L518 105" marker-end="url(#na4)"/>
<path class="ln" d="M518 150 L472 150" marker-end="url(#na4)"/>
<path class="ln" d="M288 150 L242 150" marker-end="url(#na4)"/>
</svg>
<figcaption>The tests do not need the internet: all servers run on the host.</figcaption>
</figure>

| Test | Checks |
|---|---|
| DHCP | address 10.0.2.15/24, gateway 10.0.2.2, DNS 10.0.2.3, MTU, link |
| ping | ICMP echo to the gateway (slirp answers itself), IPv6 = `UNREACHABLE` |
| DNS | a query through slirp to the host server: A, A+AAAA, NXDOMAIN, invalid name, address instead of a name |
| DNS large reply | 100 addresses (more than 512 B), a reply truncated over UDP → query over TCP |
| TCP echo | 256 KiB there and back, local and remote address, `shutdown_write` and end of stream |
| TCP refusal | closed port = `CONNECTION_REFUSED` (on Windows `TIMED_OUT`, see above), port 0 = `INVALID_ARGS` |
| TCP reset | the host closes the connection with unread data (RST): read and write = `CONNECTION_RESET` |
| TCP listening | the host connects through `hostfwd`, `ADDRESS_IN_USE`, `accept` timeout |
| UDP echo | datagrams there and back, ephemeral port, timeout |
| UDP large datagram | 60,000 B there and back (IPv4 fragmentation and reassembly) |
| permissions | missing rights, destinations outside restrictions, handing a connection to another channel |
| host name | the `*.magnumopus` rule applies after a system resolution, not after one by an own server |
| encrypted DNS | the "sifrovane DNS" suite: validated A, CNAME, wildcard, AAAA; NXDOMAIN (NSEC and NSEC3), NODATA; unsigned and bogus zone; failover from a broken resolver, DoT, cache; the profile rule |
| concurrency ("sit soubeh" suite) | reading and writing one connection from two threads (the write does not wait behind the read), two readers on one connection (FIFO queue), "only try" = `SHOULD_WAIT` and a 300 ms deadline, cancelling a read from another thread and a token cancelled beforehand, the local address of a listener and a UDP socket (`ADDRESS_NOT_AVAILABLE` for a foreign address and 127.0.0.1) |
| full session limit ("firewall" suite) | a session with `ops = 1`: the second request is set aside (it does not fail), its cancellation arrives within 2 s while the first request still waits for the user |
| connect cancellation ("firewall" suite) | a connect waits for a user question, the session meanwhile answers `INFO`, a `CancelToken` cancels it within 2 s (`CANCELED`), a late user answer changes nothing |
| NTP | `ntpd` against the host server (the "synchronizace casu" suite, see [time.md](time.md#synchronization-over-the-network-ntpd)) |

**DNS in tests is local only:** all queries go to the host helper server
from `nethelper` or to the test DoH/DoT; in a test run init also points the
unauthenticated fallback of the system resolver at the helper (`set_dns`),
so even a `dnsd` outage never reaches the real DNS. DNS query limits in
tests are 30 s (running under TCG and load). The host resolver (10.0.2.3,
slirp forwards to the Windows/Linux DNS) is tested only by
`cargo xtask test --net-online` - it depends on the host network and the
internet. Without the helper
servers (`cargo xtask run`) the tests that need them are skipped. The log
then shows `[preskoceno] sit: ...` ("skipped").

**Helper server ports:** none is fixed or guessable. Every server takes
an ephemeral port from the host kernel (bind to port 0, up to 20 attempts
on an error such as Windows 10013), and xtask passes the ports to the guest
in the initfs (`net-test`). The port for the refusal test (`closed`) is held
by xtask for the whole run as a socket bound without `listen`: the kernel
refuses connections to it and no other process can take it meanwhile. The
`hostfwd` port is held by xtask until QEMU starts. When QEMU exits right
after start with a message about a taken port (QMP or `hostfwd`, concurrent
test runs), xtask starts it again with new ports (at most 3 attempts,
message `xtask: QEMU nenastartovalo (...), novy pokus s jinymi porty` -
"QEMU did not start, new attempt with other ports").

**Load (several QEMUs at once):** UDP tests send every further attempt from
a new socket (a new flow in slirp; a late reply to an old attempt does not
arrive), and `dnsd` waits 5 s for a connection to a resolver (a lost SYN is
retransmitted).

The host also tests the formats and permissions: `cargo test -p netproto
--features std` (part of `cargo xtask test`). The firewall tests are
described in [firewall.md](firewall.md#tests).

## Plan

- **Own stack:** replaces smoltcp behind the `stack` module of the
  service. Order: ARP + IPv4 + ICMP, UDP, then TCP (congestion control,
  SACK, windows). smoltcp (0BSD licence) serves as a reference and its
  tests as a comparison suite. The protocol and programs do not change.
- **IPv6:** SLAAC, ICMPv6; the resolver already handles AAAA. The protocol
  is ready (`IpAddr` addresses).
- **Shared memory for data** instead of copying in messages (as for the
  disk).
- **More socket operations** (codes reserved in `netproto::op`): `RECV`
  with a `PEEK` flag (`RECV_FROM` too), `SHUTDOWN_READ`, `SET_OPTION` and
  `GET_OPTION` (nodelay, TTL, keep-alive, linger, broadcast) and a
  readiness signal for non-blocking loops (mio, tokio) **[designed]**.
- **Loopback** (127.0.0.1) in the service **[designed]**.
- **More cards**, static configuration, DNS cache, NTP servers from DHCP
  (option 42).
