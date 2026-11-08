#!/bin/bash
# Preklad virglrenderer (venus + render server) a QEMU s Venusem do ~/venus
# (spoustet ve WSL Ubuntu; balicky viz docs/cs/gui.md, oddil 3D).
set -e
mkdir -p ~/venus && cd ~/venus
P=~/venus/prefix
[ -d virglrenderer ] || git clone --depth 1 https://gitlab.freedesktop.org/virgl/virglrenderer.git
[ -d venus-protocol ] || git clone --depth 1 https://gitlab.freedesktop.org/virgl/venus-protocol.git || git clone --depth 1 https://gitlab.freedesktop.org/olv/venus-protocol.git
[ -d qemu ] || git clone --depth 1 --branch v10.1.0 https://gitlab.com/qemu-project/qemu.git
cd virglrenderer
meson setup build --prefix=$P -Dvenus=true -Drender-server=true -Dtests=false --wipe >/dev/null 2>&1 || meson setup build --prefix=$P -Dvenus=true -Dtests=false
ninja -C build install
cd ../qemu
export PKG_CONFIG_PATH=$P/lib/x86_64-linux-gnu/pkgconfig:$P/lib/pkgconfig
./configure --prefix=$P --target-list=x86_64-softmmu,aarch64-softmmu --enable-kvm --enable-virglrenderer --enable-opengl --enable-sdl --enable-gtk --enable-slirp --disable-docs >/tmp/qemu-configure.log 2>&1 || { tail -20 /tmp/qemu-configure.log; exit 1; }
make -j32 >/tmp/qemu-make.log 2>&1 || { tail -30 /tmp/qemu-make.log; exit 1; }
make install >/dev/null
echo HOTOVO
$P/bin/qemu-system-x86_64 --version | head -1
