# libs/yuv

Převod YUV 4:2:0 (NV12, I420) na BGRA pro kompozitor (`no_std`, `core::simd`). BT.709 s omezeným rozsahem, celočíselně s 10 bity zlomku. Popis: `docs/cs/gui.md` (Buffery s popisem a explicitní synchronizace).

## Invarianty

- Vzorce a koeficienty (`coef`) jsou stejné jako v shaderu `libs/spirv/src/yuv.rs`: GPU a CPU dávají bit po bitu stejné pixely (test `spirv::tests::yuv_reference_matches_libs_yuv_exactly` projde všech 2^24 kombinací). Změna jednoho = změna obou.
- Funkce nikdy nečtou mimo předané řezy: rozměry a kroky se ověří předem (`Error::Short`), ne až indexováním.
- SIMD (8 pixelů) a skalární okraj musí dávat stejný výsledek jako `pixel` (testy `simd_matches_scalar_*`).

## Testy

- `cargo test -p yuv`.
