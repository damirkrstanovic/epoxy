#!/usr/bin/env bash
set -euo pipefail
umask 077
fixture=$(mktemp -d)
trap 'rm -rf -- "$fixture"' EXIT
mkdir "$fixture/nss"
certutil -N --empty-password -d "sql:$fixture/nss"
python3 - "$fixture" <<'PY'
import sys
from pathlib import Path
from datetime import datetime, timezone
from cryptography import x509
from cryptography.x509.oid import NameOID
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.serialization import pkcs12
from cryptography.hazmat.primitives.asymmetric import rsa
p=Path(sys.argv[1]); key=rsa.generate_private_key(public_exponent=65537,key_size=2048)
name=x509.Name([x509.NameAttribute(NameOID.COMMON_NAME,'Synthetic native fixture')])
c=(x509.CertificateBuilder().subject_name(name).issuer_name(name).public_key(key.public_key()).serial_number(123).not_valid_before(datetime(2000,1,1,tzinfo=timezone.utc)).not_valid_after(datetime(2090,1,1,tzinfo=timezone.utc)).add_extension(x509.KeyUsage(True,True,False,False,False,False,False,None,None),critical=True).sign(key,hashes.SHA256()))
p.joinpath('cert.der').write_bytes(c.public_bytes(serialization.Encoding.DER))
p.joinpath('cert.pem').write_bytes(c.public_bytes(serialization.Encoding.PEM))
p.joinpath('key.p12').write_bytes(pkcs12.serialize_key_and_certificates(b'synthetic',key,c,None,serialization.NoEncryption()))
PY
pk12util -i "$fixture/key.p12" -d "sql:$fixture/nss" -W '' -K ''
EPOXY_SYNTHETIC_NSS_DB="$fixture/nss" EPOXY_SYNTHETIC_CERT_DER="$fixture/cert.der" EPOXY_SYNTHETIC_SIGNED_XML="$fixture/signed.xml" cargo test -p epoxy --offline --lib nss::native_tests::xmlsec_key_slot_is_checked_and_all_references_are_released -- --ignored --exact --test-threads=1
xmlsec1 --verify --pubkey-cert-pem "$fixture/cert.pem" "$fixture/signed.xml"
