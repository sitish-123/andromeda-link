#!/bin/bash
# Builds every Link package for a release into target/dist: the Android APK (signed when
# ~/.config/andromeda-link/signing.env exists), the macOS app zip, Ubuntu 24.04 .debs and Fedora RPMs (each built in
# its distrobox, ubuntu-link and fedora-link, so they link the target's libraries), and the Flatpak bundle of the app.
# Usage: release.sh [android macos deb rpm flatpak]; no argument builds them all.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=$ROOT/target/dist
VERSION="r$(git -C "$ROOT" rev-list --count HEAD).g$(git -C "$ROOT" rev-parse --short HEAD)"
# A build with uncommitted changes gets its time, so it never takes a committed build's name.
git -C "$ROOT" diff --quiet HEAD || VERSION+=".dirty$(date +%m%d%H%M)"
SIGNING=$HOME/.config/andromeda-link/signing.env
SERVICES=(-p link-daemon -p link-session -p link-mount -p link-mirror)
mkdir -p "$OUT"
cd "$ROOT"

android() {
  (
    [[ -f $SIGNING ]] && source "$SIGNING"
    cd apps/android && ./gradlew -q :app:assembleRelease
  )
  local apk=apps/android/app/build/outputs/apk/release
  if [[ -f $apk/app-release.apk ]]; then
    cp "$apk/app-release.apk" "$OUT/andromeda-link-$VERSION.apk"
  else
    echo "android: no $SIGNING, so the APK is unsigned and cannot be installed" >&2
    cp "$apk/app-release-unsigned.apk" "$OUT/andromeda-link-$VERSION-unsigned.apk"
  fi
}

macos() {
  tools/macos/build.sh
  rm -f "$OUT/Andromeda-Link-$VERSION-macos-arm64.zip"
  # -y keeps the frameworks' symlinks, which the app's signature covers.
  (cd target/macos && zip -qry "$OUT/Andromeda-Link-$VERSION-macos-arm64.zip" "Andromeda Link.app")
}

deb() {
  local target=$HOME/.cache/ubuntu-link/target
  # Ubuntu 24.04's Qt is too old for the app, so it builds against Qt for Linux from aqtinstall and the .deb carries it.
  local qt=${QT_LINUX:-$HOME/.cache/qt-linux/6.12.0/gcc_64}
  distrobox enter ubuntu-link -- bash -c "
    set -e
    export RUSTUP_HOME=$HOME/.local/share/ubuntu-link/rustup CARGO_HOME=$HOME/.local/share/ubuntu-link/cargo
    export PATH=$HOME/.local/share/ubuntu-link/cargo/bin:\$PATH CARGO_TARGET_DIR=$target
    cd '$ROOT'
    cargo build --release --locked ${SERVICES[*]}
    QMAKE=$qt/bin/qmake cargo rustc --release --locked -p link-desk --bin andromeda-link -- \
      -C link-arg=-Wl,-rpath,/usr/lib/andromeda-link/qt/lib
    QT_BUNDLE=$qt dist/deb/build-deb.sh $target/release '$OUT'"
}

rpm() {
  local target=$HOME/.cache/fedora-link/target
  distrobox enter fedora-link -- bash -c "
    export CARGO_TARGET_DIR=$target PATH=$HOME/.cargo/bin:\$PATH
    cd '$ROOT' && cargo build --release --locked ${SERVICES[*]} -p link-desk &&
    rpmbuild -bb dist/rpm/andromeda-link.spec --define 'linkroot $ROOT' --define 'linkbin $target/release' \
      --define 'linkversion 0~$VERSION' --define '_rpmdir $OUT'"
}

flatpak() {
  local build=$HOME/.cache/link-flatpak
  python3 tools/flatpak-cargo-sources.py Cargo.lock dist/flatpak/cargo-sources.json
  command flatpak run org.flatpak.Builder --user --force-clean --disable-rofiles-fuse --repo="$build/repo" \
    --state-dir="$build/state" "$build/build" dist/flatpak/org.andromeda.Link.yml
  command flatpak build-bundle "$build/repo" "$OUT/andromeda-link-$VERSION.flatpak" org.andromeda.Link \
    --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo
}

for kind in "${@:-android macos deb rpm flatpak}"; do
  for step in $kind; do
    echo "== $step"
    "$step"
  done
done
ls -la "$OUT"
