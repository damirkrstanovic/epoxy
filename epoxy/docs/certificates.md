# Cards, certificates and signing contexts

Open **Cards and certificates** from the tray, then choose **Refresh**. Opening
this menu does not access the card. Refresh reads public certificate metadata
without authentication, regardless of NSS’s friendly configuration hint. A context
that hides its certificates behind login is
shown as requiring authentication; it can still be chosen through the portal's
ordinary certificate flow, using Epoxy's native PIN dialog.

Each context shows its token/PIN-context label and module. Certificate submenus
show holder, issuer, validity dates and key usage. Expired, not-yet-valid and
unsupported certificates remain visible but cannot be selected. Eligibility
retains the existing engine's RSA, digitalSignature and contentCommitment usage
requirements; it does not establish certificate trust or legal qualification.

Choose **Use for this session**, or **Use and remember** to save the nonsecret
certificate/context identity. A checkmark identifies the selected certificate.
Reload/reselect certificates in the portal after any selection change. Existing
portal aliases are invalidated, and subsequent listings expose the chosen
certificate. **Use portal selection and forget saved choice** removes the
preference and returns certificate selection to the portal.

A session-only choice makes no preference write. If a previous choice was saved,
it is used again on the next restart unless explicitly forgotten. A remembered
choice is checked when the card is discovered/used; missing, duplicated or changed
identities never cause automatic fallback to another card. Refresh after inserting,
removing or replacing a card. While card work or PIN entry is active, selection
controls report Busy. The native service executes discovery and signing serially.

GTK fallback controls include a **Cards and certificates…** button. Its read-only
catalog, Details, selection and remember actions use the same controller as the
tray. Certificate windows have Close and Escape; closing them does not quit the
helper. The details view opens only on explicit request. Long labels are plain
text and bounded; no certificate data is used as markup or menu access keys.

## Preference file

Default: `$XDG_CONFIG_HOME/epoxy/certificate-selection.json` (normally
`~/.config/epoxy/certificate-selection.json`). Override with `--selection-path`
or `selection_path` in TOML, using an absolute path whose parent directory belongs
to this user and has mode 0700. Missing directories are created only for explicit
remember actions. File mode is 0600. The versioned JSON contains only module
identity, token URI and certificate DER SHA-256 fingerprint. Token URIs may contain
personal labels/serials; keep the file private even though it contains no credential.

Reads are bounded to 8 KiB. Directory components and files cannot be symlinks.
Writes use held directory descriptors, an exclusive temporary file and atomic
replacement; invalid, unrelated, unsafe or changed files are rejected. Malformed
preferences prevent startup rather than silently discarding the requested identity.
Inspect the configured path and correct/move that file before retrying. NSS/browser
databases are not modified by certificate selection.

PINs are neither saved nor cached. There is no convenience retention of authenticated
sessions, no consent override, and no automatic PIN retry. Metadata appears in the
explicit certificate view, not service status, tray tooltip or operational/request
logs. Troubleshooting traces retain mandatory sensitive-data masking.

## Binding and native ownership

Identity is module name/library, exact token URI bytes and certificate DER SHA-256;
slot number, list order and subject name are not selection keys. Each connection's
aliases carry a selection revision and are never reused after relisting. Before
signing, Epoxy resolves the live context, rechecks certificate validity/presence and
checks XMLSec's actual private-key slot. Token identity/slot incarnation is checked
after authentication and again immediately before signing; cached URI fields must
match a fresh C_GetTokenInfo result. A replaced card is rejected even with identical DER. A wrong-slot global key lookup is rejected.

[NSS's slot accessor](https://github.com/mozilla/nss/blob/master/lib/pk11wrap/pk11akey.c)
returns a referenced slot, and [XMLSec's private-key accessor](https://github.com/lsh123/xmlsec/blob/master/src/nss/pkikeys.c)
returns a copied key. Both references have independent RAII cleanup.

## Verification and scope

`cargo test --workspace` covers identities, stale actions, alias binding, Busy state,
preference safety and native failure handling. `scripts/test-desktop.sh` runs private
GTK/bus checks, including synthetic Unicode certificate details/selection. It does
not contact real cards or use the user's desktop bus.

`scripts/test-certificates-native.sh` creates a private synthetic NSS database and
RSA key, exercises correct/wrong XMLSec slot checks and clean NSS shutdown, signs a
synthetic document and verifies it with `xmlsec1`. Prerequisites: NSS tools
(certutil/pk12util), Python cryptography and xmlsec1. Temporary private keys are
removed. Committed DER fixtures contain only synthetic public certificates.

Actual ESS acceptance remains user-operated: refresh the two OpenSC contexts,
select the Digital Signature identity, reload portal certificates, enter the PIN
natively and sign in the test portal. Also test reconnect, remembered choice on
restart and card removal/reinsertion. External private-only software-token discovery
is not covered by the internal NSS key fixture; no SoftHSM module was available.

This feature retains the existing NSS/XMLSec backend, XML format and native PIN
policy. It does not provide imports/deletion, PIN changes, trust administration,
module installation, browser Origin policy or the separate signing-approval feature.
