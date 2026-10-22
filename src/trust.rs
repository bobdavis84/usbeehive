//! USB-C cable authenticity and trust diagnostics.
//!
//! Provides confidence-aware heuristics that evaluate e-marker descriptors,
//! vendor identities, VDO reserved bit fields, and active operational contracts
//! to categorize cables into [`CableTrustLevel`] tiers:
//!
//! - [`CableTrustLevel::KnownGood`] — Available identity and checked descriptor
//!   fields are consistent; this is not authenticity or physical certification.
//! - [`CableTrustLevel::LikelyConstrained`] — A positively reported lower cable
//!   rating limits the available charger capability; not a guarantee of safety.
//! - [`CableTrustLevel::Inconclusive`] — Insufficient controller telemetry (e.g. UCSI
//!   controller did not expose cable identity).
//! - [`CableTrustLevel::Suspicious`] — Anomalous indicators: vendor ID 0x0000,
//!   reserved VDO bits set, or operational contract exceeding cable ratings.
//!
//! # Hardware & Telemetry Limitations
//!
//! Software diagnostics running over Linux sysfs inspect logical descriptors
//! and controller telemetry only. A pure software stack cannot measure physical
//! wire gauge, shielding effectiveness, contact resistance, or counterfeit
//! cloned microcode. Definitive certification requires calibrated laboratory
//! test instruments.

use serde::{Deserialize, Serialize};

use crate::cable::CableInfo;
use crate::diagnostic::DiagnosticConfidence;
use crate::pd::CableCurrent;
use crate::power::PowerDeliveryPort;

/// Standard limitations note accompanying all trust assessments.
pub const CABLE_TRUST_LIMITATIONS_NOTE: &str =
    "Software telemetry cannot verify physical copper gauge, shielding integrity, \
     contact resistance, or counterfeit cloned e-markers. Physical inspection and \
     certified laboratory test equipment are required for definitive verification.";

/// Discrete trust classification level for a USB-C cable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CableTrustLevel {
    /// Available e-marker descriptors and reported operation are consistent, not certified.
    KnownGood,
    /// Positively reported lower cable rating; not a guarantee of physical safety.
    LikelyConstrained,
    /// Insufficient telemetry (e.g. UCSI controller omitted cable identity node).
    #[default]
    Inconclusive,
    /// Suspicious indicators: zero VID, reserved bits set, or operational contract mismatch.
    Suspicious,
}

impl CableTrustLevel {
    /// Short human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            CableTrustLevel::KnownGood => "Known Good",
            CableTrustLevel::LikelyConstrained => "Likely Constrained",
            CableTrustLevel::Inconclusive => "Inconclusive",
            CableTrustLevel::Suspicious => "Suspicious",
        }
    }

    /// `true` when the trust level indicates a serious anomaly.
    pub fn is_suspicious(self) -> bool {
        matches!(self, CableTrustLevel::Suspicious)
    }

    /// `true` when available descriptors are consistent; not physical certification.
    pub fn is_known_good(self) -> bool {
        matches!(self, CableTrustLevel::KnownGood)
    }
}

/// Individual signal or flag detected during trust analysis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustSignal {
    /// Machine-readable signal code (e.g. `"ZERO_VID"`, `"RESERVED_BITS_SET"`).
    pub code: &'static str,
    /// Plain-English explanation of what was observed.
    pub description: String,
    /// `true` if this signal represents an anomaly or risk.
    pub is_warning: bool,
}

/// Alias for [`TrustSignal`] per project contract specifications.
pub type TrustFlag = TrustSignal;

/// Comprehensive result of cable trust and authenticity analysis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustAssessment {
    /// Overall trust level.
    pub level: CableTrustLevel,
    /// Confidence level in this assessment.
    pub confidence: DiagnosticConfidence,
    /// Specific positive and negative signals that led to this verdict.
    pub flags: Vec<TrustSignal>,
    /// One-line headline summary.
    pub summary: String,
    /// Explanatory details and context.
    pub explanation: String,
    /// Disclaimers and physical limitations note.
    pub limitations_note: &'static str,
}

/// Assess the authenticity and trust tier of a cable, given optional cable telemetry
/// and operational Power Delivery port state.
///
/// `active_ma` is reported operating current in mA (not a source PDO ceiling), `0` if unknown.
/// `active_mv` is the operating voltage in mV (from the active PDO or power supply), `0` if unknown.
pub fn assess_cable_trust(
    cable: Option<&CableInfo>,
    pd_port: Option<&PowerDeliveryPort>,
    active_ma: u32,
    active_mv: u32,
) -> TrustAssessment {
    let mut flags = Vec::new();
    let mut level = CableTrustLevel::KnownGood;
    let mut confidence = DiagnosticConfidence::High;
    let mut explanation_points = Vec::new();

    // Source PDO current is a charger ceiling, never an operating-current fallback.
    // Fixed-supply selected PDO voltage is usable; PPS/variable voltage is not.
    let pdo_mv = pd_port
        .and_then(|pd| pd.source_capabilities.iter().find(|p| p.is_active))
        .filter(|p| p.r#type == crate::power::PdoType::FixedSupply)
        .map(|p| p.voltage_mv)
        .unwrap_or(0);
    let effective_ma = active_ma;
    let effective_mv = if active_mv > 0 { active_mv } else { pdo_mv };

    match cable {
        None => {
            level = CableTrustLevel::Inconclusive;
            confidence = DiagnosticConfidence::Low;
            flags.push(TrustSignal {
                code: "TELEMETRY_UNAVAILABLE",
                description:
                    "Cable identity telemetry is unavailable; its ratings cannot be verified."
                        .into(),
                is_warning: false,
            });
        }
        Some(c) => {
            // Case 2: Cable telemetry is present (e-marked or non-e-marked TypeCCable snapshot).

            // Check Rule 1: Zero Vendor ID (counterfeit / uncertified)
            if c.trust.zero_vid {
                level = CableTrustLevel::Suspicious;
                flags.push(TrustSignal {
                    code: "ZERO_VID",
                    description: "E-marker reports Vendor ID 0x0000.".into(),
                    is_warning: true,
                });
                explanation_points.push(
                    "USB-IF requires every certified e-marker manufacturer to register a valid \
                     16-bit Vendor ID. 0x0000 strongly suggests counterfeit or uncertified microcode."
                        .into(),
                );
            }

            // Check Rule 2: Dirty / Reserved VDO bits
            if c.trust.reserved_bits_set {
                level = CableTrustLevel::Suspicious;
                flags.push(TrustSignal {
                    code: "RESERVED_BITS_SET",
                    description: "E-marker sets reserved VDO bits.".into(),
                    is_warning: true,
                });
                explanation_points.push(
                    "Bits reserved by USB-PD specifications are asserted, indicating non-compliant \
                     firmware or modified reference designs."
                        .into(),
                );
            }

            // Check Rule 3: Current Mismatch
            if effective_ma > 3_200 {
                let is_3a = c.current_rating == Some(CableCurrent::ThreeAmp)
                    || (c.max_watts > 0
                        && c.max_watts <= 60
                        && c.current_rating != Some(CableCurrent::FiveAmp));
                if is_3a {
                    level = CableTrustLevel::Suspicious;
                    flags.push(TrustSignal {
                        code: "CONTRACT_EXCEEDS_RATING",
                        description: format!(
                            "Active contract negotiated {:.1}A on a cable rated for 3.0A (60W).",
                            effective_ma as f64 / 1000.0
                        ),
                        is_warning: true,
                    });
                    explanation_points.push(
                        "Negotiated power contract exceeds the physical current capacity declared by \
                         the cable's own e-marker."
                            .into(),
                    );
                }
            }

            // Check Rule 4: EPR Voltage on Non-EPR / Non-E-Marked Cable (Finding 7)
            let is_non_epr = c.max_watts > 0 && c.max_watts <= 100;
            if is_non_epr && effective_mv > 21_000 {
                level = CableTrustLevel::Suspicious;
                flags.push(TrustSignal {
                    code: "EPR_ON_SPR_CABLE",
                    description: format!(
                        "Active contract operating at {:.1}V (>20V) on a cable not rated for EPR (SPR 20V max).",
                        effective_mv as f64 / 1000.0
                    ),
                    is_warning: true,
                });
                explanation_points.push(
                    "Extended Power Range (EPR) voltages (>20V) require EPR-rated 240W cables with \
                     increased dielectric clearance."
                        .into(),
                );
            }

            // If no negative signals were found:
            if level != CableTrustLevel::Suspicious {
                if c.current_rating.is_none() {
                    if c.vendor_id == 0 {
                        level = CableTrustLevel::Inconclusive;
                        confidence = DiagnosticConfidence::Low;
                        flags.push(TrustSignal {
                            code: "TELEMETRY_UNAVAILABLE",
                            description: "Cable identity telemetry is missing; passive type does not establish its rating.".into(),
                            is_warning: false,
                        });
                    } else {
                        // Truncated e-marker VDOs (Finding 4)
                        level = CableTrustLevel::Inconclusive;
                        confidence = DiagnosticConfidence::Low;
                        if c.trust.vid_unknown {
                            flags.push(TrustSignal {
                                code: "VID_UNKNOWN_OFFLINE",
                                description: format!(
                                    "Vendor ID 0x{:04X} is not listed in local database.",
                                    c.vendor_id
                                ),
                                is_warning: false,
                            });
                        } else if c.vendor_id != 0 {
                            flags.push(TrustSignal {
                                code: "REGISTERED_VENDOR",
                                description: format!(
                                    "Vendor: {} (0x{:04X})",
                                    c.vendor_name, c.vendor_id
                                ),
                                is_warning: false,
                            });
                        }
                        flags.push(TrustSignal {
                            code: "TRUNCATED_VDOS",
                            description: "Cable e-marker telemetry is incomplete (missing Cable VDO).".into(),
                            is_warning: false,
                        });
                        explanation_points.push(
                            "E-marker response omitted the Cable VDO (VDO index 3); cable power and speed capabilities are inconclusive.".into(),
                        );
                    }
                } else {
                    // Full e-marker present
                    if c.trust.vid_unknown {
                        flags.push(TrustSignal {
                            code: "VID_UNKNOWN_OFFLINE",
                            description: format!(
                                "Vendor ID 0x{:04X} is not listed in local database.",
                                c.vendor_id
                            ),
                            is_warning: false,
                        });
                        explanation_points.push(format!(
                            "Vendor ID 0x{:04X} is not in the offline vendor database; this may be a \
                             recently registered legitimate manufacturer.",
                            c.vendor_id
                        ));
                    } else if !c.trust.zero_vid {
                        flags.push(TrustSignal {
                            code: "REGISTERED_VENDOR",
                            description: format!(
                                "Vendor: {} (0x{:04X})",
                                c.vendor_name, c.vendor_id
                            ),
                            is_warning: false,
                        });
                    }

                    level = if c.current_rating == Some(CableCurrent::ThreeAmp)
                        && pd_port.is_some_and(|p| {
                            c.max_watts > 0 && p.max_source_power_mw / 1000 > c.max_watts
                        }) {
                        CableTrustLevel::LikelyConstrained
                    } else {
                        CableTrustLevel::KnownGood
                    };
                    flags.push(TrustSignal {
                        code: "CERTIFIED_EMARKER",
                        description:
                            "Available e-marker declarations and checked bitfields are consistent."
                                .into(),
                        is_warning: false,
                    });
                }
            }
        }
    }

    let summary: String = match level {
        CableTrustLevel::KnownGood => {
            "Available cable identity and rating telemetry is consistent.".into()
        }
        CableTrustLevel::LikelyConstrained => "Cable reports a lower power/current rating.".into(),
        CableTrustLevel::Inconclusive => {
            "Cable trust inconclusive due to missing controller telemetry.".into()
        }
        CableTrustLevel::Suspicious => {
            "Cable exhibits suspicious anomalies or contract mismatches.".into()
        }
    };

    let explanation = if explanation_points.is_empty() {
        summary.clone()
    } else {
        explanation_points.join(" ")
    };

    TrustAssessment {
        level,
        confidence,
        flags,
        summary,
        explanation,
        limitations_note: CABLE_TRUST_LIMITATIONS_NOTE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pd::CableSpeed;
    use crate::vendor;

    fn make_cable(
        vid: u16,
        watts: u32,
        current: Option<CableCurrent>,
        zero_vid: bool,
        reserved_bits: bool,
    ) -> CableInfo {
        let mut c = CableInfo {
            vendor_id: vid,
            vendor_name: vendor::lookup(vid),
            max_watts: watts,
            current_rating: current,
            speed: Some(CableSpeed::Usb32Gen2),
            ..Default::default()
        };
        c.trust.zero_vid = zero_vid;
        c.trust.vid_unknown = vid != 0 && vendor::is_hex_fallback(&c.vendor_name);
        c.trust.reserved_bits_set = reserved_bits;
        if zero_vid || reserved_bits {
            c.trust.trust_level = CableTrustLevel::Suspicious;
        } else {
            c.trust.trust_level = CableTrustLevel::KnownGood;
        }
        c
    }

    #[test]
    fn clean_certified_cable_is_known_good() {
        let cable = make_cable(0x05AC, 100, Some(CableCurrent::FiveAmp), false, false);
        let assessment = assess_cable_trust(Some(&cable), None, 5_000, 20_000);
        assert_eq!(assessment.level, CableTrustLevel::KnownGood);
        assert_eq!(assessment.confidence, DiagnosticConfidence::High);
        assert!(!assessment.flags.iter().any(|f| f.is_warning));
    }

    #[test]
    fn unknown_vid_is_known_good_with_advisory_note() {
        // 0xDEAD is not in the vendor database, but has clean bits.
        // Must NOT be classified as Suspicious!
        let cable = make_cable(0xDEAD, 100, Some(CableCurrent::FiveAmp), false, false);
        let assessment = assess_cable_trust(Some(&cable), None, 5_000, 20_000);
        assert_eq!(assessment.level, CableTrustLevel::KnownGood);
        assert!(assessment
            .flags
            .iter()
            .any(|f| f.code == "VID_UNKNOWN_OFFLINE"));
        assert!(assessment.explanation.contains("0xDEAD"));
    }

    #[test]
    fn zero_vid_is_flagged_suspicious() {
        let cable = make_cable(0x0000, 100, Some(CableCurrent::FiveAmp), true, false);
        let assessment = assess_cable_trust(Some(&cable), None, 5_000, 20_000);
        assert_eq!(assessment.level, CableTrustLevel::Suspicious);
        assert!(assessment
            .flags
            .iter()
            .any(|f| f.code == "ZERO_VID" && f.is_warning));
    }

    #[test]
    fn reserved_bits_flagged_suspicious() {
        let cable = make_cable(0x05AC, 100, Some(CableCurrent::FiveAmp), false, true);
        let assessment = assess_cable_trust(Some(&cable), None, 5_000, 20_000);
        assert_eq!(assessment.level, CableTrustLevel::Suspicious);
        assert!(assessment
            .flags
            .iter()
            .any(|f| f.code == "RESERVED_BITS_SET" && f.is_warning));
    }

    #[test]
    fn contract_3a_cable_with_5a_draw_is_suspicious() {
        let cable = make_cable(0x05AC, 60, Some(CableCurrent::ThreeAmp), false, false);
        let assessment = assess_cable_trust(Some(&cable), None, 5_000, 20_000);
        assert_eq!(assessment.level, CableTrustLevel::Suspicious);
        assert!(assessment
            .flags
            .iter()
            .any(|f| f.code == "CONTRACT_EXCEEDS_RATING"));
    }

    #[test]
    fn epr_voltage_on_spr_cable_is_suspicious() {
        // Cable is 100W (SPR), but active contract is 28V (EPR)
        let cable = make_cable(0x05AC, 100, Some(CableCurrent::FiveAmp), false, false);
        let assessment = assess_cable_trust(Some(&cable), None, 5_000, 28_000);
        assert_eq!(assessment.level, CableTrustLevel::Suspicious);
        assert!(assessment
            .flags
            .iter()
            .any(|f| f.code == "EPR_ON_SPR_CABLE"));
    }

    #[test]
    fn passive_without_emarker_is_inconclusive() {
        let cable = CableInfo {
            is_passive: true,
            current_rating: None,
            ..Default::default()
        };
        let assessment = assess_cable_trust(Some(&cable), None, 3_000, 20_000);
        assert_eq!(assessment.level, CableTrustLevel::Inconclusive);
        // Missing identity cannot establish absence of an e-marker or a rating.
        assert!(!assessment.flags.iter().any(|f| f.is_warning));
    }

    #[test]
    fn missing_cable_telemetry_is_inconclusive() {
        let assessment = assess_cable_trust(None, None, 3_000, 20_000);
        assert_eq!(assessment.level, CableTrustLevel::Inconclusive);
        assert_eq!(assessment.confidence, DiagnosticConfidence::Low);
    }

    #[test]
    fn missing_cable_telemetry_with_high_current_is_inconclusive() {
        let assessment = assess_cable_trust(None, None, 5_000, 20_000);
        assert_eq!(assessment.level, CableTrustLevel::Inconclusive);
        // Missing identity cannot establish absence of an e-marker or a rating.
        assert!(!assessment.flags.iter().any(|f| f.is_warning));
    }

    use crate::cable::CableTrust;

    fn make_pdo(mw: u32, ma: u32, mv: u32, active: bool) -> crate::power::PowerDataObject {
        crate::power::PowerDataObject {
            r#type: crate::power::PdoType::FixedSupply,
            voltage_mv: mv,
            max_voltage_mv: mv,
            current_ma: ma,
            power_mw: mw,
            is_active: active,
            ..Default::default()
        }
    }

    fn make_pd_port(source_caps: Vec<crate::power::PowerDataObject>) -> PowerDeliveryPort {
        let max = source_caps.iter().map(|p| p.power_mw).max().unwrap_or(0);
        PowerDeliveryPort {
            name: "pd0".into(),
            source_capabilities: source_caps,
            max_source_power_mw: max,
            ..Default::default()
        }
    }

    #[test]
    fn non_passive_without_emarker_is_inconclusive() {
        let cable = CableInfo {
            is_active: true,
            is_passive: false,
            cable_type: "active".into(),
            current_rating: None,
            ..Default::default()
        };
        let assessment = assess_cable_trust(Some(&cable), None, 3_000, 20_000);
        assert_eq!(assessment.level, CableTrustLevel::Inconclusive);
        assert_eq!(assessment.confidence, DiagnosticConfidence::Low);
        assert!(assessment
            .flags
            .iter()
            .any(|f| f.code == "TELEMETRY_UNAVAILABLE"));
    }

    #[test]
    fn truncated_vdos_in_assess_cable_trust_is_inconclusive() {
        let cable = CableInfo {
            vendor_id: 0x05AC,
            vendor_name: "Apple".into(),
            current_rating: None,
            trust: CableTrust {
                trust_level: CableTrustLevel::Inconclusive,
                ..Default::default()
            },
            ..Default::default()
        };
        let assessment = assess_cable_trust(Some(&cable), None, 3_000, 20_000);
        assert_eq!(assessment.level, CableTrustLevel::Inconclusive);
        assert_eq!(assessment.confidence, DiagnosticConfidence::Low);
        assert!(assessment.flags.iter().any(|f| f.code == "TRUNCATED_VDOS"));
    }

    #[test]
    fn epr_voltage_on_non_emarked_cable_is_inconclusive() {
        let cable = CableInfo {
            is_passive: true,
            current_rating: None,
            max_watts: 0,
            ..Default::default()
        };
        let assessment = assess_cable_trust(Some(&cable), None, 3_000, 28_000);
        assert_eq!(assessment.level, CableTrustLevel::Inconclusive);
        // Missing identity cannot establish absence of an e-marker or a rating.
        assert!(!assessment.flags.iter().any(|f| f.is_warning));
    }

    #[test]
    fn uncoupled_pdo_resolution_checks_epr_voltage() {
        let port = make_pd_port(vec![make_pdo(140_000, 5_000, 28_000, true)]);
        let cable = make_cable(0x05AC, 100, Some(CableCurrent::FiveAmp), false, false);
        // Caller passes only current (5A), active_mv is 0.
        // Voltage must be read from active PDO (28V EPR), flagging EPR_ON_SPR_CABLE!
        let assessment = assess_cable_trust(Some(&cable), Some(&port), 5_000, 0);
        assert_eq!(assessment.level, CableTrustLevel::Suspicious);
        assert!(assessment
            .flags
            .iter()
            .any(|f| f.code == "EPR_ON_SPR_CABLE"));
    }
}
