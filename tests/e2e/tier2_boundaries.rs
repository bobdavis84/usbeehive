//! Tier 2: Boundary & Corner Cases (>=5 tests per feature area R1-R7).

use crate::e2e::harness::*;
use crate::fixture_builder::*;
use usbeehive::diagnostic::Bottleneck;
use usbeehive::power::{PdoType, PowerDataObject, PowerDeliveryPort};
use usbeehive::summary::{Category, DeviceClass};
use usbeehive::thunderbolt::system_has_usb4_link;

// =========================================================================
// Area 1: Boundary JSON Output (R1)
// =========================================================================

#[test]
fn t2_r1_boundary_empty_sysfs_root() {
    let root = TempRoot::new("t2-r1-empty-root");
    let json = run_cli_json(root.path(), &[]);
    assert_eq!(json.as_array().unwrap().len(), 0);
}

#[test]
fn t2_r1_boundary_missing_usb_typec_dirs() {
    let root = TempRoot::new("t2-r1-missing-dirs");
    // Completely unpopulated root folder
    let (ok, stdout, _) = run_cli(root.path(), &["--json"]);
    assert!(ok);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed.as_array().unwrap().len(), 0);
}

#[test]
fn t2_r1_boundary_device_missing_optional_attrs() {
    let root = TempRoot::new("t2-r1-missing-attrs");
    // Minimal USB device fixture without product, manufacturer, or serial
    let iface = InterfaceFixture {
        number: 0,
        class: 0x00,
        sub_class: 0x00,
        protocol: 0x00,
        driver: "",
    };
    UsbDeviceFixture {
        bus_port: "1-1",
        vendor: 0x1234,
        product: 0x5678,
        product_name: "",
        manufacturer: "",
        serial: "",
        speed_mbps: 480,
        max_power_ma: 500,
        version: "2.00",
        device_class: 0x00,
        bus_num: 1,
        dev_num: 2,
        interfaces: &[iface],
        removable: "unknown",
    }
    .write(root.path());

    let json = run_cli_json(root.path(), &[]);
    let dev = &json.as_array().unwrap()[0];
    assert_eq!(dev["usb"]["vendorId"], "0x1234");
    assert_eq!(dev["usb"]["productId"], "0x5678");
    assert!(dev["headline"].is_string());
}

#[test]
fn t2_r1_boundary_extreme_speed_values() {
    let root = TempRoot::new("t2-r1-extreme-speed");
    let iface = InterfaceFixture {
        number: 0,
        class: 0x00,
        sub_class: 0x00,
        protocol: 0x00,
        driver: "",
    };
    // 80,000 Mbps (USB4 Gen 4x2)
    UsbDeviceFixture {
        bus_port: "3-1",
        vendor: 0x05ac,
        product: 0x1111,
        product_name: "Ultra Speed Device",
        manufacturer: "Apple",
        serial: "SN80G",
        speed_mbps: 80_000,
        max_power_ma: 900,
        version: "3.20",
        device_class: 0x00,
        bus_num: 3,
        dev_num: 1,
        interfaces: &[iface],
        removable: "removable",
    }
    .write(root.path());

    let json = run_cli_json(root.path(), &[]);
    let dev = &json.as_array().unwrap()[0];
    assert_eq!(dev["usb"]["speed"], 80_000);
}

#[test]
fn t2_r1_boundary_special_characters_in_names() {
    let root = TempRoot::new("t2-r1-special-chars");
    let iface = InterfaceFixture {
        number: 0,
        class: 0x00,
        sub_class: 0x00,
        protocol: 0x00,
        driver: "",
    };
    UsbDeviceFixture {
        bus_port: "1-1",
        vendor: 0x046d,
        product: 0xc001,
        product_name: "Keyboard \"Pro\" & <Gaming> [v2.0] \u{2764}",
        manufacturer: "Logitech \u{00A9}",
        serial: "SN#123/456\\789",
        speed_mbps: 12,
        max_power_ma: 100,
        version: "2.00",
        device_class: 0x00,
        bus_num: 1,
        dev_num: 2,
        interfaces: &[iface],
        removable: "removable",
    }
    .write(root.path());

    let json = run_cli_json(root.path(), &[]);
    let dev = &json.as_array().unwrap()[0];
    assert!(dev["headline"]
        .as_str()
        .unwrap()
        .contains("Keyboard \"Pro\" & <Gaming>"));
}

// =========================================================================
// Area 2: Boundary Diagnostics (R2)
// =========================================================================

#[test]
fn t2_r2_boundary_zero_watt_charger_returns_none() {
    let port = PowerDeliveryPort {
        source_capabilities: vec![PowerDataObject {
            r#type: PdoType::FixedSupply,
            power_mw: 0,
            is_active: true,
            ..Default::default()
        }],
        max_source_power_mw: 0,
        ..Default::default()
    };
    assert!(usbeehive::diagnostic::ChargingDiagnostic::evaluate(&port, None, 0, 0).is_none());
}

#[test]
fn t2_r2_boundary_no_source_capabilities_returns_none() {
    let port = PowerDeliveryPort {
        source_capabilities: vec![],
        max_source_power_mw: 100_000,
        ..Default::default()
    };
    assert!(usbeehive::diagnostic::ChargingDiagnostic::evaluate(&port, None, 0, 0).is_none());
}

#[test]
fn t2_r2_boundary_zero_ma_request_handled_safely() {
    let port = PowerDeliveryPort {
        source_capabilities: vec![PowerDataObject {
            r#type: PdoType::FixedSupply,
            power_mw: 60_000,
            current_ma: 3000,
            is_active: true,
            ..Default::default()
        }],
        max_source_power_mw: 60_000,
        ..Default::default()
    };
    let diag = usbeehive::diagnostic::ChargingDiagnostic::evaluate(&port, None, 0, 0);
    assert!(diag.is_some());
    assert_eq!(diag.unwrap().bottleneck, Bottleneck::Fine);
}

#[test]
fn t2_r2_boundary_exact_80pct_threshold() {
    // Exactly 80W on a 100W charger is >= 80%, so it is Fine, not DeviceLimit
    let port = PowerDeliveryPort {
        source_capabilities: vec![
            PowerDataObject {
                r#type: PdoType::FixedSupply,
                power_mw: 100_000,
                is_active: false,
                ..Default::default()
            },
            PowerDataObject {
                r#type: PdoType::FixedSupply,
                power_mw: 80_000,
                is_active: true,
                ..Default::default()
            },
        ],
        max_source_power_mw: 100_000,
        ..Default::default()
    };
    let diag = usbeehive::diagnostic::ChargingDiagnostic::evaluate(&port, None, 0, 0).unwrap();
    assert_eq!(diag.bottleneck, Bottleneck::Fine);

    // 79W is < 80%, which flags DeviceLimit
    let port_79 = PowerDeliveryPort {
        source_capabilities: vec![
            PowerDataObject {
                r#type: PdoType::FixedSupply,
                power_mw: 100_000,
                is_active: false,
                ..Default::default()
            },
            PowerDataObject {
                r#type: PdoType::FixedSupply,
                power_mw: 79_000,
                is_active: true,
                ..Default::default()
            },
        ],
        max_source_power_mw: 100_000,
        ..Default::default()
    };
    let diag_79 =
        usbeehive::diagnostic::ChargingDiagnostic::evaluate(&port_79, None, 0, 0).unwrap();
    assert_eq!(diag_79.bottleneck, Bottleneck::DeviceLimit);
}

#[test]
fn t2_r2_boundary_orphan_pd_port_without_partner() {
    let root = TempRoot::new("t2-r2-orphan-pd");
    write_pd_port(
        root.path(),
        "pd0",
        -1,
        &[PdoFixture {
            voltage_mv: 5000,
            current_ma: 3000,
            power_mw: 15_000,
            kind: "fixed_supply",
            min_voltage_mv: 0,
            max_voltage_mv: 0,
        }],
    );

    let snap = snapshot_for(root.path());
    assert_eq!(snap.pd_ports.len(), 1);
    assert!(snap.summaries.is_empty());
}

// =========================================================================
// Area 3: Boundary State Transitions (R3)
// =========================================================================

#[test]
fn t2_r3_boundary_empty_to_empty() {
    let root1 = TempRoot::new("t2-r3-empty1");
    let root2 = TempRoot::new("t2-r3-empty2");
    let snap1 = snapshot_for(root1.path());
    let snap2 = snapshot_for(root2.path());

    let diff = snap2.diff(&snap1);
    assert!(diff.is_empty());
}

#[test]
fn t2_r3_boundary_oscillation_recovery() {
    let root = TempRoot::new("t2-r3-osc");

    // State 1: Empty
    let snap1 = snapshot_for(root.path());

    // State 2: Device plugged in
    write_standard_keyboard(root.path(), "1-1");
    let snap2 = snapshot_for(root.path());
    let diff12 = snap2.diff(&snap1);
    assert_eq!(diff12.added.len(), 1);

    // State 3: Device unplugged (remove directory)
    std::fs::remove_dir_all(root.path().join("bus/usb/devices/1-1")).unwrap();
    let snap3 = snapshot_for(root.path());
    let diff23 = snap3.diff(&snap2);
    assert_eq!(diff23.removed.len(), 1);

    // State 3 relative to State 1 should be completely empty diff
    let diff13 = snap3.diff(&snap1);
    assert!(diff13.is_empty());
}

#[test]
fn t2_r3_boundary_rapid_multi_device_change() {
    let root1 = TempRoot::new("t2-r3-multi1");
    write_standard_keyboard(root1.path(), "1-1");
    write_standard_mouse(root1.path(), "1-2");
    let snap1 = snapshot_for(root1.path());

    let root2 = TempRoot::new("t2-r3-multi2");
    // Keyboard and mouse gone, storage and hub arrived
    write_standard_storage(root2.path(), "2-1", 5000);
    write_standard_hub(root2.path(), "1-3", 4);
    let snap2 = snapshot_for(root2.path());

    let diff = snap2.diff(&snap1);
    assert_eq!(diff.removed.len(), 2);
    assert_eq!(diff.added.len(), 2);
}

#[test]
fn t2_r3_boundary_subsystem_restarts() {
    let root = TempRoot::new("t2-r3-restart");
    write_standard_keyboard(root.path(), "1-1");

    let snap1 = snapshot_for(root.path());
    let snap2 = snapshot_for(root.path());

    assert!(snap2.diff(&snap1).is_empty());
}

#[test]
fn t2_r3_boundary_role_inversion() {
    let root1 = TempRoot::new("t2-r3-role1");
    write_standard_typec_port(
        root1.path(),
        "port0",
        "[host] device",
        "[source] sink",
        false,
    );
    write_typec_partner(root1.path(), "port0", "device", &[]);
    let snap1 = snapshot_for(root1.path());

    let root2 = TempRoot::new("t2-r3-role2");
    // Invert data role from host to device
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
    assert!(diff.changed.iter().any(|id| id.contains("port0")));
}

// =========================================================================
// Area 4: Boundary Snapshot Diff (R4)
// =========================================================================

#[test]
fn t2_r4_boundary_diff_empty_vs_populated() {
    let root_empty = TempRoot::new("t2-r4-empty");
    let root_pop = TempRoot::new("t2-r4-pop");
    write_standard_keyboard(root_pop.path(), "1-1");

    let snap_empty = snapshot_for(root_empty.path());
    let snap_pop = snapshot_for(root_pop.path());

    let diff = snap_pop.diff(&snap_empty);
    assert_eq!(diff.added.len(), 1);
    assert!(diff.removed.is_empty());
}

#[test]
fn t2_r4_boundary_diff_populated_vs_empty() {
    let root_empty = TempRoot::new("t2-r4-empty-cur");
    let root_pop = TempRoot::new("t2-r4-pop-prev");
    write_standard_keyboard(root_pop.path(), "1-1");

    let snap_empty = snapshot_for(root_empty.path());
    let snap_pop = snapshot_for(root_pop.path());

    let diff = snap_empty.diff(&snap_pop);
    assert!(diff.added.is_empty());
    assert_eq!(diff.removed.len(), 1);
}

#[test]
fn t2_r4_boundary_diff_completely_disjoint() {
    let root1 = TempRoot::new("t2-r4-disjoint1");
    write_standard_keyboard(root1.path(), "1-1");

    let root2 = TempRoot::new("t2-r4-disjoint2");
    write_standard_storage(root2.path(), "2-1", 5000);

    let snap1 = snapshot_for(root1.path());
    let snap2 = snapshot_for(root2.path());

    let diff = snap2.diff(&snap1);
    assert_eq!(diff.added.len(), 1);
    assert_eq!(diff.removed.len(), 1);
}

#[test]
fn t2_r4_boundary_diff_partner_detached() {
    let root1 = TempRoot::new("t2-r4-attached");
    write_standard_typec_port(
        root1.path(),
        "port0",
        "host [device]",
        "[source] sink",
        false,
    );
    write_standard_partner(root1.path(), "port0", 0x05ac, 0x1000);
    write_standard_cable(root1.path(), "port0", 100, false, 0x05ac, false);
    let snap1 = snapshot_for(root1.path());

    let root2 = TempRoot::new("t2-r4-detached");
    write_standard_typec_port(
        root2.path(),
        "port0",
        "[host] device",
        "[source] sink",
        false,
    );
    // Partner and cable removed
    let snap2 = snapshot_for(root2.path());

    let diff = snap2.diff(&snap1);
    // Port ID stays same, but properties changed
    assert!(diff.changed.iter().any(|id| id.contains("port0")));
}

#[test]
fn t2_r4_boundary_diff_unrelated_property_no_chatter() {
    let root1 = TempRoot::new("t2-r4-noise1");
    write_standard_keyboard(root1.path(), "1-1");
    let snap1 = snapshot_for(root1.path());

    let root2 = TempRoot::new("t2-r4-noise2");
    write_standard_keyboard(root2.path(), "1-1");
    let snap2 = snapshot_for(root2.path());

    let diff = snap2.diff(&snap1);
    assert!(
        diff.is_empty(),
        "Identical hardware must not create chatter"
    );
}

// =========================================================================
// Area 5: Boundary CLI Filtering & Views (R5)
// =========================================================================

#[test]
fn t2_r5_boundary_filter_zero_matches() {
    let root = TempRoot::new("t2-r5-zero-match");
    write_standard_keyboard(root.path(), "1-1");

    let snap = snapshot_for(root.path());
    let audio_devs: Vec<_> = snap
        .summaries
        .iter()
        .filter(|s| s.device_class == DeviceClass::Audio)
        .collect();
    assert_eq!(audio_devs.len(), 0);
}

#[test]
fn t2_r5_boundary_filter_all_matches() {
    let root = TempRoot::new("t2-r5-all-match");
    write_standard_keyboard(root.path(), "1-1");
    write_standard_mouse(root.path(), "1-2");

    let snap = snapshot_for(root.path());
    let usb_devs: Vec<_> = snap
        .summaries
        .iter()
        .filter(|s| s.category == Category::UsbDevice)
        .collect();
    assert_eq!(usb_devs.len(), 2);
}

#[test]
fn t2_r5_boundary_deep_hub_chain_tree() {
    let root = TempRoot::new("t2-r5-deep-chain");
    write_standard_hub(root.path(), "1-1", 4);
    write_standard_hub(root.path(), "1-1.1", 4);
    write_standard_hub(root.path(), "1-1.1.1", 4);
    write_standard_keyboard(root.path(), "1-1.1.1.1");

    let (ok, stdout, _) = run_cli(root.path(), &[]);
    assert!(ok);
    assert!(stdout.contains("USB Keyboard"));
    assert!(stdout.contains("USB2.0 Hub"));
}

#[test]
fn t2_r5_boundary_cli_conflicting_flags() {
    let root = TempRoot::new("t2-r5-conflict");
    // --list and --tree conflict
    let (ok, _, stderr) = run_cli(root.path(), &["--list", "--tree"]);
    assert!(!ok, "Conflicting flags must exit with error");
    assert!(stderr.contains("cannot be used with"));
}

#[test]
fn t2_r5_boundary_tree_legend_empty_tree() {
    let root = TempRoot::new("t2-r5-empty-tree");
    let (ok, stdout, _) = run_cli(root.path(), &[]);
    assert!(ok);
    assert!(stdout.contains("No USB devices found."));
    // Legend line must not appear when no devices exist
    assert!(!stdout.contains("Link:"));
}

// =========================================================================
// Area 6: Boundary USB4 / Alt-Mode (R6)
// =========================================================================

#[test]
fn t2_r6_boundary_missing_thunderbolt_subsystem() {
    let root = TempRoot::new("t2-r6-missing-tb");
    // Subsystem directory /sys/bus/thunderbolt absent
    let snap = snapshot_for(root.path());
    assert!(snap.thunderbolt_routers.is_empty());
    assert!(!system_has_usb4_link(&snap.thunderbolt_routers));
}

#[test]
fn t2_r6_boundary_router_without_generation() {
    let root = TempRoot::new("t2-r6-no-gen");
    let dir = root.path().join("bus/thunderbolt/devices/0-0");
    std::fs::create_dir_all(&dir).unwrap();
    write_attr(&dir, "vendor", "0x8086");
    write_attr(&dir, "device", "0x1234");
    // Do not write "generation" file

    let snap = snapshot_for(root.path());
    // Keep the router, but represent its unknown generation explicitly.
    assert_eq!(snap.thunderbolt_routers.len(), 1);
    assert_eq!(snap.thunderbolt_routers[0].reported_generation, None);
}

#[test]
fn t2_r6_boundary_router_generation_zero() {
    let root = TempRoot::new("t2-r6-gen-zero");
    write_thunderbolt_router(root.path(), "0-0", 0, &[]);

    let snap = snapshot_for(root.path());
    // A zero generation is not a usable protocol claim, but the router
    // remains visible with a missing reported generation.
    assert_eq!(snap.thunderbolt_routers.len(), 1);
    assert_eq!(snap.thunderbolt_routers[0].reported_generation, None);
}

#[test]
fn t2_r6_boundary_single_usb4_host_no_link() {
    let root = TempRoot::new("t2-r6-host-only");
    // Host router supports USB4, but no device router is connected
    write_thunderbolt_router(root.path(), "0-0", 4, &[]);

    let snap = snapshot_for(root.path());
    assert!(!system_has_usb4_link(&snap.thunderbolt_routers));
}

#[test]
fn t2_r6_boundary_single_usb4_device_no_host() {
    let root = TempRoot::new("t2-r6-dev-only");
    // Device router claims USB4, but host adapter is missing
    write_thunderbolt_router(root.path(), "0-1", 4, &[]);

    let snap = snapshot_for(root.path());
    assert!(!system_has_usb4_link(&snap.thunderbolt_routers));
}

// =========================================================================
// Area 7: Boundary Cable Trust (R7)
// =========================================================================

#[test]
fn t2_r7_boundary_passive_non_emarked_cable() {
    let root = TempRoot::new("t2-r7-passive");
    write_standard_typec_port(
        root.path(),
        "port0",
        "host [device]",
        "[source] sink",
        false,
    );
    // Passive cable with no identity subfolder
    let dir = root.path().join("class/typec/port0-cable");
    std::fs::create_dir_all(&dir).unwrap();
    write_attr(&dir, "type", "passive");
    write_attr(&dir, "plug_type", "type-c");

    let snap = snapshot_for(root.path());
    let cable = snap.summaries[0].cable.as_ref().unwrap();

    assert!(cable.is_passive);
    assert_eq!(cable.max_watts, 0);
    assert_eq!(cable.current_rating, None);
    assert_eq!(cable.speed, None);
    assert!(!cable.trust.any());
}

#[test]
fn t2_r7_boundary_cable_no_vdos() {
    let root = TempRoot::new("t2-r7-no-vdos");
    write_standard_typec_port(
        root.path(),
        "port0",
        "host [device]",
        "[source] sink",
        false,
    );
    let dir = root.path().join("class/typec/port0-cable/identity");
    std::fs::create_dir_all(&dir).unwrap();

    let snap = snapshot_for(root.path());
    let cable = snap.summaries[0].cable.as_ref().unwrap();
    assert_eq!(cable.vendor_id, 0);
}

#[test]
fn t2_r7_boundary_dirty_bits_only() {
    let root = TempRoot::new("t2-r7-dirty-only");
    write_standard_typec_port(
        root.path(),
        "port0",
        "host [device]",
        "[source] sink",
        false,
    );
    // Valid Apple VID 0x05ac, but with dirty reserved bits
    write_standard_cable(root.path(), "port0", 100, false, 0x05ac, true);

    let snap = snapshot_for(root.path());
    let cable = snap.summaries[0].cable.as_ref().unwrap();

    assert_eq!(cable.vendor_id, 0x05ac);
    assert!(!cable.trust.zero_vid);
    assert!(!cable.trust.vid_unknown);
    assert!(cable.trust.reserved_bits_set);
    assert!(cable.trust.any());
}

#[test]
fn t2_r7_boundary_zero_vid_with_clean_vdo() {
    let root = TempRoot::new("t2-r7-zero-clean");
    write_standard_typec_port(
        root.path(),
        "port0",
        "host [device]",
        "[source] sink",
        false,
    );
    // Zero VID, but clean VDO bits
    write_standard_cable(root.path(), "port0", 60, false, 0x0000, false);

    let snap = snapshot_for(root.path());
    let cable = snap.summaries[0].cable.as_ref().unwrap();

    assert!(cable.trust.zero_vid);
    assert!(!cable.trust.reserved_bits_set);
    assert!(cable.trust.any());
}

#[test]
fn t2_r7_boundary_unknown_vid_with_clean_vdo() {
    let root = TempRoot::new("t2-r7-unk-clean");
    write_standard_typec_port(
        root.path(),
        "port0",
        "host [device]",
        "[source] sink",
        false,
    );
    // Unregistered VID, but clean VDO bits
    write_standard_cable(root.path(), "port0", 60, false, 0xbeef, false);

    let snap = snapshot_for(root.path());
    let cable = snap.summaries[0].cable.as_ref().unwrap();

    assert!(cable.trust.vid_unknown);
    assert!(!cable.trust.reserved_bits_set);
    assert!(cable.trust.any());
}
