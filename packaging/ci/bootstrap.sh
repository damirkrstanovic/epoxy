#!/usr/bin/env bash
set -euo pipefail
source /etc/os-release
case "$ID" in
    debian|ubuntu)
        export DEBIAN_FRONTEND=noninteractive
        apt-get update
        apt-get install -y --no-install-recommends ca-certificates curl build-essential pkg-config libclang-dev libnss3-dev libnspr4-dev libxml2-dev libxmlsec1-dev libxmlsec1t64-nss libgtk-4-dev libnss3-tools xmlsec1 python3 python3-cryptography debhelper dpkg-dev desktop-file-utils opensc pcscd libccid xz-utils
        rm -rf /var/lib/apt/lists/*
        ;;
    fedora)
        dnf install -y --setopt=install_weak_deps=False gcc gcc-c++ make pkgconf-pkg-config clang-devel nss-devel nspr-devel libxml2-devel xmlsec1-devel xmlsec1-nss-devel gtk4-devel nss-tools xmlsec1 xmlsec1-nss python3 python3-cryptography rpm-build desktop-file-utils opensc pcsc-lite ccid curl ca-certificates tar xz git
        dnf clean all
        ;;
    arch)
        pacman -Syu --noconfirm --needed base-devel clang pkgconf gtk4 nss nspr libxml2 xmlsec opensc pcsclite ccid python python-cryptography desktop-file-utils curl ca-certificates git
        ;;
    *) echo "Unsupported build image: $ID" >&2; exit 1 ;;
esac
