#!/usr/bin/env bash
set -euo pipefail
# Run in a disposable distro container; /input holds make-source.py output,
# /output receives artifacts. Never mount a desktop bus, token or user home.
source /etc/os-release
shopt -s nullglob
archives=(/input/epoxy-signing-*.tar.xz)
[[ ${#archives[@]} == 1 ]] || { echo 'Expected exactly one source archive' >&2; exit 1; }
archive=${archives[0]}
mkdir -p /build /output
cd /build
tar -xf "$archive"
sources=(/build/epoxy-signing-*)
[[ ${#sources[@]} == 1 && -d ${sources[0]} ]]
cd "${sources[0]}"
export SOURCE_DATE_EPOCH=$(cat SOURCE_DATE_EPOCH)
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-2}
export CARGO_PROFILE_DEV_DEBUG=0
{
    cat /etc/os-release
    printf 'SOURCE_COMMIT='
    cat SOURCE_COMMIT
    sha256sum "$archive"
    rustc --version
    cargo --version
    pkg-config --modversion gtk4 nss xmlsec1-nss
} > /output/BUILD-INFO.txt
case "$ID" in
    debian|ubuntu)
        # Toolchain is pinned in the image, newer than the distro rustc package.
        # Native build deps were explicitly installed by bootstrap.sh.
        dpkg-buildpackage -b -us -uc -d
        debs=(/build/epoxy-signing_*_amd64.deb)
        [[ ${#debs[@]} == 1 ]]
        dpkg -i "${debs[0]}"
        # Distinct download names avoid mixing Debian and Ubuntu artifacts.
        cp "${debs[0]}" "/output/$(basename "${debs[0]}" .deb)-${ID}${VERSION_ID}.deb"
        ;;
    fedora)
        mkdir -p /build/rpm/{SOURCES,SPECS,BUILD,BUILDROOT,RPMS,SRPMS}
        cp "$archive" /build/rpm/SOURCES/
        cp /input/epoxy-signing.spec /build/rpm/SPECS/
        rpmbuild -ba --nodeps --define '_topdir /build/rpm' /build/rpm/SPECS/epoxy-signing.spec
        rpms=(/build/rpm/RPMS/x86_64/epoxy-signing-*.rpm)
        [[ ${#rpms[@]} == 1 ]]
        rpm -Uvh "${rpms[0]}"
        cp "${rpms[0]}" /build/rpm/SRPMS/*.src.rpm /output/
        ;;
    arch)
        # makepkg refuses root. Dedicated unprivileged build user, not the host.
        useradd --create-home builder
        mkdir -p /build/arch
        cp /input/PKGBUILD "$archive" /build/arch/
        chown -R builder:builder /build/arch
        runuser -u builder -- env PATH="$PATH" CARGO_HOME=/home/builder/.cargo RUSTUP_HOME="$RUSTUP_HOME" CARGO_BUILD_JOBS="$CARGO_BUILD_JOBS" bash -c 'cd /build/arch; makepkg --nodeps --noconfirm'
        packages=(/build/arch/epoxy-signing-*.pkg.tar.zst)
        [[ ${#packages[@]} == 1 ]]
        pacman -U --noconfirm "${packages[0]}"
        cp "${packages[0]}" /output/
        ;;
esac
# Installed executable, library linkage and desktop file; never start a service.
epoxy --version
if ldd /usr/bin/epoxy | grep -q 'not found'; then exit 1; fi
desktop-file-validate /usr/share/applications/dev.semyon.epoxy.desktop
# Native recipes run synthetic NSS signing and independent XML verification
# in their check phase, reusing the same release build and distro compiler flags.
case "$ID" in
    debian|ubuntu) dpkg --remove epoxy-signing ;;
    fedora) rpm -e epoxy-signing ;;
    arch) pacman -R --noconfirm epoxy-signing ;;
esac
test ! -e /usr/bin/epoxy
cd /output
sha256sum -- *.deb *.rpm *.zst BUILD-INFO.txt > SHA256SUMS
