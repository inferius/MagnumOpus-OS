# Když něco nejde

Systém je nový a něco v něm ještě nemusí fungovat. Tady je, jak zachytit,
co se stalo, abychom to mohli opravit.

## 1. Diagnostický snímek: `Ctrl+Alt+Shift+S`

Když se něco na obrazovce chová divně (okno se špatně vykreslí, něco
nereaguje), stiskněte **`Ctrl+Alt+Shift+S`**. Funguje i klávesa
**Print Screen** (přes vzdálenou plochu nebo virtuální stroj ale často
neprojde, proto raději tu první).

Systém uloží:

- obrázek obrazovky,
- popis všech oken a jejich stavu do záznamu (logu).

Najdete ho ve složce `/system/log/` jako `snimek-….bmp` (uchovává se
posledních 10). Snímek jde vytvořit i z palety: akce **Diagnostický
snímek**.

⚠️ Snímek obsahuje to, co bylo na obrazovce. Nikam se sám neposílá. Než
ho někomu pošlete, prohlédněte si ho.

## 2. Hlášení: příkaz `report`

Otevřete **Terminál** a napište:

```text
report
```

Vytvoří se soubor `/system/log/report-….txt` se souhrnem stavu systému
a posledními záznamy. Ten pošlete spolu se snímkem a krátkým popisem, co
jste dělali.

## 3. Záznamy (logy)

| Kde | Co tam je |
|---|---|
| `/system/log/boot-….log` | záznam posledních 20 startů systému |
| `/system/pady/` | záznamy o pádech systému a služeb |
| `/system/log/` | diagnostické snímky a hlášení |

## Systém z USB (živé USB)

Když systém běží z USB disku, ukládá kopie záznamů i na viditelný oddíl
**MO-LOGY** ve složce `MAGNUMOS`: `LOGxx.TXT` (logy), `PADxx.TXT` (pády),
`REPORT.TXT` (hlášení) a `SNIMxx.BMP` (snímky). Ten oddíl přečte
i Windows, takže soubory snadno zkopírujete a pošlete.

Když systém z USB vůbec nenaběhne: při startu, hned po výzvě zavaděče,
stiskněte do 2 sekund klávesu **L**. Systém jen vytáhne záznamy do složky
`ZACHRANA` na oddílu MO-LOGY a skončí.

Živé USB funguje jen na počítačích s UEFI a vypnutým Secure Boot. Disky
počítače z něj systém jen čte, nic na ně nezapisuje.

## Když systém spadne

Při vážné chybě se ukáže obrazovka s kódem chyby (třeba
`KERNEL_PANIC · #MO-7F3A-21C9`) a QR kódem. Počítač se sám restartuje po
60 sekundách. Kód chyby si opište nebo vyfoťte; záznam o pádu se uloží
a po dalším startu ho najdete v `/system/pady/`.

## Kam to poslat

Snímek, hlášení (`report-….txt` nebo `REPORT.TXT`) a krátký popis pošlete
správci projektu. Napište, co jste dělali těsně předtím, než se problém
objevil.
