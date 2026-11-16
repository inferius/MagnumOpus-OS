# Testovací obraz NTFS

`testfs1` (2 MiB) je beze změny převzatý z crate `ntfs` 0.4.0
(<https://github.com/ColinFinck/ntfs>, `testdata/testfs1`, MIT OR
Apache-2.0, licence vedle). Vytvořil ho autor crate skriptem
`create-testfs1.sh` (`mkntfs -c 512 -L mylabel`, pak soubory přes ntfs-3g).
Nástroje ntfs-3g (GPL) se jen spustily, nic z nich se nelinkuje.

Obsah: jmenovka `mylabel`, `empty-file` (změna 2021-01-01 13:37),
`file-with-12345` (5 B, rezidentní data), `1000-bytes-file`, `sparse-file`
(řídký, data na začátku a na konci), složka `many_subdirs` s podsložkami
1..512 (index `INDEX_ALLOCATION`).

Používá `user/volfs` (testy na hostiteli) a `xtask` (disk USB s oddílem
NTFS pro test v QEMU).
