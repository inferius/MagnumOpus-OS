# user/gpud

Služba render (kostra 6a-3): rozdává render uzly ovladače GPU. Startovní zpráva: kanál poskytovatele (`renderproto::node`); bootstrap kanál od initu mluví stejným protokolem (`OPEN` -> `OPENED` + kanál render uzlu). Popis: `docs/cs/render.md`.

## Invarianty

- Poskytovatele vydá ovladač jen jednou (`OPEN_PROVIDER`); drží ho init (`init/src/render.rs`, `Kept`), gpud dostává duplikát. Po pádu gpud ho supervisor spustí znovu s dalším duplikátem; nová instance nejdřív zahodí staré odpovědi poskytovatele.
- Render uzly jdou přímo k ovladači: pád gpud je nepřeruší.
- Kostra povolí každý požadavek od initu; manifest `[gpu] render`, kvóty a lístky pro aplikace jsou **[navrženo]**.

## Testy

- Jen v QEMU: `cargo xtask test --qemu-wsl --gpu venus --only vulkan,gui` (bez Venusu gpud neběží).
