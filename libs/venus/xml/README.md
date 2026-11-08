# Vstupy generatoru Venusu

Zdroj pro `cargo xtask venus-gen` (vystup `libs/venus/src/vk/`). Soubory
jsou prevzate beze zmen z projektu venus-protocol
(https://gitlab.freedesktop.org/virgl/venus-protocol, commit
`02c03c815fd07f2237d7f1161bbafcf6a829373c`, adresar `xmls/`).

| Soubor | Puvod | Licence |
|---|---|---|
| `vk.xml` | registr Vulkanu (Khronos Group, Vulkan-Docs), kopie ve venus-protocol | Apache-2.0 OR MIT (`SPDX` v hlavicce souboru) |
| `VK_EXT_command_serialization.xml` | venus-protocol (Google LLC): cisla prikazu `VkCommandTypeEXT` | Apache-2.0 OR MIT (hlavicka souboru) |
| `VK_MESA_venus_protocol.xml` | venus-protocol (Google LLC): prikazy transportu (`vk*MESA`) | Apache-2.0 OR MIT (hlavicka souboru) |
| `LICENSE-venus-protocol` | licence repozitare venus-protocol | MIT |

Aktualizace: zkopirovat novou verzi `xmls/` z venus-protocol (stejna verze,
jakou ma renderer - virglrenderer ji nese v `src/venus/venus-protocol`),
spustit `cargo xtask venus-gen` a `cargo test -p venus`.
