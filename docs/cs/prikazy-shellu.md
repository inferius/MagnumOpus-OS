# Příkazy shellu

> Stránka je vygenerovaná z metadat příkazů v `libs/shlang/src/doc/` (`cargo xtask docs`) - neupravujte ji ručně. Stejná data ukazuje `help <příkaz>` v terminálu a příklady jsou testy: běží v testovací složce `/home` (soubory `a.txt`, `b.txt`, `data.json` a složka `docs`) a jejich výstup se porovnává.

## Rejstřík

- **Soubory:** [`ls`](#ls), [`cd`](#cd), [`pwd`](#pwd), [`open`](#open), [`start`](#start), [`save`](#save), [`rm`](#rm), [`cp`](#cp), [`mv`](#mv), [`mkdir`](#mkdir), [`exists`](#exists), [`df`](#df)
- **Text a výstup:** [`echo`](#echo), [`log`](#log), [`lines`](#lines), [`split`](#split), [`join`](#join), [`trim`](#trim), [`upper`](#upper), [`lower`](#lower), [`contains`](#contains), [`starts-with`](#starts-with), [`ends-with`](#ends-with), [`replace`](#replace)
- **Seznamy a tabulky:** [`where`](#where), [`each`](#each), [`select`](#select), [`sort`](#sort), [`first`](#first), [`last`](#last), [`count`](#count), [`sum`](#sum), [`group`](#group), [`reverse`](#reverse), [`range`](#range)
- **Převody a čas:** [`from`](#from), [`to`](#to), [`convert`](#convert), [`describe`](#describe), [`now`](#now), [`sleep`](#sleep)
- **Úlohy:** [`tasks`](#tasks), [`jobs`](#jobs), [`fg`](#fg), [`wait`](#wait), [`kill`](#kill), [`cancel`](#cancel)
- **Relace a bezpečnost:** [`undo`](#undo), [`preview`](#preview), [`history`](#history), [`run`](#run), [`error`](#error), [`exit`](#exit)
- **Nápověda:** [`help`](#help), [`explain`](#explain)
- **Systém:** [`audio`](#audio), [`vzhled`](#vzhled), [`povoleni`](#povoleni), [`system`](#system), [`loglevel`](#loglevel), [`report`](#report), [`install`](#install), [`shutdown`](#shutdown), [`reboot`](#reboot)

## Soubory

### `ls`

vypíše soubory ve složce jako tabulku

Vypíše obsah složky jako tabulku se sloupci jméno, cesta, druh, velikost a čas změny. Výstup je tabulka, takže ho jde dál filtrovat a řadit v rouře. Skryté soubory (jméno začíná tečkou) ukáže jen s `-a`. U složky je `size` logická velikost (součet souborů v podstromu, FS ho zná hned bez procházení) a `items` její obsah (`files` a `dirs` zvlášť); skutečně zabrané místo (sdílené bloky, deduplikace, komprese) ukáže připravovaný příkaz `disk` / `fs info`. Bez práva správy složky souhrn chybí.

```shell
ls [path] [-r] [-a]
```

Plné jméno `file-list` · vstup: nic · výstup: tabulka (name, path, kind, size, mtime, ext, access, dataset, items, files, dirs)

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `[path]` | cesta | pracovní složka | složka k vypsání |
| `-r, --recursive` | přepínač |  | i podsložky |
| `-a, --all` | přepínač |  | i skryté soubory (začínající tečkou) |

Jméno, velikost a u složky obsah.

```shell
ls | select name size items
```

```text
name       size    items
---------  ------  ---------
a.txt         5 B
b.txt        10 B
data.json    49 B
docs       3.0 KB  2 soubory
```

Jména souborů v pracovní složce.

```shell
ls | select name
```

```text
name
---------
a.txt
b.txt
data.json
docs
```

Jen velké soubory ve složce docs.

```shell
ls docs | where size > 1KB | select name
```

```text
name
-------
big.pdf
```

**Potřebuje:** čtení složky

**Chyby:** `NOT_FOUND`, `ACCESS_DENIED`

**Souvisí:** [`cd`](#cd), [`where`](#where), [`sort`](#sort), [`select`](#select)


### `cd`

změní pracovní složku (bez argumentu na domovskou)

Změní pracovní složku relace. Relativní cesty v dalších příkazech se počítají od ní; výzva terminálu ji hned ukáže. Bez argumentu přejde do domovské složky (`~`), `..` je nadřazená složka. Relace si pamatuje poslední složky: `cd -` vrátí do předchozí, `cd -2` o dvě zpět, `cd +` zase vpřed a `cd --history` je vypíše. V okně terminálu totéž dělají Alt+šipka vlevo a vpravo, když je řádek prázdný.

```shell
cd [path] [--history]
```

Plné jméno `dir-change` · vstup: nic · výstup: libovolná hodnota

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `[path]` | cesta | domovská složka | cílová složka; `-`, `-N` zpět, `+` vpřed |
| `--history` | přepínač |  | poslední složky relace (číslo pro `cd -N`) |

Přejde do složky docs a vypíše ji.

```shell
cd docs; pwd
```

```text
/home/docs
```

Návrat do předchozí složky.

```shell
cd docs; cd /tmp; cd -; pwd
```

```text
/home/docs
```

Poslední složky relace, nejnovější první.

```shell
cd docs; cd /tmp; cd --history | select path
```

```text
path
----------
/home/docs
/home
```

**Chyby:** `NOT_FOUND`, `SH_TYPE`, `SH_ARGS`

**Souvisí:** [`pwd`](#pwd), [`ls`](#ls)


### `pwd`

vrátí pracovní složku

Vrátí pracovní složku relace jako text.

```shell
pwd
```

Plné jméno `dir-current` · vstup: nic · výstup: text

Aktuální složka.

```shell
pwd
```

```text
/home
```

**Souvisí:** [`cd`](#cd)


### `open`

přečte soubor jako bajty

Přečte soubor jako bajty. Text z nich udělá až `from text` (žádný skrytý převod), tabulku z JSON `from json`. Soubor nad 64 KiB `open` nenačte celý: vrátí odkaz na soubor (cesta, velikost, druh podle začátku obsahu, např. obrázek BMP 1920×1080) a obsah přečtou až příkazy, které ho potřebují (`from text`, `lines`, `save`...). Terminál takový soubor ukáže jako souhrn; pole `size`, `kind` a `path` jdou číst přímo (`(open obr.bmp).size`).

```shell
open path
```

Plné jméno `file-read` · vstup: nic · výstup: bajty

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `path` | cesta |  | soubor ke čtení |

Obsah souboru jako text.

```shell
open a.txt | from text
```

```text
alfa
```

Záznamy z JSON a filtr v rouře.

```shell
open data.json | from json | where age > 18
```

```text
name  age
----  ---
Eva    31
```

**Potřebuje:** čtení souboru

**Chyby:** `NOT_FOUND`, `ACCESS_DENIED`

**Souvisí:** [`save`](#save), [`from`](#from), [`lines`](#lines)


### `start`

otevře soubor nebo složku v aplikaci podle typu (jako dvojklik)

Otevře soubor nebo složku v aplikaci podle typu, jako dvojklik ve Správci souborů (a `start` ve Windows). Aplikaci vybere služba „otevři cestu“ podle asociací v manifestech aplikací (`[open]`). Soubor, který žádná aplikace neotevře, ukáže Správce souborů; složku otevře Správce souborů. Holá cesta k souboru na příkazové řádce dělá totéž, jen bez Správce souborů: bez aplikace ukáže souhrn jako `open`. Obsah souboru čte `open`, ne `start`. V textové konzoli (bez grafického rozhraní) služba není a `start` skončí chybou `NOT_SUPPORTED`.

```shell
start path
```

Plné jméno `file-start` · vstup: nic · výstup: nic

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `path` | cesta |  | soubor nebo složka |

Otevře text v editoru.

```shell
start zprava.txt
```

Otevře složku ve Správci souborů.

```shell
start ~/Obrázky
```

**Potřebuje:** grafické rozhraní (služba „otevři cestu“)

**Chyby:** `NOT_FOUND`, `NOT_SUPPORTED`, `SH_OPEN`

**Souvisí:** [`open`](#open), [`ls`](#ls)


### `save`

uloží bajty nebo text do souboru (záznamy jen po `to text`/`to json`)

Uloží vstup z roury do souboru. Ukládá jen bajty a text; záznamy je potřeba nejdřív převést (`to json`, `to text`). Soubor se nahradí celý najednou: při chybě zůstane původní obsah. Změnu vrátí `undo`.

```shell
save path [-a]
```

Plné jméno `file-write` · vstup: libovolná hodnota · výstup: nic

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `path` | cesta |  | cílový soubor |
| `-a, --append` | přepínač |  | připojit na konec |

Uloží text a přečte ho zpět.

```shell
echo ahoj | save /tmp/x.txt; open /tmp/x.txt | from text
```

```text
ahoj
```

Tabulka do JSON souboru.

```shell
ls | select name | to json | save /tmp/seznam.json
```

**Potřebuje:** zápis do složky

**Chyby:** `SH_TYPE`, `ACCESS_DENIED`, `NO_SPACE`

**Souvisí:** [`open`](#open), [`to`](#to), [`undo`](#undo)


### `rm`

smaže soubory

Smaže soubory (složky jen s `-r`). Smazání jde vrátit příkazem `undo`; bezpečněji nejdřív `preview { rm ... }`.

```shell
rm ... [-r]
```

Plné jméno `file-remove` · vstup: nic · výstup: nic

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `...` | cesta |  | soubory ke smazání |
| `-r, --recursive` | přepínač |  | i složky s obsahem |

Smaže soubor a ověří, že zmizel.

```shell
rm a.txt; exists a.txt
```

```text
false
```

**Potřebuje:** zápis do složky

**Chyby:** `NOT_FOUND`, `ACCESS_DENIED`, `SH_ARGS`

**Souvisí:** [`preview`](#preview), [`undo`](#undo), [`mv`](#mv)


### `cp`

zkopíruje soubor

Zkopíruje soubor (složku s `-r`). Cíl může být i složka: kopie pak dostane stejné jméno.

```shell
cp from to [-r]
```

Plné jméno `file-copy` · vstup: nic · výstup: nic

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `from` | cesta |  | zdroj |
| `to` | cesta |  | cíl (soubor nebo složka) |
| `-r, --recursive` | přepínač |  | i složky |

Kopie a její obsah.

```shell
cp a.txt /tmp/kopie.txt; open /tmp/kopie.txt | from text
```

```text
alfa
```

**Potřebuje:** čtení zdroje, zápis do cíle

**Chyby:** `NOT_FOUND`, `ACCESS_DENIED`

**Souvisí:** [`mv`](#mv), [`rm`](#rm)


### `mv`

přesune nebo přejmenuje soubor

Přesune nebo přejmenuje soubor či složku. Uvnitř jednoho souborového systému je to okamžité.

```shell
mv from to
```

Plné jméno `file-move` · vstup: nic · výstup: nic

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `from` | cesta |  | zdroj |
| `to` | cesta |  | nové jméno nebo cílová složka |

Přejmenování.

```shell
mv a.txt c.txt; exists c.txt
```

```text
true
```

**Potřebuje:** zápis do obou složek

**Chyby:** `NOT_FOUND`, `ACCESS_DENIED`, `ALREADY_EXISTS`

**Souvisí:** [`cp`](#cp), [`rm`](#rm)


### `mkdir`

vytvoří složku

Vytvoří složku; s `-p` i chybějící nadřazené složky (a nevadí, když už existuje).

```shell
mkdir ... [-p]
```

Plné jméno `dir-create` · vstup: nic · výstup: nic

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `...` | cesta |  | složky k vytvoření |
| `-p, --parents` | přepínač |  | vytvořit i nadřazené složky |

Celá cesta najednou.

```shell
mkdir -p /tmp/a/b; exists /tmp/a/b
```

```text
true
```

**Potřebuje:** zápis do nadřazené složky

**Chyby:** `ALREADY_EXISTS`, `NOT_FOUND`, `ACCESS_DENIED`

**Souvisí:** [`ls`](#ls), [`rm`](#rm)


### `exists`

existuje soubor?

Vrátí `true`, když cesta existuje (soubor i složka), jinak `false`. Hodí se do `if`.

```shell
exists path
```

Plné jméno `file-exists` · vstup: nic · výstup: libovolná hodnota

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `path` | cesta |  | cesta |

Větvení podle existence.

```shell
if (exists a.txt) { echo je } else { echo neni }
```

```text
je
```

**Souvisí:** [`ls`](#ls), [`open`](#open)


### `df`

místo na souborovém systému

Celkové, použité a volné místo souborového systému, na kterém je cesta.

```shell
df [path]
```

Plné jméno `fs-info` · vstup: nic · výstup: tabulka (total, used, free)

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `[path]` | cesta | pracovní složka | cesta na souborovém systému |

Jen volné místo.

```shell
df | select free
```

**Chyby:** `NOT_FOUND`

**Souvisí:** [`ls`](#ls)


## Text a výstup

### `echo`

vrátí argumenty spojené mezerou

Vrátí argumenty spojené mezerou jako text. Výraz v `{...}` uvnitř uvozovek se dosadí.

```shell
echo ...
```

Plné jméno `text-print` · vstup: nic · výstup: text

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `...` | hodnota |  | hodnoty k vypsání |

Text z argumentů.

```shell
echo ahoj světe
```

```text
ahoj světe
```

Dosazení proměnné.

```shell
let n = 3; echo "je jich {n}"
```

```text
je jich 3
```

**Souvisí:** [`log`](#log), [`join`](#join)


### `log`

vypíše poznámku do terminálu

Vypíše poznámku do terminálu (tlumeně), ale nic nevrací do roury - hodí se do skriptů.

```shell
log ...
```

Plné jméno `text-log` · vstup: nic · výstup: nic

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `...` | hodnota |  | text poznámky |

Poznámka bez výstupu.

```shell
log začínám
```

**Souvisí:** [`echo`](#echo)


### `lines`

rozdělí text na řádky

Rozdělí text (nebo bajty v UTF-8) na seznam řádků. `\r\n` bere jako jeden konec řádku.

```shell
lines
```

Plné jméno `text-lines` · vstup: libovolná hodnota · výstup: seznam

Počet řádků souboru.

```shell
open b.txt | lines | count
```

```text
1
```

**Chyby:** `SH_TYPE`, `SH_FORMAT`

**Souvisí:** [`split`](#split), [`join`](#join), [`open`](#open)


### `split`

rozdělí text

Rozdělí text podle oddělovače na seznam.

```shell
split sep
```

Plné jméno `text-split` · vstup: text · výstup: seznam

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `sep` | text |  | oddělovač |

Seznam ze tří částí.

```shell
"a,b,c" | split ","
```

```text
a
b
c
```

**Chyby:** `SH_TYPE`

**Souvisí:** [`join`](#join), [`lines`](#lines)


### `join`

spojí seznam textů

Spojí seznam textů do jednoho, volitelně s oddělovačem.

```shell
join [sep]
```

Plné jméno `text-join` · vstup: libovolná hodnota · výstup: text

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `[sep]` | text | "" | oddělovač |

Spojení pomlčkou.

```shell
["a", "b"] | join "-"
```

```text
a-b
```

**Chyby:** `SH_TYPE`

**Souvisí:** [`split`](#split), [`echo`](#echo)


### `trim`

odstraní mezery na okrajích

Odstraní mezery a konce řádků na začátku a na konci textu.

```shell
trim
```

Plné jméno `text-trim` · vstup: text · výstup: text

Text bez okrajových mezer.

```shell
"  x  " | trim
```

```text
x
```

**Chyby:** `SH_TYPE`

**Souvisí:** [`replace`](#replace)


### `upper`

velká písmena

Převede text na velká písmena (i s diakritikou).

```shell
upper
```

Plné jméno `text-upper` · vstup: text · výstup: text

Velká písmena.

```shell
"čaj" | upper
```

```text
ČAJ
```

**Chyby:** `SH_TYPE`

**Souvisí:** [`lower`](#lower)


### `lower`

malá písmena

Převede text na malá písmena (i s diakritikou).

```shell
lower
```

Plné jméno `text-lower` · vstup: text · výstup: text

Malá písmena.

```shell
"ČAJ" | lower
```

```text
čaj
```

**Chyby:** `SH_TYPE`

**Souvisí:** [`upper`](#upper)


### `contains`

obsahuje text (nebo seznam) danou část?

U textu zjistí, zda obsahuje podřetězec, u seznamu, zda obsahuje prvek.

```shell
contains part
```

Plné jméno `text-contains` · vstup: libovolná hodnota · výstup: libovolná hodnota

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `part` | hodnota |  | hledaná část nebo prvek |

Podřetězec.

```shell
"beta" | contains "et"
```

```text
true
```

**Chyby:** `SH_TYPE`

**Souvisí:** [`starts-with`](#starts-with), [`ends-with`](#ends-with), [`where`](#where)


### `starts-with`

začíná textem?

Zjistí, zda text začíná daným textem.

```shell
starts-with prefix
```

Plné jméno `text-starts-with` · vstup: text · výstup: libovolná hodnota

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `prefix` | text |  | začátek |

Začátek jména.

```shell
"report.pdf" | starts-with "rep"
```

```text
true
```

**Chyby:** `SH_TYPE`

**Souvisí:** [`ends-with`](#ends-with), [`contains`](#contains)


### `ends-with`

končí textem?

Zjistí, zda text končí daným textem (třeba příponou).

```shell
ends-with suffix
```

Plné jméno `text-ends-with` · vstup: text · výstup: libovolná hodnota

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `suffix` | text |  | konec |

Přípona.

```shell
"report.pdf" | ends-with ".pdf"
```

```text
true
```

**Chyby:** `SH_TYPE`

**Souvisí:** [`starts-with`](#starts-with), [`contains`](#contains)


### `replace`

nahradí všechny výskyty

Nahradí všechny výskyty jednoho textu druhým.

```shell
replace from to
```

Plné jméno `text-replace` · vstup: text · výstup: text

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `from` | text |  | co nahradit |
| `to` | text |  | čím |

Všechny pomlčky.

```shell
"a-b-c" | replace "-" "+"
```

```text
a+b+c
```

**Chyby:** `SH_TYPE`

**Souvisí:** [`trim`](#trim), [`split`](#split)


## Seznamy a tabulky

### `where`

nechá řádky, které splní podmínku

Nechá jen řádky, které splní podmínku. V podmínce jsou sloupce řádku přímo jako jména (`size > 1MB`), celý řádek je `it`.

```shell
where cond
```

Plné jméno `list-filter` · vstup: libovolná hodnota · výstup: libovolná hodnota

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `cond` | podmínka |  | podmínka nad řádkem |

Jen textové soubory.

```shell
ls | where ext == "txt" | select name
```

```text
name
-----
a.txt
b.txt
```

**Chyby:** `SH_TYPE`, `SH_NO_MATCH`

**Souvisí:** [`select`](#select), [`sort`](#sort), [`each`](#each)


### `each`

použije funkci na každý řádek

Použije funkci na každý řádek a vrátí seznam výsledků. S `--parallel N` běží nejvýš N řádků souběžně jako úlohy.

```shell
each f [--parallel hodnota]
```

Plné jméno `list-map` · vstup: libovolná hodnota · výstup: libovolná hodnota

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `f` | funkce |  | funkce `x => ...` |
| `-p, --parallel` | číslo | 1 | nejvýš N řádků souběžně (úlohy) |

Každý prvek krát deset.

```shell
[1, 2, 3] | each x => x * 10
```

```text
10
20
30
```

**Chyby:** `SH_TYPE`

**Souvisí:** [`where`](#where), [`tasks`](#tasks)


### `select`

nechá jen vybrané sloupce

Nechá v tabulce jen vybrané sloupce (v uvedeném pořadí).

```shell
select ...
```

Plné jméno `record-select` · vstup: libovolná hodnota · výstup: libovolná hodnota

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `...` | text |  | jména sloupců |

Jméno a velikost (u složky součet podstromu).

```shell
ls | select name size
```

```text
name       size
---------  ------
a.txt         5 B
b.txt        10 B
data.json    49 B
docs       3.0 KB
```

**Chyby:** `SH_TYPE`

**Souvisí:** [`where`](#where), [`sort`](#sort)


### `sort`

seřadí řádky (podle sloupce)

Seřadí řádky vzestupně, podle sloupce nebo přímo hodnoty; `-d` sestupně.

```shell
sort [by] [-d]
```

Plné jméno `list-sort` · vstup: libovolná hodnota · výstup: libovolná hodnota

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `[by]` | text | hodnota | sloupec |
| `-d, --desc` | přepínač |  | sestupně |

Od největšího.

```shell
ls docs | sort size -d | select name
```

```text
name
---------
big.pdf
small.pdf
```

**Chyby:** `SH_TYPE`

**Souvisí:** [`first`](#first), [`reverse`](#reverse), [`group`](#group)


### `first`

první řádek (nebo prvních n)

Bez argumentu vrátí první prvek, s číslem seznam prvních n.

```shell
first [n]
```

Plné jméno `list-first` · vstup: libovolná hodnota · výstup: libovolná hodnota

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `[n]` | číslo | 1 | kolik |

První prvek.

```shell
[5, 6, 7] | first
```

```text
5
```

**Chyby:** `SH_TYPE`

**Souvisí:** [`last`](#last), [`sort`](#sort)


### `last`

poslední řádek (nebo posledních n)

Bez argumentu vrátí poslední prvek, s číslem seznam posledních n.

```shell
last [n]
```

Plné jméno `list-last` · vstup: libovolná hodnota · výstup: libovolná hodnota

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `[n]` | číslo | 1 | kolik |

Poslední dva.

```shell
[5, 6, 7] | last 2
```

```text
6
7
```

**Chyby:** `SH_TYPE`

**Souvisí:** [`first`](#first), [`reverse`](#reverse)


### `count`

počet řádků (nebo znaků textu)

Počet prvků seznamu nebo znaků textu.

```shell
count
```

Plné jméno `list-count` · vstup: libovolná hodnota · výstup: libovolná hodnota

Kolik je souborů.

```shell
ls | count
```

```text
4
```

**Chyby:** `SH_TYPE`

**Souvisí:** [`sum`](#sum), [`where`](#where)


### `sum`

součet (sloupce)

Sečte čísla, velikosti nebo doby; u tabulky daný sloupec.

```shell
sum [field]
```

Plné jméno `list-sum` · vstup: libovolná hodnota · výstup: libovolná hodnota

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `[field]` | text | hodnota | sloupec |

Velikost všech souborů složky.

```shell
ls docs | sum size
```

```text
3.0 KB
```

**Chyby:** `SH_TYPE`, `SH_ARITH`

**Souvisí:** [`count`](#count), [`group`](#group)


### `group`

seskupí řádky podle sloupce

Seskupí řádky podle hodnoty sloupce do tabulky se sloupci `key` a `items`.

```shell
group by
```

Plné jméno `list-group` · vstup: libovolná hodnota · výstup: tabulka (key, items)

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `by` | text |  | sloupec |

Druhy položek ve složce.

```shell
ls | group kind | select key
```

```text
key
----
file
dir
```

**Chyby:** `SH_TYPE`

**Souvisí:** [`sort`](#sort), [`sum`](#sum)


### `reverse`

obrátí pořadí

Obrátí pořadí prvků seznamu.

```shell
reverse
```

Plné jméno `list-reverse` · vstup: libovolná hodnota · výstup: libovolná hodnota

Pozpátku.

```shell
[1, 2, 3] | reverse
```

```text
3
2
1
```

**Chyby:** `SH_TYPE`

**Souvisí:** [`sort`](#sort)


### `range`

čísla od..do (bez horní meze)

Seznam celých čísel od `from` do `to` (horní mez se nezahrnuje).

```shell
range from to
```

Plné jméno `list-range` · vstup: nic · výstup: libovolná hodnota

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `from` | číslo |  | první číslo |
| `to` | číslo |  | mez (nezahrnuje se) |

Součet 1 + 2 + 3.

```shell
range 1 4 | sum
```

```text
6
```

**Chyby:** `SH_TYPE`

**Souvisí:** [`each`](#each)


## Převody a čas

### `from`

převede bajty/text na hodnoty (json, text, lines)

Převede bajty nebo text na hodnoty: `json` na záznamy a seznamy, `text` na text (UTF-8), `lines` na seznam řádků. Žádný převod se neděje skrytě.

```shell
from format
```

Plné jméno `data-from` · vstup: libovolná hodnota · výstup: libovolná hodnota

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `format` | text |  | json, text nebo lines |

JSON jako tabulka.

```shell
open data.json | from json | select name
```

```text
name
----
Eva
Jan
```

**Chyby:** `SH_FORMAT`, `SH_TYPE`

**Souvisí:** [`to`](#to), [`open`](#open)


### `to`

převede hodnoty na text (json, text)

Převede hodnoty na text: `json` (stroji čitelný), `text` (jako v terminálu). Potřeba před `save` u záznamů.

```shell
to format
```

Plné jméno `data-to` · vstup: libovolná hodnota · výstup: libovolná hodnota

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `format` | text |  | json nebo text |

Seznam jako JSON.

```shell
[1, 2] | to json
```

```text
[
  1,
  2
]
```

**Chyby:** `SH_FORMAT`

**Souvisí:** [`from`](#from), [`save`](#save)


### `convert`

převede jednotky, číselné soustavy, kódování textu a časová pásma

Univerzální převod. Jednotky z tabulky jednotek: `1GiB | convert MB` vrátí číslo v cílové jednotce, číslo bez jednotky převede `--from jednotka`. Soustavy `hex`, `bin`, `oct`, `dec`: celé číslo na text s předponou (`0xff`) a text s číslem zpět (`"ff" | convert dec --from hex`). Kódování `utf8`, `utf16le`, `utf16be`, `latin2` převede text na bajty, `base64` a `hex` bajty na text; zpět `--from kódování`. Čas převede do časového pásma (jméno IANA nebo posun `UTC+2`) jako místní čas s letním časem. Formáty dat (json, řádky) jsou `from` a `to`.

```shell
convert [target] [--from hodnota]
```

Plné jméno `value-convert` · vstup: libovolná hodnota · výstup: libovolná hodnota

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `[target]` | text |  | cílová jednotka, soustava, kódování nebo časové pásmo |
| `-f, --from` | text |  | zdrojová jednotka, soustava nebo kódování |

Dvojkové gigabajty na desítkové megabajty.

```shell
1GiB | convert MB
```

```text
1073.741824
```

Číslo šestnáctkově.

```shell
255 | convert hex
```

```text
0xff
```

Text v base64.

```shell
"ahoj" | convert base64
```

```text
YWhvag==
```

**Chyby:** `SH_ARGS`, `SH_TYPE`

**Souvisí:** [`from`](#from), [`to`](#to), [`describe`](#describe)


### `describe`

typ hodnoty

Vrátí typ hodnoty (text, číslo, velikost, tabulka...).

```shell
describe
```

Plné jméno `value-type` · vstup: libovolná hodnota · výstup: text

Typ velikosti.

```shell
1MB | describe
```

```text
size
```

**Souvisí:** [`to`](#to)


### `now`

aktuální čas

Aktuální čas (časová značka); rozdíl dvou časů je doba.

```shell
now
```

Plné jméno `time-now` · vstup: nic · výstup: libovolná hodnota

Teď.

```shell
now
```

**Souvisí:** [`sleep`](#sleep)


### `sleep`

počká

Počká danou dobu (`500ms`, `2s`); Ctrl-C čekání přeruší.

```shell
sleep time
```

Plné jméno `time-sleep` · vstup: nic · výstup: nic

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `time` | doba |  | doba |

Krátké čekání.

```shell
sleep 10ms
```

**Chyby:** `SH_INTERRUPTED`, `SH_TYPE`

**Souvisí:** [`now`](#now), [`tasks`](#tasks)


## Úlohy

### `tasks`

přehled úloh (vše na pozadí): tasks, tasks cancel|pause|resume|log <id>

Přehled všech úloh systému, které smíte vidět: úlohy shellu (`async`, `bg`), služby, aplikace, údržba FS i neznámé procesy. Bez akce vrátí tabulku; `cancel`, `pause`, `resume` úlohu řídí, `log` vrátí poslední řádky jejího výstupu. V okně terminálu je totéž jako panel Ctrl-T.

```shell
tasks [action] [id] [--lines hodnota]
```

Plné jméno `task-list` · vstup: nic · výstup: tabulka (id, name, state, owner, who, agent, progress, eta, cpu, memory, started, ended)

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `[action]` | text | výpis | cancel, pause, resume nebo log |
| `[id]` | číslo |  | číslo úlohy |
| `-n, --lines` | číslo | 20 | kolik řádků výstupu (tasks log) |

Jen služby.

```shell
tasks | where owner == "service"
```

Posledních 50 řádků výstupu úlohy 12.

```shell
tasks log 12 -n 50
```

**Potřebuje:** přehled úloh (taskd); cizí úlohy jen s právem

**Chyby:** `NOT_FOUND`, `NOT_SUPPORTED`, `ACCESS_DENIED`

**Souvisí:** [`jobs`](#jobs), [`cancel`](#cancel), [`wait`](#wait)


### `jobs`

úlohy na pozadí

Úlohy této relace na pozadí se stavem. Příkaz na pozadí spustí `&` na konci řádku (`cp velky.img /zaloha &`), běžící příkaz tam přesune Ctrl+Z. Úloha běží dál, relace přijímá další příkazy; konec úlohy se ohlásí řádkem v jejím bloku. `fg N` ji vrátí do popředí, `kill N` ukončí. Co úloha na pozadí změní v proměnných a ve složce, se do relace nepromítne.

```shell
jobs
```

Plné jméno `job-list` · vstup: nic · výstup: tabulka (id, state, command)

Kolik úloh běží na pozadí.

```shell
jobs | count
```

```text
0
```

**Souvisí:** [`fg`](#fg), [`kill`](#kill), [`tasks`](#tasks)


### `fg`

vrátí úlohu z pozadí do popředí (Ctrl+C ji přeruší, Ctrl+Z vrátí zpět)

Vrátí úlohu z pozadí (Ctrl+Z nebo `&`) do popředí: terminál na ni čeká jako na běžný příkaz, Ctrl+C ji přeruší a Ctrl+Z ji vrátí zpět na pozadí. Bez čísla vezme poslední úlohu. Funguje v terminálu, ve skriptu ne.

```shell
fg [id]
```

Plné jméno `job-foreground` · vstup: nic · výstup: nic

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `[id]` | číslo | poslední | číslo úlohy z `jobs` |

Úloha 1 zpět do popředí.

```shell
fg 1
```

**Chyby:** `SH_ARGS`

**Souvisí:** [`jobs`](#jobs), [`kill`](#kill)


### `wait`

počká na úlohu a vrátí její výsledek

Počká na úlohu (bez čísla na poslední) a vrátí její výsledek, nebo její chybu.

```shell
wait [id]
```

Plné jméno `job-wait` · vstup: nic · výstup: libovolná hodnota

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `[id]` | číslo | poslední | číslo úlohy |

Počká na úlohu 2 z `jobs`.

```shell
wait 2
```

**Chyby:** `NOT_FOUND`, `SH_INTERRUPTED`

**Souvisí:** [`jobs`](#jobs), [`kill`](#kill), [`cancel`](#cancel)


### `kill`

ukončí úlohu

Ukončí úlohu na pozadí podle čísla z `jobs` (příkaz spuštěný s `&` nebo přesunutý Ctrl+Z).

```shell
kill id
```

Plné jméno `job-kill` · vstup: nic · výstup: nic

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `id` | číslo |  | číslo úlohy |

Ukončí úlohu 3.

```shell
kill 3
```

**Chyby:** `NOT_FOUND`

**Souvisí:** [`jobs`](#jobs), [`cancel`](#cancel)


### `cancel`

zruší úlohu async (nebo seznam úloh)

Zruší úlohu `async` (nebo seznam úloh): úloha dostane přerušení a skončí chybou `SH_INTERRUPTED`.

```shell
cancel task
```

Plné jméno `task-cancel` · vstup: nic · výstup: nic

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `task` | hodnota |  | úloha nebo seznam úloh |

Zruší dlouhou úlohu.

```shell
let u = async { sleep 1h }; cancel $u
```

**Chyby:** `SH_TYPE`

**Souvisí:** [`tasks`](#tasks), [`wait`](#wait), [`kill`](#kill)


## Relace a bezpečnost

### `undo`

vrátí změny posledního příkazu

Vrátí změny souborů, které udělal poslední příkaz (snímek FS před ním). Funguje pro `save`, `rm`, `mv`, `cp`, `mkdir` i skripty.

```shell
undo
```

Plné jméno `command-undo` · vstup: nic · výstup: libovolná hodnota

Pak `undo` soubor vrátí.

```shell
rm a.txt
```

**Chyby:** `SH_NO_UNDO`, `SH_CONFLICT`

**Souvisí:** [`preview`](#preview), [`history`](#history)


### `preview`

spustí blok nanečisto, ukáže změny a po potvrzení je převezme

Spustí blok nanečisto nad kopií souborů, ukáže seznam změn a po potvrzení je převezme (`-y` bez dotazu). Když se mezitím soubory změnily jinde, převzetí odmítne.

```shell
preview body [-y]
```

Plné jméno `command-preview` · vstup: nic · výstup: libovolná hodnota

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `body` | blok |  | blok příkazů `{ ... }` |
| `-y, --yes` | přepínač |  | převzít bez dotazu |

Náhled smazání a převzetí.

```shell
preview -y { rm a.txt }
```

**Potřebuje:** souborový systém s klony (fsd)

**Chyby:** `SH_REJECTED`, `SH_CONFLICT`

**Souvisí:** [`undo`](#undo), [`rm`](#rm)


### `history`

historie příkazů s kontextem

Historie příkazů relace jako tabulka: řádek, složka, výsledek, doba a čas.

```shell
history
```

Plné jméno `history-list` · vstup: nic · výstup: tabulka (line, cwd, ok, duration, time)

Příkazy, které selhaly.

```shell
history | where ok == false | select line
```

**Souvisí:** [`undo`](#undo)


### `run`

spustí skript (kontroluje jeho manifest needs)

Spustí skript ze souboru s příponou `.mos` (MagnumOpus Shell). Hlavička skriptu (`lang 1`, `needs`) říká, co skript potřebuje; shell to ověří a zeptá se před spuštěním. Jiný soubor (log, text) `run` odmítne.

```shell
run path
```

Plné jméno `script-run` · vstup: nic · výstup: libovolná hodnota

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `path` | cesta |  | soubor skriptu (.mos) |

Uloží a spustí skript.

```shell
echo "echo ze skriptu" | save /tmp/s.mos; run /tmp/s.mos
```

```text
ze skriptu
```

**Potřebuje:** co skript uvede v `needs`

**Chyby:** `NOT_FOUND`, `SH_NEEDS`, `SH_PARSE`

**Souvisí:** [`error`](#error)


### `error`

vyvolá chybu

Vyvolá chybu s textem (a volitelně kódem); skript tím skončí, pokud chybu nezachytí `try`.

```shell
error message [--code hodnota]
```

Plné jméno `error-raise` · vstup: nic · výstup: nic

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `message` | text |  | text chyby |
| `--code` | text | SH_USER | kód chyby |

`try` chybu zachytí jako záznam (ok, failed, value, error).

```shell
(try (error "selhalo")).failed
```

```text
true
```

**Chyby:** `SH_USER`

**Souvisí:** [`explain`](#explain)


### `exit`

ukončí relaci

Ukončí relaci shellu (okno terminálu zavře kartu).

```shell
exit
```

Plné jméno `session-exit` · vstup: nic · výstup: nic

Konec relace.

```shell
exit
```


## Nápověda

### `help`

nápověda k příkazu

Bez argumentu přehled příkazů po skupinách, s příkazem jeho úplná nápověda: použití, argumenty, roura, příklady, chyby a související příkazy. Běžící příkaz přeruší Ctrl+C; co napíšete během běhu, čeká ve frontě a spustí se po něm (Esc frontu zruší).

```shell
help [command]
```

Plné jméno `help-show` · vstup: nic · výstup: text

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `[command]` | text |  | příkaz |

Nápověda k `ls`.

```shell
help ls
```

**Chyby:** `SH_UNKNOWN`

**Souvisí:** [`explain`](#explain)


### `explain`

vysvětlí kód chyby nebo hlášku z logu

Vysvětlí kód chyby (`NOT_FOUND`, `SH_PARSE`, `-8`) nebo hlášku z logu: co znamená a co s tím dělat.

```shell
explain code
```

Plné jméno `error-explain` · vstup: libovolná hodnota · výstup: text

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `code` | text |  | kód nebo hláška |

Co znamená chyba typu.

```shell
explain SH_TYPE
```

**Chyby:** `SH_UNKNOWN`, `SH_ARGS`

**Souvisí:** [`help`](#help), [`error`](#error)


## Systém

### `audio`

zvuk: tone [Hz], play soubor.wav, volume [%], mute, unmute, status, devices, device N

Ovládá zvuk: tón pro test, přehrání souboru WAV, hlasitost, ztlumení, stav a výběr zařízení.

```shell
audio action ... [--duration hodnota] [--level hodnota]
```

Plné jméno `audio-control` · vstup: nic · výstup: libovolná hodnota

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `action` | text |  | tone, play, volume, mute, unmute, status, devices nebo device |
| `...` | hodnota |  | hodnota akce (Hz, soubor, %, číslo zařízení) |
| `-d, --duration` | doba | 1s | délka tónu |
| `-l, --level` | číslo | 50 | úroveň tónu v % |

Krátký tón 440 Hz.

```shell
audio tone 440 -d 500ms
```

**Potřebuje:** zvuková služba (audiod)

**Chyby:** `NOT_FOUND`, `NOT_SUPPORTED`


### `vzhled`

vzhled: svetly, tmavy, auto (podle slunce); kontrast ano|ne, pohyb ano|ne; bez argumentu stav

Přepne vzhled celého prostředí: světlý, tmavý nebo automaticky podle východu a západu slunce v časovém pásmu. `kontrast ano` zapne vysoký kontrast (Přístupnost, přebije světlý i tmavý), `pohyb ano` omezí pohyb (přechod bez animace). Volba se uloží k účtu a platí hned i po dalším přihlášení. S `--system` mění volbu přihlašovací obrazovky pro všechny; to je úkon správy a potvrdí ho člověk s rolí správy v systémovém dialogu.

```shell
vzhled [what] [value] [-s]
```

Plné jméno `system-appearance` · vstup: nic · výstup: tabulka (schema, kontrast, omezit_pohyb)

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `[what]` | text |  | svetly, tmavy, auto, kontrast nebo pohyb |
| `[value]` | text |  | ano nebo ne (u kontrast a pohyb) |
| `-s, --system` | přepínač |  | systémová volba (přihlašovací obrazovka) |

Tmavý vzhled.

```shell
vzhled tmavy
```

Světlý přes den, tmavý v noci.

```shell
vzhled auto
```

Vysoký kontrast.

```shell
vzhled kontrast ano
```

**Potřebuje:** relace uživatele (s `--system` potvrzení správce)

**Chyby:** `SH_ARGS`, `ACCESS_DENIED`, `CANCELED`, `NOT_SUPPORTED`

**Souvisí:** [`audio`](#audio)


### `povoleni`

povolení aplikací: bez argumentu výpis, `povoleni odvolat <aplikace> [mikrofon]`

Bez argumentu vypíše uložená rozhodnutí uživatele o povoleních aplikací (dnes mikrofon). `povoleni odvolat <aplikace>` rozhodnutí smaže: aplikace se příště zeptá znovu.

```shell
povoleni
```

Plné jméno `system-permissions` · vstup: nic · výstup: nic

Aplikace se znovu zeptá na mikrofon.

```shell
povoleni odvolat org.libsdl.testaudio
```

**Potřebuje:** přihlášená relace (rozhodnutí patří uživateli relace)

**Chyby:** `SH_ARGS`, `NOT_FOUND`, `ACCESS_DENIED`, `NOT_SUPPORTED`

**Souvisí:** [`audio`](#audio)


### `system`

změna v /system (úkon správy, potvrdí člověk s rolí): `system write <cíl> <zdroj>`, `system rm <cesta>`, `system mkdir <cesta>`

Změní systémový soubor nebo složku v `/system`: `write` zapíše obsah souboru `zdroj` (nejvýš 60 KiB) do `cíl`, `rm` smaže soubor nebo prázdnou složku, `mkdir` založí složku. Lidé do `/system` jinak nesmí; tohle je úkon správy a potvrdí ho člověk s rolí správy v systémovém dialogu (stejně ve vydání i ve vývoji). `/system/accounts` (databáze účtů) nejde změnit nikdy.

```shell
system [op] [path] [source]
```

Plné jméno `system-file` · vstup: nic · výstup: nic

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `[op]` | text |  | write, rm nebo mkdir |
| `[path]` | cesta |  | cesta pod /system/ |
| `[source]` | cesta |  | soubor s obsahem (jen u write) |

Nové nastavení času z domova.

```shell
system write /system/config/ntpd.toml ntpd.toml
```

**Potřebuje:** relace uživatele a potvrzení člověka s rolí správy

**Chyby:** `SH_ARGS`, `ACCESS_DENIED`, `CANCELED`, `NOT_SUPPORTED`

**Souvisí:** [`vzhled`](#vzhled)


### `loglevel`

úroveň logu: bez argumentu výpis, `loglevel <modul|proces> <error|warn|info|debug|trace|vychozi>`

Bez argumentu vypíše úrovně logu; s modulem nebo procesem a úrovní ji změní (do restartu).

```shell
loglevel
```

Plné jméno `system-log-level` · vstup: nic · výstup: nic

Podrobný log služby fsd.

```shell
loglevel fsd debug
```

**Potřebuje:** právo DIAG

**Chyby:** `SH_ARGS`, `ACCESS_DENIED`

**Souvisí:** [`report`](#report)


### `report`

balíček pro vývojáře: logy, záznamy pádů a informace o stroji do jednoho souboru (vypíše cestu a obsah)

Sbalí logy, záznamy pádů a informace o stroji do jednoho souboru pro vývojáře a vypíše cestu a přehled obsahu.

```shell
report
```

Plné jméno `system-report` · vstup: nic · výstup: nic

Vytvoří balíček.

```shell
report
```

**Potřebuje:** právo DIAG

**Chyby:** `ACCESS_DENIED`

**Souvisí:** [`loglevel`](#loglevel)


### `install`

nainstaluje systém ze živého USB na disk (průvodce v krocích; s --odpovedi bez dotazů)

Průvodce instalací v krocích jako grafický instalátor: jazyk a klávesnice, kontrola počítače, kam instalovat, náhled změn disku, účet, zabezpečení, souhrn a průběh. Cizí oddíly nikdy nepřepíše; smazání celého disku chce opsat název disku. S `--odpovedi <soubor>` proběhne bez dotazů.

```shell
install [--odpovedi hodnota]
```

Plné jméno `system-install` · vstup: nic · výstup: nic

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `--odpovedi` | text |  | soubor odpovědí (instalace bez dotazů) |

Spustí průvodce.

```shell
install
```

**Potřebuje:** živé USB (instalátor)

**Chyby:** `SH_REJECTED`, `SH_USER`, `NOT_SUPPORTED`

**Souvisí:** [`report`](#report)


### `shutdown`

čistě vypne počítač (ukončí služby, pak ACPI S5)

Čistě vypne počítač: zeptá se (bez `-y`), ukončí služby a uloží data, pak ACPI S5.

```shell
shutdown [-y]
```

Plné jméno `system-shutdown` · vstup: nic · výstup: nic

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `-y, --yes` | přepínač |  | vypnout bez dotazu |

Vypnutí bez dotazu.

```shell
shutdown -y
```

**Potřebuje:** právo napájení

**Chyby:** `SH_REJECTED`, `ACCESS_DENIED`

**Souvisí:** [`reboot`](#reboot)


### `reboot`

čistě restartuje počítač

Čistě restartuje počítač (stejně jako `shutdown`, pak restart).

```shell
reboot [-y]
```

Plné jméno `system-reboot` · vstup: nic · výstup: nic

| Argument | Typ | Výchozí | Popis |
|---|---|---|---|
| `-y, --yes` | přepínač |  | restartovat bez dotazu |

Restart bez dotazu.

```shell
reboot -y
```

**Potřebuje:** právo napájení

**Chyby:** `SH_REJECTED`, `ACCESS_DENIED`

**Souvisí:** [`shutdown`](#shutdown)

