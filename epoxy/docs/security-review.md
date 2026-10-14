# Pre-review security assessment — 2026-10-15

Scope: the feature stack from `origin/master` through the tray diagnostics PR,
plus the security-review follow-up. Static review covered browser transport,
PIN lifetime, diagnostics, certificate selection, native key binding and tray
inspection. Synthetic tests do not use a real token, real PIN or tax account.

## Findings addressed

- RustSec reported informational unsoundness advisories for `anyhow` 1.0.101
  ([RUSTSEC-2026-0190](https://rustsec.org/advisories/RUSTSEC-2026-0190.html)) and
  `rand` 0.9.2
  ([RUSTSEC-2026-0097](https://rustsec.org/advisories/RUSTSEC-2026-0097.html)).
  The lockfile now uses patched 1.0.103 and 0.9.3 respectively. No claim is made
  that their affected paths were exploitable in Epoxy.
- The TTY PIN prompt printed card/certificate/XML-derived text verbatim.
  Control characters are now replaced and each field is bounded to 160
  characters, matching the existing graphical prompt's display helper.
- Page-provided PINs now have a generous 1024-byte upper bound and retain the
  existing NUL rejection. Rejection messages never include the supplied value.
- XML parsing explicitly sets `no_net: true`, with recovery disabled and the
  existing defaults leaving DTD loading/entity substitution disabled. This is
  defense in depth, not a claim that an XXE exploit was demonstrated or that
  NONET alone blocks local-file access under arbitrary parser configurations.

## Controls reviewed

- WebSocket access is restricted to loopback and exact approved portal Origins;
  the optional development exception accepts an absent Origin only.
- Diagnostics controls use a private runtime directory/socket, no-follow path
  handling, peer credentials, bounded messages and deadlines. Detailed tracing
  is opt-in and expires; masking uses an allow-list rather than a PIN-name-only
  blacklist. No unmasked payload mode is exposed. Independently enabled native
  library debug output is outside this masking boundary.
- Certificate selection is tied to an exact identity. Signing revalidates the
  live token and native key's slot; missing/replaced identities do not silently
  switch to a different card.
- The tray inspector invokes only discovery/property/layout reads and observes
  menu metadata. It redacts unknown labels and never activates menu actions.

## Remaining trust boundaries and limits

- An Origin header protects against unrelated browser pages, not local malware.
  A local process can forge it. Same-user processes, the approved portal's own
  scripts, the desktop session and native libraries are trusted by this design.
  The displayed GET_INFO host is a page claim, not proof of the requester's identity.
- Page PINs are used once per request, without application PIN caching. However,
  ordinary Rust/GTK/JSON/C/NSS buffers are not guaranteed to be zeroized. No
  memory-forensics or swap/core-dump resistance is claimed. A future zeroization
  change must cover the whole path, not just wrap one String.
- A page-supplied PIN bypasses the native prompt as explicitly configured by the
  user-facing policy. Approved or compromised portal code can therefore request
  signing without an additional native confirmation. This is not a document
  approval UI or a claim of qualified-signature regulatory certification.
- Native PKCS#11 operations may block in external libraries. Transport deadlines
  and diagnostics responsiveness do not make those native calls cancellable.
- RustSec checks the Cargo lockfile, not vulnerabilities in the installed NSS,
  XMLSec, GTK, OpenSC or PC/SC packages. Keep those distribution packages updated.
- DMS's tray-action dismissal remains a host UX limitation, not a fixed Epoxy
  focus defect. No running desktop configuration was changed.

This review is not a penetration test, comprehensive fuzzing campaign, legal
certification or guarantee of absence of vulnerabilities. Licensing findings
and binary-release obligations are recorded in [THIRD_PARTY.md](../../THIRD_PARTY.md).

## Verification evidence

- RustSec database fetched on 2026-10-15, commit
  `edb58f5974b83eb8302332821cb6d389777aa351` (1329 advisories). After the two
  lockfile updates, `cargo audit --no-fetch --json` reports zero vulnerabilities
  and no informational warnings.
- Default workspace tests: 88 passed; CLI-only: 79 passed; GTK-only: 81 passed.
  Explicitly ignored hardware/display fixtures are exercised separately below.
- The new terminal-control and oversized-PIN regressions failed before the
  implementation changes, then passed with the fixes.
- Application Clippy passes with `--no-deps -p epoxy --all-targets -D warnings`
  in default/all-features, CLI-only and GTK-only configurations. Whole-workspace
  Clippy still flags 17 preexisting warnings in bindgen-generated C bindings;
  those generated files were not edited or broadly suppressed.
- Seven private GTK/D-Bus tests and three startup/shutdown smoke runs pass.
  Synthetic NSS signing passes and XMLSec independently verifies its signature.
- Python inspector/focus unit tests pass. The isolated real-Quickshell probe
  verifies nested navigation and masked event capture without leaf activation.
- Formatting and diff whitespace checks pass. A second source review checked
  the follow-up fixes and licensing claims; its package-count correction was
  incorporated (276 total, 274 registry packages).

Tests requiring Unix/TCP sockets were run outside the filesystem sandbox on
private endpoints after the sandbox denied socket creation. Tests did not
activate real tray actions, read real PINs, access the real token or change the
running desktop session.
