# user/netd

Služba sítě (TCP/IP, DHCP, DNS klient, firewall napojení). Start (kind USER): 1) kanál k ovladači síťové karty (`netproto::link`), 2) kanál kořenové relace (vše; drží ho init a z něj vytváří užší relace pro programy přes `op::RESTRICT`), 3) volitelně kanál ke službě firewallu `fwd` (bez něj platí veřejný profil a dotazy se odmítají). Pak čeká na `INFO` od ovladače, odpoví rodiči `ok` nebo `err:<popis>` a spustí DHCP. Obsluhuje relace a sockety (protokol `netproto`). Popis: `docs/cs/sit.md`, firewall `docs/cs/firewall.md`.

## Klíčové soubory

- `src/main.rs`, `stack.rs` (zásobník), `tcp.rs`, `udp.rs`, `resolve.rs` (DNS), `fw.rs` (filtr a dotazy uživateli), `session.rs`, `forward.rs`, `link.rs` (ovladač karty), `ping.rs`.
- Knihovny: `libs/netproto` (protokol, oprávnění), `libs/netfilter` (paketový filtr), `libs/dns`, `libs/nic`.

## Invarianty

- Program dostane jen kanál užší relace s omezeným oprávněním (`op::RESTRICT`, viz `docs/cs/sit.md#oprávnění`); nikdy kořenovou relaci ani kanál ke kartě.
- Aplikace nemluví s ovladačem sítě přímo, jen se službou.
- Data z ovladače a ze sítě jsou nedůvěryhodný vstup.
- Filtr příchozího provozu (`libs/netfilter`) je jediná cesta: `netd` je jediný síťový zásobník, každý přijatý rámec projde `Filter::inbound`. Změna pravidel = test v `libs/netfilter` i v sadě `firewall`.

## Testy

- Host: `cargo test -p netproto -p netfilter`. QEMU: `cargo xtask test --only sit,firewall` (síť vyžaduje pomocné servery, které xtask spouští sám; `--net-online` navíc testuje skutečnou síť).

## Pasti

- Testy sítě používají user-mode síť QEMU (slirp, host 10.0.2.15, brána 10.0.2.2); předpoklady o adresách drž v souladu s `docs/cs/sestaveni.md`.
- Změna `netproto` zasahuje `netd`, `dnsd`, `ntpd`, `fetch` (klient `netproto`), ovladače karet a `libs/httpc`/`libs/tls`.
