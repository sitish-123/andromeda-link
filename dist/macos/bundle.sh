#!/bin/bash
# Builds "Andromeda Link.app" for macOS arm64 on Linux from a cross-built andromeda-link: Qt's frameworks, plugins
# and QML go inside, then an ad-hoc signature. Needs llvm-objdump, llvm-install-name-tool and rcodesign.
# Usage: bundle.sh BINARY [OUT.app]
set -euo pipefail
BIN=${1:?the macOS andromeda-link binary}
APP=${2:-Andromeda Link.app}
QT=${QT_MAC:-$HOME/.cache/qt-mac/6.12.0/macos}
HERE=$(dirname "$(readlink -f "$0")")
C=$APP/Contents

rm -rf "$APP"
mkdir -p "$C/MacOS" "$C/Frameworks" "$C/PlugIns" "$C/Resources/qml"
cp "$BIN" "$C/MacOS/andromeda-link"
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

# Copies every Qt framework that a binary in the bundle links, until none is missing.
while :; do
  missing=$(find "$C" -type f \( -name '*.dylib' -o -path '*/MacOS/*' -o -path '*.framework/Versions/A/*' \) \
    -exec llvm-objdump --macho --dylibs-used {} + 2> /dev/null \
    | sed -n 's|^\s*@rpath/\(Qt[A-Za-z]*\)\.framework/.*|\1|p' | sort -u \
    | while read -r name; do [[ -e $C/Frameworks/$name.framework ]] || echo "$name"; done)
  [[ -n $missing ]] || break
  for name in $missing; do
    cp -a "$QT/lib/$name.framework" "$C/Frameworks/"
    rm -rf "$C/Frameworks/$name.framework/Headers" "$C/Frameworks/$name.framework/Versions/A/Headers"
  done
done

for rpath in $(llvm-objdump --macho --private-headers "$C/MacOS/andromeda-link" | sed -n 's/^ *path \(\/[^ ]*\) .*/\1/p'); do
  llvm-install-name-tool -delete_rpath "$rpath" "$C/MacOS/andromeda-link"
done
llvm-install-name-tool -add_rpath @executable_path/../Frameworks "$C/MacOS/andromeda-link"

rcodesign sign "$APP" > /dev/null
echo "$APP: $(du -sh "$APP" | cut -f1)"
