# Plánovač: QoS, spravedlivé dělení a heterogenní CPU

Plánovač rozhoduje, které vlákno poběží na kterém CPU a jak dlouho. Cíl
systému je **svižné GUI** (kompozitor a vstup reagují hned i pod plnou
zátěží), **žádné hladovění** (úlohy na pozadí vždy postoupí) a **správné
využití hybridních CPU** (výkonná a úsporná jádra).

Tahle stránka popisuje návrh (srovnání přístupů a proč jsme zvolili tenhle),
rozhraní pro přidělování tříd, chování a naměřená čísla. Kód je v
`kernel/src/sched/` (napojení na jádro) a v knihovně `libs/schedcore`
(čistá logika testovatelná na hostiteli).

## Výchozí stav (do verze 1) a co bylo špatně

- Čtyři třídy QoS (`Realtime`, `Interactive`, `Normal`, `Background`)
  v **přísné prioritě**: dokud běží cokoli vyššího, nižší třída nedostane
  nic. Kompozitor s nekonečnou smyčkou (chyba) by zastavil i údržbu disku.
- Uživatelská vlákna vznikala vždy jako `Normal` a žádný syscall třídu
  neměnil - kompozitor neměl před aplikacemi přednost.
- Kvantum 10 ms pro všechny, umístění vláken bez ohledu na typ jádra.

## Srovnání přístupů

| Systém | Jak dělí čas | Přednost interaktivních | Dědění přes IPC | Hybridní CPU |
| --- | --- | --- | --- | --- |
| **Linux 6.6+ (EEVDF)** | Vážený virtuální čas, nejdřívější způsobilý termín; váha z `nice` | Krátký `slice` dává dřív termín; žádné třídy | Jen u futexů (PI); IPC ne | Kapacita CPU (`capacity`), EAS podle energetického modelu |
| **macOS (QoS + Mach)** | Priority s časovým rozpadem, pásma podle QoS | 5 tříd QoS (user-interactive ... background) | **Ano**: zpráva nese voucher s QoS, příjemce dostane override | P/E jádra: QoS rozhoduje o jádru (background jen na E) |
| **Fuchsia / Zircon** | Spravedlivé vážené (fair) + deadline profily | Váhy a deadline profily přidělené rolí | Profilem přes kanály (inheritance na portech a futexech) | Kapacitní umístění, jednoduché |
| **Windows** | 32 priorit, přísná priorita + dočasné zvednutí | Boost po vstupu a pro okno v popředí | Ne (jen ALPC částečně) | Thread Director (HFI): třídy práce a EcoQoS |

Co si odnášíme:

- **Přísná priorita vždy hrozí hladověním** (Windows ho řeší „balance set
  managerem", který jednou za čas zvedne hladovějící vlákno - nepředvídatelné).
- **Vážený virtuální čas** (EEVDF, Fuchsia fair) dává matematickou záruku
  podílu a dobré latence pro vlákna, která často spí.
- **Třída jako popis práce** (macOS) je pro aplikace srozumitelnější než
  čísla priorit a dobře se mapuje na typ jádra.
- **Propagace QoS zprávou** (macOS) řeší obrácení priorit u služeb, aniž
  by služby musely cokoli hlásit.

## Zvolené řešení

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Hierarchie planovace">
<defs><marker id="pl1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="280" y="10" width="200" height="50" rx="6"/><text class="t b" x="380" y="32" text-anchor="middle">výběr na CPU</text><text class="m" x="380" y="50" text-anchor="middle">nejdřív Realtime s rozpočtem</text>
<rect class="warn" x="20" y="100" width="190" height="80" rx="6"/><text class="t b" x="115" y="124" text-anchor="middle">Realtime</text><text class="m" x="115" y="144" text-anchor="middle">rozpočet / perioda (CBS)</text><text class="m" x="115" y="161" text-anchor="middle">EDF podle konce periody</text>
<rect class="box" x="300" y="100" width="440" height="80" rx="6"/><text class="t b" x="520" y="124" text-anchor="middle">spravedlivé třídy: vážený virtuální čas tříd</text><text class="m" x="520" y="146" text-anchor="middle">Interactive 15 : Normal 4 : Background 1</text><text class="m" x="520" y="163" text-anchor="middle">vyhrává třída s nejmenším virtuálním časem</text>
<rect class="ok" x="300" y="220" width="135" height="70" rx="6"/><text class="t b" x="367" y="243" text-anchor="middle">Interactive</text><text class="m" x="367" y="262" text-anchor="middle">EEVDF, kvantum 3 ms</text>
<rect class="box" x="452" y="220" width="135" height="70" rx="6"/><text class="t b" x="519" y="243" text-anchor="middle">Normal</text><text class="m" x="519" y="262" text-anchor="middle">EEVDF, kvantum 6 ms</text>
<rect class="box" x="604" y="220" width="135" height="70" rx="6"/><text class="t b" x="671" y="243" text-anchor="middle">Background</text><text class="m" x="671" y="262" text-anchor="middle">EEVDF, kvantum 12 ms</text>
<path class="ln" d="M330 60 L130 98" marker-end="url(#pl1)"/>
<path class="ln" d="M430 60 L520 98" marker-end="url(#pl1)"/>
<path class="ln" d="M440 180 L367 218" marker-end="url(#pl1)"/>
<path class="ln" d="M520 180 L519 218" marker-end="url(#pl1)"/>
<path class="ln" d="M600 180 L671 218" marker-end="url(#pl1)"/>
<path class="ln" d="M115 180 L115 250 L298 250" marker-end="url(#pl1)"/>
<text class="m" x="20" y="205">vyčerpaný rozpočet:</text><text class="m" x="20" y="221">do další periody jako Interactive</text>
<text class="m" x="20" y="315">Background má na každém vytíženém CPU zaručeno aspoň 1/20 času mimo Realtime; Realtime nejvýš 50 % každého CPU.</text>
</svg>
<figcaption>Dvě úrovně: třídy si dělí čas podle vah, vlákna uvnitř třídy podle EEVDF.</figcaption>
</figure>

**Hierarchie (varianta A z návrhu, schválená uživatelem):**

1. **Realtime** se vybírá první, ale jen dokud má **rozpočet** (`budget`
   v každé `period`, server s konstantní šířkou pásma - CBS). Mezi sebou
   EDF (nejdřív ten, komu dřív končí perioda). Když rozpočet vyčerpá,
   **nezastaví se**, jen do začátku další periody běží jako `Interactive` -
   zvuk tak při mírně podhodnoceném rozpočtu nepraská, a přitom nemůže
   zahltit systém. Přijetí (admission): jedno vlákno nejvýš 50 % periody,
   všechna Realtime vlákna dohromady nejvýš 50 % kapacity všech CPU.
2. **Spravedlivé třídy** mají každá svůj **virtuální čas** na každém CPU:
   běh třídy ho posune o `ran * 1024 / váha`. Vybere se neprázdná třída
   s nejmenším virtuálním časem. Váhy **Interactive 15, Normal 4,
   Background 1** - plně vytížené všechny tři třídy dostanou 75 %, 20 %
   a 5 %; Interactive proti samotnému Background 15:1 (94 % a 6 %).
   Třída, která byla prázdná, se zařadí na nejmenší virtuální čas aktivních
   tříd: spaním si **nenašetří** kredit, ale hned po probuzení je na řadě.
3. **Uvnitř třídy EEVDF**: každé vlákno má virtuální čas a termín
   `v + kvantum`; vybere se způsobilé vlákno (`v <= průměr`) s nejdřívějším
   termínem. Kvantum podle třídy: Interactive 3 ms (rychlé střídání),
   Normal 6 ms, Background 12 ms (méně přepnutí = víc propustnosti).

Proč ne jednodušší varianty: ploché EEVDF (Linux) dává podíl **vláknům**,
ne třídám - sto vláken na pozadí by přetlačilo kompozitor. Priorita se
stárnutím (Windows) zachová trhání a slabou záruku.

### Krátké latence probuzení

- Probuzené vlákno se zařadí s nulovým zpožděním (lag) - je hned způsobilé
  a jeho termín je `průměr + kvantum`.
- **Preempce při probuzení**: vyšší třída s menším virtuálním časem než
  běžící třída přeruší běžící vlákno hned (IPI na cílové CPU). Ve stejné
  třídě přeruší, má-li dřívější termín a běžící vlákno už běželo aspoň
  0,5 ms (proti přehazování).
- Vstup: ovladač vstupu, `inputd` i `compd` jsou `Interactive`, takže
  cesta „přerušení -> ovladač -> inputd -> compd" předbíhá Normal
  a Background na každém kroku.

## Přidělování tříd

<figure class="diagram">
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Strop QoS z jobu">
<defs><marker id="pl2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="20" y="20" width="220" height="56" rx="6"/><text class="t b" x="130" y="43" text-anchor="middle">kořenový job (init)</text><text class="m" x="130" y="62" text-anchor="middle">strop Realtime</text>
<rect class="box" x="20" y="120" width="220" height="56" rx="6"/><text class="t b" x="130" y="143" text-anchor="middle">compd, inputd, ovladače</text><text class="m" x="130" y="162" text-anchor="middle">Interactive / Realtime</text>
<rect class="box" x="300" y="120" width="200" height="56" rx="6"/><text class="t b" x="400" y="143" text-anchor="middle">job aplikací</text><text class="m" x="400" y="162" text-anchor="middle">strop Normal</text>
<rect class="ok" x="540" y="120" width="200" height="56" rx="6"/><text class="t b" x="640" y="143" text-anchor="middle">aplikace</text><text class="m" x="640" y="162" text-anchor="middle">Normal nebo Background</text>
<path class="ln" d="M130 76 L130 118" marker-end="url(#pl2)"/>
<path class="ln" d="M200 76 L380 118" marker-end="url(#pl2)"/>
<path class="ln" d="M500 148 L538 148" marker-end="url(#pl2)"/>
<text class="m" x="20" y="215">Účinná třída = nižší z (požadovaná, strop jobu a všech předků); zvednutí přes IPC strop nepřekročí jen u zpráv od vyšší třídy.</text>
</svg>
<figcaption>Strop třídy dědí job od předků; aplikace si sama Realtime nedá.</figcaption>
</figure>

Jeden syscall `SCHED_CONTROL` (číslo v [syscallech](syscally.md)) s operacemi:

| Operace | Handle | Co dělá |
| --- | --- | --- |
| `THREAD_SET` | vlákno (`MANAGE`), 0 = vlastní vlákno | třída vlákna a u Realtime rozpočet/perioda (µs) |
| `PROCESS_SET` | proces (`MANAGE`) | třída všech vláken procesu i nových |
| `JOB_SET_MAX` | job (`WRITE`) | strop třídy jobu; nejvýš strop rodiče |
| `THREAD_GET` | vlákno (`READ`), 0 = vlastní | základní a účinná třída |
| `CPU_INFO` | - | typ jádra, kapacita, čas běhu po třídách |
| `CPU_HINT` | prostředek (`POWER`) | kapacita jádra z ACPI CPPC (`acpid`) |

Pravidla:

- **Strop z jobu**: účinná třída je vždy nejvýš strop jobu procesu
  a všech předků. Žádost nad strop selže (`AccessDenied`); snížení stropu
  se projeví hned u všech vláken (při dalším plánování).
- **Snížit třídu** si vlákno smí vždy (např. údržba FS na `Background`).
- Výběr volby: proč syscall, ne objekt profilu (Fuchsia)? Práva už nesou
  handly a joby; objekt profilu by byl druhá cesta ke stejné kontrole.
  Fuchsia sama od přímých profilů přešla k rolím spravovaným službou.

Přiřazení v initu (`user/init/src/qos.rs`) a devmgr (`qos` v manifestu
ovladače):

| Služba | Třída |
| --- | --- |
| `compd`, `inputd` | Interactive |
| ovladače vstupu (`ps2`, `virtio-input`, `usb-hid`) | Interactive |
| `audiod`, `hda` | Realtime, rozpočet 1 ms / 5 ms |
| údržba FS (vlákno úloh ve `fsd`) | Background |
| job aplikací | strop Normal |

## Dědění přes IPC

<figure class="diagram">
<svg viewBox="0 0 760 200" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Propagace QoS zpravou">
<defs><marker id="pl3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="ok" x="20" y="40" width="190" height="60" rx="6"/><text class="t b" x="115" y="64" text-anchor="middle">compd</text><text class="m" x="115" y="83" text-anchor="middle">Interactive</text>
<rect class="box" x="290" y="40" width="190" height="60" rx="6"/><text class="t b" x="385" y="64" text-anchor="middle">zpráva v kanálu</text><text class="m" x="385" y="83" text-anchor="middle">nese třídu odesílatele</text>
<rect class="acc" x="560" y="40" width="180" height="60" rx="6"/><text class="t b" x="650" y="64" text-anchor="middle">obsluha (fsd)</text><text class="m" x="650" y="83" text-anchor="middle">Normal -> Interactive</text>
<path class="ln" d="M210 70 L288 70" marker-end="url(#pl3)"/>
<path class="ln" d="M480 70 L558 70" marker-end="url(#pl3)"/>
<text class="m" x="20" y="140">Zvednutí: při probuzení čekajícího (vlákno i port) a při přečtení zprávy.</text>
<text class="m" x="20" y="160">Konec: 50 ms od odeslání vlastníkem třídy, nebo přečtení zprávy nižší třídy. Jen zvedá.</text>
<text class="m" x="20" y="180">Řetězí se: obsluha přepošle půjčku dál, ale se stejným koncem platnosti.</text>
</svg>
<figcaption>Propagace QoS zprávou (jako vouchery v macOS).</figcaption>
</figure>

- Každá zpráva nese **půjčku třídy**: třídu a konec platnosti. Vlákno,
  které třídu samo má, posílá konec `teď + 50 ms`; vlákno, které ji má
  jen půjčenou, přepošle **původní konec** (půjčka se předáváním
  neprodlužuje). **[ověřeno v QEMU]**
- Kanál při zápisu zvedne vlákno, které na něm přímo čeká, a port, který
  ho sleduje, si zapamatuje nejvyšší třídu čekajících paketů a zvedne
  vlákno, které si paket vyzvedne.
- Přečtení zprávy nastaví zvednutí vlákna na půjčku zprávy, s jejím koncem
  platnosti (okno obsluhy jednoho požadavku je 50 ms od odeslání
  vlastníkem třídy). Další zpráva zvednutí nahradí (i sníží). Po konci
  platnosti se vlákno vrátí na svou třídu.
- Zvednutí nikdy nesnižuje a strop jobu se na něj **nevztahuje** - je to
  půjčka od odesílatele, ne vlastní nárok (jinak by aplikace s Normal
  stropem obsluhující kompozitor zdržovala snímek). Zvednutí přijde jen od
  vlákna, které tu třídu skutečně má, nebo jako přeposlaná půjčka
  s původním koncem platnosti.
- Realtime se přes IPC propaguje jako **Interactive** (rozpočet je vlastnost
  konkrétního vlákna, nedá se půjčit).

## Heterogenní CPU

- **Zjištění typu jádra** při startu každého CPU:
  - x86_64 Intel hybrid: `CPUID.07H:EDX[15]` (hybrid) a `CPUID.1AH:EAX[31:24]`
    (`0x40` Core = výkonné, `0x20` Atom = úsporné). HFI (`CPUID.06H:EAX[19]`,
    tabulka výkonu v paměti a přerušení teploty) zatím ne - potřebuje
    obsluhu tepelného přerušení; kapacitu může upřesnit `acpid` z CPPC.
  - aarch64: `MIDR_EL1` (výrobce ARM a číslo jádra: A53/A55/A510/A520
    úsporná, A7x/A71x/A72x výkonná, X1-X4/X925 nejvýkonnější). Tabulka
    **PPTT** dává clustery (sdílená cache).
- **Kapacita** (1024 = nejvýkonnější jádro): výkonná 1024, úsporná 512
  (výchozí odhad), přesně z ACPI CPPC `_CPC` přes `CPU_HINT`.
- **Umístění podle třídy**: Realtime a Interactive na nečinné jádro s
  nejvyšší kapacitou, Background na nečinné úsporné jádro (výkonné neprobudí,
  když úsporné nemá volno, zůstane na svém), Normal na poslední CPU kvůli
  cache, jinak nečinné.
- **Vyvažování vytížených CPU**: když po konci kvanta na CPU čeká další
  práce, vystřídané vlákno se pošle na CPU, které bude i po přesunu méně
  vytížené. Zátěž = součet vah tříd (Interactive 15, Normal 4, Background 1)
  dělený kapacitou jádra: dvě Background vlákna nejsou stejná práce jako dvě
  Interactive. Přesun jen tehdy, když cíl bude i po něm méně vytížený (žádné
  skákání tam a zpět). Respektuje typ jádra pro třídu.
- **Přesun až po přepnutí**: vlákno, které ještě běží na svém CPU, nesmí
  být v cizí frontě. Cizí CPU by ho vybralo a čekalo se zakázanými
  přerušeními, až ho původní CPU dopřepne; když si dvě CPU vlákna
  vymění, čekají na sebe navzájem a stojí obě (i jejich časovače). Proto
  se přesun vystřídaného vlákna i zařazení vlákna probuzeného během
  usínání provede až po dokončení přepnutí (`finish_switch`) a kradení
  dobíhající vlákna přeskočí (jako `on_cpu` v Linuxu). Tuto chybu měla
  verze 2 do 2026-11-13 (občas zaseknutý test latence vstupu na
  aarch64). **[implementováno]**
- **Probuzení během přepnutí v jednom slově**: „vlákno ještě běží“
  (`ON_CPU`) a „čeká na něj probuzení“ (`WAKE_PENDING`) jsou příznaky
  jednoho atomického slova vlákna a mění se jen atomickými operacemi
  na něm (CAS v `wake`, `swap` ve `finish_switch`). Dřívější dvojice
  samostatných příznaků (zapiš svůj, přečti cizí) spoléhala na pořadí
  zápis -> čtení mezi dvěma adresami. Skutečný ARM ho u `stlr` + `ldar`
  dodrží, ale QEMU TCG na x86 hostiteli ne. Výsledek: probuzení se
  ztratilo, vlákno zůstalo RUNNABLE mimo všechny fronty a test čekající
  na jeho událost vypršel (aarch64 pod zátěží hostitele, do 2026-11-14).
  Diagnostický výpis při selhání testu (`sched::dump_state`: fronty
  a časovače všech CPU, stav sledovaných vláken) tento stav zachytil.
  Stejně se rozhoduje přerušení běžícího vlákna po změně jeho stavu
  (ukončení, třída): ON_CPU se čte operací RMW na tomtéž slově
  (`on_cpu_after_update`), takže buď odejde IPI, nebo vlákno změnu uvidí.
  **[ověřeno v QEMU]**
- **Vyvažování nečinných**: nečinné CPU si bere práci z cizích front; úsporné jádro
  bere Interactive jen z fronty, kde čekají aspoň dvě vlákna (výkonná jsou
  přetížená). Vystřídané Interactive vlákno na úsporném jádře se přesune
  na nečinné výkonné.
- **Příprava na energetické řízení**: jádro počítá čas běhu po třídách
  na každém CPU (`CPU_INFO`); budoucí regulátor v `acpid` z něj zvolí
  P-stav (`_PSS` nebo CPPC `desired performance`).

Ověření: QEMU neumí hybridní topologii (`-smp` nemá typy jader ani na
x86_64, ani na `virt` aarch64). **Umístění a vyvažování na hybridních CPU
jsou ověřené jen simulací** na hostiteli (`libs/schedcore`, testy
`place_tests.rs` a `sim_tests.rs`); dekódování CPUID 0x1A, MIDR a PPTT jen
host testy nad bajty. V QEMU se ověřuje homogenní cesta (všechna jádra
kapacita 1024, chování jako dřív).

## Měření

Testy v jádře (`kernel/src/tests/sched*.rs`, běží v QEMU s 4 CPU):

- **Latence vstup -> snímek** při plné zátěži pozadí: „ovladač" (Interactive)
  probudí „inputd", ten „compd", ten vykreslí snímek; na všech CPU běží
  smyčky Background a Normal. Měří se čas od události po snímek a porovná
  s referenčním během bez zátěže ve stejném testu (pod TCG závisí absolutní
  čas na hostiteli; test hlídá hrubou chybu, čísla se logují).
- **Podíl Background** pod zátěží Interactive: na každé CPU připadá jedna
  smyčka Interactive a jedna Background. Záruka je 1/16 (62 ‰) při vahách
  15 : 1; pod TCG se běh smyček měří jen přibližně (vytížený hostitel
  přerušuje vCPU), takže test hlídá hladovění (aspoň 15 ‰, starý plánovač
  0 ‰) a opačný extrém (nejvýš 400 ‰), přesné číslo loguje.
- **Dědění přes IPC**: Normal obsluha probuzená zprávou od Interactive běží
  jako Interactive; paket portu nese třídu.
- **Realtime**: přijetí (právě jedno vlákno s 50 % na CPU) a zpoždění
  periody 2 ms pod zátěží Interactive na všech CPU.
- **Mikrobenchmark přepnutí kontextu** (ping-pong přes event).

Čísla jsou v sekci Naměřené hodnoty níže.

### Naměřené hodnoty

QEMU TCG (emulace bez akcelerace), 4 CPU, x86_64; tři běhy testů v jedné
sadě `cargo xtask test`. TCG je pomalé a časování nepřesné - čísla slouží
k porovnání a hlídání regresí, ne jako výkon skutečného hardwaru.

| Měření | Výsledek (3 běhy) |
| --- | --- |
| latence vstup -> snímek pod zátěží 12 smyček Normal a Background, medián | 728-969 µs (z toho 200 µs „kreslení“) |
| tatáž latence, maximum | ~15 ms |
| podíl Background pod zátěží Interactive (záruka 1/16 = 62 ‰) | 60-61 ‰ |
| zpoždění periody Realtime 2 ms pod zátěží Interactive, medián / max | 1,0-1,3 ms / 2,7-4,4 ms |
| přepnutí kontextu (ping-pong přes událost, jedna strana) | 90-110 µs |
| podíl Background ve verzi 1 (přísná priorita) | 0 ‰ (hladoví) |

aarch64 (`virt`, TCG, 4 CPU, tři běhy): latence medián 930-989 µs, max
7,7-14 ms; podíl Background 52-63 ‰; zpoždění periody Realtime medián
1,2-1,8 ms; přepnutí kontextu 84-107 µs.

Srovnání přepnutí kontextu se starým plánovačem se kvůli úspoře času
neměřilo (stejný test na verzi 1 je připravený, jde jen spustit); složitost
výběru je O(n) v počtu vláken jedné třídy na CPU, proti O(1) dřív.

Maximum latence (~15 ms) odpovídá jednomu kvantu Background (12 ms) na
TCG, kde se IPI a časovač doručují se zpožděním; medián je pod 1 ms.

Zpoždění periody Realtime závisí pod TCG na tom, kolik času hostitel
vCPU nedá. Test to měří sám: v práci Realtime vlákna (300 µs za
periodu) ho v hostu nikdo nepředběhne, takže mezery nad 50 µs mezi
čteními hodin jsou čas hostitele. Hranice mediánu je 10 ms / (1 − podíl
hostitele), nejvýš 100 ms. Měření 2026-11-14 (3× x86_64 + 1× aarch64
souběžně, 4 kola): hostitel bral 1-631 ‰, medián zpoždění zůstal
1,2-2,5 ms, maximum rostlo s podílem hostitele až na 30 ms. Jednorázový
medián 28 ms z plné sady tedy odpovídá přetíženému hostiteli, ne chybě
plánování Realtime. Ukradený čas se navíc počítá do rozpočtu Realtime
(jádro ho nerozliší od běhu) a vlákno pak do obnovy běží jako
Interactive; skutečný hardware tento problém nemá. **[ověřeno v QEMU]**

## Známé problémy

Z kontroly návrhu (2026-11-08), k opravě v tomto pořadí:

1. **Řetězení zvednutí bez konce** - opraveno (2026-11-13). Zpráva dřív
   nesla účinnou třídu odesílatele včetně zvednutí a přijetí obnovilo okno
   50 ms, takže dva procesy se stropem Normal si zvednutí udržely
   ping-pongem napořád. Teď zpráva nese půjčku `QosLoan` (třída + konec
   platnosti) a přeposlání konec neprodlouží; test jádra „qos: půjčka se
   ping-pongem neprodlužuje". **[ověřeno v QEMU]**
2. **Férovost mezi joby uvnitř třídy.** Čas třídy se dnes dělí po
   vláknech, aplikace s mnoha vlákny dostane větší podíl. Pro souběžné
   agenty a výpočty: dvoustupňové dělení (mezi joby, pak mezi vlákny
   jobu). **[navrženo]**
3. **Vyčerpaný rozpočet Realtime** omezí jen přednost, ne spotřebu:
   vlákno dál běží jako Interactive a bere čas ze férové části (váha 15).
   Je to záměr (zvuk nepraská), jen to musí být vidět v měření.
4. **Realtime je soft realtime.** Přijetí hlídá součet přes všechna CPU
   (50 %), ne skutečné rozmístění; na hybridním CPU mohou být výkonná
   jádra přetížená, zatímco úsporná jsou volná. Tvrdší záruky by
   potřebovaly přijetí podle kapacity jednotlivých CPU a pevné umístění.
5. **Lineární výběr** nepřepisovat na strom bez měření: nejdřív růst
   režie a nejhorší latence s počtem vláken (s revizí a podmínkami, viz
   [Psaní dokumentace](dokumentace.md#měření)); syntetické testy v QEMU
   hlídají regrese, odezvu GUI ukáže až měření na hardwaru.

## Další kroky

- **HFI** (Intel Hardware Feedback Interface): kapacity jader za běhu
  podle teploty a příkonu - potřebuje obsluhu tepelného přerušení.
- **Regulátor P-stavů** v `acpid` nad `CPU_INFO` (čas běhu po třídách)
  a ACPI `_PSS` / CPPC; jádro data připravuje, regulátor zatím není.
- `acpid` má volat `CPU_HINT` s kapacitou z `_CPC` (zatím odhad z CPUID).
- Vyvážený strom místo lineárního výběru, jen když měření ukáže potřebu
  (viz Známé problémy, bod 5).
- Měření na skutečném hybridním hardwaru (dnes jen simulace).
