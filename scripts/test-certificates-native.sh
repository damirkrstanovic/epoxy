#!/usr/bin/env bash
set -euo pipefail
umask 077
fixture=$(mktemp -d)
trap 'rm -rf -- "$fixture"' EXIT
mkdir "$fixture/nss"
certutil -N --empty-password -d "sql:$fixture/nss"
# Generate directly in this NSS version's private database, avoiding PKCS#12
# import/encoding differences between distribution NSS and Python releases.
touch "$fixture/password"
head -c 64 /dev/urandom > "$fixture/noise"
certutil -S -d "sql:$fixture/nss" -n synthetic -s 'CN=Synthetic native fixture' \
  -t u,u,u -x -k rsa -g 2048 -m 123 -v 12 -z "$fixture/noise" \
  -f "$fixture/password" --keyUsage digitalSignature,nonRepudiation
certutil -L -d "sql:$fixture/nss" -n synthetic -r > "$fixture/cert.der"
certutil -L -d "sql:$fixture/nss" -n synthetic -a > "$fixture/cert.pem"
EPOXY_SYNTHETIC_NSS_DB="$fixture/nss" EPOXY_SYNTHETIC_CERT_DER="$fixture/cert.der" EPOXY_SYNTHETIC_SIGNED_XML="$fixture/signed.xml" cargo test -p epoxy --locked --offline --profile "${EPOXY_TEST_PROFILE:-dev}" --lib nss::native_tests::xmlsec_key_slot_is_checked_and_all_references_are_released -- --ignored --exact --test-threads=1
xmlsec1 --verify --pubkey-cert-pem "$fixture/cert.pem" "$fixture/signed.xml"
