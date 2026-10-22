//! Warning-severity classification of a [`DeviceSummary`].
//!
//! The core library carries several independent health signals on a
//! summary — the charging diagnostic, the data-rate assessment, the cable
//! trust verdict and a few boolean properties. This module folds them into
//! one ordered [`Severity`] so the CLI can filter (`--severity`), rank
//! (`--summary top`) and badge (`--compact`) entries without touching the
//! library types.
//!
//! Nothing is invented: a summary with none of those signals is
//! [`Severity::Ok`], and every non-`Ok` verdict carries the library's own
//! wording as its reason.

use clap::ValueEnum;
use serde::Serialize;
use usbeehive::bos::DataRateVerdict;
use usbeehive::diagnostic::DiagnosticStatus;
use usbeehive::summary::DeviceSummary;
use usbeehive::trust::CableTrustLevel;

/// Ordered warning severity. `Ok < Info < Warning < Critical`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, ValueEnum, Serialize, Default,
)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// No health signal fired.
    #[default]
    Ok,
    /// Worth knowing, not a fault (link below advertised capability,
    /// constrained cable, no visible e-marker).
    Info,
    /// User-actionable problem (slow cable, degraded link, suspicious cable).
    Warning,
    /// Severe condition or protocol violation reported by the diagnostic.
    Critical,
}

impl Severity {
    /// Every severity, lowest first.
    pub const ALL: [Severity; 4] = [
        Severity::Ok,
        Severity::Info,
        Severity::Warning,
        Severity::Critical,
    ];

    /// Lower-case machine label (`"ok"`, `"info"`, `"warning"`, `"critical"`).
    pub fn label(self) -> &'static str {
        match self {
            Severity::Ok => "ok",
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Critical => "critical",
        }
    }

    /// One-character badge for compact listings.
    #[allow(dead_code)]
    pub fn badge(self) -> &'static str {
        match self {
            Severity::Ok => " ",
            Severity::Info => "i",
            Severity::Warning => "!",
            Severity::Critical => "X",
        }
    }
}

/// Result of [`assess`]: the worst severity plus every reason that fired.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Assessment {
    /// Highest severity among the signals.
    pub severity: Severity,
    /// Reasons, in a fixed signal order (diagnostic, data rate, cable,
    /// properties), de-duplicated.
    pub reasons: Vec<String>,
}

impl Assessment {
    fn raise(&mut self, severity: Severity, reason: impl Into<String>) {
        let reason = reason.into();
        self.severity = self.severity.max(severity);
        if !self.reasons.contains(&reason) {
            self.reasons.push(reason);
        }
    }
}

/// Classify `s`. See the module docs for the signals considered.
pub fn assess(s: &DeviceSummary) -> Assessment {
    let mut a = Assessment::default();
    assess_diagnostic(s, &mut a);
    assess_data_rate(s, &mut a);
    assess_cable(s, &mut a);
    assess_properties(s, &mut a);
    a
}

/// Shorthand for `assess(s).severity`.
pub fn severity_of(s: &DeviceSummary) -> Severity {
    assess(s).severity
}

fn assess_diagnostic(s: &DeviceSummary, a: &mut Assessment) {
    let Some(d) = &s.charging_diag else { return };
    match d.status {
        DiagnosticStatus::Critical => a.raise(Severity::Critical, d.summary.clone()),
        DiagnosticStatus::Warning => a.raise(Severity::Warning, d.summary.clone()),
        DiagnosticStatus::Healthy if d.is_warning => a.raise(Severity::Warning, d.summary.clone()),
        DiagnosticStatus::Healthy => {}
    }
}

fn assess_data_rate(s: &DeviceSummary, a: &mut Assessment) {
    let Some(r) = &s.data_rate else { return };
    let text = |fallback: &str| {
        if r.summary.is_empty() {
            fallback.to_string()
        } else {
            r.summary.clone()
        }
    };
    if r.is_warning || r.verdict == DataRateVerdict::Degraded {
        a.raise(
            Severity::Warning,
            text("Linked below the device's requirement"),
        );
    } else if r.verdict == DataRateVerdict::BelowCapability {
        a.raise(Severity::Info, text("Linked below advertised capability"));
    }
}

fn assess_cable(s: &DeviceSummary, a: &mut Assessment) {
    let Some(c) = &s.cable else { return };
    let t = &c.trust;
    if t.trust_level == CableTrustLevel::Suspicious {
        a.raise(Severity::Warning, "Cable trust: Suspicious");
    }
    if t.emarker_contract_mismatch {
        a.raise(
            Severity::Warning,
            "Active contract exceeds the cable's advertised rating",
        );
    }
    if t.unsupported_combination {
        a.raise(Severity::Warning, "Unsupported cable/charger combination");
    }
    if t.trust_level == CableTrustLevel::LikelyConstrained {
        a.raise(Severity::Info, "Cable trust: Likely Constrained");
    }
    if t.vid_unknown {
        a.raise(Severity::Info, "Cable vendor ID not in the USB-IF database");
    }
}

fn assess_properties(s: &DeviceSummary, a: &mut Assessment) {
    for (key, value) in &s.properties {
        match (key.as_str(), value.as_str()) {
            ("cable.data_speed_limit", v) => {
                a.raise(Severity::Warning, format!("Cable limits data to {v}"))
            }
            ("cable.no_emarker", "true") => a.raise(
                Severity::Info,
                "No cable e-marker visible (3A limit may apply)",
            ),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use usbeehive::bos::DataRateAssessment;
    use usbeehive::diagnostic::{Bottleneck, ChargingDiagnostic};
    use usbeehive::usb::UsbDevice;

    fn plain() -> DeviceSummary {
        DeviceSummary::from_usb_device(&UsbDevice::default())
    }

    fn diag(status: DiagnosticStatus, is_warning: bool) -> ChargingDiagnostic {
        ChargingDiagnostic {
            bottleneck: Bottleneck::Fine,
            summary: "diag".into(),
            detail: String::new(),
            is_warning,
            status,
            root_cause: Default::default(),
            confidence: Default::default(),
            recommended_action: String::new(),
            rule_id: "test",
        }
    }

    #[test]
    fn plain_device_is_ok_without_reasons() {
        let a = assess(&plain());
        assert_eq!(a.severity, Severity::Ok);
        assert!(a.reasons.is_empty());
    }

    #[test]
    fn ordering_is_ok_info_warning_critical() {
        assert!(Severity::Ok < Severity::Info);
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Critical);
        assert_eq!(Severity::ALL.iter().max(), Some(&Severity::Critical));
    }

    #[test]
    fn diagnostic_status_maps_to_severity() {
        let mut s = plain();
        s.charging_diag = Some(diag(DiagnosticStatus::Healthy, false));
        assert_eq!(severity_of(&s), Severity::Ok);
        s.charging_diag = Some(diag(DiagnosticStatus::Warning, true));
        assert_eq!(severity_of(&s), Severity::Warning);
        s.charging_diag = Some(diag(DiagnosticStatus::Critical, true));
        assert_eq!(severity_of(&s), Severity::Critical);
    }

    #[test]
    fn legacy_is_warning_flag_still_counts() {
        let mut s = plain();
        s.charging_diag = Some(diag(DiagnosticStatus::Healthy, true));
        assert_eq!(severity_of(&s), Severity::Warning);
    }

    #[test]
    fn data_rate_degraded_warns_and_below_capability_informs() {
        let mut s = plain();
        let mut r = DataRateAssessment {
            verdict: DataRateVerdict::Degraded,
            negotiated_mbps: 480,
            capable_mbps: 5000,
            functional_floor_mbps: 5000,
            capable_gen: String::new(),
            summary: "Linked at 480 Mbps".into(),
            detail: String::new(),
            is_warning: true,
        };
        s.data_rate = Some(r.clone());
        let a = assess(&s);
        assert_eq!(a.severity, Severity::Warning);
        assert_eq!(a.reasons, vec!["Linked at 480 Mbps".to_string()]);
        r.verdict = DataRateVerdict::BelowCapability;
        r.is_warning = false;
        s.data_rate = Some(r);
        assert_eq!(severity_of(&s), Severity::Info);
    }

    #[test]
    fn properties_feed_severity() {
        let mut s = plain();
        s.properties
            .push(("cable.no_emarker".into(), "true".into()));
        assert_eq!(severity_of(&s), Severity::Info);
        s.properties
            .push(("cable.data_speed_limit".into(), "USB 2.0".into()));
        let a = assess(&s);
        assert_eq!(a.severity, Severity::Warning);
        assert!(a.reasons.iter().any(|r| r.contains("USB 2.0")));
    }

    #[test]
    fn reasons_are_deduplicated_and_max_wins() {
        let mut a = Assessment::default();
        a.raise(Severity::Info, "x");
        a.raise(Severity::Warning, "x");
        a.raise(Severity::Info, "y");
        assert_eq!(a.severity, Severity::Warning);
        assert_eq!(a.reasons, vec!["x".to_string(), "y".to_string()]);
    }
}
