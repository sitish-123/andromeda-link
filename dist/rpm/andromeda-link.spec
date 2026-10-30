# Packages release binaries already built on the target Fedora, with the same files as dist/deb/build-deb.sh.
# Build: rpmbuild -bb dist/rpm/andromeda-link.spec --define "linkroot $PWD" --define "linkbin $CARGO_TARGET_DIR/release" \
#   --define "linkversion 0~r$(git rev-list --count HEAD).g$(git rev-parse --short HEAD)"
Name:           andromeda-link
Version:        %{linkversion}
Release:        1%{?dist}
Summary:        Andromeda Link: phone and desktop continuity
License:        MIT
Requires:       dbus fuse3 gtk4 gstreamer1-plugins-base gstreamer1-plugins-bad-free
Recommends:     andromeda-link-app gstreamer1-plugin-gtk4 (xdg-desktop-portal-gnome or xdg-desktop-portal-kde)
Suggests:       bluez NetworkManager scrcpy android-tools
%global debug_package %{nil}

%description
Clipboard, notifications, media, files, texts, and screen mirroring between an Android phone and the desktop.

%package app
Summary:        Andromeda Link app
Requires:       andromeda-link = %{version}-%{release} qt6-qtbase >= 6.7 qt6-qtdeclarative qt6-qtmultimedia

%description app
The desktop app for Andromeda Link: pairing, the phone's media, texts, photos, files, and screen.

%install
for b in andromeda-linkd andromeda-link-mount andromeda-link-mirror andromeda-link-apps andromeda-link-session andromeda-link; do
  install -Dm755 %{linkbin}/$b %{buildroot}%{_bindir}/$b
done
for s in andromeda-linkd andromeda-link-mount andromeda-link-session; do
  install -Dm644 %{linkroot}/dist/$s.service %{buildroot}%{_userunitdir}/$s.service
  install -d %{buildroot}%{_userunitdir}/graphical-session.target.wants
  ln -s ../$s.service %{buildroot}%{_userunitdir}/graphical-session.target.wants/$s.service
done
install -Dm644 %{linkroot}/dist/org.andromeda.Link1.service %{buildroot}%{_datadir}/dbus-1/services/org.andromeda.Link1.service
install -Dm644 %{linkroot}/api/org.andromeda.Link1.xml %{buildroot}%{_datadir}/dbus-1/interfaces/org.andromeda.Link1.xml
install -Dm644 %{linkroot}/dist/andromeda-link.ufw %{buildroot}%{_sysconfdir}/ufw/applications.d/andromeda-link
install -Dm644 %{linkroot}/dist/andromeda-link.firewalld.xml %{buildroot}%{_prefix}/lib/firewalld/services/andromeda-link.xml
install -Dm644 %{linkroot}/dist/andromeda-link-localsend.firewalld.xml \
  %{buildroot}%{_prefix}/lib/firewalld/services/andromeda-link-localsend.xml
install -Dm644 %{linkroot}/dist/60-andromeda-link-bluetooth.conf \
  %{buildroot}%{_datadir}/wireplumber/wireplumber.conf.d/60-andromeda-link-bluetooth.conf
install -Dm644 %{linkroot}/dist/org.andromeda.Link.desktop %{buildroot}%{_datadir}/applications/org.andromeda.Link.desktop
cp -r %{linkroot}/dist/icons %{buildroot}%{_datadir}/
install -Dm644 %{linkroot}/LICENSE %{buildroot}%{_licensedir}/%{name}/LICENSE

# Phones reach the daemon only through the firewall; open whichever of firewalld and ufw runs.
%post
if command -v firewall-cmd > /dev/null && firewall-cmd --state > /dev/null 2>&1; then
  firewall-cmd --reload > /dev/null 2>&1 || :
  firewall-cmd --permanent --add-service=andromeda-link > /dev/null 2>&1 && firewall-cmd --reload > /dev/null 2>&1 ||
    echo "firewalld: phones cannot pair until you run: sudo firewall-cmd --permanent --add-service=andromeda-link"
fi
if command -v ufw > /dev/null && ufw status 2> /dev/null | grep -q '^Status: active'; then
  ufw allow 'Andromeda Link' > /dev/null 2>&1 || echo "ufw: phones cannot pair until you run: sudo ufw allow 'Andromeda Link'"
fi
:

%files
%license %{_licensedir}/%{name}/LICENSE
%{_bindir}/andromeda-linkd
%{_bindir}/andromeda-link-mount
%{_bindir}/andromeda-link-mirror
%{_bindir}/andromeda-link-apps
%{_bindir}/andromeda-link-session
%{_userunitdir}/*.service
%{_userunitdir}/graphical-session.target.wants/*.service
%{_datadir}/dbus-1/services/org.andromeda.Link1.service
%{_datadir}/dbus-1/interfaces/org.andromeda.Link1.xml
%config(noreplace) %{_sysconfdir}/ufw/applications.d/andromeda-link
%{_prefix}/lib/firewalld/services/andromeda-link*.xml
%{_datadir}/wireplumber/wireplumber.conf.d/60-andromeda-link-bluetooth.conf

%files app
%{_bindir}/andromeda-link
%{_datadir}/applications/org.andromeda.Link.desktop
%{_datadir}/icons/hicolor/*/apps/org.andromeda.Link*
