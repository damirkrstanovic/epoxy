# Native packages

These are upstream packages for **x86_64**: Arch Linux, Fedora 44, Debian 13,
and Ubuntu 24.04 LTS. The package is named `epoxy-signing`; the executable remains
`epoxy`. It is unrelated to the graphics library `libepoxy`.

Each distribution builds its own binary against its system GTK, NSS, XMLSec and
libxml2. Do not install an Arch-built binary on Debian, or reuse one distribution's
DEB/RPM on another release. OpenSC, PC/SC and CCID are runtime dependencies;
proprietary middleware is not bundled. Not every smart card is supported by OpenSC.

## Installing a built package

Download the package **for your distribution**, and check its accompanying
SHA256SUMS. Checksums detect corruption; they are not publisher signatures.

```sh
# Arch
sudo pacman -U ./epoxy-signing-*.pkg.tar.zst
# Fedora
sudo dnf install ./epoxy-signing-*.x86_64.rpm
# Debian 13
sudo apt install ./epoxy-signing_*_amd64-debian13.deb
# Ubuntu 24.04
sudo apt install ./epoxy-signing_*_amd64-ubuntu24.04.deb
```

Open **Epoxy Signing** from the application menu. It starts in the tray when the
desktop supports a tray, otherwise it displays its controls. GNOME may require
an AppIndicator extension for tray support; the GTK fallback still works.
Installation does not launch Epoxy, enable login startup, change NSS databases,
or replace your existing configuration. Close an older running helper first.

These recipes are not submissions to AUR or the Fedora/Debian archives. The CI
workflow creates downloadable build artifacts; it does not publish a release,
configure package repositories, or sign packages. An APT/DNF repository and
release signing can be added separately.

## First-time OpenSC setup

Epoxy reads PKCS#11 modules from its configured NSS database. Installing OpenSC
alone does not register it in that database. If your existing configuration works,
keep it. Do not import your private key: it remains on the smart card.

1. Enable the system smart-card socket if it is not already active:
   `sudo systemctl enable --now pcscd.socket`.
2. Install the NSS tools if needed: `nss` on Arch, `nss-tools` on Fedora,
   `libnss3-tools` on Debian/Ubuntu.
3. For a fresh setup, create a separate Epoxy database (run as yourself):

   ```sh
   install -d -m 700 "$HOME/.local/share/epoxy/nssdb"
   certutil -N --empty-password -d "sql:$HOME/.local/share/epoxy/nssdb"
   ```

   Run `certutil -N` only for a new database. This database contains public
   certificates and module registration, not a cached smart-card PIN.
4. Register OpenSC using the path for your distribution:

   | Distribution | OpenSC PKCS#11 module |
   | --- | --- |
   | Arch | `/usr/lib/opensc-pkcs11.so` |
   | Fedora x86_64 | `/usr/lib64/opensc-pkcs11.so` |
   | Debian/Ubuntu amd64 | `/usr/lib/x86_64-linux-gnu/opensc-pkcs11.so` |

   ```sh
   modutil -dbdir "sql:$HOME/.local/share/epoxy/nssdb" \
     -add OpenSC -libfile /path/from/table/opensc-pkcs11.so -mechanisms FRIENDLY
   ```

   `FRIENDLY` allows public certificate discovery without asking for a PIN. Signing
   still requires authentication. Cards whose public objects require a PIN may
   need different module settings.
5. Set `nssdb_path = "sql:/home/YOUR_USER/.local/share/epoxy/nssdb"` in
   `~/.config/epoxy.toml` (or `$XDG_CONFIG_HOME/epoxy.toml`). Use an absolute path;
   TOML values do not expand `$HOME`. Preserve other existing settings.

## Optional startup

For normal desktop use, add **Epoxy Signing** to your desktop's startup applications,
or copy `/usr/share/applications/dev.semyon.epoxy.desktop` to
`~/.config/autostart/dev.semyon.epoxy.desktop`. Do not also enable socket activation.

The existing optional user systemd units are included for experienced users.
They require the graphical environment to be imported into the user manager for
GTK dialogs. Socket activation starts on a portal connection and stops when idle;
it is not the same as an always-visible tray. Nothing enables these units during
installation. Do not enable them together with desktop autostart or another helper.

## Building

Build tools: Rust **1.95.0** in the verified matrix, C/C++ compiler, libclang,
pkg-config, native development libraries and the distribution's package tools.
The locked dependencies require Rust >= 1.89, which is newer than some supported
distros' packaged compiler. `packaging/ci/Dockerfile` installs a pinned Rust
build toolchain separately; it is not shipped as a runtime dependency.

From a committed checkout:

```sh
python3 packaging/make-source.py dist
```

This exports the selected Git commit, vendors the exact Cargo.lock dependencies,
bundles their license/copyright/NOTICE files and inventory, and produces:

- `epoxy-signing-VERSION.tar.xz`: application and all vendored Rust sources,
  including MPL-covered source, plus Debian build metadata and notice bundle;
- `PKGBUILD`: Arch recipe with the source archive's actual SHA-256;
- `epoxy-signing.spec`: RPM recipe with generated bundled license metadata;
- `SHA256SUMS`.

The source export contains **committed files only**. Always distribute the matching
source archive alongside binary packages and retain it for recipients. Package
builds use `--frozen` and run without network access. Native shared libraries come
from the distro, rather than being copied into the package.

Arch: place the archive and generated PKGBUILD together and run `makepkg` as a
normal user. Fedora: put the archive in your rpmbuild SOURCES directory and build
the generated spec with `rpmbuild -ba`. Debian/Ubuntu: extract the archive and run
`dpkg-buildpackage -b -us -uc`; when using the CI-installed Rust toolchain, `-d`
skips package-manager build-dependency checking because that compiler is not a
Debian package. Install all native Build-Depends first.

For the same isolated build/install/sign/remove checks used by CI:

```sh
docker build --build-arg BASE_IMAGE=ubuntu:24.04 \
  -f packaging/ci/Dockerfile -t epoxy-package-builder .
mkdir -p packages
docker run --rm --network none \
  -v "$PWD/dist:/input:ro" -v "$PWD/packages:/output" \
  -v "$PWD/packaging/ci/build.sh:/run-build.sh:ro" \
  epoxy-package-builder bash /run-build.sh
```

Use `debian:13`, `fedora:44` or `archlinux:base-devel` for the other builds.
These containers never mount a real smart card, session bus or user home.
Synthetic signing tests verify key-slot matching and independently verify the
resulting XML signature. Actual hardware/desktop testing remains separate.
