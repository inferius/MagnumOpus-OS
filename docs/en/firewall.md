# Firewall

The firewall is built into the network service. `netd` is the only
network stack in the system, so it cannot be bypassed: a program has no
other way to the network than through a session channel, and every
received frame passes the filter before the stack gets it.

It has six layers:

| Layer | Where | What it does |
|---|---|---|
| **1. Application manifest** | `<program>.manifest.toml`, `libs/manifest`, init | application identity, job budget, network rights and allowed targets |
| **2. Asking the user** | `netd` → `fwd` → agent (UI) | a target outside the manifest rules: allow / deny once or always |
| **3. Packet filter** | `libs/netfilter` in `netd` | inbound connections by profile, blocked networks, rate limits |
| **4. Network profiles** | `fwd` | home / public / work network, recognized by the gateway MAC |
| **5. Audit** | `netd` (`AUDIT`, `STATS`) | who, where, when; denials; traffic per application |
| **6. DNS filter** | resolver in `netd` | blocked domains (`ACCESS_DENIED` instead of a translation) |

The configuration is managed by the **`fwd`** service (`user/fwd`). It
keeps it in a TOML file on the file system, so it is versioned with the
FS snapshot.

<figure class="diagram">
<svg viewBox="0 0 760 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Firewall: netd, fwd, agent">
<defs><marker id="fw1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="170" height="70" rx="6"/><text class="t b" x="95" y="45" text-anchor="middle">application</text><text class="m" x="95" y="63" text-anchor="middle">session from manifest</text><text class="m" x="95" y="79" text-anchor="middle">(ask = true)</text>
<path class="ln" d="M180 55 L278 95" marker-end="url(#fw1)"/>
<text class="m" x="190" y="60">connect</text>
<rect class="ok" x="280" y="60" width="210" height="150" rx="6"/><text class="t b" x="385" y="85" text-anchor="middle">netd</text><text class="m" x="385" y="105" text-anchor="middle">session layers + decisions</text><text class="m" x="385" y="122" text-anchor="middle">blocked networks, DNS filter</text><text class="m" x="385" y="139" text-anchor="middle">inbound filter (profile)</text><text class="m" x="385" y="156" text-anchor="middle">audit, traffic per app</text><text class="m" x="385" y="190" text-anchor="middle">ASK →   ← ANSWER, CONFIG</text>
<path class="ln-acc" d="M490 120 L568 120" marker-end="url(#fw1)"/>
<path class="ln-acc" d="M568 150 L492 150" marker-end="url(#fw1)"/>
<rect class="acc" x="570" y="60" width="180" height="120" rx="6"/><text class="t b" x="660" y="85" text-anchor="middle">fwd</text><text class="m" x="660" y="105" text-anchor="middle">configuration, profiles</text><text class="m" x="660" y="122" text-anchor="middle">permanent decisions</text><text class="m" x="660" y="139" text-anchor="middle">/config/firewall.toml</text><text class="m" x="660" y="156" text-anchor="middle">(FS session)</text>
<path class="ln" d="M660 180 L660 228" marker-end="url(#fw1)"/>
<rect class="warn" x="570" y="230" width="180" height="55" rx="6"/><text class="t b" x="660" y="253" text-anchor="middle">user agent</text><text class="m" x="660" y="271" text-anchor="middle">UI: "Allow?", network profile</text>
<path class="ln" d="M385 210 L385 248" marker-end="url(#fw1)"/>
<rect class="box" x="280" y="250" width="210" height="40" rx="6"/><text class="t b" x="385" y="275" text-anchor="middle">card (frames)</text>
</svg>
<figcaption>netd decides about every request and packet, fwd keeps the configuration and asks the user.</figcaption>
</figure>

## Application manifest

Init starts programs according to the manifest `<program>.manifest.toml`
that lies in initfs next to the program. It will later move into the
package without a format change (see
[Applications](architecture.md#applications-install-update-uninstall-proposal)).

```toml
[app]
id = "org.magnumopus.netprobe"   # required: a-z A-Z 0-9 . - _
name = "Network probe"           # for questions to the user
static_tls_reserve = "8K"        # static TLS reserve for dlopen (default 2K, at most 64K)

[limits]                         # job budget (no section = no limit)
memory = "16M"                   # user memory
kernel_memory = "4M"             # kernel memory

[network]                        # no section = no network
rights = ["info", "tcp", "udp", "dns", "ping"]
ask = true                       # target outside rules: ask the user
sockets = 32                     # most sockets at once
buffers = "4M"                   # most socket buffer memory
ops = 32                         # most concurrent session requests (1 to 1024, default 32; more wait)

[[network.connect]]
proto = ["udp"]                  # tcp, udp, icmp (default all)
to = "10.0.2.3"                  # "*", network "10.0.0.0/8", address, name, "*.name"
ports = "53"                     # "80-90", "*" (default all)

[[network.listen]]
ports = "8080"
```

- **Job:** the program runs in its own job with limits from `[limits]`
  (see [Jobs](syscalls.md#jobs-resource-budgets)). A program without a
  manifest runs in a sub-job without a limit and gets no network.
- **Network session:** init creates a narrower one from the root session
  (`RESTRICT`) with the rights, rules, application identity, the `ask`
  flag and a socket budget. The program gets only its channel.
- **Rights** are named `info`, `tcp`, `listen`, `udp`, `dns`, `ping`,
  `restrict`, `configure`, `audit` (see [Permissions](network.md#permissions)).

## Asking the user

The [query budget](architecture.md#security) applies: **everyday traffic does
not ask**. An outgoing connection allowed by the application manifest goes
through without a prompt (a prompt arises only for an **unexpected target**)
and launching an application never asks about the network.

A session layer with the `ask` flag is not a hard boundary: the user
decides about a target outside its rules. A layer without `ask` is hard,
so a parent can lock a child in and the user cannot override it.

<figure class="diagram">
<svg viewBox="0 0 760 210" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Question flow">
<defs><marker id="fw2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="130" height="170" rx="6"/><text class="t b" x="75" y="45" text-anchor="middle">application</text>
<rect class="ok" x="215" y="20" width="130" height="170" rx="6"/><text class="t b" x="280" y="45" text-anchor="middle">netd</text>
<rect class="acc" x="420" y="20" width="130" height="170" rx="6"/><text class="t b" x="485" y="45" text-anchor="middle">fwd</text>
<rect class="warn" x="625" y="20" width="125" height="170" rx="6"/><text class="t b" x="687" y="45" text-anchor="middle">agent</text>
<path class="ln" d="M140 70 L213 70" marker-end="url(#fw2)"/><text class="m" x="145" y="64">TCP_CONNECT</text>
<path class="ln" d="M345 90 L418 90" marker-end="url(#fw2)"/><text class="m" x="352" y="84">ASK</text>
<path class="ln" d="M550 110 L623 110" marker-end="url(#fw2)"/><text class="m" x="556" y="104">QUESTION</text>
<path class="ln" d="M623 135 L552 135" marker-end="url(#fw2)"/><text class="m" x="560" y="129">DECIDE</text>
<path class="ln" d="M418 155 L347 155" marker-end="url(#fw2)"/><text class="m" x="352" y="149">CONFIG, ANSWER</text>
<path class="ln" d="M213 175 L142 175" marker-end="url(#fw2)"/><text class="m" x="150" y="169">connection / ACCESS_DENIED</text>
</svg>
<figcaption>The request waits on the application's channel; other sockets and applications keep running.</figcaption>
</figure>

| Choice (`fw::choice`) | Result |
|---|---|
| `DENY_ONCE` | this request gets `ACCESS_DENIED`, the next one asks again |
| `ALLOW_ONCE` | only this request (for UDP this socket and target until it closes) |
| `ALLOW_ALWAYS` | fwd stores an `allow` rule for the application, further requests do not ask |
| `DENY_ALWAYS` | fwd stores a `deny` rule, further requests get `ACCESS_DENIED` right away |

- **A permanent rule** has the protocol and port of the question and a
  target. The target is the host name when the session got the address
  for it from the system resolver, otherwise the exact address. A deny
  wins over an allow.
- **Without an agent**, or after `ask_timeout` (default 60 s),
  `ask_default` applies (default `deny`). Without the `fwd` service netd
  denies the question right away.
- **Inbound traffic is not offered:** a datagram from a sender outside
  the rules is dropped without a question.
- A question concerns a TCP connection, a UDP datagram, a ping and a DNS
  query to an own server. The audit gets `ASK_ALLOWED` or `ASK_DENIED`.

## Configuration

Init hands the default configuration (`user/fwd/firewall.toml`) from
initfs to `fwd`. On the first start `fwd` stores it on the FS as
`/system/config/firewall/firewall.toml` and from then on reads and writes
only there. `fwd` gets an FS session with a single mounted path
`/config`, so it sees nothing else on the disk.

```toml
[firewall]
ask_default = "deny"        # without an agent / after the timeout: deny | allow
ask_timeout = 60            # s
unknown_network = "public"  # profile of an unknown network
audit_entries = 4096

[profiles.public]
inbound = "deny"            # deny | listeners
ping = false
stealth = true

[profiles.home]
inbound = "listeners"
ping = true
stealth = false
[[profiles.home.allow_in]]  # exceptions, also in deny mode
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

[[apps]]                    # user decisions (written by fwd)
id = "org.magnumopus.netprobe"
[[apps.rule]]
action = "allow"
proto = ["tcp"]
to = "api.example.com"
ports = "443"

[[networks]]                # known networks (written by fwd)
id = "52:54:00:12:35:02"
gateway = "10.0.2.2"
profile = "home"
```

- **Saving is conditional on the file revision** (`save_if`). When
  somebody else changed the file meanwhile (the user in an editor), `fwd`
  reloads it, adds its change to the new version and saves again, so
  foreign edits are not lost.
- `fwd` does not take an invalid file: at start it uses the default
  configuration and a new configuration from the agent (`CONFIG_SET`)
  ends with `INVALID_ARGS`.

## Network profiles

`netd` recognizes the network by the **gateway MAC** from an ARP reply and
announces it to `fwd` (`NETWORK`) whenever it changes. `fwd` finds the
profile of a stored network. An unknown network gets `unknown_network`
(default `public`) and `fwd` asks the agent (`NETWORK_QUESTION`). It
stores the `CLASSIFY` answer in `[[networks]]`. Until `netd` gets a
configuration, the public profile applies.

| Profile (default) | Inbound connections | Ping | Stealth | Unauthenticated DNS and NTP |
|---|---|---|---|---|
| `public` | none | no | yes | `never` |
| `home` | to ports somebody listens on | yes | no | `fallback` |
| `work` | none (only `allow_in`) | yes | yes | `fallback` |

`unauthenticated` (in `[profiles.*]`) says what to do when the
authenticated path fails (encrypted DNS in `dnsd`, NTS in `ntpd` - the
network blocks the ports): `never` = nothing, `fallback` = the
unauthenticated DHCP resolver and unauthenticated NTP. On a public network
a fallback would let an attacker on the same Wi-Fi spoof answers. `netd`
gets the rule with the profile and reports it in `Net::info()`
(`unauthenticated_fallback`), where `ntpd` reads it.

⚠️ **A gateway MAC can be spoofed.** Somebody on the same network (public
Wi-Fi) can pretend to have the MAC of your home router. The system would
then consider the network home and open listening ports. The gateway MAC
is therefore only a hint. Countermeasures:

- keep the default `public`,
- allow only what is needed in the home profile,
- later verify the network with a stronger identity: WPA2/3 Enterprise,
  the server certificate, user confirmation on every change.

## Packet filter

The logic is in `libs/netfilter` (tested on the host) and `netd` calls it
for every received frame before handing it to smoltcp.

- **A new inbound TCP connection** (SYN without ACK) and **a UDP
  datagram** outside tracked flows: the `deny` profile passes them only
  by `allow_in`, the `listeners` profile also to ports somebody listens on
  (TCP) or receives on (UDP).
- **Tracked UDP flows:** a reply to a datagram we sent (local port +
  remote address and port, valid 120 s) passes without a rule. This is
  how DNS, NTP and other clients work even on a public network. At most
  65,536 flows are tracked; the oldest drop out.
- **Ping** (ICMP echo to us) by profile. Other ICMP (replies, errors)
  always passes.
- **Blocked networks:** nothing in or out, not even for the root session.
- **Rate limits** of SYN and ICMP echo per source (token bucket, at most
  4096 tracked sources) and in total (ten times that).
- **Stealth:** a filtered packet is dropped silently. Without stealth the
  stack gets a SYN to a port nobody listens on and answers RST like a
  closed port. A filtered packet to a port with a listener never passes.
- DHCP (the server's reply to port 68) always passes.

⚠️ **IPv4 only.** The stack cannot do IPv6 yet and drops IPv6 frames.
When IPv6 arrives, it must arrive in the filter too. Further IPv4
fragments (without a protocol header) pass the filter, because the stack
does not reassemble and drops them anyway.

## DNS filter

The resolver does not translate a name from `block.domains` and returns
`ACCESS_DENIED`. The application thus learns that the name is blocked,
not that it does not exist. The pattern `ads.example` blocks the name and
its subdomains, `*.ads.example` only subdomains. This applies to the
system resolver and to a query to an own server. The audit gets
`DNS_BLOCKED`.

⚠️ **DNS is not verified** (DNSSEC, DoT, DoH are missing). A host name
rule (a permanent user decision too) applies to addresses from the system
resolver. Whoever controls the network can translate the name to their
address, and it then passes the rule.

## Audit and traffic

> The network audit will be one of the event sources for endpoint monitoring on
> a computer managed by an organization: [Organization management](organization-management.md#endpoint-monitoring-edr) **[designed]**.

`netd` keeps a ring of records (`audit_entries`, default 4096). A session
with the `AUDIT` right reads them with `Net::audit(from, max)`.

| Kind (`fw::audit`) | When |
|---|---|
| `ALLOWED` | connection, first datagram to a target, ping allowed by rules |
| `DENIED` | denied by session rules or a permanent deny |
| `ASK_ALLOWED` / `ASK_DENIED` | user decision (or timeout) |
| `LISTEN` | listening on a port |
| `INBOUND_DROP` | the filter dropped an inbound packet (`detail` = reason; the same source and reason at most once per 10 s) |
| `DNS_BLOCKED` | translation of a blocked name |
| `BLOCKED_NET` | target in a blocked network |
| `PROFILE` | network profile change |

`Net::stats()` returns the active profile and traffic per application:
bytes sent and received and the number of sockets. Sessions without an
application identity count under `-`.

```rust
let (records, next) = root.audit(0, 256)?;
for r in &records {
    println!("{} {} {:?} {}", r.seq, r.app, r.remote, r.detail);
}
let (profile, apps) = root.stats()?;
```

## Protocol

The messages are in `netproto::fw`. Init creates the `netd` ↔ `fwd`
channel. Messages on it have no replies, each stands on its own.

| Message | Direction | Contents |
|---|---|---|
| `ASK` (100) | netd → fwd | id, application, protocol, target, host name |
| `NETWORK` (101) | netd → fwd | gateway, gateway MAC |
| `ANSWER` (110) | fwd → netd | id, allow (this request only) |
| `CONFIG` (111) | fwd → netd | `FwConfig`: profile, filter, blocked domains, user decisions, audit capacity |
| `QUESTION` (120) | fwd → agent | id, application, application name, protocol, target, host name |
| `DECIDE` (121) | agent → fwd | id, choice |
| `NETWORK_QUESTION` (122) | fwd → agent | id, network id, gateway |
| `CLASSIFY` (123) | agent → fwd | id, profile |
| `CONFIG_GET` (130) | agent → fwd | → TOML configuration |
| `CONFIG_SET` (131) | agent → fwd | TOML configuration → () |
| `PROFILE_SET` (132) | agent → fwd | profile of the current network → () |

`fwd` answers the agent's requests (`CONFIG_*`, `PROFILE_SET`) in the
`netproto::reply` format. A reply starts with a status ≤ 0, a message
from `fwd` with an operation code > 0, so the agent can tell them apart.

⚠️ **The agent must be a trusted UI.** Whoever holds the agent channel
decides for the user. Today init holds it (tests). Only the system UI of
the session with secure input should get it, never an application.
Otherwise an application could allow itself anything.

## Tests

`user/init/src/tests/firewall.rs` (init plays the user agent, the
`netprobe` application has a manifest with `ask = true`):

| Test | Verifies |
|---|---|
| network | `NETWORK_QUESTION` after the gateway ARP, unknown network = `public`, `CLASSIFY home` stores the network and switches the profile |
| manifest | the application runs in a job with limits from the manifest, a manifest target without a question |
| questions | deny once, allow once (asks again next time), allow always (no question, stored in the configuration and on the FS), deny always |
| timeout | without an answer `ask_default` applies after `ask_timeout`, a late answer changes nothing |
| blocking | domain and subdomain `ACCESS_DENIED`, blocked network even for the root session |
| inbound | the public profile drops a connection from the host via `hostfwd`, the home one passes it |
| audit | all record kinds, growing order, application traffic, nothing without the `AUDIT` right |

The host tests the filter, configuration and manifest logic too
(`cargo test -p netfilter -p conf -p manifest`).

## Plan

- **The agent UI** in the session (an "Allow?" dialog with the
  application name from the manifest, an overview of audit and traffic).
- **IPv6** in the filter once the stack supports it.
- **A stronger network identity** than the gateway MAC (see above).
- **Launch gate and antivirus** (postponed): verifying a program
  signature before it starts, with a more lenient mode by setting.
  Scanning files in isolated processes without network access.
