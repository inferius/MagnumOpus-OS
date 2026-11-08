# Příručka / User guide

Příručka pro běžné uživatele: co udělat a kde to najdete. Stránky jsou
rozdělené do skupin; text odkazu je název v menu webu (`cargo xtask docs`).
A guide for everyday users: what to do and where to find it. Pages are
grouped; the link text is the name in the site menu.

### Začínáme / Getting started

| Česky | English | Obsah / Contents |
|---|---|---|
| [Začínáme](cs/zaciname.md) | [Getting started](en/getting-started.md) | spuštění, přihlášení, první kroky / starting up, signing in, first steps |
| [Paleta příkazů](cs/paleta.md) | [Command palette](en/palette.md) | jedna zkratka ke všemu v systému / one shortcut to everything in the system |

### Práce se systémem / Using the system

| Česky | English | Obsah / Contents |
|---|---|---|
| [Plocha a lišta](cs/plocha.md) | [Desktop and bar](en/desktop.md) | lišta, spouštěč, hodiny, úlohy, oznámení / bar, launcher, clock, tasks, notifications |
| [Okna](cs/okna.md) | [Windows](en/windows.md) | přesun, velikost, přichycení, celá obrazovka, přepínání / move, resize, snap, full screen, switching |
| [Aplikace](cs/aplikace.md) | [Applications](en/apps.md) | spouštění aplikací a víc oken / starting applications and more windows |
| [Terminál pro začátečníky](cs/terminal.md) | [Terminal for beginners](en/terminal.md) | první příkazy, nápověda / first commands, help |
| [Soubory a verze](cs/soubory.md) | [Files and versions](en/files.md) | soubory, koš, historie verzí, vrácení změn / files, trash, version history, undoing changes |

### Nastavení a bezpečí / Settings and safety

| Česky | English | Obsah / Contents |
|---|---|---|
| [Nastavení](cs/nastaveni.md) | [Settings](en/settings.md) | vzhled, zabezpečení, přístupnost / appearance, security, accessibility |
| [Soukromí a oprávnění](cs/soukromi.md) | [Privacy and permissions](en/privacy.md) | odznak důvěry, indikátory, dotazy na oprávnění / trust badge, indicators, permission prompts |
| [Klávesové zkratky](cs/zkratky.md) | [Keyboard shortcuts](en/shortcuts.md) | přehled zkratek / shortcut overview |
| [Když něco nejde](cs/problemy.md) | [When something goes wrong](en/troubleshooting.md) | diagnostický snímek, hlášení, záchrana / diagnostic snapshot, report, recovery |

## Obrázky / Images

Obrázky v příručce jsou zatím diagramy (SVG ve stylu design systému). Na místech,
kde bude snímek obrazovky, je značka `<!-- snimek: nazev -->`. Snímky se mají
doplňovat automaticky z testů v QEMU (`screendump` přes QMP, jako diagnostický
snímek), uložit do `docs/prirucka/snimky/<nazev>-<motiv>.png` a značku nahradit
obrázkem; ruční snímky nepřidávat, zastarávají.

Images are diagrams for now (SVG in the design-system style). Where a screenshot
belongs there is a `<!-- snimek: name -->` marker. Screenshots are meant to be
filled in automatically from QEMU tests (`screendump` over QMP), saved to
`docs/prirucka/snimky/<name>-<theme>.png`, replacing the marker; do not add
manual screenshots, they go stale.
