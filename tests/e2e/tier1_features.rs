//! Tier 1: Feature Coverage (Happy Path, >=5 tests per feature area R1-R7).

use crate::e2e::harness::*;
use crate::fixture_builder::*;
use usbeehive::diagnostic::Bottleneck;
use usbeehive::summary::{Category, DeviceClass};
use usbeehive::thunderbolt::system_has_usb4_link;

// =========================================================================
// Area 1: JSON Output Schema (R1)
// =========================================================================

#[test]
fn t1_r1_json_empty_tree_returns_empty_array() {
    let root = TempRoot::new("t1-r1-empty");
    let json = run_cli_json(root.path(), &[]);
    assert!(json.is_array(), "Expected JSON array on empty sysfs");
    assert_eq!(json.as_array().unwrap().len(), 0);
}

#[test]
fn t1_r1_json_usb_device_fields() {
    let root = TempRoot::new("t1-r1-usb-fields");
    write_standard_keyboard(root.path(), "1-1");

    let json = run_cli_json(root.path(), &[]);
    let arr = json.as_array().expect("array of devices");
    assert_eq!(arr.len(), 1);

    let dev = &arr[0];
    assert_eq!(dev["category"], "usb");
    assert_eq!(dev["headline"], "USB Keyboard");
    assert_eq!(dev["usb"]["vendorId"], "0x046d");
    assert_eq!(dev["usb"]["productId"], "0xc31c");
    assert_eq!(dev["usb"]["speed"], 12);
    assert_eq!(dev["usb"]["maxPowerMA"], 100);
}

#[test]
fn t1_r1_json_typec_charging_output() {
    let root = TempRoot::new("t1-r1-typec-charging");
    write_standard_typec_port(root.path(), "port0", "host [device]", "source [sink]", true);
    write_standard_partner(root.path(), "port0", 0x05ac, 0x1000);
    write_standard_cable(root.path(), "port0", 100, false, 0x05ac, false);
    write_standard_pd_charger(root.path(), "pd0", 0, 100, 100);
    link_partner_pd(root.path(), "port0", "pd0");

    let json = run_cli_json(root.path(), &[]);
    let arr = json.as_array().expect("array of devices");
    assert!(!arr.is_empty(), "Type-C port must be present");

    let port = arr
        .iter()
        .find(|d| d["category"] == "typec")
        .expect("typec category entry");
    assert_eq!(port["status"], "Charging");
    assert!(port["charging"].is_object());
    assert_eq!(port["charging"]["isWarning"], false);
}

#[test]
fn t1_r1_json_raw_flag_includes_sysfs_attrs() {
    let root = TempRoot::new("t1-r1-json-raw");
    write_standard_keyboard(root.path(), "1-1");

    let json = run_cli_json(root.path(), &["--raw"]);
    let dev = &json.as_array().unwrap()[0];
    assert!(
        dev["usb"]["raw"].is_object(),
        "Raw attributes must be populated when --raw is passed"
    );
    assert_eq!(dev["usb"]["raw"]["idVendor"], "046d");
    assert_eq!(dev["usb"]["raw"]["idProduct"], "c31c");
}

#[test]
fn t1_r1_json_multi_device_array() {
    let root = TempRoot::new("t1-r1-multi");
    write_standard_keyboard(root.path(), "1-1");
    write_standard_mouse(root.path(), "1-2");
    write_standard_storage(root.path(), "2-1", 5000);

    let json = run_cli_json(root.path(), &[]);
    let arr = json.as_array().unwrap();
    assert_eq!(arr.len(), 3);

    let headlines: Vec<&str> = arr.iter().filter_map(|d| d["headline"].as_str()).collect();
    assert!(headlines.contains(&"USB Keyboard"));
    assert!(headlines.contains(&"Optical USB Mouse"));
    assert!(headlines.contains(&"Ultra USB 3.0"));
}

// =========================================================================
// Area 2: Diagnostic Reasoning & Actionability (R2)
// =========================================================================

#[test]
fn t1_r2_cable_limit_bottleneck_warning() {
    let root = TempRoot::new("t1-r2-cable-limit");
    write_standard_typec_port(root.path(), "port0", "host [device]", "[source] sink", true);
    write_standard_partner(root.path(), "port0", 0x05ac, 0x1000);
    // 60W cable with 100W charger
    write_standard_cable(root.path(), "port0", 60, false, 0x0bda, false);
    write_standard_pd_charger(root.path(), "pd0", 0, 100, 100);
    link_partner_pd(root.path(), "port0", "pd0");

    let snap = snapshot_for(root.path());
    let s = &snap.summaries[0];
    let diag = s.charging_diag.as_ref().expect("charging diagnostic");

    assert_eq!(diag.bottleneck, Bottleneck::CableLimit);
    assert!(diag.is_warning);
    assert!(diag
        .summary
        .contains("Cable rating is below available source capability"));
    assert_eq!(
        diag.confidence,
        usbeehive::diagnostic::DiagnosticConfidence::Low
    );
    assert!(diag
        .detail
        .contains("no active PDO or sink demand was reported"));
    assert!(diag.detail.contains("60W"));
    assert!(diag.detail.contains("100W"));
}

#[test]
fn t1_r2_fine_unconstrained_charging() {
    let root = TempRoot::new("t1-r2-fine");
    write_standard_typec_port(root.path(), "port0", "host [device]", "source [sink]", true);
    write_standard_partner(root.path(), "port0", 0x05ac, 0x1000);
    // 240W EPR cable with 100W charger
    write_standard_cable(root.path(), "port0", 240, true, 0x05ac, false);
    write_standard_pd_charger(root.path(), "pd0", 0, 100, 100);
    link_partner_pd(root.path(), "port0", "pd0");

    let snap = snapshot_for(root.path());
    let diag = snap.summaries[0].charging_diag.as_ref().unwrap();

    assert_eq!(diag.bottleneck, Bottleneck::Fine);
    assert!(!diag.is_warning);
    assert!(diag.summary.contains("100W"));
}

#[test]
fn t1_r2_device_limit_policy() {
    // Charger offers 100W, but active PDO is 15W (<80% of charger max)
    let port = usbeehive::power::PowerDeliveryPort {
        source_capabilities: vec![
            usbeehive::power::PowerDataObject {
                r#type: usbeehive::power::PdoType::FixedSupply,
                power_mw: 100_000,
                is_active: false,
                ..Default::default()
            },
            usbeehive::power::PowerDataObject {
                r#type: usbeehive::power::PdoType::FixedSupply,
                power_mw: 15_000,
                is_active: true,
                ..Default::default()
            },
        ],
        max_source_power_mw: 100_000,
        ..Default::default()
    };

    let diag = usbeehive::diagnostic::ChargingDiagnostic::evaluate(&port, None, 0, 0).unwrap();
    assert_eq!(diag.bottleneck, Bottleneck::DeviceLimit);
    assert!(!diag.is_warning);
    assert!(diag.detail.contains("battery charge limit or thermal"));
}

#[test]
fn t1_r2_sink_limit_policy() {
    // 65W contract, but sink requests 15W draw ceiling
    let port = usbeehive::power::PowerDeliveryPort {
        source_capabilities: vec![usbeehive::power::PowerDataObject {
            r#type: usbeehive::power::PdoType::FixedSupply,
            power_mw: 65_000,
            is_active: true,
            ..Default::default()
        }],
        max_source_power_mw: 65_000,
        ..Default::default()
    };

    let diag = usbeehive::diagnostic::ChargingDiagnostic::evaluate(&port, None, 15_000, 0).unwrap();
    assert_eq!(diag.bottleneck, Bottleneck::SinkLimit);
    assert!(!diag.is_warning);
    assert!(diag.summary.contains("15W"));
    assert!(diag.detail.contains("65W"));
}

#[test]
fn t1_r2_cable_no_emarker_pinned_3a() {
    // 100W (5A) PDO, sink requested 3000mA, no cable e-marker visible
    let port = usbeehive::power::PowerDeliveryPort {
        source_capabilities: vec![usbeehive::power::PowerDataObject {
            r#type: usbeehive::power::PdoType::FixedSupply,
            voltage_mv: 20_000,
            current_ma: 5_000,
            power_mw: 100_000,
            is_active: true,
            ..Default::default()
        }],
        max_source_power_mw: 100_000,
        ..Default::default()
    };

    let diag =
        usbeehive::diagnostic::ChargingDiagnostic::evaluate(&port, None, 60_000, 3_000).unwrap();
    assert_eq!(diag.bottleneck, Bottleneck::CableNoEMarker);
    assert!(diag.is_warning);
    assert!(diag.summary.contains("3A"));
    assert!(diag.detail.contains("5.0A"));
}

// =========================================================================
// Area 3: Historical Snapshot & State-Transition Tracking (R3)
// =========================================================================

#[test]
fn t1_r3_sequential_snapshots_capture() {
    let root = TempRoot::new("t1-r3-seq");

    // State 1: only keyboard
    write_standard_keyboard(root.path(), "1-1");
    let snap1 = snapshot_for(root.path());
    assert_eq!(snap1.usb_devices.len(), 1);

    // State 2: plug in mouse
    write_standard_mouse(root.path(), "1-2");
    let snap2 = snapshot_for(root.path());
    assert_eq!(snap2.usb_devices.len(), 2);

    // Diff between sequential snapshots
    let diff = snap2.diff(&snap1);
    assert_eq!(diff.added.len(), 1);
    assert!(diff.added[0].contains("1-2"));
}

#[test]
fn t1_r3_device_arrival_transition() {
    let root1 = TempRoot::new("t1-r3-arr-1");
    let snap1 = snapshot_for(root1.path());

    let root2 = TempRoot::new("t1-r3-arr-2");
    write_standard_storage(root2.path(), "2-1", 5000);
    let snap2 = snapshot_for(root2.path());

    let diff = snap2.diff(&snap1);
    assert_eq!(diff.added.len(), 1);
    assert!(diff.removed.is_empty());
    assert!(diff.added[0].contains("2-1"));
}

#[test]
fn t1_r3_device_removal_transition() {
    let root1 = TempRoot::new("t1-r3-rem-1");
    write_standard_keyboard(root1.path(), "1-1");
    let snap1 = snapshot_for(root1.path());

    let root2 = TempRoot::new("t1-r3-rem-2");
    let snap2 = snapshot_for(root2.path());

    let diff = snap2.diff(&snap1);
    assert!(diff.added.is_empty());
    assert_eq!(diff.removed.len(), 1);
    assert!(diff.removed[0].contains("1-1"));
}

#[test]
fn t1_r3_cable_degradation_transition() {
    // Port originally with 100W cable (fine), then swapped to 60W cable (degraded)
    let root1 = TempRoot::new("t1-r3-deg-1");
    write_standard_typec_port(
        root1.path(),
        "port0",
        "host [device]",
        "source [sink]",
        true,
    );
    write_standard_partner(root1.path(), "port0", 0x05ac, 0x1000);
    write_standard_cable(root1.path(), "port0", 100, false, 0x05ac, false);
    write_standard_pd_charger(root1.path(), "pd0", 0, 100, 100);
    link_partner_pd(root1.path(), "port0", "pd0");
    let snap1 = snapshot_for(root1.path());

    let root2 = TempRoot::new("t1-r3-deg-2");
    write_standard_typec_port(
        root2.path(),
        "port0",
        "host [device]",
        "source [sink]",
        true,
    );
    write_standard_partner(root2.path(), "port0", 0x05ac, 0x1000);
    write_standard_cable(root2.path(), "port0", 60, false, 0x0bda, false); // 60W cable
    write_standard_pd_charger(root2.path(), "pd0", 0, 100, 100);
    link_partner_pd(root2.path(), "port0", "pd0");
    let snap2 = snapshot_for(root2.path());

    let diff = snap2.diff(&snap1);
    assert_eq!(diff.newly_degraded, vec![0]);
    assert!(diff.resolved.is_empty());
}

#[test]
fn t1_r3_cable_restoration_transition() {
    // Port originally with 60W cable (warning), then replaced with 100W cable (resolved)
    let root1 = TempRoot::new("t1-r3-res-1");
    write_standard_typec_port(
        root1.path(),
        "port0",
        "host [device]",
        "source [sink]",
        true,
    );
    write_standard_partner(root1.path(), "port0", 0x05ac, 0x1000);
    write_standard_cable(root1.path(), "port0", 60, false, 0x0bda, false);
    write_standard_pd_charger(root1.path(), "pd0", 0, 100, 100);
    link_partner_pd(root1.path(), "port0", "pd0");
    let snap1 = snapshot_for(root1.path());

    let root2 = TempRoot::new("t1-r3-res-2");
    write_standard_typec_port(
        root2.path(),
        "port0",
        "host [device]",
        "source [sink]",
        true,
    );
    write_standard_partner(root2.path(), "port0", 0x05ac, 0x1000);
    write_standard_cable(root2.path(), "port0", 100, false, 0x05ac, false);
    write_standard_pd_charger(root2.path(), "pd0", 0, 100, 100);
    link_partner_pd(root2.path(), "port0", "pd0");
    let snap2 = snapshot_for(root2.path());

    let diff = snap2.diff(&snap1);
    assert!(diff.newly_degraded.is_empty());
    assert_eq!(diff.resolved, vec![0]);
}

// =========================================================================
// Area 4: Snapshot Comparison Tooling (R4)
// =========================================================================

#[test]
fn t1_r4_identical_snapshots_diff_empty() {
    let root = TempRoot::new("t1-r4-same");
    write_standard_keyboard(root.path(), "1-1");
    let snap_a = snapshot_for(root.path());
    let snap_b = snapshot_for(root.path());

    let diff = snap_b.diff(&snap_a);
    assert!(diff.is_empty());
}

#[test]
fn t1_r4_added_device_detection() {
    let root1 = TempRoot::new("t1-r4-add-1");
    write_standard_keyboard(root1.path(), "1-1");
    let snap1 = snapshot_for(root1.path());

    let root2 = TempRoot::new("t1-r4-add-2");
    write_standard_keyboard(root2.path(), "1-1");
    write_standard_storage(root2.path(), "2-1", 5000);
    let snap2 = snapshot_for(root2.path());

    let diff = snap2.diff(&snap1);
    assert_eq!(diff.added.len(), 1);
    assert!(diff.added[0].contains("2-1"));
    assert!(diff.removed.is_empty());
}

#[test]
fn t1_r4_removed_device_detection() {
    let root1 = TempRoot::new("t1-r4-rm-1");
    write_standard_keyboard(root1.path(), "1-1");
    write_standard_mouse(root1.path(), "1-2");
    let snap1 = snapshot_for(root1.path());

    let root2 = TempRoot::new("t1-r4-rm-2");
    write_standard_keyboard(root2.path(), "1-1");
    let snap2 = snapshot_for(root2.path());

    let diff = snap2.diff(&snap1);
    assert!(diff.added.is_empty());
    assert_eq!(diff.removed.len(), 1);
    assert!(diff.removed[0].contains("1-2"));
}

#[test]
fn t1_r4_property_change_detection() {
    let root1 = TempRoot::new("t1-r4-prop-1");
    write_standard_typec_port(
        root1.path(),
        "port0",
        "[host] device",
        "[source] sink",
        false,
    );
    write_typec_partner(root1.path(), "port0", "device", &[]);
    let snap1 = snapshot_for(root1.path());

    let root2 = TempRoot::new("t1-r4-prop-2");
    // Swap data role from host to device
    write_standard_typec_port(
        root2.path(),
        "port0",
        "host [device]",
        "[source] sink",
        false,
    );
    write_typec_partner(root2.path(), "port0", "device", &[]);
    let snap2 = snapshot_for(root2.path());

    let diff = snap2.diff(&snap1);
    assert!(
        diff.changed.iter().any(|id| id.contains("port0")),
        "Role change must flag port in changed list"
    );
}

#[test]
fn t1_r4_bidirectional_diff_symmetry() {
    let root1 = TempRoot::new("t1-r4-sym-1");
    write_standard_keyboard(root1.path(), "1-1");
    let snap1 = snapshot_for(root1.path());

    let root2 = TempRoot::new("t1-r4-sym-2");
    write_standard_mouse(root2.path(), "1-2");
    let snap2 = snapshot_for(root2.path());

    let diff_1to2 = snap2.diff(&snap1);
    let diff_2to1 = snap1.diff(&snap2);

    assert_eq!(diff_1to2.added, diff_2to1.removed);
    assert_eq!(diff_1to2.removed, diff_2to1.added);
}

// =========================================================================
// Area 5: CLI Output Modes & Device Filtering (R5)
// =========================================================================

#[test]
fn t1_r5_cli_default_tree_rendering() {
    let root = TempRoot::new("t1-r5-tree");
    write_standard_keyboard(root.path(), "1-1");

    let (ok, stdout, _) = run_cli(root.path(), &[]);
    assert!(ok);
    assert!(stdout.contains("USB Keyboard"));
    assert!(stdout.contains("Link:")); // Tree legend check
}

#[test]
fn t1_r5_cli_list_mode_rendering() {
    let root = TempRoot::new("t1-r5-list");
    write_standard_keyboard(root.path(), "1-1");

    let (ok, stdout, _) = run_cli(root.path(), &["--list"]);
    assert!(ok);
    assert!(stdout.contains("USB Keyboard"));
    assert!(stdout.contains("VID:PID 046d:c31c"));
    assert!(stdout.contains("HID"));
}

#[test]
fn t1_r5_filter_by_category() {
    let root = TempRoot::new("t1-r5-cat");
    write_standard_keyboard(root.path(), "1-1");
    write_standard_typec_port(
        root.path(),
        "port0",
        "[host] device",
        "[source] sink",
        false,
    );

    let snap = snapshot_for(root.path());
    let usb_items: Vec<_> = snap
        .summaries
        .iter()
        .filter(|s| s.category == Category::UsbDevice)
        .collect();
    let typec_items: Vec<_> = snap
        .summaries
        .iter()
        .filter(|s| s.category == Category::TypeCPort)
        .collect();

    assert_eq!(usb_items.len(), 1);
    assert_eq!(typec_items.len(), 1);
}

#[test]
fn t1_r5_filter_by_class() {
    let root = TempRoot::new("t1-r5-class");
    write_standard_keyboard(root.path(), "1-1");
    write_standard_storage(root.path(), "2-1", 5000);

    let snap = snapshot_for(root.path());
    let keyboard_devices: Vec<_> = snap
        .summaries
        .iter()
        .filter(|s| s.device_class == DeviceClass::Keyboard)
        .collect();
    let storage_devices: Vec<_> = snap
        .summaries
        .iter()
        .filter(|s| s.device_class == DeviceClass::Storage)
        .collect();

    assert_eq!(keyboard_devices.len(), 1);
    assert_eq!(keyboard_devices[0].headline, "USB Keyboard");
    assert_eq!(storage_devices.len(), 1);
    assert_eq!(storage_devices[0].headline, "Ultra USB 3.0");
}

#[test]
fn t1_r5_filter_by_vendor() {
    let root = TempRoot::new("t1-r5-vendor");
    write_standard_keyboard(root.path(), "1-1"); // Logitech 0x046d
    write_standard_storage(root.path(), "2-1", 5000); // SanDisk 0x0781

    let snap = snapshot_for(root.path());
    let logitech: Vec<_> = snap
        .summaries
        .iter()
        .filter(|s| s.vendor_id == 0x046d)
        .collect();
    let sandisk: Vec<_> = snap
        .summaries
        .iter()
        .filter(|s| s.vendor_id == 0x0781)
        .collect();

    assert_eq!(logitech.len(), 1);
    assert_eq!(sandisk.len(), 1);
}

// =========================================================================
// Area 6: USB4 / Thunderbolt / Alternate-Mode Expansion (R6)
// =========================================================================

#[test]
fn t1_r6_usb4_host_router_detection() {
    let root = TempRoot::new("t1-r6-host");
    write_thunderbolt_router(root.path(), "0-0", 4, &[("vendor_name", "Intel")]);

    let snap = snapshot_for(root.path());
    assert_eq!(snap.thunderbolt_routers.len(), 1);

    let r = &snap.thunderbolt_routers[0];
    assert!(r.is_host);
    assert_eq!(r.generation, 4);
    assert!(r.supports_usb4());
}

#[test]
fn t1_r6_usb4_device_router_detection() {
    let root = TempRoot::new("t1-r6-dev");
    write_thunderbolt_router(root.path(), "0-1", 4, &[("device_name", "USB4 Dock")]);

    let snap = snapshot_for(root.path());
    let r = &snap.thunderbolt_routers[0];

    assert!(!r.is_host);
    assert_eq!(r.generation, 4);
    assert!(r.supports_usb4());
}

#[test]
fn t1_r6_usb4_active_link() {
    let root = TempRoot::new("t1-r6-link");
    write_thunderbolt_router(root.path(), "0-0", 4, &[]);
    write_thunderbolt_router(root.path(), "0-1", 4, &[]);

    let snap = snapshot_for(root.path());
    assert!(system_has_usb4_link(&snap.thunderbolt_routers));
}

#[test]
fn t1_r6_tbt3_router_distinction() {
    let root = TempRoot::new("t1-r6-tbt3");
    write_thunderbolt_router(root.path(), "0-0", 3, &[]);
    write_thunderbolt_router(root.path(), "0-1", 4, &[]);

    let snap = snapshot_for(root.path());
    // TBT3 host + USB4 device does not form a USB4 link because host cannot speak USB4 protocol
    assert!(!system_has_usb4_link(&snap.thunderbolt_routers));
}

#[test]
fn t1_r6_daisy_chain_topology() {
    let root = TempRoot::new("t1-r6-chain");
    write_thunderbolt_router(root.path(), "0-0", 4, &[]);
    write_thunderbolt_router(root.path(), "0-1", 4, &[]);
    // Linux encodes the route as one hexadecimal byte per hop: 0x101.
    write_thunderbolt_router(root.path(), "0-101", 4, &[]);

    let snap = snapshot_for(root.path());
    assert_eq!(snap.thunderbolt_routers.len(), 3);
    let routes: Vec<&str> = snap
        .thunderbolt_routers
        .iter()
        .map(|r| r.route.as_str())
        .collect();
    assert!(routes.contains(&"0-0"));
    assert!(routes.contains(&"0-1"));
    assert!(routes.contains(&"0-101"));
}

// =========================================================================
// Area 7: Cable Authenticity & Trust Diagnostics (R7)
// =========================================================================

#[test]
fn t1_r7_clean_cable_trust() {
    let root = TempRoot::new("t1-r7-clean");
    write_standard_typec_port(
        root.path(),
        "port0",
        "host [device]",
        "[source] sink",
        false,
    );
    // Well-formed Apple cable (VID 0x05ac)
    write_standard_cable(root.path(), "port0", 100, false, 0x05ac, false);

    let snap = snapshot_for(root.path());
    let cable = snap.summaries[0].cable.as_ref().unwrap();

    assert_eq!(cable.vendor_id, 0x05ac);
    assert!(!cable.trust.zero_vid);
    assert!(!cable.trust.vid_unknown);
    assert!(!cable.trust.reserved_bits_set);
    assert!(!cable.trust.any());
}

#[test]
fn t1_r7_zero_vid_counterfeit() {
    let root = TempRoot::new("t1-r7-zero-vid");
    write_standard_typec_port(
        root.path(),
        "port0",
        "host [device]",
        "[source] sink",
        false,
    );
    // Counterfeit cable reporting vendor ID 0x0000
    write_standard_cable(root.path(), "port0", 60, false, 0x0000, false);

    let snap = snapshot_for(root.path());
    let cable = snap.summaries[0].cable.as_ref().unwrap();

    assert_eq!(cable.vendor_id, 0x0000);
    assert!(cable.trust.zero_vid);
    assert!(cable.trust.any());
}

#[test]
fn t1_r7_unknown_vid_fallback() {
    let root = TempRoot::new("t1-r7-unknown-vid");
    write_standard_typec_port(
        root.path(),
        "port0",
        "host [device]",
        "[source] sink",
        false,
    );
    // VID 0x9999 not in standard vendor database
    write_standard_cable(root.path(), "port0", 60, false, 0x9999, false);

    let snap = snapshot_for(root.path());
    let cable = snap.summaries[0].cable.as_ref().unwrap();

    assert_eq!(cable.vendor_id, 0x9999);
    assert!(cable.trust.vid_unknown);
    assert!(cable.trust.any());
}

#[test]
fn t1_r7_reserved_dirty_bits() {
    let root = TempRoot::new("t1-r7-dirty-bits");
    write_standard_typec_port(
        root.path(),
        "port0",
        "host [device]",
        "[source] sink",
        false,
    );
    // Dirty reserved bits in cable VDO
    write_standard_cable(root.path(), "port0", 100, false, 0x05ac, true);

    let snap = snapshot_for(root.path());
    let cable = snap.summaries[0].cable.as_ref().unwrap();

    assert!(cable.trust.reserved_bits_set);
    assert!(cable.trust.any());
}

#[test]
fn t1_r7_contract_rating_mismatch() {
    let root = TempRoot::new("t1-r7-mismatch");
    write_standard_typec_port(root.path(), "port0", "host [device]", "source [sink]", true);
    write_standard_partner(root.path(), "port0", 0x05ac, 0x1000);
    // Cable rated 3A / 60W but charger offers 5A / 100W
    write_standard_cable(root.path(), "port0", 60, false, 0x05ac, false);
    write_standard_pd_charger(root.path(), "pd0", 0, 100, 100);
    link_partner_pd(root.path(), "port0", "pd0");

    let snap = snapshot_for(root.path());
    let diag = snap.summaries[0].charging_diag.as_ref().unwrap();

    assert_eq!(diag.bottleneck, Bottleneck::CableLimit);
    assert!(diag.is_warning);
}
