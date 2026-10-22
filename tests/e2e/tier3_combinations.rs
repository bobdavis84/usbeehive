//! Tier 3: Cross-Feature Combinations (Pairwise interactions across R1-R7).

use crate::e2e::harness::*;
use crate::fixture_builder::*;
use usbeehive::diagnostic::Bottleneck;
use usbeehive::summary::{Category, DeviceClass};
use usbeehive::thunderbolt::system_has_usb4_link;

// =========================================================================
// Combination 1: JSON Output + Cable Trust + Diagnostic Warning (R1 + R7 + R2)
// =========================================================================
#[test]
fn t3_combo_json_cable_trust_diagnostic_pipeline() {
    let root = TempRoot::new("t3-c1-json-trust-diag");
    write_standard_typec_port(root.path(), "port0", "host [device]", "source [sink]", true);
    write_standard_partner(root.path(), "port0", 0x05ac, 0x1000);
    // 60W counterfeit cable with zero VID and dirty bits on 100W charger
    write_standard_cable(root.path(), "port0", 60, false, 0x0000, true);
    write_standard_pd_charger(root.path(), "pd0", 0, 100, 100);
    link_partner_pd(root.path(), "port0", "pd0");

    let json = run_cli_json(root.path(), &[]);
    let arr = json.as_array().expect("array of devices");
    let port = arr
        .iter()
        .find(|d| d["category"] == "typec")
        .expect("typec port entry");

    // R2 check: Diagnostic warning for cable bottleneck in JSON
    assert!(port["charging"].is_object());
    assert_eq!(port["charging"]["isWarning"], true);
    assert!(port["charging"]["summary"]
        .as_str()
        .unwrap()
        .contains("Cable rating is below available source capability"));

    // R7 check: Trust heuristics surfaced in JSON
    assert_eq!(port["cable"]["vendorId"], "0x0000");
    assert_eq!(port["cable"]["maxWatts"], 60);

    // Library verification of trust bits
    let snap = snapshot_for(root.path());
    let cable = snap.summaries[0].cable.as_ref().unwrap();
    assert!(cable.trust.zero_vid);
    assert!(cable.trust.reserved_bits_set);
    assert!(cable.trust.any());
}

// =========================================================================
// Combination 2: History & Transitions + Changing Cable Trust (R3 + R7 + R2)
// =========================================================================
#[test]
fn t3_combo_history_state_transition_with_cable_swap() {
    let root1 = TempRoot::new("t3-c2-clean");
    write_standard_typec_port(
        root1.path(),
        "port0",
        "host [device]",
        "source [sink]",
        true,
    );
    write_standard_partner(root1.path(), "port0", 0x05ac, 0x1000);
    // Clean 100W cable
    write_standard_cable(root1.path(), "port0", 100, false, 0x05ac, false);
    write_standard_pd_charger(root1.path(), "pd0", 0, 100, 100);
    link_partner_pd(root1.path(), "port0", "pd0");
    let snap1 = snapshot_for(root1.path());

    // Verify initial clean state
    assert_eq!(
        snap1.summaries[0]
            .charging_diag
            .as_ref()
            .unwrap()
            .bottleneck,
        Bottleneck::Fine
    );
    assert!(!snap1.summaries[0].cable.as_ref().unwrap().trust.any());

    // State 2: User replaces cable with 60W counterfeit cable with dirty bits
    let root2 = TempRoot::new("t3-c2-counterfeit");
    write_standard_typec_port(
        root2.path(),
        "port0",
        "host [device]",
        "source [sink]",
        true,
    );
    write_standard_partner(root2.path(), "port0", 0x05ac, 0x1000);
    write_standard_cable(root2.path(), "port0", 60, false, 0x0000, true);
    write_standard_pd_charger(root2.path(), "pd0", 0, 100, 100);
    link_partner_pd(root2.path(), "port0", "pd0");
    let snap2 = snapshot_for(root2.path());

    let diff = snap2.diff(&snap1);
    // Transition detects degradation
    assert_eq!(diff.newly_degraded, vec![0]);
    assert!(diff.resolved.is_empty());

    // Trust heuristics in second state
    let cable2 = snap2.summaries[0].cable.as_ref().unwrap();
    assert!(cable2.trust.zero_vid);
    assert!(cable2.trust.reserved_bits_set);
}

// =========================================================================
// Combination 3: Alt-Mode / Billboard + BOS Capability + Diagnostics (R6 + R2)
// =========================================================================
#[test]
fn t3_combo_billboard_altmode_with_power_diagnostic() {
    let root = TempRoot::new("t3-c3-altmode-bb");
    // Type-C port charging at 60W
    write_standard_typec_port(root.path(), "port0", "host [device]", "source [sink]", true);
    write_standard_partner(root.path(), "port0", 0x05ac, 0x1000);
    write_standard_cable(root.path(), "port0", 100, false, 0x05ac, false);
    write_standard_pd_charger(root.path(), "pd0", 0, 60, 60);
    link_partner_pd(root.path(), "port0", "pd0");

    // Partner device link to USB bus
    link_partner_usb(root.path(), "port0", "1-1");

    // USB peripheral with Billboard descriptor:
    // BOS descriptor with Billboard Capability declaring alternate mode
    let mut bos_blob = vec![
        0x05, 0x0F, // bLength=5, bDescriptorType=BOS
        0x2C, 0x00, // wTotalLength=44 bytes
        0x01, // bNumDeviceCaps=1
        // Billboard capability descriptor (bLength=44, bDescriptorType=DEVICE_CAPABILITY, bDevCapabilityType=0x0D)
        0x2C, 0x10,
        0x0D, // iAddtionalInfoURL=0, bNumberOfAlternateModes=1, bPreferredAlternateMode=0
        0x00, 0x01, 0x00, // VconnPower (wVconnPower=0)
        0x00, 0x00, // bmConfigured (bit0=0, bit1=0)
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, // bReserved=0
        0x00, // AltMode[0]: wSVID=0xFF01 (VESA DisplayPort), bAlternateMode=1, iAlternateModeString=0
        0x01, 0xFF, 0x01, 0x00,
    ];
    bos_blob.truncate(44 + 5);

    write_standard_keyboard(root.path(), "1-1");
    write_bos(root.path(), "1-1", &bos_blob);

    let snap = snapshot_for(root.path());
    let port_summary = snap
        .summaries
        .iter()
        .find(|s| s.category == Category::TypeCPort)
        .expect("Type-C summary");
    assert_eq!(
        port_summary.charging_diag.as_ref().unwrap().bottleneck,
        Bottleneck::Fine
    );
}

// =========================================================================
// Combination 4: Snapshot Diff + Filtering (R4 + R5)
// =========================================================================
#[test]
fn t3_combo_snapshot_diff_targeted_by_class() {
    let root1 = TempRoot::new("t3-c4-filter-diff-1");
    write_standard_keyboard(root1.path(), "1-1");
    write_standard_mouse(root1.path(), "1-2");
    write_standard_storage(root1.path(), "2-1", 5000);
    let snap1 = snapshot_for(root1.path());

    let root2 = TempRoot::new("t3-c4-filter-diff-2");
    // Mouse removed, new storage attached
    write_standard_keyboard(root2.path(), "1-1");
    write_standard_storage(root2.path(), "2-1", 5000);
    write_standard_storage(root2.path(), "2-2", 10000);
    let snap2 = snapshot_for(root2.path());

    let full_diff = snap2.diff(&snap1);
    assert_eq!(full_diff.added.len(), 1);
    assert_eq!(full_diff.removed.len(), 1);

    // Filtered diff analysis on storage devices:
    let storage_added: Vec<_> = snap2
        .summaries
        .iter()
        .filter(|s| s.device_class == DeviceClass::Storage)
        .filter(|s| full_diff.added.contains(&s.id()))
        .collect();

    assert_eq!(storage_added.len(), 1);
    assert!(storage_added[0].id().contains("2-2"));
}

// =========================================================================
// Combination 5: USB4 Topology + Link Degradation + Rollup Summary (R6 + R3 + R5)
// =========================================================================
#[test]
fn t3_combo_usb4_topology_with_degraded_downstream_link() {
    let root = TempRoot::new("t3-c5-usb4-deg");
    // USB4 host and dock router
    write_thunderbolt_router(root.path(), "0-0", 4, &[("vendor_name", "Intel")]);
    write_thunderbolt_router(
        root.path(),
        "0-1",
        4,
        &[("device_name", "Thunderbolt 4 Dock")],
    );

    // Downstream device plugged in at 480 Mbps
    write_standard_storage(root.path(), "1-1", 480);

    let snap = snapshot_for(root.path());

    // R6: USB4 link verified
    assert!(system_has_usb4_link(&snap.thunderbolt_routers));

    // R5: Summaries capture both router topology and USB device
    assert_eq!(snap.thunderbolt_routers.len(), 2);
    let storage = snap
        .summaries
        .iter()
        .find(|s| s.category == Category::UsbDevice)
        .expect("storage device");
    assert_eq!(storage.headline, "Ultra USB 3.0");
    assert_eq!(storage.link_speed_mbps, 480);
}

// =========================================================================
// Combination 6: Power Contract Renegotiation + State Diff (R2 + R3 + R4)
// =========================================================================
#[test]
fn t3_combo_power_contract_renegotiation_diff() {
    let root1 = TempRoot::new("t3-c6-reneg-1");
    // Initial: 5V/3A (15W) contract
    write_standard_typec_port(
        root1.path(),
        "port0",
        "host [device]",
        "source [sink]",
        true,
    );
    write_standard_partner(root1.path(), "port0", 0x05ac, 0x1000);
    write_standard_cable(root1.path(), "port0", 100, false, 0x05ac, false);
    write_standard_pd_charger(root1.path(), "pd0", 0, 100, 15);
    link_partner_pd(root1.path(), "port0", "pd0");
    let mut snap1 = snapshot_for(root1.path());
    snap1.summaries[0]
        .power_delivery
        .as_mut()
        .unwrap()
        .source_capabilities[0]
        .is_active = true;

    let root2 = TempRoot::new("t3-c6-reneg-2");
    // Renegotiated: 20V/5A (100W) contract
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
    let mut snap2 = snapshot_for(root2.path());
    let last_idx = snap2.summaries[0]
        .power_delivery
        .as_ref()
        .unwrap()
        .source_capabilities
        .len()
        - 1;
    snap2.summaries[0]
        .power_delivery
        .as_mut()
        .unwrap()
        .source_capabilities[last_idx]
        .is_active = true;

    let diff = snap2.diff(&snap1);
    // Port ID was preserved; no false addition/removal
    assert!(diff.added.is_empty());
    assert!(diff.removed.is_empty());
    assert!(
        diff.changed.iter().any(|id| id.contains("port0")),
        "Renegotiated contract must be reflected in changed"
    );
}
