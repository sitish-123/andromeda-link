#!/bin/bash
# Cross-builds andromeda-link for macOS arm64 on Linux and bundles "Andromeda Link.app" into target/macos.
# Needs: the xtool Darwin SDK (DARWIN_SDK), Qt for macOS of the host Qt's version (QT_MAC; `uvx --from aqtinstall
# aqt install-qt mac desktop <version> clang_64 -m qtmultimedia`), clang, lld, the aarch64-apple-darwin Rust target.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SDK=${DARWIN_SDK:-$HOME/.swiftpm/swift-sdks/darwin.artifactbundle/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk}
HOST_QT=${HOST_QT:-/usr/lib/qt6}
VERSION=$("$HOST_QT"/bin/qtpaths6 --qt-version)
QT=${QT_MAC:-$HOME/.cache/qt-mac/$VERSION/macos}
OUT=$ROOT/target/macos
TOOLS=$OUT/tools
[[ -d $SDK ]] || { echo "no Darwin SDK at $SDK" >&2; exit 1; }
[[ -d $QT/lib/QtCore.framework ]] || { echo "no Qt $VERSION for macOS at $QT" >&2; exit 1; }

mkdir -p "$TOOLS/libexec"
for kind in cc:clang cxx:clang++; do
  printf '#!/bin/sh\nexec %s -target arm64-apple-macos14 -isysroot "%s" -fuse-ld=lld "$@"\n' "${kind#*:}" "$SDK" > "$TOOLS/${kind%%:*}"
done
# The host's moc, qmlcachegen and others; rcc without zstd, which Qt for macOS does not link.
for tool in "$HOST_QT"/*; do ln -sf "$tool" "$TOOLS/libexec/"; done
rm "$TOOLS/libexec/rcc"
printf '#!/bin/sh\nexec %s/rcc --no-zstd "$@"\n' "$HOST_QT" > "$TOOLS/libexec/rcc"
# cxx-qt asks qmake where Qt is; the macOS qmake cannot run here, so this one answers for it.
cat > "$TOOLS/qmake" << EOF
#!/bin/sh
case "\$2" in
  QT_VERSION) echo $VERSION ;;
  QT_HOST_LIBEXECS*|QT_INSTALL_LIBEXECS*) echo $TOOLS/libexec ;;
  QT_HOST_*|QT_INSTALL_BINS*) echo $HOST_QT/bin ;;
  QT_INSTALL_HEADERS) echo $QT/include ;;
  QT_INSTALL_LIBS) echo $QT/lib ;;
  QT_INSTALL_PLUGINS) echo $QT/plugins ;;
  QT_INSTALL_QML) echo $QT/qml ;;
  *) echo $QT ;;
esac
EOF
chmod +x "$TOOLS"/cc "$TOOLS"/cxx "$TOOLS"/qmake "$TOOLS"/libexec/rcc

export QMAKE=$TOOLS/qmake CC_aarch64_apple_darwin=$TOOLS/cc CXX_aarch64_apple_darwin=$TOOLS/cxx
export CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER=$TOOLS/cc CARGO_TARGET_DIR=$OUT/cargo
cargo build --release -p link-desk --target aarch64-apple-darwin --manifest-path "$ROOT/Cargo.toml"
QT_MAC=$QT "$ROOT/dist/macos/bundle.sh" "$OUT/cargo/aarch64-apple-darwin/release/andromeda-link" "$OUT/Andromeda Link.app"
