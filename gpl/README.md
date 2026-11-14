# gpl/ - ovladače pod GPL

Oddělená část repozitáře pro kód převzatý z projektů pod GPL (Linux, nové
ovladače NVIDIA v Rustu, části `amdgpu`, ovladače Qualcomm pro telefony).
Zbytek repozitáře je `MIT OR Apache-2.0`; **tato složka má vlastní licenci**:
**GPL-2.0-only** (`gpl/LICENSE`).

**Stav: [navrženo]** (rozhodnuto 2026-11-14). Zatím tu není žádný ovladač.
Dosavadní pravidlo „GPL nepřekládat“ platí dál pro všechno mimo `gpl/`
(tam jen fakta o hardwaru). Úplná pravidla zdrojů a licencí:
[`docs/cs/ovladace.md`](../docs/cs/ovladace.md) (kapitola Pravidla zdrojů
a licencí), seznam převzatého kódu: [`THIRD-PARTY.md`](../THIRD-PARTY.md).

## Pravidla

1. **Jen tady.** Kód pod GPL nesmí být v jádře (`kernel/`), `libs/`, službách
   (`user/`), aplikacích ani v loaderu.
2. **Samostatný proces ovladače.** Každý ovladač je vlastní program, který
   se systémem mluví jen **zprávami přes náš veřejný zdokumentovaný
   protokol** (třídy displej, render, síť, zvuk, vstup, ...). Žádné sdílené
   vnitřní struktury, hlavičky, paměť ani linkování s kódem mimo `gpl/`
   kromě níže povolených knihoven.
3. **Směr závislostí.** Ovladač v `gpl/` smí používat naše knihovny pod
   MIT/Apache (`driverkit`, protokolové crate). Opačně nikdy: nic mimo
   `gpl/` na `gpl/` nezávisí (ani jako crate, ani jako kopírovaný kód).
4. **Jedna licence v ovladači.** Nemíchat GPL-2.0-only s GPL-3.0 v jednom
   ovladači. Převzatý kód s jinou licencí (GPL-3.0, AGPL) sem nepatří.
5. **Zdroje jsou součástí distribuce.** Ke každému binárnímu ovladači z
   `gpl/` (v obrazu systému, na USB, v balíčku) se vždy dodají jeho zdroje
   a `gpl/LICENSE`.
6. **Izolace zůstává.** GPL ovladač běží jako každý jiný: omezený Resource,
   IOMMU, manifest (`driver.license = "GPL-2.0-only"`), obnova po pádu.
7. **Zápis.** Každý soubor má hlavičku podle šablony v `THIRD-PARTY.md` a
   každý převzatý zdroj má řádek v jeho tabulce.

## Proč je to v pořádku a kde je riziko

Podle FAQ Free Software Foundation jsou programy komunikující jen přes IPC
(zprávy, roury, sockety) samostatná díla, takže systém s ovladačem z `gpl/`
nepřechází pod GPL. ⚠️ **Tento výklad není soudně vyzkoušen.** Před
komerčním šířením ho nechat posoudit právníkem.

## Jak přidat ovladač

1. Ověř licenci zdroje (GPL-2.0-only nebo GPL-2.0-or-later; ne GPL-3.0 ani
   AGPL) a že existuje protokol třídy, přes kterou bude ovladač mluvit. Když
   protokol chybí, nejdřív ho navrhni a zdokumentuj (cs + en) mimo `gpl/`.
2. Vytvoř `gpl/<jmeno>/` jako vlastní crate (v pracovním prostoru zvlášť
   označený, ať je vidět, co je GPL) s manifestem ovladače
   (`<jmeno>.driver.toml`, `license = "GPL-2.0-only"`, `origin` s
   projektem a verzí).
3. Do hlavičky každého souboru dej šablonu z `THIRD-PARTY.md` (SPDX, původ,
   copyright původních autorů, co se změnilo).
4. Doplň řádek do tabulky v `THIRD-PARTY.md` (sekce Kód pod GPL).
5. Kontrola před sloučením: ovladač nic neimportuje z `kernel/`, `user/`
   ani z jiné než povolené knihovny (`driverkit`, protokoly); žádný jiný
   crate na něj nezávisí; do obrazu systému jdou i zdroje.
6. Dokumentace ovladače cs + en se značkou stavu a poznámkou o GPL.

## Soubory

- `LICENSE` - text GPL-2.0.
- `README.md` - tento soubor.
