//! Regression coverage for imported histories and semantic comparison.
use usbeehive::compare::compare;
use usbeehive::history::{SnapshotHistory, StateTransition};
use usbeehive::summary::{Category, DeviceClass, DeviceSummary, PowerSummary, Status};
use usbeehive::{usb::UsbDevice, Snapshot};

fn snapshot(contract: Option<&str>) -> Snapshot {
    Snapshot {
        summaries: vec![DeviceSummary {
            category: Category::UsbDevice,
            device_class: DeviceClass::Storage,
            device_subclass: String::new(),
            status: Status::Connected,
            headline: "Device".into(),
            subtitle: String::new(),
            icon: String::new(),
            vendor: String::new(),
            product: String::new(),
            vendor_id: 0,
            product_id: 0,
            primary_driver: String::new(),
            properties: contract
                .into_iter()
                .map(|s| ("pd_contract".into(), s.into()))
                .collect(),
            link_speed_mbps: 0,
            usb_version: String::new(),
            power: PowerSummary::default(),
            charging_diag: None,
            data_rate: None,
            usb_device: Some(UsbDevice {
                bus_port: "1-1".into(),
                ..Default::default()
            }),
            typec_port: None,
            power_delivery: None,
            cable: None,
        }],
        ..Default::default()
    }
}

#[test]
fn text_renders_each_representable_transition_including_removals() {
    use usbeehive::bos::DataRateVerdict;
    use usbeehive::compare::*;
    use usbeehive::diagnostic::RootCauseCategory;
    use usbeehive::pd::{CableCurrent, CableSpeed};
    let mut modified = compare(&snapshot(None), &snapshot(Some("5V")))
        .modified
        .remove(0);
    modified.pd_contract = Some(PdContractDifference {
        active_pdo_index: Some(ValueChange::new(Some(0), Some(1))),
        ..Default::default()
    });
    modified.diagnostic = Some(DiagnosticDifference {
        root_cause: Some(ValueChange::new(
            RootCauseCategory::Cable,
            RootCauseCategory::Charger,
        )),
        summary: Some(ValueChange::new(
            "Old explanation".into(),
            "New explanation".into(),
        )),
        recommended_action: Some(ValueChange::new(Some("Replace cable".into()), None)),
        ..Default::default()
    });
    modified.cable = Some(CableDifference {
        speed: Some(ValueChange::new(
            Some(CableSpeed::Usb20),
            Some(CableSpeed::Usb4Gen3),
        )),
        current_rating: Some(ValueChange::new(
            Some(CableCurrent::ThreeAmp),
            Some(CableCurrent::FiveAmp),
        )),
        is_suspicious: Some(ValueChange::new(false, true)),
        ..Default::default()
    });
    modified.data_rate = Some(DataRateDifference {
        verdict: Some(ValueChange::new(
            DataRateVerdict::Unknown,
            DataRateVerdict::Degraded,
        )),
        is_warning: Some(ValueChange::new(false, true)),
    });
    let text = SnapshotDiff {
        modified: vec![modified],
        ..Default::default()
    }
    .render_text();
    for expected in [
        "Active PDO Index: Some(0) -> Some(1)",
        "Root Cause: Cable -> Charger",
        "Old explanation -> New explanation",
        "Replace cable -> [absent]",
        "Usb20",
        "Usb4Gen3",
        "ThreeAmp",
        "FiveAmp",
        "Cable Suspicious: false -> true",
        "Data Rate Verdict: Unknown -> Degraded",
        "Data Rate Warning: false -> true",
    ] {
        assert!(text.contains(expected), "missing {expected} in {text}");
    }
}

#[test]
fn comparison_uses_canonical_summaries_not_raw_telemetry() {
    let before = Snapshot::default();
    let mut after = Snapshot::default();
    after.usb_devices.push(UsbDevice {
        bus_port: "1-1".into(),
        ..Default::default()
    });
    assert!(compare(&before, &after).is_empty());
    let mut history = SnapshotHistory::new();
    history.push(before, 0);
    assert!(history.push(after, 1).is_empty());
}

#[test]
fn added_power_is_not_presented_as_measured_draw() {
    let mut after = snapshot(None);
    after.summaries[0].power.power_in_mw = 4500;
    let text = compare(&Snapshot::default(), &after).render_text();
    assert!(text.contains("Requested Ceiling: 4500 mW"));
    assert!(!text.contains("Power Draw"));
}

#[test]
fn cable_notes_and_individual_flags_survive_unchanged_aggregate_warning() {
    let mut before = snapshot(None);
    before.summaries[0].cable = Some(usbeehive::cable::CableInfo::default());
    before.summaries[0].cable.as_mut().unwrap().trust.zero_vid = true;
    for notes_only in [true, false] {
        let mut after = before.clone();
        let trust = &mut after.summaries[0].cable.as_mut().unwrap().trust;
        if notes_only {
            trust.notes.push("New evidence".into());
        } else {
            trust.zero_vid = false;
            trust.reserved_bits_set = true;
        }
        let mut history = SnapshotHistory::new();
        history.push(before.clone(), 0);
        let events = history.push(after, 1);
        let event = events
            .iter()
            .find_map(|t| {
                if let StateTransition::CableWarningChanged(e) = t {
                    Some(e)
                } else {
                    None
                }
            })
            .expect("changed cable evidence must be retained");
        assert!(!event.warning_raised && !event.warning_cleared);
        assert!(
            events.iter().any(|t| t.to_string().contains(&event.reason)),
            "text events must preserve changed evidence"
        );
        if notes_only {
            assert!(event.reason.contains("New evidence"));
        } else {
            assert!(
                event.reason.contains("reserved_bits_set") && event.reason.contains("zero_vid")
            );
        }
    }
}

#[test]
fn imported_history_normalizes_bounds_and_sequence_counter() {
    let mut original = SnapshotHistory::new();
    for n in 0..4 {
        original.push(snapshot(None), n);
    }
    for capacity in [0, 2] {
        let mut json = serde_json::to_value(&original).unwrap();
        json["capacity"] = capacity.into();
        json["total_pushed"] = 0.into();
        let mut history: SnapshotHistory = serde_json::from_value(json).unwrap();
        assert_eq!(history.capacity(), (capacity as usize).max(1));
        assert_eq!(history.len(), history.capacity());
        assert_eq!(history.latest().unwrap().sequence, 3);
        assert_eq!(history.total_pushed(), 4);
        history.push(snapshot(None), 4);
        assert_eq!(history.len(), history.capacity());
        assert_eq!(history.latest().unwrap().sequence, 4);
    }
}

#[test]
fn contract_presence_changes_are_diffed_and_recorded() {
    for (a, b) in [
        (None, Some("5V @ 3A")),
        (Some("5V @ 3A"), None),
        (Some("5V"), Some("9V")),
    ] {
        let before = snapshot(a);
        let after = snapshot(b);
        let diff = compare(&before, &after);
        assert_eq!(diff.modified.len(), 1, "contract {a:?} -> {b:?}");
        let text = diff.render_text();
        assert!(text.contains(a.unwrap_or("[absent]")) && text.contains(b.unwrap_or("[absent]")));
        let mut history = SnapshotHistory::new();
        history.push(before, 0);
        assert!(history
            .push(after, 1)
            .iter()
            .any(|t| matches!(t, StateTransition::ContractRenegotiated(_))));
    }
}
