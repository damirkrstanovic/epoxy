# Native Linux packaging implementation plan

Goal: build installable x86_64 packages for Arch, Fedora, Debian 13 and Ubuntu
24.04, using system shared libraries and preserving the GTK/tray experience.

Architecture: one shared staging layout, native PKGBUILD/RPM/Debian recipes,
locked Rust source dependencies, and isolated distribution builds in CI. Package
installation must not start Epoxy, enable autostart, change a user's NSS database,
or install proprietary middleware. Distribution repository submission is separate.

## Deliverables

- [ ] Portable native build PR: replace fixed include/library paths with
  pkg-config, and check XMLSec runtime ABI against the actual compile-time
  headers rather than a hard-coded 1.3. Test real synthetic NSS signing on 1.2
  and 1.3; never weaken version checking without this evidence.
- [ ] Packaging PR: shared install script and desktop/icon assets; Arch
  PKGBUILD, Fedora spec and Debian metadata. Preserve system library dependencies
  and bundle Rust notices and corresponding vendored source in release artifacts.
  Document OpenSC/NSS setup without altering existing user databases.
- [ ] CI and verification: distro build matrix, package install/removal checks,
  executable/version checks, synthetic signing, artifact checksums, source bundle.
  CI uploads build artifacts; no automatic public release or repository publishing.

## Review focus

- XMLSec 1.2 versus 1.3 ABI and symbols: real signing/verification in each image.
- Fedora lib64 / Debian multiarch paths: pkg-config discovery, no path guesses.
- Upgrades/removal: native manager metadata and no user-file deletion hooks.
- Desktop startup: tray default, GUI fallback, no duplicate service activation.
- Licensing: exact Cargo.lock vendored source plus retained notices; no native
  libraries or vendor middleware bundled in the application package.

## Execution

Implement inline on feature branches in the existing isolated worktree. Existing
96-test baseline passed before starting. First validate the compatibility failure
in an isolated distro image, then apply the portable build change. Run native
package tooling and synthetic tests without accessing the real card/session.
