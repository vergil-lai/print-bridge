#!/bin/sh
set -eu

ASSETS=${1:?Usage: build-rpm-repository.sh ASSETS_DIR OUTPUT_DIR}
OUT=${2:?Usage: build-rpm-repository.sh ASSETS_DIR OUTPUT_DIR}
: "${KEY_FINGERPRINT:?KEY_FINGERPRINT is required}"
: "${REPOSITORY_GPG_PASSPHRASE:?REPOSITORY_GPG_PASSPHRASE is required}"

for command in createrepo_c gpg rpm; do
  command -v "$command" >/dev/null 2>&1 || {
    echo "Required command not found: $command" >&2
    exit 1
  }
done

rm -rf "$OUT"
mkdir -p "$OUT/packages"

cat >"$OUT/printbridge.repo" <<'EOF'
[printbridge]
name=PrintBridge
baseurl=https://printbridge.pages.dev/rpm
enabled=1
gpgcheck=0
repo_gpgcheck=1
gpgkey=https://printbridge.pages.dev/rpm/RPM-GPG-KEY-printbridge
metadata_expire=5m
EOF

found_rpm=false
for package in "$ASSETS"/*.rpm; do
  if [ ! -f "$package" ]; then
    continue
  fi

  found_rpm=true
  cp "$package" "$OUT/packages/"
done

if [ "$found_rpm" = false ]; then
  echo "No rpm packages found in $ASSETS." >&2
  exit 1
fi

require_package() {
  expected_name=$1
  expected_arch=$2

  for package in "$OUT/packages"/*.rpm; do
    package_name=$(rpm -qp --queryformat '%{NAME}' "$package")
    package_arch=$(rpm -qp --queryformat '%{ARCH}' "$package")

    if [ "$package_name" = "$expected_name" ] && [ "$package_arch" = "$expected_arch" ]; then
      return 0
    fi
  done

  echo "Missing $expected_name package for $expected_arch." >&2
  exit 1
}

require_package "print-bridge" "x86_64"
require_package "print-bridge" "aarch64"
require_package "print-bridge-server" "x86_64"
require_package "print-bridge-server" "aarch64"

createrepo_c --checksum sha256 "$OUT"

gpg --batch --list-secret-keys "$KEY_FINGERPRINT" >/dev/null
gpg --batch --yes --armor --export "$KEY_FINGERPRINT" >"$OUT/RPM-GPG-KEY-printbridge"
gpg --batch --yes \
  --pinentry-mode loopback \
  --passphrase "$REPOSITORY_GPG_PASSPHRASE" \
  --local-user "$KEY_FINGERPRINT" \
  --armor \
  --detach-sign \
  --output "$OUT/repodata/repomd.xml.asc" \
  "$OUT/repodata/repomd.xml"

echo "Built signed RPM repository in $OUT."
