# Reálný čas

Systém má dvoje hodiny:

| Hodiny | Volání | Na co | Vlastnosti |
|---|---|---|---|
| monotónní | `CLOCK_MONOTONIC`, `libos::time::now()` | měření, termíny, timeouty | ns od startu jádra, nikdy necouvne, nemůže selhat |
| reálné („na zdi“) | `CLOCK_REALTIME`, `libos::time::wall()` | časy souborů, zobrazení, logy | sekundy + ns od 1970 UTC, **může skočit** (nastavení hodin) |

Termín čekání se vždy počítá z monotónních hodin. Reálný čas se mezitím
může posunout třeba o hodinu (oprava podle sítě) a timeout by pak vypršel
okamžitě nebo nikdy.

## Odkud jádro reálný čas bere

<figure class="diagram">
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Zdroje realneho casu">
<defs><marker id="tm1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="200" height="70" rx="6"/><text class="t b" x="110" y="45" text-anchor="middle">loader (UEFI)</text><text class="m" x="110" y="64" text-anchor="middle">GetTime + TSC těsně před</text><text class="m" x="110" y="80" text-anchor="middle">ukončením boot services</text>
<path class="ln" d="M210 55 L278 55" marker-end="url(#tm1)"/>
<rect class="acc" x="280" y="20" width="220" height="70" rx="6"/><text class="t b" x="390" y="45" text-anchor="middle">BootInfo.firmware_time</text><text class="m" x="390" y="64" text-anchor="middle">sekundy od 1970, ns, pásmo,</text><text class="m" x="390" y="80" text-anchor="middle">TSC v okamžiku čtení</text>
<path class="ln" d="M500 55 L568 90" marker-end="url(#tm1)"/>
<rect class="box" x="10" y="140" width="200" height="70" rx="6"/><text class="t b" x="110" y="165" text-anchor="middle">CMOS RTC (porty 70h/71h)</text><text class="m" x="110" y="184" text-anchor="middle">záloha, když UEFI čas nedal;</text><text class="m" x="110" y="200" text-anchor="middle">století z ACPI FADT</text>
<path class="ln" d="M210 175 L568 125" marker-end="url(#tm1)"/>
<rect class="ok" x="570" y="70" width="180" height="80" rx="6"/><text class="t b" x="660" y="95" text-anchor="middle">jádro: počátek</text><text class="m" x="660" y="114" text-anchor="middle">reálný čas v okamžiku</text><text class="m" x="660" y="130" text-anchor="middle">0 monotónních hodin</text>
<text class="m" x="10" y="238">CLOCK_REALTIME = počátek + CLOCK_MONOTONIC. Posun mezi čtením v loaderu a startem hodin jádra se dopočítá z TSC.</text>
</svg>
<figcaption>Zdroj času při startu: UEFI, jinak CMOS RTC, jinak je čas neznámý.</figcaption>
</figure>

1. **UEFI `GetTime`.** Loader přečte čas těsně před `ExitBootServices`
   a zároveň hodnotu TSC. Jádro po kalibraci TSC dopočítá, kolik času
   uběhlo do startu jeho hodin, a uloží počátek.
2. **CMOS RTC.** Když firmware čas nedal (chyba, neplatné datum), jádro
   přečte hodiny MC146818 samo:
   - Čte dvakrát po sobě, dokud se čtení neshodují. Během aktualizace by
     se jinak smíchaly dvě sekundy.
   - Umí BCD i binární zápis a 12- i 24hodinový režim.
   - Století bere z registru uvedeného v ACPI FADT, jinak předpokládá 20xx.
   - Když FADT hlásí, že RTC chybí, hodiny se nečtou.
3. **Nic.** `CLOCK_REALTIME` vrací `NOT_SUPPORTED` a `fsd` použije čas od
   startu. Soubory pak vypadají jako z roku 1970.

Log při startu ukazuje výsledek:

```text
loader: cas firmwaru: 2026-11-07 14:03:12 (pasmo neuvedeno, beru jako UTC)
kernel: realny cas: 2026-11-07 14:03:13 UTC (zdroj UEFI, pasmo neuvedeno - bran jako UTC)
```

⚠️ **Časové pásmo:** hodiny PC pásmo neznají a UEFI ho téměř vždy hlásí
jako „neuvedeno“. Jádro pak čas bere jako **UTC** a nastaví příznak
`ZONE_ASSUMED_UTC`.

Linux má v RTC obvykle UTC, **Windows místní čas**. Na počítači
s Windows bude reálný čas posunutý o pásmo (v ČR o 1 až 2 hodiny), dokud
nepřijde nastavení pásma a synchronizace podle sítě.

Když firmware pásmo uvede, platí UTC = místní − pásmo. To je konvence
UEFI 2.x; jiné verze specifikace ji uvádějí s opačným znaménkem.

⚠️ **Přesnost:** firmware i RTC mají rozlišení 1 s, počátek je tedy přesný
na sekundu. Zpřesnění a nastavování hodin přijde se sítí (zdroj
`source::SET`). Do té doby `CLOCK_REALTIME` nejde nastavit.

## Formát

```rust
pub struct WallTime {
    pub secs: i64,  // sekundy od 1970-01-01 00:00:00 UTC, záporné = dřív
    pub nanos: u32, // 0..1_000_000_000
}
```

- **Rozsah:** ±292 miliard let, tedy i data před rokem 1970 a daleko za
  rokem 2262 (hranice u64 nanosekund).
- **Přestupné sekundy:** nepočítají se (jako v POSIX), každý den má
  86 400 s.
- **Převod na kalendář:** proleptický gregoriánský kalendář přes `to_civil`
  a `from_civil` v `abi::time` (algoritmus „days from civil“). Testy
  pokrývají vybrané roky od −4713 do 10⁹ a každý sedmý den v rozsahu
  ±2 000 let.

```rust
use libos::time;

if let Some(t) = time::wall() {
    let c = t.to_civil();
    println!("{:04}-{:02}-{:02} {:02}:{:02} UTC", c.year, c.month, c.day, c.hour, c.minute);
}
let r = time::wall_reading()?;            // i se zdrojem a příznaky
if r.flags & time::flags::ZONE_ASSUMED_UTC != 0 { /* pásmo jen odhadnuté */ }
```

## Časy v souborovém systému

`fsd` nastavuje čas změn ve FS z reálných hodin. Dnešní formát inode
ukládá časy jako u64 nanosekund od 1970. Čas před rokem 1970 se proto
ořízne na 1970 a formát končí v roce 2262.

Změna formátu časů ve FS je samostatný krok. Má umožnit data před rokem
1970 a odstranit hranici 2262, například pro kopii ze starého archivu nebo
z NTFS. Viz [Souborový systém](souborovy-system.md).
