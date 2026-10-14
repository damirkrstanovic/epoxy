#!/usr/bin/env bash
# Synthetic GUI/tray checks only. Never opens a token or requests a real PIN.
set -euo pipefail
cd "$(dirname "$0")/.."
for tool in cargo python3 Xvfb xvfb-run dbus-run-session dbus-daemon; do
  command -v "$tool" >/dev/null || { echo "Missing test prerequisite: $tool" >&2; exit 1; }
done
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
chmod 700 "$fixture"
cat > "$fixture/bus.conf" <<CONF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig><type>session</type><listen>unix:tmpdir=$fixture</listen><auth>EXTERNAL</auth>
<policy context="default"><allow send_destination="*"/><allow receive_sender="*"/><allow own="*"/></policy></busconfig>
CONF
export GDK_BACKEND=x11 GSK_RENDERER=cairo GTK_A11Y=none EPOXY_PRIVATE_TEST_BUS=1
# No service directories: desktop portals/keyrings cannot autostart on this test bus.
xvfb-run -a dbus-run-session --config-file "$fixture/bus.conf" -- \
  cargo test -p epoxy --lib ui::tests::desktop_widgets_and_pin_lifecycle -- --ignored --test-threads=1
dbus-run-session --config-file "$fixture/bus.conf" -- \
  cargo test -p epoxy --lib desktop::tray::bus_tests::delayed_watcher -- --ignored --test-threads=1
dbus-run-session --config-file "$fixture/bus.conf" -- \
  cargo test -p epoxy --lib desktop::tray::bus_tests::nonresponding -- --ignored --test-threads=1
dbus-run-session --config-file "$fixture/bus.conf" -- \
  cargo test -p epoxy --lib watcher_without_host -- --ignored --test-threads=1
dbus-run-session --config-file "$fixture/bus.conf" -- \
  cargo test -p epoxy --lib late_watcher_transient -- --ignored --test-threads=1
EPOXY_TEST_BUS_SOCKET="$fixture/restart-bus" DBUS_SESSION_BUS_ADDRESS="unix:path=$fixture/restart-bus" \
  cargo test -p epoxy --lib session_bus_restart_recovers -- --ignored --test-threads=1
cargo build -p epoxy
xvfb-run -a dbus-run-session --config-file "$fixture/bus.conf" -- \
  python3 scripts/desktop-smoke.py "${CARGO_TARGET_DIR:-target}/debug/epoxy"
