#!/usr/bin/env bash
# Builds two .debs from release binaries already built on the target release (Ubuntu 24.04 or later, Debian 13), with
# the same files as pkg/PKGBUILD's andromeda-link-git: andromeda-link (daemon, session, mount, mirror, apps) and
# andromeda-link-app (the Qt app, which needs Qt 6.7; on older releases the app comes as a Flatpak). The daemon and the
# mount start with the graphical session here, since only Andromeda has andromeda-session.target.
# Usage: build-deb.sh TARGET_RELEASE_DIR OUT_DIR
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
BIN=${1:?target release dir}
OUT=${2:?output dir}
VERSION="0~r$(git -C "$ROOT" rev-list --count HEAD).g$(git -C "$ROOT" rev-parse --short HEAD)"
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
P=$STAGE/andromeda-link
A=$STAGE/andromeda-link-app

for b in andromeda-linkd andromeda-link-mount andromeda-link-mirror andromeda-link-apps andromeda-link-session; do
  install -Dm755 "$BIN/$b" "$P/usr/bin/$b"
done
U=$P/usr/lib/systemd/user
for s in andromeda-linkd andromeda-link-mount andromeda-link-session; do
  install -Dm644 "$ROOT/dist/$s.service" "$U/$s.service"
  install -d "$U/graphical-session.target.wants"
  ln -s "../$s.service" "$U/graphical-session.target.wants/$s.service"
done
install -Dm644 "$ROOT/dist/org.andromeda.Link1.service" "$P/usr/share/dbus-1/services/org.andromeda.Link1.service"
install -Dm644 "$ROOT/api/org.andromeda.Link1.xml" "$P/usr/share/dbus-1/interfaces/org.andromeda.Link1.xml"
install -Dm644 "$ROOT/dist/andromeda-link.ufw" "$P/etc/ufw/applications.d/andromeda-link"
install -Dm644 "$ROOT/dist/60-andromeda-link-bluetooth.conf" \
  "$P/usr/share/wireplumber/wireplumber.conf.d/60-andromeda-link-bluetooth.conf"
install -Dm644 "$ROOT/LICENSE" "$P/usr/share/doc/andromeda-link/copyright"
install -Dm755 "$BIN/andromeda-link" "$A/usr/bin/andromeda-link"
install -Dm644 "$ROOT/dist/org.andromeda.Link.desktop" "$A/usr/share/applications/org.andromeda.Link.desktop"
install -Dm644 "$ROOT/LICENSE" "$A/usr/share/doc/andromeda-link-app/copyright"

install -d "$P/DEBIAN" "$A/DEBIAN"
cat > "$P/DEBIAN/control" << EOF
Package: andromeda-link
Version: $VERSION
Architecture: amd64
Maintainer: Andromeda <sitish@akintechnologies.com>
Depends: libc6, libgcc-s1, dbus, fuse3, libgtk-4-1, gstreamer1.0-plugins-base, gstreamer1.0-plugins-bad
Recommends: andromeda-link-app, gstreamer1.0-gtk4, xdg-desktop-portal-gnome | xdg-desktop-portal-kde
Suggests: bluez, network-manager, gstreamer1.0-vaapi, gstreamer1.0-libav, scrcpy, adb
Description: Andromeda Link: phone and desktop continuity
 Clipboard, notifications, media, files, texts, and screen mirroring between an Android phone and the desktop.
EOF
echo /etc/ufw/applications.d/andromeda-link > "$P/DEBIAN/conffiles"
# Phones reach the daemon only through the firewall; open it where ufw is running, as pkg/andromeda-link.install does.
cat > "$P/DEBIAN/postinst" << 'EOF'
#!/bin/sh
set -e
command -v ufw > /dev/null || exit 0
ufw status 2> /dev/null | grep -q '^Status: active' || exit 0
ufw allow 'Andromeda Link' > /dev/null 2>&1 || echo "ufw: phones cannot pair until you run: sudo ufw allow 'Andromeda Link'"
EOF
chmod 755 "$P/DEBIAN/postinst"
cat > "$A/DEBIAN/control" << EOF
Package: andromeda-link-app
Version: $VERSION
Architecture: amd64
Maintainer: Andromeda <sitish@akintechnologies.com>
Depends: andromeda-link (= $VERSION), libqt6core6t64 (>= 6.7) | libqt6core6 (>= 6.7), libqt6qml6, libqt6quick6,
 libqt6multimedia6, qml6-module-qtquick, qml6-module-qtquick-controls, qml6-module-qtquick-templates,
 qml6-module-qtquick-layouts, qml6-module-qtquick-dialogs, qml6-module-qtquick-shapes, qml6-module-qtquick-effects,
 qml6-module-qtmultimedia, qml6-module-qtqml-workerscript
Description: Andromeda Link app
 The desktop app for Andromeda Link: pairing, the phone's media, texts, photos, files, and screen.
EOF
mkdir -p "$OUT"
dpkg-deb --root-owner-group --build "$P" "$OUT/andromeda-link_${VERSION}_amd64.deb"
dpkg-deb --root-owner-group --build "$A" "$OUT/andromeda-link-app_${VERSION}_amd64.deb"
