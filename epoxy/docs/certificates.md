# Cards, certificates and signing contexts

Open **Cards and certificates** from the tray, then choose **Refresh**. Opening
this menu does not access the card. Refresh reads public certificate metadata
without authentication, regardless of NSS’s friendly configuration hint. A context
that hides its certificates behind login is
shown as requiring authentication; it can still be chosen through the portal's
ordinary certificate flow, using Epoxy's native PIN dialog.

Certificates appear directly by holder and purpose, with the selected certificate
marked. Personal names use given-name/surname attributes when present, avoiding
certificate identifiers embedded in the common name. Open a certificate and choose
**Use this certificate** to make it your preferred choice, including future launches.
**Certificate details…** shows issuer, expiry and purpose. Expired/unsupported
certificates remain inspectable but cannot be selected. Module and PIN-context
information, plus **Choose automatically**, are under **Advanced**.

On public card discovery (Refresh or the portal's reader request), Epoxy automatically
selects exactly one eligible certificate in a dedicated Digital Signature context
when no choice is already active. It recognizes OpenSC's `Digital Signature PIN`
label/suffix as a preference hint, not as proof of key identity or trust. Multiple
candidates, duplicates or ineligible certificates require a choice. Other signing
certificates remain selectable. Eligibility retains the engine's RSA, digitalSignature
and contentCommitment requirements; it does not establish trust/legal qualification.

An existing explicit/remembered choice takes priority. If the selected identity
disappears or becomes invalid/ambiguous, Epoxy never substitutes a different card.
Advanced → **Choose automatically** explicitly forgets the preference and returns
to automatic selection. Automatic choices make no preference writes. Remembered
selection stores only nonsecret identity; no PIN is saved or cached.

Reload the portal after changing certificates; previous aliases are invalidated.
Startup and GET_INFO remain lazy: automatic selection runs on discovery, not during
startup. While card work or PIN entry is active, controls report Busy.

GTK fallback provides the same compact certificate list, **Use this certificate**,
**Certificate details…**, Refresh and collapsed Advanced controls. Close/Escape
dismiss certificate windows without quitting the helper. Labels are plain text and
bounded; no certificate data is used as markup or menu access keys.

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
