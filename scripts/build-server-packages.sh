#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -n 1)
TARGET=${CARGO_BUILD_TARGET:-$(rustc -vV | sed -n 's/^host: //p')}
ARCH=${PACKAGE_ARCH:-amd64}
OUT="$ROOT/target/packages"
STAGE="$ROOT/target/package-stage"

cargo build --release -p print-bridge-server --target "$TARGET"
rm -rf "$STAGE"
mkdir -p "$STAGE/deb/DEBIAN" "$STAGE/deb/usr/bin" "$STAGE/deb/lib/systemd/system" "$OUT"
install -m 0755 "$ROOT/target/$TARGET/release/print-bridge" "$STAGE/deb/usr/bin/print-bridge"
install -m 0644 "$ROOT/apps/server/packaging/systemd/print-bridge.service" "$STAGE/deb/lib/systemd/system/print-bridge.service"
sed -e "s/\${VERSION}/$VERSION/" -e "s/\${ARCH}/$ARCH/" "$ROOT/apps/server/packaging/deb/control" > "$STAGE/deb/DEBIAN/control"
for script in postinst prerm postrm; do
  install -m 0755 "$ROOT/apps/server/packaging/deb/$script" "$STAGE/deb/DEBIAN/$script"
done
dpkg-deb --build "$STAGE/deb" "$OUT/print-bridge-server_${VERSION}_${ARCH}.deb"

if command -v rpmbuild >/dev/null 2>&1; then
  RPMROOT="$STAGE/rpmbuild"
  mkdir -p "$RPMROOT/BUILD" "$RPMROOT/BUILDROOT" "$RPMROOT/RPMS" "$RPMROOT/SOURCES" "$RPMROOT/SPECS" "$RPMROOT/SRPMS"
  install -m 0755 "$ROOT/target/$TARGET/release/print-bridge" "$RPMROOT/SOURCES/print-bridge"
  install -m 0644 "$ROOT/apps/server/packaging/systemd/print-bridge.service" "$RPMROOT/SOURCES/print-bridge.service"
  sed "s/__VERSION__/$VERSION/" "$ROOT/apps/server/packaging/rpm/print-bridge.spec" > "$RPMROOT/SPECS/print-bridge.spec"
  rpmbuild --define "_topdir $RPMROOT" -bb "$RPMROOT/SPECS/print-bridge.spec"
  find "$RPMROOT/RPMS" -name '*.rpm' -exec cp {} "$OUT/" \;
fi
