# user/ffdec

Proces dekodéru médií nad FFmpeg (`docs/cs/multimedia.md`, kapitola Proces `ffdec`): program v C, PIE nad `libc.so` a `libav*.so` (port FFmpeg, `xtask/src/cports/`). Spouští ho jen `mediad`, jednou na relaci. `src/ffdec.c` - demux přes `AVIOContext` nad kanálem zdroje, dekódování, video NV12 přes libswscale do poolu s plotem, zvuk s16 stereo přes libswresample do kruhu `audiod`; `src/proto.c`, `src/proto.h` - kódování zpráv `mediaproto`.

## Invarianty

- Žádné handly kromě bootstrap kanálu (řídicí) a kanálu zdroje z `PROBE`; nikdy soubory, síť ani protokoly FFmpeg (`--disable-protocols --disable-network`).
- `proto.h` musí sedět s `libs/mediaproto` (čísla operací a událostí, `MAX_READ`, `MAX_POOL`, fourcc NV12): host test `libs/mediaproto/src/c_header_tests.rs`.
- Odpověď na čtení zdroje se musí vejít do zprávy kanálu (64 KiB): čtení nejvýš `SRC_MAX_READ` (60 KiB).
- Buffer z poolu se zapisuje jen volný (vrácený `RELEASE`); plot snímku pokračuje od hodnoty při `POOL`.
- Komentáře česky bez diakritiky, ASCII (jako `.rs`).

## Testy

- Host: `cargo test -p mediaproto` (shoda `proto.h`). QEMU: `cargo xtask test --only "media ffmpeg"` (AV1 a VP9 bit po bitu podle MD5, zvuk WAV, skok, poškozený vstup).

## Pasti

- Jména funkcí nesmí kolidovat s API FFmpeg (`avio_read` je funkce libavformat).
- Sestavení: `xtask/src/dynlink.rs` `build_ffdec` (cbuild, hlavičky z předpony portů, `magnumopus/sys.h` z `libs/mosys`).
