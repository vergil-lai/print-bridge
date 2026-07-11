#[test]
fn unit_runs_as_printbridge_and_uses_notify() {
    let unit = include_str!("../packaging/systemd/print-bridge.service");
    assert!(unit.contains("User=printbridge"));
    assert!(unit.contains("Group=printbridge"));
    assert!(unit.contains("Type=notify"));
    assert!(unit.contains("RuntimeDirectory=print-bridge"));
    assert!(unit.contains("ExecStart=/usr/bin/print-bridge serve"));
}
