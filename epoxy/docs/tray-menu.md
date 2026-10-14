# Tray menu: user-facing controls and focus investigation

Everyday signing needs no certificate selection when a sole usable certificate is
visible. With several candidates, a unique dedicated Digital Signature certificate
still takes priority. Existing explicit/remembered and missing identities remain
pinned; no automatic substitution of a different card is introduced.

| Option | What it does |
| --- | --- |
| Service ready | The local service is listening; certificate availability is under Certificates. |
| Certificates | Shows certificate holder/purpose and the selected identity. |
| Used for signing | Disabled indication that this certificate is already selected. |
| Use for signing | Selects and remembers another exact certificate. Reload portal after changing it. |
| Certificate details | Shows issuer, validity and purpose. Raw technical fields are only in the explicit GTK details view. |
| Refresh certificates | Rescans after card changes or retries discovery; startup already scans once. |
| Forget saved certificate choice | Shown after an explicit choice. Removes its preference file and reapplies the default rule. |
| Enable troubleshooting logs (5 minutes) | Temporarily enables detailed logs and masked request tracing. |
| Disable troubleshooting logs | Ends that temporary logging session early. |
| Advanced → Log settings | Opens the GTK window for level/trace/lifetime controls. |
| Quit Epoxy | Stops the service and tray. |

The old Advanced rows were PKCS#11 token/PIN contexts, not certificate choices.
ESUFL… was text within ESS's raw token label; the menu did not interpret its prefix.
The default mode checkbox and empty/PIN-only context rows are now removed from the
tray. A selected certificate is already in use without an extra click. GTK labels
and window lookup align with the tray; technical details remain optional.

## Focus: dismissal explained; keeping the menu open remains a host UX issue

Installed dms-shell1.6.2-1 / quickshell0.3.2-1.1. Matching official v1.6.2 tray QML
starts pendingActionCloseTimer after every leaf activation (including toggles),
closes after80ms, and binds layer keyboard focus to showMenu. Submenu navigation
returns early without that timer. User confirms the symptom only after actions.
Epoxy's certificate/troubleshooting-toggle callbacks do not request GTK focus.
Log settings deliberately opens GTK. ksni0.3.6 update_menu compares properties/
children and emits no layout replacement for an unchanged menu; the one-second
Epoxy update is not an unconditional menu rebuild.

Source established popup dismissal/release. Subsequent user testing confirmed
that focus returns to whichever application was active, browser or terminal.
This explains why the previously focused browser or terminal becomes active. It
does not satisfy the request to keep Refresh available without dismissing the
menu. No Epoxy focus fix or host configuration change has been made.
Keeping the host menu open after an action would be a separate usability change.
Other tray applications have not been compared live. Sources:
https://github.com/AvengeMedia/DankMaterialShell/blob/v1.6.2/quickshell/Modules/DankBar/Widgets/SystemTrayBar.qml
https://niri-wm.github.io/niri/IPC.html

Read-only helper: scripts/trace-tray-focus.py (default30 seconds). Start it, focus
the browser, open Epoxy tray, activate a log toggle, then inspect the trace. It
prints only timestamps, window IDs and app IDs; ignores window titles and raw
payloads. No focused application window does not establish which shell surface
owns keyboard focus. Synthetic event-ordering/title-omission check passed; it
has not been run on the user's compositor.

Verification: default74/CLI66/GTK68 automatic tests pass; application Clippy all
configurations, fmt/diff and seven private GTK/bus tests plus three startup/shutdown
smoke cases pass. Sole unnamed-profile fallback and Advanced structure regressions
were red before fixes and green after; stale/Busy/missing/duplicate/date checks
remain green. Review found a focus/documentation overclaim, corrected; no functional
findings. No actual card/PIN/user desktop interaction was performed.


## Read-only D-Bus menu diagnostics

Run from the desktop session, with exactly one Epoxy tray instance:

```sh
python3 scripts/inspect-tray.py                 # current menu structure
python3 scripts/inspect-tray.py --watch 60      # bounded passive event capture
```

Dependencies: Python, python-dbus; watch mode also needs python-gobject.
The collector discovers Epoxy by its SNI Id (`dev.semyon.epoxy`), reads its menu,
then monitors only that menu's D-Bus interface. It never activates an item,
requests a PIN, opens the token, changes selection or contacts the portal.
Unknown labels, certificate identities, issuer/date values and event payloads
are omitted or redacted. Output contains menu IDs, hierarchy, enabled/visible
state, submenu presence, revisions and event kinds. Store redirected logs with
`umask 077`. Session-bus policy may disallow monitoring; snapshot mode still
works without BecomeMonitor. Stop early with Ctrl-C; maximum watch is 300 seconds.

Initial/final snapshots map IDs at those times. Structural changes can reassign
IDs during a capture, so do not assume an old label still belongs to the same ID.
The recorder observes menu traffic; it cannot establish physical pointer
hit-testing, Wayland keyboard focus, or whether a compositor displayed a popup.

### Findings from 2026-10-14

Live read-only GetLayout (including a shallow read of the selected certificate)
showed an enabled submenu with `Used for signing` and `Certificate details…`.
The tick is a selection indicator, not a checkbox. User retesting confirmed the
row opens its submenu. With one usable certificate there is no second choice to
select. Selecting another available certificate uses its `Use for signing`
action; clearing a saved preference can leave the same default selected.
A later passive capture recorded submenu opens/layout reads and leaf clicks
without screenshots. Filtered DMS menu error logs showed no matching errors.

The actual running DMS v1.6.2 SystemTrayBar.qml starts an 80 ms close timer after
leaf actions, including Refresh. Its menu keyboard focus follows visibility.
Native DMS controls such as the Wi-Fi toggle do not run this tray close path.
Thus retaining focus after Refresh requires a host-side keep-open policy or a
separate Epoxy-owned panel; no standard menu property used here requests that
behavior. Do not work around it by grabbing focus back after the host closes.
No running shell files were changed. A structural-update/stale-handle theory was
not established and was not pursued after the user confirmed navigation works.

Sources:
- [DMS tray implementation](https://github.com/AvengeMedia/DankMaterialShell/blob/v1.6.2/quickshell/Modules/DankBar/Widgets/SystemTrayBar.qml)
- [DMS native network controls](https://github.com/AvengeMedia/DankMaterialShell/blob/v1.6.2/quickshell/Modules/ControlCenter/Details/NetworkDetail.qml)

### Automated reproduction

```sh
python3 scripts/test-inspect-tray.py
scripts/test-tray-host.sh
```

The integration probe requires Quickshell, Xvfb/xvfb-run, dbus-run-session and
Python D-Bus/GObject. It uses a private bus with no activation directories and a
private X display, a synthetic SNI/menu and a reduced DMS navigation pattern.
It invokes the QML click handler, verifies nested navigation through real
QsMenuOpener, captures redacted metadata and checks no leaf action was activated.
It does not test physical pointer routing or the real compositor's focus policy.
