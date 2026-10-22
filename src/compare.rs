//! Pure-core snapshot comparison engine.
//!
//! Provides [`compare`], which inspects two [`Snapshot`] points in time and
//! yields a structured, semantic [`SnapshotDiff`] describing added devices,
//! removed devices, and granular modifications across connection status,
//! power draw, USB-PD contracts, link speeds, charging bottlenecks, and cable telemetry.
//!
//! # Zero-IO
//!
//! This module is entirely IO-free and compiles under `default-features = false`.

use std::collections::{BTreeSet, HashMap};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::bos::DataRateVerdict;
use crate::diagnostic::{Bottleneck, DiagnosticStatus, RootCauseCategory};
use crate::pd::{CableCurrent, CableSpeed};
use crate::snapshot::Snapshot;
use crate::summary::{DeviceSummary, PowerRole, Status};
use crate::trust::CableTrustLevel;

/// A transition of a value from a baseline state to a current state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValueChange<T> {
    /// Baseline / previous value.
    pub from: T,
    /// Current / after value.
    pub to: T,
}

impl<T> ValueChange<T> {
    /// Create a new value transition.
    pub fn new(from: T, to: T) -> Self {
        Self { from, to }
    }
}

/// High-level difference between two [`Snapshot`]s.
///
/// Contains newly added devices, removed devices, and detailed modifications
/// for devices present in both snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SnapshotDiff {
    /// Devices present in `after` but not in `before`.
    pub added: Vec<DeviceSummary>,
    /// Devices present in `before` but not in `after`.
    pub removed: Vec<DeviceSummary>,
    /// Devices present in both snapshots that underwent state, power, PD,
    /// diagnostic, or cable transitions.
    pub modified: Vec<DeviceDifference>,
}

impl SnapshotDiff {
    /// Returns `true` if no devices were added, removed, or modified.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.modified.is_empty()
    }

    /// Total count of all changed device entities.
    pub fn total_changes(&self) -> usize {
        self.added.len() + self.removed.len() + self.modified.len()
    }

    /// Returns `true` if any modified device transitioned to a warning/critical state
    /// or has an unresolved cable trust warning.
    pub fn has_warnings(&self) -> bool {
        self.modified.iter().any(|d| {
            d.diagnostic
                .as_ref()
                .is_some_and(|diag| diag.is_newly_degraded())
                || d.cable.as_ref().is_some_and(|c| {
                    c.is_suspicious == Some(ValueChange::new(false, true))
                        || !c.notes_added.is_empty()
                })
        })
    }

    /// Short human-readable summary string (e.g. `"+1 added, -1 removed, ~2 modified"`).
    pub fn summary_string(&self) -> String {
        if self.is_empty() {
            "No differences".to_string()
        } else {
            format!(
                "+{} added, -{} removed, ~{} modified",
                self.added.len(),
                self.removed.len(),
                self.modified.len()
            )
        }
    }

    /// Summary IDs of added devices.
    pub fn added_ids(&self) -> Vec<String> {
        self.added.iter().map(|s| s.id()).collect()
    }

    /// Summary IDs of removed devices.
    pub fn removed_ids(&self) -> Vec<String> {
        self.removed.iter().map(|s| s.id()).collect()
    }

    /// Summary IDs of modified devices.
    pub fn modified_ids(&self) -> Vec<String> {
        self.modified.iter().map(|d| d.id.clone()).collect()
    }

    /// Render this diff as a structured text report with ASCII diff symbols.
    pub fn render_text(&self) -> String {
        format!("{self}")
    }

    /// Serialize the diff to compact JSON string.
    #[cfg(any(feature = "cli", feature = "dbus"))]
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Serialize the diff to pretty-printed JSON string.
    #[cfg(any(feature = "cli", feature = "dbus"))]
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize a `SnapshotDiff` from a JSON string.
    #[cfg(any(feature = "cli", feature = "dbus"))]
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

/// Detailed breakdown of changes observed for a single device present in both snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceDifference {
    /// Stable identifier (e.g. `"typec:port0"`, `"usb:1-1.2"`).
    pub id: String,
    /// Primary headline / product name (e.g. `"USB-C Port 0"`, `"SanDisk Ultra"`).
    pub headline: String,
    /// Subtitle / vendor & class (e.g. `"Anker 737 Charger"`, `"Mass Storage"`).
    pub subtitle: String,

    /// Connection / power flow status transition.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub status: Option<ValueChange<Status>>,

    /// Power negotiation and draw transitions.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub power: Option<PowerDifference>,

    /// USB Power Delivery contract and active PDO profile shifts.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub pd_contract: Option<PdContractDifference>,

    /// Negotiated link speed transition in Mbps.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub link_speed_mbps: Option<ValueChange<u32>>,

    /// Charging bottleneck and health diagnostic transitions.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub diagnostic: Option<DiagnosticDifference>,

    /// Cable rating, trust level, and warning transitions.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cable: Option<CableDifference>,

    /// USB BOS link speed capability versus actual link rate transitions.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub data_rate: Option<DataRateDifference>,

    /// Key-value attribute transitions (`transport.*`, `data_role`, etc.).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub property_changes: Vec<PropertyDifference>,
}

/// Differences in power flow, ceilings, or declared bus draw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PowerDifference {
    /// Power sinking into device in milliwatts.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub power_in_mw: Option<ValueChange<u32>>,

    /// Power sourcing out through port in milliwatts.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub power_out_mw: Option<ValueChange<u32>>,

    /// Negotiated active contract ceiling in milliwatts.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub contract_mw: Option<ValueChange<u32>>,

    /// Operating power role (`Sink`, `Source`, `DualRole`, `Unknown`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub power_role: Option<ValueChange<PowerRole>>,

    /// Plain USB descriptor max power draw in mA (e.g. `500 -> 900`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub usb_max_power_ma: Option<ValueChange<u32>>,
}

impl PowerDifference {
    /// Returns `true` if no power fields changed.
    pub fn is_empty(&self) -> bool {
        self.power_in_mw.is_none()
            && self.power_out_mw.is_none()
            && self.contract_mw.is_none()
            && self.power_role.is_none()
            && self.usb_max_power_ma.is_none()
    }
}

/// Differences in USB-PD negotiated contract and active PDO index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PdContractDifference {
    /// Contract description string (e.g. `"5.0V @ 3.00A" -> "20.0V @ 5.00A"`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub contract_string: Option<ValueChange<String>>,

    /// Zero-based index of active PDO in source capability list.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub active_pdo_index: Option<ValueChange<Option<usize>>>,

    /// Contract voltage in millivolts (e.g. `5000 -> 20000`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub voltage_mv: Option<ValueChange<u32>>,

    /// Contract current in milliamps (e.g. `3000 -> 5000`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub current_ma: Option<ValueChange<u32>>,

    /// Active contract power in milliwatts (e.g. `15000 -> 100000`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_power_mw: Option<ValueChange<u32>>,
}

impl PdContractDifference {
    /// Returns `true` if no contract fields changed.
    pub fn is_empty(&self) -> bool {
        self.contract_string.is_none()
            && self.active_pdo_index.is_none()
            && self.voltage_mv.is_none()
            && self.current_ma.is_none()
            && self.max_power_mw.is_none()
    }
}

/// Differences in charging bottleneck and diagnostic health assessment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DiagnosticDifference {
    /// Overall health status (`Healthy`, `Warning`, `Critical`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub status: Option<ValueChange<DiagnosticStatus>>,

    /// Bottleneck classification (`ChargerLimit`, `CableLimit`, `Fine`, etc.).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub bottleneck: Option<ValueChange<Bottleneck>>,

    /// Warning flag transition (`false -> true` = newly degraded; `true -> false` = resolved).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub is_warning: Option<ValueChange<bool>>,

    /// Subsystem root-cause category (`Cable`, `Charger`, `Sink`, `Port`, `Firmware`, `None`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub root_cause: Option<ValueChange<RootCauseCategory>>,

    /// Diagnostic explanation text.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub summary: Option<ValueChange<String>>,

    /// Plain-English recommended action.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub recommended_action: Option<ValueChange<Option<String>>>,
}

impl DiagnosticDifference {
    /// Returns `true` if no diagnostic fields changed.
    pub fn is_empty(&self) -> bool {
        self.status.is_none()
            && self.bottleneck.is_none()
            && self.is_warning.is_none()
            && self.root_cause.is_none()
            && self.summary.is_none()
            && self.recommended_action.is_none()
    }

    /// `true` if a new warning was raised.
    pub fn is_newly_degraded(&self) -> bool {
        self.is_warning == Some(ValueChange::new(false, true))
    }

    /// `true` if an existing warning was resolved.
    pub fn is_resolved(&self) -> bool {
        self.is_warning == Some(ValueChange::new(true, false))
    }
}

/// Differences in attached cable capabilities and trust grading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CableDifference {
    /// Categorical trust grading (`KnownGood`, `LikelyConstrained`, `Inconclusive`, `Suspicious`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub trust_level: Option<ValueChange<CableTrustLevel>>,

    /// Flag indicating suspicious markers (zero VID, reserved bits, contract mismatch).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub is_suspicious: Option<ValueChange<bool>>,

    /// Maximum wattage rating (e.g. `60 -> 100`, or `None -> Some(100)`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_watts: Option<ValueChange<Option<u32>>>,

    /// Maximum current rating (`ThreeAmp`, `FiveAmp`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub current_rating: Option<ValueChange<Option<CableCurrent>>>,

    /// High-speed data rating.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub speed: Option<ValueChange<Option<CableSpeed>>>,

    /// Trust notes newly added in `after`.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub notes_added: Vec<String>,
    /// Trust notes cleared in `after`.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub notes_removed: Vec<String>,
}

impl CableDifference {
    /// Returns `true` if no cable fields changed.
    pub fn is_empty(&self) -> bool {
        self.trust_level.is_none()
            && self.is_suspicious.is_none()
            && self.max_watts.is_none()
            && self.current_rating.is_none()
            && self.speed.is_none()
            && self.notes_added.is_empty()
            && self.notes_removed.is_empty()
    }
}

/// Differences in device BOS capability versus negotiated link rate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DataRateDifference {
    /// Assessment verdict (`Optimal`, `CapabilityDegraded`, `SuppressedByQuirk`, `Unknown`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub verdict: Option<ValueChange<DataRateVerdict>>,
    /// Shortfall warning flag transition.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub is_warning: Option<ValueChange<bool>>,
}

impl DataRateDifference {
    /// Returns `true` if no data rate fields changed.
    pub fn is_empty(&self) -> bool {
        self.verdict.is_none() && self.is_warning.is_none()
    }
}

/// A named property transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropertyDifference {
    /// Machine property key (e.g. `"transport.usb4"`, `"data_role"`).
    pub key: String,
    /// Property value transition (`None` if absent).
    pub change: ValueChange<Option<String>>,
}

const CURATED_PROPERTY_KEYS: [&str; 7] = [
    "pd_contract",
    "data_role",
    "power_mode",
    "usb_link_verdict",
    "port.peer_state",
    "power.source",
    "kernel.quirks",
];

/// Compare two system snapshots and return their semantic difference.
///
/// Compares `after` relative to `before`. Identifies newly arrived devices (`added`),
/// removed devices (`removed`), and detailed state/power/cable/diagnostic shifts (`modified`).
/// All resulting collections are deterministically sorted by device `id`.
///
/// Comparison is summary-driven: callers must supply canonical `summaries`
/// (including embedded PD/cable assessments). Top-level raw telemetry and metadata
/// are not independently diffed or normalized; raw-only snapshots compare empty.
pub fn compare(before: &Snapshot, after: &Snapshot) -> SnapshotDiff {
    let before_map: HashMap<String, &DeviceSummary> =
        before.summaries.iter().map(|s| (s.id(), s)).collect();
    let after_map: HashMap<String, &DeviceSummary> =
        after.summaries.iter().map(|s| (s.id(), s)).collect();

    // 1. Newly added devices (in after, not in before)
    let mut added: Vec<DeviceSummary> = after
        .summaries
        .iter()
        .filter(|s| !before_map.contains_key(&s.id()))
        .cloned()
        .collect();
    added.sort_by_key(|a| a.id());

    // 2. Removed devices (in before, not in after)
    let mut removed: Vec<DeviceSummary> = before
        .summaries
        .iter()
        .filter(|s| !after_map.contains_key(&s.id()))
        .cloned()
        .collect();
    removed.sort_by_key(|a| a.id());

    // 3. Modified devices (present in both)
    let mut modified: Vec<DeviceDifference> = Vec::new();
    for cur in &after.summaries {
        let id = cur.id();
        if let Some(prev) = before_map.get(&id) {
            if let Some(diff) = diff_device(prev, cur) {
                modified.push(diff);
            }
        }
    }
    modified.sort_by(|a, b| a.id.cmp(&b.id));

    SnapshotDiff {
        added,
        removed,
        modified,
    }
}

fn diff_device(prev: &DeviceSummary, cur: &DeviceSummary) -> Option<DeviceDifference> {
    // 1. Status change
    let status = if prev.status != cur.status {
        Some(ValueChange::new(prev.status, cur.status))
    } else {
        None
    };

    // 2. Power differences
    let mut power_diff = PowerDifference::default();
    if prev.power.power_in_mw != cur.power.power_in_mw {
        power_diff.power_in_mw = Some(ValueChange::new(
            prev.power.power_in_mw,
            cur.power.power_in_mw,
        ));
    }
    if prev.power.power_out_mw != cur.power.power_out_mw {
        power_diff.power_out_mw = Some(ValueChange::new(
            prev.power.power_out_mw,
            cur.power.power_out_mw,
        ));
    }
    if prev.power.contract_mw != cur.power.contract_mw {
        power_diff.contract_mw = Some(ValueChange::new(
            prev.power.contract_mw,
            cur.power.contract_mw,
        ));
    }
    if prev.power.power_role != cur.power.power_role {
        power_diff.power_role = Some(ValueChange::new(
            prev.power.power_role,
            cur.power.power_role,
        ));
    }
    let prev_bus_ma = get_u32_prop(prev, "usb_max_power_ma");
    let cur_bus_ma = get_u32_prop(cur, "usb_max_power_ma");
    if prev_bus_ma != cur_bus_ma {
        if let (Some(p), Some(c)) = (prev_bus_ma, cur_bus_ma) {
            power_diff.usb_max_power_ma = Some(ValueChange::new(p, c));
        }
    }
    let power = if power_diff.is_empty() {
        None
    } else {
        Some(power_diff)
    };

    // 3. PD contract differences
    let mut pd_diff = PdContractDifference::default();
    let prev_contract = get_str_prop(prev, "pd_contract");
    let cur_contract = get_str_prop(cur, "pd_contract");
    if prev_contract != cur_contract {
        if let (Some(p), Some(c)) = (prev_contract, cur_contract) {
            pd_diff.contract_string = Some(ValueChange::new(p, c));
        }
    }
    let prev_active_idx = get_active_pdo_index(prev);
    let cur_active_idx = get_active_pdo_index(cur);
    if prev_active_idx != cur_active_idx {
        pd_diff.active_pdo_index = Some(ValueChange::new(prev_active_idx, cur_active_idx));
    }
    let (prev_v, prev_i, prev_w) = get_active_pdo_metrics(prev);
    let (cur_v, cur_i, cur_w) = get_active_pdo_metrics(cur);
    if prev_v != cur_v && (prev_v > 0 || cur_v > 0) {
        pd_diff.voltage_mv = Some(ValueChange::new(prev_v, cur_v));
    }
    if prev_i != cur_i && (prev_i > 0 || cur_i > 0) {
        pd_diff.current_ma = Some(ValueChange::new(prev_i, cur_i));
    }
    if prev_w != cur_w && (prev_w > 0 || cur_w > 0) {
        pd_diff.max_power_mw = Some(ValueChange::new(prev_w, cur_w));
    }
    let pd_contract = if pd_diff.is_empty() {
        None
    } else {
        Some(pd_diff)
    };

    // 4. Link speed
    let link_speed_mbps = if prev.link_speed_mbps != cur.link_speed_mbps {
        Some(ValueChange::new(prev.link_speed_mbps, cur.link_speed_mbps))
    } else {
        None
    };

    // 5. Diagnostic difference
    let mut diag_diff = DiagnosticDifference::default();
    let prev_diag = prev.charging_diag.as_ref();
    let cur_diag = cur.charging_diag.as_ref();
    match (prev_diag, cur_diag) {
        (Some(p), Some(c)) => {
            if p.status != c.status {
                diag_diff.status = Some(ValueChange::new(p.status, c.status));
            }
            if p.bottleneck != c.bottleneck {
                diag_diff.bottleneck = Some(ValueChange::new(p.bottleneck, c.bottleneck));
            }
            if p.is_warning != c.is_warning {
                diag_diff.is_warning = Some(ValueChange::new(p.is_warning, c.is_warning));
            }
            if p.root_cause != c.root_cause {
                diag_diff.root_cause = Some(ValueChange::new(p.root_cause, c.root_cause));
            }
            if p.summary != c.summary {
                diag_diff.summary = Some(ValueChange::new(p.summary.clone(), c.summary.clone()));
            }
            if p.recommended_action != c.recommended_action {
                let p_act = if p.recommended_action.is_empty() {
                    None
                } else {
                    Some(p.recommended_action.clone())
                };
                let c_act = if c.recommended_action.is_empty() {
                    None
                } else {
                    Some(c.recommended_action.clone())
                };
                diag_diff.recommended_action = Some(ValueChange::new(p_act, c_act));
            }
        }
        (None, Some(c)) => {
            diag_diff.status = Some(ValueChange::new(DiagnosticStatus::Healthy, c.status));
            diag_diff.bottleneck = Some(ValueChange::new(Bottleneck::Fine, c.bottleneck));
            diag_diff.is_warning = Some(ValueChange::new(false, c.is_warning));
            diag_diff.root_cause = Some(ValueChange::new(RootCauseCategory::None, c.root_cause));
            diag_diff.summary = Some(ValueChange::new(String::new(), c.summary.clone()));
            let c_act = if c.recommended_action.is_empty() {
                None
            } else {
                Some(c.recommended_action.clone())
            };
            diag_diff.recommended_action = Some(ValueChange::new(None, c_act));
        }
        (Some(p), None) => {
            diag_diff.status = Some(ValueChange::new(p.status, DiagnosticStatus::Healthy));
            diag_diff.is_warning = Some(ValueChange::new(p.is_warning, false));
        }
        (None, None) => {}
    }
    let diagnostic = if diag_diff.is_empty() {
        None
    } else {
        Some(diag_diff)
    };

    // 6. Cable differences
    let mut cable_diff = CableDifference::default();
    let prev_c = prev.cable.as_ref();
    let cur_c = cur.cable.as_ref();
    match (prev_c, cur_c) {
        (Some(p), Some(c)) => {
            if p.trust.trust_level != c.trust.trust_level {
                cable_diff.trust_level =
                    Some(ValueChange::new(p.trust.trust_level, c.trust.trust_level));
            }
            if p.trust.is_suspicious() != c.trust.is_suspicious() {
                cable_diff.is_suspicious = Some(ValueChange::new(
                    p.trust.is_suspicious(),
                    c.trust.is_suspicious(),
                ));
            }
            if p.max_watts != c.max_watts {
                cable_diff.max_watts = Some(ValueChange::new(Some(p.max_watts), Some(c.max_watts)));
            }
            if p.current_rating != c.current_rating {
                cable_diff.current_rating =
                    Some(ValueChange::new(p.current_rating, c.current_rating));
            }
            if p.speed != c.speed {
                cable_diff.speed = Some(ValueChange::new(p.speed, c.speed));
            }
            cable_diff.notes_added = c
                .trust
                .notes
                .iter()
                .filter(|n| !p.trust.notes.contains(n))
                .cloned()
                .collect();
            cable_diff.notes_removed = p
                .trust
                .notes
                .iter()
                .filter(|n| !c.trust.notes.contains(n))
                .cloned()
                .collect();
        }
        (None, Some(c)) => {
            cable_diff.trust_level = Some(ValueChange::new(
                CableTrustLevel::Inconclusive,
                c.trust.trust_level,
            ));
            cable_diff.is_suspicious = Some(ValueChange::new(false, c.trust.is_suspicious()));
            cable_diff.max_watts = Some(ValueChange::new(None, Some(c.max_watts)));
            cable_diff.current_rating = Some(ValueChange::new(None, c.current_rating));
            cable_diff.speed = Some(ValueChange::new(None, c.speed));
            cable_diff.notes_added = c.trust.notes.clone();
        }
        (Some(p), None) => {
            cable_diff.trust_level = Some(ValueChange::new(
                p.trust.trust_level,
                CableTrustLevel::Inconclusive,
            ));
            cable_diff.is_suspicious = Some(ValueChange::new(p.trust.is_suspicious(), false));
            cable_diff.max_watts = Some(ValueChange::new(Some(p.max_watts), None));
            cable_diff.current_rating = Some(ValueChange::new(p.current_rating, None));
            cable_diff.speed = Some(ValueChange::new(p.speed, None));
        }
        (None, None) => {}
    }
    let cable = if cable_diff.is_empty() {
        None
    } else {
        Some(cable_diff)
    };

    // 7. Data rate differences
    let mut dr_diff = DataRateDifference::default();
    let prev_dr = prev.data_rate.as_ref();
    let cur_dr = cur.data_rate.as_ref();
    match (prev_dr, cur_dr) {
        (Some(p), Some(c)) => {
            if p.verdict != c.verdict {
                dr_diff.verdict = Some(ValueChange::new(p.verdict, c.verdict));
            }
            if (p.verdict == DataRateVerdict::Degraded) != (c.verdict == DataRateVerdict::Degraded)
            {
                dr_diff.is_warning = Some(ValueChange::new(
                    p.verdict == DataRateVerdict::Degraded,
                    c.verdict == DataRateVerdict::Degraded,
                ));
            }
        }
        (None, Some(c)) => {
            dr_diff.verdict = Some(ValueChange::new(DataRateVerdict::Unknown, c.verdict));
            dr_diff.is_warning = Some(ValueChange::new(
                false,
                c.verdict == DataRateVerdict::Degraded,
            ));
        }
        (Some(p), None) => {
            dr_diff.is_warning = Some(ValueChange::new(
                p.verdict == DataRateVerdict::Degraded,
                false,
            ));
        }
        (None, None) => {}
    }
    let data_rate = if dr_diff.is_empty() {
        None
    } else {
        Some(dr_diff)
    };

    // 8. Curated Property differences
    let property_changes = diff_properties(prev, cur);

    // If nothing changed, return None
    if status.is_none()
        && power.is_none()
        && pd_contract.is_none()
        && link_speed_mbps.is_none()
        && diagnostic.is_none()
        && cable.is_none()
        && data_rate.is_none()
        && property_changes.is_empty()
    {
        return None;
    }

    Some(DeviceDifference {
        id: cur.id(),
        headline: cur.headline.clone(),
        subtitle: cur.subtitle.clone(),
        status,
        power,
        pd_contract,
        link_speed_mbps,
        diagnostic,
        cable,
        data_rate,
        property_changes,
    })
}

fn diff_properties(prev: &DeviceSummary, cur: &DeviceSummary) -> Vec<PropertyDifference> {
    let mut out = Vec::new();
    let prev_props: HashMap<&str, &str> = prev
        .properties
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let cur_props: HashMap<&str, &str> = cur
        .properties
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    let all_keys: BTreeSet<&str> = prev_props.keys().chain(cur_props.keys()).copied().collect();
    for key in all_keys {
        let is_curated = CURATED_PROPERTY_KEYS.contains(&key)
            || key.starts_with("transport.")
            || key.starts_with("cable.trust.")
            || key.starts_with("usb_altmode");
        if !is_curated {
            continue;
        }

        let p_val = prev_props.get(key).copied();
        let c_val = cur_props.get(key).copied();
        if p_val != c_val {
            out.push(PropertyDifference {
                key: key.to_string(),
                change: ValueChange::new(
                    p_val.map(|s| s.to_string()),
                    c_val.map(|s| s.to_string()),
                ),
            });
        }
    }
    out
}

fn get_str_prop(summary: &DeviceSummary, key: &str) -> Option<String> {
    summary
        .properties
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
}

fn get_u32_prop(summary: &DeviceSummary, key: &str) -> Option<u32> {
    get_str_prop(summary, key).and_then(|s| s.parse().ok())
}

fn get_active_pdo_index(summary: &DeviceSummary) -> Option<usize> {
    summary
        .power_delivery
        .as_ref()
        .and_then(|pd| pd.source_capabilities.iter().position(|p| p.is_active))
}

fn get_active_pdo_metrics(summary: &DeviceSummary) -> (u32, u32, u32) {
    if let Some(pdo) = summary
        .power_delivery
        .as_ref()
        .and_then(|pd| pd.source_capabilities.iter().find(|p| p.is_active))
    {
        (pdo.voltage_mv, pdo.current_ma, pdo.power_mw)
    } else {
        (0, 0, 0)
    }
}

impl fmt::Display for SnapshotDiff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return writeln!(f, "No differences found between snapshots.");
        }

        writeln!(
            f,
            "Snapshot difference: +{} added, -{} removed, ~{} modified",
            self.added.len(),
            self.removed.len(),
            self.modified.len()
        )?;

        // Added Devices
        if !self.added.is_empty() {
            writeln!(f, "\n=== Added Devices (+{}) ===", self.added.len())?;
            for dev in &self.added {
                writeln!(f, "  + [{}] {} ({})", dev.id(), dev.headline, dev.subtitle)?;
                if dev.status != Status::Empty && dev.status != Status::Connected {
                    writeln!(f, "      Status: {:?}", dev.status)?;
                }
                if dev.power.power_in_mw > 0 {
                    writeln!(f, "      Requested Ceiling: {} mW", dev.power.power_in_mw)?;
                }
                if let Some(prop) = dev.properties.iter().find(|(k, _)| k == "pd_contract") {
                    writeln!(f, "      PD Contract: {}", prop.1)?;
                }
                if dev.link_speed_mbps > 0 {
                    writeln!(f, "      Link Speed: {} Mbps", dev.link_speed_mbps)?;
                }
            }
        }

        // Removed Devices
        if !self.removed.is_empty() {
            writeln!(f, "\n=== Removed Devices (-{}) ===", self.removed.len())?;
            for dev in &self.removed {
                writeln!(f, "  - [{}] {} ({})", dev.id(), dev.headline, dev.subtitle)?;
            }
        }

        // Modified Devices
        if !self.modified.is_empty() {
            writeln!(f, "\n=== Modified Devices (~{}) ===", self.modified.len())?;
            for dev in &self.modified {
                writeln!(f, "  ~ [{}] {}", dev.id, dev.headline)?;
                if let Some(s) = &dev.status {
                    writeln!(f, "      Status: {:?} -> {:?}", s.from, s.to)?;
                }
                if let Some(p) = &dev.power {
                    if let Some(pin) = &p.power_in_mw {
                        writeln!(f, "      Power In: {} mW -> {} mW", pin.from, pin.to)?;
                    }
                    if let Some(pout) = &p.power_out_mw {
                        writeln!(f, "      Power Out: {} mW -> {} mW", pout.from, pout.to)?;
                    }
                    if let Some(cmw) = &p.contract_mw {
                        writeln!(
                            f,
                            "      Contract Ceiling: {} mW -> {} mW",
                            cmw.from, cmw.to
                        )?;
                    }
                    if let Some(role) = &p.power_role {
                        writeln!(f, "      Power Role: {:?} -> {:?}", role.from, role.to)?;
                    }
                    if let Some(ma) = &p.usb_max_power_ma {
                        writeln!(
                            f,
                            "      Declared Max Bus Allowance: {} mA -> {} mA",
                            ma.from, ma.to
                        )?;
                    }
                }
                if let Some(pd) = &dev.pd_contract {
                    if let Some(index) = &pd.active_pdo_index {
                        writeln!(
                            f,
                            "      Active PDO Index: {:?} -> {:?}",
                            index.from, index.to
                        )?;
                    }
                    if let Some(cs) = &pd.contract_string {
                        writeln!(f, "      PD Contract: {} -> {}", cs.from, cs.to)?;
                    }
                    if let Some(v) = &pd.voltage_mv {
                        writeln!(f, "      Voltage: {} mV -> {} mV", v.from, v.to)?;
                    }
                    if let Some(i) = &pd.current_ma {
                        writeln!(f, "      Current: {} mA -> {} mA", i.from, i.to)?;
                    }
                    if let Some(w) = &pd.max_power_mw {
                        writeln!(f, "      Max Power: {} mW -> {} mW", w.from, w.to)?;
                    }
                }
                if let Some(spd) = &dev.link_speed_mbps {
                    writeln!(f, "      Link Speed: {} Mbps -> {} Mbps", spd.from, spd.to)?;
                }
                if let Some(diag) = &dev.diagnostic {
                    if let Some(cause) = &diag.root_cause {
                        writeln!(f, "      Root Cause: {:?} -> {:?}", cause.from, cause.to)?;
                    }
                    if let Some(summary) = &diag.summary {
                        writeln!(
                            f,
                            "      Diagnostic Summary: {} -> {}",
                            summary.from, summary.to
                        )?;
                    }
                    if let Some(st) = &diag.status {
                        writeln!(f, "      Diagnostic Status: {:?} -> {:?}", st.from, st.to)?;
                    }
                    if let Some(bn) = &diag.bottleneck {
                        writeln!(f, "      Bottleneck: {:?} -> {:?}", bn.from, bn.to)?;
                    }
                    if let Some(w) = &diag.is_warning {
                        if w.to && !w.from {
                            writeln!(f, "      Warning: Newly degraded!")?;
                        } else if !w.to && w.from {
                            writeln!(f, "      Warning: Resolved!")?;
                        }
                    }
                    if let Some(act) = &diag.recommended_action {
                        writeln!(
                            f,
                            "      Recommended Action: {} -> {}",
                            act.from.as_deref().unwrap_or("[absent]"),
                            act.to.as_deref().unwrap_or("[absent]")
                        )?;
                    }
                }
                if let Some(c) = &dev.cable {
                    if let Some(suspicious) = &c.is_suspicious {
                        writeln!(
                            f,
                            "      Cable Suspicious: {} -> {}",
                            suspicious.from, suspicious.to
                        )?;
                    }
                    if let Some(current) = &c.current_rating {
                        writeln!(
                            f,
                            "      Cable Current Rating: {:?} -> {:?}",
                            current.from, current.to
                        )?;
                    }
                    if let Some(speed) = &c.speed {
                        writeln!(f, "      Cable Speed: {:?} -> {:?}", speed.from, speed.to)?;
                    }
                    if let Some(tl) = &c.trust_level {
                        writeln!(f, "      Cable Trust: {:?} -> {:?}", tl.from, tl.to)?;
                    }
                    if let Some(mw) = &c.max_watts {
                        let from_str = mw
                            .from
                            .map(|w| format!("{w}W"))
                            .unwrap_or_else(|| "none".into());
                        let to_str = mw
                            .to
                            .map(|w| format!("{w}W"))
                            .unwrap_or_else(|| "none".into());
                        writeln!(f, "      Cable Rating: {} -> {}", from_str, to_str)?;
                    }
                    for note in &c.notes_added {
                        writeln!(f, "      Cable Warning Raised: {}", note)?;
                    }
                    for note in &c.notes_removed {
                        writeln!(f, "      Cable Warning Cleared: {}", note)?;
                    }
                }
                if let Some(rate) = &dev.data_rate {
                    if let Some(verdict) = &rate.verdict {
                        writeln!(
                            f,
                            "      Data Rate Verdict: {:?} -> {:?}",
                            verdict.from, verdict.to
                        )?;
                    }
                    if let Some(warning) = &rate.is_warning {
                        writeln!(
                            f,
                            "      Data Rate Warning: {} -> {}",
                            warning.from, warning.to
                        )?;
                    }
                }
                for prop in &dev.property_changes {
                    let from_str = prop.change.from.as_deref().unwrap_or("[absent]");
                    let to_str = prop.change.to.as_deref().unwrap_or("[absent]");
                    writeln!(f, "      Property {}: {} -> {}", prop.key, from_str, to_str)?;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summary::{Category, DeviceClass, DeviceSummary, PowerSummary, Status};
    use crate::usb::UsbDevice;

    fn make_summary(id: &str, speed: u32) -> DeviceSummary {
        DeviceSummary {
            category: Category::UsbDevice,
            device_class: DeviceClass::Storage,
            device_subclass: String::new(),
            status: Status::Connected,
            headline: format!("Device {id}"),
            subtitle: "Test Subtitle".into(),
            icon: "drive-removable-media".into(),
            vendor: "Vendor".into(),
            product: "Product".into(),
            vendor_id: 0x1234,
            product_id: 0x5678,
            primary_driver: "usb-storage".into(),
            properties: vec![("data_role".into(), "device".into())],
            link_speed_mbps: speed,
            usb_version: "3.2".into(),
            power: PowerSummary::default(),
            charging_diag: None,
            data_rate: None,
            usb_device: Some(UsbDevice {
                bus_port: id.into(),
                ..Default::default()
            }),
            typec_port: None,
            power_delivery: None,
            cable: None,
        }
    }

    #[test]
    fn test_compare_empty_snapshots() {
        let snap1 = Snapshot::new();
        let snap2 = Snapshot::new();
        let diff = compare(&snap1, &snap2);
        assert!(diff.is_empty());
        assert_eq!(diff.total_changes(), 0);
        assert_eq!(diff.summary_string(), "No differences");
    }

    #[test]
    fn test_compare_added_removed_modified() {
        let mut snap1 = Snapshot::new();
        snap1.summaries = vec![make_summary("1-1", 480), make_summary("1-2", 5000)];

        let mut snap2 = Snapshot::new();
        // 1-1 modified (speed 480 -> 10000), 1-2 removed, 1-3 added
        snap2.summaries = vec![make_summary("1-1", 10000), make_summary("1-3", 12)];

        let diff = compare(&snap1, &snap2);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0].id(), "usb:1-3");
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.removed[0].id(), "usb:1-2");
        assert_eq!(diff.modified.len(), 1);
        assert_eq!(diff.modified[0].id, "usb:1-1");
        assert_eq!(
            diff.modified[0].link_speed_mbps,
            Some(ValueChange::new(480, 10000))
        );

        let text = diff.render_text();
        assert!(text.contains("+ [usb:1-3]"));
        assert!(text.contains("- [usb:1-2]"));
        assert!(text.contains("~ [usb:1-1]"));
        assert!(text.contains("480 Mbps -> 10000 Mbps"));
    }

    #[test]
    fn test_compare_json_roundtrip() {
        let snap1 = Snapshot {
            summaries: vec![make_summary("1-1", 480)],
            ..Default::default()
        };
        let snap2 = Snapshot {
            summaries: vec![make_summary("1-1", 5000), make_summary("1-2", 480)],
            ..Default::default()
        };
        let diff = compare(&snap1, &snap2);
        let json = serde_json::to_string(&diff).expect("serialize diff");
        let parsed: SnapshotDiff = serde_json::from_str(&json).expect("deserialize diff");
        assert_eq!(diff, parsed);
    }
}
