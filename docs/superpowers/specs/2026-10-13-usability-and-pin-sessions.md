# Usability and PIN session review

Status: source-based analysis and recommendations; no caching or UI behavior has been implemented.

Reviewed Epoxy v0.5.0, revision `929c3bcce0be988ef28560f5355fce4c049224a0`. This supplements the Linux integration design. Live ESS behavior through Epoxy/NSS remains to be tested by the card owner.

## Findings

| Finding | Evidence | Consequence and proposed response |
| --- | --- | --- |
| Oversized, generic PIN window | `epoxy/src/ui.rs::new_pin_popup`: width 720, title "epoxy", button "OK" | Use a compact, approximately 400-pixel window, allow text wrapping/accessibility scaling, and name the action "Log in" or "Sign declaration". |
| Context is difficult to understand | Dialog prints token name and certificate name; signing reason may be a server path or "unidentified" | Show operation, validated requesting portal, selected PIN context and certificate. Keep technical identifiers in expandable details. Do not present inferred document details as verified facts. |
| Unnecessary authentication churn | `nss.rs::get_certs_in_token` and `authenticate` return `TokenLogoutGuard`; its destructor unconditionally calls `PK11_Logout` | Certificate listing and signing discard login state. Even friendly-token listing receives a logout guard without authenticating. Introduce explicit authentication ownership/lifetime, rather than simply deleting logout. Login state is shared across sessions for a token in the application. |
| Approval currently depends on a PIN callback | `server.rs::get_signed_xml` prompts via authentication, then signs; there is no independent approval gate | If authentication is reused or the token needs no PIN, the native prompt may disappear entirely. Add explicit per-signature approval independent of whether a PIN is needed. |
| Incomplete cancellation and keyboard behavior | `ui.rs` handles buttons/Enter but has no explicit Escape, close-request, initial entry focus or request timeout | Specify and test all dismissal paths; cancel exactly once and promptly resolve the browser request. Source inspection alone does not prove the window-close path hangs. |
| Dialog and connection lifetimes are not bound | `PinInfo` contains certificate/reason, not an operation ID or cancellation handle | Cancel pending authorization on browser disconnect, timeout, card removal and quit. Recheck cancellation immediately before signing; a signature already performed cannot be undone. |
| PIN wait blocks service execution | Synchronous `reply_rx.recv()` in the password callback; server uses a current-thread runtime and synchronous handlers | Move serialized native crypto work away from connection/control handling. Diagnostics, expiry and cancellation must stay responsive while a dialog is open. |
| Prompt queue has unclear behavior | Bounded channel uses `try_send`; UI creates windows as messages arrive without an explicit authorization state machine | Permit one active authorization, bound pending work and report busy/cancelled outcomes. Avoid overlapping prompts and silent queue failures. |
| Wrong PIN gives little guidance | NSS callback rejects `retry != 0`; protocol returns generic authentication errors | Show rejection distinctly from cancellation/missing card. Never automatically retry a PIN; show remaining attempts only if reliably available from the token. |
| Tray availability is not guaranteed | A desktop may lack a StatusNotifierItem host; native window is modal without a browser transient parent | Use tray status/attention where supported, plus a reliable native dialog path. Avoid repeated focus stealing and do not promise global modality on Wayland. |

These are source findings and expected consequences, not results of interactive desktop testing. UI accessibility, compositor focus behavior and actual prompt counts need live checks.

## What can be cached?

| Mechanism | Benefit | Recommendation |
| --- | --- | --- |
| Selected context and certificate metadata | Avoid choosing the same context repeatedly | Remember an explicit preference using stable identity; invalidate on removal/replacement/ambiguity and recheck certificate eligibility before use. No PIN required. |
| Authenticated token session | Avoid redundant ordinary login requests without retaining an application PIN copy | Support an opt-in short lifetime where token policy permits. This does not authorize a signature and does not override per-key authentication. |
| Application-held PIN in memory | Could supply the PIN again for later authentication | Technically possible, but defer. This retains a reusable secret, needs explicit policy/consent, secret-lifetime handling and native-library copy analysis, and must not silently bypass per-use consent. |
| PIN on disk or in a keyring | Survives app restart | Exclude from this design. |

Session retention is not equivalent to PIN storage. Conversely, absence of an application cache is not proof that NSS/OpenSC or a native callback retains no transient copies. The current Rust callback creates String/CString copies and passes an allocated buffer to NSS; it does not explicitly zero its own copies.

The tested ESS signing key reports `CKA_ALWAYS_AUTHENTICATE=true`. OpenSC maps PKCS#15 per-use consent to this attribute; context-specific authentication is required for the operation. Ordinary token login retention cannot replace it. OpenSC also documents that PIN caching normally respects per-use consent. Do not enable `pin_cache_ignore_user_consent` as a usability shortcut. A successful Java/OpenSC signature does not establish how NSS/XMLSec handles repeated ESS signing.

## Proposed user experience

1. Choose a clearly labelled PIN context once, preferably the eligible Digital Signature context for this card; preserve an explicit preference rather than hardcoding slot 1.
2. List public certificates without authentication when the token allows it. If private enumeration requires login, explain "Unlock certificate list" separately from signing.
3. Present one compact native authorization dialog for each signing request. Show the portal, operation and selected context; include PIN entry only when authentication is required. Combine approval and PIN entry where possible, instead of producing two successive dialogs.
4. Use explicit action buttons, keyboard navigation, initial PIN focus, Enter to confirm, Escape/window-close to cancel, and accessible layout/text. Clear the entry on dismissal and keep secrets out of labels, tooltips, clipboard automation and logs.
5. Native approval remains required even while a session is authenticated. Never silently sign because a PIN was entered for an earlier request. For unidentified XML, explain what cannot be identified and require review or reject the operation.
6. Tray displays Ready, Waiting for approval, Working, or Card unavailable. Add "Lock card session" and session-expiry status. Do not put sensitive document details into desktop notifications.

Optional session retention proposal: disabled by default, user-selectable up to five minutes, with both idle expiry and a five-minute absolute ceiling after authentication. Expire on card removal/replacement, logout, app quit, manual lock and desktop session lock. A context switch cancels incompatible pending work and releases any retained authentication under the same policy. Keep the feature unavailable when required lock/removal monitoring cannot be established; expiry must not depend on the blocked crypto worker. This is a proposed setting, not a claim that the ESS signing PIN can be reused for five minutes.

## Validation required before changing authentication lifetime

- Synthetic/software-token tests: ordinary authentication retained within the policy, expiry/logout exactly once, cache isolation by module/token/context and no automatic signature when authenticated.
- Per-use-authentication tests: each required operation authenticates correctly; retaining ordinary login does not suppress `CKU_CONTEXT_SPECIFIC` handling. No consent-override configuration is applied.
- User-run ESS tests: certificate listing, portal login and repeated test-environment declaration signatures, recording prompt count and authentication outcome without recording PINs or document contents.
- Cancellation tests: Escape, window close, Cancel, browser disconnect, card removal, expiry and quit while prompting and while native work is blocked. A cancelled queued operation never signs.
- Responsiveness checks: diagnostics can be changed while a prompt waits; multiple connections cannot overlap authorizations or consume another request's approval.
- Desktop checks: GTK keyboard navigation, long labels, display scaling, screen lock and tray absence on representative X11/Wayland desktops.

## PR boundaries

Keep all PRs in `damirkrstanovic/epoxy`; submit nothing to the original repository. Design each feature for later upstream review with a single clear outcome, minimal public API changes, its own tests/documentation and an explicit dependency list. A dependent PR must be reviewable as a diff against its prerequisite, rather than repeating prerequisite changes. Independent features should branch from master; stacked features should identify the prerequisite branch and intended eventual base. Avoid combining UI cleanup, authentication retention and native backend restructuring in one patch.

Suggested usability splits: (a) complete cancellation/keyboard handling, (b) compact contextual authorization and explicit approval, (c) stable PIN-context preference, (d) optional authenticated-session retention after backend authentication tests, (e) tray status and manual session lock. Authentication retention depends on the approval gate and responsive expiry/control execution. Existing operational diagnostics and packaging retain their separate PR boundaries.

## References

- [OpenSC maintainer discussion of multiple PIN contexts, per-use consent and caching](https://github.com/OpenSC/OpenSC/discussions/3467)
- [PKCS#11 v3.1 specification](https://docs.oasis-open.org/pkcs11/pkcs11-spec/v3.1/pkcs11-spec-v3.1.html) (reference for implementation review; the web fetch of the complete document was unavailable during this review).
- Local source: `epoxy/src/ui.rs`, `epoxy/src/nss.rs`, `epoxy/src/server.rs`, `epoxy/src/main.rs`.
