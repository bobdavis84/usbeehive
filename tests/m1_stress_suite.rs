//! Milestone 1 Adversarial & Stress Testing Suite
//!
//! Validates:
//! - R2: Diagnostic reasoning, actionability, backwards compatibility
//! - R7: Cable trust classification, dirty bit checks, contract mismatches, conservative hedging
//! - Pure-core compilation (zero I/O, zero network/fs) under all feature combinations
//! - Serialization stability and preservation of legacy fields

use usbeehive::cable::{CableInfo, CableTrust};
use usbeehive::diagnostic::{
    Bottleneck, ChargingDiagnostic, DiagnosticConfidence, DiagnosticStatus, RootCauseCategory,
};
use usbeehive::pd::{
    cable_vdo_reserved_bits_set, id_header_reserved_bits_set, CableCurrent, CableSpeed,
};
use usbeehive::power::{PdoType, PowerDataObject, PowerDeliveryPort};
use usbeehive::trust::{assess_cable_trust, CableTrustLevel, CABLE_TRUST_LIMITATIONS_NOTE};

fn make_pdo(power_mw: u32, current_ma: u32, voltage_mv: u32, active: bool) -> PowerDataObject {
    PowerDataObject {
        r#type: PdoType::FixedSupply,
        power_mw,
        current_ma,
        voltage_mv,
        is_active: active,
        ..Default::default()
    }
}

fn make_pd_port(caps: Vec<PowerDataObject>) -> PowerDeliveryPort {
    let max = caps.iter().map(|p| p.power_mw).max().unwrap_or(0);
    PowerDeliveryPort {
        source_capabilities: caps,
        max_source_power_mw: max,
        ..Default::default()
    }
}

// =========================================================================
// 1. Backwards Compatibility & Semantics Preservation
// =========================================================================

#[test]
fn test_backwards_compat_destructuring() {
    let port = make_pd_port(vec![make_pdo(100_000, 5000, 20_000, true)]);
    let cable = CableInfo {
        max_watts: 60,
        current_rating: Some(CableCurrent::ThreeAmp),
        ..Default::default()
    };

    let diag = ChargingDiagnostic::evaluate(&port, Some(&cable), 0, 0).expect("diagnostic present");

    // Legacy callers must be able to destructure or access exact 4 legacy fields
    let ChargingDiagnostic {
        bottleneck,
        summary,
        detail,
        is_warning,
        ..
    } = &diag;

    assert_eq!(*bottleneck, Bottleneck::CableLimit);
    assert!(*is_warning);
    assert_eq!(summary, "Cable rating is below available source capability");
    assert!(detail.contains("60W"));
    assert!(detail.contains("100W"));
    assert!(detail.contains("demand is unavailable"));
}

#[test]
fn test_backwards_compat_legacy_scenarios_exact_match() {
    // Scenario 1: Empty port -> None
    assert!(ChargingDiagnostic::evaluate(&PowerDeliveryPort::default(), None, 0, 0).is_none());

    // Scenario 2: 0W max source -> None
    let zero_port = make_pd_port(vec![make_pdo(0, 0, 0, false)]);
    assert!(ChargingDiagnostic::evaluate(&zero_port, None, 0, 0).is_none());

    // Scenario 3: DeviceLimit (<80% of charger)
    let port_device = make_pd_port(vec![
        make_pdo(100_000, 5000, 20_000, false),
        make_pdo(15_000, 3000, 5000, true),
    ]);
    let d = ChargingDiagnostic::evaluate(&port_device, None, 0, 0).unwrap();
    assert_eq!(d.bottleneck, Bottleneck::DeviceLimit);
    assert!(!d.is_warning);
    assert!(d.summary.contains("15W"));
    assert!(d.detail.contains("Negotiated 15W of the 100W"));

    // Scenario 4: SinkLimit (healthy contract, but sink requests <80%)
    let port_sink = make_pd_port(vec![make_pdo(65_000, 3250, 20_000, true)]);
    let cable_100w = CableInfo {
        max_watts: 100,
        current_rating: Some(CableCurrent::FiveAmp),
        ..Default::default()
    };
    let d = ChargingDiagnostic::evaluate(&port_sink, Some(&cable_100w), 15_000, 0).unwrap();
    assert_eq!(d.bottleneck, Bottleneck::SinkLimit);
    assert!(!d.is_warning);
    assert!(d.summary.contains("15W"));
    assert!(d.detail.contains("65W"));

    // Scenario 5: Fine (active matches charger)
    let port_fine = make_pd_port(vec![make_pdo(60_000, 3000, 20_000, true)]);
    let d = ChargingDiagnostic::evaluate(&port_fine, None, 0, 0).unwrap();
    assert_eq!(d.bottleneck, Bottleneck::Fine);
    assert!(!d.is_warning);
    assert_eq!(d.summary, "Charging at up to 60W");
    assert_eq!(d.detail, "");

    // Scenario 6: CableNoEMarker (pinned at 3A on >3A PDO without e-marker)
    let port_no_em = make_pd_port(vec![make_pdo(100_000, 5000, 20_000, true)]);
    let d = ChargingDiagnostic::evaluate(&port_no_em, None, 60_000, 3000).unwrap();
    assert_eq!(d.bottleneck, Bottleneck::CableNoEMarker);
    assert!(d.is_warning);
    assert_eq!(d.summary, "Cable may be limiting current to 3A");
}

// =========================================================================
// 2. Serialization & Wire Stability
// =========================================================================

#[test]
fn test_charging_diagnostic_json_serialization_keys() {
    let port = make_pd_port(vec![make_pdo(100_000, 5000, 20_000, true)]);
    let cable = CableInfo {
        max_watts: 60,
        ..Default::default()
    };
    let d = ChargingDiagnostic::evaluate(&port, Some(&cable), 0, 0).unwrap();

    let json_str = serde_json::to_string(&d).expect("serialize ChargingDiagnostic");
    let val: serde_json::Value = serde_json::from_str(&json_str).expect("deserialize JSON value");

    // Check all legacy keys
    assert_eq!(val["bottleneck"], "CableLimit");
    assert_eq!(
        val["summary"],
        "Cable rating is below available source capability"
    );
    assert_eq!(
        val["detail"],
        "Selected source offer is 100W and exceeds the cable's 60W rating; sink operating demand is unavailable."
    );
    assert_eq!(val["is_warning"].as_bool(), Some(true));

    // Check all additive R2 keys
    assert_eq!(val["status"], "Warning");
    assert_eq!(val["root_cause"], "Cable");
    assert_eq!(val["confidence"], "Medium");
    assert_eq!(val["rule_id"], "CABLE_LIMIT");
    assert!(val["recommended_action"]
        .as_str()
        .unwrap()
        .contains("Check the sink's operating demand"));
}

#[test]
fn test_cable_trust_json_serialization_keys() {
    let trust = CableTrust {
        zero_vid: true,
        vid_unknown: false,
        reserved_bits_set: true,
        trust_level: CableTrustLevel::Suspicious,
        emarker_contract_mismatch: true,
        unsupported_combination: false,
        notes: vec!["Suspicious test note".into()],
    };

    let json_str = serde_json::to_string(&trust).expect("serialize CableTrust");
    let val: serde_json::Value = serde_json::from_str(&json_str).expect("deserialize JSON value");

    assert_eq!(val["zero_vid"].as_bool(), Some(true));
    assert_eq!(val["vid_unknown"].as_bool(), Some(false));
    assert_eq!(val["reserved_bits_set"].as_bool(), Some(true));
    assert_eq!(val["trust_level"], "Suspicious");
    assert_eq!(val["emarker_contract_mismatch"].as_bool(), Some(true));
    assert_eq!(val["unsupported_combination"].as_bool(), Some(false));
    assert_eq!(val["notes"][0], "Suspicious test note");
}

#[test]
fn test_trust_assessment_json_serialization_keys() {
    // Missing identity is unavailable evidence, not a confirmed unsafe cable.
    let assessment = assess_cable_trust(None, None, 5000, 20000);
    let json_str = serde_json::to_string(&assessment).expect("serialize TrustAssessment");
    let val: serde_json::Value = serde_json::from_str(&json_str).expect("deserialize JSON value");

    assert_eq!(val["level"], "Inconclusive");
    assert_eq!(val["confidence"], "Low");
    assert_eq!(
        val["summary"],
        "Cable trust inconclusive due to missing controller telemetry."
    );
    assert_eq!(val["limitations_note"], CABLE_TRUST_LIMITATIONS_NOTE);
    assert!(!val["flags"].as_array().unwrap().is_empty());
    assert_eq!(val["flags"][0]["code"], "TELEMETRY_UNAVAILABLE");
    assert_eq!(val["flags"][0]["is_warning"].as_bool(), Some(false));
}

// =========================================================================
// 3. Zero-I/O Verification
// =========================================================================

#[test]
fn test_zero_io_pure_computation() {
    // Count open file descriptors in /proc/self/fd before and after executing
    // a large batch of diagnostic and trust evaluations.
    let count_fds = || -> usize {
        std::fs::read_dir("/proc/self/fd")
            .map(|entries| entries.count())
            .unwrap_or(0)
    };

    let fds_before = count_fds();

    let port = make_pd_port(vec![make_pdo(100_000, 5000, 20_000, true)]);
    let mut cable = CableInfo {
        max_watts: 60,
        vendor_id: 0x05AC,
        vendor_name: "Apple".into(),
        current_rating: Some(CableCurrent::ThreeAmp),
        ..Default::default()
    };

    for i in 0..1000 {
        let req_mw = (i % 100) * 1000;
        let req_ma = (i % 5) * 1000;
        let _ = ChargingDiagnostic::evaluate(&port, Some(&cable), req_mw, req_ma);
        cable.check_contract(req_ma, 20_000);
        let _ = assess_cable_trust(Some(&cable), Some(&port), req_ma, 20_000);
    }

    let fds_after = count_fds();
    assert_eq!(
        fds_before, fds_after,
        "Pure core diagnostics leaked or created file descriptors!"
    );
}

// =========================================================================
// 4. Exact Boundary Stress Testing
// =========================================================================

#[test]
fn test_exact_80_percent_threshold_boundaries() {
    // 1. DeviceLimit threshold: active_w < charger_max_w * 0.8
    // charger = 100W. 80W is exact 80%.
    // 79W -> < 80W -> DeviceLimit
    // 80W -> not < 80W -> Fine
    // 81W -> not < 80W -> Fine
    let cable_100w = CableInfo {
        max_watts: 100,
        current_rating: Some(CableCurrent::FiveAmp),
        ..Default::default()
    };

    let port_79 = make_pd_port(vec![
        make_pdo(100_000, 5000, 20_000, false),
        make_pdo(79_000, 3950, 20_000, true),
    ]);
    let d79 = ChargingDiagnostic::evaluate(&port_79, Some(&cable_100w), 0, 0).unwrap();
    assert_eq!(d79.bottleneck, Bottleneck::DeviceLimit);
    assert_eq!(d79.status, DiagnosticStatus::Healthy);
    assert_eq!(d79.root_cause, RootCauseCategory::Sink);

    let port_80 = make_pd_port(vec![
        make_pdo(100_000, 5000, 20_000, false),
        make_pdo(80_000, 4000, 20_000, true),
    ]);
    let d80 = ChargingDiagnostic::evaluate(&port_80, Some(&cable_100w), 0, 0).unwrap();
    assert_eq!(d80.bottleneck, Bottleneck::Fine);
    assert_eq!(d80.status, DiagnosticStatus::Healthy);
    assert_eq!(d80.root_cause, RootCauseCategory::None);

    let port_81 = make_pd_port(vec![
        make_pdo(100_000, 5000, 20_000, false),
        make_pdo(81_000, 4050, 20_000, true),
    ]);
    let d81 = ChargingDiagnostic::evaluate(&port_81, Some(&cable_100w), 0, 0).unwrap();
    assert_eq!(d81.bottleneck, Bottleneck::Fine);

    // 2. SinkLimit threshold: requested_w < active_w * 0.8
    // active = 100W. 80W is exact 80%.
    // requested = 79W -> SinkLimit
    // requested = 80W -> Fine
    // requested = 81W -> Fine
    let port_100 = make_pd_port(vec![make_pdo(100_000, 5000, 20_000, true)]);
    let ds79 = ChargingDiagnostic::evaluate(&port_100, Some(&cable_100w), 79_000, 0).unwrap();
    assert_eq!(ds79.bottleneck, Bottleneck::SinkLimit);
    assert_eq!(ds79.status, DiagnosticStatus::Healthy);
    assert_eq!(ds79.root_cause, RootCauseCategory::Sink);

    let ds80 = ChargingDiagnostic::evaluate(&port_100, Some(&cable_100w), 80_000, 0).unwrap();
    assert_eq!(ds80.bottleneck, Bottleneck::Fine);

    let ds81 = ChargingDiagnostic::evaluate(&port_100, Some(&cable_100w), 81_000, 0).unwrap();
    assert_eq!(ds81.bottleneck, Bottleneck::Fine);
}

#[test]
fn test_contract_out_of_spec_boundaries() {
    // Requirement for ContractOutOfSpec:
    // Known 3A rating and reported operating current > 3200; PDO ceiling is not draw.
    let cable_3a = CableInfo {
        max_watts: 60,
        current_rating: Some(CableCurrent::ThreeAmp),
        ..Default::default()
    };

    // Boundary 1: active_ma = 3200 (exact threshold, not > 3200) -> CableLimit
    let port_3200 = make_pd_port(vec![make_pdo(100_000, 3200, 20_000, true)]);
    let d = ChargingDiagnostic::evaluate(&port_3200, Some(&cable_3a), 65_000, 3200).unwrap();
    assert_eq!(d.bottleneck, Bottleneck::CableLimit);

    // Boundary 2: active_ma = 3201, requested_ma = 3100 (exact threshold, not > 3100) -> CableLimit
    let port_3201 = make_pd_port(vec![make_pdo(100_000, 3201, 20_000, true)]);
    let d = ChargingDiagnostic::evaluate(&port_3201, Some(&cable_3a), 65_000, 3100).unwrap();
    assert_eq!(d.bottleneck, Bottleneck::CableLimit);

    // Boundary 3: operating current = 3201 -> ContractOutOfSpec!
    let d = ChargingDiagnostic::evaluate(&port_3201, Some(&cable_3a), 65_000, 3201).unwrap();
    assert_eq!(d.bottleneck, Bottleneck::ContractOutOfSpec);
    assert_eq!(d.status, DiagnosticStatus::Critical);
    assert_eq!(d.root_cause, RootCauseCategory::Firmware);
    assert_eq!(d.confidence, DiagnosticConfidence::High);
}

// =========================================================================
// 5. Cable VDO & ID Header Reserved Bits Exhaustive Testing
// =========================================================================

#[test]
fn test_id_header_reserved_bits_exhaustive() {
    // ID Header mask: bits 16..20 (0x001F_0000) and bit 22 (0x0040_0000) = 0x005F_0000
    let expected_reserved_bits = [16, 17, 18, 19, 20, 22];

    for bit in 0..32 {
        let vdo = 1u32 << bit;
        let is_reserved = id_header_reserved_bits_set(vdo);
        if expected_reserved_bits.contains(&bit) {
            assert!(
                is_reserved,
                "Bit {bit} in ID Header was expected to be reserved"
            );
        } else {
            assert!(
                !is_reserved,
                "Bit {bit} in ID Header was NOT expected to be reserved"
            );
        }
    }
}

#[test]
fn test_cable_vdo_passive_reserved_bits_exhaustive() {
    // Passive cable VDO mask: bit 3, bits 7..8, bits 11..17 = 0x0003_F988
    let expected_reserved_bits = [3, 7, 8, 11, 12, 13, 14, 15, 16, 17];

    for bit in 0..32 {
        let vdo = 1u32 << bit;
        let is_reserved = cable_vdo_reserved_bits_set(vdo, false);
        if expected_reserved_bits.contains(&bit) {
            assert!(
                is_reserved,
                "Bit {bit} in passive Cable VDO was expected to be reserved"
            );
        } else {
            assert!(
                !is_reserved,
                "Bit {bit} in passive Cable VDO was NOT expected to be reserved"
            );
        }
    }
}

#[test]
fn test_cable_vdo_active_reserved_bits_exhaustive() {
    // Active cable VDO mask: bit 3 only (0x0000_0008)
    for bit in 0..32 {
        let vdo = 1u32 << bit;
        let is_reserved = cable_vdo_reserved_bits_set(vdo, true);
        if bit == 3 {
            assert!(
                is_reserved,
                "Bit 3 in active Cable VDO was expected to be reserved"
            );
        } else {
            assert!(
                !is_reserved,
                "Bit {bit} in active Cable VDO was NOT expected to be reserved"
            );
        }
    }
}

// =========================================================================
// 6. Extreme & Adversarial Numerical Inputs (No Panics)
// =========================================================================

#[test]
fn test_numerical_limits_and_overflow_protection() {
    // Test u32::MAX values across evaluate
    let extreme_pdo = make_pdo(u32::MAX, u32::MAX, u32::MAX, true);
    let mut extreme_port = make_pd_port(vec![extreme_pdo]);
    extreme_port.max_source_power_mw = u32::MAX;

    let extreme_cable = CableInfo {
        max_watts: u32::MAX,
        current_rating: Some(CableCurrent::FiveAmp),
        speed: Some(CableSpeed::Usb32Gen2),
        ..Default::default()
    };

    // Must execute cleanly without overflow panic
    let diag =
        ChargingDiagnostic::evaluate(&extreme_port, Some(&extreme_cable), u32::MAX, u32::MAX);
    assert!(diag.is_some());

    // Test extreme voltages in trust assessment
    let trust_assessment = assess_cable_trust(
        Some(&extreme_cable),
        Some(&extreme_port),
        u32::MAX,
        u32::MAX,
    );
    assert_eq!(
        trust_assessment.limitations_note,
        CABLE_TRUST_LIMITATIONS_NOTE
    );

    // Test zeroed / partial combinations
    let partial_port = make_pd_port(vec![make_pdo(5000, 0, 0, true)]);
    let _ = ChargingDiagnostic::evaluate(&partial_port, None, 0, 0);
}

#[test]
fn test_conservative_hedging_unknown_vid_not_suspicious() {
    // VID 0x1234 is not in the bundled database.
    // Conservative hedging rule: MUST be classified as KnownGood with an advisory note,
    // NOT Suspicious!
    let mut cable = CableInfo {
        vendor_id: 0x1234,
        vendor_name: "0x1234".into(),
        max_watts: 100,
        current_rating: Some(CableCurrent::FiveAmp),
        ..Default::default()
    };
    cable.trust.vid_unknown = true;
    cable.trust.trust_level = CableTrustLevel::KnownGood;

    let port = make_pd_port(vec![make_pdo(100_000, 5000, 20_000, true)]);
    let assessment = assess_cable_trust(Some(&cable), Some(&port), 5000, 20000);

    assert_eq!(assessment.level, CableTrustLevel::KnownGood);
    assert_eq!(assessment.confidence, DiagnosticConfidence::High);
    assert!(!assessment.level.is_suspicious());
    assert!(assessment
        .flags
        .iter()
        .any(|f| f.code == "VID_UNKNOWN_OFFLINE"));
    assert!(!assessment.flags.iter().any(|f| f.is_warning));
}
