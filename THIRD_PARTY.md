# Licensing and distribution notes

Epoxy and the local `xmlsec-nss-sys` wrapper are MIT-licensed; retain the root
[LICENSE](LICENSE), including the original author's copyright notice. The Cargo
package metadata states that license explicitly. This does not relicense native
libraries, generated third-party bindings, or other dependencies.

## Review scope

Reviewed on 2026-10-15 using `cargo metadata --locked --all-features` and the
installed native-library package metadata. The resolved Cargo graph contains
276 packages (274 registry dependencies and two local packages), including
build, development and non-Linux dependencies.
No incompatible license was identified for distributing this project as an
open-source MIT application with the conditions below. This is a technical
inventory, not a legal opinion or a complete binary-release notice bundle.

| Component | License / obligation |
| --- | --- |
| Most Rust dependencies | MIT, Apache-2.0, BSD, ISC or Unlicense; retain applicable license/copyright/NOTICE material when redistributing their code in a binary. For OR expressions choose an offered license; AND expressions require both. |
| `option-ext` 0.2.0, via `dirs` | MPL-2.0; preserve its license and make its exact covered source, including any changes, available to recipients. This does not require relicensing Epoxy's separate files. Source: [crate archive](https://crates.io/api/v1/crates/option-ext/0.2.0/download), [repository](https://github.com/soc/option-ext). |
| `unicode-ident` | `(MIT OR Apache-2.0) AND Unicode-3.0`; the Unicode notice is additional, not an alternative to MIT/Apache. |
| `ksni` | Unlicense in crate metadata; preserve upstream provenance. |
| GTK and related native libraries | GTK is LGPL-2.1-or-later. Prefer distribution-provided shared libraries. Shipping these libraries or statically linking them adds their notice, source and relinking obligations. The MIT license of gtk-rs bindings does not replace native GTK's license. |
| NSS / NSPR | MPL-2.0; native shared libraries. If bundled, supply their notices and corresponding covered source. |
| XMLSec / libxml2 | MIT-style licenses; retain their own notices if redistributed. |
| OpenSC PKCS#11 module | LGPL; a separately installed runtime module. Keep it a system dependency rather than bundling proprietary token middleware. Check the exact OpenSC release's notices if distributing it. |
| PC/SC service | Separate system service with component-specific licenses; do not infer the client library's license from the aggregate package license list. |
| DMS-derived navigation test | MIT, copyright Avenge Media LLC; attribution and full permission text are in [scripts/fixtures/LICENSE.dms](scripts/fixtures/LICENSE.dms). The fixture references that notice. |

Python D-Bus/GObject, Quickshell, Xvfb and command-line test tools are external
test dependencies, not code bundled into the Epoxy executable. If a distribution
image bundles them, its distributor must also comply with their licenses.
The reviewed tree contains no SmartBox JAR, decompiled Java implementation or
vendor PKCS#11 binary. Protocol interoperability does not grant redistribution
rights to those vendor components.

## Before publishing binary packages

1. Generate the license/copyright/NOTICE bundle for the exact lockfile, target
   and features used, including statically incorporated Rust dependencies.
   Preserve upstream notices rather than replacing them with this summary.
2. Provide the MPL-covered source (notably `option-ext`) and any modified covered
   files alongside the release or through a durable source-download location.
3. Prefer system shared libraries; inventory every library actually bundled in
   an AppImage/container/standalone archive and satisfy its own requirements.
4. Recheck this inventory when dependencies or packaging change. Cargo metadata
   is evidence of declared licensing, not a file-by-file provenance audit.

References: [MPL FAQ](https://www.mozilla.org/en-US/MPL/2.0/FAQ/),
[GTK licensing](https://www.gtk.org/),
[OpenSC COPYING](https://github.com/OpenSC/OpenSC/blob/master/COPYING).
