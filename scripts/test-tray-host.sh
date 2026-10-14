#!/usr/bin/env bash
# Real Quickshell client + synthetic Epoxy-style menu; private bus/display only.
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
for tool in qs xvfb-run dbus-run-session python3 timeout; do
  command -v "$tool" >/dev/null || { echo "Missing test prerequisite: $tool" >&2; exit 1; }
done
python3 -c 'import dbus; from gi.repository import GLib'
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
mkdir "$fixture/runtime" "$fixture/cache" "$fixture/config"
chmod 700 "$fixture/runtime"
cat > "$fixture/bus.conf" <<CONF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig><type>session</type><listen>unix:tmpdir=$fixture</listen><auth>EXTERNAL</auth><policy context="default"><allow send_destination="*"/><allow receive_sender="*"/><allow own="*"/></policy></busconfig>
CONF
unset WAYLAND_DISPLAY
export XDG_RUNTIME_DIR="$fixture/runtime" XDG_CACHE_HOME="$fixture/cache" XDG_CONFIG_HOME="$fixture/config" QT_QPA_PLATFORM=xcb QT_QUICK_BACKEND=software
export EPOXY_PROBE_ROOT="$repo" EPOXY_PROBE_LOGS="$fixture"
if ! xvfb-run -a dbus-run-session --config-file "$fixture/bus.conf" -- bash -c '
  set -euo pipefail
  python3 "$EPOXY_PROBE_ROOT/scripts/fixtures/tray-menu-service.py" > "$EPOXY_PROBE_LOGS/server.log" 2>&1 &
  server=$!
  client=
  trap '\''kill "$server" ${client:+"$client"} 2>/dev/null || true'\'' EXIT
  timeout 12 qs -p "$EPOXY_PROBE_ROOT/scripts/fixtures/tray-navigation.qml" > "$EPOXY_PROBE_LOGS/client.log" 2>&1 &
  client=$!
  # Wait for the real client to find the synthetic menu, not a fixed startup sleep.
  for attempt in {1..100}; do
    if grep -q "NAV Certificates" "$EPOXY_PROBE_LOGS/client.log"; then break; fi
    sleep 0.03
  done
  python3 "$EPOXY_PROBE_ROOT/scripts/inspect-tray.py" --watch 2 > "$EPOXY_PROBE_LOGS/inspect.log"
  wait "$client"
'; then
  cat "$fixture/client.log" "$fixture/server.log" >&2
  exit 1
fi
python3 - "$fixture" <<'PY'
import json, sys
from pathlib import Path
p=Path(sys.argv[1]); client=(p/'client.log').read_text(); server=(p/'server.log').read_text()
assert 'PASS nested navigation' in client, client
assert 'FAIL' not in client and ' ERROR' not in client, client
rows=[json.loads(x) for x in (p/'inspect.log').read_text().splitlines()]
snapshots=[x for x in rows if x.get('snapshot')]
assert len(snapshots)==2, rows
assert any(x.get('method') == 'GetLayout' and x.get('id')==2 for x in rows), rows
assert 'Synthetic CA' not in (p/'inspect.log').read_text()
assert 'clicked' not in server, 'navigation and inspector must not activate leaf actions'
print('PASS: real Quickshell opens nested certificate details; recorder captures redacted metadata; no leaf activation')
PY
