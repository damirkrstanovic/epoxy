# Request PIN handling

A nonempty `pin` in GET_CERTIFICATES or GET_SIGNED_XML is used for that request
when card authentication is needed. The native/TTY prompt is used only when the
field is absent or empty. Whitespace is preserved as part of the PIN, not trimmed. Embedded NUL bytes are
rejected before authentication rather than becoming an empty native C string.

This policy replaces the earlier native-only policy: supplying a PIN to the page
lets that page handle it. Epoxy cannot remove the portal's PIN field. To keep the
PIN out of page code, leave its field empty and use the native prompt. Portal
acceptance of empty input must be checked by the user.

Each request creates a one-use prompt. It cannot reuse credentials across requests
or fall back to another prompt after a supplied PIN is consumed/rejected. NSS's
existing rejection of authentication retry callbacks remains in force. A new
request can initiate another attempt. There is no PIN cache, persistence,
convenience authentication retention or new automatic retry. Existing token,
certificate and actual private-key-slot validation remains unchanged.

The wrapper's Debug output is always redacted. Operational logs exclude PINs,
and optional wire traces still mask them before decoding. No guarantee of total
process-memory erasure is made; the browser, JSON decoding and NSS handle copies
during a live request.

Tests cover both request schemas, redacted Debug, supplied/empty credential routing,
one-use behavior and absence of reuse across requests. Native/portal acceptance
remains user-operated; synthetic tests do not use the user's card or PIN.

Application-owned PIN buffers are zeroized on release, including cancellation and
validation failure. See [PIN memory lifetime](pin-memory.md) for coverage and
external-library limitations; no PIN caching is added.
