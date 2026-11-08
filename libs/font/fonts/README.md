# Pisma / Fonts

- `Tuffy.ttf` - Tuffy Regular, Thatcher Ulrich (http://tulrich.com/fonts/),
  public domain. Zalozni pismo (latinka vcetne cestiny). Prevzato
  z testovacich pisem crate `fontdb` 0.24.
- `Inter-Light.ttf`, `Inter-Regular.ttf`, `Inter-Medium.ttf`,
  `Inter-SemiBold.ttf`, `Inter-Bold.ttf` - Inter 4.1 (rsms/inter, staticke
  rezy z `extras/ttf/`), SIL OFL 1.1 (`Inter-OFL.txt`). Pismo UI.
- `JetBrainsMono-Regular.ttf`, `JetBrainsMono-SemiBold.ttf` - JetBrains
  Mono 2.304, SIL OFL 1.1 (`JetBrainsMono-OFL.txt`). Terminal a kod.

Velikost: rez Interu ~0,41 MB, JetBrains Mono ~0,27 MB. Program si vlozi
jen rezy, ktere kresli (cargo vlastnosti `inter-*`, `mono-*` v `libs/font`).
