# Native library portability

The build uses `pkg-config` for XMLSec's NSS backend and NSS headers/linker flags,
including Fedora's lib64 and Debian's multiarch layout. Minimum XMLSec is 1.2.39;
GTK desktop builds require 4.4 or newer. Install the corresponding development
packages and libclang before building. No private copy of a native library is
bundled.

Runtime XMLSec ABI validation uses the exact version of the compile-time headers.
A binary built against 1.2 is not interchangeable with a 1.3 build. Build packages
for each distribution/release rather than moving a compiled binary between them.

XMLSec 1.2's certificate loader only obtains a public key. Epoxy explicitly looks
up the associated NSS private-key handle after authentication and attaches it to
the XMLSec key. This also runs on newer XMLSec, removing dependence on the
certificate loader's changing behavior. The key material remains in NSS/the card.
The server still checks that the resulting key belongs to the selected token
before signing. Failure to obtain a private key fails closed.

Reference ownership follows the NSS/XMLSec APIs: certificate references are
released, key references transfer once to XMLSec on successful adoption, and
failure paths release their owners. The synthetic native test checks correct and
incorrect slots repeatedly, signs, then requires clean NSS shutdown (which fails
on leaked references). Its certificate/key are generated in an isolated NSS
database to avoid differences in PKCS#12 encodings across distro tools.

Validation during development: the original hard-coded 1.3 check failed on
Ubuntu 24.04's 1.2.39, and the original certificate loader failed the private-key
slot test. Both pass after this change. Synthetic signing and independent XML
verification also pass on Debian 13's 1.2.41 and Arch's 1.3.12.

Upstream ownership/behavior references:
- [XMLSec 1.2.39 public-key extraction](https://github.com/lsh123/xmlsec/blob/xmlsec-1_2_39/src/nss/x509.c)
- [XMLSec NSS key adoption](https://github.com/lsh123/xmlsec/blob/xmlsec-1_2_39/src/nss/pkikeys.c)
