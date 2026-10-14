# Browser Boundary Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan inline, task-by-task, followed by one fresh-context branch review.

**Goal:** Restrict the signing WebSocket to the approved portal origins and loopback, with bounded network resource use, preserving the tested PIN/signing flow.

**Architecture:** A small browser transport policy validates Origin during the HTTP upgrade, configures transport limits/deadlines and supplies shared loopback validation. The serialized server adds deadlines only around network waits. Existing native authentication, certificate identity, XML signing and GTK/tray remain unchanged.

**Tech Stack:** Existing Rust/Tokio/tokio-tungstenite0.28, Clap/serde, NSS/XMLSec; no new runtime dependency.

**Spec:** ../specs/2026-10-13-linux-native-epoxy-design.md (Browser boundary and PIN-page integration). User approved website restrictions as next independent PR with “go”. This executes inline in an isolated worktree.

## Global Constraints

- Exact approved origins: `https://eporezi.purs.gov.rs`, `https://test.purs.gov.rs`.
- JSON host is untrusted display metadata; never authorizes a connection.
- Listen only on loopback; developer/test access without Origin requires explicit opt-in.
- PIN caching and convenience authenticated-session retention are excluded. Preserve request-scoped page PIN/native fallback and exact certificate/key-slot checks.
- No user card/PIN/tax data, desktop changes, deployment/merge or upstream submission during implementation. PR targets the user fork and remains draft.

## Review Focus

- Duplicate/null/lookalike/malformed Origin; developer exception must admit only absent headers, never wrong present values.
- IPv4/IPv6/mapped loopback versus wildcard/nonlocal and inherited systemd descriptors; validate the actual addresses.
- Slow incomplete upgrade, missing first GET_INFO, oversized frames/aggregate fragments, stalled writer; no secret payload/error/header logging.
- Long native PIN wait must not be wrapped in a network request deadline. Control frames and normal Close must remain usable.
- Test-mode fixtures must exercise the same production upgrade guard; allowed GET_INFO remains lazy and denied clients get no protocol event/native request.

## Task1: Upgrade policy, loopback and configuration

**Files:** Create epoxy/src/browser.rs (+ tests submodule); modify lib.rs, config.rs, main.rs; test real temporary TCP sockets.

- [ ] Write red tests for exact/duplicate/null/lookalike Origins and absent-only developer exception; config precedence false override and nonzero connections.
- [ ] Add allow_missing_origin=false TOML/CLI option; fixed-category rejection logs.
- [ ] Enforce loopback resolved endpoints, inherited listener and peer, including IPv4-mapped IPv6.
- [ ] Upgrade with header callback, explicit16MiB frame/message cap and32MiB write-buffer cap;5-second upgrade deadline. Retain tungstenite's existing64KiB HTTP-header/packet attack checks.
- [ ] Prove real HTTP403 versus101, slow/header-bound rejection, oversized/aggregate fragmentation handling.

## Task2: Protocol network deadlines

**Files:** Modify server.rs; tests in same private server module.

- [ ] Red tests: valid upgrade but no first GET_INFO releases connection; idle request expires; stalled sink expires; waiting for native credential is not timed out by a whole-operation deadline.
- [ ] Add10-second initial GET_INFO,600-second next-request idle and5-second response-write deadlines. Only wrap network waits.
- [ ] Skip Ping/Pong control frames; reject binary requests without payload echo; keep normalClose behavior.
- [ ] Verify allowed GET_INFO before native access, wrong origin cannot spoof JSONhost, diagnostics masking unaffected.

## Task3: QA, docs and separate local build

**Files:** browser-boundary.md usage, README links, desktop-smoke.py Origin header, local fixture wrappers outside repo.

- [ ] Update synthetic desktop smoke to supply an allowed Origin, without opting out; keep underlying private-bus/NSS isolation.
- [ ] Full workspace/default/CLI/GTK tests, application --no-deps Clippy, fmt/diff, private GUI/bus/smoke and focused boundary wire tests.
- [ ] One independent final review; fix findings, document limitations.
- [ ] Preserve existing launchers, add separate run-browser-boundary.sh and diagnostics wrapper; document manual real-portal acceptance.
- [ ] Commit/push separate codex/browser-boundary branch, draft PR on codex/seamless-certificate-menu; no upstream/merge.

## Rulings and evidence

Ruling: Origin restriction defends the browser boundary, not local-process authentication; non-browser clients can forge an allowed Origin, per RFC6455. No per-signature approval/browser-companion/packaging scope expansion.

Ruling: Network deadlines do not forcibly interrupt synchronous NSS/XMLSec or PIN callbacks, and runtime blocking may defer timer observation. There is no new native PIN timeout; native worker isolation is separate scope.

Ruling:16MiB bounds the encoded request, roughly12MiB XML before JSON/certificate overhead. Signed responses need write-buffer headroom. Unusually large real declarations and idle/reconnect behavior remain user-operated QA. Limits are fixed defaults for this bounded feature; tuning can follow measured compatibility needs.
