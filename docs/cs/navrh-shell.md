# Jazyk shellu a chytrý terminál

> Stav: **odsouhlaseno a implementováno (milník 6b)** - [dva zápisy](#rozhodnutí-jeden-jazyk-dva-zápisy-b-a-c),
> [rozhodnutí k zápisu](#rozhodnutí-k-zápisu) a [rozhodnutí ke gramatice](#rozhodnutí-ke-gramatice).
> Jazyk popisuje [Gramatika](#gramatika-jazyka-lang-1) a [Sémantika](#sémantika),
> terminál [Terminál a služba relací](#terminál-a-služba-relací). Kapitoly
> o průzkumu a variantách ukazují, z čeho se vybíralo.
>
> Kód: `libs/shlang` (jazyk, `no_std` + `alloc`, testy na hostiteli),
> `libs/term` (protokol, relace, VT100, obrazovka), `user/termd` (prostředník), `user/shsess` (proces relace).

## Co se naučit od ostatních

| Shell | Co funguje | Na co si lidé stěžují |
|---|---|---|
| **Nushell** | tabulky a záznamy v rourách, `where`/`sort-by` bez `grep`/`awk`, čitelné chyby s místem v příkazu | časté nekompatibilní změny mezi verzemi; dlouho chybělo řízení úloh a `&&`/`\|\|`; vlastní `ls` a spol. se chovají jinak než systémové nástroje; slabší ošetření chyb a Ctrl-C; „spíš programovací jazyk než shell“ |
| **PowerShell** | objekty v rourách, jednotné parametry, nápověda | upovídané názvy (`Get-ChildItem`); objekty žijí jen uvnitř .NET; **roura pokazí binární data** (soubor stažený přes `>` je dvakrát větší); skryté převody typů; obtížné ladění roury |
| **Oils (YSH)** | cesta z `sh` k jazyku s daty; dva režimy - příkazový a výrazový; strukturovaná data „venku“ (JSON/TSV mezi procesy) | dva jazyky v jednom, pomalý vývoj |
| **Elvish** | hodnoty (seznamy, mapy, funkce) v rourách; nenulový návratový kód = výjimka | strukturovaná data jen mezi vestavěnými příkazy, externí programy dostanou bajty |
| **Murex** | typovaná data v rourách, `try/catch`, chyby s číslem řádku | malá komunita |
| **fish** | nic se nedělí na slova, rozumné uvozovky, našeptávání z historie, barvení syntaxe, nápověda | jen text, žádné struktury |

Zdroje: [The case for Nushell (HN)](https://news.ycombinator.com/item?id=37313733),
[I want to like Nushell](https://experimentalworks.net/posts/2024-02-24-i-want-to-like-nushell/),
[Ask HN: alternative shells as daily drivers](https://news.ycombinator.com/item?id=34722208),
[Oils: exterior-first](https://www.oilshell.org/blog/2023/06/ysh-design.html),
[Oils: why objects](https://www.oilshell.org/blog/2024/12/objects.html),
[PowerShell a binární data](https://brianreiter.org/tag/pipeline/),
[Elvish: unique semantics](https://elv.sh/learn/unique-semantics.html),
[Murex](https://github.com/lmorg/murex).

**Čím se lišíme:** máme vlastní OS. Programy v manifestu deklarují vstup
a výstup a roura nese binární záznamy se schématem
([terminál](architektura.md#terminál-líné-undo-a-relace)). Strukturovaná data
tak nejsou výsada vestavěných příkazů, jako je to u Nushellu nebo Elvishe,
a nic se nemusí parsovat z JSON. Proto smíme být „venku“ strukturovaní jako
Oils, ale bez textového mezikroku.

## Zásady (z toho plyne)

1. **Rychlé psaní interaktivně, čitelné skripty.** Krátké názvy (`ls`,
   `cp`, `where`), ne `Get-ChildItem`.
2. **Dva režimy jako Oils a PowerShell:** příkazový (slova jsou text,
   `ls -l /tmp`) a výrazový (`let`, `if`, porovnání, čísla s jednotkami).
   Hranice režimů je vidět na první pohled.
3. **Nic se tiše nemění.** Žádné dělení na slova, žádné rozvinutí `*`
   v uvozovkách, žádné skryté převody typů. **Bajtový proud je jiný typ
   než proud záznamů** a mezi nimi se převádí jen výslovně (`from json`,
   `to text`). Chyba PowerShellu s binárními daty tak nemůže nastat.
4. **Chyby jsou hodnoty:** kód + popis + místo. Neošetřená chyba se šíří
   sama a skript zastaví (jako `set -e`, ale spolehlivě);
   `try {...} else {...}` ji zpracuje. Interaktivně se vypíše
   s vysvětlením z [chyby.md](chyby.md).
5. **To, co v shellu běžně čekáš, je od začátku:** `&&` a `||`, řízení
   úloh (pozadí, přepnutí, Ctrl-C vždy funguje), přesměrování.
6. **Stabilita:** skript deklaruje verzi jazyka (`lang 1`). Nekompatibilní
   změna = nová verze, stará dál běží. Jde o poučení z Nushellu.
7. **Vestavěné příkazy nenapodobují cizí nástroje.** Jsou to naše
   programy s manifestem: `ls` vrací záznamy souborů. Na POSIX nástroje je
   zvlášť `sh` a VT100 adaptér.

## Varianty syntaxe

Stejná úloha ve třech variantách: velké soubory starší než rok, seřazené,
a smazání s potvrzením.

**A - roury záznamů (jako Nushell)**

```text
ls ~/Dokumenty -r | where size > 100MB and mtime < now - 1y | sort-by size -d
ls ~/Dokumenty -r | where size > 100MB | each { |f| rm $f.path }
```

**B - příkazový + výrazový režim (doporučeno)**

```mos
ls ~/Dokumenty -r | where size > 100MB && mtime < now - 1y | sort size desc
let velke = ls ~/Dokumenty -r | where size > 100MB
for f in $velke { rm $f.path? }          # ? = chyba přeruší skript
if (df /).free < 5GB { notify "Dochází místo" }
```

**C - blíž Rustu**

```text
ls("~/Dokumenty", recursive: true).filter(|f| f.size > 100.mb && f.mtime < now() - 1.year).sort_by(.size).rev()
```

| | A Nushell | B dva režimy | C Rust |
|---|---|---|---|
| interaktivní psaní | dobré | **dobré** | upovídané |
| skripty, logika | dobré | **dobré** (výrazy jako v programovacím jazyce) | nejlepší |
| učení pro uživatele bash | střední | **nejsnazší** (příkazy vypadají jako dřív) | těžké |
| hranice text / výraz | nejasná (`$x` vs `x`) | **jasná** (`let`, `if`, `(...)` = výraz) | jen výrazy |
| doplňování podle typů | ano | ano | ano |

## Rozhodnutí: jeden jazyk, dva zápisy (B a C)

> **Odsouhlaseno:** jazyk má **dva zápisy** - příkazový B (výchozí pro
> interaktivní psaní) a výrazový C blízký Rustu (pro skripty a logiku).
> Oba jsou **zápisy téhož jazyka**, ne dva jazyky. To se musí přísně hlídat.

```mos
ls ~/Dok -r | where size > 100MB                          # B
ls("~/Dok", recursive: true).where(f => f.size > 100MB)   # C, totéž
ls ~/Dok -r | where (size > 100MB && ext == "pdf")      # míchání: výraz v ( ) je C
```

**Pravidla, která to drží pohromadě** (kontroluje je CI, ne jen dohoda):

1. **Jeden strom, jedna sémantika.** Oba parsery vytvoří stejný strom
   výrazů. Typy, vyhodnocení, roury, chyby, manifest oprávnění,
   doplňování i nápověda existují jen jednou, nad stromem. Zápis nesmí mít
   vlastní chování.
2. **Každá konstrukce v obou zápisech.** Nová funkce jazyka se přijme jen
   s tvarem v B i v C. Výjimkou je, když tvar v jednom zápisu nedává smysl;
   pak se to výslovně zapíše do tabulky výjimek v této kapitole.
3. **Převod tam a zpět je test.** Formátovač umí `fmt --style b|c`. Testy
   pro každý příklad z dokumentace a testovací sady ověří:
   `B -> strom -> C -> strom` dá stejný strom a stejný výsledek, a totéž
   opačně.
4. **Dokumentace ukazuje oba tvary** (generované formátovačem, ne psané
   ručně dvakrát).
5. **Míchání je povolené** jen na jasné hranici: výraz v `( )` uvnitř
   příkazu je C, příkaz uvnitř výrazu se volá jako funkce.
6. **Verze jazyka (`lang 1`) platí pro oba zápisy** najednou.

**Tabulka výjimek** (konstrukce jen v jednom zápisu): žádná. Zkratky
zápisu B mají v C plný tvar se stejným stromem, viz
[Zkratky zápisu B](#zkratky-zápisu-b-a-jejich-plný-tvar).

## Rozhodnutí k zápisu

| # | Otázka | Rozhodnutí |
|---|---|---|
| 1 | základní styl | **B i C jako dva zápisy jednoho jazyka** |
| 2 | názvy příkazů | **obojí**: krátký název (`ls`, `cp`) a plné jméno `sloveso-podstatné` (`file-list`) jako alias; nápověda ukazuje oba |
| 3 | proměnné | **`$` jen v argumentech příkazu** (tam, kde by jinak byl text); ve výrazech, v podmínkách `where` a v zápisu C bez `$`. Holé slovo v argumentu, které se jmenuje jako proměnná, vyvolá napovídku „myslel jsi `$x`?“ |
| 4 | funkce (closure) | **`x => výraz`** a `(a, b) => výraz`, tělo s víc příkazy `x => { ... }`, v obou zápisech; pojmenovaná `fn jméno(x) { }`. Ne `\|x\|` - `\|` je v shellu roura |
| 5 | chyby | **neošetřená chyba se šíří sama** (jako výjimka) a zastaví skript; **chyba je hodnota** s kódem, popisem, místem a příčinou. Ošetření `try { } else e => ...`, záloha `cmd \|\| jiný`, výsledek jako hodnota `let r = try cmd`. `?` jen volitelně ve funkcích v C |
| 6 | čeština | **anglické názvy**, česká nápověda a chybové hlášky |
| 7 | skript bez `lang` | **nejnovější verze + varování** |

## Pravidla zápisu B

- **Příkazový režim:** `prikaz arg -p hodnota --dlouhy`. Slovo je text,
  `$x` je hodnota proměnné, `*.txt` rozvine shell podle FS (jen bez
  uvozovek). Argumenty mají typy z manifestu programu: chyba „čekám cestu,
  dostal jsem číslo“ přijde před spuštěním.
- **Výrazový režim:** za `let`, `if`, `for`, `while`, `return`, v podmínce
  `where` a uvnitř `( )`. Čísla s jednotkami (`100MB`, `1y`, `30s`),
  porovnání, `&&` `||` `!`, řetězce v `"..."` s `{výraz}` uvnitř, seznamy
  `[1, 2]`, záznamy `{name: "a", size: 1KB}`. Proměnné bez `$`.
- **Roura:** `|` předává záznamy (nebo bajty, podle manifestu). Běžné
  operace jsou krátká slova: `where`, `select`, `sort`, `group`, `first`,
  `count`, `each f => ...`. Převody jsou výslovné: `from json`,
  `to text`, `lines`.
- **Chyby:** neošetřená chyba zastaví skript s vysvětlením.
  `try { rm $f } else e => log "nešlo: {e.message}"` ji zpracuje,
  `rm $f || echo "nevadí"` použije zálohu, `let r = try rm $f` vrátí
  výsledek jako hodnotu (`r.failed`, `r.error.code`).
- **Manifest skriptu:** hlavička, kterou shell zkontroluje před spuštěním
  a podle které předá capabilities:

```mos
lang 1
needs {
    read: ["~/Fotky"]
    write: ["~/Fotky/zmensene"]
    net: none
}
for f in (ls ~/Fotky | where ext == "jpg") {
    image resize $f.path --width 1600 -o ~/Fotky/zmensene/{f.name}
}
```

- **Bezpečné výchozí chování:** příkaz, který mění soubory, dostane
  koncept v klonu FS a undo. Hromadná změna nad N souborů běží přes „ukaž, co se
  stane“ ([terminál](architektura.md#shell-a-chytrý-terminál-návrh)).

## Rozhodnutí ke gramatice

| # | Otázka | Rozhodnutí |
|---|---|---|
| 1 | hranice příkazového a výrazového režimu v B | **podle rozsahu proměnných**: holé slovo na začátku části roury (příkaz, `( )`, tělo closure) je příkaz, pokud to není proměnná ani klíčové slovo. Jinak je to výraz. `(x + 1)` je výraz, když `x` je proměnná; `(ls ~)` je vnořený příkaz |
| 2 | podmínka `where` | holé jméno je **proměnná, když ji parser vidí v rozsahu**, jinak pole řádku `it`. Když se proměnná jmenuje jako známé pole vstupu (schéma z podpisu předchozího příkazu), je to **chyba** s radou `it.size`. Pozdější `let size = 5` tak nikdy potichu nezmění význam filtru |
| 3 | přesměrování | `>`, `>>` a `<` platí **jen v příkazovém režimu** a jsou zkratkou: `cmd > f` je `cmd \| save f`, `>>` je `save --append`, `cmd < f` je `open f \| cmd`. Ve výrazu (podmínka `where`, `if`, `( )`) je `>` vždy porovnání. Záznamy do souboru jen s výslovným `to text`/`to json` |
| 4 | volba zápisu | **po příkazech**: slovo hned následované `(` (`ls("~")`) nebo `.příkaz(` je tvar C, jinak B. Hlavička `lang 1 c` vynutí zápis C pro celý soubor (příkaz v B je pak chyba) |

## Gramatika jazyka (lang 1)

<figure class="diagram">
<svg viewBox="0 0 760 270" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Jeden strom pro oba zapisy">
<defs><marker id="sh1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="230" height="56" rx="6"/><text class="t b" x="125" y="42" text-anchor="middle">zápis B (příkazový)</text><text class="mono t" x="125" y="62" text-anchor="middle">ls ~ -r | where size &gt; 1MB</text>
<rect class="box" x="10" y="110" width="230" height="56" rx="6"/><text class="t b" x="125" y="132" text-anchor="middle">zápis C (výrazový)</text><text class="mono t" x="125" y="152" text-anchor="middle">ls("~", recursive: true)...</text>
<path class="ln" d="M240 48 L318 82" marker-end="url(#sh1)"/>
<path class="ln" d="M240 138 L318 104" marker-end="url(#sh1)"/>
<rect class="acc" x="320" y="55" width="150" height="76" rx="6"/><text class="t b" x="395" y="82" text-anchor="middle">společný strom</text><text class="m" x="395" y="100" text-anchor="middle">ast.rs, rozsah</text><text class="m" x="395" y="116" text-anchor="middle">proměnných</text>
<path class="ln" d="M470 70 L548 34" marker-end="url(#sh1)"/>
<path class="ln" d="M470 93 L548 93" marker-end="url(#sh1)"/>
<path class="ln" d="M470 116 L548 152" marker-end="url(#sh1)"/>
<rect class="ok" x="550" y="10" width="200" height="46" rx="6"/><text class="t b" x="650" y="30" text-anchor="middle">formátovač</text><text class="m" x="650" y="47" text-anchor="middle">fmt --style b | c</text>
<rect class="ok" x="550" y="70" width="200" height="46" rx="6"/><text class="t b" x="650" y="90" text-anchor="middle">interpret</text><text class="m" x="650" y="107" text-anchor="middle">typy, roury, chyby</text>
<rect class="ok" x="550" y="130" width="200" height="46" rx="6"/><text class="t b" x="650" y="150" text-anchor="middle">doplňování, nápověda</text><text class="m" x="650" y="167" text-anchor="middle">podpisy příkazů</text>
<path class="ln-acc dash" d="M650 56 C 650 230, 125 230, 125 168" marker-end="url(#sh1)"/>
<text class="m" x="10" y="250">Test převodu: B -&gt; strom -&gt; C -&gt; strom dá stejný strom i stejný výsledek (a opačně). Je to povinná brána v cargo xtask test.</text>
</svg>
<figcaption>Oba parsery dělají stejný strom; všechno ostatní pracuje jen se stromem.</figcaption>
</figure>

### Lexikální prvky

- **Komentář:** `#` do konce řádku. Komentář na samostatném řádku formátovač
  zachová.
- **Čísla:** `42`, `-5`, `3.14`, `1_000_000`. **S jednotkou** (bez mezery):

  | Druh | Jednotky | Hodnota |
  |---|---|---|
  | velikost | `B`, `KB`, `MB`, `GB`, `TB` (×1000), `KiB`, `MiB`, `GiB`, `TiB` (×1024) | bajty (celé číslo) |
  | doba | `ns`, `us`, `ms`, `s`, `m` (minuta), `min`, `h`, `d`, `w`, `y` (365 dní) | nanosekundy (celé číslo) |
  | frekvence | `Hz`, `kHz`, `MHz`, `GHz` | hertzy (desetinné) |
  | procenta | `%` | poměr, 1 = 100 % (desetinné) |

  **Složená doba** jde od větší jednotky k menší, každá jednotka jednou:
  `1h30m`, `2m30s`, `1d12h`. `%` za číslem je procento jen tehdy, když za
  ním nenásleduje číslo ani jméno (`10%3` je zbytek po dělení).
  Počítá se jen se stejnou veličinou nebo s číslem (`440Hz + 60Hz`,
  `1MHz / 4`); procento z hodnoty `50% * 200MB` dá 100 MB, `10% * 1h`
  6 minut. Různé veličiny sečíst ani porovnat nejde (chyba `SH_TYPE`).
  Výpis volí největší jednotku, ve které je hodnota aspoň 1 (`2.4 GHz`,
  `50%`) **[implementováno]**.

  Jednotky jsou data: tabulka `shlang::units::TABLE` (zápis, veličina,
  násobek základní jednotky). Novou veličinu (délka, teplota, rychlost
  přenosu) přidá řádek tabulky a rozměr; lexer, výpis a nápověda k neznámé
  jednotce ji převezmou. Velikost a doba jsou dál přesná celá čísla,
  ostatní veličiny desetinné číslo v základní jednotce.

  **Převody** dělá `convert` ([Příkazy shellu](prikazy-shellu.md#convert)):
  jednotky z téže tabulky (`1GiB | convert MB` = 1073.741824, číslo
  v cílové jednotce), číselné soustavy (`255 | convert hex` = `0xff`),
  kódování textu (`utf8`, `utf16le`, `utf16be`, `latin2`, `base64`, `hex`;
  zpět `--from`) a časová pásma (`now | convert Europe/Prague`). Pásma
  bere ze stejné knihovny jako služba času, hodiny v liště a instalátor
  (`libs/tz` přes `libos::time::zone`: jména IANA, pevný posun, letní čas
  podle současných pravidel bez historie). Formáty dat (json, řádky) zůstávají
  na `from` / `to` **[implementováno]**.

  **Otevřená otázka:** `m` je dnes minuta. Až přibude délka, `m` (metr)
  by kolidovalo; řešení (metr jen v `convert`, jiný zápis minuty, nebo
  podle kontextu) zatím není rozhodnuté **[navrženo]**.

- **Řetězce:** `"..."` s escape sekvencemi `\n \t \r \0 \\ \" \{ \}`
  a vloženými výrazy `{výraz}`; `'...'` je doslovný text bez escape sekvencí
  a bez `{}`.
- **Jména:** písmena (i s diakritikou), číslice a `_`. Jména příkazů v B
  smějí mít `-` (`file-list`); v C se píše `_` (`file_list(...)`) a parser
  to převede zpět, když takový příkaz existuje.
- **Klíčová slova:** `let fn if else for in while return break continue try
  bg true false null lang needs`.

### Gramatika (EBNF)

```text
skript      = [ "lang" CISLO [ "b" | "c" ] KONEC ] [ needs KONEC ] { prikaz KONEC }
needs       = "needs" "{" { ("read" | "write") ":" seznam_cest | "net" ":" ("none" | seznam_cest) } "}"
prikaz      = "let" JMENO "=" retez
            | JMENO "=" retez                          (* JMENO je promenna *)
            | "fn" JMENO "(" [ JMENO { "," JMENO } ] ")" blok
            | "return" [ retez ] | "break" | "continue"
            | retez
blok        = "{" { prikaz KONEC } "}"

(* zapis B *)
retez       = a_retez { "||" a_retez } [ "&" ]
a_retez     = roura { "&&" roura }
roura       = cast { "|" volani_B } [ (">" | ">>") arg | "<" arg ]
cast        = volani_B                                 (* hole slovo, ktere neni promenna *)
            | vyraz_bez_logiky                         (* jinak *)
volani_B    = PRIKAZ { arg | prepinac } | PRIKAZ "(" argumenty_C ")"
prepinac    = "-" PISMENA | "--" JMENO [ "=" arg ] | "--"
arg         = "$" JMENO { "." JMENO } | RETEZEC | "(" retez ")" { pripona }
            | seznam | zaznam | blok | closure | SLOVO
SLOVO       = text, cislo s jednotkou, true/false/null, vzor (*.txt) nebo slovo s {vyrazem}

(* zapis C: retez = vyraz *)
vyraz       = closure | vyraz "||" vyraz | vyraz "&&" vyraz | porovnani
porovnani   = soucet [ ("==" | "!=" | "<" | "<=" | ">" | ">=") soucet ]
soucet      = soucin { ("+" | "-") soucin }
soucin      = unarni { ("*" | "/" | "%") unarni }
unarni      = ("!" | "-") unarni | zaklad { pripona }
pripona     = "." JMENO | "." JMENO "(" argumenty_C ")" | "[" vyraz "]" | "?"
zaklad      = CISLO | RETEZEC | JMENO | JMENO "(" argumenty_C ")" | "glob" "(" RETEZEC ")"
            | "(" vyraz ")" | seznam | zaznam | blok
            | "if" vyraz blok [ "else" ( blok | if ) ] | "for" JMENO "in" vyraz blok
            | "while" vyraz blok | "try" ( blok | cast ) [ "else" ( closure | blok ) ]
            | "bg" blok
argumenty_C = [ arg_C { "," arg_C } ]
arg_C       = [ JMENO ":" ] vyraz
closure     = ( JMENO | "(" [ JMENO { "," JMENO } ] ")" ) "=>" ( blok | telo )
seznam      = "[" [ vyraz { "," vyraz } ] "]"
zaznam      = "{" [ klic ":" vyraz { "," klic ":" vyraz } ] "}"
```

Výrazy (aritmetika, porovnání, closure, `if`, `try`, ...) jsou **v obou
zápisech stejné**. Zápisy se liší jen tím, jak se píše příkaz a roura:

| Konstrukce | Zápis B | Zápis C | Strom |
|---|---|---|---|
| příkaz | `ls ~/Dok` | `ls("~/Dok")` | `Call` |
| přepínač | `-r`, `--recursive` | `recursive: true` | pojmenovaný argument `recursive` |
| přepínač s hodnotou | `--width 1600`, `--width=1600` | `width: 1600` | pojmenovaný argument |
| proměnná v argumentu | `$f.path` | `f.path` | `Field(Var)` |
| vzor souborů | `*.txt` | `glob("*.txt")` | `Glob` |
| text s výrazem | `~/out/{f.name}` | `"~/out/{f.name}"` | `Interp` |
| roura | `a \| b x` | `a().b(x)` | `Pipe` |
| podmínka | `where size > 1MB` | `.where(it => it.size > 1MB)` | `Closure(it)` |
| pozadí | `cmd &` | `bg { cmd() }` | `Bg` |
| uložení do souboru | `cmd > f`, `cmd >> f` | `.save("f")`, `.save("f", append: true)` | `Pipe(..., save)` |
| program cestou nebo se jménem s `-` bez podpisu | `./run.sh x`, `my-tool a` | `exec("./run.sh", "x")`, `exec("my-tool", "a")` | `Call` |

### Hranice režimů v zápisu B

- **Na začátku části roury** (řádek, `let x =`, obsah `( )`, tělo closure):
  holé slovo, které není proměnná ani klíčové slovo, je **příkaz**. Jinak je
  to **výraz**. Slovo hned následované `(` je volání ve tvaru C.
- **Za `|`** je vždy příkaz.
- **Argumenty příkazu:** holé slovo je text. Může to být i číslo
  s jednotkou nebo `true`/`false`/`null` (`first 5` dá číslo 5, ne text).
  `$x` je hodnota proměnné, `( )` vnořená roura nebo výraz, `[ ]` seznam,
  `{ }` záznam nebo blok. Text, který vypadá jako číslo, dejte do uvozovek
  (`"5"`).
- **Výraz** je za `let`, v podmínce `if`/`while`, za `for ... in`,
  v podmínce `where` a v `{vloženém výrazu}`. Příkaz se ve výrazu volá jako
  funkce (`df("/")`) nebo v závorce (`(df /)`).
- `$` patří **jen do argumentů příkazu**. Ve výrazu je `$x` chyba s radou
  „napište jen x“. Holé slovo v argumentu, které se jmenuje jako proměnná,
  vyvolá varování „myslel jsi `$x`?“.

### Zkratky zápisu B a jejich plný tvar

Zkratky nejsou výjimky: mají plný tvar, který existuje v obou zápisech.
Formátovač v B tiskne zkratku, v C plný tvar.

| Zkratka B | Plný tvar | Poznámka |
|---|---|---|
| `cmd &` | `bg { cmd }` | úloha na pozadí |
| `cmd > f`, `cmd >> f` | `cmd \| save f`, `cmd \| save f --append` | `save` přijme jen bajty nebo text |
| `cmd < f` | `open f \| cmd` | |
| `where size > 1MB` | `where it => it.size > 1MB` | `it` je řádek |
| `-r` | `--recursive` | krátký přepínač podle podpisu |
| `-` + číslice | záporné číslo | `-5` není přepínač |
| `--` | (nic) | další slova začínající `-` jsou text |

### Podmínka `where`

```shell-b
let limit = 1MB
ls | where size > limit && ext == "pdf"
```

`size` a `ext` nejsou proměnné, takže jsou to pole řádku. `limit` je
proměnná. Kdyby existovala proměnná `size`, parser ohlásí chybu „`size` je
proměnná i pole řádku“. Pak napište `it.size`, proměnnou přejmenujte, nebo
použijte closure s vlastním jménem:

```shell-b
let size = 5
ls | where f => f.size > size
```

Uvnitř podmínky se příkaz volá jako funkce (`now()`), protože holé slovo je
pole: `where mtime < now() - 1y`.

## Sémantika

### Hodnoty a typy

| Typ | Příklad | Poznámka |
|---|---|---|
| `int`, `float` | `42`, `1.5` | celé číslo se v aritmetice s desetinným převede (jediný automatický převod) |
| `size` | `100MB` | velikost se nesčítá ani neporovnává s číslem: `size > 100` je chyba s radou `100B` |
| `duration` | `30s`, `1h`, `1h30m` | |
| `frequency`, `percent` | `2.4GHz`, `50%` | veličiny z tabulky jednotek (viz Čísla) |
| `time` | `now()` | `time - duration` = `time`, `time - time` = `duration` |
| `text` | `"abc"` | |
| `bytes` | `open f` | **bajtový proud**, jiný typ než text a záznamy |
| `list`, `record` | `[1, 2]`, `{a: 1}` | seznam záznamů je tabulka |
| `closure` | `x => x * 2` | zachytí kopie použitých proměnných |
| `error` | `e` v `try ... else e => ...` | pole `code`, `message`, `hint`, `cause` |
| `null`, `nothing` | | `nothing` = příkaz nic nevrátil |
| chybějící hodnota | `size` cizí složky | `null` s důvodem: **skrytá** (chybí právo) nebo **nedostupná** (zdroj ji nevede); viz níže |

- **Chybějící hodnota** (`Value::Missing`) se chová jako `null`: rovná se
  `null`, porovnání `<` / `>` je `false` (`where size > 1MB` ji vynechá
  místo chyby), při řazení jde na začátek. Terminál ji ukáže jako „—“
  a v patičce tabulky vysvětlí důvod. Prázdná složka není chybějící
  hodnota: má 0 B, 0 souborů a 0 složek. Dnes shell stav odhaduje
  z odpovědi fsd (složka bez práva spravovat = souhrn skrytý, nulový čas =
  nedostupný) **[implementováno]**; přesné příznaky ve fsproto (masky
  `known` / `hidden` u `Stat` a `LIST_STAT_PAGE`) jsou zadání pro vlákno
  FS **[navrženo]**.
- **Žádné skryté převody.** Mezi bajty, textem a záznamy se převádí
  jen výslovně: `from json`, `from text`, `lines`, `to text`, `to json`.
  `open data.json | where ...` je chyba s radou `from json`.
- **Rovnost** různých typů je chyba (kromě čísel a `null`). `size == 5`
  by jinak potichu vrátilo `false`.
- Podmínka `if`/`while`/`where` musí být `bool`. Hodnota se nikdy
  nepočítá jako „pravdivá“.
- Pole na seznamu záznamů vrátí sloupec: `(ls).name` je seznam jmen.

### Výstup

Na nejvyšší úrovni (řádek, tělo cyklu, větve `if` jako příkaz) se hodnota
každého příkazu vypíše jako blok výstupu. V těle closure a funkce je
výsledkem hodnota posledního příkazu. Hodnoty dřívějších příkazů se tam
zahodí; poznámku vypíše `log`.

### Chyby

- Neošetřená chyba se **šíří sama** a zastaví skript. Změny souborů,
  které řádek udělal před chybou, se převezmou (jako v jiných shellech)
  a jdou vrátit `undo`.
- `try { ... } else e => ...` ji zpracuje, `cmd || jiný` použije zálohu
  (pravá strana běží i tehdy, když je levá `false`), `let r = try cmd`
  vrátí výsledek jako záznam `{ok, failed, value, error}`. `x?` takový
  výsledek rozbalí nebo jeho chybu vyvolá; v C je `?` jen volitelné, protože
  chyba se šíří sama.
- `a && b`: chyba v `a` se šíří (nezachytí se), `false` vrátí `false`,
  jinak se vyhodnotí `b`.
- **Ctrl-C** (`SH_INTERRUPTED`) nejde zachytit: `try` ani `||` ho nezastaví.
- Chyba nese kód, popis, místo ve zdroji (řádek, sloupec, podtržení),
  radu a příčinu. `explain kód` ukáže vysvětlení z [Kódů chyb](chyby.md)
  a z [kapitoly Chyby shellu](#chyby-shellu).
- Chyba systému je v hlášení **jedna věta** s kódem jednou na konci:
  „Složku /x nejde vypsat: nemáte právo výpisu (ACCESS_DENIED)“. Důvod
  u běžných kódů operací se soubory dává shell vlastní větou (výpis, čtení,
  zápis; `NOT_FOUND`, `ALREADY_EXISTS`, `NOT_EMPTY`), jinak česky z katalogu
  (věta **Význam** v [Kódech chyb](chyby.md)). Popisy v `abi` zůstávají
  ASCII a slouží jen jako náhrada **[implementováno]** (testy na hostiteli
  `tests/misc.rs`).
- Místo ve zdroji (řádek, sloupec, podtržení `^^`) hlášení ukáže jen
  u chyby syntaxe a u chyby, která patří k části delšího řádku. U chyby
  celého příkazu nebo celé roury (`ls` -> `ACCESS_DENIED`) by nic
  neříkalo, proto chybí **[implementováno]**.

### Funkce, closure, úlohy

- `fn jméno(a, b) { ... }` vidí své parametry, globální proměnné a `input`
  (vstup z roury). Volá se jako příkaz (`jméno 1 2`, `jméno --b=2 --a=1`)
  nebo jako funkce (`jméno(1, 2)`).
- Closure zachytí **kopie** proměnných, které používá, v okamžiku vytvoření.
  Pozdější změna proměnné ji neovlivní.
- **Úloha na pozadí** (`cmd &`, `bg { }`) běží kooperativně: když je shell
  volný, nebo při `wait`. `jobs` je vypíše, `kill N` zruší čekající úlohu.
- **Hloubku rekurze** hlídá skutečná spotřeba zásobníku. Rozpočet nastaví
  hostitel; `shsess` má zásobník 256 KB a interpret smí 160 KB. Při překročení
  vznikne `SH_DEPTH`, ne pád.

### Verze jazyka

`lang 1` platí pro oba zápisy. Skript bez `lang` poběží v nejnovější verzi
s varováním. Neznámá verze je chyba `SH_LANG`. Nekompatibilní změna =
nová verze; starý skript pak dál běží ve své.

## Vestavěné příkazy

Každý příkaz má krátké jméno a plné jméno `sloveso-podstatné` (alias).
Úplná reference všech příkazů (popis, argumenty a přepínače, roura,
příklady, chyby, související příkazy) je na stránce
[Příkazy shellu](prikazy-shellu.md). Stránku generuje `cargo xtask docs`
z metadat v `libs/shlang/src/doc/` - stejná data ukazuje `help <příkaz>`
v terminálu a příklady jsou testy (`cargo test -p shlang doc`), takže
nápověda nezastará.

## Příklady

Každý příklad je napsaný v jednom zápisu. Druhý zápis (označený `*`)
vygeneroval formatter při `cargo xtask docs`. Test `cargo test -p shlang`
ověří u každého příkladu z této stránky převod tam i zpět.

Velké soubory starší než rok, seřazené podle velikosti:

```shell-b
ls ~/Dokumenty -r | where size > 100MB && mtime < now() - 1y | sort size -d
```

Smazání s ošetřením chyby:

```shell-b
for f in (ls ~/tmp | where ext == "log") {
    try rm $f.path else e => log "nešlo: {e.message}"
}
```

Data z JSON, výsledek do souboru:

```shell-b
open lide.json | from json | where age >= 18 | select name age | to json > dospeli.json
```

Funkce a closure v zápisu C:

```shell-c
fn velke(slozka, limit) {
    return ls(slozka, recursive: true).where(it => it.size > limit)
}
let soucet = velke("~/Videa", 1GB).sum("size")
```

Záloha a výsledek jako hodnota:

```shell-b
let r = try rm stary.txt
if r.failed {
    echo "nesmazano: {r.error.code}"
}
rm docasny.txt || echo "nebyl tam"
```

Skript s manifestem oprávnění:

```shell-b
lang 1
needs {
    read: ["~/Fotky"]
    write: ["~/Fotky/male"]
    net: none
}
for f in (ls ~/Fotky | where ext == "jpg") {
    cp $f.path ~/Fotky/male/{f.name}
}
```

## Soubory: koncept v klonu, undo, náhled a zámek

> **Změna proti první verzi shellu:** změny nanečisto už nejsou překryv
> v paměti shellu, ale **klon datasetu „bokem“ přímo ve FS** (`ASIDE_*`
> z [FS fáze 2](fs-historie.md)). Undo nestojí na snapshotu celého svazku,
> ale na **pohledu** (`VIEW`) jen dotčených datasetů. Po programu se změny
> zjišťují ze žurnálu a operace `DIFF`, ne procházením stromu. Proč: velké
> zápisy nestojí paměť, program v `preview` pracuje nad stejným konceptem
> jako vestavěné příkazy a undo programu je rychlé i na velkém disku.

<figure class="diagram">
<svg viewBox="0 0 760 270" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Koncept radku v klonu FS">
<defs><marker id="sh2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="80" width="130" height="56" rx="6"/><text class="t b" x="75" y="104" text-anchor="middle">příkaz</text><text class="m" x="75" y="122" text-anchor="middle">rm, save, mv, program</text>
<path class="ln" d="M140 108 L188 108" marker-end="url(#sh2)"/>
<rect class="acc" x="190" y="70" width="150" height="76" rx="6"/><text class="t b" x="265" y="94" text-anchor="middle">klon bokem</text><text class="m" x="265" y="112" text-anchor="middle">ASIDE_BEGIN při</text><text class="m" x="265" y="128" text-anchor="middle">prvním zápisu</text>
<path class="ln" d="M340 108 L378 108" marker-end="url(#sh2)"/>
<rect class="box" x="380" y="80" width="130" height="56" rx="6"/><text class="t b" x="445" y="104" text-anchor="middle">ASIDE_CHANGES</text><text class="m" x="445" y="122" text-anchor="middle">konec řádku</text>
<path class="ln" d="M510 95 L558 48" marker-end="url(#sh2)"/>
<path class="ln" d="M510 122 L558 172" marker-end="url(#sh2)"/>
<rect class="ok" x="560" y="20" width="190" height="56" rx="6"/><text class="t b" x="655" y="42" text-anchor="middle">≤ N: VIEW + APPLY</text><text class="m" x="655" y="60" text-anchor="middle">pohled pro undo, převzetí</text>
<rect class="warn" x="560" y="145" width="190" height="56" rx="6"/><text class="t b" x="655" y="167" text-anchor="middle">&gt; N nebo preview</text><text class="m" x="655" y="185" text-anchor="middle">diff, potvrdit / DISCARD</text>
<path class="ln dash" d="M655 145 L655 78" marker-end="url(#sh2)"/>
<text class="m" x="10" y="236">Čtecí řádek klon nezakládá a nic nestojí. Klon leží v /tmp/&lt;uid&gt;/.aside/&lt;relace&gt;-&lt;n&gt;; vnořené datasety jsou v něm hranice</text>
<text class="m" x="10" y="254">a zápis do nich dostane vlastní klon. Převzetí je trojcestné: cizí změna mezitím = SH_CONFLICT, ne přepsání.</text>
</svg>
<figcaption>Změny řádku jdou do klonu datasetu ve FS; na konci řádku se převezmou s pohledem pro undo.</figcaption>
</figure>

- **Koncept řádku:** první zápis řádku do datasetu založí jeho klon
  (`ASIDE_BEGIN`, `libs/shlang/src/vfs/draft.rs`). Další zápisy i čtení
  toho datasetu jdou do klonu, takže řádek vidí své vlastní změny. Zápis do
  vnořeného datasetu (v klonu je to hranice) dostane klon toho datasetu.
  Na konci řádku shell vezme diff z FS (`ASIDE_CHANGES`). Řádek, který jen
  čte, klon nezaloží a nestojí nic navíc.
- **Undo:** před převzetím (`ASIDE_APPLY`) shell vytvoří **pohled**
  (`VIEW`, levný dočasný snapshot a pozice v žurnálu) každého datasetu se
  změnami. `undo` srovná změněné cesty se stavem v pohledu (čte přes
  `SNAP_*_OF`) a funguje opakovaně (posledních 16 příkazů, starší pohledy se
  uvolní). Pohledy po restartu uklidí FS sám.
- **Konflikt:** převzetí je trojcestné proti stavu, ze kterého klon
  vznikl. Když jiná relace nebo program mezitím změnil stejný soubor,
  změna se nepřevezme a řádek skončí `SH_CONFLICT` se seznamem cest.
  Ostatní změny jsou zapsané a `undo` vrátí jen je.
- **„Ukaž, co se stane“:** `preview { ... }` přidá další vrstvu (klon
  klonu), spustí blok, ukáže tabulku změn (`added`, `modified`, `removed`,
  `dir-added`, `dir-removed`, `replaced` = soubor nahrazený složkou nebo
  naopak, `moved`) a převezme ji do konceptu řádku až po potvrzení (`-y` bez
  dotazu). Odmítnutí = `ASIDE_DISCARD`, `SH_REJECTED` a nic se nezmění.
- **Zámek nebezpečných operací:** příkaz, který by změnil víc než N
  souborů mimo `/tmp` (výchozí N = 20), se před převzetím vždy zeptá. Je to
  stejný dotaz jako u `preview`. Bez připojeného okna se odmítne.
- **Holá cesta jako příkaz** (`/`, `./x`, `../`, `~`, `~/Dokumenty`, `.`,
  `..`; `libs/shlang/src/eval/bare.rs`): složka = `cd`, skript `.mos` =
  `run` (s kontrolou `needs`), program (ELF) = spuštění stejnou cestou jako
  program podle jména (práva z manifestu, náhled, undo; argumenty dostane
  program), jiný soubor = aplikace podle typu přes službu „otevři cestu“
  (`open-app`, jen aplikace s asociací); bez aplikace obsah jako `open`
  a poznámka „Pro typ .xyz není aplikace.“ (bez služby, např. v záložní
  konzoli, jen obsah). Příkaz `start <cesta>` otevře soubor nebo složku
  jako dvojklik (soubor bez aplikace ukáže správce souborů); popis služby
  `docs/cs/spravce-souboru.md` **[ověřeno v QEMU]**.
  Složka, skript ani soubor argumenty neberou (`SH_ARGS`); neexistující
  cesta je `NOT_FOUND` s radou. Zvýraznění ukazuje holou cestu jako cestu,
  ne jako neznámý příkaz. Hostitel v OS (`shsess`) zatím programy z cesty
  nespouští: chyba `NOT_SUPPORTED` **[implementováno]**.
- **Spouštění programů podle jména** (`ping 10.0.2.2`, `std-exec net`)
  **[ověřeno v QEMU]** (test `ucty: programy ze shellu (sit podle
  manifestu)`, x86_64 i aarch64):
  - Shell (`shsess`) sám síť nemá a programy nespouští. Pošle službě
    „exec“ initu (`user/init/src/exec.rs`, protokol `launchproto::exec`)
    jen jméno, argumenty, pracovní složku, připojené cesty a jestli smí
    program mít síť (`needs`). Kanál služby má každá relace uživatele
    (místní i SSH), okno ho dostane od initu a termd ho předá relacím
    shellu.
  - Init spustí program přes launchd (jeden registr příkazů: `[[command]]`,
    aplikace z disku, `/system/bin`, initfs) v jobu relace, s relací FS
    s identitou uživatele relace (zúženou připojenými cestami) a se sítí
    jen tehdy, když ji program má v manifestu (`[network]`, příznak
    `NET_NEEDS_MANIFEST`). launchd síť zúží manifestem, firewall
    rozhoduje podle id aplikace. Program bez manifestu síť nedostane.
  - Výstup programu jde do bloku po řádcích, jak přichází (stdout jako
    text, stderr jako poznámka). Vstup je data z roury (`"a" | prog`).
    Bez roury se po 300 ms běhu ukáže textová výzva a každý zadaný řádek
    jde programu (interaktivní stdin, např. `nc`). Ctrl+C program ukončí
    (`SH_INTERRUPTED`), nenulový návratový kód je `SH_EXIT`.
  - Argumenty dostane program v pořadí zápisu jako text (`ping x --count 2`
    = `x`, `--count`, `2`; `--jméno=hodnota` = `--jméno`, `hodnota`).
    `~` a `~/...` na začátku textového argumentu shell rozvine na domov
    (i v uvozovkách: AST je od holého slova neodliší).
  - Typovaný kanál (program vrací hodnoty shellu místo bajtů) a výstup
    programu jako hodnota pro další příkaz roury **[navrženo]**. Spuštění
    cestou (`./prog`, `/apps/...`) zatím `NOT_SUPPORTED` **[navrženo]**.
- **Manifest `needs`:** `run skript` před spuštěním ukáže, co skript chce
  (`read`, `write`, `net`), a po potvrzení ho omezí: přístup mimo uvedené
  cesty je `SH_NEEDS`. Skript dostane nejvýš průsečík s právy toho, kdo ho
  spouští: zápis jen tam, kam volající smí zapisovat, čtení tam, kde smí
  číst, síť jen se sítí. Jinak `SH_NEEDS` ještě před spuštěním. Úloha na
  pozadí (`bg`, `&`) si omezení nese s sebou, i když doběhne až po skončení
  skriptu.
- **Programy:** dostanou oprávnění z manifestu přes vlastní relaci fsd
  (`CONNECT_AS` s připojenými cestami, `term::perm::mounts`; zápis přebije
  čtení). Mimo `preview` shell před spuštěním převezme dosavadní koncept
  řádku **se stejným zámkem** jako na konci řádku (odmítnutí = program se
  nespustí) a vytvoří pohledy datasetů, kam program smí psát (s plnými
  právy `/`, domov a pracovní složka). Po skončení žurnál jednou zprávou
  řekne, jestli se něco změnilo. Když ne, záznam undo se zahodí, takže
  `undo` vrátí předchozí příkaz. Když ano, `DIFF` vrátí změněné cesty a
  `undo` je srovná s pohledem.
- **Program v `preview`:** shell založí klony datasetů, kam program smí
  psát, a připojí mu je **pod původními jmény** (`ExecNeeds::aside`).
  Zápis mimo klony dostane jen pro čtení. Program tak mění jen koncept:
  jeho změny jsou v tabulce náhledu a po odmítnutí zmizí.

```shell-b
preview { rm *.tmp; echo hotovo | save ~/vysledek.txt }
undo
```

⚠️ **Omezení:**

- Klon nejde založit tam, kde uživatel nemá soukromou dočasnou složku
  `/tmp/<uid>` (zakládá ji init při otevření relace,
  [Účty](ucty.md)). Zápis pak skončí chybou s radou.
- Undo programu s plnými právy hlídá datasety `/`, domova a pracovní
  složky. Změnu v jiném vnořeném datasetu nevrátí.
- Pohled zachytí i změny jiných relací ve stejných složkách během běhu
  programu. `undo` by je vrátilo také.
- Když je konfliktů víc, než FS vypíše (přes 1024), řádek skončí
  `SH_CONFLICT` a převzetí nejde vrátit (shell neví jistě, co se převzalo).

**Pojistky:**

- `undo` se nikdy nedotkne vnořeného datasetu: pohled vidí jen svůj
  dataset, srovnání vnořené datasety přeskočí (i při srovnání celé složky).
- Po programu shell nejdřív potvrdí změny ve FS (`COMMIT`) a teprve pak se
  ptá žurnálu - žurnál ukazuje jen potvrzené záznamy.
- Čtení v řádku s klonem se FS na dataset neptá: vezme klon nad cestou
  a jen u hranice vnořeného datasetu (`NOT_SUPPORTED`) to zopakuje přesně.
- Zahození klonu (uvolnění bloků) se odkládá, až je shell volný; výsledek
  příkazu je v terminálu hned.
- Program v `preview` dostane klony i vnořených datasetů domova
  a pracovní složky.

## Terminál a služba relací

<figure class="diagram">
<svg viewBox="0 0 760 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Sluzba relaci">
<defs><marker id="sh3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="30" width="170" height="60" rx="6"/><text class="t b" x="95" y="55" text-anchor="middle">okno terminálu</text><text class="m" x="95" y="74" text-anchor="middle">(6c GUI, test, později SSH)</text>
<path class="ln" d="M180 60 L248 60" marker-end="url(#sh3)"/>
<text class="m" x="184" y="50">protokol</text>
<rect class="acc" x="250" y="30" width="150" height="60" rx="6"/><text class="t b" x="325" y="55" text-anchor="middle">termd</text><text class="m" x="325" y="74" text-anchor="middle">jen směruje zprávy</text>
<path class="ln" d="M400 50 L468 40" marker-end="url(#sh3)"/>
<path class="ln" d="M400 75 L468 110" marker-end="url(#sh3)"/>
<rect class="box" x="470" y="15" width="150" height="50" rx="6"/><text class="t b" x="545" y="36" text-anchor="middle">shsess 1</text><text class="m" x="545" y="54" text-anchor="middle">interpret, undo, úlohy</text>
<rect class="box" x="470" y="85" width="150" height="50" rx="6"/><text class="t b" x="545" y="106" text-anchor="middle">shsess 2</text><text class="m" x="545" y="124" text-anchor="middle">vlastní proces</text>
<path class="ln" d="M620 40 L668 75" marker-end="url(#sh3)"/>
<path class="ln" d="M620 110 L668 85" marker-end="url(#sh3)"/>
<rect class="box" x="670" y="55" width="80" height="50" rx="6"/><text class="t b" x="710" y="78" text-anchor="middle">fsd</text><text class="m" x="710" y="95" text-anchor="middle">klony</text>
<rect class="warn" x="250" y="170" width="150" height="56" rx="6"/><text class="t b" x="325" y="193" text-anchor="middle">registr relací</text><text class="m" x="325" y="211" text-anchor="middle">náhradní konce</text>
<path class="ln dash" d="M400 198 L545 137" marker-end="url(#sh3)"/>
<path class="ln dash" d="M325 90 L325 168" marker-end="url(#sh3)"/>
<text class="m" x="332" y="140">Park / Claim</text>
<text class="m" x="10" y="262">Pád relace neshodí ostatní ani termd. Pád termd: relace a úlohy běží dál, nový termd je převezme z registru (Adopt)</text>
<text class="m" x="10" y="280">a okno se k nim připojí (Attach) s přehráním bloků.</text>
</svg>
<figcaption>Každá relace ve vlastním procesu; termd je jen prostředník, registr drží relace přes jeho restart.</figcaption>
</figure>

**Proces na relaci** (`libs/term/src/broker.rs`, `libs/term/src/ctl.rs`):

- termd (`user/termd`) nemá interpret. Přijímá zprávy okna a směruje je
  podle čísla relace do procesu `shsess` (`user/shsess`), jeden na relaci.
  Čísla bloků a výzev relace `N` leží v rozsahu `N << 32`, takže odpověď na
  výzvu (`Answer`) jde směrovat bez tabulky.
- **Pád relace** neshodí ostatní relace ani termd; okno dostane `Fail`
  „relace N skončila“.
- **Registr relací:** každá relace má kromě hlavního kanálu k termd
  náhradní kanál. Jeho druhý konec termd uschová v registru (`Park`).
  Registr drží proces, který termd spouští (dnes test v initu, později
  supervisor).
- **Konec nebo pád termd:** relace se odpojí. Rozběhnutý příkaz i úlohy na
  pozadí doběhnou a výstup se uloží do bloků. Nový termd si po startu
  vyzvedne náhradní konce (`Claim`), pošle na ně `Adopt` a relace přejdou
  na nový termd. Okno se pak připojí (`Attach`) s přehráním bloků. Bez
  registru relace doběhnou a skončí.
- **Klony bokem** mají ve jméně značku relace (`/tmp/<uid>/.aside/<značka>-<n>`,
  relace systému `/tmp/.aside/<značka>-<n>`).
  Nový termd zahodí klony relací, které už neběží (pád uprostřed řádku).
  Proto smí v systému běžet jen jeden termd (jeden registr).

⚠️ **Omezení:** `Sessions` hlásí pracovní složku z posledního `Attach`
(termd se relace neptá, aby neblokoval na zaneprázdněné relaci).


### Nativní protokol (`libs/term/src/proto.rs`)

Binární zprávy (1 B druh, pole little-endian, text = délka + UTF-8). Okno
posílá `Hello`, `Attach` (0 = nová relace), `Detach`, `Run`, `Interrupt`,
`Complete`, `Answer`, `Format`, `Sessions` a `Highlight`. Služba posílá
`BlockStart`, `Item`, `BlockEnd`, `Prompt`, `Completions`, `Formatted`,
`Highlighted`, `Cwd` (aktuální adresář po každém bloku, výzva se po `cd`
změní hned) a `Fail`.

Šedý návrh na konci řádku: u jména příkazu z historie, u argumentu
z doplňování podle typu argumentu (`cd` jen adresáře, cesty, pole tabulky)
- okno si ho samo vyžádá (`Complete` s příznakem `seq` 0x8000_0000).
Tab: jedna shoda se doplní rovnou, víc shod otevře nabídku, kterou psaní
zužuje (šipky/Tab výběr, Enter potvrdí, Esc zavře). V okně GUI je
nabídka sdílený seznam návrhů (`ui::complete`, vzhled `#compl`) ve vrstvě
popoveru nad řádkem vstupu, u začátku doplňovaného slova. Řádek si fokus
nechá: šipky, PgUp/PgDn a Home/End mění výběr, Enter nebo Tab doplní,
Esc nebo klik mimo zavře, klik na položku ji doplní. Výstup se posouvá
kolečkem a Shift+PgUp/PgDn (samotné PgUp je tabulka); když nejsem dole,
ukazuje to pruh „níže N řádků“, nový blok nebo psaní vrátí dolů. `help`
je přehled po skupinách (jméno a popis), podrobnosti v `help <příkaz>`;
řádek tabulky se láme jen v buňce popisu.

- **Párování odpovědí (verze 2):** `Complete` a `Format` nesou číslo
  požadavku `seq`, `Completions` a `Formatted` ho vrací. Mezi požadavkem
  a odpovědí můžou přijít události jiných bloků (úlohy `async`, `bg`,
  pozdní konec zrušené úlohy). Okno je proto páruje podle `seq` a odpověď
  na starší požadavek zahodí. Stejně funguje `seq` u `Highlight`.
- **Položky bloku:** typovaná hodnota, poznámka, varování, chyba (kód,
  popis, hlášení s místem), diff, průběh, odkaz.
- **Hodnoty se posílají typovaně.** Seznam záznamů se stejnými sloupci jde
  jako **tabulka se schématem**: jména sloupců jednou, pak řádky. Terminál
  ví, že `size` je velikost a `mtime` čas, a zobrazí je po svém (zarovnání,
  jednotky, místní čas). Na text se převádí až na konci.
- **VT100 jen jako adaptér** (`libs/term/src/vt.rs`): tabulky jako
  zarovnaný text, chyby červeně, poznámky tlumeně, odkazy jako OSC 8. Pro
  sériovou linku a převzaté programy.

### Relace a bloky

- Každý příkaz je **blok** (řádek, položky, výsledek, doba). Blok jde
  sbalit, zkopírovat, spustit znovu nebo porovnat dva běhy; to dělá
  terminál, služba drží data.
- **Relace patří službě, ne oknu.** `Detach` (zavření nebo pád okna) relaci nezruší:
  rozběhnutý příkaz doběhne, výstup se uloží do bloků a po `Attach` se
  přehraje. Dotaz bez připojeného okna se bezpečně odmítne.
- **Ctrl-C:** okno pošle `Interrupt`. Interpret se na přerušení ptá
  průběžně (příkazy, cykly, položky `where`/`each`, `sleep` po 50 ms).
- **Doplňování podle typů** (`Complete`): na místě příkazu jména příkazů
  a funkcí, za `-` přepínače z podpisu, za `$` proměnné, v podmínce `where`
  a u `sort`/`select`/`group`/`sum` pole vstupu (schéma z podpisu
  předchozího příkazu), jinde cesty ze souborového systému.
- **Historie s kontextem:** řádek, pracovní složka, výsledek, doba a čas
  (`history`, hledání `History::search` podle textu a složky).
- **Vysvětlení chyb:** `explain NOT_FOUND`, `explain SH_PARSE`, `explain -8`
  nebo text hlášky z logu. Texty se berou přímo z [chyby.md](chyby.md),
  z této stránky a z [hlasky.md](hlasky.md).
- **Převod zápisu** (`Format`): terminál může řádek přepsat do druhého zápisu.

### Obrazovka a písmo

`libs/term/src/screen.rs` drží textovou mřížku (rozumí výstupu adaptéru
VT100) a kreslí ji písmem Noto Sans Mono s diakritikou do lineárního
framebufferu. Od 1080 řádků se použije větší písmo. Stejná obrazovka bude
záchranná textová konzole.

### Barvy terminálu

> **Změna proti první verzi shellu:** terminál je barevný. Protokol nenese
> barvy ani ANSI, jen **sémantické třídy**. Barvu přidělí až motiv na konci
> cesty (okno, sériová linka, dokumentace). Zvýraznění bere tokeny
> z lexeru `libs/shlang` (jediný zdroj pravdy pro terminál i tento web).

**Dvě úrovně zvýraznění** (`shlang::highlight`):

- **Lexikální, hned v okně** při každé klávese, bez zprávy. Slovo na místě
  příkazu je příkaz, pak přepínače, řetězce, čísla, komentáře, klíčová
  slova a roury. Stejné zvýraznění má řádek bloku ve výpisu a příklady
  v dokumentaci.
- **Významové, od služby** (`Highlight` -> `Highlighted`, s pořadím
  `seq`). Služba zná příkazy, funkce, programy a proměnné relace.
  Neznámý příkaz dostane třídu `unknown` (podtržení už při psaní),
  proměnná `var`, pole v podmínce `where` `field`. Navíc vrátí místo chyby
  parseru. Nedopsaný řádek (otevřený řetězec, roura na konci) chybu
  nehlásí. Okno odpovědi se starším `seq` zahodí, takže písmena na
  odpověď nikdy nečekají.

| Třída | Co | Příklad |
|---|---|---|
| `cmd` | známý příkaz, funkce, program | `ls`, `where`, `.count()` |
| `unknown` | neznámý příkaz (podtržený) | `lss` |
| `kw` | klíčové slovo | `let`, `for`, `try`, `true` |
| `flag` | přepínač | `-r`, `--depth` |
| `var` | proměnná | `$f`, `it` |
| `field` | pole záznamu | `it.size`, `size` ve `where` |
| `str` | řetězec | `"ahoj"` |
| `interp` | závorky vloženého výrazu | `"{f.name}"` |
| `num` | číslo i s jednotkou | `5MB`, `2s`, `1.5` |
| `path` | cesta, vzor, cíl přesměrování | `~/foto`, `*.jpg` |
| `comment` | komentář | `# pozn.` |
| `op` | operátor, závorka | `>`, `&&`, `=>` |
| `pipe` | roura | `\|` |

```shell-b
ls ~/foto -r | where size > 5MB | sort mtime   # velké fotky
```

```shell-b
for f in (ls *.jpg) { echo "zpracovávám {f.name}" }
```

**Barevné výpisy** (`term::render`): tabulky se barví podle typu hodnot
a schématu, ne podle ANSI v programech.

- **Jméno souboru podle druhu:** `dir`, `dataset` (vnořený dataset: `kind`
  zůstává `"dir"`, pole `dataset = true`), `symlink`, `exec` (právo spustit `x` v poli
  `access`), `image`, `archive`, `document`, `media` (podle přípony),
  `hidden` (tečka), `noaccess` (bez práva číst).
- **Velikost a stáří v barevné škále** (4 stupně): velikost do 1 MB,
  100 MB, 1 GB a víc; stáří do dne, 30 dnů, roku a starší.
- `true` zeleně, `false` a chyby barvou chyby, `null` tlumeně, změny
  v diffu podle druhu.
- `ls` má nová pole `dataset` (kořen vnořeného datasetu) a `access` (práva relace `rwx` z fsd, `LIST_STAT_PAGE`
  je posílá u každé položky).

**Motiv** (`term::theme`): vestavěný tmavý (výchozí) a světlý. Uživatel je
přebije souborem `~/.config/term/theme.toml`; neznámá třída nebo špatná
barva je chyba s číslem řádku.

```toml
base = "dark"            # nebo "light"
[syntax]
cmd = "#7ee2a8"
unknown = "#ff8a80 underline"
[files]
dir = "#7fb4ff bold"
[ui]
bg = "#15171c"
scale3 = "#ff8a80"       # nejvyšší stupeň škály
[ext]
image = "jpg png heic"   # přípony k druhu souboru
```

**Konec cesty:** adaptér VT100 kreslí 24bitové barvy (`38;2;r;g;b`), tučné
a podtržení; obrazovka (`term::screen`) je umí vykreslit do framebufferu.
Barvy tříd na tomto webu (`sh-*`, `fk-*`) generuje `xtask` z téhož motivu.
Test hlídá, že zvýraznění v dokumentaci a v terminálu dávají stejné
úseky.

Příklad, který nejde převést do druhého zápisu (smíšený B a C, skript
s hlavičkou), má v dokumentaci jazyk `mos`: zvýrazní se stejně, jen bez
převodu.

⚠️ **Plán:** obarvení vlastního textu uživatelem (např. `echo` se stylem
třídy) je navazující návrh.

### Interaktivní terminál a konzole

> **Změna proti první verzi shellu:** terminál není „konzole ze 70. let“.
> Výsledky jsou interaktivní tabulky, doplňování je nabídka, průběh se
> mění na místě v reálném čase a výzvy jsou prvky protokolu. Logika je
> v knihovně `term::ui` (bez systémových volání, testy na hostiteli), takže
> stejně poslouží textové konzoli i oknu v GUI 6c.

<figure class="diagram">
<svg viewBox="0 0 760 230" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Konzole a interaktivni terminal">
<defs><marker id="sh4" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="140" height="56" rx="6"/><text class="t b" x="80" y="44" text-anchor="middle">inputd</text><text class="m" x="80" y="62" text-anchor="middle">fokus = klávesy</text>
<rect class="box" x="10" y="140" width="140" height="56" rx="6"/><text class="t b" x="80" y="164" text-anchor="middle">jádro</text><text class="m" x="80" y="182" text-anchor="middle">FRAMEBUFFER, FB_CONTROL</text>
<path class="ln" d="M150 48 L248 90" marker-end="url(#sh4)"/>
<path class="ln" d="M150 168 L248 130" marker-end="url(#sh4)"/>
<rect class="acc" x="250" y="60" width="220" height="100" rx="6"/><text class="t b" x="360" y="84" text-anchor="middle">user/console</text><text class="m" x="360" y="104" text-anchor="middle">term::ui (editor, tabulky,</text><text class="m" x="360" y="120" text-anchor="middle">nabídka, výzvy, průběhy)</text><text class="m" x="360" y="140" text-anchor="middle">term::screen -> framebuffer</text>
<path class="ln" d="M470 110 L568 110" marker-end="url(#sh4)"/>
<text class="m" x="478" y="102">nativní protokol</text>
<rect class="box" x="570" y="82" width="180" height="56" rx="6"/><text class="t b" x="660" y="106" text-anchor="middle">termd + shsess</text><text class="m" x="660" y="124" text-anchor="middle">relace, interpret</text>
<path class="ln dash" d="M360 60 L360 22" marker-end="url(#sh4)"/>
<text class="m" x="372" y="30">supervisor / test: display, focus, release</text>
</svg>
<figcaption>Konzole dostane displej a fokus od jejich vlastníka; sama si je nebere.</figcaption>
</figure>

**Protokol** (doplněk k nativnímu protokolu):

- `ItemUpdate { session, block, index, item }`: změna položky bloku na
  místě. Pruh průběhu se tak mění v reálném čase a rychlost a odhad
  zbývajícího času si okno spočítá samo z časů aktualizací.
- `Prompt { …, choices }` + `Choose { prompt, index }`: výzva s volbami
  (prázdné `choices` = ano/ne a odpověď `Answer`).
- Průběh hlásí interpret (`Host::progress`): `for` a `each` nad seznamem
  od 20 položek (aktualizace jen při změně procenta) a `sleep` od 1 s.

**Okno** (`term::ui::App`):

- **Editor řádku:** zvýraznění okamžitě, významové od služby podle `seq`;
  šedá nápověda z historie (šipka doprava nebo End ji přijme); historie
  šipkami; Ctrl-W slovo, Ctrl-U řádek, Ctrl-F převod B/C; Tab doplňování
  (jedna možnost se doplní rovnou, víc = nabídka).
- **Panel úloh:** Ctrl-T otevře seznam úloh (šipky, `c` zrušit, `p`
  pozastavit, `r` pokračovat, Esc zavřít); stavový řádek ukazuje počet
  běžících úloh a upozornění na dokončení nebo chybu.
- **Bloky:** každý příkaz s barevnou značkou stavu (běží, hotovo, chyba),
  dobou běhu a točícím se znakem u běžícího. Ctrl-L sbalí všechny,
  mezera sbalí vybraný.
- **Interaktivní tabulka:** PgUp z editoru vstoupí do poslední tabulky.
  Šipky a PgUp/PgDn vybírají řádek, `s` řadí podle dalšího sloupce, `S`
  obrátí směr, `/` filtr (podřetězec v libovolné buňce). Akce nad řádkem
  se spustí jako běžný řádek, takže platí historie, náhled, zámek
  i undo: Enter otevře soubor (`open`) nebo složku (`cd …; ls`), `c` vloží
  cestu do řádku a `d` smaže s náhledem (`preview { rm … }`).
- **Výzvy:** tlačítka Ano/Ne nebo volby, šipky a Enter, `a`/`n`,
  číslice u voleb.
- **Řádek zadaný před připojením** k relaci počká a odešle se po
  `Attached`.

```shell-b
for f in (ls ~/foto -r) { echo "kontroluji {f.name}" }
```

**Konzole** (`user/console`): textová a záchranná konzole pro běh bez
GUI, pád GUI a nouzový režim (v GUI je terminál okno `gui-term`). Displej
ani fokus si nebere sama. Předá jí je jejich vlastník (supervisor, zatím
test v initu) stejně jako kompozitoru. Řídicí příkazy jsou text
s odpovědí `ok` / `err KÓD` jako u inputd:

| Příkaz | Handly | Co |
|---|---|---|
| `display` | framebuffer | převzít displej (`FB_CONTROL` `CLAIM`) a kreslit |
| `focus` | kanál událostí od inputd | brát klávesy |
| `release` | - | přestat kreslit, vrátit displej jádru; odpověď nese framebuffer, kanál fokusu se zavře |
| `status` | - | `ok drawing=… focus=… session=… connected=…` |
| `dump` | - | `ok` + text obrazovky (testy, diagnostika) |

Kreslí do obrazu v paměti a do framebufferu kopíruje jen změněné textové
řádky (při psaní typicky jen řádek editoru), jen při změně nebo u
běžícího příkazu (nejvýš asi 7 snímků za sekundu). Jméno souboru v akci
nad řádkem je vždy text: vložený výraz `{…}` se escapuje.

⚠️ **Omezení:** myš (klik na řádek, odkaz) ještě není; displej a fokus
v běžném běhu rozdělí supervisor spolu s GUI 6c.

### Stav zapojení

| Co | Stav |
|---|---|
| jazyk, formátovač, interpret, undo, preview, zámek, needs | hotovo, testy na hostiteli (`cargo test -p shlang`) |
| protokol, služba relací, prostředník, registr, VT100, obrazovka, připojené cesty | hotovo, testy na hostiteli (`cargo test -p term`) |
| `termd` a `shsess` v initfs | hotovo; relace ve vlastních procesech, FS přes `fsproto` (klony bokem, pohledy, `LIST_STAT_PAGE`), výstup zrcadlí do logu |
| test v QEMU | hotovo: `user/init/src/tests/shell.rs` spustí termd s vlastní relací fsd a ověří zápis (koncept v klonu), undo přes pohled fsd, Ctrl-C, převod zápisu, přehrání bloků po odpojení a to, že relace i její úloha přežijí pád termd a nový termd je převezme z registru; spouštění termd mimo testy patří do supervisoru initu (později) |
| `async`/`await` | hotovo: úlohy ve vláknech `shsess` s vlastní relací fsd; testy `cargo test -p term` (vlákna std) a test v QEMU (souběh, undo úlohy, termín, zrušení) |
| vstup z klávesnice | hotovo: konzole dostane fokus od `inputd` (test v initu píše umělým vstupem) |
| kreslení do framebufferu | hotovo: `user/console`, framebuffer od jádra (`kind::FRAMEBUFFER`, `FB_CONTROL`); test v QEMU napíše `echo ahoj` a ověří obrazovku; spouštění konzole mimo testy patří do supervisoru |
| spouštění programů s vlastní relací fsd | hotovo: služba exec initu, programy podle jména přes launchd, síť podle manifestu, stdio a Ctrl+C (`user/init/src/tests/exec.rs`); mapování oprávnění `term::perm` (i z klonů pro `preview`) |

### Velké soubory: hodnota „soubor“

`open` souboru nad 64 KiB ho nečte celý: vrátí hodnotu typu `file` (cesta,
velikost, druh podle prvních 512 B, např. „obrázek BMP 1920×1080“). Obsah
načte až příkaz, který na vstupu potřebuje bajty nebo text (`from text`,
`lines`, `save`, `count`...); `describe`, zobrazení v terminálu a uložení do
proměnné obsah nečtou. Pole `size`, `kind` a `path` jdou číst přímo.
**[implementováno]** Proudové zpracování (roura, která čte velký text po
částech a skončí dřív, např. `lines | first 10`) **[navrženo]**.

### Řádky ve vláknech: Ctrl+Z a pozadí

Řádek relace běží ve vlastním vlákně procesu relace (`shsess`) nad
interpretem relace a vlastním spojením na fsd (stejná identita; koncept
a pohledy undo jsou objekty FS, ne spojení). Hlavní vlákno mezitím obsluhuje
okno: Ctrl+C, Ctrl+Z, výzvy, další řádky (fronta) a služby prostředí (zvuk,
agent, přehled úloh), které řádek ve vlákně nevolá sám. Kód:
`libs/term/src/fg.rs` (vlákno řádku) a `libs/term/src/session/line.rs`
(obsluha relací). **[implementováno]**

```text
okno ──Run/Interrupt/Background──▶ hlavní vlákno shsess ──▶ vlákno řádku
  ▲                                   │  schránka zpráv, výzvy,   (interpret
  └──────── bloky, výzvy ─────────────┘  požadavky na prostředí    relace)
```

- **Ctrl+Z** (zpráva `Background`): řádek běží dál jako úloha na pozadí
  a relace pokračuje kopií stavu ze začátku řádku (proměnné, funkce, složka,
  historie). Co úloha změní v proměnných a ve složce, se do relace
  nepromítne (jako subshell). Undo předchozích řádků zůstane úloze; relace
  po Ctrl+Z začíná s prázdným undo.
- **`příkaz &`**: rovnou na pozadí (úloha dostane kopii stavu, relace se
  nemění).
- **`jobs`, `fg N`, `kill N`**: řízení úloh relace; `fg` úlohu vrátí do
  popředí (Ctrl+C ji přeruší, Ctrl+Z vrátí zpět). `bg N` jen ohlásí, že
  úloha už běží (zastavená úloha zatím není).
- Konec úlohy na pozadí ohlásí řádek v jejím bloku: „[1] hotovo: …“,
  „[1] přerušeno: …“ nebo „[1] skončilo chybou: …“.
- Řádek napsaný během běhu jde do fronty (v okně viditelně) a spustí se po
  řádku v popředí, v pořadí.
- Bez vláken v prostředí (testy) běží řádek jako dřív v hlavním vlákně.

### Vzhled okna podle návrhu (design/terminal.html)

Okno `gui-term` kreslí text mřížkou znaků (stejný snímek jako konzole)
a podle rolí řádků (`term::ui::draw::Role`) nad ní dokreslí prvky návrhu
(`user/gui-term/src/rows.rs`). Snímek pro porovnání: `target/shots/terminal.png`
(test vzhledu `look_shots`).

| Prvek návrhu | Stav |
|---|---|
| levý pruh bloku podle stavu (běží, hotovo, chyba), příkaz tučně, čas a výsledek vpravo písmem UI | **[implementováno]** |
| rámeček chyby (danger-soft, okraj danger) | **[implementováno]** |
| řádky okna s vlastní výškou podle role (rozestupy z `design/terminal.html`): hlavička bloku 28 px, rámeček chyby a náhledu s okraji `s3`, patička. Řádky se skládají od spodního okraje nahoru, stavový řádek nahoře je připnutý a starší výstup se schová pod ním; plochu každého řádku dává v přístupnosti potomek `Row` uzlu terminálu | **[ověřeno v QEMU]** |
| patička okna (`.input-wrap`): linka přes celou šířku, okraj `s3`, řádek vstupu vysoký `control_h` (36 px) s pilulkou složky 28 px, pod ním o `s2` nápověda kláves; sedí u spodního okraje | **[ověřeno v QEMU]** |
| pilulka složky ve výzvě: ikona a celá cesta (domov jako `~`), šířka podle textu; řádek pro ni nechává buňky podle tokenů (`App::cwd_lead`). Delší cesta než třetina šířky se zkrátí zprostředka po celých složkách (`~/…/src/arch`), pak bez začátku (`…/b/c`), nakonec jméno poslední složky (`short_path`) | **[implementováno]** (zkracování: testy na hostiteli `tests_ui.rs`) |
| text řádku vstupu za pilulkou: začíná pevnou mezerou `s3` za skutečně nakreslenou pilulkou pro libovolně dlouhou cestu. Jméno se měří písmem, kterým se kreslí (proporcionální `small`, užší než stejný počet buněk mřížky), a okno řádek vstupu posune (`term::ui::pill::cwd_pill`, odsazení řádku mřížky). Kurzor, klik myší a výběr počítají x znaku stejně, takže sedí se znaky | **[ověřeno v QEMU]** (snímek s dlouhou cestou s diakritikou; geometrie, kurzor a klik: testy na hostiteli `tests_pill.rs`) |
| náhled změn (zámek nebezpečných operací, `preview`): žlutý pruh vlevo jako u bloku, rámeček (`.diffbox`) se záhlavím, seznamem změn (nejvýš 6, pak „… a N dalších“) a tlačítky `.btn.small` (28 px, hlavní s ikonou potvrzení) „Převzít N změn“ / „Zahodit“; tlačítka jsou i v přístupnosti, seznam změn ukazuje i konzole | **[ověřeno v QEMU]** |
| nápověda kláves jako štítky (`.kbd`): každá klávesa kombinace zvlášť (`Ctrl` `C`), plocha `surface`, okraj `border_strong` se silnější spodní hranou, písmo `caption`, mezi položkami `s4` | **[implementováno]** |
| linka pod záhlavím tabulky | **[implementováno]** (typ sloupce pod jménem zatím ne) |
| výběr textu myší (jako Windows Terminal): tah vybere znaky, dvojklik slovo (`ui::text::word_at`), trojklik řádky, Shift+klik rozšíří; barva `accent_soft`. Výběr drží buňky obrazovky a s novým výstupem se posune s textem, když se text vybraných řádků změní, zruší se (`libs/term/src/select.rs`). Psaní výběr zruší | **[ověřeno v QEMU]** (tah myší fyzickým vstupem, test `gui_term.rs`; dvojklik a trojklik testy na hostiteli `tests_select.rs`) |
| schránka: Ctrl+C při výběru a Ctrl+Shift+C kopírují (výběr pak zmizí), bez výběru Ctrl+C přeruší příkaz; Ctrl+V a Ctrl+Shift+V vloží na kurzor řádku vstupu. Vložení nic nespustí: poslední konec řádku se zahodí, vnitřní zůstanou (řádek je ukáže znakem ↵, Enter spustí vše jako jeden skript), tabulátor = mezera, řídicí znaky pryč, nejvýš 64 Ki znaků | **[ověřeno v QEMU]** (Ctrl+C a Ctrl+V z fyzické klávesnice, test `gui_term.rs`; schránka chce fyzické gesto) |
| klik do řádku vstupu posune kurzor na znak pod ukazatelem | **[implementováno]** (test na hostiteli) |
| doplňování jako nabídka nad řádkem: seznam návrhů `ui::complete::Completion` ve vrstvě popoveru (smí přesahovat okno do pracovní plochy), kotva = buňka začátku slova, záhlaví podle typu položek („Cesty“, „Příkazy“, ...), ikona a typ u položky. Řádek vstupu si drží fokus (combobox); v přístupnosti má stav `AUTOCOMPLETE`, s otevřeným seznamem `EXPANDED`, seznam je `ListBox` s položkami `ListOption` (ACTIVATE doplní). Položky a výběr drží `term::ui` (`App::menu`, `float_menu` vypne textové řádky nabídky) | **[ověřeno v QEMU]** (Tab a šipka z fyzické klávesnice, seznam nad řádkem, doplnění přes přístupnost: test `gui_term.rs`; klik myší jen test komponenty) |
| přepínač zápisu B / C v titulku (stav `App::notation_c`; klik, Ctrl+F i přístupnost převedou rozepsaný řádek) | **[ověřeno v QEMU]** |
| nový blok se prolne (`motion.base`, křivka `ease_enter`); bloky přehrané po připojení ne. Posun o 6 px z návrhu mřížka nedělá (prolnutí je zároveň náhrada pro „Omezit pohyb“) | **[implementováno]** |
| panel úloh vpravo (šířka `size.side_panel`): karty úloh se stavem, pruhem průběhu a tlačítkem Zrušit, v přístupnosti seznam. Sám se ukáže jen na obrazovce širší než 1280 px a v okně aspoň 900 px, jinak úlohy ukáže Ctrl-T. Výchozí okno 1040 px, nejvýš 85 % šířky pracovní plochy (`Window::request_size` před prvním vykreslením) | **[implementováno]** (testy v QEMU mají obrazovku 1280 px, panel ověřen jednorázově se sníženou hranicí) |
| karty relací v titulku: karta každé relace služby (ostatní se štítkem „odpojená“), tlačítko + pro novou relaci; klik i přístupnost přepnou relaci (odpojit a připojit jinou). Přehrání po přepnutí se nezasekne: na `AttachRecent` (tag 15, jen okno, které umí vkládat starší bloky; `Attach` přehraje vše v pořadí před `Attached`, např. pro adaptér VT) relace pošle nejdřív posledních 12 bloků, pak `Attached` a starší bloky po dávkách 16 od nejnovějších, okno je vloží před novější podle čísla bloku a zpracuje nejvýš 64 zpráv na snímek | **[ověřeno v QEMU]** |
| akce bloku: u bloku pod ukazatelem tlačítka Sbalit / Rozbalit a Spustit znovu (čas bloku se posune vlevo), v přístupnosti u všech viditelných bloků | **[ověřeno v QEMU]** (přes přístupnost; vzhled při najetí myší zatím bez snímku) |
| akce bloku „Kopírovat výstup“: výstup bloku jako prostý text bez barev (tabulka celá, ne jen viditelná část) do schránky (`Window::copy_text`); tlačítko mezi Sbalit a Spustit znovu, v přístupnosti „Kopírovat výstup <příkaz>“ | **[implementováno]** (text bloku test na hostiteli) |
| akce bloku „porovnat s předchozím během“: rozdíl výstupu proti poslednímu běhu stejného příkazu | **[navrženo]** (samostatný kus po kroku 5) |

### Ukazatel činnosti a průběh

Příkazy činnost nekreslí samy znaky: hlásí ji strukturovaně (běžící blok,
položka `Progress { done, total }`) a okno ji vykreslí jedním společným
modulem `libs/term/src/activity.rs`. Tím jdou všechny příkazy (`cp`,
stahování, scrub...) stejně.

| Kam se kreslí | Činnost | Průběh | Stav |
|---|---|---|---|
| okno terminálu (`gui-term`) a záložní konzole | kroužek s otáčejícím se obloukem v barvě akcentu (1 buňka, ~16 snímků/s) | vyhlazený pruh s dílčími buňkami po 1/8, procenta, rychlost, odhad času | **[implementováno]** |
| cizí terminál, znaková záloha | braillovy tečky `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏`, nejvýš 10 snímků/s | pruh z `▏▎▍▌▋▊▉█` | **[implementováno]** |
| „Omezit pohyb“ | statický `◌` (běží) / `●` (hotovo) | jen celé buňky | **[navrženo]** (režim je, systémový přepínač zatím ne) |
| výstup, který není terminál (soubor, skript) | žádná animace | jen konečný stav | **[implementováno]** |
| průběh od cizích programů přes OSC 9;4 | | jako výše | **[navrženo]** |

Okno a konzole kreslí kroužek i dílčí buňky samy, protože písma tyto znaky
nemají. Na drátě jsou to znaky ze soukromé oblasti Unicode (U+F0000
kroužek, U+F0100 dílčí buňka pruhu) a stejné krytí pixelů počítá
`activity::coverage` pro obě. Kroužek se překresluje jen tehdy, když něco
běží.

## Souběžnost: async a await

> **Hotovo (varianta A):** úloha `async { }`, čekání `await` (v C i
> `x.await`), `await all [...]`, `await any [...]`, časový limit `timeout`,
> zrušení `cancel` a paralelní `each --parallel N`. Klíčová slova jsou
> stejná v obou zápisech.

```shell-b
let a = async { ls ~/Fotky -r | where ext == "jpg" | count }
let b = async { ls ~/Dokumenty -r | count }
let (fotky, dokumenty) = await all [a, b]
let prvni = await any [a, b] timeout 5s
```

```shell-b
let c = async { open ~/data.json | from json }
let r = try await c
if r.failed { echo "nejde: {r.error.message}" }
```

```shell-b
ls ~/Fotky | each --parallel 4 f => { open $f.path | from text | lines | count }
```

Hodnota `async { }` je popisovač úlohy (záznam `{task: n}`); `await` vrátí
výsledek bloku. `await all` vrátí seznam v pořadí seznamu (rozloží ho
`let (a, b) = ...`), `await any` výsledek první hotové úlohy.

**Pravidla:**

- **Strukturovaná souběžnost.** Úloha nepřežije blok, ve kterém vznikla
  (skript, volání funkce): když blok skončí (i chybou nebo Ctrl-C), běžící
  úlohy se zruší a blok to oznámí poznámkou. Úloha z příkazové řádky patří
  relaci, takže na ni jde počkat i dalším řádkem. Výjimka je výslovné `bg`.
- **Chyby:** chyba úlohy se projeví při `await` a šíří se sama;
  `let r = try await a` ji vrátí jako hodnotu. `await all` skončí hned při
  první chybě a ostatní úlohy zruší.
- **Termín:** `await ... timeout 5s` po termínu vrátí chybu `TIMED_OUT`;
  úloha běží dál (`cancel` ji zruší).
- **Ctrl-C** během `await` zruší úlohy, na které se čeká.
- **Soubory:** úloha má vlastní koncept změn. Změny se převezmou, až úloha
  doběhne, a mají vlastní záznam `undo` v relaci. Zrušená nebo chybná úloha
  koncept zahodí, nic se nezmění. Zámek nebezpečných operací platí i pro
  úlohu (výzva v terminálu, bez okna se odmítne).
- **Oprávnění se nerozšiřují:** úloha dědí manifest `needs`, pracovní
  složku a zámek volajícího.
- **Kopie proměnných:** úloha vidí kopii proměnných z doby vzniku (pozdější
  změna ji neovlivní). Closure do úlohy předat nejde (`SH_TYPE`), definujte
  ji uvnitř `async { }`.
- **Terminál:** každá úloha má vlastní blok (`async ...`) s výstupem,
  průběhem a výsledkem. Bloky se po odpojení a připojení okna přehrají.

**Pod tím** (proces relace `shsess`):

- každá úloha běží ve **vlastním vlákně** procesu relace (`libos::thread`,
  zásobník 256 KB) s vlastním interpretem a vlastní relací fsd (`CONNECT`:
  stejná identita a cesty), takže koncept úlohy se nemíchá s řádkem,
- hodnoty jdou mezi vlákny zakódované stejně jako v protokolu terminálu
  (interpret používá `Rc`, které vlákna sdílet nesmí),
- vlákno úlohy hlásí výstup do sdílené tabulky a probudí hlavní vlákno
  **událostí jádra**; hlavní vlákno čeká na zprávu okna nebo na událost
  (`wait_many`), `await` čeká na událost s termínem. Žádné aktivní čekání,
- `cancel` nastaví příznak, který interpret úlohy čte (jako Ctrl-C).

```text
okno ──Run──▶ shsess (hlavní vlákno: interpret řádku)
                 │ async { } ─▶ vlákno úlohy (interpret, relace fsd, koncept)
                 │                 │ výstup, výzvy, výsledek ─▶ tabulka úloh
                 │ ◀── událost ────┘
                 └─▶ bloky úloh oknu, await vyzvedne výsledek a undo
```

⚠️ **Omezení:**

- hostitel bez vláken (testy interpretu) úlohy spustí **na místě**
  postupně; výsledky jsou stejné, jen bez souběhu,
- úloha nespouští programy (`exec` je zatím jen v hlavním vlákně),
- přepínání mezi běžícími úlohami v okně zatím není (bloky jsou pod sebou),
- dvě úlohy, které mění stejný soubor, skončí konfliktem `SH_CONFLICT`
  u té, která doběhne druhá (jako dva řádky v různých oknech).

| Varianta | Zápis | Proč ne |
|---|---|---|
| **A (zvoleno)** | `async`/`await`, `await all/any`, `each --parallel` | - |
| B | `&` a `wait` jako v bash | bez návratových hodnot, chyby se ztrácejí |
| C | jen `each --parallel`, `par { }` | nejde čekat na libovolnou ani první hotovou úlohu |

## Chyby shellu

Kódy chyb shellu (`SH_*`). Chyby systému (`NOT_FOUND`, ...) jsou
v [Kódech chyb](chyby.md). `explain KÓD` ukáže text odsud. Úplnost hlídá test
v `libs/shlang` (každý kód v cs i en).

### `SH_PARSE` (syntaxe)

**Význam:** zdroj nejde rozparsovat.

**Kdy vzniká:** chybí závorka nebo uvozovka, neznámý znak, `$` ve výrazu,
přesměrování na špatném místě, proměnná se jménem pole v podmínce `where`.

**Co s tím:** hlášení ukazuje řádek, sloupec a radu. `fmt --style c`
ukáže, jak parser příkaz pochopil.

### `SH_UNKNOWN` (neznámé jméno)

**Význam:** proměnná, pole, funkce nebo příkaz neexistuje.

**Kdy vzniká:** překlep, proměnná bez `let`, pole, které záznam nemá
(hlášení vypíše, jaká pole má).

**Co s tím:** rada nabídne nejpodobnější jméno („myslel jste `ls`?“).
Seznam příkazů: `help`.

### `SH_TYPE` (typ)

**Význam:** hodnota je jiného typu, než operace čeká.

**Kdy vzniká:** `size > 100` (chybí jednotka), `"a" + 1`, bajty do `where`
(chybí `from json`), záznamy do souboru (chybí `to text`), podmínka, která
není `bool`.

**Co s tím:** převeďte výslovně (`from json`, `to text`, `lines`) nebo
doplňte jednotku (`100MB`, `30s`).

### `SH_ARGS` (argumenty)

**Význam:** příkaz dostal špatné argumenty.

**Kdy vzniká:** chybí povinný argument, je jich moc, neznámý přepínač,
přepínač bez hodnoty, index mimo seznam.

**Co s tím:** rada ukáže použití příkazu; podrobně `help příkaz`.

### `SH_NO_MATCH` (vzor nic nenašel)

**Význam:** vzor souborů (`*.txt`) nenašel žádný soubor.

**Kdy vzniká:** ve složce nic takového není. Shell vzor nikdy potichu
nepředá jako text.

**Co s tím:** zkontrolujte složku a vzor; když je to záměr, ošetřete
`try` nebo `||`.

### `SH_INTERRUPTED` (přerušeno)

**Význam:** uživatel stiskl Ctrl-C.

**Kdy vzniká:** kdykoli během příkazu. Nejde zachytit `try` ani `||`.

**Co s tím:** nic; změny přerušeného řádku se převezmou jen do místa
přerušení (a jdou vrátit `undo`).

### `SH_NEEDS` (manifest)

**Význam:** skript nebo program chce něco, co mu manifest `needs`
nedovoluje.

**Kdy vzniká:** čtení nebo zápis mimo uvedené cesty, skript chce víc než
volající, program spuštěný uvnitř `preview`.

**Co s tím:** doplňte cestu do `needs { read: [...] write: [...] }`.
Program spusťte mimo `preview`.

### `SH_REJECTED` (odmítnuto)

**Význam:** uživatel změny nebo oprávnění odmítl.

**Kdy vzniká:** odpověď „ne“ v náhledu, u zámku nebezpečných operací nebo
u oprávnění skriptu; dotaz bez připojeného okna.

**Co s tím:** nic se nezměnilo. Spusťte znovu a potvrďte, nebo použijte
`preview` a prohlédněte si změny.

### `SH_NO_UNDO` (není co vrátit)

**Význam:** `undo` nemá žádný převzatý příkaz.

**Kdy vzniká:** v relaci zatím nic nezměnilo soubory nebo už bylo vráceno
vše (pamatuje se 16 příkazů).

**Co s tím:** nic.

### `SH_ARITH` (aritmetika)

**Význam:** dělení nulou nebo přetečení čísla.

**Kdy vzniká:** `x / 0`, výsledek mimo 64bitové celé číslo.

**Co s tím:** ošetřete dělitel; pro velká čísla použijte desetinná.

### `SH_USER` (chyba skriptu)

**Význam:** chybu vyvolal skript příkazem `error`.

**Kdy vzniká:** `error "zpráva"` bez `--code`.

**Co s tím:** podle zprávy skriptu.

### `SH_FORMAT` (formát dat)

**Význam:** data nejdou převést.

**Kdy vzniká:** neplatný JSON v `from json`, bajty, které nejsou UTF-8.

**Co s tím:** hlášení uvádí pozici v datech.

### `SH_LANG` (verze jazyka)

**Význam:** skript chce verzi jazyka, kterou tento shell neumí.

**Kdy vzniká:** `lang 2` a vyšší.

**Co s tím:** aktualizujte systém nebo skript přepište na `lang 1`.

### `SH_DEPTH` (hloubka)

**Význam:** příliš hluboké vnoření nebo rekurze.

**Kdy vzniká:** nekonečná rekurze funkce, extrémně vnořený výraz. Hlídá to
spotřeba zásobníku, ne počet volání.

**Co s tím:** přepište rekurzi na cyklus (`for`, `while`).

### `SH_OPEN` (otevření v aplikaci)

**Význam:** aplikace cestu neotevřela.

**Kdy vzniká:** `start` nebo holá cesta k souboru, když služba „otevři
cestu“ aplikaci nespustí, aplikace do 20 s neodpoví nebo akci odmítne.
Chybějící soubor nebo právo je `NOT_FOUND` / `ACCESS_DENIED`, ne `SH_OPEN`.

**Co s tím:** zpráva říká aplikaci a důvod. Obsah souboru ukáže `open`.

### `SH_EXIT` (program skončil s chybou)

**Význam:** spuštěný program skončil s nenulovým návratovým kódem.

**Kdy vzniká:** program ze systému (`ping`, `nc`, ...) ohlásí neúspěch
(cíl neodpověděl, spojení odmítnuto). Kód programu je ve zprávě.

**Co s tím:** výstup programu nad chybou říká důvod.

### `SH_CONFLICT` (konflikt)

**Význam:** část změn řádku se nepřevzala, protože se soubor mezitím
změnil i jinak (jiná relace, program).

**Kdy vzniká:** při převzetí konceptu řádku nebo náhledu. Převzetí je
trojcestné: proti stavu, ze kterého klon vznikl. Co se v původu změnilo jen
jinde, se převezme; co se změnilo na obou stranách, zůstane, jak to je
v původu.

**Co s tím:** ostatní změny jsou zapsané a `undo` je vrátí. Příkaz
zopakujte nad aktuálním stavem souborů ze zprávy.
