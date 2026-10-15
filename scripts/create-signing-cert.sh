#!/usr/bin/env bash
# Creates the self-signed code-signing certificate that Murmur releases are
# signed with. macOS ties Accessibility and microphone grants to the signing
# certificate, so every release must use the same one: run this once, store
# the outputs as GitHub secrets, and keep a private backup.
#
# Usage: scripts/create-signing-cert.sh <output-dir> [common-name]
#
# Writes into <output-dir> (created with mode 700, files 600):
#   murmur-signing.p12         certificate and private key (PKCS#12)
#   murmur-signing.p12.base64  the same, base64 on one line: MACOS_CERT_P12_BASE64
#   murmur-signing.password    the .p12 password: MACOS_CERT_PASSWORD
#   murmur-signing.identity    the certificate name: MACOS_SIGNING_IDENTITY
#   murmur-signing.pem         the public certificate, safe to share
#
# The output directory must be outside any git checkout, so the private key
# can never be committed by accident.

set -euo pipefail

usage() {
  echo "usage: $0 <output-dir> [common-name]" >&2
  exit 2
}

out_dir="${1:-}"
[ -n "$out_dir" ] || usage
common_name="${2:-Murmur Code Signing}"

if [ -e "$out_dir/murmur-signing.p12" ]; then
  echo "error: $out_dir/murmur-signing.p12 already exists; refusing to overwrite it." >&2
  exit 1
fi

mkdir -p "$out_dir"
chmod 700 "$out_dir"
if git -C "$out_dir" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "error: $out_dir is inside a git work tree; choose a directory outside any repository." >&2
  exit 1
fi

umask 077
work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT

cat >"$work_dir/openssl.cnf" <<EOF
[req]
distinguished_name = dn
x509_extensions = codesign
prompt = no

[dn]
CN = $common_name

[codesign]
basicConstraints = critical, CA:FALSE
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
subjectKeyIdentifier = hash
EOF

openssl req -x509 -new -newkey rsa:3072 -sha256 -days 3650 -nodes \
  -config "$work_dir/openssl.cnf" \
  -keyout "$work_dir/key.pem" \
  -out "$work_dir/cert.pem"

password="$(openssl rand -base64 32 | tr -d '\n')"

# 3DES and SHA-1 MAC: the PKCS#12 encoding that `security import` accepts on
# every macOS version, whether this runs with OpenSSL or LibreSSL.
openssl pkcs12 -export \
  -inkey "$work_dir/key.pem" \
  -in "$work_dir/cert.pem" \
  -name "$common_name" \
  -keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES -macalg sha1 \
  -passout "pass:$password" \
  -out "$out_dir/murmur-signing.p12"

openssl base64 -A -in "$out_dir/murmur-signing.p12" >"$out_dir/murmur-signing.p12.base64"
printf '%s' "$password" >"$out_dir/murmur-signing.password"
printf '%s' "$common_name" >"$out_dir/murmur-signing.identity"
cp "$work_dir/cert.pem" "$out_dir/murmur-signing.pem"

fingerprint="$(openssl x509 -in "$work_dir/cert.pem" -noout -fingerprint -sha256)"
cat <<EOF
Created a self-signed code-signing certificate, valid for 10 years.
  Name:    $common_name
  $fingerprint
  Output:  $out_dir

Next steps (see BUILD.md, "Stable release signing"):
  1. Add three secrets to the release-signing environment of the repository:
       MACOS_CERT_P12_BASE64   contents of murmur-signing.p12.base64
       MACOS_CERT_PASSWORD     contents of murmur-signing.password
       MACOS_SIGNING_IDENTITY  contents of murmur-signing.identity
  2. Back up murmur-signing.p12 and its password somewhere private.
     Losing them means users must grant permissions again after the next update.
  3. Delete the local copies once both are stored.
EOF
