# user/nettools

Síťové příkazy shellu `ping`, `resolve` a `nc` v jednom programu se std; chování podle jména (`argv[0]`). xtask ho dá do initfs pod každým jménem (`stdsys::STD_ALIASES`) s vlastním manifestem (`ping.manifest.toml`, `nc.manifest.toml`, `resolve.manifest.toml`). Popis: `docs/cs/sit.md`, kapitola Síťové příkazy shellu.

## Invarianty

- Každý příkaz má vlastní id aplikace a jen potřebná práva sítě; síť dostane jen přes službu exec initu (shell sám síť nemá).
- Přepínače anglicky, hlášení česky. Kód: 0 úspěch, 1 cíl neodpověděl nebo spojení selhalo, 2 špatné použití nebo bez sítě.
- `ping` jde přes `Net::ping` z `std::os::magnumopus::runtime` (std nemá ICMP); `#![feature(magnumopus_runtime, rustc_private)]`.

## Testy

- QEMU: `cargo xtask test --only ucty` (test `ucty: programy ze shellu (sit podle manifestu)`, část `nettools`: ping na bránu QEMU, resolve, nc TCP/UDP proti echo serverům xtasku, zavřený port).
