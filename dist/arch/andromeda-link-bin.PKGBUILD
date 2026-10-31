# The AUR's andromeda-link-bin: the Arch package from a GitHub release, repackaged. The release workflow fills in
# @VERSION@ and @SHA256@ and pushes it to the AUR.
pkgname=andromeda-link-bin
pkgver=@VERSION@
pkgrel=1
pkgdesc='Andromeda Link: your Android phone and your Linux desktop as one (release build)'
arch=(x86_64)
url='https://github.com/sitish-123/andromeda-link'
license=(MIT)
depends=(glibc gcc-libs dbus fuse3 gtk4 qt6-base qt6-declarative qt6-multimedia gstreamer gst-plugins-base-libs
  gst-plugins-bad gst-plugin-gtk4)
optdepends=('bluez: Link over Bluetooth when no Wi-Fi reaches the phone'
  "networkmanager: joining the phone's hotspot or Wi-Fi Direct group for large transfers away from Wi-Fi"
  'gst-plugin-va: hardware decoding of the mirrored phone screen'
  'gst-libav: software decoding of the mirrored phone screen without VA-API'
  'scrcpy: phone apps in their own windows (andromeda-link-apps)'
  'android-tools: adb for andromeda-link-apps')
provides=(andromeda-link)
conflicts=(andromeda-link andromeda-link-git)
backup=(etc/ufw/applications.d/andromeda-link)
install=andromeda-link.install
source=("$url/releases/download/v$pkgver/andromeda-link-$pkgver-1-x86_64.pkg.tar.zst"
  "andromeda-link.install")
noextract=("andromeda-link-$pkgver-1-x86_64.pkg.tar.zst")
sha256sums=('@SHA256@' '@INSTALL_SHA256@')

package() {
  bsdtar -xf "andromeda-link-$pkgver-1-x86_64.pkg.tar.zst" -C "$pkgdir" usr etc
}
