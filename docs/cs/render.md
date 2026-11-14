# Třída render a služba gpud

**Třída render** je společné rozhraní mezi klientem 3D (aplikace s Vulkanem,
kompozitor, testovací program) a ovladačem GPU. Rozhraní tvoří protokol
`libs/renderproto`, ploty jádra ([ploty.md](ploty.md)) a popis bufferu
([sdilene-buffery.md](sdilene-buffery.md)).

Render uzly rozdává služba **gpud**, ne vlastník displeje. Jsou to kroky 6a-3
a 6a-4 základů pro grafiku ([gpu-amd-nvidia.md](gpu-amd-nvidia.md#vlivy-na-základy),
bod 8).

## Stav

| Část | Co | Stav |
|---|---|---|
| `renderproto` | společná obálka: `INFO`, `QUERY`, `CREATE_CONTEXT`, `CREATE_BUFFER`, `IMPORT_BUFFER`, `DESTROY`, `BIND`, `SUBMIT` s ploty jádra | **[ověřeno v QEMU]** Venus ve WSL (sady `vulkan`, `gui`); kódování host testy |
| Venus | `SUBMIT` na frontu = `ring_idx`; dokončení jen přes ploty jádra; čekání na frontu Vulkanu (`Venus::wait_queue`) místo dotazování `vkGetFenceStatus` | **[ověřeno v QEMU]** WSL (`cargo xtask test --qemu-wsl --gpu venus`) |
| termín zaseknutí | `HANG_TIMEOUT` 10 s: výstupní ploty `FENCE_FAILED`, log `hang` | **[implementováno]** (bez testu: renderer v QEMU se nezasekne) |
| gpud | služba pod supervisorem; poskytovatele vydá ovladač jednou (`OPEN_PROVIDER`), init drží konec, gpud duplikát | **[ověřeno v QEMU]** (Venus ve WSL) |
| `IMPORT_BUFFER` u Venusu | buffer klienta (VMO s právem `DMA` + `bufdesc`) jako blob `GUEST` nad `DMA_MAP` | **[navrženo]** s NVK nebo skutečnou GPU (video v 6a-4 jde do kompozitoru přes okenní protokol, ne přes GPU); dnes `NOT_SUPPORTED` |
| oprávnění aplikací | manifest `[gpu] render = true`, kvóty na klienta, spojení přes lístky z launchd | **[navrženo]** (dnes render uzel otevírá jen init: kompozitor, testy) |
| obnova klientů gpud | po pádu gpud se klienti připojí znovu přes lístky (`libs/svcclient`) | **[navrženo]**; dnes gpud restartuje supervisor a klienty má jen init |

## Proč služba a ne displej

Do kroku 6a-2 otevíral render uzel vlastník displeje: init, pak kompozitor
(`OPEN_RENDER` na kanálu displeje). Kompozitor tak rozhodoval o přístupu ke
3D, ačkoli s displejem to nemá nic společného. Navíc by musel znát manifesty
aplikací.

Nově si init na kanálu displeje vyžádá **jednou** kanál poskytovatele
(`OPEN_PROVIDER`; další pokus vrátí `BAD_STATE`). Udělá to ještě předtím,
než displej dostane kompozitor, a poskytovatele předá službě **gpud**.

<figure>
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Render uzly přes gpud">
<style>
.box{fill:#f4f6fb;stroke:#4a5a80;stroke-width:1.2}
.acc{fill:#e8f1e4;stroke:#3f7a3a;stroke-width:1.2}
.t{font:13px sans-serif;fill:#1d2333}
.m{font:11px sans-serif;fill:#3a4256}
.b{font-weight:bold}
.ln{stroke:#4a5a80;stroke-width:1.2;fill:none}
</style>
<defs><marker id="rn1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path d="M0,0 L10,5 L0,10 z" fill="#4a5a80"/></marker></defs>
<rect class="box" x="10" y="20" width="200" height="70" rx="6"/>
<text class="t b" x="110" y="44" text-anchor="middle">init</text>
<text class="m" x="110" y="62" text-anchor="middle">OPEN_PROVIDER (jednou)</text>
<text class="m" x="110" y="78" text-anchor="middle">drží konec poskytovatele</text>
<rect class="acc" x="280" y="20" width="200" height="70" rx="6"/>
<text class="t b" x="380" y="44" text-anchor="middle">gpud</text>
<text class="m" x="380" y="62" text-anchor="middle">duplikát poskytovatele</text>
<text class="m" x="380" y="78" text-anchor="middle">OPEN -> render uzel</text>
<rect class="box" x="550" y="20" width="200" height="70" rx="6"/>
<text class="t b" x="650" y="44" text-anchor="middle">virtio-gpu</text>
<text class="m" x="650" y="62" text-anchor="middle">poskytovatel + render uzly</text>
<text class="m" x="650" y="78" text-anchor="middle">SUBMIT, ploty jádra</text>
<path class="ln" d="M210 55 L278 55" marker-end="url(#rn1)"/>
<path class="ln" d="M480 55 L548 55" marker-end="url(#rn1)"/>
<rect class="box" x="10" y="160" width="200" height="60" rx="6"/>
<text class="t b" x="110" y="184" text-anchor="middle">kompozitor, vk-probe</text>
<text class="m" x="110" y="202" text-anchor="middle">render uzel od initu</text>
<path class="ln" d="M210 190 L640 190 L640 92" marker-end="url(#rn1)"/>
<text class="m" x="420" y="182" text-anchor="middle">renderproto přímo k ovladači (pád gpud ho nepřeruší)</text>
</svg>
<figcaption>Poskytovatele drží init, render uzly rozdává gpud, klient mluví s ovladačem přímo.</figcaption>
</figure>

- **Pád gpud** render uzly nepřeruší: jdou přímo k ovladači. Supervisor
  gpud spustí znovu a nová instance dostane od initu další duplikát
  poskytovatele (`Kept`). Odpovědi, které poskytovatel poslal staré
  instanci, nová zahodí.
- **Pád ovladače GPU** zavře poskytovatele: nové render uzly už nebudou,
  dokud se nevyřeší obnova ovladače displeje.
- **Klienti dnes:** render uzel otevírá init (`init::render::open_render`).
  Kompozitor ho dostane s příkazem `backend gpu`, testovací program
  Vulkanu ve startovní zprávě. Aplikace s manifestem `[gpu] render = true`
  přijdou s launchd **[navrženo]**.

## Protokol `renderproto`

Jeden kanál = jeden klient = nejvýš jeden kontext (jako soubor render uzlu
DRM). Zavření kanálu zničí kontext i všechny buffery klienta; nedokončené
příkazy skončí `FENCE_FAILED`.

| Op | Pole | Odpověď | Venus (virtio-gpu) |
|---|---|---|---|
| `INFO` | - | třída ovladače, uAPI (id, verze, velikost), velikost sdílené paměti | uAPI = capsety |
| `QUERY` | klíč, a, b | data | `CAPSET` (a = id, b = verze): obsah capsetu |
| `CREATE_CONTEXT` | uAPI | `CONTEXT` | kontext s capsetem |
| `CREATE_BUFFER` | paměť, příznaky (`MAPPABLE`...), id ovladače, velikost | `BUFFER` (id, velikost, `map_info`) + VMO, když `MAPPABLE` | blob `HOST3D`, id ovladače = `VkDeviceMemory` |
| `IMPORT_BUFFER` | `bufdesc` + VMO v handlech | `BUFFER` | `NOT_SUPPORTED` (přijde s NVK) |
| `DESTROY` | buffer | `DONE` | odvolání okna, zničení blobu |
| `BIND` | data ovladače (`VM_BIND`) | `DONE` | nic (virtio-gpu nemá adresní prostor GPU) |
| `SUBMIT` | fronta, hodnoty plotů k čekání a k signalizaci, data (nejvýš 60 KiB, násobek 4) | `SUBMITTED` hned | fronta = `ring_idx` (0 = osa CPU, 1..63 = fronty Vulkanu) |

### `SUBMIT` a ploty

Ploty jdou v handlech zprávy: nejdřív ploty k čekání (právo `WAIT`), pak ploty
k signalizaci (právo `SIGNAL`), nejvýš 16 a 16. V datech zprávy jsou jen jejich
hodnoty. Ovladač:

1. příkaz přijme a odpoví `SUBMITTED`; dokončení hlásí jen ploty,
2. na ploty k čekání se registruje do svého portu (`FENCE_WAIT_ASYNC`).
   Příkaz pošle zařízení, až jsou všechny splněné. Pořadí v rámci fronty
   zůstává: příkaz nepředběhne dřívější příkaz stejné fronty, který ještě
   čeká,
3. po splnění plotu zařízení (virtio: plot na ose `ring_idx`) posune ploty
   k signalizaci. S prázdnými daty jde příkaz přes zařízení také: plot
   fronty se splní až po vší práci fronty odeslané předtím.

Chyby, kterými příkaz skončí:

| Co se stane | Výsledek |
|---|---|
| plot k čekání skončil chybou nebo byl opuštěný | příkaz se nepošle; ploty k signalizaci `FENCE_FAILED` |
| zařízení vrátí chybu | ploty k signalizaci `FENCE_FAILED`, log s chybou |
| čekání plus běh trvá déle než `HANG_TIMEOUT` (10 s) | ploty k signalizaci `FENCE_FAILED`, log `hang` |

`HANG_TIMEOUT` je konstanta třídy (`renderproto::HANG_TIMEOUT`):

- Windows (TDR) má 2 s, Linux (amdgpu, i915) kolem 10 s.
- 10 s nechává rezervu pro dlouhé výpočty a pomalé emulace (lavapipe ve
  WSL).
- Nikdo tak nečeká do nekonečna, ani když klient pošle cyklus plotů.

Klient smí mít nejvýš 16 nedokončených příkazů, pak dostane `SHOULD_WAIT`.

### Venus nad ploty jádra

`libs/venus` (`Venus`) má jeden plot jádra na spojení:

- **`call` a `send`** pošlou proud na frontu 0 (osa CPU) s další hodnotou
  plotu a čekají na ni s termínem.
- **`wait_queue(ring_idx)`** pošle prázdný `SUBMIT` na frontu svázanou
  s osou (`VkDeviceQueueTimelineInfoMESA`, vk-probe a vkcomp používají
  osu 1). Plot se splní, až fronta Vulkanu dokončí práci odeslanou
  předtím. Dotazování `vkGetFenceStatus` zmizelo.
- **Chyba plotu nebo vypršení** = spojení rozjeté (`Error::Broken`).

## Co se změnilo proti `gpuproto`

> **Změna proti `gpuproto` (6a-3): co a proč.** `gpuproto` byl protokol
> jen pro Venus.
>
> - **Nová obálka.** `GET_CAPSET` je teď `QUERY CAPSET`, `CREATE_BLOB` je
>   `CREATE_BUFFER` a `DESTROY_BLOB` je `DESTROY`. Obálka je společná pro
>   další ovladače (NVK, AMD).
> - **`SUBMIT` s plotem.** Dřív odpovídal až po zpracování a uměl jen osu
>   0, takže čekání na GPU šlo jen dotazováním. Teď odpovídá hned
>   a dokončení hlásí ploty jádra na libovolné frontě.
> - **`SCANOUT_BLOB` je pryč.** Byl to pokus kroku 4, při kterém render
>   klient přepisoval obraz kompozitoru. Zobrazení bez kopie patří
>   vlastníkovi displeje (compd) a přijde s třídou displej v2.
> - **`OPEN_RENDER` na displeji je pryč.** Nahradil ho jednorázový
>   `OPEN_PROVIDER` a gpud.

## Testy

| Test | Kde | Co ověřuje |
|---|---|---|
| `cargo test -p renderproto` | host | kódování všech zpráv, limity (počet plotů, velikost dat), poskytovatel |
| `cargo test -p bufdesc` | host | popis bufferu (viz [sdilene-buffery.md](sdilene-buffery.md)) |
| sada `vulkan` | QEMU ve WSL (`--qemu-wsl --gpu venus`) | render uzel od gpud, vk-probe: zařízení, obrázek s čekáním přes plot fronty, skládání |
| sada `gui` (`backend gpu`) | QEMU ve WSL | kompozitor s render uzlem od gpud, stejné pixely jako CPU |
| bez Venusu | QEMU Windows (výchozí) | gpud neběží nebo vrací `NOT_SUPPORTED`, testy se přeskočí, zbytek beze změny |
