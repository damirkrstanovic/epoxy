#!/usr/bin/env python3
"""Read-only Epoxy menu diagnostics; never activates a menu action.

Requires python-dbus and (for --watch) python-gobject. All certificate labels
and unknown menu strings are redacted. Only menu-interface metadata is observed.
"""
import argparse
import json
import sys
import time

SAFE_LABELS = frozenset(('Certificates', 'Service ready', 'Starting', 'Working',
    'Waiting for PIN', 'Stopping', 'Used for signing', 'Use for signing',
    'Certificate details…', 'Refresh certificates', 'Forget saved certificate choice',
    'Enable troubleshooting logs (5 minutes)', 'Disable troubleshooting logs',
    'Advanced', 'Log settings…', 'Quit Epoxy'))
MENU_INTERFACE = 'com.canonical.dbusmenu'
PROPERTIES = ['label', 'enabled', 'visible', 'children-display', 'type', 'toggle-type', 'toggle-state']


def safe_label(label):
    label = str(label)
    if label in SAFE_LABELS:
        return label
    if label.startswith('✓ '):
        return '[selected certificate]'
    for prefix in ('Issued by:', 'Valid until:', 'Valid from:', 'Purpose:'):
        if label.startswith(prefix):
            return prefix + ' [redacted]'
    return '[redacted]'


def describe_layout(layout, depth=0):
    node_id, properties, children = layout
    row = {'id': int(node_id), 'depth': depth,
           'label': safe_label(properties.get('label', '')),
           'enabled': bool(properties.get('enabled', True)),
           'visible': bool(properties.get('visible', True)),
           'submenu': properties.get('children-display') == 'submenu',
           'children': len(children)}
    rows = [row]
    for child in children:
        rows.extend(describe_layout(child, depth + 1))
    return rows


def event_summary(member, args):
    """Whitelist metadata: never serialize arbitrary Event.data or labels."""
    try:
        if member == 'GetLayout':
            return {'method': member, 'id': int(args[0]), 'depth': int(args[1])}
        if member == 'AboutToShow':
            return {'method': member, 'id': int(args[0])}
        if member == 'Event':
            event = str(args[1])
            return {'method': member, 'id': int(args[0]),
                    'event': event if event in ('clicked', 'opened', 'closed', 'hovered') else '[redacted]'}
        if member == 'LayoutUpdated':
            return {'signal': member, 'revision': int(args[0]), 'parent': int(args[1])}
        if member == 'ItemsPropertiesUpdated':
            return {'signal': member, 'updated_ids': [int(v[0]) for v in args[0]],
                    'removed_ids': [int(v[0]) for v in args[1]]}
        if member in ('AboutToShowGroup', 'EventGroup'):
            return {'method': member, 'count': len(args[0])}
    except (IndexError, TypeError, ValueError):
        return {'error': 'invalid_menu_metadata'}
    return None


def find_epoxy(bus, dbus):
    watcher = dbus.Interface(bus.get_object('org.kde.StatusNotifierWatcher', '/StatusNotifierWatcher', introspect=False),
                             'org.freedesktop.DBus.Properties')
    items = watcher.Get('org.kde.StatusNotifierWatcher', 'RegisteredStatusNotifierItems', timeout=3)
    found = []
    for entry in items:
        service, slash, path = str(entry).partition('/')
        if not service.startswith(('org.kde.StatusNotifierItem-', ':')):
            continue
        path = '/' + path if slash else '/StatusNotifierItem'
        try:
            item = dbus.Interface(bus.get_object(service, path, introspect=False), 'org.freedesktop.DBus.Properties')
            app_id = str(item.Get('org.kde.StatusNotifierItem', 'Id', timeout=3))
            if app_id != 'dev.semyon.epoxy':
                continue
            menu_path = str(item.Get('org.kde.StatusNotifierItem', 'Menu', timeout=3))
            found.append((service, menu_path, str(bus.get_name_owner(service))))
        except dbus.DBusException:
            continue
    if len(found) != 1:
        raise RuntimeError(f'Expected one Epoxy tray item; found {len(found)}. Start its desktop build or quit duplicates.')
    return found[0]


def snapshot(bus, dbus, service, path):
    proxy = dbus.Interface(bus.get_object(service, path, introspect=False), MENU_INTERFACE)
    revision, layout = proxy.GetLayout(0, -1, PROPERTIES, timeout=3)
    return {'snapshot': True, 'revision': int(revision), 'items': describe_layout(layout)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--watch', type=int, default=0, metavar='SECONDS', help='Passively observe Epoxy menu traffic for 1–300 seconds')
    args = parser.parse_args()
    if not 0 <= args.watch <= 300:
        parser.error('--watch must be between 0 and 300 seconds')
    import dbus
    if args.watch:
        import dbus.mainloop.glib
        from gi.repository import GLib
        dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
    bus = dbus.SessionBus(private=True)
    service, path, owner = find_epoxy(bus, dbus)
    print(json.dumps(snapshot(bus, dbus, service, path)), flush=True)
    if not args.watch:
        return
    # A separate connection becomes a passive monitor. It sends no menu calls.
    monitor = dbus.SessionBus(private=True)
    loop = GLib.MainLoop()
    started = time.monotonic()
    def receive(_connection, message):
        if message.get_interface() != MENU_INTERFACE or message.get_path() != path:
            return
        member = message.get_member()
        summary = event_summary(member, message.get_args_list())
        if summary is not None:
            print(json.dumps({'elapsed_ms': round((time.monotonic() - started) * 1000), **summary}), flush=True)
    monitor.add_message_filter(receive)
    matches = [f"type='method_call',destination='{destination}',path='{path}',interface='{MENU_INTERFACE}'"
               for destination in set((service, owner))]
    matches.append(f"type='signal',sender='{owner}',path='{path}',interface='{MENU_INTERFACE}'")
    monitor.call_blocking('org.freedesktop.DBus', '/org/freedesktop/DBus',
                         'org.freedesktop.DBus.Monitoring', 'BecomeMonitor',
                         'asu', (matches, dbus.UInt32(0)), timeout=3)
    def finish():
        loop.quit()
        return False
    GLib.timeout_add(args.watch * 1000, finish)
    print(json.dumps({'watching_seconds': args.watch, 'read_only': True}), flush=True)
    try:
        loop.run()
    finally:
        monitor.close()
    print(json.dumps(snapshot(bus, dbus, service, path)), flush=True)


if __name__ == '__main__':
    try:
        main()
    except KeyboardInterrupt:
        pass
    except Exception as error:
        # D-Bus errors can contain data: report their class rather than their body.
        print(f'Inspection failed ({type(error).__name__}); check Epoxy, session bus and dependencies.', file=sys.stderr)
        sys.exit(1)
