Name: print-bridge-server
Version: __VERSION__
Release: 1%{?dist}
Summary: Headless PrintBridge Agent
License: Apache-2.0
Requires: systemd, cups-client, libreoffice
Provides: print-bridge
Conflicts: print-bridge-desktop

%description
Runs PrintBridge as the dedicated printbridge system user.

%install
install -D -m 0755 %{_sourcedir}/print-bridge %{buildroot}%{_bindir}/print-bridge
install -D -m 0644 %{_sourcedir}/print-bridge.service %{buildroot}/usr/lib/systemd/system/print-bridge.service

%pre
getent group printbridge >/dev/null || groupadd -r printbridge
getent passwd printbridge >/dev/null || useradd -r -g printbridge -d /var/lib/print-bridge -s /sbin/nologin printbridge

%post
systemctl daemon-reload >/dev/null 2>&1 || :
systemctl enable --now print-bridge.service >/dev/null 2>&1 || :

%preun
if [ "$1" -eq 0 ]; then
  systemctl disable --now print-bridge.service >/dev/null 2>&1 || :
fi

%postun
systemctl daemon-reload >/dev/null 2>&1 || :
if [ "$1" -ge 1 ]; then
  systemctl try-restart print-bridge.service >/dev/null 2>&1 || :
fi

%files
%{_bindir}/print-bridge
/usr/lib/systemd/system/print-bridge.service

%changelog
