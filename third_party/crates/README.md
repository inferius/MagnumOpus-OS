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
| `sunset-0.6.0` | zatím beze změny kódu; odstraněny `rust-toolchain.toml`, `Cargo.lock`, `testing/` / no code change yet; removed `rust-toolchain.toml`, `Cargo.lock`, `testing/` |
