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

# Ovladac grafiky pro kompozitor (W3): zdrojaky jsou ve slozce Wine
# repozitare (third_party/wine/winemo.drv), configure ho zna ze zaplaty
# 0008. Kopiruji se pri kazdem behu jen zmenene soubory: zmena ovladace
# prelozi jen jeho objekty (otisk zdroje WINE_KEY je jen ze zaplat).
mkdir -p "$S/dlls/winemo.drv"
for f in "$WINE_PATCHES"/winemo.drv/*; do
    d="$S/dlls/winemo.drv/$(basename "$f")"
    tr -d '\r' < "$f" > "$d.new"
    if cmp -s "$d.new" "$d"; then rm "$d.new"; else mv "$d.new" "$d"; fi
done

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
# configure zjistuje hlavicky sysrootu (HAVE_SYS_UN_H ...): zmena obsahu
# hlavicek = znovu configure na miste (objekty zustanou, make prelozi jen
# soubory zavisle na config.h).
SYSKEY=$(cd "$MOS_SYSROOT" && find include -type f | LC_ALL=C sort | xargs cat | sha256sum | cut -c1-16)
if [ ! -f "$B/Makefile" ] || [ "$(cat "$B/.sysroot-key" 2>/dev/null)" != "$SYSKEY" ]; then
    [ -f "$B/Makefile" ] || rm -rf "$B"
    mkdir -p "$B" && cd "$B"
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
    echo "$SYSKEY" > "$B/.sysroot-key"
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

# Testovaci program Windows (user/wintest, sada `wine`): preklad jako
# programy Wine (clang -target x86_64-windows, hlavicky msvcrt z Wine),
# link pres winegcc z build stromu (winecrt0, ucrtbase, kernel32), ale
# bez --wine-builtin = obycejny PE program.
WT="$WINE_OUT/tests"
mkdir -p "$WT"
WSRC="$MOS_TOOLS/../../user/wintest"
TOOLS="$W/tools-$WINE_KEY/tools"
# wintest.c (konzole) a gui.c (W3: okna, GDI): user32 a gdi32 navic.
: > "$W/logs/wintest.log"
for c in wintest gui; do
    clang -target x86_64-windows -fuse-ld=lld --no-default-config -c -o "$WT/$c.o" "$WSRC/$c.c" \
        -I"$SRC/$WINE_TOP/include" -I"$SRC/$WINE_TOP/include/msvcrt" -Iinclude -D_UCRT -O2 -Wall \
        >> "$W/logs/wintest.log" 2>&1 || { cat "$W/logs/wintest.log"; exit 1; }
done
"$TOOLS/winegcc/winegcc" -o "$WT/wintest.exe" --wine-objdir . --winebuild "$TOOLS/winebuild/winebuild" \
    -b x86_64-windows -mconsole "$WT/wintest.o" "$WT/gui.o" dlls/winecrt0/x86_64-windows/libwinecrt0.a \
    libs/compiler-rt/x86_64-windows/libcompiler-rt.a dlls/ucrtbase/x86_64-windows/libucrtbase.a \
    dlls/user32/x86_64-windows/libuser32.a dlls/gdi32/x86_64-windows/libgdi32.a \
    dlls/kernel32/x86_64-windows/libkernel32.a dlls/ntdll/x86_64-windows/libntdll.a \
    >> "$W/logs/wintest.log" 2>&1 || { cat "$W/logs/wintest.log"; exit 1; }
rm -f "$WT/gui.o"
rm -f "$WT/wintest.o"

# Vysledek do target/wine/<arch>/tree: rozlozeni adresare aplikace
# (/apps/org.winehq.wine): bin/wineserver, lib/wine/x86_64-unix (i zavadec
# `wine`; bin/wine je v instalaci Wine jen odkaz na nej),
# lib/wine/x86_64-windows, share/wine (nls, wine.inf). Wine hleda sve
# adresare relativne k ntdll.so, takze prefix neni potreba. Bez ladicich
# informaci (llvm-strip).
INST="$W/install-$WINE_KEY"
rm -rf "$INST"
make install DESTDIR="$INST" > "$W/logs/install.log" 2>&1 || { tail -20 "$W/logs/install.log"; exit 1; }
P="$INST/usr/local"
T="$WINE_OUT/tree.new"
rm -rf "$T"
mkdir -p "$T/bin" "$T/lib" "$T/share"
for f in wine wineserver; do
    [ -f "$P/bin/$f" ] && cp "$P/bin/$f" "$T/bin/"
done
cp -r "$P/lib/wine" "$T/lib/"
cp -r "$P/share/wine" "$T/share/"
# Importni knihovny (.a) jen pro vyvoj; ostatni bez symbolu a ladicich
# informaci (PE i ELF; dynamicke symboly ELF zustanou). Soubor, ktery
# neni objekt (data), llvm-strip odmitne a zustane beze zmeny.
find "$T/lib" -name '*.a' -delete
find "$T/bin" "$T/lib" -type f -exec llvm-strip {} \; 2>/dev/null || true
# Manifest zavadece (launchd: [app] jit = true, Wine prepina PE na RX).
cp "$WINE_PATCHES/wine.manifest.toml" "$T/lib/wine/x86_64-unix/wine.manifest.toml"
rm -rf "$WINE_OUT/tree" "$WINE_OUT/x86_64-windows" "$WINE_OUT/x86_64-unix" "$WINE_OUT/bin"
mv "$T" "$WINE_OUT/tree"
rm -rf "$INST"
echo "xtask-wine: strom $(du -sh "$WINE_OUT/tree" | cut -f1), soubory: $(find "$WINE_OUT/tree" -type f | wc -l)"
echo "xtask-wine: hotovo"
