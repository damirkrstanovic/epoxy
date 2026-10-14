# Desktop UI and Tray Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans for the existing native execution workflow. Implement task-by-task, with an independent whole-branch review at the end. The human reviewed and approved this plan and its clarified scope on 2026-10-14.

**Goal:** Add a useful native GTK control window and system tray while retaining the tested OpenSC signing path and making PIN dismissal and Quit reliable.

**Architecture:** A shared, secret-free desktop controller exposes service activity, diagnostics and a shutdown request. GTK owns all widgets on its existing independent GUI thread. StatusNotifierItem runs on its own runtime/thread with bounded connection attempts and sends presentation requests to GTK; neither waits behind the synchronous NSS/PIN callback. Preserve the current signing engine rather than introducing a backend actor in this feature.

**Tech Stack:** Existing Rust, GTK4 0.10, Tokio, tracing and diagnostics controller; optional `ksni` 0.3 StatusNotifierItem implementation. GTK3/AppIndicator, Java, webviews and PIN caching are excluded.

**Spec:** `docs/superpowers/specs/2026-10-13-linux-native-epoxy-design.md`, GTK/tray, logging and performance sections; `docs/superpowers/specs/2026-10-13-usability-and-pin-sessions.md`, no-caching policy.

## Global Constraints

- Keep GTK and tray in the default desktop build; retain CLI-only and GTK-without-tray builds.
- Service readiness must not wait for GTK or tray registration. Two seconds is a provisional UI fallback threshold, not a permanent tray-registration deadline: show the control window once if no tray host is available, then continue background recovery until shutdown.
- GTK and tray diagnostics reuse the existing controller, including mandatory masking and automatic expiry; tracing remains off by default.
- No PIN caching or convenience authenticated-session retention. No actual PIN/identity/document data enters desktop status, logs, tooltips or notifications.
- For this desktop-controls PR, selected certificate and PIN-context labels appear in the native PIN dialog; the status window and tray show service/operation state. The separate context-selection feature may add an explicit read-only identity/details view in the local GTK window, outside the secret-free status model. Personal identities never enter tray tooltips, notifications or routine logs. Full certificate/key management remains outside Epoxy.
- Closing a PIN window or pressing Escape/Cancel resolves its reply exactly once as cancellation. Quit cancels pending PIN entry and stops new signing work.
- A native signature already executing cannot be undone or safely interrupted by this UI; keep the UI responsive with a Stopping state until native execution/cleanup returns.
- Preserve wire operations/statuses, cryptographic algorithms, NSS module/database selection, and operation-scoped authentication/logout.
- Submit feature PRs only in `damirkrstanovic/epoxy`; do not submit upstream or merge automatically.

## Review Focus

1. Repeated GTK activation/tray clicks must open one control window, never recreate the PIN receiver or panic.
2. Multiple connections and completion/drop paths must not leave activity counters stuck or falsely show Ready while a PIN is pending.
3. Quit during a PIN callback must unblock the callback; queued/cancelled work must not sign after shutdown is requested.
4. Absent/restarting tray watcher or slow D-Bus must keep GTK and service readiness usable, without repeated focus stealing.
5. Diagnostics toggled by CLI, expiry, GTK or tray must agree; sensitive request data must never enter the new state/presentation paths.

---

## Branch, dependency and feature boundary

Workspace: `/home/damirk/Downloads/ESS/epoxy-desktop-ui`, branch `codex/desktop-ui`.
Starting revision: `6a2c207f90d69c4ae8ab3ddcf7bd19a73444d240` (diagnostics PR #2).

This feature depends on diagnostics. While PR #2 remains open, target its branch so the review contains only desktop changes. If diagnostics merges first, rebase onto fork master and target master. Do not duplicate the diagnostics implementation or its commits in the presented feature diff.

The UI consists of a status window, tray integration, native PIN-dialog usability and the lifecycle changes those controls require. Stable context preference/selection, per-signature approval independent of authentication, browser PIN-field integration/Origin enforcement, native build portability and distribution packaging remain independently scoped features. Do not add disabled placeholder actions for those future features. Diagnostics output remains stderr/journal as accepted in PR #2; a managed file sink and open-log-location action require a separate logging-output feature rather than opening a nonexistent app log directory.

## Proposed UI

- Start quietly in the tray when a host is available. Add `--show-window` for explicit presentation; use it in the local desktop test launcher.
- Left-click the tray to open the single control window; right-click shows the menu.
- Control window: approximately 460 px wide with native theme, wrapping text and scalable layout. Title Epoxy; service status and listening endpoint first, connection/activity information second, diagnostics controls below, Quit in the footer. No custom dashboard, hardcoded theme colors or new UI framework.
- Show Starting, Service ready, Working, Waiting for PIN, or Stopping based on actual state. A separate last-result label distinguishes success, cancellation and failure. Do not claim card presence or certificate readiness from listener readiness.
- Diagnostics: operational-level choice, masked-request option, lifetime 1–3600 seconds (default 300), Enable and Disable, and a live expiry indicator. Initial custom startup filters are labelled as startup/custom settings rather than misrepresented as a simple level.
- Tray menu: current service status, Open Epoxy, Enable masked diagnostics for five minutes, Disable diagnostics, Quit. Tooltip contains no certificate/reader/token identities. Use a standard icon name with a small embedded fallback pixmap.
- If the tray host is absent, present the control window once. Watcher loss/recovery changes availability, without automatically hiding a window the user is using. Closing the control window hides it when the tray is available; without a tray it requests Quit, following ordinary desktop close behavior.
- PIN window: approximately 420 px wide, purpose and certificate/context labels wrapped, masked entry with initial focus, Enter to submit, Escape/Cancel/window-close to cancel. Preserve card-required authentication, do not add secret retention. Keep controls usable while it is open; avoid application-wide modality that blocks the diagnostics window. Native transient association is only to an existing visible Epoxy window, not a fabricated browser parent.

### Task 1: Secret-free desktop state and controls

**Files:** Create `epoxy/src/desktop/mod.rs`, `epoxy/src/desktop/state.rs`; export the module from `epoxy/src/lib.rs`. Tests live in `state.rs`.

**Interfaces:** `DesktopController::new(Diagnostics)`, `set_ready(SocketAddr)`, `snapshot() -> DesktopSnapshot`, `connection_started() -> ConnectionGuard`, `operation_started(id: u64, kind: OperationKind) -> OperationGuard`, `pin_requested() -> PromptGuard`, `request_quit()`, `shutdown_requested() -> CancellationToken`, `service_finished()`, and `service_stopped() -> CancellationToken`. Controller clones are Send+Sync and own no GTK/NSS handles.

`DesktopSnapshot` contains endpoint, connection count, enum activity, last enum result and tray availability. It has no arbitrary request strings, certificate/token names, XML or PIN field. Guards release counts on drop; `OperationGuard::complete(status: u32)` records a safe result. `PromptGuard` marks cancellation on explicit cancelled completion; ordinary drop clears pending state. A PIN guard is tied to the currently executing native operation, not a later connection's completed GET_INFO. The controller delegates diagnostics enable/disable/status to `Diagnostics` rather than maintaining another policy.

- [ ] Write behavioral tests before implementation:

```rust
#[test]
fn pin_wait_and_shutdown_take_priority_over_other_activity() {
    let (diagnostics, _subscriber) = diagnostics::subscriber("info", std::io::sink).unwrap();
    let controller = DesktopController::new(diagnostics);
    controller.set_ready("127.0.0.1:17165".parse().unwrap());
    let operation = controller.operation_started(7, OperationKind::Signing);
    let prompt = controller.pin_requested();
    assert_eq!(controller.snapshot().activity, Activity::WaitingForPin);
    controller.request_quit();
    assert_eq!(controller.snapshot().activity, Activity::Stopping);
    drop(prompt);
    drop(operation);
    assert_eq!(controller.snapshot().activity, Activity::Stopping);
}
```

Also test RAII cleanup on errors, two connections, cancellation followed by authentication failure, later success, idempotent Quit, readiness not implying card presence and counters remaining nonnegative under concurrent snapshots.

- [ ] Run `cargo test -p epoxy --no-default-features desktop::state`; verify red for missing state transitions. Use compile scaffolding only to reach behavioral assertions.
- [ ] Implement typed state and short mutex-held updates, with no formatting/I/O/GTK/NSS calls while locked. Snapshot subscribers read immutable copies. Preserve the diagnostics status source of truth:

```rust
pub fn enable_diagnostics(&self, level: diagnostics::Level, requests: bool, ttl: u64) -> anyhow::Result<()> {
    self.diagnostics.enable(level, requests, ttl)
}
```

Store only the controller handles and typed metadata specified above. Cancellation beats a late success when displaying the stopped operation's last result; do not relabel unrelated later operations as cancelled.

- [ ] Run tests and full suite; commit `feat: expose desktop service state and controls`.

### Task 2: GTK control window and complete PIN dismissal

**Files:** Refactor `epoxy/src/ui.rs` into `epoxy/src/ui/mod.rs`, `control_window.rs`, `pin_dialog.rs`, `prompt.rs`; update `pin.rs`, `examples/ui.rs` and GUI tests as required. Keep the existing `ui_pin_method` entry point working for the mock example; add a controller-aware desktop entry point instead of breaking it unnecessarily.

**Interfaces:** `UiHandle::show_window()` posts an idempotent presentation request to the GTK owner thread. `desktop_pin_method(controller: DesktopController, show_window: bool, ct: CancellationToken) -> (PinMethod, UiHandle)` supplies the native prompt and presentation handle. The existing PIN channel remains separate from the bounded/coalesced presentation channel so tray clicks cannot crowd out authentication requests. A private `PendingReply` abstraction owns the oneshot sender and clears/completes it once.

- [ ] Write reply lifecycle tests first, using synthetic secrets:

```rust
#[test]
fn cancel_then_close_resolves_once_without_a_pin() {
    let (tx, rx) = oneshot::channel();
    let mut reply = PendingReply::new(tx);
    reply.cancel();
    reply.cancel();
    assert_eq!(rx.recv().unwrap(), None);
}
```

Also test successful submit versus later close, channel closure, shutdown cancellation, and repeated/coalesced Show requests while a PIN reply is pending. The GTK scenario also verifies hide-versus-Quit window close behavior with/without a tray, and that repeated watcher loss does not keep presenting the window. Test that cancelled entry content does not enter desktop snapshots or log capture.

- [ ] Verify red, then implement the reply abstraction and GTK callbacks. Take the sender once, copy input only for immediate authentication delivery, clear the entry immediately, then close the PIN window. Use the same completion helper from Enter, submit, Escape, Cancel and close-request; never format PIN-channel payloads in errors.
- [ ] Build a single control window with native widgets and the proposed hierarchy. Bind level, lifetime and masked-request controls to the existing controller; render live expiry from `Diagnostics::status()`, including changes originating in CLI or automatic expiry. Refresh at most once per second; operation/prompt state changes can request earlier redraws without busy polling.
- [ ] Make GTK activation idempotent: initialize receiver loops only when their Option is Some; later activations present the existing window. Store widget ownership on the GUI thread and keep tray callbacks free of GTK objects.
- [ ] Add one serialized GTK integration scenario covering actual window presentation, repeated activate, focused PIN entry, Enter, Escape, Cancel, close-request, and diagnostics button activation while a PIN is pending. Use Xvfb/isolated session bus where available, with test-only synthetic prompts; never invoke real NSS signing. Do not silently skip GUI tests and claim full verification when display infrastructure is unavailable.
- [ ] Verify accessible long labels, display scaling and visible PIN masking through rendered/manual QA. No assertion that merely checks the chosen window width or mirrors widget construction counts as a usability test.
- [ ] Run suite and commit `feat: add native desktop controls and reliable PIN cancellation`.

### Task 3: StatusNotifierItem tray on an independent runtime

**Files:** Create `epoxy/src/desktop/tray.rs`; update `epoxy/Cargo.toml`, lockfile and add a small icon asset/helper as needed.

**Interfaces:** Optional Cargo feature `tray = ["ui-gtk", "dep:ksni"]`, included in defaults. `start_tray(controller, ui_handle) -> TrayRuntime` launches an owned thread with a single-thread async runtime. `TrayRuntime` stops on service completion and drops its handle cleanly. Tray model methods produce typed menu/status values and call controller methods; they cannot sign or change card authentication.

- [ ] Test menu behavior/model first: Open requests GTK presentation; Enable delegates `Level::Debug, masked=true, ttl=300`; Disable restores diagnostics baseline; Quit signals shutdown. Include diagnostics enabled with requests off, expiry, GTK absence and privacy of title/tooltip/status. Use the pure typed menu model for behavioral assertions, not tests that merely mirror label construction. Verify that changing service state updates status rather than reconstructing multiple tray instances.
- [ ] Run tests red, then implement ksni Tray with a stable app ID, standard themed icon plus fallback pixmap, left activation and the menu. Use `ksni = { version = "0.3", optional = true }`; do not change GTK4's version or introduce GTK3.
- [ ] Run tray startup outside the service runtime. After two seconds without confirmed tray availability, present the GTK fallback once; do not permanently abandon registration. ksni's `assume_sni_available(true)` permits late watcher registration; its `watcher_offline` implementation records absence and returns true to keep monitoring. `watcher_online` records availability. A spawn handle alone is not proof that a watcher displayed the tray: preserve any offline state established during startup. D-Bus connection/startup failure records Unavailable. Bound individual connection attempts, clean up cancelled attempts, and retry failures with cancellable backoff capped at 30 seconds until shutdown. Missing watcher monitoring does not require repeatedly recreating the tray service. Recovery never hides the fallback window or repeatedly steals focus. Report transitions through operational logging without flooding repeated identical failures.
- [ ] Add isolated D-Bus tests with a synthetic StatusNotifierWatcher for registration, menu callbacks, disappearance/reappearance and missing watcher. Test delayed watcher appearance beyond the fallback threshold, session-bus recovery, and bounded attempt cancellation with a controlled nonresponding watcher/service. Verify recovery creates only one tray and shutdown cancels pending attempts/backoff. Never register test trays on the user's active desktop bus during automated tests.
- [ ] Verify the dependency stays optional: CLI-only builds must not link GTK or ksni; GTK-without-tray builds use the control window fallback.
- [ ] Run suite and commit `feat: add optional status notifier tray controls`.

### Task 4: Service integration and graceful desktop Quit

**Files:** Modify `main.rs`, `server.rs`, `config.rs`, `pin.rs` and UI lifecycle code. Use library desktop types in both library/UI and binary modules, avoiding duplicate controller implementations.

**Interfaces:** Pass a controller clone into the server alongside diagnostics. `--show-window` applies only to GUI mode; reject incompatible CLI-only/TTY use clearly. Readiness is recorded from the actual bound socket. Connection and operation guards are scoped to corresponding lifecycle boundaries. Shutdown uses the controller request token and a separate completion token; cancelling PIN input must not wait on the blocked service runtime.

- [ ] Write tests proving the listener/GET_INFO handshake becomes available while tray registration is deliberately delayed, and that a dropped connection releases its counter.
- [ ] Add synthetic blocking-prompt tests: request Quit through the desktop controller, resolve pending UI prompt as None, verify the request callback unblocks and no signing action occurs. Check the server rejects another queued signing request after shutdown and the private diagnostics socket is removed on clean completion.
- [ ] Verify red, then integrate readiness/activity and shutdown guards. Before each request's native handler and immediately before irreversible `signer.sign`, check the shutdown token. Never interrupt or move opaque NSS/XMLSec objects across threads. Preserve response order/statuses and the same NSS session lifecycle.

```rust
if desktop.shutdown_requested().is_cancelled() {
    return Err(anyhow::anyhow!("service shutting down"));
}
```

Place the equivalent controlled protocol failure inside signing where its return type requires a ResultResponse. Do not return a signed XML response after shutdown has been requested, while acknowledging that native signing already performed cannot be undone.

- [ ] On GUI Quit: show Stopping, prevent new approvals, clear/cancel pending PIN windows immediately on the GTK thread, keep status controls responsive while the service finishes. After native cleanup and controller/socket/tray shutdown, signal service completion and quit GTK. Use a service-lifetime guard to signal completion on early errors/panic unwinding as well, so Tokio runtime cleanup cannot wait forever for the held GTK loop. Do not call process::exit to bypass cleanup.
- [ ] Handle desktop SIGINT/SIGTERM via a GTK/GLib-thread signal source or another independent owner that stays runnable during a PIN wait; remove sources on exit. Preserve existing TTY behavior unless cancellable terminal input is implemented and tested; do not install a signal handler that leaves a blocking TTY read unable to exit.
- [ ] Keep controls safe on GUI failure or missing tray host; logging uses fixed categories/metadata and never serializes desktop prompt payloads. Closed PIN/GUI channels return controlled failures.
- [ ] Run full suite and commit `feat: integrate desktop state and clean shutdown`.

### Task 5: Local test build, performance checks and focused PR

**Files:** Add `epoxy/docs/desktop.md`, update README and local test fixture documentation/scripts outside the PR as needed. Include plan execution evidence with the feature PR.

- [ ] Document tray-first startup, `--show-window`, PIN dismissal, window-close behavior with/without tray, diagnostics controls and clean Quit. Explain service readiness rather than imply a card has been validated. Do not prescribe or install desktop extensions/bar configuration automatically.
- [ ] Run formatting, full default-feature suite, full CLI-only suite and GTK-without-tray suite. Run application-only Clippy in all three configurations. Report unchanged generated NSS-binding strict workspace errors separately, as in PR #2. Confirm no unrelated code/build-package changes entered the feature.
- [ ] Measure cold/warm listener readiness, first handshake and first UI presentation independently. Delay/remove the synthetic tray host and verify listener/handshake performance remains independent of the tray's two-second deadline. Report measured values with debug/release mode and environment, not a generic "fast startup" claim.
- [ ] Produce an isolated local GTK/tray test build in this worktree; keep the currently working diagnostics build and its NSS fixture intact. Use the existing isolated OpenSC database only for user-operated login/test-portal checks. No real PIN, tax XML or identity data in test fixtures/screenshots/PRs.
- [ ] User desktop QA: tray appears on the current Niri/Wayland setup if its bar supplies a watcher; Open Epoxy, diagnostics toggle/expiry, status during native PIN entry, Escape/close cancellation, Quit and restart. If no watcher is present, verify the GTK fallback instead of treating this as service failure. User-run test-portal login/signing should continue to work.
- [ ] Independent whole-branch review, fix Important/Critical findings with failing regression tests, rerun affected/full checks. Open the focused feature PR in the user's fork with explicit diagnostics dependency. Do not merge or submit upstream.

## Completion criteria

The desktop build presents a working tray and control window (or a usable window fallback), diagnostics agree across interfaces, PIN dismissal and Quit unblock pending callbacks, and the tested signing path stays intact. UI/controls remain responsive during a PIN wait; an already-running opaque native operation can delay final process exit but does not freeze the controls. No backend thread migration, secret cache, context selector or browser extension is hidden in this feature.

## Technical references checked during planning

- https://docs.rs/ksni/latest/ksni/ (0.3.6 documentation when checked)
- https://docs.rs/ksni/latest/ksni/trait.Tray.html (`watcher_offline`: return false shuts the service down)
- https://docs.rs/ksni/0.3.6/ksni/struct.TrayServiceBuilder.html (late watcher support)
- Existing GTK4 0.10 source/API and the user's successful native PIN-dialog run; do not upgrade to the newer docs.rs latest GTK version as part of this feature.

## Execution evidence (2026-10-14)

Implemented inline on `codex/desktop-ui`, based on diagnostics PR #2. The existing
local diagnostics binary/NSS fixture remains intact. No context-selector, PIN cache,
certificate manager, browser companion or backend migration is included.

- Desktop lifecycle guards: four red→green tests for pending PIN, shutdown priority,
  cancellation versus authentication failure, concurrent connections and RAII cleanup.
- Reply ownership: two red→green tests for exactly-once cancellation/submit/drop.
- GTK regression: original window-close timed out; new paths pass for Enter, Continue,
  Escape, Cancel, close, Quit with a blocking synthetic callback, repeated activation,
  coalesced presentation and diagnostics during a prompt.
- Tray menu regression: actions now control the existing diagnostics/controller.
- Private-bus checks pass for a watcher appearing after 2.2 seconds, watcher restart,
  bounded nonresponding registration, shutdown during backoff and full bus restart.
  ksni swallows update failures after bus disconnect; a separate zbus close monitor
  fixes the reproduced stale Available state. Failed attempts drop their own runtimes.
- Queued-signing regression originally initialized NSS after shutdown; it now returns
  the compatible error without invoking native initialization. Connection accounting
  is verified over a real synthetic WebSocket handshake.
- Default workspace: 38 automatic tests pass; six GUI/bus tests run separately.
  CLI-only: 35 tests pass. GTK without tray: 37 automatic tests pass, GUI test separate.
  Application-only Clippy with `--all-targets --no-deps -D warnings` passes in all three
  configurations; generated NSS bindings retain the previously established workspace
  strict-Clippy limitations. Formatting passes.
- Debug-build synthetic X11/Xvfb/private-bus startup, fresh processes with OS caches
  uncontrolled: listener 37.2/31.6/31.6 ms; first GET_INFO completed 44.4/32.5/32.4 ms.
  First presentation request: 2221.5 ms for missing-tray fallback; 161.2/160.2 ms with
  `--show-window`. Presentation requests are measured separately from rendered frame
  timing; these are not cold filesystem-cache benchmarks. No NSS/card/PIN access.
- Actual Niri/Wayland tray rendering and user-run ESS portal checks remain manual QA.
  Native-library transient PIN buffers are unchanged; no compliance claim is made.

### Independent review and final fixes

The independent whole-branch review found no Critical or Minor findings and two
Important tray issues. Both were reproduced before fixes: a watcher with no active
host was falsely Available; a late watcher's temporary registration rejection never
retried. Availability now requires a successfully registered tray and an uncached,
bounded/cancellable host-property probe. Host presence changes recover on the same
watcher without recreating the tray. Registration errors end the attempt and enter
bounded background retry; ordinary watcher absence keeps ownership monitoring alive.
The transient-rejection test verifies exactly one live tray remains after recovery.

The GTK scenario additionally checks quiet-launch fallback, close-to-hide with a
host, and no repeated presentation after host loss. Long synthetic labels were
visually inspected at 100% and 200% scaling. Actual compositor focus/accessibility,
ESS signing/portal acceptance, and broader native-buffer compliance remain user QA
or separately scoped work. Reviewer deferrals concerning browser-disconnect/card
removal/timeouts, approval without a PIN callback, context selection, certificate
administration, Origin policy, browser integration and packaging agree with the
approved feature boundaries and are not hidden completion claims for this PR.
