# libs/bufdesc

Popis sdíleného bufferu nad VMO (`no_std`, závisí jen na `wire`): fourcc a modifikátor jako v DRM, roviny `{offset, pitch}`, šířka, výška, umístění, typ cache. Rozhodnutí 6a (`docs/cs/gpu-amd-nvidia.md` bod 2), popis `docs/cs/sdilene-buffery.md`.

## Invarianty

- Popis od jiného procesu je nedůvěryhodný: příjemce volá `BufDesc::validate(velikost VMO)` dřív, než z bufferu čte nebo ho předá zařízení. Veškerá aritmetika s kontrolou přetečení.
- Kódování má jediný tvar (72 B): nepoužité roviny nulové, rezervní bajt 0, jinak `Encoding`.
- Práva k bufferu určuje handle VMO, synchronizaci ploty ve zprávách protokolu; popis nenese ani jedno.
- Hodnoty fourcc a modifikátorů se nemění (shodné s DRM kvůli Mese).

## Testy

- `cargo test -p bufdesc` (`src/tests.rs`: rozložení formátů, meze proti VMO, přetečení, kódování).
