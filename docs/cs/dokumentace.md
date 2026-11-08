# Psaní dokumentace

Pravidla, aby dokumentace odpovídala kódu: jasně odlišený cíl od
současného stavu, jedna stránka jako zdroj pravdy pro každou věc,
tvrzení podložená testy a měření s podmínkami.

## Značky stavu

Každá vlastnost, u které se cíl a současný stav můžou lišit, nese značku
přímo u sebe (v textu nebo v buňce tabulky):

| Značka | Význam | Co ji opravňuje |
|---|---|---|
| **[navrženo]** | popsaný cíl, kód zatím není | návrh je odsouhlasený |
| **[implementováno]** | kód je v `main`, automatický test chybí nebo běží jen na hostiteli | commit v `main` |
| **[ověřeno v QEMU]** | prochází test v `cargo xtask test` (x86_64 i aarch64, pokud se týká obou) | jméno testu u tvrzení |
| **[ověřeno na HW]** | vyzkoušeno na skutečném stroji | stroj, revize sestavení, datum |

Značka platí pro nejbližší tvrzení, ne pro celou kapitolu. Když se mění
kód, mění se i značka ve stejném commitu. Anglická verze používá
`[designed]`, `[implemented]`, `[verified in QEMU]`, `[verified on HW]`.

Příklad:

```markdown
- `MemoryObject`: stránky se alokují hned **[implementováno]**;
  copy-on-write klony **[navrženo]**.
- Kompozitor přežije pád ovladače displeje **[ověřeno v QEMU]**
  (test `gui: compd: pad ovladace displeje`).
```

## Jeden zdroj pravdy

Každá věc má **jednu vlastnickou stránku** (ovladač displeje
[Nastavení obrazu](displej.md), Venus [Vulkan přes Venus](venus.md),
plánovač [Plánovač](planovac.md) atd.). Jen tam jsou podrobnosti, čísla
a stav.

- **Souhrnné tabulky** (architektura, stav GUI, rozcestníky) uvádějí jen
  jednu větu, značku stavu a **odkaz** na vlastnickou stránku. Podrobnosti
  neopakují: zastarávají jako první.
- **Čísla se neopakují.** Syscally, práva, limity a velikosti jsou
  v [Syscally](syscally.md) nebo u kódu; jinde jen odkaz.
- **Stav milníku** je jen v [Architektura](architektura.md) (kapitola
  Aktuální odchylky od návrhu) a na stránce části.
- Kdo mění chování, upraví vlastnickou stránku a projde odkazy na ni
  (`grep` jména stránky v `docs/`), jestli souhrn pořád sedí.

## Tvrzení a testy

Důležité tvrzení (bezpečnost, obnova po pádu, výkon, formát na disku)
odkazuje na test, který ho ověřuje: jméno testu v `cargo xtask test`
nebo `cargo test -p <balík> <test>`. Tvrzení bez testu má nejvýš značku
**[implementováno]**.

## Měření

Každé číslo z měření uvádí:

- **revizi sestavení** (commit z řádku `sestaveni <commit> (<datum>)`
  v logu jádra), `*` znamená neuložené změny,
- **profil** (debug, release),
- **prostředí**: hostitel, QEMU s KVM/WHPX nebo bez akcelerace (TCG),
  VMware, skutečný stroj (model),
- **velikost úlohy** (rozlišení, počet souborů, velikost dat).

Příklad: „skládání 1280x800: 2,1 ms (release, `b24c01f`, QEMU + KVM,
Ryzen 7 5800X)“. Čísla bez podmínek se neporovnávají.

## Kontrola před sloučením

- značky stavu odpovídají kódu v této větvi,
- vlastnická stránka je aktuální, souhrny jen odkazují,
- nová tvrzení mají test, nová čísla podmínky,
- cs a en verze říkají totéž,
- `cargo xtask docs` projde. Kromě odkazů a kotev kontroluje značky stavu
  (neznámá značka je chyba) a vypíše varování: měření bez revize
  sestavení, cs a en verze s jiným počtem kapitol. Varování ze staré
  dokumentace postupně odstraňuje audit po dávkách sloučení.
