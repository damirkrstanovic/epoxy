# Usability and PIN session review

Status: source-based analysis and recommendations; no UI behavior has been implemented. PIN caching and authenticated-session retention are excluded by user direction.

Reviewed Epoxy v0.5.0, revision `929c3bcce0be988ef28560f5355fce4c049224a0`. This supplements the Linux integration design. Live ESS behavior through Epoxy/NSS remains to be tested by the card owner.

## Findings

| Finding | Evidence | Consequence and proposed response |
| --- | --- | --- |
| Oversized, generic PIN window | `epoxy/src/ui.rs::new_pin_popup`: width 720, title "epoxy", button "OK" | Use a compact, approximately 400-pixel window, allow text wrapping/accessibility scaling, and name the action "Log in" or "Sign declaration". |
| Context is difficult to understand | Dialog prints token name and certificate name; signing reason may be a server path or "unidentified" | Show operation, validated requesting portal, selected PIN context and certificate. Keep technical identifiers in expandable details. Do not present inferred document details as verified facts. |
| Authentication lifetime needs explicit ownership | `nss.rs::get_certs_in_token` and `authenticate` return `TokenLogoutGuard`; its destructor unconditionally calls `PK11_Logout` | Preserve operation-scoped authentication and logout; do not extend it for convenience. Even friendly-token listing receives a logout guard without authenticating: test ownership so enumeration cannot interfere with another in-flight operation. Serialize access per token/context. |
| Approval currently depends on a PIN callback | `server.rs::get_signed_xml` prompts via authentication, then signs; there is no independent approval gate | If authentication is reused or the token needs no PIN, the native prompt may disappear entirely. Add explicit per-signature approval independent of whether a PIN is needed. |
| Incomplete cancellation and keyboard behavior | `ui.rs` handles buttons/Enter but has no explicit Escape, close-request, initial entry focus or request timeout | Specify and test all dismissal paths; cancel exactly once and promptly resolve the browser request. Source inspection alone does not prove the window-close path hangs. |
| Dialog and connection lifetimes are not bound | `PinInfo` contains certificate/reason, not an operation ID or cancellation handle | Cancel pending authorization on browser disconnect, timeout, card removal and quit. Recheck cancellation immediately before signing; a signature already performed cannot be undone. |
| PIN wait blocks service execution | Synchronous `reply_rx.recv()` in the password callback; server uses a current-thread runtime and synchronous handlers | Move serialized native crypto work away from connection/control handling. Diagnostics, expiry and cancellation must stay responsive while a dialog is open. |
| Prompt queue has unclear behavior | Bounded channel uses `try_send`; UI creates windows as messages arrive without an explicit authorization state machine | Permit one active authorization, bound pending work and report busy/cancelled outcomes. Avoid overlapping prompts and silent queue failures. |
| Wrong PIN gives little guidance | NSS callback rejects `retry != 0`; protocol returns generic authentication errors | Show rejection distinctly from cancellation/missing card. Never automatically retry a PIN; show remaining attempts only if reliably available from the token. |
| Tray availability is not guaranteed | A desktop may lack a StatusNotifierItem host; native window is modal without a browser transient parent | Use tray status/attention where supported, plus a reliable native dialog path. Avoid repeated focus stealing and do not promise global modality on Wayland. |

These are source findings and expected consequences, not results of interactive desktop testing. UI accessibility, compositor focus behavior and actual prompt counts need live checks.

## Authentication policy: no PIN caching

The user has excluded PIN caching for the qualified signing-certificate workflow. Do not implement a remember-PIN option, retain a PIN for later operations in memory, persist it to disk/keyring, or add authenticated-session retention as a convenience feature. Do not configure native middleware to override per-use consent or enable PIN reuse across requests. This is the project's explicit policy; this review does not make a general legal claim about all qualified certificates or jurisdictions.

Only nonsecret context preferences and public certificate metadata may be remembered. Validate stable card identity and certificate eligibility before use; invalidate selection on removal, replacement or ambiguity.

The tested ESS signing key reports `CKA_ALWAYS_AUTHENTICATE=true`. Preserve context-specific authentication for each operation that requires it. Do not enable `pin_cache_ignore_user_consent` as a usability shortcut. A successful Java/OpenSC signature does not establish how NSS/XMLSec handles repeated ESS signing.

A PIN necessarily exists transiently while entered and passed to native authentication. Minimize that lifetime, clear the GTK entry on dismissal, use secret-aware buffers where possible, and verify native buffer ownership before attempting cleanup. The current callback creates String/CString copies and passes an allocated buffer to NSS; it does not explicitly zero its own copies. No application cache does not establish that native libraries retain no transient copies: inspect and configure the selected OpenSC/NSS path before claiming compliance with this policy.

## Proposed user experience

1. Choose a clearly labelled PIN context once, preferably the eligible Digital Signature context for this card; preserve an explicit preference rather than hardcoding slot 1.
2. List public certificates without authentication when the token allows it. If private enumeration requires login, explain "Unlock certificate list" separately from signing.
3. Present one compact native authorization dialog for each signing request. Show the portal, operation and selected context; include PIN entry only when authentication is required. Combine approval and PIN entry where possible, instead of producing two successive dialogs.
4. Use explicit action buttons, keyboard navigation, initial PIN focus, Enter to confirm, Escape/window-close to cancel, and accessible layout/text. Clear the entry on dismissal and keep secrets out of labels, tooltips, clipboard automation and logs.
5. Native approval remains required even while a session is authenticated. Never silently sign because a PIN was entered for an earlier request. For unidentified XML, explain what cannot be identified and require review or reject the operation.
6. Tray displays Ready, Waiting for approval, Working, or Card unavailable. Do not put sensitive document details into desktop notifications.

## Authentication and usability validation

- Synthetic/software-token tests: operation-scoped authentication/logout, serialized token access and explicit approval for every signing request. No remember-PIN or session-retention option is present.
- Per-use-authentication tests: each required operation authenticates correctly through `CKU_CONTEXT_SPECIFIC`; no application PIN reuse, native consent override or convenience session retention is applied.
- User-run ESS tests: certificate listing, portal login and repeated test-environment declaration signatures, recording prompt count and authentication outcome without recording PINs or document contents.
- Cancellation tests: Escape, window close, Cancel, browser disconnect, card removal, timeout and quit while prompting and while native work is blocked. A cancelled queued operation never signs.
- Responsiveness checks: diagnostics can be changed while a prompt waits; multiple connections cannot overlap authorizations or consume another request's approval.
- Desktop checks: GTK keyboard navigation, long labels, display scaling, screen lock and tray absence on representative X11/Wayland desktops.

## PR boundaries

Keep all PRs in `damirkrstanovic/epoxy`; submit nothing to the original repository. Design each feature for later upstream review with a single clear outcome, minimal public API changes, its own tests/documentation and an explicit dependency list. A dependent PR must be reviewable as a diff against its prerequisite, rather than repeating prerequisite changes. Independent features should branch from master; stacked features should identify the prerequisite branch and intended eventual base. Avoid combining UI cleanup and native backend restructuring in one patch.

Suggested usability splits: (a) complete cancellation/keyboard handling, (b) compact contextual authorization and explicit approval, (c) stable PIN-context preference, (d) tray status. The approval gate must not depend on a PIN callback. Existing operational diagnostics and packaging retain their separate PR boundaries. PIN caching and convenience authenticated-session retention have no planned PR.

## References

- [OpenSC maintainer discussion of multiple PIN contexts, per-use consent and caching](https://github.com/OpenSC/OpenSC/discussions/3467)
- [PKCS#11 v3.1 specification](https://docs.oasis-open.org/pkcs11/pkcs11-spec/v3.1/pkcs11-spec-v3.1.html) (reference for implementation review; the web fetch of the complete document was unavailable during this review).
- Local source: `epoxy/src/ui.rs`, `epoxy/src/nss.rs`, `epoxy/src/server.rs`, `epoxy/src/main.rs`.
