# Převzaté crates se změnami / Vendored crates with changes

Crates z crates.io, které potřebují změnu pro MagnumOpus. Hlavní workspace
je bere přes `[patch.crates-io]` v kořenovém `Cargo.toml`. Každá změna je
v kódu označená komentářem `MagnumOpus:`. Licence a verze: `THIRD-PARTY.md`.

Crates from crates.io that need a change for MagnumOpus. The main workspace
uses them through `[patch.crates-io]` in the root `Cargo.toml`. Every change
is marked in the code with a `MagnumOpus:` comment. Licences and versions:
`THIRD-PARTY.md`.

| Crate | Změny / Changes |
|---|---|
| `getrandom-0.4.3` | backend `target_os = "magnumopus"` (syscall `RANDOM_GET` s `REQUIRE_SEEDED`); stejná záplata jako `third_party/ports/getrandom-0.4.3.patch`. Odstraněny `benches/`, `tests/`. |
| `sunset-0.6.0` | (1) ověření podpisu klíče aplikací: `Runner::set_external_pubkey_verify`, `ServPubkeyAuth::parts` (blob klíče, podepsaná zpráva RFC 4252, blob podpisu), `has_signature`; podpis pak ověří služba účtů mimo proces spojení. (2) typ `sk-ssh-ed25519@openssh.com` (`PubKey::SkEd25519`, `Signature::SkEd25519`): jen rozbor a kódování, sunset ho sám neověřuje. (3) server: `ServPtyRequest::size` (sloupce, řádky, `TERM`) a událost `ServEvent::SessionWinch` (`window-change`). (4) server: `Runner::exit_status` (kód ukončení příkazu). (6) `Runner::close_channel`: server kanál zavře sám (EOF a CLOSE; sunset dřív zavíral jen v odpověď klientovi). (5) oprava chyby: `Runner::output` kopíroval do celého `buf` místo `buf[..l]` (panika s větším bufferem; kandidát na hlášení autorovi). Odstraněny `rust-toolchain.toml`, `Cargo.lock`, `testing/`. / (1) key signature verified by the application: `Runner::set_external_pubkey_verify`, `ServPubkeyAuth::parts`, `has_signature`; the account service verifies it outside the connection process. (2) the `sk-ssh-ed25519@openssh.com` type: parsing and encoding only. (3) server: `ServPtyRequest::size` and the `ServEvent::SessionWinch` event. |
| `getrandom-0.4.3` (lints) | `check-cfg` zná `target_os = "magnumopus"` (bez varování při překladu na hostiteli). |
