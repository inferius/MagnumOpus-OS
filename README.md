# os

Nový operační systém psaný od nuly v Rustu. Pracovní název zatím není, v kódu
se používá `os` / `kernel`.

Aktuální stav: **milník 4 (userspace)**. Vlastní UEFI loader, mikrokernel
s plánovačem na všech CPU, objekty a handly s právy, syscally, kanály (IPC),
objekty paměti, procesy. První proces `init` běží v userspace, spouští
další programy z initfs a komunikuje s nimi.

**Dokumentace (česky i anglicky): [docs/](docs/README.md)** - návrh,
sestavení, jak funguje jádro, syscally, kódy chyb, hlášky v logu.

## Rychlý start

```bash
cargo xtask run      # spustí systém v QEMU
cargo xtask test     # všechny testy (hostitel, jádro, userspace)
```

Požadované nástroje a podrobnosti: [docs/cs/sestaveni.md](docs/cs/sestaveni.md).

