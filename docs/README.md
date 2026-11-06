# Dokumentace / Documentation

| Česky | English | Obsah / Contents |
|---|---|---|
| [architektura.md](cs/architektura.md) | [architecture.md](en/architecture.md) | návrh systému a důvody / system design and reasons |
| [sestaveni.md](cs/sestaveni.md) | [building.md](en/building.md) | nástroje, příkazy, testy, ladění, struktura / tools, commands, tests, debugging, layout |
| [jadro.md](cs/jadro.md) | [kernel.md](en/kernel.md) | jak jádro funguje / how the kernel works |
| [syscally.md](cs/syscally.md) | [syscalls.md](en/syscalls.md) | referenční popis syscallů / syscall reference |
| [chyby.md](cs/chyby.md) | [errors.md](en/errors.md) | kódy chyb a návratové kódy procesů / error and exit codes |
| [hlasky.md](cs/hlasky.md) | [messages.md](en/messages.md) | hlášky v logu a co znamenají / log messages and their meaning |
| [libos.md](cs/libos.md) | [libos.md](en/libos.md) | knihovna pro programy / program library |
| [souborovy-system.md](cs/souborovy-system.md) | [file-system.md](en/file-system.md) | souborový systém: formát, algoritmy, služba fsd, protokol / file system: format, algorithms, fsd service, protocol |

Česká verze je primární; anglická ji překládá. Úplnost popisu chyb
a syscallů v obou jazycích hlídá test (`cargo test -p abi`).

The Czech version is primary; the English one translates it. A test
(`cargo test -p abi`) checks that every error and syscall is documented in
both languages.
