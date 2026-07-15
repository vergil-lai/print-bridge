#!/bin/sh
set -eu

ASSETS=${1:?Usage: build-apt-repository.sh ASSETS_DIR OUTPUT_DIR}
OUT=${2:?Usage: build-apt-repository.sh ASSETS_DIR OUTPUT_DIR}
: "${KEY_FINGERPRINT:?KEY_FINGERPRINT is required}"
: "${APT_GPG_PASSPHRASE:?APT_GPG_PASSPHRASE is required}"

for command in apt-ftparchive dpkg-deb gpg gzip sha256sum; do
  command -v "$command" >/dev/null 2>&1 || {
    echo "Required command not found: $command" >&2
    exit 1
  }
done

rm -rf "$OUT"
POOL="$OUT/pool/main/p/print-bridge"
mkdir -p "$POOL"

found_deb=false
for package in "$ASSETS"/*.deb; do
  if [ ! -f "$package" ]; then
    continue
  fi

  found_deb=true
  cp "$package" "$POOL/"
done

if [ "$found_deb" = false ]; then
  echo "No deb packages found in $ASSETS." >&2
  exit 1
fi

require_package() {
  expected_name=$1
  expected_arch=$2

  for package in "$POOL"/*.deb; do
    package_name=$(dpkg-deb -f "$package" Package)
    package_arch=$(dpkg-deb -f "$package" Architecture)

    if [ "$package_name" = "$expected_name" ] && [ "$package_arch" = "$expected_arch" ]; then
      return 0
    fi
  done

  echo "Missing $expected_name package for $expected_arch." >&2
  exit 1
}

require_package "print-bridge-desktop" "amd64"
require_package "print-bridge-desktop" "arm64"
require_package "print-bridge-server" "amd64"
require_package "print-bridge-server" "arm64"

generate_packages_index() {
  arch=$1
  directory="$OUT/dists/stable/main/binary-$arch"
  mkdir -p "$directory/by-hash/SHA256"

  (
    cd "$OUT"
    apt-ftparchive --arch "$arch" packages pool
  ) >"$directory/Packages"
  gzip -9 -c "$directory/Packages" >"$directory/Packages.gz"

  for index in "$directory/Packages" "$directory/Packages.gz"; do
    hash=$(sha256sum "$index" | cut -d ' ' -f 1)
    cp "$index" "$directory/by-hash/SHA256/$hash"
  done
}

generate_packages_index amd64
generate_packages_index arm64

(
  cd "$OUT"
  apt-ftparchive \
    -o APT::FTPArchive::Release::Origin="PrintBridge" \
    -o APT::FTPArchive::Release::Label="PrintBridge" \
    -o APT::FTPArchive::Release::Suite="stable" \
    -o APT::FTPArchive::Release::Codename="stable" \
    -o APT::FTPArchive::Release::Architectures="amd64 arm64" \
    -o APT::FTPArchive::Release::Components="main" \
    -o APT::FTPArchive::Release::Description="PrintBridge packages" \
    -o APT::FTPArchive::Release::Acquire-By-Hash="yes" \
    release dists/stable
) >"$OUT/dists/stable/Release"

gpg --batch --list-secret-keys "$KEY_FINGERPRINT" >/dev/null
gpg --batch --yes --export "$KEY_FINGERPRINT" >"$OUT/printbridge-archive-keyring.gpg"
gpg --batch --yes --armor --export "$KEY_FINGERPRINT" >"$OUT/printbridge-archive-keyring.asc"

gpg --batch --yes \
  --pinentry-mode loopback \
  --passphrase "$APT_GPG_PASSPHRASE" \
  --local-user "$KEY_FINGERPRINT" \
  --clearsign \
  --output "$OUT/dists/stable/InRelease" \
  "$OUT/dists/stable/Release"

gpg --batch --yes \
  --pinentry-mode loopback \
  --passphrase "$APT_GPG_PASSPHRASE" \
  --local-user "$KEY_FINGERPRINT" \
  --armor \
  --detach-sign \
  --output "$OUT/dists/stable/Release.gpg" \
  "$OUT/dists/stable/Release"

echo "Built signed APT repository in $OUT."
