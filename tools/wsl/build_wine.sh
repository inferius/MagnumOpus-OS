#!/bin/bash
# Sestaveni Wine pro MagnumOpus ve WSL (docs/cs/windows-vrstva.md, etapa W0).
# Spousti `cargo xtask wine`; promenne nastavi xtask:
#   WINE_TARBALL  archiv wine-<verze>.tar.xz (overeny SHA-256 v xtasku)
#   WINE_TOP      adresar v archivu (wine-11.0)
#   WINE_PATCHES  adresar se zaplatami *.patch (third_party/wine)
#   WINE_KEY      otisk archivu a zaplat (zmena = novy zdroj a konfigurace)
#   MOS_SYSROOT   sysroot C (target/c-sysroot/x86_64)
#   MOS_TOOLS     adresar tools/wsl (obal prekladace mos-cc)
#   WINE_OUT      kam zkopirovat vysledek (target/wine/x86_64)
#   WINE_KEEP_GOING=1  make -k (prehled chyb portu)
# Pracovni adresare jsou na ext4 ve WSL (~/.cache/magnumopus-wine): pres
# /mnt by sestaveni bylo mnohonasobne pomalejsi.
set -e
: "${WINE_TARBALL:?}" "${WINE_TOP:?}" "${WINE_PATCHES:?}" "${WINE_KEY:?}" "${MOS_SYSROOT:?}" "${MOS_TOOLS:?}" "${WINE_OUT:?}"
W=~/.cache/magnumopus-wine
SRC=$W/src-$WINE_KEY
mkdir -p "$W/bin" "$W/logs"

# Obal prekladace a nastroje LLVM 23 pod jmeny, ktera hleda configure.
tr -d '\r' < "$MOS_TOOLS/mos-cc" > "$W/bin/mos-cc"
chmod +x "$W/bin/mos-cc"
for t in clang ld.lld lld-link llvm-ar llvm-ranlib llvm-strip llvm-dlltool; do
    ln -sf "/usr/bin/$t-23" "$W/bin/$t"
done
export PATH=$W/bin:$PATH MOS_SYSROOT

# Zdroj: rozbalit a zaplatovat jen jednou na otisk.
if [ ! -f "$SRC/.hotovo" ]; then
    rm -rf "$SRC" && mkdir -p "$SRC"
    tar -xJf "$WINE_TARBALL" -C "$SRC"
    for p in "$WINE_PATCHES"/*.patch; do
        [ -e "$p" ] || continue
        (cd "$SRC/$WINE_TOP" && patch -p1 -s < "$p")
    done
    touch "$SRC/.hotovo"
fi
S=$SRC/$WINE_TOP

# Nativni nastroje Wine (winebuild, widl, wrc, winegcc) pro krizove sestaveni.
T=$W/tools-$WINE_KEY
if [ ! -x "$T/tools/winebuild/winebuild" ]; then
    rm -rf "$T" && mkdir -p "$T" && cd "$T"
    "$S/configure" --enable-win64 --without-x --without-freetype --disable-tests CC=gcc \
        > "$W/logs/tools-configure.log" 2>&1 || { tail -30 "$W/logs/tools-configure.log"; exit 1; }
    make -j"$(nproc)" __tooldeps__ > "$W/logs/tools-make.log" 2>&1 || { tail -30 "$W/logs/tools-make.log"; exit 1; }
fi

# Wine pro x86_64-unknown-magnumopus: DLL PE clangem, unixova cast mos-cc.
B=$W/build-$WINE_KEY
if [ ! -f "$B/Makefile" ]; then
    rm -rf "$B" && mkdir -p "$B" && cd "$B"
    "$S/configure" --host=x86_64-unknown-magnumopus --with-wine-tools="$T" \
        --enable-archs=x86_64 --with-mingw=clang --disable-tests \
        --without-x --without-freetype --without-fontconfig --without-gstreamer --without-wayland \
        --without-vulkan --without-opengl --without-pulse --without-alsa --without-oss \
        --without-coreaudio --without-cups --without-dbus --without-gnutls --without-krb5 \
        --without-gssapi --without-netapi --without-sane --without-v4l2 --without-usb \
        --without-pcap --without-sdl --without-capi --without-gphoto --without-inotify \
        --without-udev --without-unwind --without-xinerama --without-ffmpeg --without-opencl \
        CC=mos-cc AR=llvm-ar RANLIB=llvm-ranlib STRIP=llvm-strip x86_64_CC=clang \
        > "$W/logs/configure.log" 2>&1 || { tail -40 "$W/logs/configure.log"; exit 1; }
fi
cd "$B"
if [ "${WINE_KEEP_GOING:-0}" = 1 ]; then
    make -k -j"$(nproc)" > "$W/logs/make.log" 2>&1 || true
    echo "xtask-wine: PE DLL: $(ls dlls/*/x86_64-windows/*.dll 2>/dev/null | wc -l), unixove .so: $(ls dlls/*/*.so 2>/dev/null | wc -l)"
    echo "xtask-wine: chyby podle souboru:"
    grep -o "^[^ :]*\.c:[0-9]*:[0-9]*: error" "$W/logs/make.log" | cut -d: -f1 | sed "s#.*/$WINE_TOP/##" | sort | uniq -c | sort -rn | head -40
    echo "xtask-wine: log $W/logs/make.log"
else
    make -j"$(nproc)" > "$W/logs/make.log" 2>&1 || { grep -m 20 "error" "$W/logs/make.log"; echo "xtask-wine: log $W/logs/make.log"; exit 1; }
fi

# Vysledek do target/wine/<arch>: DLL PE a unixove knihovny.
mkdir -p "$WINE_OUT/x86_64-windows" "$WINE_OUT/x86_64-unix"
cp -u dlls/*/x86_64-windows/*.dll "$WINE_OUT/x86_64-windows/" 2>/dev/null || true
cp -u dlls/*/*.so "$WINE_OUT/x86_64-unix/" 2>/dev/null || true
echo "xtask-wine: hotovo"
