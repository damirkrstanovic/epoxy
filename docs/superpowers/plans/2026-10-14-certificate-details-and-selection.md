# Certificate Details and Signing Context Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans for the previously chosen inline workflow. Implement task-by-task with one independent whole-branch review. Human review of this written plan precedes product changes.

**Goal:** Let the user inspect public card certificates and select the exact signing certificate/PIN context from the tray, with a usable GTK fallback.

**Architecture:** A separate certificate controller carries owned public metadata and bounded commands. The existing signing thread owns NSS/XMLSec and executes discovery/selection commands serially with portal operations. Selection binds module, token URI and certificate fingerprint; it is checked again before signing. No native handle crosses into GTK or tray threads.

**Tech Stack:** Existing Rust, GTK4 0.10, ksni, NSS/XMLSec, x509-parser, Tokio channels, serde; SHA-256 via a direct sha2 dependency. No new UI framework or vendor middleware.

**Spec:** `docs/superpowers/specs/2026-10-13-linux-native-epoxy-design.md`, OpenSC/PIN-context and GTK/tray sections; `2026-10-13-usability-and-pin-sessions.md`. User confirmed certificate details **and** signing-context selection on 2026-10-14. Routine UI belongs in the tray menu; optional details/GTK fallback windows must have visible dismissal.

## Branch and PR

Worktree `/home/damirk/Downloads/ESS/epoxy-certificates`, branch `codex/certificate-selection`, initially based on desktop revision `824ef84`, with prerequisite updates merged through `c2cfc73`. This is an independent feature PR stacked on desktop PR #3 while it remains open. Its diff must not repeat desktop/diagnostics implementation changes. Only the user's fork receives a PR; no merge or upstream submission.

## Global constraints

- Keep NSS/XMLSec/OpenSC on the current native execution thread. No PIN cache, convenience session retention or consent override.
- Opening the tray must never authenticate or sign. Public discovery is explicit/on demand, with no PIN prompt or automatic retries of authentication.
- Never select by slot index, list order, subject name or a hardcoded ESS slot. Existing portal IDs remain connection-local indexes only.
- Personal metadata may be shown in explicitly opened certificate menus/details. It must not enter DesktopSnapshot, tooltip, notifications, ordinary logs or masked wire diagnostics.
- A selection is session-only unless the user explicitly chooses to remember it. Persist only its nonsecret identity in an app-private file. NSS/browser databases are not rewritten.
- Keep service startup/GET_INFO lazy and fast. Selection/discovery while a signing/PIN operation is active must report Busy or wait visibly; it must not interfere with native authentication.
- Missing/changed/ambiguous/expired selected certificates fail closed; no automatic switch to a different identity.
- Preserve RSA-SHA256/XML signature serialization and card-required per-use authentication. Add validation of the actual XMLSec private-key slot before signing.
- Certificate imports/deletion, key generation, PIN changes, trust management, provider installation and browser integration are outside this feature.

## Review focus

1. Same subject or DER certificate across different modules/cards must not redirect signing to an unintended key.
2. A selection change after GET_CERTIFICATES must invalidate the earlier connection's alias rather than reinterpret it.
3. Native discovery and PIN callbacks must not deadlock GTK/tray controls or overlap token authentication.
4. Public listing must not request a PIN, change PIN retry counters, or leave a convenience authenticated session.
5. Malformed/stale preference files and card removal must retain a useful explanation without unsafe file replacement, credential logging or fallback signing.

## User experience

Tray click opens the host menu. Add **Cards and certificates** with Refresh, discovery status and one submenu per clearly named context. Each certificate shows holder, issuer, validity dates and key-usage purpose; expired/unsupported entries remain inspectable with an ineligibility reason.

An eligible certificate offers **Use for this session** and **Use and remember**. The selected entry gets a checkmark, with a separate action to return to portal selection and forget a saved preference. A certificate-free or private-only context explains its state; no invisible PIN request is triggered by menu opening.

Show a short explicit instruction after selection: reload/reselect certificates in the portal. Existing aliases are invalidated by the selection revision. A missing remembered identity is shown as unavailable and remains selected until the user chooses/clears it.

Detailed certificate information can stay in nested tray entries. GTK fallback presents the same catalog/actions; a separate details dialog opens only on request and has Close/Escape. The advanced diagnostics window remains optional, not the routine application interface.

## Types and file map

- `epoxy/src/certificates/model.rs`: public metadata, eligibility, unique matching and selection revisions. Identity types have no Debug/Display implementation.
- `epoxy/src/certificates/controller.rs`: bounded requests, loading/Busy/Unavailable state and owned snapshots. It owns no native objects.
- `epoxy/src/certificates/preferences.rs`: versioned private preference file; explicit save/forget only.
- `epoxy/src/certificates/native.rs`: NSS public catalog and selected-key resolution on the native thread.
- `epoxy/src/ui/certificates.rs`: GTK fallback list/details with visible dismissal and plain-text labels.
- `epoxy/src/desktop/certificate_menu.rs`: tray submenus/checkmarks/actions; no sensitive tooltip/status data.
- Modify `nss.rs`, `xmlsec.rs`, `server.rs`, `main.rs`, `config.rs`, `desktop/state.rs`, `desktop/tray.rs`, and `xmlsec-nss-sys/src/lib.h` for checked native access, command routing, key binding and preference-path configuration.

```rust
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ContextKey {
    pub module_name: String,
    pub module_library: Option<String>,
    pub token_uri: Vec<u8>,
}
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CertificateKey {
    pub context: ContextKey,
    pub der_sha256: [u8; 32],
}
pub enum CertificateCommand {
    Refresh,
    Select { key: CertificateKey, remember: bool, generation: u64 },
    Clear { forget_saved: bool },
}
```

Context display text and X.509 details are owned strings kept outside the service-status model. `CatalogStatus` uses fixed enum variants: NotLoaded, Loading, Ready, Busy, NoCard, NativeUnavailable. Each catalog carries a generation; each accepted selection change increments a selection revision. Reject stale action generations and duplicated matching identities.

### Task 1: Identity, metadata and bounded controller

- [x] Write model tests for two identical subjects with different keys; reordered contexts retaining the same selection; duplicate identical keys becoming ambiguous; expired/unsupported certificates visible but not selectable; stale-generation selection rejection. Use synthetic module/token/certificate values, not real identities.

```rust
#[test]
fn a_slot_reorder_does_not_change_a_selected_identity() {
    let first = synthetic_certificate("module-a", b"token-a", [1; 32]);
    let second = synthetic_certificate("module-a", b"token-b", [2; 32]);
    let mut model = CatalogModel::from_entries(vec![first.clone(), second.clone()]);
    model.select(&second.key, model.generation()).unwrap();
    model.replace_entries(vec![second.clone(), first]);
    assert_eq!(model.selected_key(), Some(&second.key));
}
```

Define `synthetic_certificate` in test utilities with literal eligible validity/usage data. Implement `CatalogModel::from_entries`, `generation`, `select`, `replace_entries`, `selected_key` and unique matching; selection errors use fixed enums.

- [x] Watch the tests fail, then implement metadata parsing from DER with SHA-256, validity and RSA/digitalSignature/nonRepudiation eligibility matching the existing engine. Add synthetic DER fixtures for Unicode subjects, expiry and unsupported usage.
- [x] Add bounded controller tests: repeated Refresh coalesces, stale/cancelled commands do not change selection, native Busy is visible, controller snapshots contain no PIN/request/XML data. Implement `CertificateController::new() -> (Self, Receiver<CertificateCommand>)`, `request_refresh`, `request_select`, `request_clear`, `snapshot` and native-thread `publish`.
- [x] Run focused tests/full suite; commit the catalog/controller deliverable.

### Task 2: Checked, public-only native catalog and command routing

- [x] Test failed NSS initialization using a temporary invalid database without accessing the user's card. Replace the unchecked null return and main initializer `expect` path with a stored native initialization Result and controlled operation errors.
- [x] Add checked NSS module/URI accessors. `PK11_GetModule` exposes `SECMODModule.commonName`/`dllName`; copy them only while the token is alive. Preserve URI bytes without lossy conversion. Check all native pointers before CStr conversion.
- [x] Implement `public_catalog(&Nss, allow_soft_tokens) -> Result<Vec<PublicContext>, CatalogError>` without prompting. Use a `PinPrompt` that always returns None where the existing certificate-listing API needs a context; record a private-only context as RequiresAuthentication rather than asking/retrying. All borrowed NSS/DER objects are dropped before publishing owned metadata. Preserve logout guard semantics.
- [x] Write a synthetic native executor test that proves catalog requests execute on the same thread as signing and cannot overlap a blocking native operation; GTK/diagnostics controllers remain responsive. Route the bounded receiver through the existing service runtime, with commands rejected on shutdown. No reader probe occurs at startup or GET_INFO.
- [x] Run tests/full suite; commit public discovery and command routing. If software-token infrastructure is unavailable, report that limitation; do not silently count a skipped native-authentication test as passed.

### Task 3: Enforce the selected module/context/certificate through signing

- [x] Store connection-local certificate records with `CertificateKey` and selection revision. Without a preference, preserve portal selection. With a preference, GET_TERMINALS/GET_CERTIFICATES expose only the matching eligible identity; missing/ambiguous matches produce compatible empty/error responses.
- [x] Revalidate a Select command against fresh public metadata before committing it. Reject changes during an active authorization/signature. Native source labels never determine a match.
- [x] Add tests that changing selection after certificate listing makes the old alias fail before authentication, that swapping context/module/fingerprint cannot redirect signing, and that a missing remembered identity never falls back to another card. Use a synthetic native backend boundary with a signing counter; failure paths must leave it at zero.
- [x] Validate the XMLSec key's actual native slot before `signer.sign`. The current `xmlSecNssAppKeyFromCertLoadSECItem` performs certificate-based global key lookup. Include the installed public NSS PKI header to expose `xmlSecNssPKIKeyDataGetPrivKey`; obtain the private-key slot via `PK11_GetSlotFromPrivateKey`, check it against the selected live token, and release the slot according to verified NSS ownership semantics. A wrong-slot key is rejected even if its certificate DER matches. Retain the existing algorithms, certificate formatting and native authentication.
- [x] Test wrong-key-slot rejection and cleanup separately from UI preferences, including equal DER in different modules. Audit native ownership against library source before adding pointer wrappers; implement RAII for acquired references. No opaque key crosses threads.
- [x] Run tests/full suite; commit exact signing identity enforcement.

### Task 4: Explicit nonsecret preferences

- [x] Add `--selection-path` and config support; default to an app-private XDG config location. The local test launcher uses a file inside its existing private fixture, never the browser NSS directory.
- [x] Test session-only choice performs no write; explicit remember reloads the exact key; malformed/oversized/version-unknown preference returns a fixed local error; symlinks/unrelated files are rejected; failed save leaves the previous selection intact.
- [x] Implement versioned JSON bounded to 8 KiB, a 0700 app directory and 0600 preference file. Use verified directory descriptors and atomic same-directory replacement, with checked existing file ownership/type. Save before committing a remembered selection; Forget removes only a verified app-owned preference. Do not log preference contents.
- [x] Run focused tests/full suite; commit preference storage.

### Task 5: Tray certificate actions and GTK fallback

- [x] Write native-menu action tests for Refresh, selecting one eligible certificate, remembering/clearing, generation mismatch and disabled selection during a PIN wait. Metadata appears only in explicitly opened certificate entries; tooltip and DesktopSnapshot stay identity-free. Escape underscores in untrusted menu labels.
- [x] Implement certificate submenus with issuer, validity and purpose, eligible actions and selected checkmark. Refresh is explicit; no placeholder disabled fake feature or hidden PIN prompt. Fixed discovery/error states explain NoCard, Busy, RequiresAuthentication and selected identity unavailable.
- [x] Add a private GTK scenario for the fallback list/details, selection/remember actions, refresh, Close/Escape and cancellation while the native service is busy. Test plain-text long Unicode labels and no implicit signing during discovery/selection.
- [x] Add certificate controls to the no-tray fallback; keep routine tray interactions menu-only. Do not add a dashboard or certificate-administration actions.
- [x] Run tests/full suite; commit the tray/fallback certificate UI.

### Task 6: Reviewable PR and user-operated ESS acceptance

- [x] Verify default, CLI-only and GTK-only workspace suites, formatting and application Clippy. Confirm startup/GET_INFO still avoids native initialization; verify request logging remains masked after discovery/selection.
- [x] Run all affected isolated GUI/bus tests; inspect long labels at 100%/200%. Extend synthetic portal flow to cover selection revisions and same-subject/different-context aliases.
- [x] Preserve the working desktop helper and build a separate certificate test helper using the existing isolated OpenSC database/preferences path. No real PIN/identity/DER/XML is committed or sent in a PR.
- [x] Independent whole-branch review, then one failing-test-first fix pass for Important/Critical findings. Publish only to the user's fork, identifying desktop PR #3 as prerequisite. Keep actual card acceptance pending user interaction rather than declaring it covered by synthetic tests.
- [ ] User acceptance: refresh the ESS card's two contexts, inspect issuer/expiry/purpose, select Digital Signature by identity, reload portal certificates, sign manually in the test portal, test reconnect/restart, remove/reinsert card and confirm no fallback/wrong-key signing. The PIN remains native and uncached.

## Self-review

Scope maps to the user's confirmed functions. Native catalog, selected key enforcement and persistence remain separate units and test cycles. Display identities are confined to an explicit certificate view; operational status remains secret-free. Existing synchronous native work limits cancellation of an opaque call, but controls retain their independent threads. Broader certificate administration, cryptographic backend replacement and package portability are not implied by this plan.

## Implementation evidence (2026-10-14)

- Default after review fixes: 63 automatic tests (55 library + 8 server). CLI-only: 56; GTK-only: 58.
- Eight explicit native/GTK/bus cases run separately, including Unicode plain-text details, session/remember/clear actions, Busy, Close/Escape and tray recovery. Synthetic long-label screenshots inspected at 100%/200%; list summaries were shortened and details remain scrollable.
- Synthetic NSS RSA key: correct/wrong XMLSec slot checks repeated 100 times; clean NSS shutdown; generated XML signature verified by xmlsec1. No real card accessed.
- Dynamic request tracing remains masked after catalog/selection, including an unlisted signing alias rejected before NSS initialization. Certificate-control operation logs include fixed operation/error enums and duration, without identity values.
- Three fresh debug processes: listener about 31.5 ms; GET_INFO handshake about 32–36 ms, with nonexistent NSS path. No native initialization or PIN access; clean SIGTERM. These are fresh-process measurements with uncontrolled OS caches, not filesystem-cold benchmarks.
- Private user test launcher is separate from the preserved desktop build and uses the existing isolated OpenSC NSS database plus an isolated preference path. Actual ESS test-portal signing/reconnect/removal acceptance remains user-operated.
- External private-only software-token authentication behavior is not covered by the internal NSS key fixture (SoftHSM unavailable).

## Final independent review

The reviewer found no Critical/Minor issues and two Important issues. Both were
fixed with RED→GREEN regressions: card replacement during native PIN entry now
checks fresh token information and slot series after authentication and before
signing; public discovery enumerates visible certificates without authentication
instead of assuming NSS’s friendly hint establishes public visibility. The entire
suite is green; the synthetic native signature still verifies and NSS shuts down
cleanly. No automatic merge or upstream submission is requested.

Review rulings and limits (the cost is the behavior not established/implemented):

- Actual ESS login/declaration/reconnect/removal acceptance — remains user-operated, no real card accessed — cost if wrong: live ESS compatibility is not established by synthetic tests.

- ESS per-use PIN/CKA_ALWAYS_AUTHENTICATE counts — preserve native policy without caching, verify on the actual card — cost if wrong: native prompt/consent behavior may differ from the software-token fixture.

- External private-only SoftHSM behavior — fixture unavailable; explicit limitation, public nonfriendly regression added — cost if wrong: private-object visibility/login/logout differences remain unverified.

- Browser Origin and PIN integration — separate approved feature, no scope expansion — cost if wrong: inherited browser boundary limitations remain.

- Independent signature approval/document purpose — separate approved feature, no scope expansion — cost if wrong: inherited native authorization behavior remains.

- Browser disconnect/timeout during blocked native PIN work — inherited lifecycle outside this feature; existing Quit checks retained — cost if wrong: abandoning a browser request may still require native cancellation/Quit.

- Portable native setup, provider installation and packaging — separate approved features — cost if wrong: installation still depends on current native library/NSS setup.

- Certificate administration, trust/revocation and additional algorithms — expressly excluded, current eligibility is usability metadata — cost if wrong: the view cannot establish trust/qualification or administer certificates.

- Unrelated prerequisite diagnostics/tray behavior — previously reviewed outside this diff; integration tests retained — cost if wrong: this review does not independently establish all prerequisite behavior.

## Approved UX follow-up (2026-10-14)

The user approved a bounded redesign after inspecting tray screenshots: certificates
appear directly by name and purpose, one Use this certificate action remembers the
choice, details remain explicit, and technical/reset controls move under Advanced.
On discovery a unique eligible identity in the dedicated Digital Signature context
is selected automatically. Existing selections remain pinned, including while absent.
Ambiguous, duplicate or ineligible candidates do not receive a default. Startup/GET_INFO
stay lazy. Explicit choice revalidation cannot apply a default before save succeeds.

RED→GREEN evidence: automatic default/no fallback, ambiguity/ineligibility, personal
name extraction without embedded CN identifier, single-level remembered menu action,
stale/Busy controls, and failed explicit save without implicit selection. GTK checks
cover the single remembered action, collapsed Advanced and Details Close/Escape.


### Tray controls follow-up — 2026-10-14

User reported leaf actions returning focus to the launch terminal, no checked
automatic-mode indicator, and an inconvenient top-level order. Their host is
DankMaterialShell (installed dms-shell 1.6.2-1). Inspection of current official DMS
SystemTrayBar.qml shows leaf activation starts pendingActionCloseTimer (80 ms),
which calls trayMenuState.close(). This explains the popup dismissal; it is a host
policy, not an Epoxy window presentation request. Installed-version source was not
available locally, and no user's desktop configuration was changed. No standard
tray keep-open property was identified; host-specific focus/reopening hacks were
not added. Source: https://github.com/AvengeMedia/DankMaterialShell/blob/master/quickshell/Modules/DBar/Widgets/SystemTrayBar.qml

Root order is now status, Cards and certificates, Enable troubleshooting logs,
Disable troubleshooting logs, Advanced. Advanced contains diagnostics, certificate
options/context details and Quit Epoxy. Choose automatically is a standard ksni
CheckmarkItem backed by explicit automatic-mode state, independent of whether a
manual preference is persisted. Reset returns to automatic mode; clicking already
checked is a no-op. Use this certificate clears the mode tick. No listener stop/start
feature was added: Quit still exits both tray and service.

Meaningful red/green regressions cover mode state and root menu order/actions.
Fresh validation: default68/CLI61/GTK63 automatic tests pass, application Clippy
passes all three builds, fmt/diff checks pass, all seven private GTK/bus checks and
three readiness/shutdown smoke scenarios pass. Local certificate binary rebuilt.
No real card/PIN/signing or host tray interaction was run. DMS popup closure remains
a host limitation; actual standard checkmark rendering is for user desktop QA.

Independent bounded-diff review found no Critical, Important or Minor findings;
confirmed ksni serializes CheckmarkItem as the standard checkmark toggle type.

### Automatic desktop discovery — 2026-10-14

User expects an already inserted token to appear without manual Refresh. Root cause:
startup constructs an unscanned controller and never queues a catalog request; only
Refresh and portal GET_TERMINALS discover public certificates. Desktop mode now
queues one existing Refresh after listener readiness, executing on the same native
thread via the serialized dispatcher. The catalog future yields before processing.
GTK remains independent; no new native threads, authentication/PIN requests or
convenience sessions are introduced. CLI remains lazy. Manual/remembered choices
retain priority because the existing publish rules are unchanged. Later card changes
still use Refresh/portal discovery; continuous hotplug watching is not added.

The real-process private desktop smoke regression was red on the previous binary
(no unrequested scan) and green after the change (exactly one public Refresh after
listener readiness). It uses an absent synthetic NSS DB and verifies backend
failure leaves GET_INFO, diagnostics and clean shutdown working. Readiness measured
32–73 ms and handshake 34–79 ms in these three synthetic runs, not actual ESS cold
initialization. Synchronous native backend calls can delay protocol processing while
they execute; listener/GTK readiness does not wait for discovery.

Fresh default68/CLI61/GTK63 automatic suites, application Clippy all configurations,
fmt/diff, seven private GTK/bus checks and three smoke scenarios pass. Local binary
rebuilt. No user's card, PIN, signing request or session bus was accessed.

Independent bounded startup review found no Critical, Important or Minor issues;
checked native serialization, authentication-free discovery and shutdown ordering.
