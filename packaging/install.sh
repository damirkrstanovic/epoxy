#!/usr/bin/env bash
set -euo pipefail
# Staging only: no service activation and no writes to a user's home directory.
if [[ $# != 2 || -z $1 || $1 == / ]]; then
    echo 'Usage: packaging/install.sh STAGING_DIRECTORY BUILT_BINARY' >&2
    exit 2
fi
stage=$(realpath -m "$1")
binary=$(realpath "$2")
root=$(cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
test -d packaging/licenses || { echo 'Missing license bundle: build from the generated source archive.' >&2; exit 1; }
install -Dm755 "$binary" "$stage/usr/bin/epoxy"
install -Dm644 packaging/common/epoxy-signing.desktop "$stage/usr/share/applications/epoxy-signing.desktop"
install -Dm644 epoxy/assets/tray.svg "$stage/usr/share/icons/hicolor/scalable/apps/epoxy-signing.svg"
for file in LICENSE THIRD_PARTY.md README.md packaging/README.md; do
    name=$(basename "$file")
    [[ $file == packaging/README.md ]] && name=PACKAGING.md
    install -Dm644 "$file" "$stage/usr/share/doc/epoxy-signing/$name"
done
cp -a packaging/licenses "$stage/usr/share/doc/epoxy-signing/third-party-licenses"
cp -a epoxy/docs "$stage/usr/share/doc/epoxy-signing/guide"
# Socket activation is optional and has desktop-session caveats; do not enable it.
for file in epoxy/systemd/*; do
    install -Dm644 "$file" "$stage/usr/lib/systemd/user/$(basename "$file")"
done
