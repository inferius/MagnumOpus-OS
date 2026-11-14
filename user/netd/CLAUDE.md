# user/netd

Služba sítě (TCP/IP, DHCP, DNS klient, firewall napojení). Start (kind USER): 1) kanál k ovladači síťové karty (`netproto::link`), 2) kanál kořenové relace (vše; drží ho init a z něj vytváří užší relace pro programy přes `op::RESTRICT`), 3) volitelně kanál ke službě firewallu `fwd` (bez něj platí veřejný profil a dotazy se odmítají). Pak čeká na `INFO` od ovladače, odpoví rodiči `ok` nebo `err:<popis>` a spustí DHCP. Obsluhuje relace a sockety (protokol `netproto`). Popis: `docs/cs/sit.md`, firewall `docs/cs/firewall.md`.

## Klíčové soubory

- `src/main.rs` (smyčka, port, sloty), `ops.rs` (dotazy na kanálu: číslo, termín, `CANCEL`, fronty směrů), `obj.rs` (objekty, `Pending`, `Op`), `stack.rs` (zásobník), `tcp.rs`, `udp.rs`, `resolve.rs` (DNS), `fw.rs` (filtr a dotazy uživateli), `session.rs`, `forward.rs`, `link.rs` (ovladač karty), `ping.rs`.
- Knihovny: `libs/netproto` (protokol, oprávnění), `libs/netfilter` (paketový filtr), `libs/dns`, `libs/nic`.

## Invarianty

- Program dostane jen kanál užší relace s omezeným oprávněním (`op::RESTRICT`, viz `docs/cs/sit.md#oprávnění`); nikdy kořenovou relaci ani kanál ke kartě.
- Aplikace nemluví s ovladačem sítě přímo, jen se službou.
- Data z ovladače a ze sítě jsou nedůvěryhodný vstup.
- Protokol netproto v2: každý dotaz má v hlavičce číslo a absolutní termín, odpověď nese číslo. Kanál se čte pořád (i s rozpracovanými dotazy). Na socketu je v každém směru (`obj::Dir`) rozpracovaný nejvýš jeden dotaz a další čekají ve frontě FIFO (nejvýš `DIRECTION_QUEUE`), aby se data neproložila. Na relaci platí limit `Quota::ops` (`OpTicket`); další dotazy se odloží (`ops::SESSION_BACKLOG`) a s plným odkladem se kanál nečte (protitlak), `CANCEL` platí i pro odložené. Termín 0 (`TRY`) = neblokující: místo `TIMED_OUT` vrací `SHOULD_WAIT` (`stack::expired`). Zrušení (`CANCEL`, zavření kanálu) musí uvolnit všechno, co dotaz drží: `obj::cancel_pending`.
- Čas dotazu na uživatele se do termínu operace nepočítá (`Deferred::extend`).
- Filtr příchozího provozu (`libs/netfilter`) je jediná cesta: `netd` je jediný síťový zásobník, každý přijatý rámec projde `Filter::inbound`. Změna pravidel = test v `libs/netfilter` i v sadě `firewall`.

## Testy

- Host: `cargo test -p netproto -p netfilter`. QEMU: `cargo xtask test --only sit,firewall` (síť vyžaduje pomocné servery, které xtask spouští sám; `--net-online` navíc testuje skutečnou síť).

## Pasti

- Testy sítě používají user-mode síť QEMU (slirp, host 10.0.2.15, brána 10.0.2.2); předpoklady o adresách drž v souladu s `docs/cs/sestaveni.md`.
- Nový dotaz s čekáním = varianta `Pending` + větev v `obj::cancel_pending` (jinak zrušení nechá viset socket nebo port) + `wake_at`.
- Změna `netproto` zasahuje `netd`, `dnsd`, `ntpd`, `fetch` (klient `netproto`), ovladače karet a `libs/httpc`/`libs/tls`.
