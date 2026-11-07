# Jazyk shellu a chytrý terminál

> Stav: **odsouhlaseno a implementováno (milník 6b)** - [dva zápisy](#rozhodnutí-jeden-jazyk-dva-zápisy-b-a-c),
> [rozhodnutí k zápisu](#rozhodnutí-k-zápisu) a [rozhodnutí ke gramatice](#rozhodnutí-ke-gramatice).
> Jazyk popisuje [Gramatika](#gramatika-jazyka-lang-1) a [Sémantika](#sémantika),
> terminál [Terminál a služba relací](#terminál-a-služba-relací). Kapitoly
> o průzkumu a variantách ukazují, z čeho se vybíralo.
>
> Kód: `libs/shlang` (jazyk, `no_std` + `alloc`, testy na hostiteli),
> `libs/term` (protokol, relace, VT100, obrazovka), `user/termd` (služba).

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

```text
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

```text
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

```text
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
  líný snapshot (undo). Hromadná změna nad N souborů běží přes „ukaž, co se
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
  | velikost | `B`, `KB`, `MB`, `GB`, `TB` (×1000), `KiB`, `MiB`, `GiB`, `TiB` (×1024) | bajty |
  | doba | `ns`, `us`, `ms`, `s`, `min`, `h`, `d`, `w`, `y` (365 dní) | nanosekundy |

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
| `duration` | `30s`, `1h` | |
| `time` | `now()` | `time - duration` = `time`, `time - time` = `duration` |
| `text` | `"abc"` | |
| `bytes` | `open f` | **bajtový proud**, jiný typ než text a záznamy |
| `list`, `record` | `[1, 2]`, `{a: 1}` | seznam záznamů je tabulka |
| `closure` | `x => x * 2` | zachytí kopie použitých proměnných |
| `error` | `e` v `try ... else e => ...` | pole `code`, `message`, `hint`, `cause` |
| `null`, `nothing` | | `nothing` = příkaz nic nevrátil |

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

### Funkce, closure, úlohy

- `fn jméno(a, b) { ... }` vidí své parametry, globální proměnné a `input`
  (vstup z roury). Volá se jako příkaz (`jméno 1 2`, `jméno --b=2 --a=1`)
  nebo jako funkce (`jméno(1, 2)`).
- Closure zachytí **kopie** proměnných, které používá, v okamžiku vytvoření.
  Pozdější změna proměnné ji neovlivní.
- **Úloha na pozadí** (`cmd &`, `bg { }`) běží kooperativně: když je shell
  volný, nebo při `wait`. `jobs` je vypíše, `kill N` zruší čekající úlohu.
- **Hloubku rekurze** hlídá skutečná spotřeba zásobníku. Rozpočet nastaví
  hostitel; termd má zásobník 256 KB a interpret smí 160 KB. Při překročení
  vznikne `SH_DEPTH`, ne pád.

### Verze jazyka

`lang 1` platí pro oba zápisy. Skript bez `lang` poběží v nejnovější verzi
s varováním. Neznámá verze je chyba `SH_LANG`. Nekompatibilní změna =
nová verze; starý skript pak dál běží ve své.

## Vestavěné příkazy

Každý příkaz má krátké jméno a plné jméno `sloveso-podstatné` (alias).
`help` je vypíše, `help ls` ukáže použití a přepínače.

| Krátké | Plné | Co dělá |
|---|---|---|
| `ls` | `file-list` | soubory jako tabulka `{name, path, kind, size, mtime, ext}`; `-r`, `-a` |
| `cd`, `pwd` | `dir-change`, `dir-current` | pracovní složka |
| `open` (`cat`) | `file-read` | soubor jako bajty |
| `save` | `file-write` | uloží bajty nebo text; `-a` připojí |
| `rm`, `cp`, `mv`, `mkdir` | `file-remove`, `file-copy`, `file-move`, `dir-create` | `-r`, `-p` |
| `exists`, `df` | `file-exists`, `fs-info` | |
| `where`, `each`, `select`, `sort`, `first`, `last`, `count`, `sum`, `group`, `reverse`, `range` | `list-filter`, `list-map`, ... | práce s tabulkami a seznamy |
| `echo`, `log`, `lines`, `split`, `join`, `trim`, `upper`, `lower`, `contains`, `starts-with`, `ends-with`, `replace` | `text-...` | text |
| `from`, `to`, `describe` | `data-from`, `data-to`, `value-type` | výslovné převody (`json`, `text`, `lines`) |
| `now`, `sleep` | `time-now`, `time-sleep` | |
| `undo`, `preview` | `command-undo`, `command-preview` | viz [Soubory](#soubory-líné-undo-náhled-a-zámek) |
| `jobs`, `wait`, `kill` | `job-list`, `job-wait`, `job-kill` | úlohy na pozadí |
| `history`, `explain`, `help`, `run`, `error`, `exit` | `history-list`, `error-explain`, `help-show`, `script-run`, `error-raise`, `session-exit` | |

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

## Soubory: líné undo, náhled a zámek

<figure class="diagram">
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Zmeny souboru prikazu">
<defs><marker id="sh2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="80" width="130" height="56" rx="6"/><text class="t b" x="75" y="104" text-anchor="middle">příkaz</text><text class="m" x="75" y="122" text-anchor="middle">rm, save, mv ...</text>
<path class="ln" d="M140 108 L188 108" marker-end="url(#sh2)"/>
<rect class="acc" x="190" y="80" width="140" height="56" rx="6"/><text class="t b" x="260" y="104" text-anchor="middle">překryv</text><text class="m" x="260" y="122" text-anchor="middle">změny nanečisto</text>
<path class="ln" d="M330 108 L378 108" marker-end="url(#sh2)"/>
<rect class="box" x="380" y="80" width="120" height="56" rx="6"/><text class="t b" x="440" y="104" text-anchor="middle">diff</text><text class="m" x="440" y="122" text-anchor="middle">konec řádku</text>
<path class="ln" d="M500 95 L558 48" marker-end="url(#sh2)"/>
<path class="ln" d="M500 122 L558 172" marker-end="url(#sh2)"/>
<rect class="ok" x="560" y="20" width="190" height="56" rx="6"/><text class="t b" x="655" y="42" text-anchor="middle">≤ N souborů: převzít</text><text class="m" x="655" y="60" text-anchor="middle">nejdřív líný snapshot</text>
<rect class="warn" x="560" y="145" width="190" height="56" rx="6"/><text class="t b" x="655" y="167" text-anchor="middle">&gt; N nebo preview</text><text class="m" x="655" y="185" text-anchor="middle">ukázat diff, potvrdit</text>
<path class="ln dash" d="M655 145 L655 78" marker-end="url(#sh2)"/>
<text class="m" x="10" y="232">Čtecí příkaz nic nestojí (překryv je prázdný, snapshot nevznikne). undo vrátí poslední převzaté změny podle snapshotu.</text>
</svg>
<figcaption>Změny jdou do překryvu; převezmou se na konci řádku s líným snapshotem.</figcaption>
</figure>

- **Líné undo:** každý příkaz zapisuje do překryvu. Na konci řádku shell
  spočítá diff. Prázdný diff = nic (čtecí příkazy nestojí nic). Jinak
  shell **teprve teď** vytvoří snapshot (`shell-undo-N`, na FS v3 levný
  CoW) a změny převezme. `undo` vrátí poslední příkaz a funguje
  opakovaně (posledních 16 příkazů). FS bez snapshotů: shell si původní
  obsah změněných souborů drží v paměti.
- **„Ukaž, co se stane“:** `preview { ... }` spustí blok nanečisto, ukáže
  tabulku změn (`added`, `modified`, `removed`, `dir-added`, `dir-removed`,
  `replaced` = soubor nahrazený složkou nebo naopak)
  a převezme je až po potvrzení (`-y` bez dotazu). Odmítnutí = `SH_REJECTED`
  a nic se nezmění.
- **Zámek nebezpečných operací:** příkaz, který by změnil víc než N
  souborů mimo `/tmp` (výchozí N = 20), se před převzetím vždy zeptá. Je to
  stejný dotaz jako u `preview`. Bez připojeného okna se odmítne.
- **Manifest `needs`:** `run skript` před spuštěním ukáže, co skript chce
  (`read`, `write`, `net`), a po potvrzení ho omezí: přístup mimo uvedené
  cesty je `SH_NEEDS`. Skript dostane nejvýš průsečík s právy toho, kdo ho
  spouští: zápis jen tam, kam volající smí zapisovat, čtení tam, kde smí
  číst, síť jen se sítí. Jinak `SH_NEEDS` ještě před spuštěním. Úloha na
  pozadí (`bg`, `&`) si omezení nese s sebou, i když doběhne až po skončení
  skriptu.
- **Programy:** dostanou oprávnění z manifestu přes vlastní relaci fsd
  (`CONNECT_AS` s připojenými cestami, `term::perm::mounts`; zápis přebije
  čtení, cesty se připojí na stejné místo). Před spuštěním programu shell
  převezme dosavadní změny řádku (program vidí skutečný FS), a to **se
  stejným zámkem nebezpečných operací** jako na konci řádku: odmítnutí =
  program se nespustí. Pak udělá snapshot a zapamatuje si složky, kam
  program smí psát (s plnými právy celý FS). Když program nic nezměnil,
  záznam undo se zahodí, takže `undo` vrátí předchozí příkaz. Jinak `undo`
  ty složky srovná se snapshotem.

⚠️ **Omezení:** překryv drží zapsaná data v paměti až do konce řádku
(velké zápisy stojí paměť). Programy mimo shell do překryvu nevidí, proto je
`preview` odmítne (`SH_NEEDS`), dokud fsd nebude umět klony a diff
datasetů přes protokol (`libs/fs` je umí, `fsproto` zatím ne). Snapshot
přes `fsproto` je snapshot celého svazku, ne jen dotčeného datasetu.

## Terminál a služba relací

<figure class="diagram">
<svg viewBox="0 0 760 260" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Sluzba relaci">
<defs><marker id="sh3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="30" width="170" height="60" rx="6"/><text class="t b" x="95" y="55" text-anchor="middle">okno terminálu</text><text class="m" x="95" y="74" text-anchor="middle">(6c GUI, test, později SSH)</text>
<path class="ln" d="M180 60 L268 90" marker-end="url(#sh3)"/>
<text class="m" x="185" y="56">nativní protokol</text>
<rect class="acc" x="270" y="50" width="220" height="110" rx="6"/><text class="t b" x="380" y="75" text-anchor="middle">termd</text><text class="m" x="380" y="95" text-anchor="middle">relace: interpret, cwd,</text><text class="m" x="380" y="111" text-anchor="middle">historie, undo, úlohy</text><text class="m" x="380" y="127" text-anchor="middle">bloky výstupu</text><text class="m" x="380" y="143" text-anchor="middle">přežijí odpojení okna</text>
<path class="ln" d="M490 80 L578 60" marker-end="url(#sh3)"/>
<rect class="box" x="580" y="30" width="170" height="56" rx="6"/><text class="t b" x="665" y="54" text-anchor="middle">fsd</text><text class="m" x="665" y="72" text-anchor="middle">soubory, snapshoty</text>
<path class="ln" d="M490 125 L578 140" marker-end="url(#sh3)"/>
<rect class="box" x="580" y="115" width="170" height="56" rx="6"/><text class="t b" x="665" y="139" text-anchor="middle">VT100 adaptér</text><text class="m" x="665" y="157" text-anchor="middle">log / sériová linka</text>
<path class="ln dash" d="M380 160 L380 198" marker-end="url(#sh3)"/>
<rect class="box" x="270" y="200" width="220" height="44" rx="6"/><text class="t b" x="380" y="220" text-anchor="middle">obrazovka + vlastní písmo</text><text class="m" x="380" y="236" text-anchor="middle">framebuffer (capability)</text>
</svg>
<figcaption>Relace patří službě termd; okno se jen připojuje.</figcaption>
</figure>

### Nativní protokol (`libs/term/src/proto.rs`)

Binární zprávy (1 B druh, pole little-endian, text = délka + UTF-8). Okno
posílá `Hello`, `Attach` (0 = nová relace), `Detach`, `Run`, `Interrupt`,
`Complete`, `Answer`, `Format` a `Sessions`. Služba posílá `BlockStart`,
`Item`, `BlockEnd`, `Prompt`, `Completions`, `Formatted` a `Fail`.

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
- **Relace patří službě.** `Detach` (zavření nebo pád okna) relaci nezruší:
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

### Stav zapojení

| Co | Stav |
|---|---|
| jazyk, formátovač, interpret, undo, preview, zámek, needs | hotovo, testy na hostiteli (`cargo test -p shlang`) |
| protokol, služba relací, VT100, obrazovka, připojené cesty | hotovo, testy na hostiteli (`cargo test -p term`) |
| `termd` v initfs | hotovo; FS přes `fsproto`, výstup zrcadlí do logu |
| test v QEMU | hotovo: `user/init/src/tests/shell.rs` spustí termd s vlastní relací fsd a ověří zápis, undo přes snapshot fsd, Ctrl-C, převod zápisu a přehrání bloků po odpojení; spouštění termd mimo testy patří do supervisoru initu (později) |
| vstup z klávesnice | čeká na `inputd` (milník 6a, větev mimo `main`); do té doby posílá řádky okno nebo test |
| kreslení do framebufferu | knihovna hotová; čeká na předání framebufferu jako capability (jádro, `userboot`) |
| spouštění programů s vlastní relací fsd | mapování oprávnění hotové (`term::perm`); zatím žádný program s rozhraním pro shell |

## Souběžnost: async a await (rozhodnuto, zatím neimplementováno)

> **Odsouhlaseno (varianta A):** úloha `async { }`, čekání `await`,
> `await all [...]`, `await any [...]`, časový limit `timeout`, zrušení
> `cancel` a paralelní `each --parallel N`. Stejně v obou zápisech.
> Implementace přijde po barvách a interaktivitě terminálu.

```shell-b
let a = async { fetch https://example.com/data.json | from json }
let b = async { ls ~/Fotky -r | where ext == "jpg" | count }
let (data, pocet) = await all [a, b]          # počká na obě, chyba se šíří
let prvni = await any [a, b] timeout 5s       # první hotová, jinak TIMED_OUT
ls ~/Fotky | each --parallel 8 f => image resize $f.path --width 800
```

```shell-c
let a = async { fetch("https://example.com/data.json").from_json() }
let data = a.await
```

**Pravidla:**

- **Strukturovaná souběžnost.** Úloha nepřežije blok, ve kterém vznikla:
  když blok skončí (i chybou nebo Ctrl-C), běžící podúlohy se zruší.
  Nezůstanou tak zapomenuté procesy. Výjimka je výslovné `bg`, které úlohu
  předá službě relací.
- **Chyby:** chyba úlohy se projeví při `await` a šíří se sama;
  `let r = try await a` ji vrátí jako hodnotu.
- **Oprávnění se nerozšiřují:** úloha dědí manifest `needs` skriptu
  (průnik, stejně jako `bg`).
- **Terminál:** každá úloha má vlastní blok s průběhem, mezi běžícími
  úlohami jde přepínat.
- **Pod tím:** úloha je proces nebo vlákno s handlem, `await` čeká na jeho
  signál přes objekt **port** jádra (jedno vlákno čeká na mnoho událostí),
  `timeout` je časovač portu, `cancel` ukončí úlohu. Žádné aktivní čekání.

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
