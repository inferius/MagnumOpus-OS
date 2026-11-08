# Začínáme

MagnumOpus je nový operační systém. Tahle příručka popisuje jen to, co už
opravdu funguje. Co se teprve chystá, je označené slovem *připravujeme*.

## Spuštění

Po zapnutí počítače systém naběhne rovnou na **plochu**: dole je lišta,
nahoře okna aplikací. Pokud se grafika nespustí, objeví se textová
konzole a systém se dá ovládat příkazy.

Přihlašovací obrazovka zatím není (*připravujeme*). Systém se přihlásí
automaticky. Účty s heslem, PINem a bezpečnostním klíčem už systém umí,
jen pro ně ještě chybí obrazovka.

<!-- snimek: plocha-po-startu -->

<figure class="diagram">
<svg viewBox="0 0 760 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Plocha po startu">
<rect class="box" x="10" y="10" width="740" height="280" rx="10"/>
<rect class="acc" x="60" y="40" width="340" height="170" rx="12"/><text class="t b" x="230" y="64" text-anchor="middle">okno aplikace</text><text class="m" x="230" y="84" text-anchor="middle">titulek s odznakem důvěry</text>
<rect class="box" x="300" y="80" width="360" height="160" rx="12"/><text class="t" x="480" y="160" text-anchor="middle">další okno</text>
<rect class="ok" x="20" y="250" width="720" height="32" rx="10"/>
<text class="t b" x="44" y="271">⊞</text><text class="m" x="80" y="271">aplikace</text><text class="m" x="560" y="271">úlohy · síť · zvuk · oznámení</text><text class="t b" x="700" y="271">14:05</text>
</svg>
<figcaption>Plocha: okna nahoře, lišta dole. Vlevo spouštěč a aplikace, vpravo stav a hodiny.</figcaption>
</figure>

## První kroky

1. **Otevřete paletu příkazů** klávesou `Ctrl+K`. Napište, co chcete
   udělat (třeba „terminál“), a stiskněte Enter. Paleta je nejrychlejší
   cesta ke všemu v systému, viz [Paleta příkazů](paleta.md).
2. **Spusťte aplikaci** kliknutím na její ikonu v liště nebo ve spouštěči
   (ikona mřížky vlevo dole).
3. **Okna** přesouváte tažením za titulek a velikost měníte tažením za
   okraj, viz [Okna](okna.md).

## Co je v systému teď

| Aplikace | K čemu je |
|---|---|
| Terminál | příkazy, práce se soubory, nápověda `help` |
| Správce úloh | co právě běží (zatím s ukázkovými daty) |
| Ukázka | ukázková aplikace s ovládacími prvky |

Správce souborů, Nastavení a další aplikace *připravujeme*.

## Vypnutí a restart

Otevřete paletu (`Ctrl+K`) a napište **Vypnout** nebo **Restartovat**.
Systém se zeptá na potvrzení. V terminálu totéž udělají příkazy
`shutdown` a `reboot`.
