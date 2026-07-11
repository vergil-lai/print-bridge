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
install -D -m 0644 %{_sourcedir}/print-bridge.service %{buildroot}%{_unitdir}/print-bridge.service

%pre
getent group printbridge >/dev/null || groupadd -r printbridge
getent passwd printbridge >/dev/null || useradd -r -g printbridge -d /var/lib/print-bridge -s /sbin/nologin printbridge

%post
%systemd_post print-bridge.service

%preun
%systemd_preun print-bridge.service

%postun
%systemd_postun_with_restart print-bridge.service

%files
%{_bindir}/print-bridge
%{_unitdir}/print-bridge.service

%changelog
