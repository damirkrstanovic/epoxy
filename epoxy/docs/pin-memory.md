# PIN memory lifetime

PIN ownership is represented by `SecretPin`, a non-cloneable, non-serializable
wrapper around `zeroize::Zeroizing<String>` with redacted Debug output. Once
Epoxy owns a PIN value, dropping it overwrites the full current String allocation
(including spare capacity) before release. `zeroize` uses writes that cannot be
removed as dead stores by the optimizer.

## Covered paths

- A deserialized page PIN is wrapped before validation. Oversized/NUL-containing
  values are wiped on rejection. A request that never needs its PIN wipes it on
  drop. A supplied PIN is still used once; no cross-request caching is added.
- `PinPrompt` and GTK reply channels transfer `SecretPin`, rather than returning
  an ordinary String. Unused/late replies, a closed receiver, request cancellation
  and shutdown races release the secret owner. A cancelled prompt clears any
  queued PIN even if the prompt object remains alive.
- Native GTK entry is copied directly from a borrowed GTK text pointer into one
  zeroizing owner. This avoids gtk-rs `Editable::text()`'s intermediate GString
  allocation. The old PasswordEntryBuffer is replaced before replying, including
  cancellation, rather than merely setting its text to empty. GTK's allocator
  still owns that old buffer. This requires GTK 4.4 or newer.
- NSS gets a directly allocated, NUL-terminated `PORT_Alloc` buffer, without an
  intermediate CString. Epoxy's secret is wiped immediately after the copy;
  invalid native values return null without attempting an empty-PIN login.
  Ownership of the returned allocation belongs to NSS. The verified normal
  authentication path in NSS 3.131 clears the password before freeing it.
  Epoxy never retains, wipes or frees that pointer after handing it to NSS.
- Successful diagnostics JSON parses are owned by a cleanup guard that wipes
  parsed strings and object keys on every exit, including budget/shape rejection.
  Sanitization borrows the input and copies only allow-listed output. This avoids
  discarding an ordinary unprotected PIN String just because its log was masked.
- Incoming WebSocket text is scrubbed after parsing when `Bytes::try_into_mut`
  can establish exclusive ownership. Shared/immutable buffers are not modified.

## Boundaries: not a whole-process erasure guarantee

The guarantee applies to the current application-owned PIN allocations described
above. It does **not** mean no copy can remain anywhere in memory:

- Browser input and its WebSocket send buffers belong to the browser.
- Tungstenite can share message storage with its internal read/fragment buffers;
  those shared allocations cannot be safely overwritten through the returned
  immutable Bytes. The cleanup is best-effort for transport storage.
- Serde JSON scratch space, partial-deserialization error paths, unknown-field
  buffers and earlier allocations are controlled by the parser, not SecretPin.
- GTK input methods, event processing and secure-allocation fallback paths are
  outside Epoxy's allocator. Releasing the old widget buffer reduces retention;
  it is not proof that all GTK/driver copies have been wiped, especially when
  locking secure memory is unavailable. If GTK does not provide the expected
  GtkText delegate, Epoxy clears the visible text and cancels submission; it
  cannot apply buffer replacement on that fallback path. Native terminal input has similar
  external buffering limits.
- NSS/PKCS#11 modules own their internal copies and may differ across versions.
  The normal NSS authentication cleanup was inspected; third-party modules have
  not been audited or modified. Protected-authentication-path sentinel strings
  are not handled like ordinary PIN passwords by NSS.
- Registers, allocator history before ownership, kernel/network buffers, swap,
  abrupt process termination and privileged inspection are not covered. Drop
  does not run after SIGKILL or process abort.

These limits are why process dump/debug-attach protection is enabled separately.
See [process protection](process-protection.md). Neither feature claims protection
against a compromised same-user launcher or privileged attacker.

## Verification

`tests/pin_memory.rs` uses a test-only allocator to inspect synthetic allocations
immediately **before** the system allocator frees them. It never reads freed
memory. It covers used/unused/rejected PINs, queued cancellation, shutdown during
native reply, and abandoned/closed GTK reply channels. The first three cases
failed against ordinary String storage and passed after the ownership change.
The GTK lifecycle fixture weakly references the original entry buffer and checks
it is released for submit, Escape, close, Cancel and shutdown; that check also
failed before the buffer replacement change. Native handoff tests check exact
termination and invalid-input rejection without opening a real token.

Sources inspected:
- [zeroize implementation](https://docs.rs/zeroize/1.9.0/zeroize/)
- [GTK PasswordEntry](https://docs.gtk.org/gtk4/class.PasswordEntry.html)
- [GTK 4.24.1 buffer implementation](https://github.com/GNOME/gtk/blob/4.24.1/gtk/gtkpasswordentrybuffer.c)
- [NSS 3.131 password handling](https://github.com/nss-dev/nss/blob/NSS_3_131_RTM/lib/pk11wrap/pk11auth.c)
