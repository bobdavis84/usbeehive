//! Adversarial Empirical Challenger Stress Suite for Milestone 2 (Challenger 1).
//!
//! Empirically tests:
//! 1. Snapshot Comparison Engine (`src/compare.rs`):
//!    - Compare identical snapshots (empty and multi-device full) -> diff is empty
//!    - Compare empty snapshot to full snapshot -> all devices marked `added`
//!    - Compare full snapshot to empty snapshot -> all devices marked `removed`
//!    - Completely disjoint snapshots -> all removed and added, zero modified
//!    - Complex simultaneous modifications on same device (power draw shift + PD contract
//!      renegotiation + link speed upgrade + cable trust degradation + diagnostics)
//!    - Extreme boundary values (u32::MAX, 0, edge string values)
//! 2. History Ring Buffer (`src/history.rs`):
//!    - Massive sequential pushes: 1,000 snapshots into capacity 10 buffer
//!    - Strict capacity enforcement and FIFO eviction (oldest=990, newest=999, total=1000)
//!    - Out-of-bounds indexing safety and evicted sequence returns `None`
//!    - Zero-capacity and capacity-1 clamping and transition detection
//!    - Dynamic capacity resizing (shrinkage & expansion) and buffer clear
//! 3. State Transitions Engine (`detect_transitions`):
//!    - Explicit verification of all 9 transition variants:
//!      `DeviceConnected`, `DeviceDisconnected`, `LinkSpeedChanged`, `PowerChanged`,
//!      `ContractRenegotiated`, `BottleneckChanged`, `CableWarningChanged`,
//!      `PortRoleChanged`, `DataRateChanged`
//!    - Display formatting and prefix badges ([+], [-], [~], [!], [*])
//!    - 50-cycle oscillating connections (plug-unplug-plug-unplug)
//!    - Sequence queries (`transitions_since`, `filter_transitions`, `timeline`, `diff_range`)
//! 4. Serialization Invariants:
//!    - Round-trip fidelity for Snapshot, SnapshotDiff, SnapshotHistory, StateTransition

use usbeehive::bos::{DataRateAssessment, DataRateVerdict};
use usbeehive::cable::{CableInfo, CableTrust};
use usbeehive::compare::{compare, ValueChange};
use usbeehive::diagnostic::{
    Bottleneck, ChargingDiagnostic, DiagnosticConfidence, DiagnosticStatus, RootCauseCategory,
};
use usbeehive::history::{SnapshotHistory, StateTransition};
use usbeehive::pd::{CableCurrent, CableSpeed};
use usbeehive::power::{PdoType, PowerDataObject, PowerDeliveryPort};
use usbeehive::snapshot::{Snapshot, SnapshotMetadata};
use usbeehive::summary::{Category, DeviceClass, DeviceSummary, PowerRole, PowerSummary, Status};
use usbeehive::thunderbolt::ThunderboltRouter;
use usbeehive::trust::CableTrustLevel;
use usbeehive::typec::TypeCPort;
use usbeehive::usb::UsbDevice;

// =========================================================================
// Test Fixture Helpers
// =========================================================================

fn make_usb_summary(
    bus_port: &str,
    speed: u32,
    headline: &str,
    vendor: &str,
    product: &str,
) -> DeviceSummary {
    DeviceSummary {
        category: Category::UsbDevice,
        device_class: DeviceClass::Storage,
        device_subclass: "SCSI".into(),
        status: Status::Connected,
        headline: headline.into(),
        subtitle: format!("{speed} Mbps"),
        icon: "drive-removable-media".into(),
        vendor: vendor.into(),
        product: product.into(),
        vendor_id: 0x1234,
        product_id: 0x5678,
        link_speed_mbps: speed,
        usb_version: "3.2".into(),
        primary_driver: "uas".into(),
        power: PowerSummary {
            power_role: PowerRole::Sink,
            power_in_mw: 4500,
            power_out_mw: 0,
            contract_mw: 0,
        },
        charging_diag: None,
        data_rate: None,
        cable: None,
        typec_port: None,
        power_delivery: None,
        usb_device: Some(UsbDevice {
            bus_port: bus_port.into(),
            speed,
            product: product.into(),
            manufacturer: vendor.into(),
            vendor_id: 0x1234,
            product_id: 0x5678,
            ..Default::default()
        }),
        properties: vec![
            ("bus_port".into(), bus_port.into()),
            ("transport.protocol".into(), "usb".into()),
        ],
    }
}

fn make_port_summary(port_name: &str, port_number: i32, headline: &str) -> DeviceSummary {
    DeviceSummary {
        category: Category::TypeCPort,
        device_class: DeviceClass::Unknown,
        device_subclass: String::new(),
        status: Status::Connected,
        headline: headline.into(),
        subtitle: "Type-C Port".into(),
        icon: "preferences-system-power".into(),
        vendor: String::new(),
        product: String::new(),
        vendor_id: 0,
        product_id: 0,
        link_speed_mbps: 0,
        usb_version: String::new(),
        primary_driver: String::new(),
        power: PowerSummary {
            power_role: PowerRole::DualRole,
            power_in_mw: 0,
            power_out_mw: 0,
            contract_mw: 0,
        },
        charging_diag: None,
        data_rate: None,
        cable: None,
        typec_port: Some(TypeCPort {
            port_name: port_name.into(),
            port_number,
            ..Default::default()
        }),
        power_delivery: None,
        usb_device: None,
        properties: vec![("data_role".into(), "host".into())],
    }
}

fn make_populated_snapshot(count: usize) -> Snapshot {
    let mut snap = Snapshot::new().with_metadata(SnapshotMetadata::new(
        1729512000000,
        1,
        "stress-test-harness",
    ));

    for i in 1..=count {
        let bus_port = format!("1-1.{i}");
        let summary = make_usb_summary(
            &bus_port,
            5000,
            &format!("Device {i}"),
            "VendorCo",
            &format!("Product {i}"),
        );
        snap.summaries.push(summary);
        snap.usb_devices.push(UsbDevice {
            bus_port,
            speed: 5000,
            product: format!("Product {i}"),
            manufacturer: "VendorCo".into(),
            ..Default::default()
        });
    }

    snap
}

// =========================================================================
// 1. Snapshot Comparison Engine Tests (`src/compare.rs`)
// =========================================================================

#[test]
fn stress_compare_identical_empty_snapshots() {
    let before = Snapshot::new();
    let after = Snapshot::new();

    let diff = compare(&before, &after);
    assert!(diff.is_empty(), "Empty vs empty diff must be empty");
    assert_eq!(diff.total_changes(), 0);
    assert_eq!(diff.summary_string(), "No differences");
    assert!(!diff.has_warnings());
    assert!(diff.added.is_empty());
    assert!(diff.removed.is_empty());
    assert!(diff.modified.is_empty());
}

#[test]
fn stress_compare_identical_populated_snapshots() {
    let mut snap = make_populated_snapshot(10);
    snap.typec_ports.push(TypeCPort {
        port_number: 0,
        port_name: "port0".into(),
        ..Default::default()
    });
    snap.summaries
        .push(make_port_summary("port0", 0, "Type-C Port 0"));
    snap.thunderbolt_routers.push(ThunderboltRouter {
        route: "0-0".into(),
        generation: 4,
        ..Default::default()
    });

    let diff = compare(&snap, &snap.clone());
    assert!(
        diff.is_empty(),
        "Populated snapshot compared to itself must be empty"
    );
    assert_eq!(diff.total_changes(), 0);
    assert_eq!(diff.summary_string(), "No differences");
    assert!(!diff.has_warnings());
    assert!(diff.added.is_empty());
    assert!(diff.removed.is_empty());
    assert!(diff.modified.is_empty());
}

#[test]
fn stress_compare_empty_to_full_all_added() {
    let before = Snapshot::new();
    let after = make_populated_snapshot(12);

    let diff = compare(&before, &after);
    assert!(!diff.is_empty());
    assert_eq!(diff.total_changes(), 12);
    assert_eq!(diff.added.len(), 12);
    assert!(diff.removed.is_empty());
    assert!(diff.modified.is_empty());
    assert_eq!(diff.summary_string(), "+12 added, -0 removed, ~0 modified");

    // Verify added IDs are sorted deterministically
    let added_ids = diff.added_ids();
    let mut sorted_ids = added_ids.clone();
    sorted_ids.sort();
    assert_eq!(added_ids, sorted_ids);
}

#[test]
fn stress_compare_full_to_empty_all_removed() {
    let before = make_populated_snapshot(15);
    let after = Snapshot::new();

    let diff = compare(&before, &after);
    assert!(!diff.is_empty());
    assert_eq!(diff.total_changes(), 15);
    assert!(diff.added.is_empty());
    assert_eq!(diff.removed.len(), 15);
    assert!(diff.modified.is_empty());
    assert_eq!(diff.summary_string(), "+0 added, -15 removed, ~0 modified");

    // Verify removed IDs are sorted deterministically
    let removed_ids = diff.removed_ids();
    let mut sorted_ids = removed_ids.clone();
    sorted_ids.sort();
    assert_eq!(removed_ids, sorted_ids);
}

#[test]
fn stress_compare_disjoint_snapshots() {
    let mut before = Snapshot::new();
    for i in 1..=5 {
        before.summaries.push(make_usb_summary(
            &format!("1-1.{i}"),
            480,
            &format!("Old Dev {i}"),
            "V1",
            "P1",
        ));
    }

    let mut after = Snapshot::new();
    for i in 1..=5 {
        after.summaries.push(make_usb_summary(
            &format!("2-1.{i}"),
            5000,
            &format!("New Dev {i}"),
            "V2",
            "P2",
        ));
    }

    let diff = compare(&before, &after);
    assert_eq!(diff.total_changes(), 10);
    assert_eq!(diff.removed.len(), 5);
    assert_eq!(diff.added.len(), 5);
    assert!(diff.modified.is_empty());
    assert_eq!(diff.summary_string(), "+5 added, -5 removed, ~0 modified");
}

#[test]
fn stress_compare_complex_simultaneous_modifications() {
    // Construct single device baseline with every sub-structure populated
    let mut before_summary = make_port_summary("port0", 0, "Multi-Function Type-C Hub");
    before_summary.status = Status::Connected;
    before_summary.link_speed_mbps = 480;
    before_summary.power = PowerSummary {
        power_role: PowerRole::Sink,
        power_in_mw: 4500,
        power_out_mw: 0,
        contract_mw: 15000,
    };
    before_summary.properties = vec![
        ("usb_max_power_ma".into(), "500".into()),
        ("pd_contract".into(), "5.0V @ 3.00A".into()),
        ("data_role".into(), "device".into()),
        ("transport.protocol".into(), "usb2".into()),
        ("cable.trust.notes".into(), "clean".into()),
    ];
    before_summary.power_delivery = Some(PowerDeliveryPort {
        name: "pd0".into(),
        parent_port_number: 0,
        source_capabilities: vec![
            PowerDataObject {
                r#type: PdoType::FixedSupply,
                voltage_mv: 5000,
                max_voltage_mv: 5000,
                current_ma: 3000,
                power_mw: 15000,
                is_active: true,
                index: 1,
            },
            PowerDataObject {
                r#type: PdoType::FixedSupply,
                voltage_mv: 20000,
                max_voltage_mv: 20000,
                current_ma: 5000,
                power_mw: 100000,
                is_active: false,
                index: 2,
            },
        ],
        max_source_power_mw: 100000,
        ..Default::default()
    });
    before_summary.charging_diag = Some(ChargingDiagnostic {
        rule_id: "optimal",
        bottleneck: Bottleneck::Fine,
        is_warning: false,
        summary: "Normal charging".into(),
        detail: "Healthy baseline".into(),
        root_cause: RootCauseCategory::None,
        status: DiagnosticStatus::Healthy,
        confidence: DiagnosticConfidence::High,
        recommended_action: String::new(),
    });
    before_summary.cable = Some(CableInfo {
        cable_type: "active".into(),
        speed: Some(CableSpeed::Usb32Gen2),
        current_rating: Some(CableCurrent::FiveAmp),
        max_watts: 100,
        trust: CableTrust {
            trust_level: CableTrustLevel::KnownGood,
            notes: vec!["Certified".into()],
            ..Default::default()
        },
        ..Default::default()
    });
    before_summary.data_rate = Some(DataRateAssessment {
        verdict: DataRateVerdict::AtCapability,
        negotiated_mbps: 480,
        capable_mbps: 480,
        functional_floor_mbps: 480,
        capable_gen: "High-Speed".into(),
        summary: "Optimal High-Speed".into(),
        detail: String::new(),
        is_warning: false,
    });

    let before = Snapshot {
        summaries: vec![before_summary],
        ..Default::default()
    };

    // Construct after snapshot where EVERY single category shifted simultaneously
    let mut after_summary = make_port_summary("port0", 0, "Multi-Function Type-C Hub");
    after_summary.status = Status::Sourcing;
    after_summary.link_speed_mbps = 10000;
    after_summary.power = PowerSummary {
        power_role: PowerRole::Source,
        power_in_mw: 90000,
        power_out_mw: 15000,
        contract_mw: 100000,
    };
    after_summary.properties = vec![
        ("usb_max_power_ma".into(), "900".into()),
        ("pd_contract".into(), "20.0V @ 5.00A".into()),
        ("data_role".into(), "host".into()),
        ("transport.protocol".into(), "usb3.2".into()),
        ("cable.trust.notes".into(), "dirty_bits".into()),
    ];
    after_summary.power_delivery = Some(PowerDeliveryPort {
        name: "pd0".into(),
        parent_port_number: 0,
        source_capabilities: vec![
            PowerDataObject {
                r#type: PdoType::FixedSupply,
                voltage_mv: 5000,
                max_voltage_mv: 5000,
                current_ma: 3000,
                power_mw: 15000,
                is_active: false,
                index: 1,
            },
            PowerDataObject {
                r#type: PdoType::FixedSupply,
                voltage_mv: 20000,
                max_voltage_mv: 20000,
                current_ma: 5000,
                power_mw: 100000,
                is_active: true,
                index: 2,
            },
        ],
        max_source_power_mw: 100000,
        ..Default::default()
    });
    after_summary.charging_diag = Some(ChargingDiagnostic {
        rule_id: "cable_limits_charger",
        bottleneck: Bottleneck::CableLimit,
        is_warning: true,
        summary: "Cable bottleneck detected".into(),
        detail: "Current restricted".into(),
        root_cause: RootCauseCategory::Cable,
        status: DiagnosticStatus::Warning,
        confidence: DiagnosticConfidence::High,
        recommended_action: "Replace with 240W EPR cable".into(),
    });
    after_summary.cable = Some(CableInfo {
        cable_type: "passive".into(),
        speed: Some(CableSpeed::Usb20),
        current_rating: Some(CableCurrent::ThreeAmp),
        max_watts: 60,
        trust: CableTrust {
            trust_level: CableTrustLevel::Suspicious,
            reserved_bits_set: true,
            notes: vec!["Dirty reserved bits".into()],
            ..Default::default()
        },
        ..Default::default()
    });
    after_summary.data_rate = Some(DataRateAssessment {
        verdict: DataRateVerdict::Degraded,
        negotiated_mbps: 480,
        capable_mbps: 10000,
        functional_floor_mbps: 10000,
        capable_gen: "Gen 2x1".into(),
        summary: "SuperSpeed device linked at USB 2.0".into(),
        detail: "Cable missing SS pairs".into(),
        is_warning: true,
    });

    let after = Snapshot {
        summaries: vec![after_summary],
        ..Default::default()
    };

    let diff = compare(&before, &after);
    assert!(!diff.is_empty());
    assert_eq!(diff.modified.len(), 1);
    assert_eq!(diff.added.len(), 0);
    assert_eq!(diff.removed.len(), 0);
    assert!(diff.has_warnings(), "Warning flag must be raised");

    let m = &diff.modified[0];
    assert_eq!(m.id, "typec:port0");

    // 1. Status shift
    assert_eq!(
        m.status,
        Some(ValueChange::new(Status::Connected, Status::Sourcing))
    );

    // 2. Power shift
    let pwr = m.power.as_ref().expect("power diff present");
    assert_eq!(pwr.power_in_mw, Some(ValueChange::new(4500, 90000)));
    assert_eq!(pwr.power_out_mw, Some(ValueChange::new(0, 15000)));
    assert_eq!(pwr.contract_mw, Some(ValueChange::new(15000, 100000)));
    assert_eq!(
        pwr.power_role,
        Some(ValueChange::new(PowerRole::Sink, PowerRole::Source))
    );
    assert_eq!(pwr.usb_max_power_ma, Some(ValueChange::new(500, 900)));

    // 3. PD Contract shift
    let pd = m.pd_contract.as_ref().expect("pd contract diff present");
    assert_eq!(
        pd.contract_string,
        Some(ValueChange::new(
            "5.0V @ 3.00A".into(),
            "20.0V @ 5.00A".into()
        ))
    );
    assert_eq!(
        pd.active_pdo_index,
        Some(ValueChange::new(Some(0), Some(1)))
    );
    assert_eq!(pd.voltage_mv, Some(ValueChange::new(5000, 20000)));
    assert_eq!(pd.current_ma, Some(ValueChange::new(3000, 5000)));
    assert_eq!(pd.max_power_mw, Some(ValueChange::new(15000, 100000)));

    // 4. Link speed shift
    assert_eq!(m.link_speed_mbps, Some(ValueChange::new(480, 10000)));

    // 5. Diagnostic shift
    let diag = m.diagnostic.as_ref().expect("diagnostic diff present");
    assert_eq!(
        diag.status,
        Some(ValueChange::new(
            DiagnosticStatus::Healthy,
            DiagnosticStatus::Warning
        ))
    );
    assert_eq!(
        diag.bottleneck,
        Some(ValueChange::new(Bottleneck::Fine, Bottleneck::CableLimit))
    );
    assert_eq!(diag.is_warning, Some(ValueChange::new(false, true)));
    assert_eq!(
        diag.root_cause,
        Some(ValueChange::new(
            RootCauseCategory::None,
            RootCauseCategory::Cable
        ))
    );
    assert!(diag.is_newly_degraded());

    // 6. Cable shift
    let cable = m.cable.as_ref().expect("cable diff present");
    assert_eq!(
        cable.trust_level,
        Some(ValueChange::new(
            CableTrustLevel::KnownGood,
            CableTrustLevel::Suspicious
        ))
    );
    assert_eq!(cable.is_suspicious, Some(ValueChange::new(false, true)));
    assert_eq!(cable.max_watts, Some(ValueChange::new(Some(100), Some(60))));
    assert_eq!(
        cable.current_rating,
        Some(ValueChange::new(
            Some(CableCurrent::FiveAmp),
            Some(CableCurrent::ThreeAmp)
        ))
    );
    assert_eq!(
        cable.speed,
        Some(ValueChange::new(
            Some(CableSpeed::Usb32Gen2),
            Some(CableSpeed::Usb20)
        ))
    );
    assert!(cable
        .notes_added
        .contains(&"Dirty reserved bits".to_string()));
    assert!(cable.notes_removed.contains(&"Certified".to_string()));

    // 7. Data rate shift
    let dr = m.data_rate.as_ref().expect("data rate diff present");
    assert_eq!(
        dr.verdict,
        Some(ValueChange::new(
            DataRateVerdict::AtCapability,
            DataRateVerdict::Degraded
        ))
    );
    assert_eq!(dr.is_warning, Some(ValueChange::new(false, true)));

    // 8. Curated properties
    assert!(m.property_changes.iter().any(|p| p.key == "data_role"
        && p.change == ValueChange::new(Some("device".into()), Some("host".into()))));
    assert!(m
        .property_changes
        .iter()
        .any(|p| p.key == "transport.protocol"
            && p.change == ValueChange::new(Some("usb2".into()), Some("usb3.2".into()))));

    // Verify Display ASCII diff output
    let rendered = diff.render_text();
    assert!(rendered.contains("~ [typec:port0]"));
    assert!(rendered.contains("Power In:"));
    assert!(rendered.contains("->"));
    assert!(rendered.contains("Contract Ceiling:"));
    assert!(rendered.contains("Bottleneck:"));
    assert!(rendered.contains("Cable Trust:"));
}

#[test]
fn stress_compare_extreme_boundary_values() {
    let mut before = Snapshot::new();
    let mut dev_max = make_usb_summary("1-1", u32::MAX, "Max Boundary Device", "Vendor", "Prod");
    dev_max.power.power_in_mw = u32::MAX;
    dev_max.power.contract_mw = u32::MAX;
    before.summaries.push(dev_max);

    let mut after = Snapshot::new();
    let mut dev_zero = make_usb_summary("1-1", 0, "Zero Boundary Device", "Vendor", "Prod");
    dev_zero.power.power_in_mw = 0;
    dev_zero.power.contract_mw = 0;
    after.summaries.push(dev_zero);

    let diff = compare(&before, &after);
    assert_eq!(diff.modified.len(), 1);
    let m = &diff.modified[0];
    assert_eq!(m.link_speed_mbps, Some(ValueChange::new(u32::MAX, 0)));
    let p = m.power.as_ref().unwrap();
    assert_eq!(p.power_in_mw, Some(ValueChange::new(u32::MAX, 0)));
    assert_eq!(p.contract_mw, Some(ValueChange::new(u32::MAX, 0)));
}

// =========================================================================
// 2. Historical Snapshot Ring Buffer Tests (`src/history.rs`)
// =========================================================================

#[test]
fn stress_history_massive_sequential_pushes_1000() {
    let capacity = 10;
    let mut history = SnapshotHistory::with_capacity(capacity);
    assert_eq!(history.capacity(), capacity);
    assert_eq!(history.len(), 0);
    assert_eq!(history.total_pushed(), 0);

    // Push 1,000 snapshots sequentially
    for seq in 0..1000 {
        let snap = Snapshot {
            summaries: vec![make_usb_summary(
                "1-1",
                480 + (seq as u32 % 5) * 1000,
                &format!("Device Rev {seq}"),
                "Vendor",
                "Prod",
            )],
            ..Default::default()
        };
        let transitions = history.push(snap, 1_000_000 + seq);
        if seq == 0 {
            assert!(
                transitions.is_empty(),
                "First baseline push must have 0 transitions"
            );
        } else {
            assert!(
                !transitions.is_empty() || (seq % 5 == 0),
                "Transitions expected on speed shift"
            );
        }
    }

    // Verify ring buffer capacity and eviction invariants
    assert_eq!(
        history.len(),
        capacity,
        "Ring buffer length must equal capacity"
    );
    assert_eq!(
        history.total_pushed(),
        1000,
        "Total pushed count must be 1000"
    );

    let oldest = history.oldest().expect("oldest entry exists");
    let latest = history.latest().expect("latest entry exists");
    assert_eq!(oldest.sequence, 990, "Oldest retained sequence must be 990");
    assert_eq!(latest.sequence, 999, "Latest retained sequence must be 999");

    // Verify sequence indexing from 0 to 9 in the retained buffer
    let retained_seqs: Vec<u64> = history.entries().map(|e| e.sequence).collect();
    assert_eq!(
        retained_seqs,
        vec![990, 991, 992, 993, 994, 995, 996, 997, 998, 999]
    );

    // Verify get_by_sequence eviction semantics
    assert!(
        history.get_by_sequence(989).is_none(),
        "Evicted sequence 989 must return None"
    );
    assert!(
        history.get_by_sequence(990).is_some(),
        "Retained sequence 990 must return Some"
    );
    assert!(
        history.get_by_sequence(999).is_some(),
        "Latest sequence 999 must return Some"
    );
    assert!(
        history.get_by_sequence(1000).is_none(),
        "Future sequence 1000 must return None"
    );

    // Verify diff_latest compares 998 and 999
    let diff = history.diff_latest().expect("diff_latest succeeds");
    assert_eq!(diff.modified.len(), 1);
    assert_eq!(diff.modified[0].id, "usb:1-1");
}

#[test]
fn stress_history_zero_capacity_and_capacity_one() {
    // A: Zero capacity must be clamped to 1
    let mut history_zero = SnapshotHistory::with_capacity(0);
    assert_eq!(
        history_zero.capacity(),
        1,
        "Capacity 0 must be clamped to 1"
    );

    // Push 5 snapshots into capacity 1 history
    for i in 0..5 {
        let snap = Snapshot {
            summaries: vec![make_usb_summary(
                "1-1",
                480 + i * 500,
                &format!("Dev {i}"),
                "V",
                "P",
            )],
            ..Default::default()
        };
        let trans = history_zero.push(snap, 1000 * (i as u64 + 1));
        if i > 0 {
            // Even though capacity is 1, transitions must still be detected against previous before eviction!
            assert_eq!(trans.len(), 1);
            assert!(matches!(trans[0], StateTransition::LinkSpeedChanged(_)));
        }
    }

    assert_eq!(history_zero.len(), 1);
    assert_eq!(history_zero.total_pushed(), 5);
    assert_eq!(history_zero.oldest().unwrap().sequence, 4);
    assert_eq!(history_zero.latest().unwrap().sequence, 4);

    // diff_latest requires at least 2 entries, so it returns None when capacity == 1
    assert!(history_zero.diff_latest().is_none());

    // set_capacity(0) must also clamp to 1
    history_zero.set_capacity(0);
    assert_eq!(history_zero.capacity(), 1);
    assert_eq!(history_zero.len(), 1);
}

#[test]
fn stress_history_dynamic_resizing_and_clear() {
    let mut history = SnapshotHistory::with_capacity(20);

    for i in 0..15 {
        let snap = Snapshot {
            summaries: vec![make_usb_summary("1-1", 480, "Dev", "V", "P")],
            ..Default::default()
        };
        history.push(snap, 1000 * i);
    }
    assert_eq!(history.len(), 15);
    assert_eq!(history.oldest().unwrap().sequence, 0);
    assert_eq!(history.latest().unwrap().sequence, 14);

    // Resize down to 5: oldest 10 entries evicted
    history.set_capacity(5);
    assert_eq!(history.capacity(), 5);
    assert_eq!(history.len(), 5);
    assert_eq!(history.oldest().unwrap().sequence, 10);
    assert_eq!(history.latest().unwrap().sequence, 14);

    // Resize up to 30: retained entries untouched
    history.set_capacity(30);
    assert_eq!(history.capacity(), 30);
    assert_eq!(history.len(), 5);
    assert_eq!(history.oldest().unwrap().sequence, 10);
    assert_eq!(history.latest().unwrap().sequence, 14);

    // Clear history: resets len and total_pushed to 0
    history.clear();
    assert_eq!(history.len(), 0);
    assert_eq!(history.total_pushed(), 0);
    assert!(history.is_empty());
    assert!(history.oldest().is_none());
    assert!(history.latest().is_none());
}

// =========================================================================
// 3. State Transitions Engine Tests (`detect_transitions`)
// =========================================================================

#[test]
fn stress_all_nine_state_transitions_triggered_and_verified() {
    let mut history = SnapshotHistory::with_capacity(20);

    // -------------------------------------------------------------------------
    // Step 0: Baseline Snapshot (empty) -> 0 transitions
    // -------------------------------------------------------------------------
    let snap0 = Snapshot::new();
    let trans0 = history.push(snap0, 1000);
    assert!(trans0.is_empty(), "Baseline snapshot yields 0 transitions");

    // -------------------------------------------------------------------------
    // Step 1: DeviceConnected (Dev 1 arrives)
    // -------------------------------------------------------------------------
    let mut dev1 = make_usb_summary("1-1.1", 480, "USB Flash Drive", "SanDisk", "Cruzer");
    let snap1 = Snapshot {
        summaries: vec![dev1.clone()],
        ..Default::default()
    };
    let trans1 = history.push(snap1, 2000);
    assert_eq!(trans1.len(), 1);
    match &trans1[0] {
        StateTransition::DeviceConnected(e) => {
            assert_eq!(e.id, "usb:1-1.1");
            assert_eq!(e.headline, "USB Flash Drive");
            assert_eq!(e.link_speed_mbps, 480);
            assert_eq!(e.status, Status::Connected);
            let disp = format!("{}", trans1[0]);
            assert!(disp.starts_with("[+] CONNECTED: usb:1-1.1"));
        }
        _ => panic!("Expected DeviceConnected"),
    }

    // -------------------------------------------------------------------------
    // Step 2: LinkSpeedChanged (Dev 1 upgrades from 480 Mbps to 5000 Mbps)
    // -------------------------------------------------------------------------
    dev1.link_speed_mbps = 5000;
    dev1.subtitle = "5000 Mbps".into();
    let snap2 = Snapshot {
        summaries: vec![dev1.clone()],
        ..Default::default()
    };
    let trans2 = history.push(snap2, 3000);
    assert_eq!(trans2.len(), 1);
    match &trans2[0] {
        StateTransition::LinkSpeedChanged(e) => {
            assert_eq!(e.id, "usb:1-1.1");
            assert_eq!(e.old_speed_mbps, 480);
            assert_eq!(e.new_speed_mbps, 5000);
            let disp = format!("{}", trans2[0]);
            assert!(disp.starts_with("[~] SPEED CHANGED: usb:1-1.1"));
        }
        _ => panic!("Expected LinkSpeedChanged"),
    }

    // -------------------------------------------------------------------------
    // Step 3: PowerChanged (Dev 1 power_in_mw jumps from 4500 to 9000 mW)
    // -------------------------------------------------------------------------
    dev1.power.power_in_mw = 9000;
    let snap3 = Snapshot {
        summaries: vec![dev1.clone()],
        ..Default::default()
    };
    let trans3 = history.push(snap3, 4000);
    assert_eq!(trans3.len(), 1);
    match &trans3[0] {
        StateTransition::PowerChanged(e) => {
            assert_eq!(e.id, "usb:1-1.1");
            assert_eq!(e.old_power_in_mw, 4500);
            assert_eq!(e.new_power_in_mw, 9000);
            let disp = format!("{}", trans3[0]);
            assert!(disp.starts_with("[~] POWER CHANGED: usb:1-1.1"));
        }
        _ => panic!("Expected PowerChanged"),
    }

    // -------------------------------------------------------------------------
    // Step 4: Add Type-C Port 0 with initial PD contract -> DeviceConnected
    // -------------------------------------------------------------------------
    let mut port0 = make_port_summary("port0", 0, "Type-C Port 0");
    port0.power.power_role = PowerRole::Source;
    port0.power.contract_mw = 15000;
    port0.power_delivery = Some(PowerDeliveryPort {
        name: "pd0".into(),
        parent_port_number: 0,
        source_capabilities: vec![
            PowerDataObject {
                r#type: PdoType::FixedSupply,
                voltage_mv: 5000,
                max_voltage_mv: 5000,
                current_ma: 3000,
                power_mw: 15000,
                is_active: true,
                index: 1,
            },
            PowerDataObject {
                r#type: PdoType::FixedSupply,
                voltage_mv: 20000,
                max_voltage_mv: 20000,
                current_ma: 5000,
                power_mw: 100000,
                is_active: false,
                index: 2,
            },
        ],
        max_source_power_mw: 100000,
        ..Default::default()
    });
    let snap4 = Snapshot {
        summaries: vec![dev1.clone(), port0.clone()],
        ..Default::default()
    };
    let trans4 = history.push(snap4, 5000);
    assert!(trans4
        .iter()
        .any(|t| matches!(t, StateTransition::DeviceConnected(e) if e.id == "typec:port0")));

    // -------------------------------------------------------------------------
    // Step 5: ContractRenegotiated (Port 0 renegotiates from 5V@3A/15W to 20V@5A/100W)
    // -------------------------------------------------------------------------
    port0.power.contract_mw = 100000;
    port0.power_delivery = Some(PowerDeliveryPort {
        name: "pd0".into(),
        parent_port_number: 0,
        source_capabilities: vec![
            PowerDataObject {
                r#type: PdoType::FixedSupply,
                voltage_mv: 5000,
                max_voltage_mv: 5000,
                current_ma: 3000,
                power_mw: 15000,
                is_active: false,
                index: 1,
            },
            PowerDataObject {
                r#type: PdoType::FixedSupply,
                voltage_mv: 20000,
                max_voltage_mv: 20000,
                current_ma: 5000,
                power_mw: 100000,
                is_active: true,
                index: 2,
            },
        ],
        max_source_power_mw: 100000,
        ..Default::default()
    });
    let snap5 = Snapshot {
        summaries: vec![dev1.clone(), port0.clone()],
        ..Default::default()
    };
    let trans5 = history.push(snap5, 6000);
    let contract_event = trans5
        .iter()
        .find(|t| matches!(t, StateTransition::ContractRenegotiated(_)))
        .expect("ContractRenegotiated event must be present");
    match contract_event {
        StateTransition::ContractRenegotiated(e) => {
            assert_eq!(e.id, "typec:port0");
            assert_eq!(e.old_power_mw, 15000);
            assert_eq!(e.new_power_mw, 100000);
            assert_eq!(e.old_voltage_mv, 5000);
            assert_eq!(e.new_voltage_mv, 20000);
            assert_eq!(e.old_current_ma, 3000);
            assert_eq!(e.new_current_ma, 5000);
            assert_eq!(e.old_pdo_index, Some(1));
            assert_eq!(e.new_pdo_index, Some(2));
            let disp = format!("{contract_event}");
            assert!(disp.starts_with("[~] CONTRACT RENEGOTIATED: typec:port0"));
        }
        _ => unreachable!(),
    }

    // -------------------------------------------------------------------------
    // Step 6: BottleneckChanged (Warning Raised: charger undersized)
    // -------------------------------------------------------------------------
    port0.charging_diag = Some(ChargingDiagnostic {
        rule_id: "charger_undersized",
        bottleneck: Bottleneck::ChargerLimit,
        is_warning: true,
        summary: "Charger undersized for 100W sink".into(),
        detail: "Supplying 15W".into(),
        root_cause: RootCauseCategory::Charger,
        status: DiagnosticStatus::Warning,
        confidence: DiagnosticConfidence::High,
        recommended_action: "Use 100W charger".into(),
    });
    let snap6 = Snapshot {
        summaries: vec![dev1.clone(), port0.clone()],
        ..Default::default()
    };
    let trans6 = history.push(snap6, 7000);
    let bottleneck_event = trans6
        .iter()
        .find(|t| matches!(t, StateTransition::BottleneckChanged(_)))
        .expect("BottleneckChanged event must be present");
    match bottleneck_event {
        StateTransition::BottleneckChanged(e) => {
            assert_eq!(e.id, "typec:port0");
            assert_eq!(e.new_bottleneck, Some(Bottleneck::ChargerLimit));
            assert_eq!(e.new_status, Some(DiagnosticStatus::Warning));
            assert!(e.warning_raised, "Warning must be flagged as raised");
            assert!(!e.warning_cleared);
            let disp = format!("{bottleneck_event}");
            assert!(disp.starts_with("[!] BOTTLENECK DEGRADED: typec:port0"));
        }
        _ => unreachable!(),
    }

    // -------------------------------------------------------------------------
    // Step 7: BottleneckChanged (Warning Cleared / Resolved)
    // -------------------------------------------------------------------------
    port0.charging_diag = Some(ChargingDiagnostic {
        rule_id: "optimal",
        bottleneck: Bottleneck::Fine,
        is_warning: false,
        summary: "Optimal 100W charging".into(),
        detail: String::new(),
        root_cause: RootCauseCategory::None,
        status: DiagnosticStatus::Healthy,
        confidence: DiagnosticConfidence::High,
        recommended_action: String::new(),
    });
    let snap7 = Snapshot {
        summaries: vec![dev1.clone(), port0.clone()],
        ..Default::default()
    };
    let trans7 = history.push(snap7, 8000);
    let bottleneck_resolved = trans7
        .iter()
        .find(|t| matches!(t, StateTransition::BottleneckChanged(_)))
        .expect("BottleneckChanged resolved event");
    match bottleneck_resolved {
        StateTransition::BottleneckChanged(e) => {
            assert_eq!(e.id, "typec:port0");
            assert_eq!(e.new_bottleneck, Some(Bottleneck::Fine));
            assert_eq!(e.new_status, Some(DiagnosticStatus::Healthy));
            assert!(!e.warning_raised);
            assert!(e.warning_cleared, "Warning must be flagged as cleared");
            let disp = format!("{bottleneck_resolved}");
            assert!(disp.starts_with("[*] BOTTLENECK RESOLVED: typec:port0"));
        }
        _ => unreachable!(),
    }

    // -------------------------------------------------------------------------
    // Step 8: CableWarningChanged (Warning Raised: suspicious cable)
    // -------------------------------------------------------------------------
    port0.cable = Some(CableInfo {
        cable_type: "passive".into(),
        speed: Some(CableSpeed::Usb20),
        current_rating: Some(CableCurrent::ThreeAmp),
        max_watts: 60,
        trust: CableTrust {
            trust_level: CableTrustLevel::Suspicious,
            reserved_bits_set: true,
            notes: vec!["Dirty reserved bits".into()],
            ..Default::default()
        },
        ..Default::default()
    });
    let snap8 = Snapshot {
        summaries: vec![dev1.clone(), port0.clone()],
        ..Default::default()
    };
    let trans8 = history.push(snap8, 9000);
    let cable_event = trans8
        .iter()
        .find(|t| matches!(t, StateTransition::CableWarningChanged(_)))
        .expect("CableWarningChanged event must be present");
    match cable_event {
        StateTransition::CableWarningChanged(e) => {
            assert_eq!(e.id, "typec:port0");
            assert_eq!(e.new_trust_level, Some(CableTrustLevel::Suspicious));
            assert!(e.warning_raised);
            assert!(!e.warning_cleared);
            let disp = format!("{cable_event}");
            assert!(disp.starts_with("[!] CABLE WARNING RAISED: typec:port0"));
        }
        _ => unreachable!(),
    }

    // -------------------------------------------------------------------------
    // Step 9: CableWarningChanged (Warning Cleared: cable swapped for certified)
    // -------------------------------------------------------------------------
    port0.cable = Some(CableInfo {
        cable_type: "active".into(),
        speed: Some(CableSpeed::Usb32Gen2),
        current_rating: Some(CableCurrent::FiveAmp),
        max_watts: 100,
        trust: CableTrust {
            trust_level: CableTrustLevel::KnownGood,
            reserved_bits_set: false,
            notes: vec![],
            ..Default::default()
        },
        ..Default::default()
    });
    let snap9 = Snapshot {
        summaries: vec![dev1.clone(), port0.clone()],
        ..Default::default()
    };
    let trans9 = history.push(snap9, 10000);
    let cable_cleared = trans9
        .iter()
        .find(|t| matches!(t, StateTransition::CableWarningChanged(_)))
        .expect("CableWarningChanged cleared event");
    match cable_cleared {
        StateTransition::CableWarningChanged(e) => {
            assert_eq!(e.id, "typec:port0");
            assert_eq!(e.new_trust_level, Some(CableTrustLevel::KnownGood));
            assert!(!e.warning_raised);
            assert!(e.warning_cleared);
            let disp = format!("{cable_cleared}");
            assert!(disp.starts_with("[*] CABLE WARNING CLEARED: typec:port0"));
        }
        _ => unreachable!(),
    }

    // -------------------------------------------------------------------------
    // Step 10: PortRoleChanged (Port 0 flips status, power_role, and data_role)
    // -------------------------------------------------------------------------
    port0.status = Status::Sourcing;
    port0.power.power_role = PowerRole::Sink;
    port0.properties = vec![("data_role".into(), "device".into())];
    let snap10 = Snapshot {
        summaries: vec![dev1.clone(), port0.clone()],
        ..Default::default()
    };
    let trans10 = history.push(snap10, 11000);
    let port_event = trans10
        .iter()
        .find(|t| matches!(t, StateTransition::PortRoleChanged(_)))
        .expect("PortRoleChanged event must be present");
    match port_event {
        StateTransition::PortRoleChanged(e) => {
            assert_eq!(e.id, "typec:port0");
            assert_eq!(e.old_status, Status::Connected);
            assert_eq!(e.new_status, Status::Sourcing);
            assert_eq!(e.old_power_role, PowerRole::Source);
            assert_eq!(e.new_power_role, PowerRole::Sink);
            assert_eq!(e.old_data_role, Some("host".into()));
            assert_eq!(e.new_data_role, Some("device".into()));
            let disp = format!("{port_event}");
            assert!(disp.starts_with("[~] PORT ROLE CHANGED: typec:port0"));
        }
        _ => unreachable!(),
    }

    // -------------------------------------------------------------------------
    // Step 11: DataRateChanged (Degraded: Dev 1 drops from 5000 to 480 Mbps)
    // -------------------------------------------------------------------------
    dev1.data_rate = Some(DataRateAssessment {
        verdict: DataRateVerdict::Degraded,
        negotiated_mbps: 480,
        capable_mbps: 5000,
        functional_floor_mbps: 5000,
        capable_gen: "Gen 1x1".into(),
        summary: "SuperSpeed device linked at USB 2.0".into(),
        detail: String::new(),
        is_warning: true,
    });
    let snap11 = Snapshot {
        summaries: vec![dev1.clone(), port0.clone()],
        ..Default::default()
    };
    let trans11 = history.push(snap11, 12000);
    let dr_event = trans11
        .iter()
        .find(|t| matches!(t, StateTransition::DataRateChanged(_)))
        .expect("DataRateChanged degraded event");
    match dr_event {
        StateTransition::DataRateChanged(e) => {
            assert_eq!(e.id, "usb:1-1.1");
            assert_eq!(e.new_verdict, Some(DataRateVerdict::Degraded));
            assert!(e.is_warning);
            assert!(!e.is_restored);
            let disp = format!("{dr_event}");
            assert!(disp.starts_with("[!] DATA-RATE DEGRADED: usb:1-1.1"));
        }
        _ => unreachable!(),
    }

    // -------------------------------------------------------------------------
    // Step 12: DataRateChanged (Restored: Dev 1 returns to 5000 Mbps)
    // -------------------------------------------------------------------------
    dev1.data_rate = Some(DataRateAssessment {
        verdict: DataRateVerdict::AtCapability,
        negotiated_mbps: 5000,
        capable_mbps: 5000,
        functional_floor_mbps: 5000,
        capable_gen: "Gen 1x1".into(),
        summary: "Optimal link rate".into(),
        detail: String::new(),
        is_warning: false,
    });
    let snap12 = Snapshot {
        summaries: vec![dev1.clone(), port0.clone()],
        ..Default::default()
    };
    let trans12 = history.push(snap12, 13000);
    let dr_restored = trans12
        .iter()
        .find(|t| matches!(t, StateTransition::DataRateChanged(_)))
        .expect("DataRateChanged restored event");
    match dr_restored {
        StateTransition::DataRateChanged(e) => {
            assert_eq!(e.id, "usb:1-1.1");
            assert_eq!(e.new_verdict, Some(DataRateVerdict::AtCapability));
            assert!(!e.is_warning);
            assert!(e.is_restored);
            let disp = format!("{dr_restored}");
            assert!(disp.starts_with("[*] DATA-RATE RESTORED: usb:1-1.1"));
        }
        _ => unreachable!(),
    }

    // -------------------------------------------------------------------------
    // Step 13: DeviceDisconnected (Dev 1 removed)
    // -------------------------------------------------------------------------
    let snap13 = Snapshot {
        summaries: vec![port0.clone()],
        ..Default::default()
    };
    let trans13 = history.push(snap13, 14000);
    let disc_event = trans13
        .iter()
        .find(|t| matches!(t, StateTransition::DeviceDisconnected(_)))
        .expect("DeviceDisconnected event must be present");
    match disc_event {
        StateTransition::DeviceDisconnected(e) => {
            assert_eq!(e.id, "usb:1-1.1");
            assert_eq!(e.headline, "USB Flash Drive");
            assert_eq!(e.category, Category::UsbDevice);
            let disp = format!("{disc_event}");
            assert!(disp.starts_with("[-] DISCONNECTED: usb:1-1.1"));
        }
        _ => unreachable!(),
    }
}

#[test]
fn stress_oscillating_connections_plug_unplug_50_cycles() {
    let mut history = SnapshotHistory::with_capacity(60);
    let dev = make_usb_summary("1-2", 480, "Oscillating Mouse", "Logitech", "G502");

    // Establish baseline
    history.push(Snapshot::new(), 1000);

    for cycle in 1..=50 {
        if cycle % 2 == 1 {
            // Plug in
            let snap = Snapshot {
                summaries: vec![dev.clone()],
                ..Default::default()
            };
            let transitions = history.push(snap, 1000 + cycle * 100);
            assert_eq!(transitions.len(), 1);
            assert!(
                matches!(transitions[0], StateTransition::DeviceConnected(_)),
                "Cycle {cycle}: expected DeviceConnected"
            );
            assert_eq!(transitions[0].id(), "usb:1-2");
        } else {
            // Unplug
            let snap = Snapshot::new();
            let transitions = history.push(snap, 1000 + cycle * 100);
            assert_eq!(transitions.len(), 1);
            assert!(
                matches!(transitions[0], StateTransition::DeviceDisconnected(_)),
                "Cycle {cycle}: expected DeviceDisconnected"
            );
            assert_eq!(transitions[0].id(), "usb:1-2");
        }
    }

    assert_eq!(history.total_pushed(), 51);
    assert_eq!(history.len(), 51);

    let all_trans = history.all_transitions();
    assert_eq!(all_trans.len(), 50);

    // Verify alternating pattern
    for (idx, trans) in all_trans.iter().enumerate() {
        if idx % 2 == 0 {
            assert!(matches!(trans, StateTransition::DeviceConnected(_)));
        } else {
            assert!(matches!(trans, StateTransition::DeviceDisconnected(_)));
        }
    }
}

#[test]
fn stress_history_queries_and_timeline() {
    let mut history = SnapshotHistory::with_capacity(10);
    let dev_a = make_usb_summary("1-1", 480, "Device A", "V", "P");
    let dev_b = make_usb_summary("1-2", 5000, "Device B", "V", "P");

    history.push(Snapshot::new(), 1000);
    history.push(
        Snapshot {
            summaries: vec![dev_a.clone()],
            ..Default::default()
        },
        2000,
    );
    history.push(
        Snapshot {
            summaries: vec![dev_a.clone(), dev_b.clone()],
            ..Default::default()
        },
        3000,
    );

    // Test filter_transitions by ID
    let dev_a_transitions = history.filter_transitions(|t| t.id() == "usb:1-1");
    assert_eq!(dev_a_transitions.len(), 1);

    let dev_b_transitions = history.filter_transitions(|t| t.id() == "usb:1-2");
    assert_eq!(dev_b_transitions.len(), 1);

    // Test transitions_since
    let after_seq1 = history.transitions_since(1);
    assert_eq!(after_seq1.len(), 1);
    assert_eq!(after_seq1[0].id(), "usb:1-2");

    let after_seq2 = history.transitions_since(2);
    assert!(after_seq2.is_empty());

    // Test timeline
    let timeline = history.timeline();
    assert_eq!(timeline.len(), 2);
    assert_eq!(timeline[0].sequence, 1);
    assert_eq!(timeline[0].timestamp_ms, 2000);
    assert_eq!(timeline[1].sequence, 2);
    assert_eq!(timeline[1].timestamp_ms, 3000);

    // Test diff_range
    let range_diff = history.diff_range(0, 2).expect("diff_range exists");
    assert_eq!(range_diff.added.len(), 2);
    assert!(range_diff.removed.is_empty());
}

// =========================================================================
// 4. Serde Round-Trip & Invariants Tests
// =========================================================================

#[test]
fn stress_serde_roundtrip_all_models() {
    let mut snap = make_populated_snapshot(3);
    snap.metadata = Some(SnapshotMetadata::new(1729512345000, 42, "roundtrip-host"));

    // Serialize and deserialize Snapshot
    let snap_json = serde_json::to_string(&snap).expect("serialize snapshot");
    let snap_deser: Snapshot = serde_json::from_str(&snap_json).expect("deserialize snapshot");
    assert_eq!(snap, snap_deser);

    // Compute diff and round-trip SnapshotDiff
    let empty_snap = Snapshot::new();
    let diff = compare(&empty_snap, &snap);
    let diff_json = serde_json::to_string(&diff).expect("serialize diff");
    let diff_deser: usbeehive::compare::SnapshotDiff =
        serde_json::from_str(&diff_json).expect("deserialize diff");
    assert_eq!(diff, diff_deser);

    // Push into history and round-trip SnapshotHistory
    let mut history = SnapshotHistory::with_capacity(5);
    history.push(empty_snap, 1000);
    history.push(snap, 2000);

    let hist_json = serde_json::to_string(&history).expect("serialize history");
    let hist_deser: SnapshotHistory =
        serde_json::from_str(&hist_json).expect("deserialize history");
    assert_eq!(history, hist_deser);

    // Round-trip Timeline
    let timeline = history.timeline();
    let tl_json = serde_json::to_string(&timeline).expect("serialize timeline");
    let tl_deser: Vec<usbeehive::history::TimelineEntry> =
        serde_json::from_str(&tl_json).expect("deserialize timeline");
    assert_eq!(timeline, tl_deser);
}
