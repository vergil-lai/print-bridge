#[test]
fn server_packages_conflict_with_desktop_without_replacing_it() {
    let deb = include_str!("../packaging/deb/control");
    let rpm = include_str!("../packaging/rpm/print-bridge.spec");
    for metadata in [deb, rpm] {
        assert!(metadata.contains("Provides: print-bridge"));
        assert!(metadata.contains("Conflicts: print-bridge-desktop"));
        assert!(!metadata.contains("Replaces:"));
        assert!(!metadata.contains("Obsoletes:"));
    }
}

#[test]
fn desktop_packages_conflict_with_server_without_replacing_it() {
    let deb = include_str!("../../desktop/packaging/deb/control");
    let rpm = include_str!("../../desktop/packaging/rpm/metadata");
    for metadata in [deb, rpm] {
        assert!(metadata.contains("Provides: print-bridge"));
        assert!(metadata.contains("Conflicts: print-bridge-server"));
        assert!(!metadata.contains("Replaces:"));
        assert!(!metadata.contains("Obsoletes:"));
    }
}

#[test]
fn maintainer_scripts_preserve_data_except_on_purge() {
    let prerm = include_str!("../packaging/deb/prerm");
    let postrm = include_str!("../packaging/deb/postrm");
    assert!(prerm.contains("\"remove\""));
    assert!(postrm.contains("\"purge\""));
    assert!(!prerm.contains("rm -rf"));
}

#[test]
fn rpm_spec_does_not_require_distribution_systemd_macros() {
    let rpm = include_str!("../packaging/rpm/print-bridge.spec");

    for unsupported_macro in [
        "%{_unitdir}",
        "%systemd_post",
        "%systemd_preun",
        "%systemd_postun_with_restart",
    ] {
        assert!(
            !rpm.contains(unsupported_macro),
            "unexpected {unsupported_macro}"
        );
    }
    assert!(rpm.contains("/usr/lib/systemd/system/print-bridge.service"));
    assert!(rpm.contains("systemctl daemon-reload"));
}
