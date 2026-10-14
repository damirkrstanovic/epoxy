# PIN flow follow-up: evidence and manual checks

The user approved request-scoped page PIN use on 2026-10-14, replacing the
earlier native-only policy. Request PIN handling and compact GTK presentation
are separate stacked patches on the certificate branch, for the user fork only.

Policy: nonempty page PIN takes priority only for its request; absent/empty uses
native/TTY. No persistence/cache/cross-request reuse/new authentication retention.
One-use callbacks, NSS retry rejection, shutdown checks, invalid NUL rejection and
redacted Debug/wire diagnostics are covered. Certificate/context/key-slot binding
is retained. See request-pins.md.

UI: fixed-size width360; action and page-reported hostname on main view, bounded
wrapping/two-line ellipsis; certificate/card labels collapsed. GET_INFO's hostname
is display-only and visibly unverified, not an authenticated Origin. Host validation
accepts at most253 ASCII bytes with standard1–63-character DNS labels.

Regression evidence: missing request prompt API was red before implementation;
request parsing/one-use/fallback/shutdown/NUL/debug tests now green. Fixed-window
GUI regression was red before layout changes, green after. Long-content scenario
uses a253-byte hostname and an over160-character unbroken action; collapsed
geometry stays<=420x400 logical pixels at100%/200%. Entry focus, Enter/Continue,
Escape/Cancel/close/shutdown, entry clearing and diagnostic controls pass.

Final automatic suites: default73, CLI66, GTK68 pass. Application Clippy and
format/diff checks pass. Seven private GTK/bus tests and three startup/shutdown
smoke scenarios pass at both scales; synthetic NSS slot/cleanup/signature test
passes and xmlsec1 reports Verification status: OK. No real card/PIN/portal or
user session bus was accessed.

One intermediate parallel CLI run failed the existing logging-capture assertion
for `certificate control completed`; serial and final parallel suites pass. Five
unchanged-baseline binary runs also passed, so the intermittent failure's cause
is not established. It is recorded for follow-up, not claimed fixed by these changes.

Independent policy review found a shutdown guard bypass and verified its fix.
UI review prompted long-content limits and visible hostname provenance; both
fixed and re-reviewed. The initial floating concern was withdrawn after checking
GTK Wayland min=max hints and niri automatic floating documentation.

Placement evidence (source-level, not a real niri-session test):
GTK passes Window.resizable through to GdkToplevelLayout; its Wayland nonresizable
branch sets minimum and maximum dimensions equal and transmits xdg_toplevel size
hints. Niri automatically floats fixed-size windows and centers new floating
windows by default; compositor rules can override this. Sources:
https://github.com/GNOME/gtk/blob/main/gtk/gtkwindow.c
https://github.com/GNOME/gtk/blob/main/gdk/wayland/gdktoplevel-wayland.c
https://niri-wm.github.io/niri/Floating-Windows.html
https://niri-wm.github.io/niri/Configuration%3A-Window-Rules.html

Manual acceptance: quit earlier helper, launch run-pin-ux.sh, test normal correct
page PIN (no native prompt), and an empty/missing page PIN (compact native prompt,
if portal permits empty input). Check actual niri floating placement and both
login/declaration signing. Do not deliberately submit wrong PINs to a real card.
Certificate Refresh/default/manual remembered choices and dynamic masked logging
remain available. Native real-card and test-portal acceptance remain user-operated.
