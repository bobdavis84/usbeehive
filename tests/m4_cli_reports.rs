//! CLI reporting contract tests.
#![cfg(feature = "cli")]
#[test]
fn summary_json_has_rollups() {
    let o = run(&[
        "--sysfs-root",
        "/nonexistent-usbeehive-fixture",
        "--summary",
        "--json",
    ]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["total"], 0);
    assert!(v["countsByType"].is_object());
    assert!(v["ports"].is_array());
}
#[test]
fn snapshot_save_and_diff_roundtrip() {
    let p = std::env::temp_dir().join(format!("usbeehive-m4-{}.json", std::process::id()));
    let o = run(&[
        "--sysfs-root",
        "/nonexistent-usbeehive-fixture",
        "--save-snapshot",
        p.to_str().unwrap(),
    ]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let o = run(&["--json", "diff", p.to_str().unwrap(), p.to_str().unwrap()]);
    std::fs::remove_file(p).unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["added"], serde_json::json!([]));
}
#[test]
fn csv_export_has_header() {
    let o = run(&[
        "--sysfs-root",
        "/nonexistent-usbeehive-fixture",
        "--format",
        "csv",
    ]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(String::from_utf8_lossy(&o.stdout).starts_with("id,headline,severity"));
}
#[test]
fn bounded_history_json_is_inspectable() {
    let o = run(&[
        "--sysfs-root",
        "/nonexistent-usbeehive-fixture",
        "--history",
        "--samples",
        "2",
        "--interval-ms",
        "1",
        "--json",
    ]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let lines: Vec<_> = o
        .stdout
        .split(|b| *b == b'\n')
        .filter(|s| !s.is_empty())
        .collect();
    assert_eq!(lines.len(), 2);
    let v: serde_json::Value = serde_json::from_slice(lines[1]).unwrap();
    assert_eq!(v["sequence"], 1);
    assert!(v["transitions"].is_array());
}
#[test]
fn raw_only_snapshot_is_rejected() {
    let p = std::env::temp_dir().join(format!("m4-invalid-{}.json", std::process::id()));
    let raw = usbeehive::Snapshot {
        usb_devices: vec![Default::default()],
        ..Default::default()
    };
    std::fs::write(&p, serde_json::to_vec(&raw).unwrap()).unwrap();
    let o = run(&["diff", p.to_str().unwrap(), p.to_str().unwrap()]);
    std::fs::remove_file(p).unwrap();
    assert!(!o.status.success());
    assert!(o.stdout.is_empty());
}
#[test]
fn summary_text_is_readable() {
    let o = run(&[
        "--sysfs-root",
        "/nonexistent-usbeehive-fixture",
        "--summary",
    ]);
    assert!(o.status.success());
    assert!(String::from_utf8_lossy(&o.stdout).contains("Devices: 0"));
}

#[test]
fn altmode_report_is_available_in_text_and_json() {
    let text = run(&[
        "--sysfs-root",
        "/nonexistent-usbeehive-fixture",
        "--altmodes",
    ]);
    assert!(
        text.status.success(),
        "{}",
        String::from_utf8_lossy(&text.stderr)
    );
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("Alternate modes"));
    assert!(text.contains("subsystem not present"));

    let json = run(&[
        "--sysfs-root",
        "/nonexistent-usbeehive-fixture",
        "--altmodes",
        "--json",
    ]);
    assert!(
        json.status.success(),
        "{}",
        String::from_utf8_lossy(&json.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(report["usb4"]["thunderbolt_bus"], "absent");
    assert!(report["caveats"].is_array());
}

#[test]
fn history_rejects_snapshot_save_without_stdout() {
    let p = std::env::temp_dir().join(format!("m4-history-save-{}.json", std::process::id()));
    let o = run(&[
        "--sysfs-root",
        "/nonexistent-usbeehive-fixture",
        "--history",
        "--samples",
        "1",
        "--save-snapshot",
        p.to_str().unwrap(),
    ]);
    assert!(
        !o.status.success(),
        "history silently accepted snapshot save"
    );
    assert!(o.stdout.is_empty());
    assert!(!p.exists());
}

#[test]
fn diff_rejects_history_and_altmodes_without_stdout() {
    let p = std::env::temp_dir().join(format!("m4-diff-modes-{}.json", std::process::id()));
    std::fs::write(
        &p,
        serde_json::to_vec(&usbeehive::Snapshot::default()).unwrap(),
    )
    .unwrap();
    for mode in [vec!["--altmodes"], vec!["--history", "--samples", "1"]] {
        let mut args = vec!["--sysfs-root", "/nonexistent-usbeehive-fixture"];
        args.extend(mode);
        args.extend(["diff", p.to_str().unwrap(), "live"]);
        let o = run(&args);
        assert!(!o.status.success(), "accepted {args:?}");
        assert!(o.stdout.is_empty(), "emitted output for {args:?}");
    }
    std::fs::remove_file(p).unwrap();
}

#[test]
fn incompatible_modes_fail_without_stdout() {
    for args in [
        vec!["--watch", "--summary"],
        vec!["--watch", "--class", "storage"],
        vec!["--format", "csv", "diff", "missing"],
        vec!["--port", "-1"],
    ] {
        let o = run(&args);
        assert!(!o.status.success(), "accepted {args:?}");
        assert!(o.stdout.is_empty());
    }
}
#[test]
fn unsupported_snapshot_shapes_fail_without_stdout() {
    let p = std::env::temp_dir().join(format!("m4-shapes-{}.json", std::process::id()));
    for text in [
        "{}",
        r#"{"summaries":[],"usbDevices":[{}]}"#,
        "[]",
        "null",
        r#"{"summaries":[{"vendorId":"1234","headline":"Legacy"}]}"#,
        "not json",
    ] {
        std::fs::write(&p, text).unwrap();
        let o = run(&["diff", p.to_str().unwrap(), p.to_str().unwrap()]);
        assert!(!o.status.success(), "accepted {text}");
        assert!(o.stdout.is_empty());
    }
    std::fs::remove_file(p).unwrap();
    let o = run(&[
        "--save-snapshot",
        "/nonexistent-usbeehive-dir/snapshot.json",
    ]);
    assert!(!o.status.success());
    assert!(o.stdout.is_empty());
}
#[test]
fn diff_reports_link_change_in_text_and_json() {
    let a = std::env::temp_dir().join(format!("m4-a-{}.json", std::process::id()));
    let b = std::env::temp_dir().join(format!("m4-b-{}.json", std::process::id()));
    let mut d = usbeehive::usb::UsbDevice {
        bus_port: "1-2".into(),
        speed: 480,
        ..Default::default()
    };
    let before =
        usbeehive::Snapshot::from(vec![usbeehive::summary::DeviceSummary::from_usb_device(&d)]);
    d.speed = 5000;
    let after =
        usbeehive::Snapshot::from(vec![usbeehive::summary::DeviceSummary::from_usb_device(&d)]);
    std::fs::write(&a, serde_json::to_vec(&before).unwrap()).unwrap();
    std::fs::write(&b, serde_json::to_vec(&after).unwrap()).unwrap();
    let o = run(&["diff", a.to_str().unwrap(), b.to_str().unwrap()]);
    assert!(o.status.success());
    assert!(String::from_utf8_lossy(&o.stdout).contains("480 Mbps -> 5000 Mbps"));
    let o = run(&["diff", a.to_str().unwrap(), b.to_str().unwrap(), "--json"]);
    assert!(o.status.success());
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["modified"][0]["link_speed_mbps"]["to"], 5000);
    std::fs::remove_file(a).unwrap();
    std::fs::remove_file(b).unwrap();
}
#[test]
fn history_observes_live_fixture_link_transition() {
    use std::io::{BufRead, BufReader};
    let root = std::env::temp_dir().join(format!("m4-history-{}", std::process::id()));
    let usb = root.join("bus/usb/devices/1-1");
    std::fs::create_dir_all(&usb).unwrap();
    for (key, value) in [
        ("idVendor", "1234"),
        ("idProduct", "5678"),
        ("speed", "480"),
        ("bDeviceClass", "00"),
        ("product", "Fixture"),
    ] {
        std::fs::write(usb.join(key), value).unwrap();
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_usbeehive"))
        .args([
            "--sysfs-root",
            root.to_str().unwrap(),
            "--watch",
            "--history",
            "--samples",
            "2",
            "--interval-ms",
            "500",
            "--json",
        ])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let baseline: serde_json::Value =
        serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    assert_eq!(baseline["baseline"], true);
    std::fs::write(usb.join("speed"), "5000").unwrap();
    let event = lines.next().unwrap().unwrap();
    assert!(child.wait().unwrap().success());
    std::fs::remove_dir_all(root).unwrap();
    assert!(event.contains("LinkSpeedChanged"), "{event}");
}
use std::process::Command;
fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_usbeehive"))
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn filtered_json_keeps_array_contract() {
    let o = run(&[
        "--sysfs-root",
        "/nonexistent-usbeehive-fixture",
        "--class",
        "storage",
        "--json",
    ]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&o.stdout).unwrap(),
        serde_json::json!([])
    );
}
