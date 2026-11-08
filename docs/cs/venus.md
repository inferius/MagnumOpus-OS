# Vulkan přes Venus

Vlastní minimální ovladač **Venus** (Vulkan přes virtio-gpu) v Rustu, bez
Mesy. Aplikace v systému serializuje volání Vulkanu do proudu příkazů
podle protokolu venus-protocol; renderer na hostiteli (virglrenderer
s render serverem) je dekóduje a volá skutečný Vulkan hostitele
(ve WSL softwarový **lavapipe**, na Linuxu s GPU ovladač GPU).

Stav: **kroky 1-3 hotové** - test v QEMU vypíše jméno fyzického zařízení
Vulkan od hostitele, vytvoří logické zařízení, obrázek vyplní barvou
(`vkCmdClearColorImage`) a pixely přečte zpět přes blob z `VkDeviceMemory`.
Další kroky viz [Plán](#plán).

<figure class="diagram">
<svg viewBox="0 0 780 420" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Vrstvy Venusu">
<defs><marker id="v1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="20" y="20" width="230" height="64" rx="6"/><text class="t b" x="135" y="44" text-anchor="middle">vk-probe (aplikace)</text><text class="m" x="135" y="64" text-anchor="middle">vkCreateInstance, vkEnumerate...</text>
<rect class="acc" x="20" y="110" width="230" height="110" rx="6"/><text class="t b" x="135" y="134" text-anchor="middle">libs/venus</text><text class="m" x="135" y="154" text-anchor="middle">vk: typy a příkazy (generované)</text><text class="m" x="135" y="172" text-anchor="middle">cs: kódování proudu</text><text class="m" x="135" y="190" text-anchor="middle">client: kontext, blob odpovědí,</text><text class="m" x="135" y="206" text-anchor="middle">volání s plotem</text>
<rect class="box" x="20" y="250" width="230" height="64" rx="6"/><text class="t b" x="135" y="274" text-anchor="middle">init / kompozitor</text><text class="m" x="135" y="294" text-anchor="middle">OPEN_RENDER na displeji</text>
<path class="ln" d="M135 84 L135 108" marker-end="url(#v1)"/>
<path class="ln dash" d="M135 250 L135 222" marker-end="url(#v1)"/><text class="m" x="142" y="241">kanál render uzlu</text>
<rect class="acc" x="300" y="110" width="210" height="140" rx="6"/><text class="t b" x="405" y="134" text-anchor="middle">user/virtio-gpu</text><text class="m" x="405" y="154" text-anchor="middle">render uzel (gpuproto)</text><text class="m" x="405" y="172" text-anchor="middle">CTX_CREATE s capsetem 4</text><text class="m" x="405" y="190" text-anchor="middle">RESOURCE_CREATE_BLOB</text><text class="m" x="405" y="208" text-anchor="middle">RESOURCE_MAP_BLOB</text><text class="m" x="405" y="226" text-anchor="middle">SUBMIT_3D s plotem</text>
<path class="ln" d="M250 160 L298 160" marker-end="url(#v1)"/><text class="m" x="256" y="152">SUBMIT</text>
<path class="ln dash" d="M298 200 L252 200" marker-end="url(#v1)"/><text class="m" x="256" y="216">VMO blobu</text>
<rect class="box" x="560" y="110" width="200" height="140" rx="6"/><text class="t b" x="660" y="134" text-anchor="middle">QEMU virtio-gpu</text><text class="m" x="660" y="154" text-anchor="middle">venus=on, blob=on</text><text class="m" x="660" y="172" text-anchor="middle">hostmem=256M: BAR se</text><text class="m" x="660" y="190" text-anchor="middle">sdílenou pamětí</text><text class="m" x="660" y="208" text-anchor="middle">(capability typ 8, id 1)</text>
<path class="ln" d="M510 180 L558 180" marker-end="url(#v1)"/><text class="m" x="514" y="172">virtqueue</text>
<rect class="ok" x="560" y="290" width="200" height="110" rx="6"/><text class="t b" x="660" y="314" text-anchor="middle">virglrenderer</text><text class="m" x="660" y="334" text-anchor="middle">render server (vkr)</text><text class="m" x="660" y="352" text-anchor="middle">dekóduje proud, volá</text><text class="m" x="660" y="370" text-anchor="middle">Vulkan hostitele (lavapipe)</text>
<path class="ln" d="M660 250 L660 288" marker-end="url(#v1)"/>
<rect class="warn" x="300" y="300" width="210" height="90" rx="6"/><text class="t b" x="405" y="324" text-anchor="middle">blob odpovědí</text><text class="m" x="405" y="344" text-anchor="middle">sdílená paměť hostitele</text><text class="m" x="405" y="362" text-anchor="middle">namapovaná v BARu;</text><text class="m" x="405" y="380" text-anchor="middle">renderer píše, klient čte</text>
<path class="ln dash" d="M558 345 L512 345" marker-end="url(#v1)"/>
<path class="ln dash" d="M300 345 L180 222" marker-end="url(#v1)"/>
</svg>
<figcaption>Vrstvy: aplikace, knihovna venus, render uzel ovladače, zařízení QEMU, renderer hostitele. Čárkovaně sdílená paměť.</figcaption>
</figure>

## Jak to běží

```bash
cargo xtask test --qemu-wsl --gpu venus
```

Test `vulkan` v initu (před sadou `gui`, dokud displej drží init):

1. `INFO` na displeji: má ovladač schopnost `VENUS`? Když ne, test se
   **přeskočí** - výchozí `cargo xtask test` na Windows QEMU (bez Venusu)
   projde na x86_64 i `--arch aarch64` beze změny.
2. `OPEN_RENDER` vrátí kanál render uzlu, init spustí `vk-probe` s tímto
   kanálem.
3. `vk-probe` přes `libs/venus` vytvoří instanci, vyjmenuje fyzická
   zařízení, přečte jejich vlastnosti a pošle řádek
   `ok <počet>|<jméno>|<typ>|<api>|<vendor:device>` (první test).
4. Pak (`user/vk-probe/src/clear.rs`, druhý test): logické zařízení
   s frontou na časové ose 1, obrázek 64x64 `R8G8B8A8_UNORM` v paměti
   zařízení, buffer v paměti `HOST_VISIBLE | HOST_COHERENT`, command
   buffer (bariéra, `vkCmdClearColorImage`, bariéra,
   `vkCmdCopyImageToBuffer`, bariéra pro hosta), `vkQueueSubmit` s plotem,
   čekání `vkGetFenceStatus`. Pixely se čtou přímo z bloba namapovaného
   z `VkDeviceMemory` bufferu; všech 4096 musí mít vyplněnou barvu.

Výpis z běhu ve WSL (lavapipe):

```text
virtio-gpu 00:06.0: 3D: virgl=1 blob=1 context_init=1 sdilena pamet=256 MiB capsety: virgl(v1, 308 B) virgl2(v2, 1408 B) venus(v0, 160 B)
vk-probe: renderer Venus, vk.xml 1.4.357, format proudu 1
vk-probe: zarizeni 0: llvmpipe (LLVM 20.1.2, 256 bits) (CPU), Vulkan 1.4.318, 10005:0000
vk-probe: rodin front 1, typu pameti 2, hald 1
init: test vulkan: fyzicke zarizeni Vulkan: llvmpipe (LLVM 20.1.2, 256 bits) (...)
[ok] vulkan: venus: jmeno fyzickeho zarizeni Vulkan od hostitele
vk-probe: obrazek: ok 64x64 4080bfff map_info=1 276 ms
[ok] vulkan: venus: obrazek vyplneny barvou a precteny zpet (pamet, command buffer)
```

`map_info=1` = paměť s cache (`MAP_CACHE_CACHED`): okno sdílené paměti
se mapuje jako běžná paměť (typ cache fyzické VMO, viz
[Syscally](syscally.md), `VMO_CREATE_PHYSICAL`).

Ve WSL zároveň selhávají testy sítě (pomocné servery běží na Windows
a z WSL nejsou vidět) a sada `gui` (displej GTK pod Xvfb nezapne výstup,
ovladač hlásí „zadny vystup“). Pro Venus to nevadí; plná sada běží
na Windows QEMU.

## Spojení s rendererem (`venus::client`)

| Krok | Zpráva / příkaz | Proč |
|---|---|---|
| 1 | `INFO` (render uzel) | capset Venus (4) a velikost sdílené paměti |
| 2 | `GET_CAPSET 4 v0` | `wire_format_version` = 1 a `supports_blob_id_0` |
| 3 | `CREATE_CONTEXT 4` | ovladač pošle `CTX_CREATE` s `context_init` = 4 |
| 4 | `CREATE_BLOB HOST3D, MAPPABLE, blob_id 0, 64 KiB` | renderer vytvoří sdílenou paměť (shm), ovladač ji namapuje do BARu a klientovi pošle VMO okna |
| 5 | `vkSetReplyCommandStreamMESA` | renderer bude odpovědi psát do toho blobu |
| volání | `vkSeekReplyCommandStreamMESA(0)` + příkazy s `GENERATE_REPLY` | `SUBMIT` s plotem na ose 0; odpovědi se čtou po kusech (4 KiB, pak 4x víc) |

**Synchronizace:** render server dekóduje proud asynchronně, odpověď na
`SUBMIT_3D` by mohla přijít dřív, než renderer odpovědi zapíše. Proto
ovladač posílá `SUBMIT_3D` s `FLAG_FENCE | FLAG_INFO_RING_IDX` a
`ring_idx` 0: osa 0 je u Venusu osa CPU, plot se splní, až renderer
zpracuje příkazy před ním (`vkr_context_submit_fence`). QEMU pošle
odpověď příkazu až po splnění plotu. Ring ve sdílené paměti
(`vkCreateRingMESA`) zatím není potřeba.

Příklad (z `user/vk-probe`):

```rust
let mut v = Venus::connect(render_channel)?;
let instance = vk::VkInstance(v.new_id()); // handle přiděluje klient
let app = vk::VkApplicationInfo { p_application_name: Some("vk-probe"), api_version: api_version(1, 1, 0), ..Default::default() };
let info = vk::VkInstanceCreateInfo { p_application_info: Some(&app), ..Default::default() };
let r = v.call(|e| cmd::create_instance(e, REPLY, &info, instance), cmd::decode_create_instance_reply)?;
check(r.ret)?;
```

## Kódování proudu (`venus::cs`)

Pravidla podle `vn_protocol_driver_types.h` (venus-protocol):

| Co | Na drátě |
|---|---|
| hlavička příkazu | `VkCommandTypeEXT` (i32), `VkCommandFlagsEXT` (u32) |
| skalár, enum, bitmaska | little-endian, vždy zarovnáno na 4 B; `uint8_t`/`uint16_t` zabere 4 B |
| `size_t`, `VkDeviceSize`, handle | 8 B; handle = id objektu, které přiděluje **klient** (i u výstupů jako `pInstance`) |
| ukazatel na jednu hodnotu | velikost pole 0/1 (u64), pak hodnota |
| pole (pevné i s délkou) | velikost u64, pak prvky; pole skalarů < 4 B je husté a doplněné na 4 |
| řetězec | velikost `strlen + 1`, bajty s NUL, doplnění na 4 |
| struktura s `sType` | sType, řetězec pNext (ukazatel, sType, zbytek řetězce, členy), členy |
| výstupní struktura v požadavku | „částečně“: jen sType, NULL pNext, id handlů a velikosti pevných polí |
| odpověď | typ příkazu, návratová hodnota (když není `void`), výstupní parametry |

Dekodér je bezpečný na libovolná data (čtení za koncem, obří pole
= chyba, ne panika); enkodér chyby (kratší pole než jeho délka, NUL
v řetězci, nepovolená struktura v pNext) zaznamená a vrátí z `finish`.

## Generátor (`cargo xtask venus-gen`)

Vstupy v repozitáři: `libs/venus/xml/vk.xml` (registr Vulkanu),
`VK_EXT_command_serialization.xml` (čísla příkazů) a
`VK_MESA_venus_protocol.xml` (příkazy transportu) z projektu
venus-protocol (licence v `libs/venus/xml/README.md`) a seznam
`libs/venus/subset.txt`:

```text
command vkCreateInstance          # příkaz (typy se doplní samy)
chain VkPhysicalDeviceIDProperties  # struktura povolená v řetězci pNext
```

Výstup `libs/venus/src/vk/` (soubory do ~1000 řádků): typy a konstanty
jako v C (`VK_SUCCESS`, `VkFormat` = `i32`, handle = `struct VkInstance(u64)`),
struktury (`encode`, `encode_partial`, `decode`) a příkazy v modulu `cmd`
(`cmd::create_instance`, `cmd::CreateInstanceReply`,
`cmd::decode_create_instance_reply`). Vstupní struktury mají odkazy
(`Option<&'a T>`, `&'a [T]`, `Option<&'a str>`), počty zůstávají jako
v C (`enabled_layer_count`).

`cargo xtask test` spouští `venus-check`: vygenerovaný kód musí odpovídat
vstupům (otisk vstupů v `target/venus-check.stamp`; beze změny se XML
znovu neparsuje). Generátor přebírá pravidla `vn_protocol.py`: nekonstantní
ukazatel parametru je výstup, výstup, který je délkou jiného parametru,
je vstup i výstup (`pPhysicalDeviceCount`), výstupní handle a struktury
jsou „částečné“.

Unie (jen vstup) jsou výčet s variantou pro každý člen; kóduje se index
členu (u32) a člen, renderer přijme kterýkoli
(`VkClearColorValue::Float32([..])`). Struktury řetězce pNext (`chain`
v `subset.txt`, např. `VkDeviceQueueTimelineInfoMESA`) jsou varianty
výčtu `Next` a pole `p_next` rodiče.

Zatím neumí (chyba při generování, ne tiché špatné kódování): unie ve
výstupu, řetězec pNext ve výstupu, vícerozměrná pole, ukazatele ve
výstupních strukturách.

## Render uzel (`gpuproto`)

Jeden kanál = jeden klient = nejvýš jeden kontext. Otevře ho vlastník
displeje (`guiproto::display` `OPEN_RENDER`) a předá dál.

| Op | Pole | Odpověď |
|---|---|---|
| `INFO` | - | capsety (id, verze, velikost), velikost sdílené paměti |
| `GET_CAPSET` | id, verze | obsah capsetu |
| `CREATE_CONTEXT` | capset | `CONTEXT` |
| `CREATE_BLOB` | paměť (`HOST3D`), příznaky (`MAPPABLE`...), blob_id, velikost | `BLOB` (resource, velikost, `map_info`) + VMO, když `MAPPABLE` |
| `DESTROY_BLOB` | resource | `DONE` |
| `SUBMIT` | ring_idx (zatím jen 0), plot, proud (nejvýš 60 KiB, násobek 4) | `SUBMITTED` (s plotem až po zpracování) |
| `SCANOUT_BLOB` | resource (0 = vrátit původní obraz), w, h, formát, krok řádku | `DONE`; pokus kroku 4, jen vlastní blob klienta |

Zavření kanálu: ovladač odvolá okna sdílené paměti (`VMO_REVOKE`),
odmapuje a uvolní bloby a zničí kontext.

**Asynchronní fronta.** Ovladač má 32 slotů (každý 72 KiB DMA paměti
pro příkaz a odpověď, dvojice deskriptorů). Synchronní příkazy (displej,
bloby, kontexty) čekají jen na svůj slot. `SUBMIT` s plotem se jen
odešle; odpověď `SUBMITTED` pošle hlavní smyčka, až zařízení slot vrátí
(přerušení je ve `wait_many` spolu s kanály). Pomalý renderer jednoho
klienta tak neblokuje displej ani jiné klienty. Dva sloty jsou vždy
volné pro synchronní příkazy; klient smí mít nejvýš 4 nedokončené
`SUBMIT` (pak `SHOULD_WAIT`).

## Rizika

Vyřešená rizika z kroku 1:

- **Odvolání.** Okno sdílené paměti je fyzická VMO a ovladač ji při
  `DESTROY_BLOB` a konci klienta odvolá (`VMO_REVOKE`): mapování zmizí
  i v procesu klienta, přístup přes staré mapování skončí výjimkou. Když
  odvolání selže, rozsah se už nikomu nepůjčí (karanténa).
- **Typ cache.** Fyzická VMO dostane typ cache podle `map_info` zařízení
  (QEMU hlásí `CACHED`); x86_64 PAT má položku WC, aarch64 Normal-NC.
- **Synchronní čekání.** Viz asynchronní fronta výše; klient čeká 40 s
  a po vypršení spojení označí za rozjeté (`Error::Broken`).

⚠️ Zbývá: jiný `ring_idx` než 0 ovladač odmítne - plot na ose, kterou
klient nesvázal s frontou, by se nesplnil a slot by zůstal obsazený.
Čekání na GPU proto zatím přes `vkGetFenceStatus`.

Bez KVM nebo s TCG (aarch64 ve WSL) Venus netestujeme; `vk.xml` má
3,3 MB (v gitu komprimovaný na zlomek).

## Plán

1. ✅ Jméno fyzického zařízení (`vkCreateInstance`, `vkEnumeratePhysicalDevices`,
   `vkGetPhysicalDeviceProperties`, fronty a typy paměti).
2. ✅ Paměť: `vkCreateDevice`, `vkGetDeviceQueue2` s časovou osou,
   `vkAllocateMemory`, blob s `blob_id` = id `VkDeviceMemory` namapovaný
   do klienta.
3. ✅ Obrázek a command buffer: `vkCreateImage`, `vkCmdClearColorImage`
   (unie v generátoru), `vkCmdCopyImageToBuffer`, `vkQueueSubmit`, plot,
   zpětné čtení pixelů z bloba.
4. Skládání kompozitoru přes Vulkan.

### Pokus: zobrazení bez kopie (`SET_SCANOUT_BLOB`)

Ideál pro kompozitor: grafická karta poskládá obraz do své paměti a ta se
rovnou zobrazí, bez kopie přes procesor. virtio-gpu na to má
`SET_SCANOUT_BLOB`; u virgl/Venus musí být blob podložený dmabuf
a displej QEMU ho importuje přes EGL.

Pokus (`vk-probe`, buffer z kroku 3 na výstupu 0 po dobu 0,5 s):

| Část | Výsledek ve WSL |
|---|---|
| export `VkDeviceMemory` jako dmabuf (lavapipe `VK_EXT_external_memory_dma_buf`) | funguje |
| `SET_SCANOUT_BLOB` v zařízení, vrácení původního obrazu | zařízení přijme, odpověď OK |
| vykreslení v displeji GTK pod Xvfb | **nejde**: `eglMakeCurrent failed` přesně při zobrazení i vrácení; softwarové EGL pod Xvfb dmabuf neimportuje (ve WSL chybí `/dev/dri` i `/dev/udmabuf`) |

Závěr: cesta bez kopie je správně na straně systému, ale ve WSL ji nejde
ověřit na obrazovce. Kompozitor přes Vulkan proto bude mít obě cesty:
bez kopie (Linux se skutečnou GPU) a se zpětným čtením snímku do bufferu
a dnešní 2D cestou (WSL a všude jinde).
