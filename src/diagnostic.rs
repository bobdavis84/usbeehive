//! Charging-bottleneck diagnostic.
//!
//! Looks at a [`PowerDeliveryPort`] (charger advertisement + active contract)
//! plus the optional cable view, and produces a [`ChargingDiagnostic`] that
//! identifies the limiting factor.

use serde::{Deserialize, Serialize};

use crate::cable::CableInfo;
use crate::power::PowerDeliveryPort;

/// Overall health assessment of charging/power state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DiagnosticStatus {
    /// Optimal operation; negotiation matches charger capabilities or sink policy.
    #[default]
    Healthy,
    /// Suboptimal throughput (e.g. 60W cable with 100W charger, missing e-marker, underpowered charger).
    Warning,
    /// Severe condition, hazard, or protocol violation (e.g. non-compliant contract, >3A on non-e-marked cable).
    Critical,
}

/// Categorical subsystem responsible for the observed limitation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RootCauseCategory {
    /// Cable rating, absence of e-marker, or physical cable constraint.
    Cable,
    /// Charger advertisement is undersized for host needs or lower than device capability.
    Charger,
    /// Device internal policy: battery charge threshold, thermal throttling, or sleep draw.
    Sink,
    /// Host port controller hardware limitation or non-PD port mode.
    Port,
    /// UCSI firmware anomaly, contract negotiation bug, or safety violation.
    Firmware,
    /// No limitation; charging is optimal.
    #[default]
    None,
}

/// Confidence level of the diagnostic assessment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DiagnosticConfidence {
    /// Supported by unambiguous direct descriptors (e.g. e-marker VDO + charger PDO).
    #[default]
    High,
    /// Supported by telemetry with known alternative explanations (e.g. pinned 3A draw).
    Medium,
    /// Ambiguous telemetry or missing controller readings.
    Low,
}

/// Categorical reason a USB-C port may not be charging at full speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Bottleneck {
    /// No charger detected (or no source PDOs).
    NoCharger,
    /// Charger is offering less than expected.
    ChargerLimit,
    /// Reported sink demand exceeds the cable rating, or (when demand is
    /// unknown) the selected source offer exceeds it with lower confidence.
    CableLimit,
    /// Contract current pinned at 3A while the selected PDO offers more
    /// and no cable e-marker is visible — cables without an e-marker are
    /// limited to 3A by spec.
    CableNoEMarker,
    /// The negotiated contract is well below what the charger can deliver.
    DeviceLimit,
    /// The contract is healthy but the sink is requesting much less than
    /// it allows — typically a battery charge limit or thermal policy on
    /// the sink side, not a cable or charger problem.
    SinkLimit,
    /// Charging is at the charger's maximum.
    Fine,
    /// Contract exceeds spec limits (e.g. >3A contract without an e-marked cable).
    ContractOutOfSpec,
}

fn static_str_from_string(s: String) -> &'static str {
    match s.as_str() {
        "FINE" => "FINE",
        "CONTRACT_OUT_OF_SPEC" => "CONTRACT_OUT_OF_SPEC",
        "PORT_HARDWARE_LIMIT" => "PORT_HARDWARE_LIMIT",
        "CABLE_LIMIT" => "CABLE_LIMIT",
        "CABLE_NO_EMARKER" => "CABLE_NO_EMARKER",
        "CHARGER_UNDERPOWERED" => "CHARGER_UNDERPOWERED",
        "DEVICE_LIMIT" => "DEVICE_LIMIT",
        "SINK_LIMIT" => "SINK_LIMIT",
        "" => "FINE",
        _ => "UNKNOWN",
    }
}

/// Helper for deserializing a static string.
pub fn deserialize_static_str<'de, D>(deserializer: D) -> Result<&'static str, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Ok(static_str_from_string(s))
}

#[derive(Deserialize)]
struct ChargingDiagnosticHelper {
    bottleneck: Bottleneck,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    detail: String,
    #[serde(default)]
    is_warning: bool,
    #[serde(default)]
    status: Option<DiagnosticStatus>,
    #[serde(default)]
    root_cause: Option<RootCauseCategory>,
    #[serde(default)]
    confidence: Option<DiagnosticConfidence>,
    #[serde(default)]
    recommended_action: String,
    #[serde(default)]
    rule_id: String,
}

/// Result of [`ChargingDiagnostic::evaluate`].
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ChargingDiagnostic {
    /// Categorical bottleneck classification.
    pub bottleneck: Bottleneck,
    /// One-line headline (`"Cable is limiting charging speed"`).
    pub summary: String,
    /// Optional second-line detail.
    pub detail: String,
    /// `true` when the bottleneck is user-actionable (i.e. swap the cable).
    pub is_warning: bool,

    /// Categorical health assessment (`Healthy`, `Warning`, `Critical`).
    pub status: DiagnosticStatus,
    /// Subsystem responsible for the observed limitation.
    pub root_cause: RootCauseCategory,
    /// Confidence level in this diagnosis.
    pub confidence: DiagnosticConfidence,
    /// Actionable plain-English recommendation for the user.
    pub recommended_action: String,
    /// Diagnostic rule identifier. Unrecognized deserialized IDs map to `"UNKNOWN"`
    /// rather than retaining arbitrary input in permanent static allocations.
    #[serde(deserialize_with = "deserialize_static_str")]
    pub rule_id: &'static str,
}

impl<'de> Deserialize<'de> for ChargingDiagnostic {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let helper = ChargingDiagnosticHelper::deserialize(deserializer)?;
        let (root, rule) = match helper.bottleneck {
            Bottleneck::CableLimit => (RootCauseCategory::Cable, "CABLE_LIMIT"),
            Bottleneck::CableNoEMarker => (RootCauseCategory::Cable, "CABLE_NO_EMARKER"),
            Bottleneck::ChargerLimit => (RootCauseCategory::Charger, "CHARGER_UNDERPOWERED"),
            Bottleneck::DeviceLimit => (RootCauseCategory::Sink, "DEVICE_LIMIT"),
            Bottleneck::SinkLimit => (RootCauseCategory::Sink, "SINK_LIMIT"),
            Bottleneck::ContractOutOfSpec => (RootCauseCategory::Firmware, "CONTRACT_OUT_OF_SPEC"),
            Bottleneck::NoCharger => (RootCauseCategory::None, "UNKNOWN"),
            Bottleneck::Fine => (RootCauseCategory::None, "FINE"),
        };
        let status = if helper.bottleneck == Bottleneck::ContractOutOfSpec {
            DiagnosticStatus::Critical
        } else if helper.is_warning {
            DiagnosticStatus::Warning
        } else {
            DiagnosticStatus::Healthy
        };
        Ok(ChargingDiagnostic {
            bottleneck: helper.bottleneck,
            summary: helper.summary,
            detail: helper.detail,
            is_warning: helper.is_warning,
            status: helper.status.unwrap_or(status),
            root_cause: helper.root_cause.unwrap_or(root),
            confidence: helper.confidence.unwrap_or(DiagnosticConfidence::Low),
            recommended_action: helper.recommended_action,
            rule_id: if helper.rule_id.is_empty() {
                rule
            } else {
                static_str_from_string(helper.rule_id)
            },
        })
    }
}

impl Default for ChargingDiagnostic {
    fn default() -> Self {
        ChargingDiagnostic {
            bottleneck: Bottleneck::Fine,
            summary: String::new(),
            detail: String::new(),
            is_warning: false,
            status: DiagnosticStatus::Healthy,
            root_cause: RootCauseCategory::None,
            confidence: DiagnosticConfidence::High,
            recommended_action: String::new(),
            rule_id: "FINE",
        }
    }
}

impl ChargingDiagnostic {
    /// Compute a diagnostic for `pd_port`, optionally informed by `cable`
    /// and the sink's requested operating power.
    ///
    /// `requested_mw` is the RDO operating power (`voltage_now ×
    /// current_now` off the UCSI power-supply) — the draw ceiling the sink
    /// asked for, **not** a measured flow. Pass `0` when unavailable.
    /// It distinguishes "the negotiation came out low" (cable/charger
    /// problem — what this app exists to catch) from "the contract is
    /// fine but the sink is asking for less" (benign sink policy, e.g. a
    /// battery charge limit).
    ///
    /// `requested_ma` is the RDO operating current in mA (`current_now`
    /// off the UCSI power-supply), `0` = unknown. A request pinned right
    /// at 3A while the selected PDO offers more — with no cable e-marker
    /// visible — is the signature of a non-e-marked cable, which the spec
    /// caps at 3A.
    ///
    /// Returns `None` when no source capabilities are advertised — without
    /// that we have no charger reference to compare against.
    pub fn evaluate(
        pd_port: &PowerDeliveryPort,
        cable: Option<&CableInfo>,
        requested_mw: u32,
        requested_ma: u32,
    ) -> Option<ChargingDiagnostic> {
        if pd_port.source_capabilities.is_empty() {
            return None;
        }
        let charger_max_w = pd_port.max_source_power_mw / 1000;
        if charger_max_w == 0 {
            return None;
        }

        let active_pdo = pd_port.source_capabilities.iter().find(|p| p.is_active);
        let active_w = active_pdo
            .map(|p| p.power_mw / 1000)
            .filter(|w| *w > 0)
            .unwrap_or(charger_max_w);
        let active_ma = active_pdo.map(|p| p.current_ma).unwrap_or(0);

        let cable_max_w = cable.map(|c| c.max_watts).unwrap_or(0);
        let requested_w = requested_mw / 1000;

        let sink_cap_max_w = pd_port
            .sink_capabilities
            .iter()
            .map(|p| p.power_mw / 1000)
            .max()
            .unwrap_or(0);

        // A source PDO is an available ceiling, not the sink's operating current.
        // Missing cable identity does not establish a 3A rating.
        let known_3a =
            cable.is_some_and(|c| c.current_rating == Some(crate::pd::CableCurrent::ThreeAmp));

        if known_3a && requested_ma > 3_200 {
            Some(ChargingDiagnostic {
                bottleneck: Bottleneck::ContractOutOfSpec,
                summary: "Reported operating current exceeds cable's declared 3A rating".into(),
                detail: format!(
                    "Reported operating current is {:.1}A on a cable declaring 3A. Verify the telemetry and cable before continuing high-current operation.",
                    requested_ma as f64 / 1000.0
                ),
                is_warning: true,
                status: DiagnosticStatus::Critical,
                root_cause: RootCauseCategory::Firmware,
                confidence: DiagnosticConfidence::High,
                recommended_action: "Immediately replace or verify the cable and controller telemetry before further high-current use; use a suitably rated 5A cable if the reported current is correct.".into(),
                rule_id: "CONTRACT_OUT_OF_SPEC",
            })
        } else if sink_cap_max_w > 0
            && sink_cap_max_w <= active_w
            && sink_cap_max_w < charger_max_w
            && (cable_max_w == 0 || cable_max_w >= sink_cap_max_w)
            && (active_w as f64) < (charger_max_w as f64) * 0.8
        {
            Some(ChargingDiagnostic {
                bottleneck: Bottleneck::DeviceLimit,
                summary: format!("Contract limited to {active_w}W by port hardware"),
                detail: format!(
                    "Negotiated {active_w}W of the {charger_max_w}W offered — host port hardware sink capability ceiling is {sink_cap_max_w}W."
                ),
                is_warning: false,
                status: DiagnosticStatus::Healthy,
                root_cause: RootCauseCategory::Port,
                confidence: DiagnosticConfidence::High,
                recommended_action: "No action required; charging is operating at maximum rate supported by this port's hardware.".into(),
                rule_id: "PORT_HARDWARE_LIMIT",
            })
        } else if cable_max_w > 0
            && (if requested_mw > 0 {
                u64::from(requested_mw) > u64::from(cable_max_w) * 1000
            } else {
                active_w >= cable_max_w && charger_max_w > cable_max_w
            })
            && (sink_cap_max_w == 0 || cable_max_w < sink_cap_max_w)
        {
            let (confidence, detail, recommended_action) = if requested_w > 0 {
                (
                    DiagnosticConfidence::High,
                    format!(
                        "Sink requests up to {requested_w}W, but the cable is rated for {cable_max_w}W; charger can deliver {charger_max_w}W."
                    ),
                    format!(
                        "Use a USB-C cable rated for at least {requested_w}W to support the reported request."
                    ),
                )
            } else if active_pdo.is_some() {
                (
                    DiagnosticConfidence::Medium,
                    format!(
                        "Selected source offer is {active_w}W and exceeds the cable's {cable_max_w}W rating; sink operating demand is unavailable."
                    ),
                    format!(
                        "Check the sink's operating demand; use a cable rated above {cable_max_w}W only if the device needs more power."
                    ),
                )
            } else {
                (
                    DiagnosticConfidence::Low,
                    format!(
                        "Charger advertises {charger_max_w}W, above the cable's {cable_max_w}W rating, but no active PDO or sink demand was reported."
                    ),
                    "Verify the active contract and sink demand before replacing the cable.".into(),
                )
            };
            Some(ChargingDiagnostic {
                bottleneck: Bottleneck::CableLimit,
                summary: if requested_w > 0 {
                    "Cable rating is below reported sink demand".into()
                } else {
                    "Cable rating is below available source capability".into()
                },
                detail,
                is_warning: true,
                status: DiagnosticStatus::Warning,
                root_cause: RootCauseCategory::Cable,
                confidence,
                recommended_action,
                rule_id: "CABLE_LIMIT",
            })
        } else if cable.and_then(|c| c.current_rating).is_none()
            && active_ma > 3_100
            && (2_900..=3_100).contains(&requested_ma)
        {
            // Request pinned at 3A against a >3A PDO with no e-marker
            // rating in sight (covers cable-absent too). Phrase as "not
            // visible", never "missing" — some UCSI firmwares simply
            // don't populate cable nodes.
            // Sitting after CableLimit is safe: cable.rs sets max_watts and
            // current_rating together, so max_watts > 0 implies a rating is
            // present. Pinned by cable.rs::max_watts_implies_current_rating_is_set.
            Some(ChargingDiagnostic {
                bottleneck: Bottleneck::CableNoEMarker,
                summary: "Cable may be limiting current to 3A".into(),
                detail: format!(
                    "Contract offers {:.1}A but the device draws 3.0A and no cable e-marker \
                     is visible — cables without an e-marker are limited to 3A",
                    active_ma as f64 / 1000.0
                ),
                is_warning: true,
                status: DiagnosticStatus::Warning,
                root_cause: RootCauseCategory::Cable,
                confidence: DiagnosticConfidence::Medium,
                recommended_action:
                    "Use an e-marked 5A USB-C cable to enable charging rates above 60W (3A).".into(),
                rule_id: "CABLE_NO_EMARKER",
            })
        } else if requested_w > charger_max_w || (charger_max_w <= 15 && requested_w == 0) {
            let (detail, recommended_action) = if requested_w > 0 {
                (
                    format!(
                        "Device requests {requested_w}W, but charger provides only {charger_max_w}W."
                    ),
                    format!(
                        "Connect a higher-wattage USB-PD charger ({requested_w}W or higher) for adequate charging."
                    ),
                )
            } else {
                (
                    format!(
                        "Charger provides only {charger_max_w}W, which is insufficient for standard host operation."
                    ),
                    "Connect a higher-wattage USB-PD charger (45W or higher) for adequate charging.".to_string(),
                )
            };
            Some(ChargingDiagnostic {
                bottleneck: Bottleneck::ChargerLimit,
                summary: format!("Charger power is limited to {charger_max_w}W"),
                detail,
                is_warning: true,
                status: DiagnosticStatus::Warning,
                root_cause: RootCauseCategory::Charger,
                confidence: DiagnosticConfidence::High,
                recommended_action,
                rule_id: "CHARGER_UNDERPOWERED",
            })
        } else if active_w > 0 && (active_w as f64) < (charger_max_w as f64) * 0.8 {
            Some(ChargingDiagnostic {
                bottleneck: Bottleneck::DeviceLimit,
                summary: format!("Contract limited to {active_w}W"),
                detail: format!(
                    "Negotiated {active_w}W of the {charger_max_w}W the charger offers — \
                     often the device's own policy (battery charge limit or thermal)"
                ),
                is_warning: false,
                status: DiagnosticStatus::Healthy,
                root_cause: RootCauseCategory::Sink,
                confidence: DiagnosticConfidence::Medium,
                recommended_action: "No action required; the device is operating within its own power/thermal policy.".into(),
                rule_id: "DEVICE_LIMIT",
            })
        } else if requested_w > 0 && (requested_w as f64) < (active_w as f64) * 0.8 {
            // Contract is healthy — the sink itself is asking for less.
            Some(ChargingDiagnostic {
                bottleneck: Bottleneck::SinkLimit,
                summary: format!("Device is limiting its draw to {requested_w}W"),
                detail: format!(
                    "Contract allows {active_w}W; the device requests up to {requested_w}W — \
                     often a battery charge limit or thermal policy"
                ),
                is_warning: false,
                status: DiagnosticStatus::Healthy,
                root_cause: RootCauseCategory::Sink,
                confidence: DiagnosticConfidence::High,
                recommended_action:
                    "No action required; the connected device has voluntarily reduced power draw."
                        .into(),
                rule_id: "SINK_LIMIT",
            })
        } else {
            Some(ChargingDiagnostic {
                bottleneck: Bottleneck::Fine,
                summary: format!("Charging at up to {active_w}W"),
                detail: String::new(),
                is_warning: false,
                status: DiagnosticStatus::Healthy,
                root_cause: RootCauseCategory::None,
                confidence: DiagnosticConfidence::High,
                recommended_action:
                    "No action required; charging is operating optimally at charger maximum.".into(),
                rule_id: "FINE",
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power::{PdoType, PowerDataObject};

    fn pdo(power_mw: u32, active: bool) -> PowerDataObject {
        PowerDataObject {
            r#type: PdoType::FixedSupply,
            power_mw,
            is_active: active,
            ..Default::default()
        }
    }

    fn pd(caps: Vec<PowerDataObject>) -> PowerDeliveryPort {
        let max = caps.iter().map(|p| p.power_mw).max().unwrap_or(0);
        PowerDeliveryPort {
            source_capabilities: caps,
            max_source_power_mw: max,
            ..Default::default()
        }
    }

    #[test]
    fn no_source_caps_returns_none() {
        assert!(ChargingDiagnostic::evaluate(&PowerDeliveryPort::default(), None, 0, 0).is_none());
    }

    #[test]
    fn zero_charger_returns_none() {
        let port = pd(vec![pdo(0, false)]);
        assert!(ChargingDiagnostic::evaluate(&port, None, 0, 0).is_none());
    }

    #[test]
    fn cable_limit_flagged_with_warning() {
        let port = pd(vec![pdo(100_000, true)]);
        let cable = CableInfo {
            max_watts: 60,
            ..Default::default()
        };
        let d = ChargingDiagnostic::evaluate(&port, Some(&cable), 80_000, 0).unwrap();
        assert_eq!(d.bottleneck, Bottleneck::CableLimit);
        assert!(d.is_warning);
        assert!(d.detail.contains("60W"));
    }

    #[test]
    fn cable_rating_boundary_uses_precise_milliwatts() {
        let port = pd(vec![pdo(60_000, true)]);
        let cable = CableInfo {
            max_watts: 60,
            ..Default::default()
        };
        for (requested_mw, expected) in [
            (59_999, Bottleneck::Fine),
            (60_000, Bottleneck::Fine),
            (60_001, Bottleneck::CableLimit),
            (80_000, Bottleneck::CableLimit),
        ] {
            let d = ChargingDiagnostic::evaluate(&port, Some(&cable), requested_mw, 3_000).unwrap();
            assert_eq!(d.bottleneck, expected, "request {requested_mw}mW");
            assert_eq!(d.is_warning, expected == Bottleneck::CableLimit);
        }
    }

    #[test]
    fn cable_above_selected_contract_is_not_called_the_bottleneck() {
        // A 60W cable cannot be the cause of a 45W selected offer even when
        // the charger advertises a separate 100W PDO.
        let port = pd(vec![pdo(100_000, false), pdo(45_000, true)]);
        let cable = CableInfo {
            max_watts: 60,
            ..Default::default()
        };
        let d = ChargingDiagnostic::evaluate(&port, Some(&cable), 45_000, 0).unwrap();
        assert_ne!(d.bottleneck, Bottleneck::CableLimit);
        assert_eq!(d.root_cause, RootCauseCategory::Sink);
    }

    #[test]
    fn device_limit_when_active_below_80pct() {
        let port = pd(vec![pdo(100_000, false), pdo(15_000, true)]);
        let d = ChargingDiagnostic::evaluate(&port, None, 0, 0).unwrap();
        assert_eq!(d.bottleneck, Bottleneck::DeviceLimit);
        assert!(!d.is_warning);
        assert!(d.summary.contains("15W"));
    }

    #[test]
    fn fine_when_active_meets_charger() {
        let port = pd(vec![pdo(60_000, true)]);
        let d = ChargingDiagnostic::evaluate(&port, None, 0, 0).unwrap();
        assert_eq!(d.bottleneck, Bottleneck::Fine);
        assert!(d.summary.contains("60W"));
    }

    #[test]
    fn fine_when_no_active_pdo_uses_max() {
        let port = pd(vec![pdo(100_000, false)]);
        let cable = CableInfo {
            max_watts: 240,
            ..Default::default()
        };
        let d = ChargingDiagnostic::evaluate(&port, Some(&cable), 80_000, 0).unwrap();
        assert_eq!(d.bottleneck, Bottleneck::Fine);
        assert!(d.summary.contains("100W"));
    }

    #[test]
    fn sink_limit_when_request_far_below_healthy_contract() {
        // 65W charger, full 65W contract negotiated, but the sink only
        // requests 15W in its RDO (e.g. a laptop with an 80% battery
        // charge limit). Benign — must not raise is_warning.
        let port = pd(vec![pdo(65_000, true)]);
        let d = ChargingDiagnostic::evaluate(&port, None, 15_000, 0).unwrap();
        assert_eq!(d.bottleneck, Bottleneck::SinkLimit);
        assert!(!d.is_warning);
        assert!(d.summary.contains("15W"));
        assert!(d.detail.contains("65W"));
    }

    #[test]
    fn sink_limit_not_raised_when_request_near_contract() {
        // Requesting 60W of a 65W contract (> 80%) is Fine, not SinkLimit.
        let port = pd(vec![pdo(65_000, true)]);
        let d = ChargingDiagnostic::evaluate(&port, None, 60_000, 0).unwrap();
        assert_eq!(d.bottleneck, Bottleneck::Fine);
    }

    #[test]
    fn below_rating_sink_request_is_not_cable_limited() {
        // A 10W request is not limited by a 60W cable, even when the source
        // advertises a 100W PDO.
        let port = pd(vec![pdo(100_000, true)]);
        let cable = CableInfo {
            max_watts: 60,
            ..Default::default()
        };
        let d = ChargingDiagnostic::evaluate(&port, Some(&cable), 10_000, 0).unwrap();
        assert_eq!(d.bottleneck, Bottleneck::SinkLimit);
        assert!(!d.is_warning);
    }

    #[test]
    fn reported_sink_demand_above_cable_rating_triggers_warning() {
        let port = pd(vec![pdo(100_000, true)]);
        let cable = CableInfo {
            max_watts: 60,
            ..Default::default()
        };
        let d = ChargingDiagnostic::evaluate(&port, Some(&cable), 80_000, 0).unwrap();
        assert_eq!(d.bottleneck, Bottleneck::CableLimit);
        assert!(d.is_warning);
        assert_eq!(d.confidence, DiagnosticConfidence::High);
        assert!(d.recommended_action.contains("80W"));
    }

    #[test]
    fn device_limit_wins_over_sink_limit() {
        // Contract itself is low (15W of 100W) — that's the dominant
        // story even if the request is lower still.
        let port = pd(vec![pdo(100_000, false), pdo(15_000, true)]);
        let d = ChargingDiagnostic::evaluate(&port, None, 5_000, 0).unwrap();
        assert_eq!(d.bottleneck, Bottleneck::DeviceLimit);
    }

    fn pdo_with_current(power_mw: u32, current_ma: u32, active: bool) -> PowerDataObject {
        PowerDataObject {
            r#type: PdoType::FixedSupply,
            current_ma,
            power_mw,
            is_active: active,
            ..Default::default()
        }
    }

    #[test]
    fn cable_no_emarker_flagged_when_request_pinned_at_3a() {
        // 20V/5A active PDO, no cable e-marker visible, sink draws exactly
        // 3.0A — the signature of a non-e-marked cable (3A by spec).
        let port = pd(vec![pdo_with_current(100_000, 5_000, true)]);
        let d = ChargingDiagnostic::evaluate(&port, None, 60_000, 3_000).unwrap();
        assert_eq!(d.bottleneck, Bottleneck::CableNoEMarker);
        assert!(d.is_warning);
        assert!(d.summary.contains("3A"));
        assert!(d.detail.contains("5.0A"));
    }

    #[test]
    fn cable_no_emarker_not_raised_when_emarker_rating_present() {
        // The reported 60W request meets the cable rating, so neither a
        // cable overload nor an absent-marker inference is justified.
        use crate::pd::CableCurrent;
        let port = pd(vec![pdo_with_current(100_000, 5_000, true)]);
        let cable = CableInfo {
            current_rating: Some(CableCurrent::ThreeAmp),
            max_watts: 60,
            ..Default::default()
        };
        let d = ChargingDiagnostic::evaluate(&port, Some(&cable), 60_000, 3_000).unwrap();
        assert_eq!(d.bottleneck, Bottleneck::SinkLimit);
        assert!(!d.is_warning);
    }

    #[test]
    fn cable_no_emarker_not_raised_when_request_above_3a() {
        // 3.25A flowing proves the cable isn't 3A-limited — no hint.
        let port = pd(vec![pdo_with_current(100_000, 5_000, true)]);
        let d = ChargingDiagnostic::evaluate(&port, None, 65_000, 3_250).unwrap();
        assert_ne!(d.bottleneck, Bottleneck::CableNoEMarker);
    }

    #[test]
    fn cable_no_emarker_not_raised_when_pdo_offers_only_3a() {
        // A 3A contract fully consumed is Fine, not a cable hint.
        let port = pd(vec![pdo_with_current(60_000, 3_000, true)]);
        let d = ChargingDiagnostic::evaluate(&port, None, 60_000, 3_000).unwrap();
        assert_eq!(d.bottleneck, Bottleneck::Fine);
    }

    #[test]
    fn device_limit_detail_carries_benign_hint() {
        // On the live laptop a 5V/3A contract of a 65W advertisement is
        // diagnosed DeviceLimit — it must not read as a cable accusation.
        let port = pd(vec![pdo(65_000, false), pdo(15_000, true)]);
        let d = ChargingDiagnostic::evaluate(&port, None, 0, 0).unwrap();
        assert_eq!(d.bottleneck, Bottleneck::DeviceLimit);
        assert!(d.detail.contains("battery charge limit or thermal"));
    }

    #[test]
    fn r2_structured_fields_on_cable_limit() {
        let port = pd(vec![pdo(100_000, true)]);
        let cable = CableInfo {
            max_watts: 60,
            ..Default::default()
        };
        let d = ChargingDiagnostic::evaluate(&port, Some(&cable), 80_000, 0).unwrap();
        assert_eq!(d.status, DiagnosticStatus::Warning);
        assert_eq!(d.root_cause, RootCauseCategory::Cable);
        assert_eq!(d.confidence, DiagnosticConfidence::High);
        assert_eq!(d.rule_id, "CABLE_LIMIT");
        assert!(d.recommended_action.contains("80W"));
    }

    #[test]
    fn r2_charger_underpowered_evaluates_warning() {
        // 15W charger (5V/3A) with device requesting 45W (45_000 mW)
        let port = pd(vec![pdo(15_000, true)]);
        let d = ChargingDiagnostic::evaluate(&port, None, 45_000, 0).unwrap();
        assert_eq!(d.bottleneck, Bottleneck::ChargerLimit);
        assert!(d.is_warning);
        assert_eq!(d.status, DiagnosticStatus::Warning);
        assert_eq!(d.root_cause, RootCauseCategory::Charger);
        assert_eq!(d.confidence, DiagnosticConfidence::High);
        assert_eq!(d.rule_id, "CHARGER_UNDERPOWERED");
        assert!(d.summary.contains("15W"));
        assert!(d.recommended_action.contains("45W or higher"));
    }

    #[test]
    fn r2_contract_out_of_spec_safety_critical() {
        // Positive 3A rating plus reported 4A operating current is a mismatch.
        let port = pd(vec![pdo_with_current(100_000, 5_000, true)]);
        let cable = CableInfo {
            current_rating: Some(crate::pd::CableCurrent::ThreeAmp),
            max_watts: 60,
            ..Default::default()
        };
        let d = ChargingDiagnostic::evaluate(&port, Some(&cable), 80_000, 4_000).unwrap();
        assert_eq!(d.bottleneck, Bottleneck::ContractOutOfSpec);
        assert!(d.is_warning);
        assert_eq!(d.status, DiagnosticStatus::Critical);
        assert_eq!(d.root_cause, RootCauseCategory::Firmware);
        assert_eq!(d.confidence, DiagnosticConfidence::High);
        assert_eq!(d.rule_id, "CONTRACT_OUT_OF_SPEC");
        assert!(d.recommended_action.contains("Immediately replace"));
    }

    #[test]
    fn r2_fine_and_sink_limit_have_healthy_status() {
        let port = pd(vec![pdo(65_000, true)]);
        let d_sink = ChargingDiagnostic::evaluate(&port, None, 15_000, 0).unwrap();
        assert_eq!(d_sink.status, DiagnosticStatus::Healthy);
        assert_eq!(d_sink.root_cause, RootCauseCategory::Sink);
        assert_eq!(d_sink.rule_id, "SINK_LIMIT");

        let d_fine = ChargingDiagnostic::evaluate(&port, None, 65_000, 0).unwrap();
        assert_eq!(d_fine.status, DiagnosticStatus::Healthy);
        assert_eq!(d_fine.root_cause, RootCauseCategory::None);
        assert_eq!(d_fine.rule_id, "FINE");
    }

    #[test]
    fn r2_charging_diagnostic_serde_roundtrip() {
        let port = pd(vec![pdo(100_000, true)]);
        let cable = CableInfo {
            max_watts: 60,
            ..Default::default()
        };
        let d = ChargingDiagnostic::evaluate(&port, Some(&cable), 80_000, 0).unwrap();
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"status\":\"Warning\""));
        assert!(json.contains("\"root_cause\":\"Cable\""));
        assert!(json.contains("\"confidence\":\"High\""));
        assert!(json.contains("\"rule_id\":\"CABLE_LIMIT\""));
        assert!(json.contains("\"recommended_action\":"));

        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["bottleneck"], "CableLimit");
        assert_eq!(val["status"], "Warning");
        assert_eq!(val["root_cause"], "Cable");
        assert_eq!(val["confidence"], "High");
        assert_eq!(val["rule_id"], "CABLE_LIMIT");
        assert_eq!(val["is_warning"], true);
    }

    #[test]
    fn r2_underpowered_charger_with_zero_requested_w() {
        // 15W charger (5V/3A) with no UCSI telemetry (requested_mw = 0)
        let port = pd(vec![pdo(15_000, true)]);
        let d = ChargingDiagnostic::evaluate(&port, None, 0, 0).unwrap();
        assert_eq!(d.bottleneck, Bottleneck::ChargerLimit);
        assert!(d.is_warning);
        assert_eq!(d.status, DiagnosticStatus::Warning);
        assert_eq!(d.root_cause, RootCauseCategory::Charger);
        assert_eq!(d.confidence, DiagnosticConfidence::High);
        assert_eq!(d.rule_id, "CHARGER_UNDERPOWERED");
        assert!(d.summary.contains("15W"));
        assert!(d.recommended_action.contains("45W or higher"));
    }

    #[test]
    fn r2_contract_out_of_spec_zero_requested_ma_no_cable() {
        // 5A contract (100W) active, cable is None, requested_ma = 0
        let port = pd(vec![pdo_with_current(100_000, 5_000, true)]);
        let d = ChargingDiagnostic::evaluate(&port, None, 0, 0).unwrap();
        // Available PDO current and absent identity do not prove unsafe operation.
        assert_eq!(d.bottleneck, Bottleneck::Fine);
        assert!(!d.is_warning);
        assert_ne!(d.status, DiagnosticStatus::Critical);
    }

    #[test]
    fn r2_port_hardware_sink_limit_assignment() {
        // Charger offers 100W (20V/5A), but port hardware sink-caps ceiling is 45W (15V/3A)
        let mut port = pd(vec![
            pdo_with_current(100_000, 5_000, false),
            pdo_with_current(45_000, 3_000, true),
        ]);
        port.sink_capabilities = vec![pdo_with_current(45_000, 3_000, false)];

        let d = ChargingDiagnostic::evaluate(&port, None, 0, 0).unwrap();
        assert_eq!(d.bottleneck, Bottleneck::DeviceLimit);
        assert_eq!(d.status, DiagnosticStatus::Healthy);
        assert_eq!(d.root_cause, RootCauseCategory::Port);
        assert_eq!(d.confidence, DiagnosticConfidence::High);
        assert_eq!(d.rule_id, "PORT_HARDWARE_LIMIT");
        assert!(d.summary.contains("by port hardware"));
        assert!(d
            .detail
            .contains("host port hardware sink capability ceiling is 45W"));
    }

    #[test]
    fn r2_port_hardware_limit_takes_precedence_over_cable_limit_when_cable_meets_port_ceiling() {
        // 140W charger (28V/5A), host port hardware sink-caps ceiling is 100W (20V/5A).
        // 100W compliant cable attached.
        let mut port = pd(vec![
            pdo_with_current(140_000, 5_000, false),
            pdo_with_current(100_000, 5_000, true),
        ]);
        port.sink_capabilities = vec![pdo_with_current(100_000, 5_000, false)];
        let cable = CableInfo {
            max_watts: 100,
            current_rating: Some(crate::pd::CableCurrent::FiveAmp),
            ..Default::default()
        };

        let d = ChargingDiagnostic::evaluate(&port, Some(&cable), 80_000, 0).unwrap();
        assert_eq!(d.bottleneck, Bottleneck::DeviceLimit);
        assert_eq!(d.status, DiagnosticStatus::Healthy);
        assert_eq!(d.root_cause, RootCauseCategory::Port);
        assert_eq!(d.confidence, DiagnosticConfidence::High);
        assert_eq!(d.rule_id, "PORT_HARDWARE_LIMIT");
        assert!(d.summary.contains("limited to 100W by port hardware"));
    }

    #[test]
    fn r2_cable_limit_triggers_when_cable_is_below_port_sink_ceiling() {
        // 140W charger (28V/5A), host port sink-caps ceiling is 100W (20V/5A).
        // Cable is only 60W (3A) -> Cable is the genuine bottleneck!
        let mut port = pd(vec![
            pdo_with_current(140_000, 5_000, false),
            pdo_with_current(100_000, 5_000, false),
            pdo_with_current(60_000, 3_000, true),
        ]);
        port.sink_capabilities = vec![pdo_with_current(100_000, 5_000, false)];
        let cable = CableInfo {
            max_watts: 60,
            current_rating: Some(crate::pd::CableCurrent::ThreeAmp),
            ..Default::default()
        };

        let d = ChargingDiagnostic::evaluate(&port, Some(&cable), 80_000, 0).unwrap();
        assert_eq!(d.bottleneck, Bottleneck::CableLimit);
        assert_eq!(d.status, DiagnosticStatus::Warning);
        assert_eq!(d.root_cause, RootCauseCategory::Cable);
        assert_eq!(d.confidence, DiagnosticConfidence::High);
        assert_eq!(d.rule_id, "CABLE_LIMIT");
        assert!(d.recommended_action.contains("80W"));
    }
}
