# Požadavky na hardware

Co musí počítač umět, aby na něm MagnumOpus běžel. Stránka zatím pokrývá
procesor; ostatní (paměť, firmware, zařízení) přibude s měřeními.

Obsah:

1. [Procesor](#procesor)
2. [Co se stane na starším procesoru](#co-se-stane-na-starším-procesoru)
3. [Proč tahle úroveň](#proč-tahle-úroveň)

## Procesor

Všechny programy (služby, ovladače, aplikace) se překládají pro cíle
`x86_64-unknown-magnumopus` a `aarch64-unknown-magnumopus` s FPU a SIMD
(hard-float, [architektura.md](architektura.md)). Jádro samo FPU
nepoužívá, ale stav FPU každého vlákna ukládá ([jadro.md](jadro.md)).

| Architektura | Základní úroveň | Instrukce navíc proti nejstaršímu CPU dané architektury | Zhruba od |
|---|---|---|---|
| x86_64 | **x86-64-v2** | SSE3, SSSE3, SSE4.1, SSE4.2, POPCNT, CMPXCHG16B, LAHF/SAHF v 64bitovém režimu | Intel Nehalem (2008), AMD Bulldozer (2011) |
| aarch64 | ARMv8-A s FP a AdvSIMD (NEON) | - (FP a NEON jsou na běžných jádrech vždy) | každé běžné ARMv8 |

Vyšší úrovně (AVX, AVX2, AVX-512, AES-NI) programy nepředpokládají;
knihovny, které je umějí využít, je zjistí za běhu (CPUID a XGETBV:
jádro zapne v XCR0 AVX a AVX-512, když je CPU má, a ukládá jejich stav
přes XSAVE, [jadro.md](jadro.md)). AMX zatím zapnuté není.

## Co se stane na starším procesoru

**[implementováno]** Jádro hned po startu (před inicializací CPU,
`check_cpu_baseline` v `kernel/src/main.rs`, `arch::missing_baseline`)
ověří základní úroveň. Když něco chybí, napíše na obrazovku i do logu

```
procesor nesplnuje pozadavky systemu (x86-64-v2): chybi SSE4.1, SSE4.2, POPCNT
```

a zastaví se **bez restartu** (stejný stroj by se jen točil dokola).
V testovacím běhu QEMU skončí s neúspěchem. Bez kontroly by některý
program spadl na neplatné instrukci (`#UD`) náhodně až za běhu.

## Proč tahle úroveň

- **x86-64-v2:** CMPXCHG16B potřebuje vrstva Windows (atomické operace
  nad 16 B), SSE4.2 a POPCNT zrychlují práci s textem a bitovými mapami.
  Vyřadí jen procesory před rokem 2008 (Intel) a 2011 (AMD). Vyšší úroveň
  (v3 = AVX2) by vyřadila ještě běžně používané stroje (např. Intel
  Pentium a Celeron do roku 2020), proto se AVX2 zjišťuje za běhu.
- **aarch64:** FP a NEON jsou na všech běžných jádrech; SVE se zatím
  nepoužívá (běžný hardware ho má zřídka).
- QEMU testy běží s `-cpu max` (x86_64) a `-cpu cortex-a76` (aarch64), takže základní úroveň splňují.
