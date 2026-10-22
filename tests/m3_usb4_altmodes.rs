//! USB4 and alternate-mode fixture coverage.
#![cfg(feature = "sysfs")]

#[test]
fn absent_subsystems_have_an_explicit_report() {
    let root = TempRoot::new("m3-absent");
    let mut mgr = DeviceManager::with_sysfs(Sysfs::with_root(root.path()));
    mgr.refresh();
    let report = usbeehive::altmode::analyze(mgr.snapshot());
    assert_eq!(
        report.usb4.thunderbolt_bus,
        usbeehive::altmode::BusState::Absent
    );
    assert!(report.to_text().contains("not present"));
}
#[test]
fn linux_hex_routes_nest_and_missing_generation_is_retained() {
    let root = TempRoot::new("m3-routes");
    write_thunderbolt_router(root.path(), "0-0", 4, &[]);
    write_thunderbolt_router(root.path(), "0-1", 4, &[]);
    write_attr(
        &root.path().join("bus/thunderbolt/devices/0-301"),
        "device_name",
        "Dock",
    );
    write_attr(
        &root.path().join("bus/thunderbolt/devices/0-301"),
        "vendor",
        "0x8086",
    );
    write_attr(
        &root.path().join("bus/thunderbolt/devices/0-301"),
        "device",
        "0x1234",
    );
    // Retimer names (ABI form `<device>:<port>.<index>`) and invalid routes
    // must not become routers.
    write_thunderbolt_router(root.path(), "0-1:1.1", 4, &[]);
    write_thunderbolt_router(root.path(), "0-xyz", 4, &[]);
    let mut mgr = DeviceManager::with_sysfs(Sysfs::with_root(root.path()));
    mgr.refresh();
    assert_eq!(mgr.snapshot().thunderbolt_routers.len(), 3);
    let report = usbeehive::altmode::analyze(mgr.snapshot());
    let dock = &report.usb4.topology[0].children[0].children[0];
    assert_eq!(dock.route, "0-301");
    assert_eq!(dock.depth, 2);
    assert!(report.to_text().contains("generation unavailable"));
    let raw = mgr
        .snapshot()
        .thunderbolt_routers
        .iter()
        .find(|r| r.route == "0-301")
        .unwrap();
    assert_eq!(raw.generation, 0); // legacy numeric sentinel remains compatible
    assert_eq!(
        serde_json::to_value(raw).unwrap()["reported_generation"],
        serde_json::Value::Null
    );
}

#[test]
fn dp_facts_reach_summaries_without_guessing_usb4_port() {
    let root = TempRoot::new("m3-dp");
    write_typec_port(root.path(), "port0", &[]);
    write_typec_partner(root.path(), "port0", "downstream", &[]);
    write_attr(
        &root.path().join("class/typec/port0-partner.0"),
        "svid",
        "ff01",
    );
    write_attr(
        &root.path().join("class/typec/port0-partner.0"),
        "mode",
        "1",
    );
    write_attr(
        &root.path().join("class/typec/port0-partner.0"),
        "active",
        "yes",
    );
    write_attr(
        &root.path().join("class/typec/port0-partner.0"),
        "vdo",
        "0x00000c46",
    );
    write_thunderbolt_router(root.path(), "0-0", 4, &[]);
    write_thunderbolt_router(root.path(), "0-1", 4, &[]);
    let mut mgr = DeviceManager::with_sysfs(Sysfs::with_root(root.path()));
    mgr.refresh();
    let summary = &mgr.snapshot().summaries[0];
    assert!(summary
        .properties
        .iter()
        .any(|(k, v)| k == "altmode.dp" && v == "active"));
    assert!(!summary
        .properties
        .iter()
        .any(|(k, _)| k == "transport.usb4"));
    let report = usbeehive::altmode::analyze(mgr.snapshot());
    assert_eq!(report.usb4.usb4_link_active, Some(true));
    assert!(report.to_text().contains("DisplayPort"));
    assert!(report.to_text().contains("C,D"));
}

#[test]
fn incomplete_routers_do_not_claim_a_negative_usb4_link() {
    let root = TempRoot::new("m3-unknown");
    write_thunderbolt_router(root.path(), "0-0", 4, &[]);
    write_attr(
        &root.path().join("bus/thunderbolt/devices/0-1"),
        "rx_speed",
        "20garbage",
    );
    write_attr(
        &root.path().join("bus/thunderbolt/devices/0-1"),
        "vendor",
        "0x8086",
    );
    write_attr(
        &root.path().join("bus/thunderbolt/devices/0-1"),
        "device",
        "0x1234",
    );
    let mut mgr = DeviceManager::with_sysfs(Sysfs::with_root(root.path()));
    mgr.refresh();
    let report = usbeehive::altmode::analyze(mgr.snapshot());
    assert_eq!(report.usb4.usb4_link_active, None);
    assert_eq!(report.usb4.topology[0].children[0].rx_speed_gbps, None);
}

#[test]
fn nested_partner_modes_are_read_and_missing_vdo_stays_null() {
    let root = TempRoot::new("m3-nested");
    write_typec_port(root.path(), "port0", &[]);
    write_typec_partner(root.path(), "port0", "downstream", &[]);
    let nested = root
        .path()
        .join("class/typec/port0-partner/port0-partner.0");
    write_attr(&nested, "svid", "ff01");
    write_attr(&nested, "mode", "1");
    let mut mgr = DeviceManager::with_sysfs(Sysfs::with_root(root.path()));
    mgr.refresh();
    let modes = &mgr.snapshot().typec_ports[0]
        .partner
        .as_ref()
        .unwrap()
        .altmodes;
    assert_eq!(modes.len(), 1);
    assert_eq!(modes[0].vdo, None);
    let value = serde_json::to_value(&modes[0]).unwrap();
    assert!(value["vdo"].is_null());
}

#[test]
fn reported_dp_pin_assignment_and_activity_preserve_absence() {
    let root = TempRoot::new("m3-pins");
    write_typec_port(root.path(), "port0", &[]);
    write_typec_partner(root.path(), "port0", "downstream", &[]);
    let dir = write_typec_altmode(root.path(), "port0", 0, &[("svid", "ff01"), ("mode", "1")]);
    write_attr(&dir.join("displayport"), "pin_assignment", "C [D]");
    let mut mgr = DeviceManager::with_sysfs(Sysfs::with_root(root.path()));
    mgr.refresh();
    let modes = &mgr.snapshot().typec_ports[0]
        .partner
        .as_ref()
        .unwrap()
        .altmodes;
    let value = serde_json::to_value(&modes[0]).unwrap();
    assert_eq!(value["pin_assignment"], "D");
    assert!(value["reported_active"].is_null());
    let report = usbeehive::altmode::analyze(mgr.snapshot());
    assert!(report.to_text().contains("active state unavailable"));
    assert!(report.to_text().contains("configured pin D"));
}

#[test]
fn billboard_descriptor_failures_reach_structured_and_text_reports() {
    let root = TempRoot::new("m3-billboard");
    let dir = root.path().join("bus/usb/devices/1-1");
    write_attr(&dir, "idVendor", "1234");
    write_attr(&dir, "idProduct", "5678");
    write_billboard(root.path(), "1-1", 0xff01, 2, 1);
    let mut mgr = DeviceManager::with_sysfs(Sysfs::with_root(root.path()));
    mgr.refresh();
    let report = usbeehive::altmode::analyze(mgr.snapshot());
    assert_eq!(report.billboards.len(), 1);
    assert_eq!(report.billboards[0].svids, vec![0xff01]);
    assert_eq!(report.billboards[0].failure_reasons, vec!["no_usb_pd"]);
    assert!(report.to_text().contains("Unsuccessful"));
    let restored: usbeehive::AltModeReport =
        serde_json::from_str(&serde_json::to_string(&report).unwrap()).unwrap();
    assert_eq!(restored, report);
}

mod fixture_builder;
use fixture_builder::*;
use usbeehive::{DeviceManager, Sysfs};

#[test]
fn snapshot_captures_domain_and_source_presence() {
    let root = TempRoot::new("m3-domain");
    write_attr(
        &root.path().join("bus/thunderbolt/devices/domain0"),
        "security",
        "user",
    );
    let mut mgr = DeviceManager::with_sysfs(Sysfs::with_root(root.path()));
    mgr.refresh();
    let snap = mgr.snapshot();
    assert_eq!(snap.thunderbolt_domains.len(), 1);
    assert_eq!(
        snap.thunderbolt_domains[0].security.as_deref(),
        Some("user")
    );
    let availability = snap.availability.unwrap();
    assert!(availability.thunderbolt_bus);
    assert!(!availability.typec_class);
}
