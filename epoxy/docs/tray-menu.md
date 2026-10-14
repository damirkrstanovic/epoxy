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

## Focus issue: resolved by user testing

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
The terminal-specific issue is closed as normal host popup behavior. No Epoxy
focus fix, host edits, configuration changes or reopen hacks were required.
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
