//! Evidence-aware safety and legacy JSON regressions.
use usbeehive::{
    cable::CableInfo,
    diagnostic::{ChargingDiagnostic, DiagnosticStatus},
    power::{PdoType, PowerDataObject, PowerDeliveryPort},
    trust::{assess_cable_trust, CableTrustLevel},
};

fn port() -> PowerDeliveryPort {
    PowerDeliveryPort {
        source_capabilities: vec![PowerDataObject {
            r#type: PdoType::FixedSupply,
            voltage_mv: 20_000,
            current_ma: 5_000,
            power_mw: 100_000,
            is_active: true,
            ..Default::default()
        }],
        max_source_power_mw: 100_000,
        ..Default::default()
    }
}

#[test]
fn positive_lower_rating_is_constrained_not_certified() {
    let c = CableInfo {
        current_rating: Some(usbeehive::pd::CableCurrent::ThreeAmp),
        max_watts: 60,
        vendor_id: 0x05ac,
        ..Default::default()
    };
    let t = assess_cable_trust(Some(&c), Some(&port()), 0, 20_000);
    assert_eq!(t.level, CableTrustLevel::LikelyConstrained);
    assert!(!t.flags.iter().any(|f| f.is_warning));
    let d = ChargingDiagnostic::evaluate(&port(), Some(&c), 0, 0).unwrap();
    assert_ne!(d.status, DiagnosticStatus::Critical);
    assert_eq!(
        assess_cable_trust(Some(&c), Some(&port()), 4_000, 20_000).level,
        CableTrustLevel::Suspicious
    );
    assert_eq!(
        ChargingDiagnostic::evaluate(&port(), Some(&c), 80_000, 4_000)
            .unwrap()
            .status,
        DiagnosticStatus::Critical
    );
}

#[test]
fn unknown_rule_ids_use_bounded_static_fallback() {
    for id in ["future-rule-1", "future-rule-2"] {
        let d: ChargingDiagnostic =
            serde_json::from_value(serde_json::json!({"bottleneck":"Fine", "rule_id":id})).unwrap();
        assert_eq!(d.rule_id, "UNKNOWN");
    }
    for rule in [
        "FINE",
        "CONTRACT_OUT_OF_SPEC",
        "PORT_HARDWARE_LIMIT",
        "CABLE_LIMIT",
        "CABLE_NO_EMARKER",
        "CHARGER_UNDERPOWERED",
        "DEVICE_LIMIT",
        "SINK_LIMIT",
    ] {
        let d: ChargingDiagnostic =
            serde_json::from_value(serde_json::json!({"bottleneck":"Fine", "rule_id":rule}))
                .unwrap();
        assert_eq!(d.rule_id, rule);
    }
}

#[test]
fn legacy_trust_defaults_infer_old_anomalies() {
    for (json, level) in [
        (
            r#"{"zero_vid":true,"vid_unknown":false,"reserved_bits_set":false}"#,
            CableTrustLevel::Suspicious,
        ),
        (
            r#"{"zero_vid":false,"vid_unknown":true,"reserved_bits_set":false}"#,
            CableTrustLevel::Inconclusive,
        ),
    ] {
        let t: usbeehive::cable::CableTrust = serde_json::from_str(json).unwrap();
        assert_eq!(t.trust_level, level);
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(
            serde_json::from_str::<usbeehive::cable::CableTrust>(&json).unwrap(),
            t
        );
    }
}

#[test]
fn legacy_diagnostic_infers_warning_fields() {
    let d: ChargingDiagnostic =
        serde_json::from_str(r#"{"bottleneck":"CableLimit","is_warning":true}"#).unwrap();
    assert_eq!(d.status, DiagnosticStatus::Warning);
    assert_eq!(
        d.root_cause,
        usbeehive::diagnostic::RootCauseCategory::Cable
    );
    assert_eq!(
        d.confidence,
        usbeehive::diagnostic::DiagnosticConfidence::Low
    );
    assert_eq!(d.rule_id, "CABLE_LIMIT");
    let restored: ChargingDiagnostic =
        serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
    assert_eq!(restored, d);
}

#[test]
fn missing_identity_is_not_evidence_of_unsafe_current() {
    for cable in [
        None,
        Some(CableInfo {
            is_passive: true,
            ..Default::default()
        }),
    ] {
        for current in [0, 5_000] {
            let d = ChargingDiagnostic::evaluate(&port(), cable.as_ref(), 0, current).unwrap();
            assert_ne!(d.status, DiagnosticStatus::Critical);
            let t = assess_cable_trust(cable.as_ref(), Some(&port()), current, 28_000);
            assert_eq!(t.level, CableTrustLevel::Inconclusive);
            assert!(!t.flags.iter().any(|f| f.is_warning));
        }
    }
    let mut c = CableInfo::default();
    c.check_contract(5_000, 28_000);
    assert!(!c.trust.is_suspicious());
}
