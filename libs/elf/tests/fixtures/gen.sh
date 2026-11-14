#!/bin/sh
# Vytvori testovaci ELF pro host testy `libs/elf` (commitovane vystupy,
# testy clang nepotrebuji). Spoustet v teto slozce: `sh gen.sh`.
# Potrebuje clang a ld.lld (LLVM 17+, kvuli -z pack-relative-relocs). Triple
# *-linux-gnu jen kvuli ovladaci clang (-shared); ABI ELF je stejne, libc se
# nelinkuje (-nostdlib).
set -e
# Git Bash (MSYS) by jinak prevedl /system/lib/ld.so na cestu Windows.
export MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*'
CLANG=${CLANG:-clang}
# Segmenty zarovnane na 64 KiB jako programy systemu (user/user.ld).
COMMON="-ffreestanding -nostdlib -fuse-ld=lld -O1 -fno-stack-protector -Wl,-z,max-page-size=65536"
for ARCH in x86_64 aarch64; do
    T="--target=$ARCH-unknown-linux-gnu"
    # Zavislost (jen SONAME a symboly dep_value, dep_call).
    printf 'int dep_value = 5;\nint dep_call(int x) { return x * 2; }\n' > dep.c
    $CLANG $T $COMMON -fPIC -shared -Wl,-soname,libdep.so.1 -Wl,-z,now dep.c -o $ARCH-libdep.so
    # Knihovna s RELA (RELATIVE v .rela.dyn), BIND_NOW a plne RELRO.
    $CLANG $T $COMMON -fPIC -shared -Wl,-soname,libfix.so.1 -Wl,-z,now -Wl,-z,relro \
        -Wl,--hash-style=both -Wl,-rpath,'$ORIGIN' -Wl,--enable-new-dtags \
        lib.c $ARCH-libdep.so -o $ARCH-libfix.so
    # Tataz knihovna s RELR (-z pack-relative-relocs) a jen GNU hash.
    $CLANG $T $COMMON -fPIC -shared -Wl,-soname,libfix.so.1 -Wl,-z,now \
        -Wl,--hash-style=gnu -Wl,-z,pack-relative-relocs \
        lib.c $ARCH-libdep.so -o $ARCH-libfix-relr.so
    # Program PIE s PT_INTERP.
    $CLANG $T $COMMON -fPIE -pie -Wl,-z,now -Wl,--dynamic-linker,/system/lib/ld.so \
        -Wl,-rpath,'$ORIGIN' -Wl,--enable-new-dtags \
        main.c $ARCH-libfix.so -o $ARCH-main.pie
    # Knihovna s TEXTREL jen na x86_64 (na aarch64 ji lld nevytvori: absolutni
    # relokace MOVW nejdou dynamicky; kontrola v parseru je na architekture
    # nezavisla).
    if [ "$ARCH" = x86_64 ]; then
        $CLANG $T $COMMON -fno-pic -mcmodel=large -shared -Wl,-z,notext -Wl,-soname,libtextrel.so \
            textrel.c $ARCH-libdep.so -o $ARCH-textrel.so
    fi
    rm -f dep.c
done
ls -l *.so *.pie
