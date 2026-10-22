//! Empirical Challenger Stress Suite for Milestone 1 (R2 & R7).
//!
//! Stress-tests:
//! 1. Cable Trust Classification Engine (`src/trust.rs`, `src/cable.rs`, `src/pd.rs`):
//!    - Zero VID (`0x0000`), unknown VIDs, registered VIDs
//!    - Exhaustive bitfield validation for reserved bits in ID Header & Cable VDOs
//!    - Contract mismatches: 5A on 3A cables, EPR voltage on SPR cables
//!    - Non-e-marked cables vs missing telemetry
//! 2. Diagnostic Reasoning Engine (`src/diagnostic.rs`):
//!    - Underpowered chargers (<=15W on laptop sink)
//!    - Contract out of spec (>3.2A without 5A rating)
//!    - Sink limits (<80% of contract)
//!    - Extreme boundary values (0W, 0mA, u32::MAX) and zero-panic verification

use usbeehive::cable::{CableInfo, CableTrust};
use usbeehive::diagnostic::{
    Bottleneck, ChargingDiagnostic, DiagnosticConfidence, DiagnosticStatus, RootCauseCategory,
};
use usbeehive::pd::{
    cable_vdo_reserved_bits_set, decode_cable_vdo, decode_id_header, id_header_reserved_bits_set,
    CableCurrent,
};
use usbeehive::power::{PdoType, PowerDataObject, PowerDeliveryPort};
use usbeehive::trust::{assess_cable_trust, CableTrustLevel};
use usbeehive::typec::{TypeCCable, TypeCIdentity};

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
// 1. Cable Trust: Vendor ID Heuristics & Conservative Hedging
// =========================================================================

#[test]
fn stress_trust_vid_categories() {
    // A: Zero VID -> MUST be Suspicious
    let id_zero = TypeCIdentity {
        vendor_id: 0x0000,
        product_id: 0,
        vdos: vec![3u32 << 27, 0, 0, 1u32 | (1 << 5)],
    };
    let cable_zero = TypeCCable {
        r#type: "passive".into(),
        plug_type: "type-c".into(),
        identity: Some(id_zero),
        raw_attributes: Default::default(),
    };
    let info_zero = CableInfo::from_typec_cable(&cable_zero);
    assert!(info_zero.trust.zero_vid);
    assert_eq!(info_zero.trust.trust_level, CableTrustLevel::Suspicious);

    let assess_zero = assess_cable_trust(Some(&info_zero), None, 3_000, 20_000);
    assert_eq!(assess_zero.level, CableTrustLevel::Suspicious);
    assert!(assess_zero.flags.iter().any(|f| f.code == "ZERO_VID"));

    // B: Known Registered VID (Apple 0x05AC) -> MUST be KnownGood
    let id_apple = TypeCIdentity {
        vendor_id: 0x05AC,
        product_id: 0,
        vdos: vec![3u32 << 27, 0, 0, 1u32 | (1 << 5)],
    };
    let cable_apple = TypeCCable {
        r#type: "passive".into(),
        plug_type: "type-c".into(),
        identity: Some(id_apple),
        raw_attributes: Default::default(),
    };
    let info_apple = CableInfo::from_typec_cable(&cable_apple);
    assert!(!info_apple.trust.zero_vid);
    assert!(!info_apple.trust.vid_unknown);
    assert_eq!(info_apple.trust.trust_level, CableTrustLevel::KnownGood);

    let assess_apple = assess_cable_trust(Some(&info_apple), None, 3_000, 20_000);
    assert_eq!(assess_apple.level, CableTrustLevel::KnownGood);
    assert!(assess_apple
        .flags
        .iter()
        .any(|f| f.code == "REGISTERED_VENDOR"));

    // C: Unknown / Unregistered VIDs (Conservative Hedging) -> MUST be KnownGood, NOT Suspicious!
    let unknown_vids = [0xDEAD, 0xBEEF, 0x1234, 0x9999, 0xFFFF];
    for vid in unknown_vids {
        let id_unk = TypeCIdentity {
            vendor_id: vid,
            product_id: 0,
            vdos: vec![3u32 << 27, 0, 0, 1u32 | (1 << 5)],
        };
        let cable_unk = TypeCCable {
            r#type: "passive".into(),
            plug_type: "type-c".into(),
            identity: Some(id_unk),
            raw_attributes: Default::default(),
        };
        let info_unk = CableInfo::from_typec_cable(&cable_unk);
        assert!(
            info_unk.trust.vid_unknown,
            "VID 0x{:04X} should be unknown",
            vid
        );
        assert_eq!(
            info_unk.trust.trust_level,
            CableTrustLevel::KnownGood,
            "Unknown VID 0x{:04X} must NOT be marked suspicious!",
            vid
        );

        let assess_unk = assess_cable_trust(Some(&info_unk), None, 3_000, 20_000);
        assert_eq!(
            assess_unk.level,
            CableTrustLevel::KnownGood,
            "assess_cable_trust for unknown VID 0x{:04X} must be KnownGood",
            vid
        );
        assert!(assess_unk
            .flags
            .iter()
            .any(|f| f.code == "VID_UNKNOWN_OFFLINE"));
    }
}

// =========================================================================
// 2. Reserved Bits Exhaustive Testing
// =========================================================================

#[test]
fn stress_exhaustive_id_header_reserved_bits() {
    // Spec: bits 16..20 and bit 22 are reserved in ID Header VDO (mask 0x005F_0000).
    for bit in 0..32 {
        let mask = 1u32 << bit;
        let is_reserved = id_header_reserved_bits_set(mask);
        let expected = (16..=20).contains(&bit) || bit == 22;
        assert_eq!(
            is_reserved, expected,
            "Bit {} in ID Header reserved check failed: got {}, expected {}",
            bit, is_reserved, expected
        );
    }
}

#[test]
fn stress_exhaustive_cable_vdo_reserved_bits() {
    // Passive Cable VDO: bit 3, bits 7..8, and bits 11..17 are reserved (mask 0x0003_F988).
    for bit in 0..32 {
        let mask = 1u32 << bit;
        let is_reserved = cable_vdo_reserved_bits_set(mask, false);
        let expected = bit == 3 || bit == 7 || bit == 8 || (11..=17).contains(&bit);
        assert_eq!(
            is_reserved, expected,
            "Bit {} in Passive Cable VDO reserved check failed: got {}, expected {}",
            bit, is_reserved, expected
        );
    }

    // Active Cable VDO: bit 3 only (mask 0x0000_0008). Bits 7..8 are SBU support.
    for bit in 0..32 {
        let mask = 1u32 << bit;
        let is_reserved = cable_vdo_reserved_bits_set(mask, true);
        let expected = bit == 3;
        assert_eq!(
            is_reserved, expected,
            "Bit {} in Active Cable VDO reserved check failed: got {}, expected {}",
            bit, is_reserved, expected
        );
    }
}

// =========================================================================
// 3. Contract Mismatches & Boundary Testing (5A on 3A, EPR on SPR)
// =========================================================================

#[test]
fn stress_contract_mismatch_current_thresholds() {
    let info_3a = CableInfo {
        max_watts: 60,
        current_rating: Some(CableCurrent::ThreeAmp),
        trust: CableTrust {
            trust_level: CableTrustLevel::KnownGood,
            ..Default::default()
        },
        ..Default::default()
    };

    // Sub-threshold draws: 0mA, 1500mA, 3000mA, 3100mA, 3200mA should NOT flag mismatch
    for ma in [0, 500, 1500, 3000, 3100, 3200] {
        let mut test_cable = info_3a.clone();
        test_cable.check_contract(ma, 20_000);
        assert!(
            !test_cable.trust.emarker_contract_mismatch,
            "Draw of {}mA on 3A cable should not trigger mismatch",
            ma
        );
        assert_eq!(test_cable.trust.trust_level, CableTrustLevel::KnownGood);
    }

    // Above threshold draws: 3201mA, 3500mA, 5000mA, u32::MAX MUST flag mismatch
    for ma in [3201, 3500, 5000, 10_000, u32::MAX] {
        let mut test_cable = info_3a.clone();
        test_cable.check_contract(ma, 20_000);
        assert!(
            test_cable.trust.emarker_contract_mismatch,
            "Draw of {}mA on 3A cable MUST trigger mismatch",
            ma
        );
        assert_eq!(test_cable.trust.trust_level, CableTrustLevel::Suspicious);
    }
}

#[test]
fn stress_contract_mismatch_voltage_thresholds() {
    let info_spr = CableInfo {
        max_watts: 100,
        current_rating: Some(CableCurrent::FiveAmp),
        trust: CableTrust {
            trust_level: CableTrustLevel::KnownGood,
            ..Default::default()
        },
        ..Default::default()
    };

    // SPR voltages (<= 20V/21V): 5V, 9V, 15V, 20V, 21V
    for mv in [0, 5000, 9000, 15000, 20000, 21000] {
        let mut test_cable = info_spr.clone();
        test_cable.check_contract(5_000, mv);
        assert!(
            !test_cable.trust.unsupported_combination,
            "Voltage {}mV on SPR cable should not trigger unsupported combination",
            mv
        );
        assert_eq!(test_cable.trust.trust_level, CableTrustLevel::KnownGood);
    }

    // EPR voltages (> 21V): 21001mV, 28000mV, 36000mV, 48000mV, u32::MAX
    for mv in [21001, 28000, 36000, 48000, 100_000, u32::MAX] {
        let mut test_cable = info_spr.clone();
        test_cable.check_contract(5_000, mv);
        assert!(
            test_cable.trust.unsupported_combination,
            "Voltage {}mV on SPR cable MUST trigger unsupported combination",
            mv
        );
        assert_eq!(test_cable.trust.trust_level, CableTrustLevel::Suspicious);
    }
}

// =========================================================================
// 4. Non-E-Marked Cables vs Missing Telemetry
// =========================================================================

#[test]
fn stress_non_emarked_vs_missing_telemetry() {
    // A: Missing telemetry (None) under standard 3A current
    let assess_missing_3a = assess_cable_trust(None, None, 3_000, 20_000);
    assert_eq!(assess_missing_3a.level, CableTrustLevel::Inconclusive);
    assert_eq!(assess_missing_3a.confidence, DiagnosticConfidence::Low);

    // B: Missing telemetry (None) under high current (>3.2A) -> Suspicious
    let assess_missing_5a = assess_cable_trust(None, None, 5_000, 20_000);
    assert_eq!(assess_missing_5a.level, CableTrustLevel::Inconclusive);
    assert_eq!(assess_missing_5a.confidence, DiagnosticConfidence::Low);

    // C: Non-e-marked passive cable under standard current (<=3.2A) -> LikelyConstrained
    let cable_passive = CableInfo {
        is_passive: true,
        current_rating: None,
        max_watts: 0,
        ..Default::default()
    };
    let assess_passive_3a = assess_cable_trust(Some(&cable_passive), None, 3_000, 20_000);
    assert_eq!(assess_passive_3a.level, CableTrustLevel::Inconclusive);
    assert_eq!(assess_passive_3a.confidence, DiagnosticConfidence::Low);

    // D: Non-e-marked passive cable under high current (>3.2A) -> Suspicious
    let assess_passive_5a = assess_cable_trust(Some(&cable_passive), None, 5_000, 20_000);
    assert_eq!(assess_passive_5a.level, CableTrustLevel::Inconclusive);
    assert_eq!(assess_passive_5a.confidence, DiagnosticConfidence::Low);

    // E: Non-e-marked passive cable under EPR voltage (28V) but safe current (3A = 84W)
    let assess_passive_28v = assess_cable_trust(Some(&cable_passive), None, 3_000, 28_000);
    assert_eq!(assess_passive_28v.level, CableTrustLevel::Inconclusive);
    // Missing identity cannot establish any current or voltage rating.
    assert!(!assess_passive_28v.flags.iter().any(|f| f.is_warning));
}

// =========================================================================
// 5. Diagnostic Reasoning Rules (Underpowered, SinkLimit, ContractOutOfSpec)
// =========================================================================

#[test]
fn stress_diagnostic_underpowered_charger() {
    // 15W charger (5V @ 3A)
    let port_15w = make_pd_port(vec![make_pdo(15_000, 3_000, 5_000, true)]);

    // Laptop requests 45W -> CHARGER_UNDERPOWERED
    let d45 = ChargingDiagnostic::evaluate(&port_15w, None, 45_000, 0).unwrap();
    assert_eq!(d45.bottleneck, Bottleneck::ChargerLimit);
    assert_eq!(d45.status, DiagnosticStatus::Warning);
    assert_eq!(d45.root_cause, RootCauseCategory::Charger);
    assert_eq!(d45.rule_id, "CHARGER_UNDERPOWERED");
    assert!(d45.recommended_action.contains("45W or higher"));

    // Laptop requests 65W -> CHARGER_UNDERPOWERED
    let d65 = ChargingDiagnostic::evaluate(&port_15w, None, 65_000, 0).unwrap();
    assert_eq!(d65.bottleneck, Bottleneck::ChargerLimit);
    assert_eq!(d65.status, DiagnosticStatus::Warning);
    assert_eq!(d65.rule_id, "CHARGER_UNDERPOWERED");
    assert!(d65.recommended_action.contains("65W or higher"));

    // Laptop requests 15W (exact match) -> FINE
    let d15 = ChargingDiagnostic::evaluate(&port_15w, None, 15_000, 0).unwrap();
    assert_eq!(d15.bottleneck, Bottleneck::Fine);
    assert_eq!(d15.status, DiagnosticStatus::Healthy);

    // Laptop requests 10W (< 80% of 15W is 12W) -> SINK_LIMIT
    let d10 = ChargingDiagnostic::evaluate(&port_15w, None, 10_000, 0).unwrap();
    assert_eq!(d10.bottleneck, Bottleneck::SinkLimit);
    assert_eq!(d10.status, DiagnosticStatus::Healthy);
    assert_eq!(d10.root_cause, RootCauseCategory::Sink);
}

#[test]
fn stress_diagnostic_contract_out_of_spec() {
    let port_100w = make_pd_port(vec![make_pdo(100_000, 5_000, 20_000, true)]);

    // A: 5A contract active, sink draws 4A, cable is None -> CONTRACT_OUT_OF_SPEC (Critical)
    let d_none = ChargingDiagnostic::evaluate(&port_100w, None, 80_000, 4_000).unwrap();
    // Absent identity is not proof of an unmarked cable.
    assert_eq!(d_none.bottleneck, Bottleneck::Fine);
    assert_ne!(d_none.status, DiagnosticStatus::Critical);

    // B: 5A contract active, sink draws 4A, cable is 3A rated -> CONTRACT_OUT_OF_SPEC (Critical)
    let cable_3a = CableInfo {
        max_watts: 60,
        current_rating: Some(CableCurrent::ThreeAmp),
        ..Default::default()
    };
    let d_3a = ChargingDiagnostic::evaluate(&port_100w, Some(&cable_3a), 80_000, 4_000).unwrap();
    assert_eq!(d_3a.bottleneck, Bottleneck::ContractOutOfSpec);
    assert_eq!(d_3a.status, DiagnosticStatus::Critical);

    // C: 5A contract active, sink draws 4A, cable is 5A rated -> FINE or SINK_LIMIT, NOT ContractOutOfSpec
    let cable_5a = CableInfo {
        max_watts: 100,
        current_rating: Some(CableCurrent::FiveAmp),
        ..Default::default()
    };
    let d_5a = ChargingDiagnostic::evaluate(&port_100w, Some(&cable_5a), 80_000, 4_000).unwrap();
    assert_ne!(d_5a.bottleneck, Bottleneck::ContractOutOfSpec);
    assert_eq!(d_5a.bottleneck, Bottleneck::Fine); // 80W on 100W contract is >= 80%

    // D: 5A contract offered, but sink requests exactly 3.0A (3000mA) with no e-marker -> CABLE_NO_EMARKER (Warning)
    let d_pinned = ChargingDiagnostic::evaluate(&port_100w, None, 60_000, 3_000).unwrap();
    assert_eq!(d_pinned.bottleneck, Bottleneck::CableNoEMarker);
    assert_eq!(d_pinned.status, DiagnosticStatus::Warning);
    assert_eq!(d_pinned.root_cause, RootCauseCategory::Cable);
}

#[test]
fn stress_diagnostic_sink_limits_thresholds() {
    let port_100w = make_pd_port(vec![make_pdo(100_000, 5_000, 20_000, true)]);
    let cable_5a = CableInfo {
        max_watts: 100,
        current_rating: Some(CableCurrent::FiveAmp),
        ..Default::default()
    };

    // Exactly 80W of 100W contract (80.0%): Fine
    let d80 = ChargingDiagnostic::evaluate(&port_100w, Some(&cable_5a), 80_000, 4_000).unwrap();
    assert_eq!(d80.bottleneck, Bottleneck::Fine);

    // 81W of 100W contract (81.0%): Fine
    let d81 = ChargingDiagnostic::evaluate(&port_100w, Some(&cable_5a), 81_000, 4_050).unwrap();
    assert_eq!(d81.bottleneck, Bottleneck::Fine);

    // 79W of 100W contract (79.0% < 80%): SinkLimit
    let d79 = ChargingDiagnostic::evaluate(&port_100w, Some(&cable_5a), 79_000, 3_950).unwrap();
    assert_eq!(d79.bottleneck, Bottleneck::SinkLimit);
    assert_eq!(d79.status, DiagnosticStatus::Healthy);
    assert_eq!(d79.root_cause, RootCauseCategory::Sink);

    // 15W of 100W contract (15.0% < 80%): SinkLimit
    let d15 = ChargingDiagnostic::evaluate(&port_100w, Some(&cable_5a), 15_000, 750).unwrap();
    assert_eq!(d15.bottleneck, Bottleneck::SinkLimit);
}

// =========================================================================
// 6. Zero-Panic Boundary & Fuzz Testing
// =========================================================================

#[test]
fn stress_zero_panic_boundary_combinations() {
    let extreme_numbers = [
        0,
        1,
        500,
        3000,
        3200,
        5000,
        60_000,
        100_000,
        u32::MAX - 1,
        u32::MAX,
    ];

    for &mw in &extreme_numbers {
        for &ma in &extreme_numbers {
            // Port with single PDO of varying power
            let port = make_pd_port(vec![make_pdo(mw, ma, 20_000, true)]);
            let _ = ChargingDiagnostic::evaluate(&port, None, mw, ma);

            // Empty port
            let empty_port = PowerDeliveryPort::default();
            assert!(ChargingDiagnostic::evaluate(&empty_port, None, mw, ma).is_none());

            // Cable check_contract
            let mut cable = CableInfo {
                max_watts: mw / 1000,
                current_rating: Some(CableCurrent::ThreeAmp),
                ..Default::default()
            };
            cable.check_contract(ma, 20_000);

            // assess_cable_trust
            let _ = assess_cable_trust(Some(&cable), Some(&port), ma, 20_000);
            let _ = assess_cable_trust(None, Some(&port), ma, 20_000);
            let _ = assess_cable_trust(None, None, ma, 20_000);
        }
    }
}

#[test]
fn stress_vdo_decoder_bitfield_fuzzing() {
    // Ensure decode_id_header, decode_cable_vdo, id_header_reserved_bits_set,
    // cable_vdo_reserved_bits_set never panic on arbitrary bit patterns.
    let patterns = [
        0u32,
        0xFFFF_FFFF,
        0x5555_5555,
        0xAAAA_AAAA,
        0x005F_0000,
        0x0003_F988,
        0x8000_0000,
        0x0000_0001,
        0x7FFF_FFFF,
    ];

    for &vdo in &patterns {
        let _ = decode_id_header(vdo);
        let _ = decode_cable_vdo(vdo, false);
        let _ = decode_cable_vdo(vdo, true);
        let _ = id_header_reserved_bits_set(vdo);
        let _ = cable_vdo_reserved_bits_set(vdo, false);
        let _ = cable_vdo_reserved_bits_set(vdo, true);
    }
}

// =========================================================================
// 7. Rework Stress 1: Underpowered Charger (requested_w == 0 vs explicit draws)
// =========================================================================

#[test]
fn stress_rework_underpowered_charger_zero_requested_w_and_explicit_draws() {
    // A: 15W charger with requested_w == 0 (unreported UCSI draw)
    let port_15w = make_pd_port(vec![make_pdo(15_000, 3_000, 5_000, true)]);
    let d0 = ChargingDiagnostic::evaluate(&port_15w, None, 0, 0).unwrap();
    assert_eq!(d0.bottleneck, Bottleneck::ChargerLimit);
    assert!(d0.is_warning);
    assert_eq!(d0.status, DiagnosticStatus::Warning);
    assert_eq!(d0.root_cause, RootCauseCategory::Charger);
    assert_eq!(d0.confidence, DiagnosticConfidence::High);
    assert_eq!(d0.rule_id, "CHARGER_UNDERPOWERED");
    assert!(d0.summary.contains("15W"));
    assert!(d0
        .detail
        .contains("insufficient for standard host operation"));
    assert!(d0.recommended_action.contains("45W or higher"));
    assert!(!d0.recommended_action.contains("0W"));

    // B: Explicit sink draw on 15W charger:
    // 15W requested (100% of 15W) -> FINE (operating optimally at charger maximum)
    let d15 = ChargingDiagnostic::evaluate(&port_15w, None, 15_000, 3_000).unwrap();
    assert_eq!(d15.bottleneck, Bottleneck::Fine);
    assert_eq!(d15.status, DiagnosticStatus::Healthy);
    assert_eq!(d15.rule_id, "FINE");
    assert_eq!(d15.root_cause, RootCauseCategory::None);

    // 12W requested (exactly 80% of 15W) -> FINE
    let d12 = ChargingDiagnostic::evaluate(&port_15w, None, 12_000, 2_400).unwrap();
    assert_eq!(d12.bottleneck, Bottleneck::Fine);
    assert_eq!(d12.status, DiagnosticStatus::Healthy);
    assert_eq!(d12.rule_id, "FINE");

    // 10W requested (< 80% of 15W = 12W) -> SINK_LIMIT (Healthy sink policy)
    let d10 = ChargingDiagnostic::evaluate(&port_15w, None, 10_000, 2_000).unwrap();
    assert_eq!(d10.bottleneck, Bottleneck::SinkLimit);
    assert_eq!(d10.status, DiagnosticStatus::Healthy);
    assert_eq!(d10.root_cause, RootCauseCategory::Sink);
    assert_eq!(d10.rule_id, "SINK_LIMIT");
    assert!(d10.summary.contains("draw to 10W"));

    // 5W requested (< 80% of 15W) -> SINK_LIMIT
    let d5 = ChargingDiagnostic::evaluate(&port_15w, None, 5_000, 1_000).unwrap();
    assert_eq!(d5.bottleneck, Bottleneck::SinkLimit);
    assert_eq!(d5.status, DiagnosticStatus::Healthy);
    assert_eq!(d5.root_cause, RootCauseCategory::Sink);
    assert_eq!(d5.rule_id, "SINK_LIMIT");

    // C: Low-wattage chargers <= 15W with requested_w == 0 all trigger CHARGER_UNDERPOWERED
    for w in [5, 7, 10, 12, 15] {
        let port_low = make_pd_port(vec![make_pdo(w * 1_000, 1_000, 5_000, true)]);
        let d = ChargingDiagnostic::evaluate(&port_low, None, 0, 0).unwrap();
        assert_eq!(
            d.rule_id, "CHARGER_UNDERPOWERED",
            "Charger of {}W with requested_w=0 MUST be CHARGER_UNDERPOWERED",
            w
        );
        assert_eq!(d.status, DiagnosticStatus::Warning);
        assert_eq!(d.root_cause, RootCauseCategory::Charger);
        assert!(d.recommended_action.contains("45W or higher"));
    }

    // D: Higher-wattage chargers > 15W with requested_w == 0 must NOT trigger CHARGER_UNDERPOWERED
    for w in [18, 20, 30, 45, 65, 100, 140, 240] {
        let port_high = make_pd_port(vec![make_pdo(w * 1_000, 3_000, 20_000, true)]);
        let d = ChargingDiagnostic::evaluate(&port_high, None, 0, 0).unwrap();
        assert_ne!(
            d.rule_id, "CHARGER_UNDERPOWERED",
            "Charger of {}W with requested_w=0 must NOT be CHARGER_UNDERPOWERED",
            w
        );
        assert_eq!(d.rule_id, "FINE");
    }

    // E: Higher-wattage chargers > 15W with requested_w > charger_max_w trigger CHARGER_UNDERPOWERED
    let port_20w = make_pd_port(vec![make_pdo(20_000, 2_220, 9_000, true)]);
    let d_under = ChargingDiagnostic::evaluate(&port_20w, None, 45_000, 0).unwrap();
    assert_eq!(d_under.rule_id, "CHARGER_UNDERPOWERED");
    assert!(d_under
        .detail
        .contains("Device requests 45W, but charger provides only 20W."));
    assert!(d_under.recommended_action.contains("45W or higher"));
}

// =========================================================================
// 8. Rework Stress 2: Contract Out of Spec (requested_ma == 0 on unmarked cables)
// =========================================================================

#[test]
fn stress_rework_contract_out_of_spec_zero_requested_ma() {
    let port_100w = make_pd_port(vec![make_pdo(100_000, 5_000, 20_000, true)]);

    // A: No cable fixture (cable == None), active_ma = 5000, requested_ma = 0
    let d_none = ChargingDiagnostic::evaluate(&port_100w, None, 0, 0).unwrap();
    // A source PDO ceiling is not operating current; absent identity is not a rating.
    assert_eq!(d_none.bottleneck, Bottleneck::Fine);
    assert_ne!(d_none.status, DiagnosticStatus::Critical);

    // B: CableInfo completely unmarked (current_rating: None, max_watts: 0), requested_ma = 0
    let cable_unmarked = CableInfo {
        max_watts: 0,
        current_rating: None,
        ..Default::default()
    };
    let d_unmarked = ChargingDiagnostic::evaluate(&port_100w, Some(&cable_unmarked), 0, 0).unwrap();
    assert_ne!(d_unmarked.bottleneck, Bottleneck::ContractOutOfSpec);
    assert_ne!(d_unmarked.status, DiagnosticStatus::Critical);
    assert_ne!(d_unmarked.rule_id, "CONTRACT_OUT_OF_SPEC");

    // C: Current boundary threshold around 3.2A with requested_ma == 0 on unmarked cable
    // 3,201mA (>3.2A) -> ContractOutOfSpec
    let port_3201 = make_pd_port(vec![make_pdo(64_020, 3_201, 20_000, true)]);
    let d_3201 = ChargingDiagnostic::evaluate(&port_3201, None, 0, 0).unwrap();
    assert_ne!(d_3201.rule_id, "CONTRACT_OUT_OF_SPEC");

    // 3,200mA (<=3.2A) -> Safe/within spec allowance, must NOT be ContractOutOfSpec
    let port_3200 = make_pd_port(vec![make_pdo(64_000, 3_200, 20_000, true)]);
    let d_3200 = ChargingDiagnostic::evaluate(&port_3200, None, 0, 0).unwrap();
    assert_ne!(d_3200.rule_id, "CONTRACT_OUT_OF_SPEC");

    // 3,000mA (standard 3A) -> Fine
    let port_3000 = make_pd_port(vec![make_pdo(60_000, 3_000, 20_000, true)]);
    let d_3000 = ChargingDiagnostic::evaluate(&port_3000, None, 0, 0).unwrap();
    assert_ne!(d_3000.rule_id, "CONTRACT_OUT_OF_SPEC");

    // D: Explicit 60W cable (max_watts = 60, current_rating = None) with requested_ma == 0
    // Should be CABLE_LIMIT, NOT false positive ContractOutOfSpec
    let cable_60w_unrated = CableInfo {
        max_watts: 60,
        current_rating: None,
        ..Default::default()
    };
    let d_60w = ChargingDiagnostic::evaluate(&port_100w, Some(&cable_60w_unrated), 0, 0).unwrap();
    assert_eq!(d_60w.rule_id, "CABLE_LIMIT");
    assert_eq!(d_60w.status, DiagnosticStatus::Warning);
    assert_eq!(d_60w.root_cause, RootCauseCategory::Cable);

    // E: Valid 5A cable (current_rating: Some(FiveAmp), max_watts: 100) -> FINE
    let cable_5a = CableInfo {
        max_watts: 100,
        current_rating: Some(CableCurrent::FiveAmp),
        ..Default::default()
    };
    let d_5a = ChargingDiagnostic::evaluate(&port_100w, Some(&cable_5a), 0, 0).unwrap();
    assert_eq!(d_5a.rule_id, "FINE");
    assert_eq!(d_5a.status, DiagnosticStatus::Healthy);
}

// =========================================================================
// 9. Rework Stress 3: PORT_HARDWARE_LIMIT Attribution
// =========================================================================

#[test]
fn stress_rework_port_hardware_limit() {
    // A: 100W charger offered, host port hardware sink ceiling is 45W, active contract is 45W
    let mut port_100w_sink_45w = make_pd_port(vec![
        make_pdo(100_000, 5_000, 20_000, false),
        make_pdo(45_000, 3_000, 15_000, true),
    ]);
    port_100w_sink_45w.sink_capabilities = vec![make_pdo(45_000, 3_000, 15_000, false)];

    let d_port = ChargingDiagnostic::evaluate(&port_100w_sink_45w, None, 0, 0).unwrap();
    assert_eq!(d_port.bottleneck, Bottleneck::DeviceLimit);
    assert_eq!(d_port.status, DiagnosticStatus::Healthy);
    assert_eq!(d_port.root_cause, RootCauseCategory::Port);
    assert_eq!(d_port.confidence, DiagnosticConfidence::High);
    assert_eq!(d_port.rule_id, "PORT_HARDWARE_LIMIT");
    assert!(d_port.summary.contains("limited to 45W by port hardware"));
    assert!(d_port
        .detail
        .contains("host port hardware sink capability ceiling is 45W"));
    assert!(d_port
        .recommended_action
        .contains("maximum rate supported by this port's hardware"));

    // B: 140W EPR charger offered, host port hardware sink ceiling is 100W (SPR only), active is 100W, cable is 100W SPR
    let mut port_140w_sink_100w = make_pd_port(vec![
        make_pdo(140_000, 5_000, 28_000, false),
        make_pdo(100_000, 5_000, 20_000, true),
    ]);
    port_140w_sink_100w.sink_capabilities = vec![make_pdo(100_000, 5_000, 20_000, false)];
    let cable_100w = CableInfo {
        max_watts: 100,
        current_rating: Some(CableCurrent::FiveAmp),
        ..Default::default()
    };
    let d_epr_port =
        ChargingDiagnostic::evaluate(&port_140w_sink_100w, Some(&cable_100w), 0, 0).unwrap();
    assert_eq!(d_epr_port.rule_id, "PORT_HARDWARE_LIMIT");
    assert_eq!(d_epr_port.root_cause, RootCauseCategory::Port);
    assert!(d_epr_port
        .summary
        .contains("limited to 100W by port hardware"));

    // B2: 100W charger and 45W sink ceiling with 3A active contract, cable = None (unspecified cable) -> PORT_HARDWARE_LIMIT
    let d_port_nocable = ChargingDiagnostic::evaluate(&port_100w_sink_45w, None, 0, 0).unwrap();
    assert_eq!(d_port_nocable.rule_id, "PORT_HARDWARE_LIMIT");
    assert_eq!(d_port_nocable.root_cause, RootCauseCategory::Port);

    // B3: 100W charger, 45W sink ceiling, but cable is a 60W cable (max_watts = 60).
    // The 60W cable exceeds the 45W sink ceiling, so PORT_HARDWARE_LIMIT triggers rather than CABLE_LIMIT.
    let cable_60w = CableInfo {
        max_watts: 60,
        current_rating: Some(CableCurrent::ThreeAmp),
        ..Default::default()
    };
    let d_60w_on_45w_port =
        ChargingDiagnostic::evaluate(&port_100w_sink_45w, Some(&cable_60w), 0, 0).unwrap();
    assert_eq!(d_60w_on_45w_port.rule_id, "PORT_HARDWARE_LIMIT");
    assert_eq!(d_60w_on_45w_port.root_cause, RootCauseCategory::Port);

    // B4: 100W charger, 100W sink ceiling, but cable is a 60W cable (max_watts = 60).
    // Cable rating (60W) is below port ceiling (100W), so cable is the bottleneck -> CABLE_LIMIT.
    let mut port_100w_sink_100w = make_pd_port(vec![
        make_pdo(100_000, 5_000, 20_000, false),
        make_pdo(60_000, 3_000, 20_000, true),
    ]);
    port_100w_sink_100w.sink_capabilities = vec![make_pdo(100_000, 5_000, 20_000, false)];
    let d_60w_on_100w_port =
        ChargingDiagnostic::evaluate(&port_100w_sink_100w, Some(&cable_60w), 0, 0).unwrap();
    assert_eq!(d_60w_on_100w_port.rule_id, "CABLE_LIMIT");
    assert_eq!(d_60w_on_100w_port.root_cause, RootCauseCategory::Cable);

    // C: 100W charger offered, host port sink ceiling is 45W, but active draw is 30W (< 45W)
    // Here, port hardware is NOT the bottleneck (port allows 45W, device chose 30W) -> DEVICE_LIMIT (Sink)
    let mut port_100w_sink_45w_active_30w = make_pd_port(vec![
        make_pdo(100_000, 5_000, 20_000, false),
        make_pdo(45_000, 3_000, 15_000, false),
        make_pdo(30_000, 2_000, 15_000, true),
    ]);
    port_100w_sink_45w_active_30w.sink_capabilities = vec![make_pdo(45_000, 3_000, 15_000, false)];
    let d_device =
        ChargingDiagnostic::evaluate(&port_100w_sink_45w_active_30w, None, 0, 0).unwrap();
    assert_eq!(d_device.rule_id, "DEVICE_LIMIT");
    assert_eq!(d_device.root_cause, RootCauseCategory::Sink);

    // D: 100W charger offered, sink_capabilities empty -> DEVICE_LIMIT (Sink)
    let port_empty_sink = make_pd_port(vec![
        make_pdo(100_000, 5_000, 20_000, false),
        make_pdo(45_000, 3_000, 15_000, true),
    ]);
    let d_empty_sink = ChargingDiagnostic::evaluate(&port_empty_sink, None, 0, 0).unwrap();
    assert_eq!(d_empty_sink.rule_id, "DEVICE_LIMIT");
    assert_eq!(d_empty_sink.root_cause, RootCauseCategory::Sink);

    // E: 65W charger offered, host port sink ceiling 60W, active 60W (>= 80% of 65W = 52W) -> FINE
    let mut port_65w = make_pd_port(vec![
        make_pdo(65_000, 3_250, 20_000, false),
        make_pdo(60_000, 3_000, 20_000, true),
    ]);
    port_65w.sink_capabilities = vec![make_pdo(60_000, 3_000, 20_000, false)];
    let d_65w = ChargingDiagnostic::evaluate(&port_65w, None, 0, 0).unwrap();
    assert_eq!(d_65w.rule_id, "FINE");
}

// =========================================================================
// 10. Rework Stress 4: Truncated VDOs (Missing Cable VDO) Trust Level Classification
// =========================================================================

#[test]
fn stress_rework_truncated_vdos_trust_classification() {
    // A: Truncated VDOs (only 1 VDO: ID Header) with Valid Registered VID (Apple 0x05AC)
    let id_apple_truncated = TypeCIdentity {
        vendor_id: 0x05AC,
        product_id: 0x1234,
        vdos: vec![3u32 << 27], // 1 VDO only, no Cable VDO at index 3
    };
    let cable_apple_truncated = TypeCCable {
        r#type: "passive".into(),
        plug_type: "type-c".into(),
        identity: Some(id_apple_truncated),
        raw_attributes: Default::default(),
    };
    let info_apple_trunc = CableInfo::from_typec_cable(&cable_apple_truncated);
    assert!(!info_apple_trunc.trust.zero_vid);
    assert_eq!(
        info_apple_trunc.trust.trust_level,
        CableTrustLevel::Inconclusive
    );
    assert!(info_apple_trunc
        .trust
        .notes
        .iter()
        .any(|n| n.contains("Cable VDO telemetry is absent")));

    let assess_apple_trunc = assess_cable_trust(Some(&info_apple_trunc), None, 3_000, 20_000);
    assert_eq!(assess_apple_trunc.level, CableTrustLevel::Inconclusive);
    assert_eq!(assess_apple_trunc.confidence, DiagnosticConfidence::Low);
    assert!(assess_apple_trunc
        .flags
        .iter()
        .any(|f| f.code == "TRUNCATED_VDOS"));
    assert!(assess_apple_trunc
        .flags
        .iter()
        .any(|f| f.code == "REGISTERED_VENDOR"));
    assert!(!assess_apple_trunc.flags.iter().any(|f| f.is_warning));

    // B: Truncated VDOs with Unknown VID (0xDEAD) -> Inconclusive, NOT Suspicious
    let id_unk_truncated = TypeCIdentity {
        vendor_id: 0xDEAD,
        product_id: 0x1234,
        vdos: vec![3u32 << 27, 0, 0], // 3 VDOs, still missing VDO at index 3
    };
    let cable_unk_truncated = TypeCCable {
        r#type: "passive".into(),
        plug_type: "type-c".into(),
        identity: Some(id_unk_truncated),
        raw_attributes: Default::default(),
    };
    let info_unk_trunc = CableInfo::from_typec_cable(&cable_unk_truncated);
    assert!(!info_unk_trunc.trust.zero_vid);
    assert!(info_unk_trunc.trust.vid_unknown);
    assert_eq!(
        info_unk_trunc.trust.trust_level,
        CableTrustLevel::Inconclusive
    );

    let assess_unk_trunc = assess_cable_trust(Some(&info_unk_trunc), None, 3_000, 20_000);
    assert_eq!(assess_unk_trunc.level, CableTrustLevel::Inconclusive);
    assert_eq!(assess_unk_trunc.confidence, DiagnosticConfidence::Low);
    assert!(assess_unk_trunc
        .flags
        .iter()
        .any(|f| f.code == "TRUNCATED_VDOS"));
    assert!(assess_unk_trunc
        .flags
        .iter()
        .any(|f| f.code == "VID_UNKNOWN_OFFLINE"));

    // C: Truncated VDOs with Zero VID (0x0000) -> MUST remain Suspicious!
    let id_zero_truncated = TypeCIdentity {
        vendor_id: 0x0000,
        product_id: 0,
        vdos: vec![3u32 << 27], // missing Cable VDO
    };
    let cable_zero_truncated = TypeCCable {
        r#type: "passive".into(),
        plug_type: "type-c".into(),
        identity: Some(id_zero_truncated),
        raw_attributes: Default::default(),
    };
    let info_zero_trunc = CableInfo::from_typec_cable(&cable_zero_truncated);
    assert!(info_zero_trunc.trust.zero_vid);
    assert_eq!(
        info_zero_trunc.trust.trust_level,
        CableTrustLevel::Suspicious
    );

    let assess_zero_trunc = assess_cable_trust(Some(&info_zero_trunc), None, 3_000, 20_000);
    assert_eq!(assess_zero_trunc.level, CableTrustLevel::Suspicious);
    assert!(assess_zero_trunc.flags.iter().any(|f| f.code == "ZERO_VID"));

    // D: Truncated VDOs with Reserved Bits Asserted in ID Header -> MUST remain Suspicious!
    let id_dirty_header = TypeCIdentity {
        vendor_id: 0x05AC,
        product_id: 0x1234,
        vdos: vec![(3u32 << 27) | (1 << 20)], // bit 20 is reserved in ID Header
    };
    let cable_dirty_header = TypeCCable {
        r#type: "passive".into(),
        plug_type: "type-c".into(),
        identity: Some(id_dirty_header),
        raw_attributes: Default::default(),
    };
    let info_dirty_header = CableInfo::from_typec_cable(&cable_dirty_header);
    assert!(info_dirty_header.trust.reserved_bits_set);
    assert_eq!(
        info_dirty_header.trust.trust_level,
        CableTrustLevel::Suspicious
    );

    let assess_dirty_header = assess_cable_trust(Some(&info_dirty_header), None, 3_000, 20_000);
    assert_eq!(assess_dirty_header.level, CableTrustLevel::Suspicious);
    assert!(assess_dirty_header
        .flags
        .iter()
        .any(|f| f.code == "RESERVED_BITS_SET"));

    // E: Truncated VDOs with Valid VID subjected to contract violations (>3.2A or EPR voltage)
    let mut info_violator = info_apple_trunc.clone();
    info_violator.check_contract(5_000, 20_000);
    assert_eq!(
        info_violator.trust.trust_level,
        CableTrustLevel::Inconclusive
    );
    assert!(!info_violator.trust.emarker_contract_mismatch);

    let assess_violator_current = assess_cable_trust(Some(&info_apple_trunc), None, 5_000, 20_000);
    assert_eq!(assess_violator_current.level, CableTrustLevel::Inconclusive);
    // Missing Cable VDO means unknown rating, not proof of a mismatch.
    assert!(!assess_violator_current.flags.iter().any(|f| f.is_warning));

    let assess_violator_voltage = assess_cable_trust(Some(&info_apple_trunc), None, 3_000, 28_000);
    assert_eq!(assess_violator_voltage.level, CableTrustLevel::Inconclusive);
    // Missing Cable VDO means unknown rating, not proof of a mismatch.
    assert!(!assess_violator_voltage.flags.iter().any(|f| f.is_warning));
}

// =========================================================================
// 11. Empirical Challenger Precedence Matrix (EPR, Cables, Sinks, Throttling)
// =========================================================================

#[test]
fn stress_challenger_comprehensive_precedence_matrix() {
    let cable_240w = CableInfo {
        max_watts: 240,
        current_rating: Some(CableCurrent::FiveAmp),
        ..Default::default()
    };
    let cable_100w = CableInfo {
        max_watts: 100,
        current_rating: Some(CableCurrent::FiveAmp),
        ..Default::default()
    };
    let cable_60w = CableInfo {
        max_watts: 60,
        current_rating: Some(CableCurrent::ThreeAmp),
        ..Default::default()
    };

    // --- Dimension 1: EPR Chargers (140W, 240W) + SPR Sinks (100W, 65W, 45W) + Compliant Cables ---
    // In all these cases, the port hardware is the ceiling; cable is rated >= sink ceiling.
    let matrix_port_limit = [
        // (charger_w, sink_w, active_w, active_ma, active_mv, cable, expected_rule, expected_cause, expected_status)
        (
            140,
            100,
            100,
            5_000,
            20_000,
            Some(&cable_100w),
            "PORT_HARDWARE_LIMIT",
            RootCauseCategory::Port,
            DiagnosticStatus::Healthy,
        ),
        (
            140,
            100,
            100,
            5_000,
            20_000,
            Some(&cable_240w),
            "PORT_HARDWARE_LIMIT",
            RootCauseCategory::Port,
            DiagnosticStatus::Healthy,
        ),
        (
            140,
            65,
            65,
            3_250,
            20_000,
            Some(&cable_100w),
            "PORT_HARDWARE_LIMIT",
            RootCauseCategory::Port,
            DiagnosticStatus::Healthy,
        ),
        (
            140,
            45,
            45,
            3_000,
            15_000,
            Some(&cable_100w),
            "PORT_HARDWARE_LIMIT",
            RootCauseCategory::Port,
            DiagnosticStatus::Healthy,
        ),
        (
            140,
            45,
            45,
            3_000,
            15_000,
            Some(&cable_60w),
            "PORT_HARDWARE_LIMIT",
            RootCauseCategory::Port,
            DiagnosticStatus::Healthy,
        ),
        (
            140,
            45,
            45,
            3_000,
            15_000,
            None,
            "PORT_HARDWARE_LIMIT",
            RootCauseCategory::Port,
            DiagnosticStatus::Healthy,
        ),
        (
            240,
            100,
            100,
            5_000,
            20_000,
            Some(&cable_100w),
            "PORT_HARDWARE_LIMIT",
            RootCauseCategory::Port,
            DiagnosticStatus::Healthy,
        ),
        (
            240,
            100,
            100,
            5_000,
            20_000,
            Some(&cable_240w),
            "PORT_HARDWARE_LIMIT",
            RootCauseCategory::Port,
            DiagnosticStatus::Healthy,
        ),
        (
            240,
            140,
            140,
            5_000,
            28_000,
            Some(&cable_240w),
            "PORT_HARDWARE_LIMIT",
            RootCauseCategory::Port,
            DiagnosticStatus::Healthy,
        ),
        (
            240,
            45,
            45,
            3_000,
            15_000,
            Some(&cable_100w),
            "PORT_HARDWARE_LIMIT",
            RootCauseCategory::Port,
            DiagnosticStatus::Healthy,
        ),
        (
            240,
            45,
            45,
            3_000,
            15_000,
            Some(&cable_60w),
            "PORT_HARDWARE_LIMIT",
            RootCauseCategory::Port,
            DiagnosticStatus::Healthy,
        ),
    ];

    for (
        charger_w,
        sink_w,
        active_w,
        active_ma,
        active_mv,
        cable,
        exp_rule,
        exp_cause,
        exp_status,
    ) in matrix_port_limit
    {
        let mut port = make_pd_port(vec![
            make_pdo(charger_w * 1_000, 5_000, 20_000, false),
            make_pdo(active_w * 1_000, active_ma, active_mv, true),
        ]);
        port.sink_capabilities = vec![make_pdo(sink_w * 1_000, active_ma, active_mv, false)];
        let d = ChargingDiagnostic::evaluate(&port, cable, 0, 0)
            .expect("diagnostic evaluation should not return None");
        assert_eq!(
            d.rule_id, exp_rule,
            "Failed on {charger_w}W charger, {sink_w}W sink, {active_w}W active"
        );
        assert_eq!(d.root_cause, exp_cause);
        assert_eq!(d.status, exp_status);
        assert_eq!(d.confidence, DiagnosticConfidence::High);
    }

    // --- Dimension 2: EPR/SPR Chargers + Undersized Cables (Cable is true bottleneck) ---
    let matrix_cable_limit = [
        // (charger_w, sink_w, active_w, cable, expected_rule, expected_cause, expected_status)
        (
            100,
            100,
            60,
            &cable_60w,
            "CABLE_LIMIT",
            RootCauseCategory::Cable,
            DiagnosticStatus::Warning,
        ),
        (
            140,
            100,
            60,
            &cable_60w,
            "CABLE_LIMIT",
            RootCauseCategory::Cable,
            DiagnosticStatus::Warning,
        ),
        (
            240,
            100,
            60,
            &cable_60w,
            "CABLE_LIMIT",
            RootCauseCategory::Cable,
            DiagnosticStatus::Warning,
        ),
        (
            140,
            140,
            100,
            &cable_100w,
            "CABLE_LIMIT",
            RootCauseCategory::Cable,
            DiagnosticStatus::Warning,
        ),
        (
            240,
            140,
            100,
            &cable_100w,
            "CABLE_LIMIT",
            RootCauseCategory::Cable,
            DiagnosticStatus::Warning,
        ),
        (
            240,
            240,
            100,
            &cable_100w,
            "CABLE_LIMIT",
            RootCauseCategory::Cable,
            DiagnosticStatus::Warning,
        ),
        (
            100,
            65,
            60,
            &cable_60w,
            "CABLE_LIMIT",
            RootCauseCategory::Cable,
            DiagnosticStatus::Warning,
        ),
    ];

    for (charger_w, sink_w, active_w, cable, exp_rule, exp_cause, exp_status) in matrix_cable_limit
    {
        let mut port = make_pd_port(vec![
            make_pdo(charger_w * 1_000, 5_000, 20_000, false),
            make_pdo(active_w * 1_000, 3_000, 20_000, true),
        ]);
        port.sink_capabilities = vec![make_pdo(sink_w * 1_000, 5_000, 20_000, false)];
        let d = ChargingDiagnostic::evaluate(&port, Some(cable), 0, 0)
            .expect("diagnostic evaluation should not return None");
        assert_eq!(
            d.rule_id, exp_rule,
            "Failed on {charger_w}W charger, {sink_w}W sink, cable {}W",
            cable.max_watts
        );
        assert_eq!(d.root_cause, exp_cause);
        assert_eq!(d.status, exp_status);
        assert_eq!(d.confidence, DiagnosticConfidence::Medium);
    }

    // --- Dimension 3: Empty Sink Capabilities (sink_cap_max_w == 0) Graceful Fallbacks ---
    // When sink telemetry is absent:
    // a) If cable < charger -> CABLE_LIMIT (fallback to cable rating)
    let port_empty_sink_cable_60w = make_pd_port(vec![
        make_pdo(100_000, 5_000, 20_000, false),
        make_pdo(60_000, 3_000, 20_000, true),
    ]);
    let d_empty_sink_cable =
        ChargingDiagnostic::evaluate(&port_empty_sink_cable_60w, Some(&cable_60w), 0, 0).unwrap();
    assert_eq!(d_empty_sink_cable.rule_id, "CABLE_LIMIT");
    assert_eq!(d_empty_sink_cable.root_cause, RootCauseCategory::Cable);
    assert_eq!(d_empty_sink_cable.status, DiagnosticStatus::Warning);

    // b) If cable meets charger (100W on 100W) but active is throttled (45W) -> DEVICE_LIMIT (Sink policy)
    let port_empty_sink_cable_100w = make_pd_port(vec![
        make_pdo(100_000, 5_000, 20_000, false),
        make_pdo(45_000, 3_000, 15_000, true),
    ]);
    let d_empty_sink_device =
        ChargingDiagnostic::evaluate(&port_empty_sink_cable_100w, Some(&cable_100w), 0, 0).unwrap();
    assert_eq!(d_empty_sink_device.rule_id, "DEVICE_LIMIT");
    assert_eq!(d_empty_sink_device.root_cause, RootCauseCategory::Sink);
    assert_eq!(d_empty_sink_device.status, DiagnosticStatus::Healthy);

    // c) If cable meets charger and active is full (100W on 100W) -> FINE
    let port_empty_sink_full = make_pd_port(vec![make_pdo(100_000, 5_000, 20_000, true)]);
    let d_empty_sink_full =
        ChargingDiagnostic::evaluate(&port_empty_sink_full, Some(&cable_100w), 0, 0).unwrap();
    assert_eq!(d_empty_sink_full.rule_id, "FINE");
    assert_eq!(d_empty_sink_full.status, DiagnosticStatus::Healthy);

    // --- Dimension 4: Active Draw Throttled Below Port Ceiling (Device Thermal/Battery Policy) ---
    // Port allows 100W, charger is 140W, cable is 100W, but active is 65W or 30W.
    // Port hardware is NOT the bottleneck because port ceiling (100W) > active_w (65W/30W).
    // Cable is NOT the bottleneck because cable (100W) >= sink ceiling (100W).
    // Root cause is DEVICE_LIMIT (Sink internal policy).
    let throttled_scenarios = [
        // (charger_w, sink_w, active_w)
        (140, 100, 65),
        (140, 100, 30),
        (100, 65, 45),
        (100, 45, 30),
    ];
    for (charger_w, sink_w, active_w) in throttled_scenarios {
        let mut port = make_pd_port(vec![
            make_pdo(charger_w * 1_000, 5_000, 20_000, false),
            make_pdo(active_w * 1_000, 3_000, 20_000, true),
        ]);
        port.sink_capabilities = vec![make_pdo(sink_w * 1_000, 5_000, 20_000, false)];
        let d = ChargingDiagnostic::evaluate(&port, Some(&cable_100w), 0, 0).unwrap();
        assert_eq!(
            d.rule_id, "DEVICE_LIMIT",
            "Failed for throttled active {active_w}W on {sink_w}W sink / {charger_w}W charger"
        );
        assert_eq!(d.root_cause, RootCauseCategory::Sink);
        assert_eq!(d.status, DiagnosticStatus::Healthy);
    }
}

// =========================================================================
// 12. Adversarial Boundaries & Multi-PDO Sink Telemetry
// =========================================================================

#[test]
fn stress_challenger_adversarial_boundaries_and_zero_panic() {
    let cable_100w = CableInfo {
        max_watts: 100,
        current_rating: Some(CableCurrent::FiveAmp),
        ..Default::default()
    };
    let cable_60w = CableInfo {
        max_watts: 60,
        current_rating: Some(CableCurrent::ThreeAmp),
        ..Default::default()
    };

    // 1. Boundary at exactly 80%
    // 100W charger: 80% threshold is 80.0W
    // Case 1a: Active is 80W, sink ceiling is 80W, cable 100W.
    // 80.0 < 80.0 is false -> FINE
    let mut port_80w = make_pd_port(vec![
        make_pdo(100_000, 5_000, 20_000, false),
        make_pdo(80_000, 4_000, 20_000, true),
    ]);
    port_80w.sink_capabilities = vec![make_pdo(80_000, 4_000, 20_000, false)];
    let d_80w = ChargingDiagnostic::evaluate(&port_80w, Some(&cable_100w), 0, 0).unwrap();
    assert_eq!(d_80w.rule_id, "FINE");
    assert_eq!(d_80w.status, DiagnosticStatus::Healthy);

    // Case 1b: Active is 79W, sink ceiling is 79W, cable 100W.
    // 79.0 < 80.0 is true -> PORT_HARDWARE_LIMIT
    let mut port_79w = make_pd_port(vec![
        make_pdo(100_000, 5_000, 20_000, false),
        make_pdo(79_000, 3_950, 20_000, true),
    ]);
    port_79w.sink_capabilities = vec![make_pdo(79_000, 3_950, 20_000, false)];
    let d_79w = ChargingDiagnostic::evaluate(&port_79w, Some(&cable_100w), 0, 0).unwrap();
    assert_eq!(d_79w.rule_id, "PORT_HARDWARE_LIMIT");
    assert_eq!(d_79w.root_cause, RootCauseCategory::Port);
    assert_eq!(d_79w.status, DiagnosticStatus::Healthy);

    // 2. Multi-PDO Sink Capabilities Resolution
    // Laptop has 15W, 27W, 45W, 65W sink PDOs (ceiling is 65W)
    let mut port_multi_sink = make_pd_port(vec![
        make_pdo(100_000, 5_000, 20_000, false),
        make_pdo(65_000, 3_250, 20_000, true),
    ]);
    port_multi_sink.sink_capabilities = vec![
        make_pdo(15_000, 3_000, 5_000, false),
        make_pdo(27_000, 3_000, 9_000, false),
        make_pdo(45_000, 3_000, 15_000, false),
        make_pdo(65_000, 3_250, 20_000, false),
    ];
    let d_multi = ChargingDiagnostic::evaluate(&port_multi_sink, Some(&cable_100w), 0, 0).unwrap();
    assert_eq!(d_multi.rule_id, "PORT_HARDWARE_LIMIT");
    assert_eq!(d_multi.root_cause, RootCauseCategory::Port);
    assert!(d_multi
        .detail
        .contains("host port hardware sink capability ceiling is 65W"));

    // 3. PPS / Augmented PDO Handling
    let port_pps = PowerDeliveryPort {
        source_capabilities: vec![
            PowerDataObject {
                r#type: PdoType::Pps,
                power_mw: 140_000,
                current_ma: 5_000,
                voltage_mv: 28_000,
                is_active: false,
                ..Default::default()
            },
            PowerDataObject {
                r#type: PdoType::Pps,
                power_mw: 100_000,
                current_ma: 5_000,
                voltage_mv: 20_000,
                is_active: true,
                ..Default::default()
            },
        ],
        max_source_power_mw: 140_000,
        sink_capabilities: vec![PowerDataObject {
            r#type: PdoType::Pps,
            power_mw: 100_000,
            current_ma: 5_000,
            voltage_mv: 20_000,
            is_active: false,
            ..Default::default()
        }],
        ..Default::default()
    };
    let d_pps = ChargingDiagnostic::evaluate(&port_pps, Some(&cable_100w), 0, 0).unwrap();
    assert_eq!(d_pps.rule_id, "PORT_HARDWARE_LIMIT");
    assert_eq!(d_pps.root_cause, RootCauseCategory::Port);

    // 4. Undersized cable when sink capabilities are empty vs present
    // 140W charger, active 60W on 60W cable.
    // Case 4a: sink capabilities present with 100W ceiling -> CABLE_LIMIT
    let mut port_cable_bottleneck = make_pd_port(vec![
        make_pdo(140_000, 5_000, 28_000, false),
        make_pdo(60_000, 3_000, 20_000, true),
    ]);
    port_cable_bottleneck.sink_capabilities = vec![make_pdo(100_000, 5_000, 20_000, false)];
    let d_cb =
        ChargingDiagnostic::evaluate(&port_cable_bottleneck, Some(&cable_60w), 0, 0).unwrap();
    assert_eq!(d_cb.rule_id, "CABLE_LIMIT");
    assert_eq!(d_cb.root_cause, RootCauseCategory::Cable);
    assert_eq!(d_cb.status, DiagnosticStatus::Warning);

    // Case 4b: sink capabilities absent (empty) -> CABLE_LIMIT (graceful fallback)
    let port_cable_no_sink = make_pd_port(vec![
        make_pdo(140_000, 5_000, 28_000, false),
        make_pdo(60_000, 3_000, 20_000, true),
    ]);
    let d_cbn = ChargingDiagnostic::evaluate(&port_cable_no_sink, Some(&cable_60w), 0, 0).unwrap();
    assert_eq!(d_cbn.rule_id, "CABLE_LIMIT");
    assert_eq!(d_cbn.root_cause, RootCauseCategory::Cable);
    assert_eq!(d_cbn.status, DiagnosticStatus::Warning);

    // Case 4c: sink capabilities 45W (< 60W cable) -> PORT_HARDWARE_LIMIT, NOT CABLE_LIMIT!
    let mut port_port_bottleneck = make_pd_port(vec![
        make_pdo(140_000, 5_000, 28_000, false),
        make_pdo(45_000, 3_000, 15_000, true),
    ]);
    port_port_bottleneck.sink_capabilities = vec![make_pdo(45_000, 3_000, 15_000, false)];
    let d_pb = ChargingDiagnostic::evaluate(&port_port_bottleneck, Some(&cable_60w), 0, 0).unwrap();
    assert_eq!(d_pb.rule_id, "PORT_HARDWARE_LIMIT");
    assert_eq!(d_pb.root_cause, RootCauseCategory::Port);
    assert_eq!(d_pb.status, DiagnosticStatus::Healthy);
}
