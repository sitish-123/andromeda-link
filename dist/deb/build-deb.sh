#!/usr/bin/env bash
# Builds one .deb from release binaries already built on the target release (Ubuntu 24.04 or later, Debian 13), with
# the same files as pkg/PKGBUILD's andromeda-link-git: the daemon, session, mount, mirror, apps, and, when built, the
# Qt app (which needs Qt 6.7, or QT_BUNDLE). The daemon and the
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
A=$P

for b in andromeda-linkd andromeda-link-mount andromeda-link-mirror andromeda-link-apps andromeda-link-session; do
  install -Dm755 "$BIN/$b" "$P/usr/bin/$b"
done
U=$P/usr/lib/systemd/user
for s in andromeda-linkd andromeda-link-mount andromeda-link-session; do
  install -Dm644 "$ROOT/dist/$s.service" "$U/$s.service"
  install -d "$U/graphical-session.target.wants"
  ln -s "../$s.service" "$U/graphical-session.target.wants/$s.service"
done
# systemd 255 (Ubuntu 24.04) cannot drop capabilities in a user unit and exits 218/CAPABILITIES; each of these options
# drops some. A user process has none to drop, and NoNewPrivileges still stops it gaining any.
sed -i -E '/^(CapabilityBoundingSet|ProtectClock|ProtectKernelModules|ProtectKernelLogs|PrivateDevices)=/d' \
  "$U/andromeda-linkd.service"
install -Dm644 "$ROOT/dist/org.andromeda.Link1.service" "$P/usr/share/dbus-1/services/org.andromeda.Link1.service"
install -Dm644 "$ROOT/api/org.andromeda.Link1.xml" "$P/usr/share/dbus-1/interfaces/org.andromeda.Link1.xml"
install -Dm644 "$ROOT/dist/andromeda-link.ufw" "$P/etc/ufw/applications.d/andromeda-link"
install -Dm644 "$ROOT/dist/andromeda-link.firewalld.xml" "$P/usr/lib/firewalld/services/andromeda-link.xml"
install -Dm644 "$ROOT/dist/andromeda-link-localsend.firewalld.xml" \
  "$P/usr/lib/firewalld/services/andromeda-link-localsend.xml"
install -Dm644 "$ROOT/dist/60-andromeda-link-bluetooth.conf" \
  "$P/usr/share/wireplumber/wireplumber.conf.d/60-andromeda-link-bluetooth.conf"
install -Dm644 "$ROOT/LICENSE" "$P/usr/share/doc/andromeda-link/copyright"
# The app needs Qt 6.7. With QT_BUNDLE (a Qt prefix, as aqtinstall writes it) the .deb carries that Qt under
# /usr/lib/andromeda-link/qt, so releases with older Qt (Ubuntu 24.04) install it the same way; the binary must then
# have been linked with the rpath /usr/lib/andromeda-link/qt/lib.
APP=$([[ -x $BIN/andromeda-link ]] && echo yes || true)
QT_DIR=/usr/lib/andromeda-link/qt
if [[ -n $APP && -n ${QT_BUNDLE:-} ]]; then
  install -Dm755 "$BIN/andromeda-link" "$A/usr/lib/andromeda-link/andromeda-link"
  install -d "$A/usr/bin" "$A$QT_DIR/lib" "$A$QT_DIR/plugins" "$A$QT_DIR/qml"
  ln -s ../lib/andromeda-link/andromeda-link "$A/usr/bin/andromeda-link"
  printf '[Paths]\nPrefix = %s\n' "$QT_DIR" > "$A/usr/lib/andromeda-link/qt.conf"
  for dir in platforms platforminputcontexts platformthemes xcbglintegrations egldeviceintegrations iconengines \
    imageformats multimedia tls wayland-decoration-client wayland-graphics-integration-client wayland-shell-integration; do
    cp -a "$QT_BUNDLE/plugins/$dir" "$A$QT_DIR/plugins/"
  done
  for module in QtQml QtQuick QtMultimedia; do cp -a "$QT_BUNDLE/qml/$module" "$A$QT_DIR/qml/"; done
  # Every library of the Qt prefix that the app, a plugin or a QML module needs, until none is missing.
  while :; do
    missing=$(find "$A/usr/lib/andromeda-link" -type f \( -name '*.so*' -o -name andromeda-link \) -print0 \
      | xargs -0 objdump -p 2> /dev/null | sed -n 's/^ *NEEDED *//p' | sort -u \
      | while read -r lib; do [[ -e $QT_BUNDLE/lib/$lib && ! -e $A$QT_DIR/lib/$lib ]] && echo "$lib"; done || true)
    [[ -n $missing ]] || break
    for lib in $missing; do cp -L "$QT_BUNDLE/lib/$lib" "$A$QT_DIR/lib/$lib"; done
  done
elif [[ -n $APP ]]; then
  install -Dm755 "$BIN/andromeda-link" "$A/usr/bin/andromeda-link"
fi
install -Dm644 "$ROOT/dist/org.andromeda.Link.desktop" "$A/usr/share/applications/org.andromeda.Link.desktop"
cp -r "$ROOT/dist/icons" "$A/usr/share/"

install -d "$P/DEBIAN"
if [[ -n ${QT_BUNDLE:-} ]]; then
  APP_DEPENDS="libc6, libstdc++6, libgl1, libegl1, libfontconfig1, libfreetype6, libdbus-1-3, libxkbcommon0,
 libxkbcommon-x11-0, libxcb-cursor0, libxcb-icccm4, libxcb-keysyms1, libxcb-shape0, libwayland-client0,
 libwayland-cursor0, libwayland-egl1, libglib2.0-0t64 | libglib2.0-0, libpulse0"
else
  APP_DEPENDS="libqt6core6t64 (>= 6.7) | libqt6core6 (>= 6.7), libqt6qml6, libqt6quick6,
 libqt6multimedia6, qml6-module-qtquick, qml6-module-qtquick-controls, qml6-module-qtquick-templates,
 qml6-module-qtquick-layouts, qml6-module-qtquick-dialogs, qml6-module-qtquick-shapes, qml6-module-qtquick-effects,
 qml6-module-qtmultimedia, qml6-module-qtqml-workerscript"
fi
cat > "$P/DEBIAN/control" << EOF
Package: andromeda-link
Version: $VERSION
Architecture: amd64
Maintainer: Andromeda <sitish@akintechnologies.com>
Depends: libc6, libgcc-s1, dbus, fuse3, libgtk-4-1, gstreamer1.0-plugins-base, gstreamer1.0-plugins-bad${APP:+,
 $APP_DEPENDS}
Replaces: andromeda-link-app
Breaks: andromeda-link-app
Recommends: gstreamer1.0-gtk4, xdg-desktop-portal-gnome | xdg-desktop-portal-kde
Suggests: bluez, network-manager, gstreamer1.0-vaapi, gstreamer1.0-libav, scrcpy, adb
Description: Andromeda Link: phone and desktop continuity
 Clipboard, notifications, media, files, texts, and screen mirroring between an Android phone and the desktop.
EOF
echo /etc/ufw/applications.d/andromeda-link > "$P/DEBIAN/conffiles"
# Phones reach the daemon only through the firewall; open whichever one runs, as pkg/andromeda-link.install does.
cat > "$P/DEBIAN/postinst" << 'EOF'
#!/bin/sh
set -e
if command -v ufw > /dev/null && ufw status 2> /dev/null | grep -q '^Status: active'; then
  ufw allow 'Andromeda Link' > /dev/null 2>&1 || echo "ufw: phones cannot pair until you run: sudo ufw allow 'Andromeda Link'"
fi
if command -v firewall-cmd > /dev/null && firewall-cmd --state > /dev/null 2>&1; then
  firewall-cmd --reload > /dev/null 2>&1 || true
  firewall-cmd --permanent --add-service=andromeda-link > /dev/null 2>&1 && firewall-cmd --reload > /dev/null 2>&1 ||
    echo "firewalld: phones cannot pair until you run: sudo firewall-cmd --permanent --add-service=andromeda-link"
fi
EOF
chmod 755 "$P/DEBIAN/postinst"
mkdir -p "$OUT"
dpkg-deb --root-owner-group --build "$P" "$OUT/andromeda-link_${VERSION}_amd64.deb"
