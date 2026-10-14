# Desktop controls

The default build includes GTK4 and a StatusNotifierItem tray. The local signing
service starts independently of desktop initialization. “Service ready” means the
listener is ready; it does not establish that a reader, card or certificate is usable.

Start with `epoxy --show-window` to open the control window immediately. Otherwise
Epoxy starts quietly when a tray host is available. If no host is available after
approximately two seconds of GUI startup, it presents the control window once.
That interval is a usability fallback, not a permanent registration deadline:
a late tray host can register without restarting Epoxy. Connection/registration
attempts have five-second bounds and failures retry with backoff capped at 30 seconds.
Each failed attempt releases its D-Bus runtime/connections before retrying.
A watcher must report an active display host before Epoxy considers the tray available.
Host changes are checked without cached properties; transient registration rejection
also enters the retry path, even when watcher ownership does not change.

Tray activation opens the host-rendered tray menu, without mapping a control
window. Its menu provides service status, “Enable troubleshooting logs (5 minutes)”,
“Disable troubleshooting logs” and Quit. Troubleshooting request logs mask sensitive
data and automatically return to normal logging after five minutes. Advanced diagnostics opens the optional GTK window
only on explicit selection. The window has a visible Hide window button when a
tray is available, plus keyboard/window-close dismissal; Quit is a separate action. The tray
contains no certificate identities, token labels, PINs or document data. Its embedded
smart-card icon has a light surface and dark outline, supplied as 16/24/32/48 px
ARGB pixmaps without a theme-dependent icon name. This avoids hosts displaying an
unrecolored dark symbolic icon on a dark panel. A desktop
must supply a StatusNotifierItem host to display it; Epoxy does not install or
configure shell extensions or bars.

Closing the control window hides it when the tray is available. Without a tray,
closing it requests Quit. Tray recovery does not hide an open window. Repeated
tray loss does not repeatedly steal focus.

## PIN entry and shutdown

PINs are entered in the compact native dialog. Purpose, certificate and token/PIN
context labels wrap to accommodate long names. Enter or Continue submits; Escape,
Cancel and the window close button cancel. The entry is cleared when dismissed.
No PIN cache or convenience authenticated session is added. Diagnostic controls
remain usable while the signing thread waits for PIN entry.

Quit (or SIGINT/SIGTERM in desktop mode) cancels pending PIN entry and prevents
new signing work. The UI displays Stopping while native cleanup completes. An
opaque native signature already executing cannot be interrupted or undone; it can
delay exit, and a signed response is suppressed if shutdown has been requested.
TTY mode retains its existing terminal input/signal behavior.

This feature does not add a context selector or certificate administration. A
separate context-selection feature can include a read-only identity/details view.
Key generation, imports/deletion, PIN changes and trust administration remain outside
Epoxy. The existing NSS/XMLSec/OpenSC signing path and protocol are retained.

## Diagnostics

The control window can enable a log level and independently enable masked request
traces for 1–3600 seconds (default 300). It shows the active filter and remaining
lifetime, including changes made through `epoxy diagnostics` and automatic expiry.
Disable restores the startup filter. Detailed request tracing is off by default;
there is no unrestricted credential/document dump mode. Logs go to stderr or the
service journal; this feature does not create its own managed log directory.
See [diagnostics.md](diagnostics.md).

## Builds and verification

```sh
cargo build                                   # GTK + tray + systemd
cargo build --no-default-features --features ui-gtk  # GTK without tray
cargo build --no-default-features              # terminal only
cargo test --workspace
scripts/test-desktop.sh
```

`--show-window` requires a GTK build and `--pin-prompt ui`.
The synthetic desktop script needs Python 3, Xvfb/xvfb-run and D-Bus. It uses a
private bus without service activation directories and an explicit X11 backend,
never the user's session bus, card, PIN or tax data. The six ignored GUI/bus tests
are run explicitly by that script, not silently treated as covered by `cargo test`.
The daemon restart test owns a separate temporary bus. The helper smoke test
checks GET_INFO, dynamic masked tracing/expiry, listener readiness, GUI presentation
and clean SIGTERM/socket removal without loading NSS.

User desktop QA still needs the actual Wayland/X11 desktop and tray host: Open
Epoxy, diagnostics toggles/expiry, status during PIN entry, cancellation, Quit and
restart. Card/portal signing must be tested by the user entering the PIN themselves.
Synthetic tests do not establish production portal acceptance or qualified-signature
compliance.

## Niri window placement

A tray-only launch creates no visible tiled control window while its host is available.
The control window opens through the tray or explicit `--show-window`; the native
PIN prompt still appears when authentication requires it. Placement of these windows
belongs to the compositor. To float Epoxy windows when they are actually opened,
Niri supports this optional app-specific rule:

```kdl
window-rule {
    match app-id=r#"^dev\.semyon\.epoxy$"#
    open-floating true
}
```

Check the app ID with `niri msg pick-window` if needed. Epoxy does not install or
modify compositor rules automatically. See [Niri window rules](https://niri-wm.github.io/niri/Configuration%3A-Window-Rules.html).

Icon source is `epoxy/assets/tray.svg`; committed ARGB buffers keep builds/runtime
independent of SVG/image libraries. Regenerate only after an icon edit with
`python3 scripts/generate-tray-icons.py` (development tools: rsvg-convert and Pillow).
