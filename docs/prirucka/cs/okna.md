# Okna

Každé okno má nahoře **titulek**. Vlevo je **odznak důvěry** (kdo okno
patří a co aplikace smí), vpravo tlačítka **minimalizovat**,
**maximalizovat** a **zavřít**. Titulek a tlačítka kreslí systém, takže
fungují, i když se aplikace zasekne.

<!-- snimek: okno-titulek -->

<figure class="diagram">
<svg viewBox="0 0 760 120" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Titulek okna">
<rect class="box" x="10" y="10" width="740" height="100" rx="14"/>
<rect class="acc" x="10" y="10" width="740" height="40" rx="14"/>
<rect class="warn" x="22" y="17" width="130" height="26" rx="8"/><text class="m" x="87" y="35" text-anchor="middle">odznak důvěry</text>
<text class="t b" x="380" y="36" text-anchor="middle">název okna (sem chytněte myší)</text>
<text class="t" x="650" y="36">—</text><text class="t" x="684" y="36">□</text><text class="t" x="716" y="36">✕</text>
<text class="m" x="380" y="85" text-anchor="middle">obsah aplikace</text>
</svg>
<figcaption>Titulek okna: odznak vlevo, název uprostřed, tlačítka vpravo.</figcaption>
</figure>

## Přesun a velikost

- **Přesun:** chytněte okno za titulek a táhněte.
- **Velikost:** táhněte za kterýkoli okraj nebo roh okna.
- **Maximalizovat:** tlačítko □ v titulku. Dalším klikem se okno vrátí.
- **Minimalizovat:** tlačítko —. Okno zůstane v liště, kliknutím ho vrátíte.
- **Zavřít:** tlačítko ✕.

## Připnutí k polovině obrazovky

Podržte myš chvíli (asi půl sekundy) nad tlačítkem **maximalizovat**.
Objeví se nabídka:

| Volba | Co udělá |
|---|---|
| Maximalizovat | okno vyplní celou plochu nad lištou |
| Celá obrazovka | okno zabere celou obrazovku, lišta se skryje |
| Připnout vlevo | okno vyplní levou polovinu |
| Připnout vpravo | okno vyplní pravou polovinu |

Připnutí tažením okna k okraji a klávesami *připravujeme*.

## Celá obrazovka

- **Zapnout:** `Super+Shift+F`, v paletě akce **Celá obrazovka**, nebo
  nabídka nad tlačítkem maximalizovat.
- **Ukončit:** znovu `Super+Shift+F`, v paletě **Opustit celou
  obrazovku**, nebo najeďte myší k hornímu okraji: vysune se odznak
  s tlačítkem pro ukončení.

Na celé obrazovce se lišta skryje a oznámení se nezobrazují. Klávesa `F11`
patří aplikacím (např. přehrávači), systém ji nebere.

## Přepínání mezi okny

- Klikněte na ikonu aplikace v **liště**.
- Nebo otevřete **paletu** (`Ctrl+K`) a napište kus názvu okna.

Přepínání klávesami `Alt+Tab` *připravujeme*.

## Když aplikace nereaguje

Když aplikace 3 sekundy neodpovídá, systém její okno ztlumí a nabídne
**počkat** nebo **zavřít**. Ostatní okna i lišta běží dál a okno jde
dál přesouvat a zavřít.
