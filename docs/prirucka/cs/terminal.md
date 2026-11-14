# Terminál pro začátečníky

Terminál je okno, do kterého píšete příkazy. Výsledek se ukáže jako blok
pod příkazem, často jako přehledná tabulka.

## Nápověda

```text
help
help ls
```

`help` vypíše všechny příkazy, `help ls` popíše jeden z nich. Úplný seznam
příkazů je ve vývojářské části: [Příkazy shellu](../../cs/prikazy-shellu.md).
Když příkaz skončí chybou, napište `explain` a kód chyby: dozvíte se, co
znamená a co dělat.

## Pár užitečných příkazů

| Příkaz | Co dělá |
|---|---|
| `ls` | ukáže soubory v aktuální složce |
| `cd Dokumenty` | přejde do složky Dokumenty |
| `pwd` | napíše, ve které složce jste |
| `open poznamky.txt` | vypíše obsah souboru |
| `mkdir Projekty` | vytvoří složku |
| `cp a.txt b.txt`, `mv a.txt b.txt`, `rm a.txt` | kopírovat, přesunout, smazat |
| `undo` | vrátí poslední změnu souborů |
| `history` | dřívější příkazy |

Příklad: velké soubory, od největšího:

```text
ls ~/Dokumenty -r | where size > 100MB | sort size -d
```

## Nebojte se chyb

- Každý příkaz, který mění soubory, jde vrátit příkazem **`undo`**
  (posledních 16 příkazů).
- Když by příkaz změnil víc než 20 souborů, terminál se nejdřív zeptá.
- `preview { … }` ukáže, co by se stalo, a změny provede až po potvrzení.

## Ovládání

| Klávesa | Co udělá |
|---|---|
| `Tab` | doplní příkaz, cestu nebo název sloupce |
| `↑` `↓` | předchozí a další příkaz |
| `Ctrl+C` | zkopíruje vybraný text; bez výběru zastaví běžící příkaz |
| `Ctrl+V` | vloží text ze schránky do řádku (nic nespustí) |
| `Ctrl+L` | sbalí staré bloky výstupu |
| `Ctrl+T` | panel úloh |

Když je možností víc, Tab otevře nad řádkem seznam návrhů. Psaním ho
zúžíte, šipkami vyberete, Enter nebo Tab položku doplní, Esc seznam
zavře. Položku můžete i kliknout.

Myší: tažením vyberete text, dvojklikem slovo, trojklikem celý řádek.
Klik do řádku příkazu posune kurzor. Tlačítko „Kopírovat výstup“
v záhlaví bloku zkopíruje celý výstup příkazu.
