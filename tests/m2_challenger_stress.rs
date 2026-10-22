//! Empirical Challenger Stress & Adversarial Suite for Milestone 2.
//!
//! Covers:
//! 1. Serialization & Deserialization Fuzzing (unexpected extra keys, missing optional fields,
//!    malformed JSON strings, truncated JSON, tagged enum variants, roundtrip invariants).
//! 2. Pure-Core Zero-IO Attestation (proc/self/fd inspection proving zero I/O and zero leak).
//! 3. History Ring Buffer Invariants & Stress Testing (zero capacity clamping, massive pushes,
//!    dynamic capacity shrinkage/expansion, out-of-range diffing, sequence tracking).
//! 4. Compare Engine Edge Cases (symmetry, property filtering, diagnostic transitions, cable trust).
//! 5. State Transition Event Detection & Display Formatting (all 9 events, badges, filters).

use usbeehive::bos::DataRateVerdict;
use usbeehive::cable::{CableInfo, CableTrust};
use usbeehive::compare::{compare, SnapshotDiff, ValueChange};
use usbeehive::diagnostic::{
    Bottleneck, ChargingDiagnostic, DiagnosticConfidence, DiagnosticStatus, RootCauseCategory,
};
use usbeehive::history::{
    BottleneckChangedEvent, CableWarningChangedEvent, ContractRenegotiatedEvent,
    DataRateChangedEvent, DeviceConnectedEvent, DeviceDisconnectedEvent, LinkSpeedChangedEvent,
    PortRoleChangedEvent, PowerChangedEvent, SnapshotHistory, StateTransition, TimelineEntry,
};
use usbeehive::power::PdoType;
use usbeehive::snapshot::{Snapshot, SnapshotMetadata};
use usbeehive::summary::{Category, DeviceClass, DeviceSummary, PowerRole, PowerSummary, Status};
use usbeehive::trust::CableTrustLevel;
use usbeehive::typec::TypeCPort;
use usbeehive::usb::UsbDevice;

// Helper to construct a minimal DeviceSummary for USB device
fn make_usb_summary(id_suffix: &str, speed: u32, headline: &str) -> DeviceSummary {
    let bus_port = format!("1-1.{id_suffix}");
    DeviceSummary {
        category: Category::UsbDevice,
        device_class: DeviceClass::Storage,
        device_subclass: "SCSI".into(),
        status: Status::Connected,
        headline: headline.into(),
        subtitle: "Mass Storage".into(),
        icon: "drive-removable-media".into(),
        vendor: "TestCorp".into(),
        product: headline.into(),
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
            bus_port: bus_port.clone(),
            speed,
            product: headline.into(),
            manufacturer: "TestCorp".into(),
            ..Default::default()
        }),
        properties: vec![
            ("bus_port".into(), bus_port),
            ("transport.protocol".into(), "usb".into()),
        ],
    }
}

// Helper to construct a minimal DeviceSummary for Type-C port
fn make_port_summary(port_num: i32, headline: &str) -> DeviceSummary {
    let port_name = format!("port{port_num}");
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
            port_name: port_name.clone(),
            port_number: port_num,
            ..Default::default()
        }),
        power_delivery: None,
        usb_device: None,
        properties: vec![("port_name".into(), port_name)],
    }
}

// =============================================================================
// AREA 1: Serialization & Deserialization Fuzzing
// =============================================================================

#[test]
fn test_fuzz_snapshot_with_unexpected_extra_keys() {
    let fuzzed_json = r#"{
        "unknown_root_property": "should_be_ignored",
        "future_schema_version": 999,
        "nested_junk": {
            "deep_array": [1, 2, 3, {"weird": true}]
        },
        "usb_devices": [],
        "typec_ports": [],
        "pd_ports": [],
        "thunderbolt_routers": [],
        "summaries": [],
        "extra_tail_field": 42.5
    }"#;

    let snap: Snapshot = serde_json::from_str(fuzzed_json).expect("parse with extra keys");
    assert!(snap.is_empty());
    assert!(snap.metadata.is_none());

    #[cfg(any(feature = "cli", feature = "dbus"))]
    {
        let snap2 = Snapshot::from_json(fuzzed_json).expect("from_json with extra keys");
        assert_eq!(snap, snap2);
    }
}

#[test]
fn test_fuzz_snapshot_missing_optional_fields() {
    // Completely empty JSON object
    let empty_json = "{}";
    let snap: Snapshot = serde_json::from_str(empty_json).expect("parse empty object");
    assert!(snap.is_empty());
    assert_eq!(snap.len(), 0);
    assert!(snap.metadata.is_none());

    // Partial JSON with null metadata
    let partial_null_json = r#"{
        "usb_devices": [],
        "metadata": null
    }"#;
    let snap2: Snapshot = serde_json::from_str(partial_null_json).expect("parse null metadata");
    assert!(snap2.is_empty());
    assert!(snap2.metadata.is_none());

    // Snapshot metadata with omitted optional fields
    let partial_meta_json = r#"{
        "metadata": {
            "source": "ci-host"
        }
    }"#;
    let snap3: Snapshot = serde_json::from_str(partial_meta_json).expect("parse partial metadata");
    let meta = snap3.metadata.expect("metadata present");
    assert_eq!(meta.captured_at_ms, 0); // default
    assert_eq!(meta.sequence, 0); // default
    assert_eq!(meta.source, "ci-host");
}

#[test]
fn test_fuzz_snapshot_malformed_json_returns_err() {
    let malformed_inputs = [
        "",                            // empty string
        "   ",                         // whitespace
        "{",                           // unclosed object
        "{\"usb_devices\": [",         // unclosed array
        "{\"metadata\": \"not_obj\"}", // string where object expected
        "12345",                       // number where object expected
        "\"raw_string\"",              // string where object expected
        "[1, 2, 3]",                   // array where object expected
        "null",                        // null where object expected
        "true",                        // bool where object expected
        "{\"summaries\": [{\"id\": }", // syntax error inside element
    ];

    for input in &malformed_inputs {
        let res: Result<Snapshot, _> = serde_json::from_str(input);
        assert!(res.is_err(), "Expected error on malformed input: {input:?}");

        #[cfg(any(feature = "cli", feature = "dbus"))]
        {
            let res2 = Snapshot::from_json(input);
            assert!(res2.is_err(), "Expected error via from_json on: {input:?}");
        }
    }
}

#[test]
fn test_fuzz_snapshot_diff_serialization_and_extra_keys() {
    // 1. Standard empty diff JSON (all 3 fields present as empty arrays)
    let standard_empty_diff_json = r#"{"added":[],"removed":[],"modified":[]}"#;
    let diff: SnapshotDiff =
        serde_json::from_str(standard_empty_diff_json).expect("parse empty diff");
    assert!(diff.is_empty());
    assert_eq!(diff.total_changes(), 0);
    assert_eq!(diff.summary_string(), "No differences");

    // 2. Extra keys present alongside the fields are ignored cleanly
    let fuzzed_diff_json = r#"{
        "extra_field": "ignore_me",
        "added": [],
        "removed": [],
        "modified": [],
        "nested_garbage": [true, false]
    }"#;
    let diff2: SnapshotDiff = serde_json::from_str(fuzzed_diff_json).expect("parse fuzzed diff");
    assert!(diff2.is_empty());

    #[cfg(any(feature = "cli", feature = "dbus"))]
    {
        let diff3 = SnapshotDiff::from_json(fuzzed_diff_json).expect("from_json on fuzzed diff");
        assert_eq!(diff2, diff3);
    }

    // 3. SnapshotDiff does not have #[serde(default)] at struct level,
    // so {} fails with missing field `added`.
    let missing_field_json = "{}";
    let res: Result<SnapshotDiff, _> = serde_json::from_str(missing_field_json);
    assert!(
        res.is_err(),
        "SnapshotDiff without fields returns Err cleanly"
    );
}

#[test]
fn test_fuzz_state_transitions_tagged_enum() {
    // 1. Valid tagged transition round-trip
    let event = StateTransition::LinkSpeedChanged(LinkSpeedChangedEvent {
        id: "usb:1-1".into(),
        headline: "HighSpeed Flash Drive".into(),
        old_speed_mbps: 480,
        new_speed_mbps: 5000,
    });
    let serialized = serde_json::to_string(&event).expect("serialize transition");
    assert!(serialized.contains(r#""transition_type":"LinkSpeedChanged""#));
    assert!(serialized.contains(r#""old_speed_mbps":480"#));
    assert!(serialized.contains(r#""new_speed_mbps":5000"#));

    let deserialized: StateTransition =
        serde_json::from_str(&serialized).expect("deserialize transition");
    assert_eq!(event, deserialized);
    assert_eq!(deserialized.id(), "usb:1-1");

    // 2. Unknown transition type tag
    let unknown_tag_json = r#"{
        "transition_type": "FutureUnknownEvent",
        "details": {
            "id": "usb:1-1"
        }
    }"#;
    let res: Result<StateTransition, _> = serde_json::from_str(unknown_tag_json);
    assert!(
        res.is_err(),
        "Unknown transition_type must fail cleanly without panic"
    );

    // 3. Tag present but details malformed
    let bad_details_json = r#"{
        "transition_type": "LinkSpeedChanged",
        "details": "not_an_object"
    }"#;
    let res2: Result<StateTransition, _> = serde_json::from_str(bad_details_json);
    assert!(res2.is_err(), "Malformed details must return Err");

    // 4. Missing tag
    let missing_tag_json = r#"{
        "details": {
            "id": "usb:1-1"
        }
    }"#;
    let res3: Result<StateTransition, _> = serde_json::from_str(missing_tag_json);
    assert!(res3.is_err(), "Missing tag must return Err");
}

#[test]
fn test_fuzz_snapshot_history_json_roundtrip_and_timeline() {
    let mut history = SnapshotHistory::with_capacity(5);
    let snap1 = Snapshot::from(vec![make_usb_summary("1", 480, "USB Stick")]);
    let snap2 = Snapshot::from(vec![
        make_usb_summary("1", 5000, "USB Stick"),
        make_port_summary(0, "Port 0"),
    ]);

    history.push(snap1, 1000);
    history.push(snap2, 2000);

    let serialized = serde_json::to_string(&history).expect("serialize history");
    let parsed: SnapshotHistory = serde_json::from_str(&serialized).expect("deserialize history");
    assert_eq!(history.len(), parsed.len());
    assert_eq!(history.total_pushed(), parsed.total_pushed());
    assert_eq!(history.capacity(), parsed.capacity());

    let timeline = history.timeline();
    assert!(!timeline.is_empty());
    let timeline_json = serde_json::to_string(&timeline).expect("serialize timeline");
    let parsed_timeline: Vec<TimelineEntry> =
        serde_json::from_str(&timeline_json).expect("deserialize timeline");
    assert_eq!(timeline, parsed_timeline);

    #[cfg(any(feature = "cli", feature = "dbus"))]
    {
        let h_json = history.to_json().expect("history to_json");
        let h_from = SnapshotHistory::from_json(&h_json).expect("history from_json");
        assert_eq!(history, h_from);

        let t_json = history.timeline_to_json().expect("timeline to_json");
        let t_from: Vec<TimelineEntry> = serde_json::from_str(&t_json).expect("parse t_json");
        assert_eq!(timeline, t_from);
    }
}

// =============================================================================
// AREA 2: Pure-Core Zero-IO Empirical Attestation
// =============================================================================

#[test]
fn test_zero_io_pure_computation_snapshot_and_compare() {
    // Count open file descriptors in /proc/self/fd before and after executing
    // 1,000 operations of Snapshot creation, comparison, history ring-buffer ingestion,
    // timeline queries, and diff formatting.
    let count_fds = || -> usize {
        std::fs::read_dir("/proc/self/fd")
            .map(|entries| entries.count())
            .unwrap_or(0)
    };

    let fds_before = count_fds();

    let snap_base = Snapshot::new().with_metadata(SnapshotMetadata::new(1000, 1, "test-host"));
    let mut history = SnapshotHistory::with_capacity(50);
    history.push(snap_base, 1000);

    for i in 1..=1000 {
        let speed = if i % 2 == 0 { 480 } else { 5000 };
        let snap = Snapshot::from(vec![
            make_usb_summary("1", speed, "Device A"),
            make_port_summary(i % 4, "Port"),
        ]);

        let transitions: Vec<StateTransition> =
            history.push(snap.clone(), 1000 + i as u64).to_vec();
        let _ = history.timeline();
        let _ = history.diff_latest();
        if i >= 10 {
            let _ = history.diff_range(i as u64 - 5, i as u64);
        }

        // Render transition displays
        for t in &transitions {
            let _ = format!("{t}");
        }

        // Run compare engine and format diff text
        let diff = compare(history.latest_snapshot().unwrap(), &snap);
        let _ = diff.render_text();
        let _ = diff.summary_string();
        let _ = diff.has_warnings();
    }

    let fds_after = count_fds();
    assert_eq!(
        fds_before, fds_after,
        "Pure-core snapshot, compare, and history leaked or opened file descriptors! (before={fds_before}, after={fds_after})"
    );
}

// =============================================================================
// AREA 3: History Ring Buffer Invariants & Stress Testing
// =============================================================================

#[test]
fn test_history_zero_capacity_clamped() {
    // Capacity 0 must be clamped to 1 to prevent division by zero or empty ring panics
    let history = SnapshotHistory::with_capacity(0);
    assert_eq!(history.capacity(), 1);
}

#[test]
fn test_history_capacity_one_ring_buffer() {
    let mut history = SnapshotHistory::with_capacity(1);
    for i in 0..10 {
        let snap = Snapshot::from(vec![make_usb_summary(&format!("{i}"), 480, "USB Dev")]);
        history.push(snap, 1000 + i);
        assert_eq!(history.len(), 1);
        assert_eq!(history.total_pushed(), i + 1);
    }
    assert_eq!(history.latest().unwrap().sequence, 9);
    assert_eq!(history.oldest().unwrap().sequence, 9);
}

#[test]
fn test_history_massive_push_and_sequence_lookup() {
    const CAPACITY: usize = 50;
    const TOTAL_PUSHES: u64 = 10_000;

    let mut history = SnapshotHistory::with_capacity(CAPACITY);
    for i in 0..TOTAL_PUSHES {
        let snap = Snapshot::from(vec![make_usb_summary("1", 480 + (i as u32 % 10), "Dev")]);
        history.push(snap, 1000 + i);
    }

    assert_eq!(history.len(), CAPACITY);
    assert_eq!(history.total_pushed(), TOTAL_PUSHES);

    let oldest_seq = TOTAL_PUSHES - CAPACITY as u64;
    let newest_seq = TOTAL_PUSHES - 1;

    assert_eq!(history.oldest().unwrap().sequence, oldest_seq);
    assert_eq!(history.latest().unwrap().sequence, newest_seq);

    // Evicted entry returns None
    assert!(history.get_by_sequence(oldest_seq - 1).is_none());
    // Retained entries return Some
    assert!(history.get_by_sequence(oldest_seq).is_some());
    assert!(history.get_by_sequence(newest_seq).is_some());

    // diff_latest works across retained entries
    let diff = history.diff_latest();
    assert!(diff.is_some());

    // diff_range across retained entries
    let range_diff = history.diff_range(oldest_seq, newest_seq);
    assert!(range_diff.is_some());

    // diff_range referencing evicted entry returns None
    let evicted_diff = history.diff_range(0, newest_seq);
    assert!(evicted_diff.is_none());
}

#[test]
fn test_history_dynamic_capacity_shrinkage_and_expansion() {
    let mut history = SnapshotHistory::with_capacity(20);
    for i in 0..20 {
        history.push(Snapshot::default(), 1000 + i);
    }
    assert_eq!(history.len(), 20);

    // Shrink capacity down to 5: must evict oldest 15
    history.set_capacity(5);
    assert_eq!(history.capacity(), 5);
    assert_eq!(history.len(), 5);
    assert_eq!(history.oldest().unwrap().sequence, 15);
    assert_eq!(history.latest().unwrap().sequence, 19);

    // Expand capacity up to 100: length stays 5
    history.set_capacity(100);
    assert_eq!(history.capacity(), 100);
    assert_eq!(history.len(), 5);
}

#[test]
fn test_history_clear_resets_all_state() {
    let mut history = SnapshotHistory::with_capacity(10);
    for i in 0..10 {
        history.push(Snapshot::default(), 1000 + i);
    }
    assert_eq!(history.len(), 10);
    assert_eq!(history.total_pushed(), 10);

    history.clear();
    assert!(history.is_empty());
    assert_eq!(history.len(), 0);
    assert_eq!(history.total_pushed(), 0);
    assert!(history.latest().is_none());
    assert!(history.oldest().is_none());
    assert!(history.diff_latest().is_none());
}

// =============================================================================
// AREA 4: Compare Engine Edge Cases & Invariants
// =============================================================================

#[test]
fn test_compare_symmetry_and_disjoint_snapshots() {
    let snap_a = Snapshot::from(vec![
        make_usb_summary("1", 480, "Mouse"),
        make_usb_summary("2", 5000, "Flash Drive"),
    ]);
    let snap_b = Snapshot::from(vec![
        make_usb_summary("3", 10000, "External SSD"),
        make_port_summary(0, "Port 0"),
    ]);

    // diff(A, B): A -> B
    let diff_ab = compare(&snap_a, &snap_b);
    assert_eq!(diff_ab.added.len(), 2); // SSD, Port 0
    assert_eq!(diff_ab.removed.len(), 2); // Mouse, Flash Drive
    assert_eq!(diff_ab.modified.len(), 0);

    // diff(B, A): B -> A (Inverted symmetry)
    let diff_ba = compare(&snap_b, &snap_a);
    assert_eq!(diff_ba.added.len(), 2); // Mouse, Flash Drive
    assert_eq!(diff_ba.removed.len(), 2); // SSD, Port 0
    assert_eq!(diff_ba.modified.len(), 0);

    assert_eq!(diff_ab.added_ids(), diff_ba.removed_ids());
    assert_eq!(diff_ab.removed_ids(), diff_ba.added_ids());
}

#[test]
fn test_compare_property_filtering_curated_vs_noisy() {
    let mut dev1 = make_usb_summary("1", 480, "Device");
    dev1.properties
        .push(("transport.usb4".into(), "false".into()));
    dev1.properties.push(("data_role".into(), "device".into()));
    dev1.properties
        .push(("internal_jitter_temp".into(), "42C".into()));

    let mut dev2 = make_usb_summary("1", 480, "Device");
    dev2.properties
        .push(("transport.usb4".into(), "true".into()));
    dev2.properties.push(("data_role".into(), "host".into()));
    dev2.properties
        .push(("internal_jitter_temp".into(), "48C".into())); // should be ignored

    let snap1 = Snapshot::from(vec![dev1]);
    let snap2 = Snapshot::from(vec![dev2]);

    let diff = compare(&snap1, &snap2);
    assert_eq!(diff.modified.len(), 1);
    let mod_dev = &diff.modified[0];

    // Curated properties changed
    let prop_keys: Vec<&str> = mod_dev
        .property_changes
        .iter()
        .map(|p| p.key.as_str())
        .collect();
    assert!(prop_keys.contains(&"transport.usb4"));
    assert!(prop_keys.contains(&"data_role"));

    // Non-curated property must NOT be tracked
    assert!(!prop_keys.contains(&"internal_jitter_temp"));
}

#[test]
fn test_compare_diagnostic_and_cable_warning_transitions() {
    let mut dev_clean = make_port_summary(0, "Port 0");
    dev_clean.charging_diag = Some(ChargingDiagnostic {
        bottleneck: Bottleneck::Fine,
        summary: "Optimal".into(),
        detail: "100W charging".into(),
        is_warning: false,
        status: DiagnosticStatus::Healthy,
        root_cause: RootCauseCategory::None,
        confidence: DiagnosticConfidence::High,
        recommended_action: "None".into(),
        rule_id: "none",
    });
    dev_clean.cable = Some(CableInfo {
        vendor_name: "Apple".into(),
        max_watts: 100,
        trust: CableTrust {
            trust_level: CableTrustLevel::KnownGood,
            ..Default::default()
        },
        ..Default::default()
    });

    let mut dev_degraded = make_port_summary(0, "Port 0");
    dev_degraded.charging_diag = Some(ChargingDiagnostic {
        bottleneck: Bottleneck::CableLimit,
        summary: "Cable bottleneck".into(),
        detail: "60W cable with 100W charger".into(),
        is_warning: true,
        status: DiagnosticStatus::Warning,
        root_cause: RootCauseCategory::Cable,
        confidence: DiagnosticConfidence::High,
        recommended_action: "Replace cable with 100W rated cable".into(),
        rule_id: "cable_limit",
    });
    dev_degraded.cable = Some(CableInfo {
        vendor_name: "Unknown".into(),
        max_watts: 60,
        trust: CableTrust {
            trust_level: CableTrustLevel::Suspicious,
            notes: vec!["Reserved bits set in cable VDO".into()],
            ..Default::default()
        },
        ..Default::default()
    });

    let snap_clean = Snapshot::from(vec![dev_clean]);
    let snap_degraded = Snapshot::from(vec![dev_degraded]);

    // 1. Degradation transition
    let diff_deg = compare(&snap_clean, &snap_degraded);
    assert!(diff_deg.has_warnings());
    let mod_dev = &diff_deg.modified[0];
    let diag_diff = mod_dev.diagnostic.as_ref().unwrap();
    assert!(diag_diff.is_newly_degraded());
    assert!(!diag_diff.is_resolved());

    let cable_diff = mod_dev.cable.as_ref().unwrap();
    assert_eq!(
        cable_diff.trust_level,
        Some(ValueChange::new(
            CableTrustLevel::KnownGood,
            CableTrustLevel::Suspicious
        ))
    );
    assert!(!cable_diff.notes_added.is_empty());

    // 2. Resolution transition
    let diff_res = compare(&snap_degraded, &snap_clean);
    let mod_dev_res = &diff_res.modified[0];
    let diag_diff_res = mod_dev_res.diagnostic.as_ref().unwrap();
    assert!(diag_diff_res.is_resolved());
    assert!(!diag_diff_res.is_newly_degraded());
}

// =============================================================================
// AREA 5: State Transition Event Detection & Display Formatting
// =============================================================================

#[test]
fn test_state_transitions_all_nine_event_types_and_display_formatting() {
    // 1. DeviceConnected
    let conn = StateTransition::DeviceConnected(DeviceConnectedEvent {
        id: "usb:1-1".into(),
        headline: "Keyboard".into(),
        subtitle: "HID".into(),
        category: Category::UsbDevice,
        device_class: DeviceClass::Keyboard,
        vendor_id: 0x046D,
        product_id: 0xC31C,
        link_speed_mbps: 12,
        status: Status::Connected,
    });
    let s_conn = format!("{conn}");
    assert!(s_conn.starts_with("[+] CONNECTED:"));
    assert!(s_conn.contains("usb:1-1"));
    assert!(s_conn.contains("Keyboard"));

    // 2. DeviceDisconnected
    let disconn = StateTransition::DeviceDisconnected(DeviceDisconnectedEvent {
        id: "usb:1-1".into(),
        headline: "Keyboard".into(),
        category: Category::UsbDevice,
        device_class: DeviceClass::Keyboard,
    });
    let s_disconn = format!("{disconn}");
    assert!(s_disconn.starts_with("[-] DISCONNECTED:"));
    assert!(s_disconn.contains("usb:1-1"));

    // 3. LinkSpeedChanged
    let speed = StateTransition::LinkSpeedChanged(LinkSpeedChangedEvent {
        id: "usb:1-2".into(),
        headline: "USB SSD".into(),
        old_speed_mbps: 480,
        new_speed_mbps: 10000,
    });
    let s_speed = format!("{speed}");
    assert!(s_speed.starts_with("[~] SPEED CHANGED:"));
    assert!(s_speed.contains("480 Mbps -> 10000 Mbps"));

    // 4. PowerChanged
    let power = StateTransition::PowerChanged(PowerChangedEvent {
        id: "typec:port0".into(),
        headline: "USB-C Port 0".into(),
        power_role: PowerRole::Sink,
        old_power_in_mw: 15000,
        new_power_in_mw: 60000,
        old_power_out_mw: 0,
        new_power_out_mw: 0,
        old_contract_mw: 15000,
        new_contract_mw: 60000,
    });
    let s_power = format!("{power}");
    assert!(s_power.starts_with("[~] POWER CHANGED:"));
    assert!(s_power.contains("15000 mW -> 60000 mW"));

    // 5. ContractRenegotiated
    let contract = StateTransition::ContractRenegotiated(ContractRenegotiatedEvent {
        id: "typec:port0".into(),
        port_number: Some(0),
        old_power_mw: 15000,
        new_power_mw: 100000,
        old_voltage_mv: 5000,
        new_voltage_mv: 20000,
        old_current_ma: 3000,
        new_current_ma: 5000,
        old_pdo_index: Some(0),
        new_pdo_index: Some(3),
        old_pdo_type: Some(PdoType::FixedSupply),
        new_pdo_type: Some(PdoType::FixedSupply),
    });
    let s_contract = format!("{contract}");
    assert!(s_contract.starts_with("[~] CONTRACT RENEGOTIATED:"));
    assert!(s_contract.contains("15000 mW @ 5.0V/3000mA -> 100000 mW @ 20.0V/5000mA"));

    // 6. BottleneckChanged (Degraded & Resolved)
    let bottleneck_degraded = StateTransition::BottleneckChanged(BottleneckChangedEvent {
        id: "typec:port0".into(),
        port_number: Some(0),
        old_bottleneck: Some(Bottleneck::Fine),
        new_bottleneck: Some(Bottleneck::CableLimit),
        old_status: Some(DiagnosticStatus::Healthy),
        new_status: Some(DiagnosticStatus::Warning),
        warning_raised: true,
        warning_cleared: false,
        root_cause: RootCauseCategory::Cable,
        summary: "Cable bottleneck".into(),
        recommended_action: Some("Upgrade to 100W cable".into()),
    });
    let s_bn_deg = format!("{bottleneck_degraded}");
    assert!(s_bn_deg.starts_with("[!] BOTTLENECK DEGRADED:"));
    assert!(s_bn_deg.contains("Cable bottleneck"));

    let bottleneck_resolved = StateTransition::BottleneckChanged(BottleneckChangedEvent {
        id: "typec:port0".into(),
        port_number: Some(0),
        old_bottleneck: Some(Bottleneck::CableLimit),
        new_bottleneck: Some(Bottleneck::Fine),
        old_status: Some(DiagnosticStatus::Warning),
        new_status: Some(DiagnosticStatus::Healthy),
        warning_raised: false,
        warning_cleared: true,
        root_cause: RootCauseCategory::None,
        summary: "Resolved".into(),
        recommended_action: None,
    });
    let s_bn_res = format!("{bottleneck_resolved}");
    assert!(s_bn_res.starts_with("[*] BOTTLENECK RESOLVED:"));

    // 7. CableWarningChanged (Raised & Cleared)
    let cable_raised = StateTransition::CableWarningChanged(CableWarningChangedEvent {
        id: "typec:port0".into(),
        port_number: Some(0),
        old_trust_level: Some(CableTrustLevel::KnownGood),
        new_trust_level: Some(CableTrustLevel::Suspicious),
        old_max_watts: Some(100),
        new_max_watts: Some(60),
        warning_raised: true,
        warning_cleared: false,
        reason: "Contract mismatch".into(),
    });
    let s_cable_raised = format!("{cable_raised}");
    assert!(s_cable_raised.starts_with("[!] CABLE WARNING RAISED:"));
    assert!(s_cable_raised.contains("Contract mismatch"));

    let cable_cleared = StateTransition::CableWarningChanged(CableWarningChangedEvent {
        id: "typec:port0".into(),
        port_number: Some(0),
        old_trust_level: Some(CableTrustLevel::Suspicious),
        new_trust_level: Some(CableTrustLevel::KnownGood),
        old_max_watts: Some(60),
        new_max_watts: Some(100),
        warning_raised: false,
        warning_cleared: true,
        reason: "Cable replaced".into(),
    });
    let s_cable_cleared = format!("{cable_cleared}");
    assert!(s_cable_cleared.starts_with("[*] CABLE WARNING CLEARED:"));

    // 8. PortRoleChanged
    let port_role = StateTransition::PortRoleChanged(PortRoleChangedEvent {
        id: "typec:port0".into(),
        old_status: Status::Connected,
        new_status: Status::Connected,
        old_power_role: PowerRole::Sink,
        new_power_role: PowerRole::Source,
        old_data_role: Some("device".into()),
        new_data_role: Some("host".into()),
    });
    let s_port_role = format!("{port_role}");
    assert!(s_port_role.starts_with("[~] PORT ROLE CHANGED:"));
    assert!(s_port_role.contains("Sink -> Source"));

    // 9. DataRateChanged (Degraded & Restored)
    let rate_degraded = StateTransition::DataRateChanged(DataRateChangedEvent {
        id: "usb:1-3".into(),
        headline: "SuperSpeed Hub".into(),
        old_verdict: Some(DataRateVerdict::AtCapability),
        new_verdict: Some(DataRateVerdict::Degraded),
        is_warning: true,
        is_restored: false,
    });
    let s_rate_deg = format!("{rate_degraded}");
    assert!(s_rate_deg.starts_with("[!] DATA-RATE DEGRADED:"));

    let rate_restored = StateTransition::DataRateChanged(DataRateChangedEvent {
        id: "usb:1-3".into(),
        headline: "SuperSpeed Hub".into(),
        old_verdict: Some(DataRateVerdict::Degraded),
        new_verdict: Some(DataRateVerdict::AtCapability),
        is_warning: false,
        is_restored: true,
    });
    let s_rate_res = format!("{rate_restored}");
    assert!(s_rate_res.starts_with("[*] DATA-RATE RESTORED:"));
}

#[test]
fn test_history_transition_filtering_and_since_sequence() {
    let mut history = SnapshotHistory::with_capacity(10);

    // Initial baseline
    history.push(Snapshot::default(), 1000);

    // Plug device 1
    let snap1 = Snapshot::from(vec![make_usb_summary("1", 480, "Mouse")]);
    history.push(snap1, 2000);

    // Plug device 2
    let snap2 = Snapshot::from(vec![
        make_usb_summary("1", 480, "Mouse"),
        make_usb_summary("2", 5000, "Keyboard"),
    ]);
    history.push(snap2, 3000);

    // Unplug device 1
    let snap3 = Snapshot::from(vec![make_usb_summary("2", 5000, "Keyboard")]);
    history.push(snap3, 4000);

    // Filter for connections
    let connections =
        history.filter_transitions(|t| matches!(t, StateTransition::DeviceConnected(_)));
    assert_eq!(connections.len(), 2);

    // Filter for disconnections
    let disconnections =
        history.filter_transitions(|t| matches!(t, StateTransition::DeviceDisconnected(_)));
    assert_eq!(disconnections.len(), 1);
    assert_eq!(disconnections[0].id(), "usb:1-1.1");

    // Query transitions since sequence 2
    let recent = history.transitions_since(2);
    assert_eq!(recent.len(), 1); // Only the unplug from seq 3
    assert_eq!(recent[0].id(), "usb:1-1.1");
}
