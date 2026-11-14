# Vlastní, nebo převzaté

Co chceme psát sami a co převzít hotové. Vlastní kód píšeme tam, kde
tvoří charakter systému: bezpečnostní model, vzhled, chování, souborový
systém. Hotový kód bereme tam, kde jde o jednu dobře vymezenou věc se
standardem a jedinou správnou odpovědí (šifra, kodek, formát obrázku). Psát
ji znovu by bylo vyrábění kola: hodně práce a víc chyb než v kódu, který
léta testují tisíce projektů.

Pravidla pro ovladače (malé překládat do Rustu z BSD/ISC/MIT, velké jako
izolovaný proces v C, GPL jen v odděleném `gpl/`) popisuje
[Ovladače](ovladace.md) (kapitola Pravidla zdrojů a licencí). Převzatý kód
a jeho licence se zapisuje do `THIRD-PARTY.md`.

## Jak rozhodnout

<figure class="diagram">
<svg viewBox="0 0 780 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Rozhodnutí vlastní, nebo převzaté">
<defs><marker id="vp1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="250" height="70" rx="6"/>
<text class="t b" x="135" y="46" text-anchor="middle">Tvoří to charakter systému?</text>
<text class="m" x="135" y="66" text-anchor="middle">bezpečnost, vzhled, chování, FS,</text>
<text class="m" x="135" y="81" text-anchor="middle">model služeb, rozhraní pro lidi</text>
<rect class="box" x="10" y="130" width="250" height="70" rx="6"/>
<text class="t b" x="135" y="156" text-anchor="middle">Je to jedna věc se standardem?</text>
<text class="m" x="135" y="176" text-anchor="middle">šifra, kodek, formát, protokol,</text>
<text class="m" x="135" y="191" text-anchor="middle">Unicode, komprese</text>
<rect class="box" x="10" y="230" width="250" height="60" rx="6"/>
<text class="t b" x="135" y="256" text-anchor="middle">Je kód velký (100k+ řádků)?</text>
<text class="m" x="135" y="276" text-anchor="middle">GPU, Wi-Fi, ACPI, prohlížeč</text>
<rect class="acc" x="440" y="20" width="330" height="70" rx="6"/>
<text class="t b" x="605" y="46" text-anchor="middle">Vlastní</text>
<text class="m" x="605" y="66" text-anchor="middle">v Rustu, podle našeho návrhu;</text>
<text class="m" x="605" y="81" text-anchor="middle">cizí kód jen jako vzor nebo knihovna pod ním</text>
<rect class="ok" x="440" y="130" width="330" height="70" rx="6"/>
<text class="t b" x="605" y="156" text-anchor="middle">Převzít knihovnu v Rustu</text>
<text class="m" x="605" y="176" text-anchor="middle">crate s licencí MIT/Apache/BSD,</text>
<text class="m" x="605" y="191" text-anchor="middle">přednostně no_std; jinak překlad z BSD</text>
<rect class="warn" x="440" y="230" width="330" height="60" rx="6"/>
<text class="t b" x="605" y="256" text-anchor="middle">Převzít C jako izolovaný proces</text>
<text class="m" x="605" y="276" text-anchor="middle">jen vlastní capabilities, pád nic neshodí</text>
<line class="ln-acc" x1="260" y1="55" x2="436" y2="55" marker-end="url(#vp1)"/>
<text class="m" x="348" y="48" text-anchor="middle">ano</text>
<line class="ln" x1="135" y1="90" x2="135" y2="126" marker-end="url(#vp1)"/>
<text class="m" x="150" y="112">ne</text>
<line class="ln" x1="260" y1="165" x2="436" y2="165" marker-end="url(#vp1)"/>
<text class="m" x="348" y="158" text-anchor="middle">ano, malé</text>
<line class="ln" x1="135" y1="200" x2="135" y2="226" marker-end="url(#vp1)"/>
<text class="m" x="150" y="218">ano, velké</text>
<line class="ln" x1="260" y1="260" x2="436" y2="260" marker-end="url(#vp1)"/>
<text class="m" x="348" y="253" text-anchor="middle">ano</text>
</svg>
<figcaption>Vlastní kód tam, kde dělá systém jiným; hotový tam, kde existuje jedna správná odpověď.</figcaption>
</figure>

Další otázky, které rozhodují:

- **Bezpečnost.** Kryptografii nikdy nepíšeme sami. Chyba v ní není vidět
  a nejde ji otestovat běžnými testy.
- **Údržba.** Formáty a kodeky se vyvíjejí (nové profily, opravy
  zranitelností). Převzatý kód aktualizujeme, vlastní bychom museli
  sledovat sami.
- **Napojení na systém.** Když hotová věc předpokládá POSIX, sdílenou
  paměť bez omezení nebo root, je lepší napsat vlastní tenkou vrstvu nad ní,
  ne ji přepsat celou.
- **Licence.** MIT, Apache 2.0, BSD, ISC a zlib bereme. LGPL jen jako
  oddělený proces nebo dynamická knihovna. GPL jen v oddělené složce `gpl/` jako samostatný proces ovladače (viz [Ovladače](ovladace.md#pravidla-zdrojů-a-licencí)), AGPL nebereme.

## Vlastní (teď i do budoucna)

| Oblast | Proč vlastní | Pod tím převzaté |
|---|---|---|
| Jádro, IPC, capabilities, plánovač | jádro bezpečnostního modelu a výkonu; nic hotového nepasuje | - |
| Model služeb, init, supervisor, obnova po pádu | určuje, jak se systém chová při chybě | - |
| Účty, oprávnění, relace, správa bez roota | bezpečnostní model a UX přihlášení | Argon2id, FIDO2/CTAP2, šifry (RustCrypto) |
| Souborový systém (formát, verze, žurnál, koš, trezory) | hlavní odlišnost systému | hashe (XXH3, BLAKE3), komprese (LZ4, zstd) |
| Git vrstva FS | spojení verzí FS s gitem | `gix` (gitoxide) po portu Rust std |
| Kompozitor, knihovna UI, design systém, ikony | vzhled a svižnost jsou naše značka | tiny-skia (vektory), písma Inter a JetBrains Mono |
| Shell, terminál, příkazy, správce disku | denní nástroje, napojené na FS a capabilities | - |
| Systémové aplikace: soubory, nastavení, instalátor, správce úloh | musí sedět s návrhem a s naším FS | dekodéry obrázků, náhledy |
| Vrstva pro agenty AI | vlastní model povolení a pracovních prostorů | - |
| Malé ovladače (síť, disky, USB, zvuk, vstup) | malé, bezpečnostně citlivé, chceme Rust | fakta z datasheetů, překlad z BSD |
| Knihovna C `cstd`, později libc/POSIX vrstva | nad capabilities, ne nad Unixem | - |
| Textový editor | součást systému, ne jen aplikace | `ropey` (buffer), `tree-sitter` (zvýraznění) |
| Vlastní překladač (dlouhodobý cíl) | plná kontrola nad nástroji | do té doby Rust, clang |

## Převzaté (bylo by to vyrábění kola)

| Oblast | Co vzít | Poznámka |
|---|---|---|
| Kryptografie, TLS | RustCrypto, `rustls`, `ring`, `argon2`, `blake3` | nikdy vlastní šifry ani protokol |
| Kořenové certifikáty | seznam Mozilly (`webpki-roots`) | jen data |
| Komprese | `lz4_flex`, `ruzstd`/zstd, `miniz_oxide` (deflate) | formáty jsou pevně dané |
| Hashe | `xxhash-rust`, `blake3`, `sha2` | |
| Unicode | normalizace, segmentace, bidi, zalamování řádků (`unicode-*`, ICU4X) | tabulky se mění s každou verzí Unicode |
| Tvarování textu | `rustybuzz` (HarfBuzz v Rustu) | arabština, indická písma, ligatury |
| Vektorová grafika, SVG | `tiny-skia`, `usvg`/`resvg` | rasterizace cest, gradienty |
| Obrázky | `png`, `zune-jpeg`, `image-webp`, `gif` | dekodéry jsou častý cíl útoků, chceme ty prověřené |
| Zvuk a video | `symphonia`, `opus`, `dav1d`; FFmpeg jen jako oddělený proces | kodeky jsou obří a patentově citlivé |
| PDF | `pdfium` | ne MuPDF (AGPL) |
| Prohlížeč (jádro) | Servo nebo Ladybird | vlastní je jen okno a napojení |
| Windows aplikace | Wine + DXVK | vlastní spodní vrstva nad capabilities |
| Grafika AMD, NVIDIA, novější Intel | ovladače DRM z Linuxu přes LinuxKPI, izolovaně | stovky tisíc řádků |
| Wi-Fi | ovladače z BSD/Linuxu izolovaně, firmware výrobce | |
| ACPI | ACPICA jako proces `acpid` | už v systému |
| Git (objekty, protokol) | `gix` | |
| Regulární výrazy | `regex` | |
| JSON, serializace | `serde`, `serde_json` | |
| Časová pásma | data tzdata (IANA) | jen data, převod si děláme sami (`libs/tz`) |
| Rozložení klávesnic | data z CLDR / xkeyboard-config | jen data |
| Slovníky kontroly pravopisu | Hunspell slovníky | jen data |
| Písma | Inter, JetBrains Mono, Noto pro další písma | |

## Hraniční případy

Věci, které máme vlastní, protože hotové nesedělo (`no_std`, velikost,
napojení), ale dlouhodobě je zvážíme nahradit:

| Vlastní kód | Proč teď vlastní | Kdy zvážit převzetí |
|---|---|---|
| Rasterizer písma (`libs/font`) | malý, `no_std`, stačí pro latinku | hinting a tvarování převzít (`rustybuzz`, `skrifa`), až budou potřeba další písma |
| Čtení konfigurace (`libs/conf`) | podmnožina TOML bez alokací navíc | po portu Rust std zvážit `toml` |
| Zápis FAT (`libs/fatw`) | jen logy na flashku, pár set řádků | když bude potřeba plné čtení a zápis FAT, vzít `fatfs` |
| QR kód (`libs/qr`) | obrazovka pádu, bez alokace | zůstane, hotové crates alokují |
| DNS (`dnsd`) | malé, napojené na `netd` | DNSSEC nebo DoH převzít |

## Jak převzít

1. Ověřit licenci a zapsat ji do `THIRD-PARTY.md`.
2. Přednostně crate `no_std` + `alloc`, jinak počkat na port Rust std
   nebo dát kód do odděleného procesu.
3. Nad převzatou knihovnou napsat tenkou vlastní vrstvu (náš typ chyb,
   capabilities, logy), aby se dala vyměnit.
4. Velký C kód jen jako izolovaný proces s minimem capabilities.
