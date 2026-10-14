#!/usr/bin/env python3
"""Read-only niri focus trace. Never prints window titles or raw event payloads."""
import argparse
import json
import subprocess
import sys
import threading
import time


def trace(lines, output):
    apps = {}
    previous = object()
    focused = object()
    emitted = 0

    def emit(window_id):
        nonlocal previous, focused, emitted
        if not (window_id is None or type(window_id) is int):
            return
        app = apps.get(window_id, 'unknown application')
        app = ''.join(c if c.isprintable() else ' ' for c in app)[:80]
        marker = (window_id, app)
        if marker == previous:
            return
        previous = marker
        focused = window_id
        emitted += 1
        label = 'no focused application window' if window_id is None else f'window {window_id}: {app}'
        now = time.time()
        stamp = time.strftime('%H:%M:%S', time.localtime(now)) + f'.{int(now * 1000) % 1000:03d}'
        print(stamp, label, file=output, flush=True)

    for line in lines:
        try:
            event = json.loads(line)
            if not isinstance(event, dict):
                continue
            if 'WindowsChanged' in event:
                apps.clear()
                for window in event['WindowsChanged']['windows']:
                    if type(window.get('id')) is int:
                        app = window.get('app_id')
                        apps[window['id']] = app if isinstance(app, str) and app else 'unknown application'
                    if window.get('is_focused'):
                        emit(window.get('id'))
            elif 'WindowOpenedOrChanged' in event:
                window = event['WindowOpenedOrChanged']['window']
                if type(window.get('id')) is int:
                    app = window.get('app_id')
                    apps[window['id']] = app if isinstance(app, str) and app else 'unknown application'
                    if window.get('is_focused') or window['id'] == focused:
                        emit(window['id'])
            elif 'WindowClosed' in event:
                apps.pop(event['WindowClosed']['id'], None)
            elif 'WindowFocusChanged' in event:
                emit(event['WindowFocusChanged']['id'])
        except (ValueError, KeyError, TypeError, AttributeError):
            # Unknown/malformed events are omitted; never echo raw payloads.
            continue
    return emitted


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--seconds', type=int, default=30)
    parser.add_argument('--stdin', action='store_true', help='read synthetic/saved events instead of contacting niri')
    args = parser.parse_args()
    if args.stdin:
        trace(sys.stdin, sys.stdout)
        return
    if not 1 <= args.seconds <= 300:
        parser.error('--seconds must be between 1 and 300')
    print('Focus the browser, open Epoxy tray, then click a troubleshooting-log toggle. Only IDs/app names are recorded.', file=sys.stderr)
    try:
        process = subprocess.Popen(['niri', 'msg', '--json', 'event-stream'], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    except OSError:
        parser.exit(1, 'Cannot start niri; run this from your niri session.\n')
    timer = threading.Timer(args.seconds, process.terminate)
    timer.daemon = True
    timer.start()
    count = 0
    interrupted = False
    try:
        count = trace(process.stdout, sys.stdout)
    except KeyboardInterrupt:
        interrupted = True
    finally:
        timer.cancel()
        if process.poll() is None:
            process.terminate()
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        process.stdout.close()
    if not count and not interrupted:
        parser.exit(1, 'No focus events received. Run this in the niri session with NIRI_SOCKET available.\n')
    print('Focus trace finished.', file=sys.stderr)


if __name__ == '__main__':
    main()
