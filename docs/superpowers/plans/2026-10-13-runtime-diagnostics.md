# Runtime Diagnostics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Native execution is recommended; confirm the plan and execution method before product changes.

**Goal:** Add useful, masked runtime diagnostics that can be enabled, disabled and expired without restarting Epoxy, including while native PIN entry blocks signing.

**Architecture:** A shared diagnostics controller owns a reloadable tracing filter and separate masked-payload policy. A private Unix socket, served by a dedicated control thread, exposes typed commands to the CLI and later the tray. The browser protocol remains unchanged; sanitization happens before any request/response tracing.

**Tech Stack:** Existing Rust/Tokio/tracing/serde/clap stack; standard-library Unix sockets; direct libc dependency for effective UID verification. No new GTK/tray or signing dependencies in this PR. Socket2, already a transitive dependency, supplies bounded nonblocking connect.

**Spec:** `docs/superpowers/specs/2026-10-13-linux-native-epoxy-design.md`, runtime logging section, with the no-caching policy in `docs/superpowers/specs/2026-10-13-usability-and-pin-sessions.md`.

## Global Constraints

- Payload tracing is off by default, even when operational verbosity is debug/trace.
- Detailed requests/responses are masked before formatting. No unmasked PIN/credential or unrestricted document-dump mode.
- Default diagnostic lifetime: 300 seconds; accepted range: 1 through 3600 seconds.
- Accept error, warn, info, debug and trace runtime levels; expiry restores the pre-diagnostic filter and disables payload tracing.
- The Unix control directory is mode 700, socket mode 600, peer UID must match the effective application UID, and messages are bounded to 8 KiB.
- Controls/expiry stay responsive during a blocked native PIN callback; browser clients cannot change diagnostics.
- No PIN caching, remembered PINs or convenience authenticated-session retention.
- Keep GTK enabled by default and preserve existing CLI/systemd feature combinations.
- Work only in the user's fork. No upstream PR, merge, live tax submission or use of a real PIN for tests.

## Review Focus

1. Unknown JSON fields/keys may contain credentials: retain only schema-approved field names and safe values; mask unknown content.
2. Malformed/oversized/deep JSON may expose secrets through parsing errors: emit only bounded metadata, never source/error snippets.
3. A queued enable/disable/expiry must not revive old diagnostic state: test replacement and expiry under concurrent reads.
4. A stalled or hostile local client must not block expiry: use read deadlines and a bounded command buffer on the independent controller thread.
5. An existing path may be a symlink, wrong-owner directory or unrelated socket: refuse it without deleting or following it.

---

## PR boundary and workspace

Implementation branch: `codex/runtime-diagnostics`, based on fork `master`, isolated under `/home/damirk/Downloads/ESS/epoxy-diagnostics`. The design PR remains documentation-only. This feature has no prerequisite implementation PR. Include this plan and the relevant design documents for review; future UI/tray features consume its typed controller rather than duplicating state.

Do not change NSS authentication, XML signing, tray behavior, browser companion, context selection or packaging in this PR. Existing sensitive operational log statements are in scope because they would otherwise defeat masking.

### Task 1: Bounded wire sanitization

**Files:** Create `epoxy/src/diagnostics/mod.rs`, `epoxy/src/diagnostics/mask.rs`; modify `epoxy/src/main.rs` to declare the module. Tests live in `mask.rs`.

**Interfaces:** Produce `mask::masked_wire(text: &str) -> serde_json::Value`. Input is untrusted wire JSON. Output is bounded and safe to format. Use a 64 KiB input ceiling, 16 KiB output ceiling, maximum nesting 16 and maximum 128 visited container entries; truncation emits metadata instead of partial raw input. Do not disable serde_json's recursion limit.

- [x] Write tests first. Removing masking must make the first assertion fail; allowing malformed fallback text must make the second fail:

```rust
#[test]
fn masks_credentials_documents_and_unknown_values() {
    let trace = masked_wire(r#"{"operation":"GET_SIGNED_XML","input":{"pin":"synthetic-secret","xml":"PHNlY3JldC8+","certificate":{"alias":"0","name":"Synthetic Person"},"extra":{"password":"another-secret"}}}"#).to_string();
    for secret in ["synthetic-secret", "PHNlY3JldC8+", "Synthetic Person", "another-secret"] {
        assert!(!trace.contains(secret));
    }
    assert!(trace.contains("GET_SIGNED_XML"));
}
#[test]
fn malformed_input_never_becomes_fallback_text() {
    let trace = masked_wire(r#"{"pin":"synthetic-secret""#).to_string();
    assert!(!trace.contains("synthetic-secret"));
    assert!(trace.contains("invalid_json"));
}
```

Also test oversized input, deep nesting, huge arrays, escaped/case-varied PIN keys, sensitive arbitrary keys, encoded credentials, response certificate/token names, unknown operation strings, Unicode and nonsensitive numeric IDs/status preservation. Use synthetic data only.

- [x] Run `cargo test -p epoxy --no-default-features diagnostics::mask`; confirm failures are missing sanitization, not unrelated compile errors.
- [x] Implement with a schema-aware allowlist, not a blacklist. Retain approved operation enum strings, IDs/status, known container structure and public application metadata only. Mask PIN/session/authentication/certificate identity/document fields and unknown names/values. Document blobs expose length metadata only. Safe error metadata is constructed without formatting the original input or parser error.

Core data-flow shape:

```rust
pub fn masked_wire(text: &str) -> serde_json::Value {
    if text.len() > MAX_INPUT_BYTES {
        return serde_json::json!({"trace_omitted": "input_too_large", "bytes": text.len()});
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return serde_json::json!({"trace_omitted": "invalid_json", "bytes": text.len()});
    };
    sanitize_bounded(value)
}
```

`sanitize_bounded` owns the traversal budget and output bound; a budget failure returns a fresh omission object. Unknown keys are represented by a fixed placeholder and count, never copied verbatim. No unredacted intermediate value crosses a tracing boundary.

- [x] Rerun tests and commit `feat: add bounded masked wire diagnostics`.

### Task 2: Reloadable diagnostics policy

**Files:** Create `epoxy/src/diagnostics/state.rs`; update module and `main.rs::init_tracing`.

**Interfaces:** Produce cloneable `Diagnostics`, `Level`, `DiagnosticsStatus`, `Diagnostics::enable(level, requests, ttl)`, `disable()`, `status()`, `expire(now)` and `wire(direction, text, correlation_id)`. Only `requests = masked/off` exists. `init_tracing` returns the controller using `tracing_subscriber::reload::Layer<EnvFilter, _>`; preserve the initial full filter for disable/expiry. State contains no PIN/card data.

- [x] Write tests before implementation. A disabled controller must not generate payload events; enable must change both level and payload policy; disable/expiry must restore the baseline. Use explicit monotonic Instants for policy tests, not long sleeps.

```rust
#[test]
fn expiry_restores_baseline_and_payloads_off() {
    let start = std::time::Instant::now();
    let mut policy = Policy::new("warn".into());
    policy.enable_at(Level::Debug, true, 300, start).unwrap();
    assert!(policy.status_at(start).requests_masked);
    policy.expire(start + std::time::Duration::from_secs(300));
    assert!(!policy.status_at(start + std::time::Duration::from_secs(300)).requests_masked);
    assert_eq!(policy.active_filter(), "warn");
}
```

`Policy` is a private typed state machine used by the actual controller, with `enable_at`, `status_at`, `expire` and `active_filter`; public methods supply `Instant::now()`. Tests additionally reject TTL 0/3601, exercise repeated enable with replacement deadline, disable-after-enable, expiration after replacement, and concurrent controller reads.

- [x] Run policy tests and verify red; implement the mutex-protected state and filter reload with atomic state transitions. Propagate reload failures without partially enabling payloads. Bound lock-held work; never hold the controller lock while signing or awaiting PIN input.
- [x] Test actual tracing output using a local subscriber and in-memory writer: enabling at runtime emits masked events, disabling suppresses future payloads, default debug logging still omits payloads, invalid transitions do not expose sensitive strings. Do not call the global subscriber installer in unit tests.
- [x] Rerun tests and commit `feat: reload diagnostics policy at runtime`.

### Task 3: Private control channel and CLI

**Files:** Create `epoxy/src/diagnostics/control.rs`, `epoxy/src/diagnostics/cli.rs`; modify `config.rs`, `main.rs`, `epoxy/Cargo.toml` and lockfile only as required.

**Interfaces:** Typed serde/clap commands are `Status`, `Enable { level, requests, ttl }` and `Disable`. Expose `ControlServer::start(diagnostics, runtime_dir) -> Result<ControlServer>`, with drop cancelling/joining the worker and cleaning up only its own socket. CLI commands return before NSS, GTK or browser-listener initialization:

```text
epoxy diagnostics status
epoxy diagnostics enable --level debug --requests masked --ttl 300
epoxy diagnostics disable
```

- [x] Write CLI parser tests for these forms, default TTL, invalid level/TTL, unsupported raw/unmasked mode and unchanged ordinary server arguments. Add socket tests using unique private directories under the test temp root.
- [x] Watch tests fail, then implement typed subcommands through the existing clap configuration path. Preserve TOML/default-option behavior for normal service launch. Controller startup failure must be a clear startup error rather than pretending dynamic control is available.
- [x] Before implementing IPC, add tests asserting directory/socket modes, refusal of symlink and permissive/wrong-owner directories, same-user peer acceptance, wrong-user rejection in the credential validator, refusal to delete an existing endpoint, malformed/oversized commands and read timeout recovery. Peer validation must use socket credentials, not command-supplied UID.
- [x] Implement a dedicated std thread, not a task on the blocked current-thread Tokio runtime. Set accepted stream read/write deadlines (500 ms), cap input at 8192 bytes and process one newline-delimited command per connection. The loop polls nonblocking accept and expiry at most every 50 ms; a stalled client may delay expiry only by the bounded read deadline. Commands containing unsupported fields are rejected without printing their input.
- [x] Use the owned private `XDG_RUNTIME_DIR/epoxy` directory. Require a configured valid runtime root; no shared `/tmp` fallback. Verify effective owner, permissions and nonsymlink paths before binding; bind inside the verified private directory, chmod 600 and verify socket ownership. Refuse an existing endpoint; do not guess whether an unrelated stale socket is safe to remove. Record created endpoint identity and unlink on clean shutdown only if it still matches. Document stale-endpoint recovery rather than removing paths automatically.
- [x] Test successful control round trips, expiry while the server-side request handler is deliberately blocked, stalled-client recovery and cleanup on normal drop. CLI timeouts/errors remain bounded and contain no received raw command.
- [x] Rerun tests and commit `feat: control diagnostics over a private local socket`.

### Task 4: Safe protocol and operational logging

**Files:** Modify `epoxy/src/server.rs`, `main.rs`, `nss.rs`, `ui.rs`, and add `epoxy/tests/runtime_diagnostics.rs` if an executable integration test is useful.

**Interfaces:** Pass a controller clone into the server. Refactor request/response conversion into controller-aware helpers so sanitation happens before logging and before request serde discards PIN fields. Add request-local numeric correlation IDs generated by an AtomicU64; never derive IDs from `sbSession`. Expose typed `Request::operation()` and `Response::operation()` helpers for known operation labels. Response status/duration helpers must not serialize identity data into operational fields.

- [x] Write regression tests for normal/debug/trace output, valid and malformed browser requests, Unicode certificate names, signed XML responses and request parsing failures. Example sensitive payload is synthetic, not from local card logs:

```rust
#[test]
fn parsing_error_does_not_echo_invalid_operation() {
    let input = r#"{"operation":"synthetic-secret","input":{"pin":"synthetic-secret"}}"#;
    let error = parse_request(input).unwrap_err();
    assert!(!format!("{error:#}").contains("synthetic-secret"));
}
```

`parse_request(&str) -> anyhow::Result<Request>` uses a fixed safe error plus bounded numeric location metadata, not serde's potentially input-containing enum error. Test that wire logging, parsing and operational logs all respect this boundary.

- [x] Verify red against the current raw request/response logs and identity-bearing info/warn statements.
- [x] Remove `debug!("request: {}", text)` and response equivalent. Route payload traces through `Diagnostics::wire`; add structured operation/result/elapsed_ms/correlation_id records for handshake and operations. Transport failures use safe error classifications, not arbitrary peer-controlled error strings.
- [x] Remove personal token/certificate labels and server document paths from default logs in `server.rs`/`nss.rs`; retain those details only in native authorization UI. Avoid logging the full config at trace level. Format channel-send errors as fixed categories instead of formatting a rejected PIN request containing certificate/reason details.
- [x] Test actual WebSocket GET_INFO/GET_PROVIDERS flow without forcing lazy NSS initialization, with synthetic session/PIN fields. Verify protocol bytes/statuses are unchanged, masked diagnostics can toggle across requests on the same connection, and no real native signing occurs.
- [x] Rerun the full test suite and commit `fix: keep protocol diagnostics masked at every log level`.

### Task 5: Verification and focused PR

**Files:** Create `epoxy/docs/diagnostics.md`; update README with a short link and commands. Keep build/packaging changes out of scope.

- [x] Document the meaning of masked wire traces, TTL, control socket, runtime directory, CLI errors, stale socket recovery, journald/stderr capture and native-library/core-dump limitations. No file sink is introduced: the app writes stderr and its own events are bounded; external journal retention remains the host's responsibility.
- [x] Run `cargo fmt --all -- --check`, `cargo test --workspace`, `cargo test -p epoxy --no-default-features`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo clippy -p epoxy --no-default-features --all-targets -- -D warnings`. Record pre-existing failures separately and do not hide them by narrowing tests.
- [x] Smoke-test a foreground CLI instance on an ephemeral loopback port with a private synthetic runtime directory. Enable masked traces, issue synthetic protocol messages, disable, enable with TTL 1 and verify expiry. Test IPC responsiveness while a test-only blocked callback is active; do not add blocking test controls to production commands.
- [x] Inspect `git diff --check` and the complete diff for scope, secrets and accidental changes. Review masking adversarially: all formatted fields must be allowlisted or fixed metadata, with no raw fallback paths.
- [x] Create a PR in `damirkrstanovic/epoxy` against master. Describe before/after behavior, default-off tracing, dynamic commands, independent expiry, validation and limitations. State that GTK/tray integration and ESS signing validation remain separate features. Do not submit anything to upstream.

## Completion boundary

This PR is complete only when masked tracing, dynamic controls, expiry and safe operational logging work together and required checks pass (or a specific unavoidable environment limitation is transparently reported). It does not establish qualified-certificate compliance or live ESS compatibility. Subsequent feature PRs receive their own plans and focused validation.

## Execution record

Implemented on `codex/runtime-diagnostics` from fork master. Full workspace and CLI-only suites each pass 28 tests, and the foreground app passes a synthetic CLI/WebSocket smoke test without initializing NSS or requesting a PIN. Formatting and application-only Clippy (`--no-deps`, default and CLI builds) pass. The two specified strict Clippy commands fail on 17 warnings in unchanged generated NSS bindings: `missing_safety_doc`, `useless_transmute` and `ptr_offset_with_cast`; the binding crate was left unchanged.

The GTK-only UI example now declares `required-features = ["ui-gtk"]`, so complete CLI test builds no longer try to import an unavailable UI module. Control commands use empty struct variants instead of unit variants because serde's unit variants ignored extra fields despite `deny_unknown_fields`; regression tests enforce rejection. Diagnostics currently use stderr/journal only, with host-managed retention. No file sink, PIN caching or session retention was added.

Independent review identified four Important issues: inactivity timeouts allowed slow trickles, client connect could block on a full backlog, intermediate links were followed, and known field names were preserved outside their schema positions. Five regression tests failed before fixes and pass afterward, including directory-substitution cleanup. The fix uses absolute connection deadlines, Socket2 bounded connect, no-follow openat/mkdirat traversal with descriptor-anchored procfs paths, and operation/shape-aware masking. No second review or additional feature scope was added.

## Decisions and review limits

- Ruling: No new file sink in this PR; stderr/journal only — optional file output is not required by spec — host owns retention.
- Ruling: Reviewed/approved plan explicitly specifies this sibling worktree path — no further worktree permission required.
- Task 3: Ruling: serde unit command variants ignored unknown fields despite deny_unknown_fields — use empty struct variants and enforce rejection tests — costs a local typed command syntax change only.
- Task 5: Ruling: gate the pre-existing GTK-only ui example with required-features — CLI full tests otherwise fail on unresolved epoxy::ui — no runtime behavior changes.
- Task 5: Ruling: keep generated binding lint failures outside this diagnostics PR — unchanged xmlsec-nss-sys source generates 17 strict Clippy errors (missing_safety_doc/useless_transmute/ptr_offset_with_cast) — strict workspace lint remains blocked, application lint checked separately.
- Final: Ruling: real card/NSS/qualified-certificate correctness remains backend work — synthetic diagnostics tests cannot establish it — cost if wrong: no ESS compatibility claim can be made.
- Final: Ruling: Origin/signing authorization/browser limits stay in separate approved feature PR — diagnostic controls remain off the browser socket — cost if wrong: existing browser boundary limitations persist until that PR.
- Final: Ruling: external native stderr/core dumps remain outside masking — documented explicitly — cost if wrong: external tooling may disclose data despite safe application events.
- Final: Ruling: generated binding lint defects stay outside this feature — no generated-code lint suppression — cost if wrong: strict workspace lint stays red.
- Final: Ruling: no file sink; host retention applies — optional sink is not required — cost if wrong: no built-in file rotation.
- Final: Ruling: use Linux procfs FD paths and already-transitive Socket2 for anchoring/bounded connect — avoids user-path races and handwritten sockaddr code — cost if wrong: installations without mounted procfs cannot run controls, documented.

Local testing follow-up: a missing control subdirectory produced a misleading nonsymlink-component error. Preserve OS error causes during no-follow traversal and distinguish ENOENT for the client control directory with "start Epoxy first". The new missing-endpoint regression failed before the change and passes afterward; default and CLI-only suites now each have 29 tests. Symlink/ownership protections are unchanged.
