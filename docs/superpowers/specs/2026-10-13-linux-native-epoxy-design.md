# Epoxy Linux integration and diagnostics design

Status: ready for design review, before implementation planning.

Repository: https://github.com/damirkrstanovic/epoxy. Base revision: `929c3bcce0be988ef28560f5355fce4c049224a0` (v0.5.0).

## Purpose

Provide a usable Linux application for ESS QCA smart-card login and declaration signing on ePorezi. Retain native GTK dialogs and add tray controls, keep the actual card PIN out of the webpage, use OpenSC and the host PC/SC service instead of vendor middleware, make PIN-context selection explicit, support fast startup, and package the software using normal Linux distribution conventions.

The existing Epoxy implementation is the foundation. Keep NSS and XMLSec initially, and validate their ESS compatibility before considering a replacement signing engine. Decompilation of SmartBox informed protocol compatibility; vendor source, real card data, raw logs, and credentials must not enter this repository.

## User requirements

1. The actual PIN is entered in a native application dialog or terminal prompt, never in the ePorezi page.
2. The ESS card works through OpenSC PKCS#11 and the distribution's PC/SC stack. SafeNet is not required by the replacement.
3. The user can identify and select a PIN context within the application, rather than choosing between indistinguishable reader entries.
4. Keep a lightweight native Rust application, with GTK and a tray because they improve usability.
5. GUI initialization must not delay local-service readiness. Startup performance must be measured.
6. Support source installation and normal Arch, Debian/Ubuntu, and Fedora/RPM packages; remove distribution-specific library and Java paths.
7. Provide high-quality operational logs and detailed wire/request tracing when needed. Detailed tracing is off by default and can be changed dynamically without restarting the application. Sensitive fields are masked.
8. Work on separate branches in the user's fork and submit changes as reviewable PRs. Do not merge them automatically or open implementation PRs against upstream.

## Architecture

The Rust service implements the existing SmartBox browser protocol over loopback WebSocket connections. NSS loads OpenSC through an app-owned configuration/context; XMLSec creates the compatible XML signature. GTK presents native context selection, signing approval, and PIN entry. A StatusNotifierItem tray provides application status, context selection, diagnostic controls, and quit.

The service binds and can complete a protocol handshake before tray registration or GTK dialog initialization finishes. Tray setup runs independently. Lack of a tray host is a recoverable usability limitation, not a service failure; native dialogs and CLI controls remain available. Keep CLI builds for headless use, but retain GTK and tray functionality in the normal desktop build.

A small browser companion prevents accidental entry of the actual PIN into the portal's form. It hides only the known SmartBox PIN controls, explains native PIN entry, and supplies a nonsecret compatibility placeholder where required by the portal. The local helper ignores the browser PIN field in every mode. The browser companion must not intercept, acquire, or receive the actual card PIN.

## Runtime logging

### Normal operation

Default to `info` operational logging with detailed payload logging disabled. Log lifecycle, endpoint readiness, native-library initialization result, operation name, outcome, duration, and a locally generated correlation ID. Correlation IDs must not be the portal's session token. Default logs should omit certificate subjects, card serials, token labels containing personal names, signed XML, incoming documents, and PINs.

Failures should include the operation and relevant native error code/cause, with a useful local explanation. The browser receives compatible protocol status codes. User cancellation, no reader/card, invalid context, missing module, invalid certificate, PIN rejection/lockout, and signature failure should be distinguishable locally. Never run automatic repeated PIN attempts.

### Detailed diagnostics

Provide masked wire-format request and response traces as a separate setting from log verbosity. Preserve operation names, JSON structure, nonsensitive values, and diagnostic metadata. Redact sensitive values before formatting or output, including nested PIN/password/passphrase fields, authorization/cookie/token/session credentials, personal identifiers, private-key material, and certificate identity data.

Document/XML and certificate blobs are masked by default, with length and digest metadata where useful. Their base64 form must not be mistaken for encryption. This first implementation does not provide an unmasked PIN/credential mode or an unrestricted document-dump mode. Detailed wire tracing describes the received structure with mandatory masking; it does not mean retaining an exact unredacted byte stream.

Malformed JSON, excessively deep structures, unsupported payload types, and oversized frames must fail closed for tracing: log bounded metadata and parsing/size errors, not an unsanitized fallback string. Sanitization must precede tracing calls at every application request/response boundary. Error formatting must not reintroduce original request bodies.

Trace output should have a bounded event size and bounded file retention. Optional files go to the app's private XDG state directory with mode 600 and directory mode 700; ordinary operational logs can continue to stderr/journal. Application-level masking cannot promise to sanitize independent operating-system core dumps or external native-library debug output. Do not enable PKCS#11 library debug environments by default.

### Dynamic controls

Support commands with these intended semantics:

```text
epoxy diagnostics status
epoxy diagnostics enable --level debug --requests masked --ttl 300
epoxy diagnostics disable
```

These commands contact the running instance, rather than starting another signing server. Tray actions expose the same controller: enable masked tracing for five minutes, disable tracing immediately, and show the current diagnostic state/expiry. A GTK diagnostics panel may expose verbosity and lifetime controls without adding a second implementation of logging state.

Use tracing-subscriber's reloadable filter for runtime verbosity and shared typed settings for payload policy. Accepted runtime levels are error, warn, info, debug, and trace. Default diagnostic lifetime is 300 seconds; accept a bounded range of 1 to 3600 seconds. On expiry, restore the pre-diagnostic verbosity and disable request payload tracing. A restart begins with payload tracing off unless explicitly enabled for that invocation.

The local control channel is separate from the browser protocol: a Unix-domain socket in an app-owned mode-700 runtime directory, with mode 600 and same-user peer validation. Do not accept diagnostic controls over the browser's WebSocket. Bound control messages to 8 KiB, use typed commands, and reject invalid levels, payload policies, and durations. Avoid following symlinks or deleting an unrelated stale endpoint when creating the socket.

Control handling and expiry must remain responsive while the native PIN prompt is open or card operations are blocked. This requires independent control execution; it must not wait behind the current single-threaded signing callback. Changing diagnostics must not change signing authorization or native backend state.

## OpenSC setup and PIN-context selection

Discover the distribution's OpenSC module using standard package/p11-kit information, with an explicit module-path override. Use app-owned NSS state rather than requiring edits to a browser's NSS database. Check NSS initialization and native allocation/copy results; propagate failures instead of returning invalid contexts.

Display reader name, token label/PIN-context name, certificate purpose, and intended operation. Bind a selected certificate to its originating module and stable token identity throughout signing. Selection must not depend on slot index or list order. Persist only a user-requested preference, never the PIN. Reconfirm selection when the card is replaced, removed, or becomes ambiguous.

For the tested ESS card, OpenSC exposes a general PIN context and a Digital Signature PIN context through the same reader. Only the latter successfully signed in the existing SmartBox/OpenSC test. This is an acceptance case, not a universal rule that slot 1 is always correct.

Retain card-required authentication semantics, including keys reporting `CKA_ALWAYS_AUTHENTICATE`. Do not weaken token policy to make signatures work. Test the actual NSS/XMLSec path with ESS because success through Java/OpenSC does not establish NSS compatibility.

## GTK and tray usability

Use a native StatusNotifierItem implementation, preferably the Rust `ksni` backend, rather than mixing GTK3 AppIndicator and GTK4 in one process. Tray integration should be an independent feature and should not impose a Java or webview dependency.

Tray menu actions include application/service status, selected context, choose context, enable/disable diagnostics, open the private log location, and quit. Do not put the full personal certificate subject or identifiers into diagnostic logs when displaying them in native UI. Native window-close and Cancel actions must resolve pending requests promptly. Quitting cancels prompts and shuts down sockets cleanly.

The [usability and PIN session review](2026-10-13-usability-and-pin-sessions.md) records source findings and proposed refinements. Use a compact contextual authorization dialog with explicit action buttons and complete keyboard/cancellation handling. Signing approval must be independent of a PIN callback, so an already-authenticated token cannot silently authorize new signatures. PIN caching and convenience authenticated-session retention are excluded. Keep authentication operation-scoped, preserve card-required per-use authentication, and remember only nonsecret context preferences.

On desktops without StatusNotifierItem support, show a useful message and keep working. Document desktop-specific tray support requirements rather than installing shell extensions automatically. A default desktop installation retains GTK and tray support; a CLI-only build remains supported.

## Browser boundary and PIN-page integration

Validate the WebSocket handshake Origin against exact approved portal origins, initially `https://eporezi.purs.gov.rs` and `https://test.purs.gov.rs`. The JSON `host` field is untrusted input and is not evidence of origin. Listen only on loopback, including the address families needed by the portal, with bounded frame size and connection/request timeouts. Developer/test access without Origin must be an explicit mode rather than a default exception.

Inspect the actual PIN controls and frontend validation on the production and test pages before implementing the browser companion. Do not replace unrelated password fields or alter the tax document. Keep permissions restricted to the portal pages and the local helper capability check. Failure to find the native-PIN-capable helper must produce a clear explanation rather than silently reverting to real-PIN entry on the page.

Native approval identifies whether the request is login or declaration signing, the selected certificate/context, and the document purpose that can be established from the XML. Unknown or ambiguous signing requests require explicit native review or rejection; a trusted origin alone must not authorize arbitrary signing. No real declaration is submitted automatically during implementation testing.

## XML and protocol compatibility

Preserve SmartBox-compatible operation names, field types, statuses, certificate aliases, and reconnect behavior required by the portal. Verify rather than assume that an empty `GET_INFO` payload is sufficient after navigation/reconnection.

Retain RSA-SHA256, SHA-256 digest, enveloped transform, inclusive canonicalization, and X.509 metadata required by the working flow. Validate ESS distinguished-name ordering/encoding, Unicode, repeated attributes, issuer serial formatting, and certificate eligibility. Existing Epoxy filtering and formatter behavior must be tested against ESS; do not copy identity-specific data into fixtures.

Use synthetic certificates and software tokens for automated signing tests. Cryptographic verification is necessary but not sufficient: user-run login and declaration signing in the ePorezi test environment must be accepted before claiming ESS end-to-end support.

## Performance and distribution packaging

Measure listener readiness, first protocol handshake, first token discovery, and first PIN prompt separately. Record cold and warm results with build mode, system, and dependency versions. Desktop/tray startup must not hold up readiness; missing tray services and slow D-Bus replies should not introduce unbounded startup waits.

Replace hardcoded library/include paths with pkg-config discovery and portable header includes. Use XDG directories and standard executable/desktop/service installation locations. Retain a standalone foreground launch mode for distributions without systemd, and optional systemd user service/socket activation for those using it.

Provide source-release installation plus Arch, Debian/Ubuntu, and Fedora/RPM recipes. Validate builds and installation metadata on representative systems. Do not describe a glibc/native-library-dependent executable as universal across every Linux ABI. Keep pcscd and reader drivers supplied by the host distribution. AppImage/Flatpak are separate future assessments because host smart-card access and native-library integration need explicit validation.

## Reviewable PR sequence

1. **Runtime diagnostics:** safe structured logs, masked wire tracing, reloadable levels, independent local controls, automatic expiry, tests, and usage documentation. This is independently reviewable and provides the controller used by the tray later.
2. **Portable native backend setup:** pkg-config/header discovery, validated NSS/FFI initialization, and app-owned OpenSC configuration. Include targeted failure tests and build checks.
3. **ESS context selection:** meaningful labels, native selection/preferences, certificate-to-token binding, cancellation handling, and ESS compatibility tests. Live PIN/signing checks remain user actions.
4. **Tray and startup:** StatusNotifierItem tray integrated with GTK and diagnostics, listener-first/lazy GUI initialization where appropriate, graceful tray absence, and startup measurements.
5. **Browser PIN integration and origin policy:** restricted browser companion, native-PIN capability detection, validated origins/payloads, and browser integration tests using dummy PINs.
6. **Distribution packaging:** normal install layout and source/Arch/Debian/RPM packaging with verified dependency discovery and documented supported targets.

Use branches in `damirkrstanovic/epoxy`, one per independently reviewable PR. Prefer branches based on master for independent work; dependent PRs must state their base/dependency and avoid unrelated changes. Keep implementation plans and validation with the PR that implements them. Do not include SmartBox decompiled source, local runtime reports, real card data, or private diagnostic artifacts in PRs.

Prepare each feature for potential later upstream submission, with focused APIs, tests and documentation. Dependent PRs must expose only their own change against the prerequisite branch and explain the eventual integration order. Do not submit PRs or merge requests to the original author's repository. Do not introduce PIN caching or convenience authenticated-session retention in any PR.

## Verification criteria

- Default logging does not emit request bodies or sensitive identity data.
- Enabling tracing without a restart reveals useful masked request/response structure; disabling/expiry stops it immediately for subsequent events.
- PINs and encoded credentials in valid, nested, malformed, and oversized inputs do not appear in captured application logs.
- Runtime controls cannot be invoked from the portal, another user, or an insecure/symlinked endpoint, and remain responsive during a pending PIN prompt.
- GTK and tray remain usable; lack of tray support does not prevent service readiness or signing dialogs.
- ESS signing uses OpenSC only and the explicitly selected stable PIN context; card removal/replacement does not silently select another key.
- Cancellation and native/library failures return controlled outcomes and useful masked local diagnostics.
- Signatures verify cryptographically and are accepted by user-run test-environment login/declaration flows.
- Packaging/build checks cover the declared distribution targets and CLI/desktop feature combinations.

## Evidence and limits

The earlier local investigation confirmed ESS login and test-environment declaration signing through SmartBox modified to use OpenSC's Digital Signature PIN slot. It also reproduced and fixed a SafeNet/PC/SC shutdown crash, but SafeNet signing still failed despite a logged-in token. Epoxy's ESS compatibility has not yet been demonstrated.

Source inspection identified raw request logging before PIN-field skipping, missing Origin checks, hardcoded build paths, and unchecked NSS context initialization in Epoxy v0.5.0. SmartBox starts its tray before its server and logs browser PIN requests. Its exact slow-startup bottleneck was not profiled. These findings motivate the changes but do not justify claiming measured startup improvements or complete security coverage before testing.

## Technical references

- https://github.com/damirkrstanovic/epoxy
- https://github.com/semyon2105/epoxy
- https://docs.rs/tracing-subscriber/latest/tracing_subscriber/reload/index.html
- https://docs.rs/ksni/latest/ksni/
- https://test.purs.gov.rs/user/login.html
