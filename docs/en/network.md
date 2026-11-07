# Network

The first part of milestone 6 ("network"). These parts are done:

- the **virtio-net** card driver in userspace,
- the network service **`netd`** with a TCP/IP stack (ARP, IPv4, ICMP,
  UDP, TCP), a DHCP client and a DNS resolver,
- the **`netproto`** protocol with access through a capability and
  destination restrictions,
- an **NTP** client that sets the clock through the time service.

TLS and HTTP come later. The interface for them is ready (see
[Preparing for TLS and HTTP](#preparing-for-tls-and-http)).

| Part | Where | What it does |
|---|---|---|
| driver | `user/virtio-net` | virtio queues, DMA through the IOMMU, Ethernet frames ↔ channel |
| service | `user/netd` | TCP/IP stack (smoltcp), DHCP, DNS, permissions, sockets |
| protocol | `libs/netproto` | messages, permissions, DNS and NTP formats, client library |
| device manager | `user/devmgr` | finds the card on PCI, starts the driver |
| init | `user/init/src/system.rs` | starts `netd`, holds the root session, NTP |
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
<rect class="ok" x="300" y="80" width="200" height="110" rx="6"/><text class="t b" x="400" y="105" text-anchor="middle">netd</text><text class="m" x="400" y="125" text-anchor="middle">permissions, sockets = channels</text><text class="m" x="400" y="143" text-anchor="middle">smoltcp: ARP IPv4 ICMP UDP TCP</text><text class="m" x="400" y="161" text-anchor="middle">DHCP, own DNS resolver</text><text class="m" x="400" y="179" text-anchor="middle">1 thread, OBJECT_WAIT_MANY</text>
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
  Closing the channel closes the socket. There is at most one request on a
  channel at a time. The service reads the next message only after it has
  replied, which acts as natural back-pressure.
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

**Queues:** 0 = receive, 1 = transmit, each with at most 64 buffers of
2 KiB. A buffer holds the virtio-net header (12 B) and the frame. Both
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
| `INFO` (1) | driver → netd, first message | MAC (6 B), MTU u16, flags u8 (bit 0 = link up) |
| `FRAME` (2) | both | Ethernet frame without FCS (14 to 1514 B) |
| `LINK` (3) | driver → netd | flags u8 when the link state changes |

A frame that does not fit in the other side's queue is dropped, as on a
real network. TCP takes care of reliability.

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
2. channel of the **root session**, held by `init`.

The service waits for `INFO` from the driver, replies `ok` to its parent
and starts DHCP. Then a single thread repeats:

1. `poll` of the stack: received frames, TCP timers, DHCP,
2. progress of pending requests (connections, reads, DNS queries, ping),
3. `OBJECT_WAIT_MANY` on the driver channel and on client channels, with
   a deadline from the nearest timer. A channel with a pending request is
   not read, only its closing is watched.

| Property | Value |
|---|---|
| TCP buffers | 64 KiB receive + 64 KiB send per connection |
| TCP keep-alive | after 60 s of inactivity, an unresponsive connection is dropped after 120 s |
| connect without a limit | gives up after 75 s (`TIMED_OUT`) |
| closing a connection | FIN, cleanup when finished or after 30 s at the latest |
| listen backlog | 1 to 8 connections |
| ephemeral ports | 49152 to 65535, random start |
| UDP datagram | at most 8 KiB; queue of 32 datagrams / 64 KiB |
| DNS | A (and AAAA) query, retry after 1 s alternating servers, at most 30 s |

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

⚠️ **Limit on the number of sockets:** the service waits with a single
`OBJECT_WAIT_MANY`, which takes at most 64 objects. So there can be at
most 63 sessions and sockets at once. More get `NO_RESOURCES`. That is
enough for the first programs. A kernel object for waiting on any number
of channels (a port) will remove the limit.

⚠️ **Randomness:** TCP numbers, ephemeral ports and DNS query IDs come
from a generator seeded from RDRAND (when the CPU has it), the time and
the MAC. It is **not** a cryptographic generator. Without RDRAND the
values are predictable and an attacker on the same network could spoof a
DNS reply. The service logs this. A kernel entropy source belongs here
(TODO).

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
`SET_DNS`). The service remembers the resolution (at most 256 entries per
session). A reply from a server the program chose itself (`resolve_with`
with a server) does not count. Otherwise a program could "resolve" an
allowed name to any address.

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

| Method | Right | Additional errors |
|---|---|---|
| `Net::info`, `wait_configured` | `INFO` | `TIMED_OUT` |
| `Net::tcp_connect`, `tcp_connect_host` | `TCP_CONNECT` + destination | `CONNECTION_REFUSED`, `UNREACHABLE`, `TIMED_OUT` |
| `Net::tcp_listen` | `TCP_LISTEN` + `bind` | `ADDRESS_IN_USE` |
| `Net::udp_bind` | `UDP` + `bind` | `ADDRESS_IN_USE` |
| `Net::resolve`, `resolve_with` | `RESOLVE` (+ UDP destination for an own server) | `NOT_FOUND`, `TIMED_OUT`, `UNREACHABLE` |
| `Net::ping` | `PING` + ICMP destination | `TIMED_OUT`, `UNREACHABLE` |
| `Net::restrict` | `RESTRICT` | `INVALID_ARGS` (invalid rules) |
| `Net::set_dns` | `CONFIGURE` | |
| `Net::ntp_query` | `UDP` + destination | `TIMED_OUT`, `BAD_STATE` (unsynchronized server) |
| `TcpStream::read`, `write` | | `CONNECTION_RESET`, `TIMED_OUT` |
| `UdpSocket::send_to` | destination | `OUT_OF_RANGE` (over 8 KiB) |

A missing right or a destination outside the restrictions returns
`ACCESS_DENIED`. The new error codes are described in
[errors.md](errors.md).

`TcpStream::read` returns `Ok(0)` at the end of the stream (the other side
finished writing). `CONNECTION_RESET` means the connection was broken by an
RST or the other side stopped responding. A connection can be handed to
another process: `into_channel()` at the sender,
`TcpStream::from_channel()` at the receiver.

## The netproto protocol

Request: `op` u32 and parameters. Reply: `status` i32 (0 or an error
code), reserved u32 and the result. A new socket or session goes in the
reply handles. Encoding: little-endian numbers, IP address = family u8
(4/6) + 4/16 B, `socket_addr` = address + port u16, string = length u16 +
UTF-8, data = the rest of the message. A timeout is u64 in ms: 0 = do not
wait, `u64::MAX` = no limit.

| Channel | Op | Parameters → result |
|---|---|---|
| session | `INFO` (1) | → Info |
| session | `WAIT_CONFIGURED` (2) | timeout → Info |
| session | `TCP_CONNECT` (3) | destination, timeout → local address + connection channel |
| session | `TCP_LISTEN` (4) | port u16, backlog u16 → local address + listener channel |
| session | `UDP_BIND` (5) | port u16 → local address + socket channel |
| session | `RESOLVE` (6) | name, family u8 (0/4/6), has server u8 [server], timeout → count u16, addresses, TTL u32 |
| session | `PING` (7) | address, sequence u16, length u16, timeout → round trip ns u64 |
| session | `RESTRICT` (8) | rights u32, Policy → session channel |
| session | `SET_DNS` (9) | count u8, servers → () |
| connection | `SEND` (20) | timeout, data → accepted u32 |
| connection | `RECV` (21) | max u32, timeout → data (empty = end of stream) |
| connection | `SHUTDOWN` (22) | → () (FIN, reading continues) |
| connection | `ADDRS` (23) | → local, remote address |
| listener | `ACCEPT` (30) | timeout → remote address + connection channel |
| listener, UDP | `LOCAL_ADDR` (31) | → local address |
| UDP | `SEND_TO` (40) | destination, data → () |
| UDP | `RECV_FROM` (41) | max u32, timeout → sender, data |

**Info:** MAC (6 B), MTU u16, flags u8 (bit 0 link, bit 1 configured),
has address u8 [address, prefix u8], has gateway u8 [gateway], DNS count
u8 [addresses], session rights u32.

**Policy:** has connect u8 [count u32, rules], has bind u8 [count u32,
(from u16, to u16)...]. Rule = protocols u8, target kind u8 (0 any,
1 network: address + prefix u8, 2 name: string), port from u16, port to
u16. The service rejects (`INVALID_ARGS`) empty protocols, a prefix longer
than the address, an invalid name and more than 256 rules or 16 layers.

## DNS

The resolver is in the service (`user/netd/src/resolve.rs`), the message
format in `netproto::dns`, where the test server uses it too.

- For family 0 both A and AAAA queries are sent. One answer is enough,
  and an empty AAAA answer does not matter.
- Replies are matched by the random query ID and the server address.
- Name compression is expanded with loop protection (at most 32 jumps).
- NXDOMAIN = `NOT_FOUND`, another server error = `IO_ERROR`.
- An address instead of a name (`"10.0.2.2"`) is returned at once without
  a query.

TODO: a cache by TTL, a TCP query for truncated replies, DoT/DoH (with
TLS).

## NTP and the clock

`Net::ntp_query` sends an NTP v4 query (`netproto::ntp`) and returns the
corrected time, the clock offset and the round-trip delay. The reply is
matched by the send time (the "originate" field). An unsynchronized server
(stratum 0, leap 3) returns `BAD_STATE`.

**Only the time service** `timed` may set the clock (the `SET_CLOCK`
right, see [time.md](time.md)). So `init` only obtains the time and sends
it to the time service with a `set` message:

<figure class="diagram">
<svg viewBox="0 0 760 170" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Setting the clock from NTP">
<defs><marker id="na3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="50" width="150" height="60" rx="6"/><text class="t b" x="85" y="75" text-anchor="middle">NTP server</text><text class="m" x="85" y="93" text-anchor="middle">UDP 123</text>
<path class="ln" d="M238 80 L162 80" marker-end="url(#na3)"/>
<text class="m" x="170" y="70">t1 → t2, t3 → t4</text>
<rect class="box" x="240" y="50" width="120" height="60" rx="6"/><text class="t b" x="300" y="75" text-anchor="middle">netd</text><text class="m" x="300" y="93" text-anchor="middle">UDP socket</text>
<path class="ln" d="M438 80 L362 80" marker-end="url(#na3)"/>
<rect class="acc" x="440" y="50" width="130" height="60" rx="6"/><text class="t b" x="505" y="75" text-anchor="middle">init</text><text class="m" x="505" y="93" text-anchor="middle">ntp_query</text>
<path class="ln-acc" d="M570 80 L618 80" marker-end="url(#na3)"/>
<rect class="ok" x="620" y="50" width="130" height="60" rx="6"/><text class="t b" x="685" y="75" text-anchor="middle">timed</text><text class="m" x="685" y="93" text-anchor="middle">CLOCK_SET</text>
<text class="m" x="10" y="150">offset = ((t2 - t1) + (t3 - t4)) / 2; new time = t4 + offset. Not written to the RTC yet.</text>
</svg>
<figcaption>The network only obtains the time; only the time service may set it.</figcaption>
</figure>

```rust
// init (user/init/src/system.rs)
let offset_ns = system::sync_time_ntp(&net_root, &time_service, server)?;
```

For now the clock is set once and only in tests against the host's server.
TODO: a server from the configuration or from DHCP (option 42), periodic
synchronization and gradual adjustment instead of a jump.

## Preparing for TLS and HTTP

- **Streams:** `netproto::io::{Read, Write}` are a generic byte-stream
  interface and `TcpStream` implements them. TLS will be a wrapper
  `TlsStream<S: Read + Write>` that implements the same. The HTTP client
  will work with any `Read + Write`, over plain TCP and over TLS alike.
- **TLS will run in the program's process** (a library), not in the
  network service. Session keys stay in the program that needs them. The
  store of trusted certificates will be a system service or a read-only
  file.
- **Name-based permissions:** `Target::Host` rules already expect
  applications that may only talk to their API (`*.example.com:443`). TLS
  adds verification that the server really owns the name.
- **Randomness:** TLS needs a cryptographic generator. TLS will not happen
  without a kernel entropy source (TODO above).

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
| TCP echo | 256 KiB there and back, local and remote address, `shutdown_write` and end of stream |
| TCP refusal | closed port = `CONNECTION_REFUSED` (on Windows `TIMED_OUT`, see above), port 0 = `INVALID_ARGS` |
| TCP reset | the host closes the connection with unread data (RST): read and write = `CONNECTION_RESET` |
| TCP listening | the host connects through `hostfwd`, `ADDRESS_IN_USE`, `accept` timeout |
| UDP echo | datagrams there and back, ephemeral port, timeout |
| permissions | missing rights, destinations outside restrictions, handing a connection to another channel |
| host name | the `*.magnumopus` rule applies after a system resolution, not after one by an own server |
| NTP | a query to the host server, setting through `timed`, time source `SET` |

DNS through 10.0.2.3 (slirp forwards the queries to the host's DNS) is not
tested because the test would need the internet. Without the helper
servers (`cargo xtask run`) the tests that need them are skipped. The log
then shows `[preskoceno] sit: ...` ("skipped").

The host also tests the formats and permissions: `cargo test -p netproto
--features std` (part of `cargo xtask test`).

## Plan

- **Own stack:** replaces smoltcp behind the `stack` module of the
  service. Order: ARP + IPv4 + ICMP, UDP, then TCP (congestion control,
  SACK, windows). smoltcp (0BSD licence) serves as a reference and its
  tests as a comparison suite. The protocol and programs do not change.
- **IPv6:** SLAAC, ICMPv6; the resolver already handles AAAA. The protocol
  is ready (`IpAddr` addresses).
- **A kernel object for waiting** on any number of channels: removes the
  limit of 63 sockets.
- **A kernel entropy source** (syscall): for TCP, DNS and above all TLS.
- **Shared memory for data** instead of copying in messages (as for the
  disk).
- **More cards**, static configuration, DNS cache, periodic NTP.
- **TLS 1.3 and an HTTP client** as libraries over `Read + Write`.
