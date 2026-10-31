#!/bin/bash
# Builds "Andromeda Link.app" for macOS arm64 on Linux from a cross-built andromeda-link: Qt's frameworks, plugins
# and QML go inside, then an ad-hoc signature. On Linux it needs llvm-objdump, llvm-install-name-tool and rcodesign;
# on a Mac it uses otool, install_name_tool and codesign.
# Usage: bundle.sh BINARY_DIR [OUT.app]; BINARY_DIR holds andromeda-link, andromeda-linkd and andromeda-link-session.
set -euo pipefail
BIN=${1:?the directory of the macOS binaries}
APP=${2:-Andromeda Link.app}
QT=${QT_MAC:-$HOME/.cache/qt-mac/6.12.0/macos}
HERE=$(dirname "$(readlink -f "$0")")
C=$APP/Contents
if [[ $(uname) == Darwin ]]; then
  libs() { otool -L "$@" || true; }
  loads() { otool -l "$1"; }
  rename() { install_name_tool "$@"; }
  sign() { codesign --force --deep --sign - "$1"; }
else
  libs() { llvm-objdump --macho --dylibs-used "$@" || true; }
  loads() { llvm-objdump --macho --private-headers "$1"; }
  rename() { llvm-install-name-tool "$@"; }
  sign() { rcodesign sign "$1" > /dev/null; }
fi

rm -rf "$APP"
mkdir -p "$C/MacOS" "$C/Frameworks" "$C/PlugIns" "$C/Resources/qml" "$C/Resources/LaunchAgents"
cp "$BIN/andromeda-link" "$BIN/andromeda-linkd" "$C/MacOS/"
# What the Mac plays, read by Apple's perl for the daemon (crates/link-nowplaying).
cp "$BIN/liblink_nowplaying.dylib" "$C/Frameworks/"
# link-session runs from its own bundle, so Accessibility and notifications name it "Andromeda Link Helper".
HELPER="$C/Library/LoginItems/Andromeda Link Helper.app/Contents"
mkdir -p "$HELPER/MacOS" "$HELPER/Resources"
cp "$BIN/andromeda-link-session" "$HELPER/MacOS/"
cp "$HERE/Helper-Info.plist" "$HELPER/Info.plist"
cp "$HERE/AppIcon.icns" "$HELPER/Resources/AppIcon.icns"
cp "$HERE"/org.andromeda.*.plist "$C/Resources/LaunchAgents/"
cp "$HERE/install-agents.sh" "$C/Resources/"
cp "$HERE/Info.plist" "$C/Info.plist"
cp "$HERE/AppIcon.icns" "$C/Resources/AppIcon.icns"
printf '[Paths]\nPlugins = PlugIns\nQmlImports = Resources/qml\n' > "$C/Resources/qt.conf"

for plugin in platforms/libqcocoa.dylib imageformats/libqsvg.dylib imageformats/libqjpeg.dylib \
  iconengines/libqsvgicon.dylib multimedia/libdarwinmediaplugin.dylib tls/libqsecuretransportbackend.dylib; do
  [[ -e $QT/plugins/$plugin ]] || continue
  mkdir -p "$C/PlugIns/$(dirname "$plugin")"
  cp "$QT/plugins/$plugin" "$C/PlugIns/$plugin"
done
for module in QtQml QtQuick QtMultimedia; do
  cp -a "$QT/qml/$module" "$C/Resources/qml/"
done
# The menu bar item.
mkdir -p "$C/Resources/qml/Qt/labs"
cp -a "$QT/qml/Qt/labs/platform" "$C/Resources/qml/Qt/labs/"

# Copies every Qt framework that a binary in the bundle links, until none is missing.
while :; do
  missing=$(find "$C" -type f \( -name '*.dylib' -o -path '*/MacOS/*' -o -path '*.framework/Versions/A/*' \) \
    -print0 | xargs -0 bash -c "$(declare -f libs); libs \"\$@\"" _ 2> /dev/null \
    | sed -n 's|^\s*@rpath/\(Qt[A-Za-z0-9]*\)\.framework/.*|\1|p' | sort -u \
    | while read -r name; do [[ -e $C/Frameworks/$name.framework ]] || echo "$name"; done)
  [[ -n $missing ]] || break
  for name in $missing; do
    cp -a "$QT/lib/$name.framework" "$C/Frameworks/"
    rm -rf "$C/Frameworks/$name.framework/Headers" "$C/Frameworks/$name.framework/Versions/A/Headers"
  done
done

for rpath in $(loads "$C/MacOS/andromeda-link" | sed -n 's/^ *path \(\/[^ ]*\) .*/\1/p'); do
  rename -delete_rpath "$rpath" "$C/MacOS/andromeda-link"
done
rename -add_rpath @executable_path/../Frameworks "$C/MacOS/andromeda-link"

sign "$APP"
echo "$APP: $(du -sh "$APP" | cut -f1)"
