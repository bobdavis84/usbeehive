//! Comprehensive integration tests for Milestone 2:
//! - Pure-core Snapshot model & Serde round-trip
//! - Snapshot comparison engine (`compare`, `DeviceDifference`, `SnapshotDiff`)
//! - Historical tracking ring-buffer & 9 State Transition events (`SnapshotHistory`)

use usbeehive::bos::{BosDescriptors, DataRateAssessment, DataRateVerdict};
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
use usbeehive::typec::{TypeCCable, TypeCIdentity, TypeCPort};
use usbeehive::usb::UsbDevice;

fn make_sample_usb_summary(bus_port: &str, speed: u32, headline: &str) -> DeviceSummary {
    DeviceSummary {
        category: Category::UsbDevice,
        device_class: DeviceClass::Storage,
        device_subclass: "SCSI".into(),
        status: Status::Connected,
        headline: headline.into(),
        subtitle: "10 Gbps".into(),
        icon: "drive-removable-media".into(),
        vendor: "TestVendor".into(),
        product: "FastSSD".into(),
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
            product: headline.into(),
            manufacturer: "TestVendor".into(),
            ..Default::default()
        }),
        properties: vec![
            ("bus_port".into(), bus_port.into()),
            ("transport.protocol".into(), "usb".into()),
        ],
    }
}

fn make_sample_port_summary(port_name: &str, port_number: i32, headline: &str) -> DeviceSummary {
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
        power: PowerSummary::default(),
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
        properties: vec![],
    }
}

// -----------------------------------------------------------------------------
// Test 1: Snapshot Model & Serialization Round-Trip
// -----------------------------------------------------------------------------

#[test]
fn test_snapshot_model_roundtrip_all_fields() {
    let mut snap = Snapshot::new().with_metadata(SnapshotMetadata::new(
        1729512345678,
        100,
        "test-machine-linux",
    ));

    // USB device
    snap.usb_devices.push(UsbDevice {
        bus_port: "1-1.2".into(),
        speed: 5000,
        vendor_id: 0x0781,
        product_id: 0x5583,
        product: "Ultra Fit".into(),
        manufacturer: "SanDisk".into(),
        serial: "0123456789".into(),
        bos: Some(BosDescriptors {
            num_device_caps: 1,
            total_length: 22,
            capabilities: vec![],
            truncated: false,
        }),
        ..Default::default()
    });

    // Type-C port
    snap.typec_ports.push(TypeCPort {
        port_number: 0,
        port_name: "port0".into(),
        data_role: "host".into(),
        power_role: "source".into(),
        cable: Some(TypeCCable {
            r#type: "passive".into(),
            plug_type: "type-c".into(),
            identity: Some(TypeCIdentity {
                vendor_id: 0x0bda,
                product_id: 0x0001,
                vdos: vec![0x18000bda, 0, 0, 0x00000021],
            }),
            ..Default::default()
        }),
        ..Default::default()
    });

    // Power Delivery port
    snap.pd_ports.push(PowerDeliveryPort {
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

    // Thunderbolt router
    snap.thunderbolt_routers.push(ThunderboltRouter {
        route: "0-0".into(),
        vendor_name: "Intel".into(),
        device_name: "Host".into(),
        is_host: true,
        generation: 4,
        ..Default::default()
    });

    // DeviceSummary with diagnostic, cable, data_rate
    let mut summary = make_sample_usb_summary("1-1.2", 5000, "SanDisk Ultra Fit");
    summary.charging_diag = Some(ChargingDiagnostic {
        // Use the canonical rule: unknown IDs intentionally deserialize as UNKNOWN.
        rule_id: "CABLE_LIMIT",
        bottleneck: Bottleneck::CableLimit,
        root_cause: RootCauseCategory::Cable,
        status: DiagnosticStatus::Warning,
        confidence: DiagnosticConfidence::High,
        is_warning: true,
        summary: "Cable rated for 60W, charger can deliver 100W".into(),
        detail: "Current limited to 3A instead of 5A".into(),
        recommended_action: "Replace with 100W/5A certified Type-C cable".into(),
    });
    summary.cable = Some(CableInfo {
        cable_type: "passive".into(),
        speed: Some(CableSpeed::Usb32Gen1),
        current_rating: Some(CableCurrent::ThreeAmp),
        max_watts: 60,
        trust: CableTrust {
            trust_level: CableTrustLevel::KnownGood,
            ..Default::default()
        },
        ..Default::default()
    });
    summary.data_rate = Some(DataRateAssessment {
        verdict: DataRateVerdict::Degraded,
        negotiated_mbps: 5000,
        capable_mbps: 10000,
        functional_floor_mbps: 10000,
        capable_gen: "Gen 2x1".into(),
        summary: "Device capable of 10 Gbps connected at 5 Gbps".into(),
        detail: "Connected to USB 3.0 port instead of USB 3.2 Gen 2".into(),
        is_warning: true,
    });
    snap.summaries.push(summary);

    // Serialization & Deserialization round-trip
    let json = serde_json::to_string_pretty(&snap).expect("serialize snapshot");
    assert!(!json.is_empty());
    assert!(json.contains("test-machine-linux"));
    assert!(json.contains("CABLE_LIMIT"));

    let deserialized: Snapshot = serde_json::from_str(&json).expect("deserialize snapshot");
    assert_eq!(snap, deserialized);
    assert_eq!(deserialized.len(), 1);
    assert!(!deserialized.is_empty());
    assert!(deserialized.find_summary("usb:1-1.2").is_some());
    assert!(deserialized.find_usb_device("1-1.2").is_some());
    assert!(deserialized.find_typec_port(0).is_some());
    assert!(deserialized.find_pd_port("pd0").is_some());
}

// -----------------------------------------------------------------------------
// Test 2: Snapshot Comparison Engine
// -----------------------------------------------------------------------------

#[test]
fn test_compare_engine_added_removed_modified() {
    let mut before = Snapshot::new();
    let dev_keep = make_sample_usb_summary("1-1.1", 480, "Mouse");
    let dev_remove = make_sample_usb_summary("1-1.2", 5000, "Old Flash Drive");
    before.summaries.push(dev_keep);
    before.summaries.push(dev_remove);

    let mut after = Snapshot::new();
    let mut dev_keep_modified = make_sample_usb_summary("1-1.1", 480, "Mouse");
    // Modify Mouse properties and power
    dev_keep_modified.power.power_in_mw = 2500;
    dev_keep_modified
        .properties
        .push(("transport.speed".into(), "high-speed".into()));
    let dev_add = make_sample_usb_summary("1-1.3", 10000, "New SSD");
    after.summaries.push(dev_keep_modified);
    after.summaries.push(dev_add);

    let diff = compare(&before, &after);
    assert!(!diff.is_empty());
    assert_eq!(diff.added.len(), 1);
    assert_eq!(diff.added[0].id(), "usb:1-1.3");
    assert_eq!(diff.removed.len(), 1);
    assert_eq!(diff.removed[0].id(), "usb:1-1.2");
    assert_eq!(diff.modified.len(), 1);
    assert_eq!(diff.modified[0].id, "usb:1-1.1");

    let mod_entry = &diff.modified[0];
    let power_diff = mod_entry.power.as_ref().expect("power diff present");
    assert_eq!(power_diff.power_in_mw, Some(ValueChange::new(4500, 2500)));

    assert!(mod_entry
        .property_changes
        .iter()
        .any(|p| p.key == "transport.speed"
            && p.change == ValueChange::new(None, Some("high-speed".to_string()))));

    // Verify text rendering contains (+1), (-1), (~1) and ->
    let rendered = diff.render_text();
    assert!(rendered.contains("(+"));
    assert!(rendered.contains("(-"));
    assert!(rendered.contains("(~"));
    assert!(rendered.contains("->"));
}

#[test]
fn test_compare_engine_cable_and_diagnostic_transitions() {
    let mut before = Snapshot::new();
    let mut dev = make_sample_port_summary("port0", 0, "Type-C Display");
    dev.cable = Some(CableInfo {
        cable_type: "passive".into(),
        speed: Some(CableSpeed::Usb20),
        current_rating: Some(CableCurrent::ThreeAmp),
        max_watts: 60,
        trust: CableTrust {
            trust_level: CableTrustLevel::Suspicious,
            ..Default::default()
        },
        ..Default::default()
    });
    dev.charging_diag = Some(ChargingDiagnostic {
        rule_id: "cable_limits_charger",
        bottleneck: Bottleneck::CableLimit,
        is_warning: true,
        summary: "Cable bottleneck".into(),
        root_cause: RootCauseCategory::Cable,
        status: DiagnosticStatus::Warning,
        confidence: DiagnosticConfidence::High,
        detail: "".into(),
        recommended_action: "".into(),
    });
    before.summaries.push(dev);

    let mut after = Snapshot::new();
    let mut dev_repaired = make_sample_port_summary("port0", 0, "Type-C Display");
    dev_repaired.cable = Some(CableInfo {
        cable_type: "active".into(),
        speed: Some(CableSpeed::Usb32Gen2),
        current_rating: Some(CableCurrent::FiveAmp),
        max_watts: 100,
        trust: CableTrust {
            trust_level: CableTrustLevel::KnownGood,
            ..Default::default()
        },
        ..Default::default()
    });
    dev_repaired.charging_diag = Some(ChargingDiagnostic {
        rule_id: "optimal",
        bottleneck: Bottleneck::Fine,
        is_warning: false,
        summary: "Optimal 100W charging".into(),
        root_cause: RootCauseCategory::None,
        status: DiagnosticStatus::Healthy,
        confidence: DiagnosticConfidence::High,
        detail: "".into(),
        recommended_action: "".into(),
    });
    after.summaries.push(dev_repaired);

    let diff = compare(&before, &after);
    assert_eq!(diff.modified.len(), 1);
    let m = &diff.modified[0];

    let cable_diff = m.cable.as_ref().expect("cable diff");
    assert_eq!(
        cable_diff.speed,
        Some(ValueChange::new(
            Some(CableSpeed::Usb20),
            Some(CableSpeed::Usb32Gen2)
        ))
    );
    assert_eq!(
        cable_diff.current_rating,
        Some(ValueChange::new(
            Some(CableCurrent::ThreeAmp),
            Some(CableCurrent::FiveAmp)
        ))
    );
    assert_eq!(
        cable_diff.max_watts,
        Some(ValueChange::new(Some(60), Some(100)))
    );
    assert_eq!(
        cable_diff.trust_level,
        Some(ValueChange::new(
            CableTrustLevel::Suspicious,
            CableTrustLevel::KnownGood
        ))
    );

    let diag_diff = m.diagnostic.as_ref().expect("diagnostic diff");
    assert_eq!(
        diag_diff.bottleneck,
        Some(ValueChange::new(Bottleneck::CableLimit, Bottleneck::Fine))
    );
    assert_eq!(diag_diff.is_warning, Some(ValueChange::new(true, false)));
}

// -----------------------------------------------------------------------------
// Test 3: History Ring Buffer & 9 State Transition Events
// -----------------------------------------------------------------------------

#[test]
fn test_history_ring_buffer_capacity_and_fifo_eviction() {
    let mut history = SnapshotHistory::with_capacity(3);
    assert_eq!(history.capacity(), 3);
    assert_eq!(history.len(), 0);
    assert!(history.is_empty());

    for i in 1..=5 {
        let snap = Snapshot {
            summaries: vec![make_sample_usb_summary(
                &format!("1-1.{i}"),
                480,
                &format!("Device {i}"),
            )],
            ..Default::default()
        };
        history.push(snap, 1000 * i);
    }

    assert_eq!(history.len(), 3);
    assert!(!history.is_empty());

    // Oldest entries 0 and 1 must have been evicted; sequence numbers 2, 3, 4 remain
    let entries: Vec<_> = history.entries().collect();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].sequence, 2);
    assert_eq!(entries[1].sequence, 3);
    assert_eq!(entries[2].sequence, 4);

    assert_eq!(history.latest().unwrap().sequence, 4);
    assert_eq!(history.oldest().unwrap().sequence, 2);
}

#[test]
fn test_history_all_nine_state_transitions() {
    let mut history = SnapshotHistory::with_capacity(10);

    // Initial snapshot: 1 device and 1 Type-C port
    let mut dev1 = make_sample_usb_summary("1-1.1", 480, "Storage Device");
    dev1.data_rate = Some(DataRateAssessment {
        verdict: DataRateVerdict::Degraded,
        negotiated_mbps: 480,
        capable_mbps: 5000,
        functional_floor_mbps: 5000,
        capable_gen: "Gen 1x1".into(),
        summary: "Downgraded".into(),
        detail: "".into(),
        is_warning: true,
    });

    let mut port0 = make_sample_port_summary("port0", 0, "Type-C Port 0");
    port0.power.power_role = PowerRole::Source;
    port0.power.contract_mw = 15000;
    port0.cable = Some(CableInfo {
        cable_type: "passive".into(),
        speed: Some(CableSpeed::Usb20),
        current_rating: Some(CableCurrent::ThreeAmp),
        max_watts: 60,
        trust: CableTrust {
            trust_level: CableTrustLevel::Suspicious,
            ..Default::default()
        },
        ..Default::default()
    });
    port0.charging_diag = Some(ChargingDiagnostic {
        rule_id: "cable_limits_charger",
        bottleneck: Bottleneck::CableLimit,
        is_warning: true,
        summary: "Cable bottleneck".into(),
        root_cause: RootCauseCategory::Cable,
        status: DiagnosticStatus::Warning,
        confidence: DiagnosticConfidence::High,
        detail: "".into(),
        recommended_action: "".into(),
    });

    let snap1 = Snapshot {
        summaries: vec![dev1, port0],
        ..Default::default()
    };
    let initial_transitions = history.push(snap1, 1000);
    // Baseline snapshot records 0 transitions by design
    assert!(initial_transitions.is_empty());

    // Snapshot 2:
    // - Device 1 unplugged -> DeviceDisconnected
    // - Device 2 plugged in -> DeviceConnected
    // - Port 0:
    //   - power_role: Source -> Sink (PortRoleChanged)
    //   - contract_mw: 15W -> 60W (PowerChanged)
    //   - active PDO / renegotiation -> ContractRenegotiated
    //   - cable trust / warning cleared -> CableWarningChanged
    //   - charging bottleneck cleared -> BottleneckChanged
    //   - speed changed -> LinkSpeedChanged
    let mut dev2 = make_sample_usb_summary("1-1.2", 5000, "SuperSpeed SSD");
    dev2.data_rate = Some(DataRateAssessment {
        verdict: DataRateVerdict::AtCapability,
        negotiated_mbps: 5000,
        capable_mbps: 5000,
        functional_floor_mbps: 5000,
        capable_gen: "Gen 1x1".into(),
        summary: "Optimal".into(),
        detail: "".into(),
        is_warning: false,
    });

    let mut port0_new = make_sample_port_summary("port0", 0, "Type-C Port 0");
    port0_new.link_speed_mbps = 10000;
    port0_new.power.power_role = PowerRole::Sink;
    port0_new.power.contract_mw = 60000;
    port0_new.cable = Some(CableInfo {
        cable_type: "passive".into(),
        speed: Some(CableSpeed::Usb32Gen2),
        current_rating: Some(CableCurrent::ThreeAmp),
        max_watts: 60,
        trust: CableTrust {
            trust_level: CableTrustLevel::KnownGood,
            ..Default::default()
        },
        ..Default::default()
    });
    port0_new.charging_diag = Some(ChargingDiagnostic {
        rule_id: "optimal",
        bottleneck: Bottleneck::Fine,
        is_warning: false,
        summary: "Optimal".into(),
        root_cause: RootCauseCategory::None,
        status: DiagnosticStatus::Healthy,
        confidence: DiagnosticConfidence::High,
        detail: "".into(),
        recommended_action: "".into(),
    });
    // Add PowerDelivery with active PDO changed
    port0_new.power_delivery = Some(PowerDeliveryPort {
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
                current_ma: 3000,
                power_mw: 60000,
                is_active: true,
                index: 2,
            },
        ],
        max_source_power_mw: 60000,
        ..Default::default()
    });

    let snap2 = Snapshot {
        summaries: vec![dev2, port0_new.clone()],
        ..Default::default()
    };
    let step2_transitions = history.push(snap2, 2000);

    // Check transitions
    let has_device_connected = step2_transitions
        .iter()
        .any(|t| matches!(t, StateTransition::DeviceConnected(_)));
    let has_device_disconnected = step2_transitions
        .iter()
        .any(|t| matches!(t, StateTransition::DeviceDisconnected(_)));
    let has_link_speed_changed = step2_transitions
        .iter()
        .any(|t| matches!(t, StateTransition::LinkSpeedChanged(_)));
    let has_power_changed = step2_transitions
        .iter()
        .any(|t| matches!(t, StateTransition::PowerChanged(_)));
    let has_contract_renegotiated = step2_transitions
        .iter()
        .any(|t| matches!(t, StateTransition::ContractRenegotiated(_)));
    let has_bottleneck_changed = step2_transitions
        .iter()
        .any(|t| matches!(t, StateTransition::BottleneckChanged(_)));
    let has_cable_warning_changed = step2_transitions
        .iter()
        .any(|t| matches!(t, StateTransition::CableWarningChanged(_)));
    let has_port_role_changed = step2_transitions
        .iter()
        .any(|t| matches!(t, StateTransition::PortRoleChanged(_)));

    assert!(has_device_connected, "Expected DeviceConnected event");
    assert!(has_device_disconnected, "Expected DeviceDisconnected event");
    assert!(has_link_speed_changed, "Expected LinkSpeedChanged event");
    assert!(has_power_changed, "Expected PowerChanged event");
    assert!(
        has_contract_renegotiated,
        "Expected ContractRenegotiated event"
    );
    assert!(has_bottleneck_changed, "Expected BottleneckChanged event");
    assert!(
        has_cable_warning_changed,
        "Expected CableWarningChanged event"
    );
    assert!(has_port_role_changed, "Expected PortRoleChanged event");

    // Snapshot 3: specifically trigger DataRateChanged on dev2
    let mut dev2_down = make_sample_usb_summary("1-1.2", 480, "SuperSpeed SSD");
    dev2_down.data_rate = Some(DataRateAssessment {
        verdict: DataRateVerdict::Degraded,
        negotiated_mbps: 480,
        capable_mbps: 5000,
        functional_floor_mbps: 5000,
        capable_gen: "Gen 1x1".into(),
        summary: "Downgraded to 480 Mbps".into(),
        detail: "".into(),
        is_warning: true,
    });
    let snap3 = Snapshot {
        summaries: vec![dev2_down, port0_new],
        ..Default::default()
    };
    let step3_transitions = history.push(snap3, 3000);
    let has_data_rate_changed = step3_transitions
        .iter()
        .any(|t| matches!(t, StateTransition::DataRateChanged(_)));
    assert!(has_data_rate_changed, "Expected DataRateChanged event");

    // Check query APIs
    let dev2_transitions: Vec<_> = history
        .filter_transitions(|t| t.id() == "usb:1-1.2")
        .into_iter()
        .collect();
    assert!(!dev2_transitions.is_empty());

    let timeline = history.timeline();
    assert!(!timeline.is_empty());
    for item in &timeline {
        let rendered = format!("{}", item.transition);
        assert!(
            rendered.starts_with("[+]")
                || rendered.starts_with("[-]")
                || rendered.starts_with("[~]")
                || rendered.starts_with("[!]")
                || rendered.starts_with("[*]")
        );
    }

    let diff_latest = history.diff_latest().expect("latest diff exists");
    assert!(!diff_latest.modified.is_empty());

    let diff_range = history.diff_range(0, 2).expect("range diff exists");
    assert!(!diff_range.is_empty());
}
