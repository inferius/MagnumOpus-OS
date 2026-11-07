# Návrh: jazyk shellu (k rozhodnutí)

> Stav: **návrh k diskusi**. Odsouhlaseno je zatím jen to, že jde o nový
> jazyk s typy, chyba je hodnota (`?` jako v Rustu), skript nese manifest
> oprávnění a POSIX `sh` je zvlášť ([rozhodnutí 6a-6d](architektura.md#rozhodnutí-k-milníkům-6a-6d-odsouhlaseno)).
> Tady je syntaxe a chování, které je potřeba vybrat.

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
4. **Chyby jsou hodnoty:** kód + popis + místo. `?` chybu vrátí výš,
   `try {...} else {...}` ji zpracuje. Neošetřená chyba ve skriptu ho
   zastaví (jako `set -e`, ale spolehlivě). Interaktivně se vypíše
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

## Doporučení: B s těmito pravidly

- **Příkazový režim:** `prikaz arg -p hodnota --dlouhy`. Slovo je text,
  `$x` je hodnota proměnné, `*.txt` rozvine shell podle FS (jen bez
  uvozovek). Argumenty mají typy z manifestu programu: chyba „čekám cestu,
  dostal jsem číslo“ přijde před spuštěním.
- **Výrazový režim:** za `let`, `if`, `for`, `while`, `return` a uvnitř
  `( )`. Čísla s jednotkami (`100MB`, `1y`, `30s`), porovnání, `&&`
  `||` `!`, řetězce v `"..."` s `{výraz}` uvnitř, seznamy `[1, 2]`, záznamy
  `{name: "a", size: 1KB}`.
- **Roura:** `|` předává záznamy (nebo bajty, podle manifestu). Běžné
  operace jsou krátká slova: `where`, `select`, `sort`, `group`, `first`,
  `count`, `each { |x| ... }`. Převody jsou výslovné: `from json`,
  `to text`, `lines`.
- **Chyby:** `prikaz?` chybu propustí výš, `try { } else |e| { }` ji
  zpracuje, `$e.code` a `$e.message` jsou k dispozici. `&&`/`||` se řídí
  úspěchem.
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
    image resize $f.path --width 1600 -o ~/Fotky/zmensene/{f.name}?
}
```

- **Bezpečné výchozí chování:** příkaz, který mění soubory, dostane
  líný snapshot (undo). Hromadná změna nad N souborů běží přes „ukaž, co se
  stane“ ([terminál](architektura.md#shell-a-chytrý-terminál-návrh)).

## Otevřené otázky k rozhodnutí

| # | Otázka | Varianty | Doporučení |
|---|---|---|---|
| 1 | základní styl | A / **B** / C | **B** |
| 2 | názvy příkazů | krátké (`ls`, `cp`) / `sloveso-podstatné` (`file-list`) / obojí (krátký alias) | krátké, plné jméno v nápovědě |
| 3 | proměnné v příkazu | `$x` / bez znaku (jen ve výrazech) | `$x` v příkazech, bez `$` ve výrazech |
| 4 | zápis funkce (closure) | `{ \|x\| ... }` / `fn(x) { ... }` / `x => ...` | `{ \|x\| ... }` (krátké v rouře), `fn jméno(x) { }` pro pojmenované |
| 5 | chyby | hodnoty + `?` / výjimky (Elvish) | hodnoty + `?` (odsouhlaseno) |
| 6 | čeština v příkazech | jen anglické názvy / lokalizované aliasy | anglické názvy, česká nápověda a chybové hlášky |
| 7 | bez `lang` ve skriptu | chyba / nejnovější verze | nejnovější verze + varování |
