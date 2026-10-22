//! Sequential snapshot history and state-transition event tracking.
//!
//! Provides [`SnapshotHistory`], an in-memory bounded ring buffer that captures
//! sequential system snapshots over time, tracks discrete state changes, and
//! emits strongly typed [`StateTransition`] events for device hotplug, power
//! renegotiation, link speed shifts, charging bottlenecks, and cable warnings.
//!
//! # Architecture & Zero-IO
//!
//! This module is part of `usbeehive`'s pure-core layer (`default-features = false`).
//! It performs zero filesystem, netlink, or D-Bus I/O. Snapshots are provided
//! by callers (e.g. from `DeviceManager` or deserialized files).
//! Transition detection, like [`crate::compare::compare`], is summary-driven:
//! provide canonical summaries, not only top-level raw telemetry. No implicit
//! raw-state normalization is performed.
//!
//! # Quick start
//!
//! ```no_run
//! use usbeehive::history::SnapshotHistory;
//! use usbeehive::Snapshot;
//!
//! let mut history = SnapshotHistory::with_capacity(100);
//!
//! // Ingest initial baseline snapshot
//! let initial_snap = Snapshot::default();
//! history.push(initial_snap, 1_000);
//!
//! // Ingest subsequent snapshot after plug event
//! let updated_snap = Snapshot::default();
//! let transitions = history.push(updated_snap, 2_000);
//!
//! for transition in transitions {
//!     println!("{transition}");
//! }
//! ```

use std::collections::{HashMap, VecDeque};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::bos::DataRateVerdict;
use crate::compare::SnapshotDiff;
use crate::diagnostic::{Bottleneck, DiagnosticStatus, RootCauseCategory};
use crate::power::PdoType;
use crate::snapshot::Snapshot;
use crate::summary::{Category, DeviceClass, DeviceSummary, PowerRole, Status};
use crate::trust::CableTrustLevel;

/// Default capacity for [`SnapshotHistory`].
pub const DEFAULT_HISTORY_CAPACITY: usize = 100;

/// Bounded in-memory ring buffer tracking sequential snapshots over time.
///
/// Deserialization preserves the wire layout, clamps capacity to at least one,
/// retains only the newest entries within that bound, and raises `total_pushed`
/// to cover imported entry counts and sequence IDs (saturating at `u64::MAX`).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SnapshotHistory {
    /// Maximum number of snapshot entries retained.
    capacity: usize,
    /// Total number of snapshots pushed since initialization (survives eviction).
    total_pushed: u64,
    /// Retained historical entries.
    entries: VecDeque<HistoryEntry>,
}

impl<'de> Deserialize<'de> for SnapshotHistory {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct ImportedHistory {
            capacity: usize,
            total_pushed: u64,
            entries: VecDeque<HistoryEntry>,
        }
        let imported = ImportedHistory::deserialize(deserializer)?;
        let mut history = Self {
            capacity: imported.capacity.max(1),
            total_pushed: imported.total_pushed.max(imported.entries.len() as u64),
            entries: imported.entries,
        };
        for entry in &history.entries {
            history.total_pushed = history.total_pushed.max(entry.sequence.saturating_add(1));
        }
        history.set_capacity(history.capacity);
        Ok(history)
    }
}

impl Default for SnapshotHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl SnapshotHistory {
    /// Create a new history ring buffer with the default capacity (100).
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_HISTORY_CAPACITY)
    }

    /// Create a history ring buffer with a custom maximum capacity.
    ///
    /// Capacity is clamped to a minimum of 1.
    pub fn with_capacity(capacity: usize) -> Self {
        let cap = capacity.max(1);
        SnapshotHistory {
            capacity: cap,
            total_pushed: 0,
            entries: VecDeque::with_capacity(cap),
        }
    }

    /// Maximum number of historical snapshots this buffer will hold.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Update the buffer capacity. If the new capacity is smaller than `len()`,
    /// oldest entries are evicted until `len() == new_capacity`.
    pub fn set_capacity(&mut self, new_capacity: usize) {
        self.capacity = new_capacity.max(1);
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
    }

    /// Number of snapshots currently in the ring buffer.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` when no snapshots have been recorded yet.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Total count of snapshots pushed to this history buffer over its lifetime,
    /// including entries that have since been evicted.
    pub fn total_pushed(&self) -> u64 {
        self.total_pushed
    }

    /// Record a snapshot at the specified timestamp in milliseconds.
    ///
    /// If this is the first snapshot pushed, it establishes the baseline and
    /// records zero transitions. Subsequent pushes compute discrete
    /// [`StateTransition`] events relative to the immediately preceding snapshot.
    ///
    /// Returns a slice of the newly generated transitions.
    pub fn push(&mut self, snapshot: Snapshot, timestamp_ms: u64) -> &[StateTransition] {
        self.push_with_time(snapshot, timestamp_ms, None)
    }

    /// Record a snapshot with both millisecond timestamp and an optional human-readable
    /// time string (e.g. ISO 8601).
    pub fn push_with_time(
        &mut self,
        snapshot: Snapshot,
        timestamp_ms: u64,
        timestamp_str: Option<String>,
    ) -> &[StateTransition] {
        let transitions = if let Some(prev) = self.entries.back() {
            detect_transitions(&prev.snapshot, &snapshot)
        } else {
            Vec::new()
        };

        let sequence = self.total_pushed;
        self.total_pushed = self.total_pushed.saturating_add(1);

        while self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }

        let entry = HistoryEntry {
            sequence,
            timestamp_ms,
            timestamp_str,
            snapshot,
            transitions,
        };

        self.entries.push_back(entry);
        &self.entries.back().unwrap().transitions
    }

    /// Most recently pushed history entry.
    pub fn latest(&self) -> Option<&HistoryEntry> {
        self.entries.back()
    }

    /// Most recently pushed snapshot.
    pub fn latest_snapshot(&self) -> Option<&Snapshot> {
        self.entries.back().map(|e| &e.snapshot)
    }

    /// Oldest history entry currently in the buffer.
    pub fn oldest(&self) -> Option<&HistoryEntry> {
        self.entries.front()
    }

    /// Get history entry by buffer index (`0` = oldest retained, `len() - 1` = newest).
    pub fn get(&self, index: usize) -> Option<&HistoryEntry> {
        self.entries.get(index)
    }

    /// Look up an entry by its monotonic sequence ID. Returns `None` if the entry
    /// has already been evicted from the ring buffer.
    pub fn get_by_sequence(&self, sequence: u64) -> Option<&HistoryEntry> {
        self.entries.iter().find(|e| e.sequence == sequence)
    }

    /// Iterate over all retained history entries in chronological order (oldest to newest).
    pub fn entries(&self) -> impl Iterator<Item = &HistoryEntry> {
        self.entries.iter()
    }

    /// Collect all state transitions across all retained history entries.
    pub fn all_transitions(&self) -> Vec<&StateTransition> {
        self.entries
            .iter()
            .flat_map(|e| e.transitions.iter())
            .collect()
    }

    /// Collect all transitions that occurred strictly after `since_sequence`.
    pub fn transitions_since(&self, since_sequence: u64) -> Vec<&StateTransition> {
        self.entries
            .iter()
            .filter(|e| e.sequence > since_sequence)
            .flat_map(|e| e.transitions.iter())
            .collect()
    }

    /// Filter retained transitions matching a custom predicate.
    pub fn filter_transitions<F>(&self, predicate: F) -> Vec<&StateTransition>
    where
        F: Fn(&StateTransition) -> bool,
    {
        self.entries
            .iter()
            .flat_map(|e| e.transitions.iter())
            .filter(|t| predicate(t))
            .collect()
    }

    /// Export a flattened timeline of every transition event with associated metadata.
    pub fn timeline(&self) -> Vec<TimelineEntry> {
        let mut timeline = Vec::new();
        for entry in &self.entries {
            for transition in &entry.transitions {
                timeline.push(TimelineEntry {
                    sequence: entry.sequence,
                    timestamp_ms: entry.timestamp_ms,
                    timestamp_str: entry.timestamp_str.clone(),
                    transition: transition.clone(),
                });
            }
        }
        timeline
    }

    /// Difference between the two most recent snapshots in history.
    pub fn diff_latest(&self) -> Option<SnapshotDiff> {
        if self.entries.len() < 2 {
            return None;
        }
        let cur = &self.entries[self.entries.len() - 1].snapshot;
        let prev = &self.entries[self.entries.len() - 2].snapshot;
        Some(crate::compare::compare(prev, cur))
    }

    /// Difference between the snapshot at `from_seq` and the snapshot at `to_seq`.
    pub fn diff_range(&self, from_seq: u64, to_seq: u64) -> Option<SnapshotDiff> {
        let from_snap = &self.get_by_sequence(from_seq)?.snapshot;
        let to_snap = &self.get_by_sequence(to_seq)?.snapshot;
        Some(crate::compare::compare(from_snap, to_snap))
    }

    /// Clear all recorded entries while resetting `total_pushed` to zero.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.total_pushed = 0;
    }

    /// Serialize this history buffer to compact JSON string.
    #[cfg(any(feature = "cli", feature = "dbus"))]
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Serialize this history buffer to pretty-printed JSON string.
    #[cfg(any(feature = "cli", feature = "dbus"))]
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize a `SnapshotHistory` from a JSON string.
    #[cfg(any(feature = "cli", feature = "dbus"))]
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Serialize the transition timeline to JSON.
    #[cfg(any(feature = "cli", feature = "dbus"))]
    pub fn timeline_to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&self.timeline())
    }
}

/// A timestamped snapshot entry inside [`SnapshotHistory`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryEntry {
    /// Monotonic sequence number (0, 1, 2, ...).
    pub sequence: u64,
    /// Timestamp in milliseconds (typically UNIX epoch millis).
    pub timestamp_ms: u64,
    /// Optional human-readable timestamp string (e.g. ISO 8601).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp_str: Option<String>,
    /// System snapshot at this timestamp.
    pub snapshot: Snapshot,
    /// Discrete state transitions detected relative to the previous snapshot.
    pub transitions: Vec<StateTransition>,
}

/// A single item in an exported chronological event timeline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimelineEntry {
    /// Monotonic sequence index of the originating snapshot.
    pub sequence: u64,
    /// Timestamp in milliseconds.
    pub timestamp_ms: u64,
    /// Optional human-readable timestamp string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp_str: Option<String>,
    /// State transition event that occurred.
    pub transition: StateTransition,
}

/// Strongly typed state-transition event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "transition_type", content = "details")]
pub enum StateTransition {
    /// A new USB device or Type-C port summary appeared.
    DeviceConnected(DeviceConnectedEvent),
    /// A USB device or Type-C port summary disappeared.
    DeviceDisconnected(DeviceDisconnectedEvent),
    /// The negotiated link speed of a connected device changed.
    LinkSpeedChanged(LinkSpeedChangedEvent),
    /// Negotiated power, contract wattage, or draw ceiling changed.
    PowerChanged(PowerChangedEvent),
    /// USB Power Delivery contract renegotiated (voltage, current, or PDO index).
    ContractRenegotiated(ContractRenegotiatedEvent),
    /// Charging diagnostic bottleneck or health status changed.
    BottleneckChanged(BottleneckChangedEvent),
    /// Cable trust assessment level or cable warning changed.
    CableWarningChanged(CableWarningChangedEvent),
    /// Type-C port connection status, data role, or power role changed.
    PortRoleChanged(PortRoleChangedEvent),
    /// BOS data-rate verdict changed (e.g. SuperSpeed device running degraded).
    DataRateChanged(DataRateChangedEvent),
}

impl StateTransition {
    /// Return the canonical identifier of the device or port involved in this transition.
    pub fn id(&self) -> &str {
        match self {
            StateTransition::DeviceConnected(e) => &e.id,
            StateTransition::DeviceDisconnected(e) => &e.id,
            StateTransition::LinkSpeedChanged(e) => &e.id,
            StateTransition::PowerChanged(e) => &e.id,
            StateTransition::ContractRenegotiated(e) => &e.id,
            StateTransition::BottleneckChanged(e) => &e.id,
            StateTransition::CableWarningChanged(e) => &e.id,
            StateTransition::PortRoleChanged(e) => &e.id,
            StateTransition::DataRateChanged(e) => &e.id,
        }
    }
}

impl fmt::Display for StateTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateTransition::DeviceConnected(e) => {
                write!(
                    f,
                    "[+] CONNECTED: {} ({} - {:?}, {} Mbps)",
                    e.id, e.headline, e.device_class, e.link_speed_mbps
                )
            }
            StateTransition::DeviceDisconnected(e) => {
                write!(f, "[-] DISCONNECTED: {} ({})", e.id, e.headline)
            }
            StateTransition::LinkSpeedChanged(e) => {
                write!(
                    f,
                    "[~] SPEED CHANGED: {} ({} Mbps -> {} Mbps)",
                    e.id, e.old_speed_mbps, e.new_speed_mbps
                )
            }
            StateTransition::PowerChanged(e) => {
                write!(
                    f,
                    "[~] POWER CHANGED: {} (in: {} mW -> {} mW, contract: {} mW -> {} mW)",
                    e.id,
                    e.old_power_in_mw,
                    e.new_power_in_mw,
                    e.old_contract_mw,
                    e.new_contract_mw
                )
            }
            StateTransition::ContractRenegotiated(e) => {
                write!(
                    f,
                    "[~] CONTRACT RENEGOTIATED: {} ({} mW @ {:.1}V/{}mA -> {} mW @ {:.1}V/{}mA)",
                    e.id,
                    e.old_power_mw,
                    e.old_voltage_mv as f64 / 1000.0,
                    e.old_current_ma,
                    e.new_power_mw,
                    e.new_voltage_mv as f64 / 1000.0,
                    e.new_current_ma,
                )
            }
            StateTransition::BottleneckChanged(e) => {
                if e.warning_raised {
                    write!(
                        f,
                        "[!] BOTTLENECK DEGRADED: {} ({:?} -> {:?}: {})",
                        e.id, e.old_bottleneck, e.new_bottleneck, e.summary
                    )
                } else if e.warning_cleared {
                    write!(
                        f,
                        "[*] BOTTLENECK RESOLVED: {} ({:?} -> {:?})",
                        e.id, e.old_bottleneck, e.new_bottleneck
                    )
                } else {
                    write!(
                        f,
                        "[~] BOTTLENECK CHANGED: {} ({:?} -> {:?})",
                        e.id, e.old_bottleneck, e.new_bottleneck
                    )
                }
            }
            StateTransition::CableWarningChanged(e) => {
                if e.warning_raised {
                    write!(
                        f,
                        "[!] CABLE WARNING RAISED: {} ({:?} -> {:?}: {})",
                        e.id, e.old_trust_level, e.new_trust_level, e.reason
                    )
                } else if e.warning_cleared {
                    write!(
                        f,
                        "[*] CABLE WARNING CLEARED: {} ({:?} -> {:?}: {})",
                        e.id, e.old_trust_level, e.new_trust_level, e.reason
                    )
                } else {
                    write!(
                        f,
                        "[~] CABLE TRUST CHANGED: {} ({:?} -> {:?}: {})",
                        e.id, e.old_trust_level, e.new_trust_level, e.reason
                    )
                }
            }
            StateTransition::PortRoleChanged(e) => {
                write!(
                    f,
                    "[~] PORT ROLE CHANGED: {} (status: {:?} -> {:?}, power: {:?} -> {:?})",
                    e.id, e.old_status, e.new_status, e.old_power_role, e.new_power_role
                )
            }
            StateTransition::DataRateChanged(e) => {
                if e.is_warning {
                    write!(
                        f,
                        "[!] DATA-RATE DEGRADED: {} ({:?} -> {:?})",
                        e.id, e.old_verdict, e.new_verdict
                    )
                } else if e.is_restored {
                    write!(
                        f,
                        "[*] DATA-RATE RESTORED: {} ({:?} -> {:?})",
                        e.id, e.old_verdict, e.new_verdict
                    )
                } else {
                    write!(
                        f,
                        "[~] DATA-RATE VERDICT: {} ({:?} -> {:?})",
                        e.id, e.old_verdict, e.new_verdict
                    )
                }
            }
        }
    }
}

/// Payload for a device connection event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceConnectedEvent {
    /// Summary identifier (`"usb:1-1"`, `"typec:port0"`).
    pub id: String,
    /// Product headline or port label.
    pub headline: String,
    /// Subtitle (vendor + classification).
    pub subtitle: String,
    /// Device category (`UsbDevice`, `TypeCPort`, `Hub`).
    pub category: Category,
    /// Coarse classification.
    pub device_class: DeviceClass,
    /// Vendor ID (0 for Type-C ports).
    pub vendor_id: u16,
    /// Product ID (0 for Type-C ports).
    pub product_id: u16,
    /// Link speed in Mbps.
    pub link_speed_mbps: u32,
    /// Initial port/device status.
    pub status: Status,
}

/// Payload for a device disconnection event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceDisconnectedEvent {
    /// Summary identifier.
    pub id: String,
    /// Product headline at the time of disconnection.
    pub headline: String,
    /// Category.
    pub category: Category,
    /// Device classification.
    pub device_class: DeviceClass,
}

/// Payload for a link speed change event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkSpeedChangedEvent {
    /// Device identifier.
    pub id: String,
    /// Product headline.
    pub headline: String,
    /// Previous negotiated link speed in Mbps.
    pub old_speed_mbps: u32,
    /// New negotiated link speed in Mbps.
    pub new_speed_mbps: u32,
}

/// Payload for a power change event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowerChangedEvent {
    /// Summary identifier.
    pub id: String,
    /// Product headline.
    pub headline: String,
    /// Power role.
    pub power_role: PowerRole,
    /// Previous power in mW.
    pub old_power_in_mw: u32,
    /// New power in mW.
    pub new_power_in_mw: u32,
    /// Previous sourcing power in mW.
    pub old_power_out_mw: u32,
    /// New sourcing power in mW.
    pub new_power_out_mw: u32,
    /// Previous contract ceiling in mW.
    pub old_contract_mw: u32,
    /// New contract ceiling in mW.
    pub new_contract_mw: u32,
}

/// Payload for a USB Power Delivery contract renegotiation event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractRenegotiatedEvent {
    /// Port identifier.
    pub id: String,
    /// Type-C port number if linked.
    pub port_number: Option<i32>,
    /// Previous contract wattage in mW.
    pub old_power_mw: u32,
    /// New contract wattage in mW.
    pub new_power_mw: u32,
    /// Previous contract voltage in mV.
    pub old_voltage_mv: u32,
    /// New contract voltage in mV.
    pub new_voltage_mv: u32,
    /// Previous contract current in mA.
    pub old_current_ma: u32,
    /// New contract current in mA.
    pub new_current_ma: u32,
    /// Previous active PDO index.
    pub old_pdo_index: Option<u32>,
    /// New active PDO index.
    pub new_pdo_index: Option<u32>,
    /// Previous active PDO type.
    pub old_pdo_type: Option<PdoType>,
    /// New active PDO type.
    pub new_pdo_type: Option<PdoType>,
}

/// Payload for a charging bottleneck or diagnostic status change event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BottleneckChangedEvent {
    /// Port identifier.
    pub id: String,
    /// Type-C port number if linked.
    pub port_number: Option<i32>,
    /// Previous bottleneck categorization.
    pub old_bottleneck: Option<Bottleneck>,
    /// New bottleneck categorization.
    pub new_bottleneck: Option<Bottleneck>,
    /// Previous diagnostic health status.
    pub old_status: Option<DiagnosticStatus>,
    /// New diagnostic health status.
    pub new_status: Option<DiagnosticStatus>,
    /// `true` if a new warning was raised.
    pub warning_raised: bool,
    /// `true` if a previous warning cleared.
    pub warning_cleared: bool,
    /// Subsystem responsible for the bottleneck.
    pub root_cause: RootCauseCategory,
    /// Headline summary.
    pub summary: String,
    /// Actionable user recommendation.
    pub recommended_action: Option<String>,
}

/// Payload for a cable warning or trust level transition event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CableWarningChangedEvent {
    /// Port identifier.
    pub id: String,
    /// Port number.
    pub port_number: Option<i32>,
    /// Previous cable trust level.
    pub old_trust_level: Option<CableTrustLevel>,
    /// New cable trust level.
    pub new_trust_level: Option<CableTrustLevel>,
    /// Previous cable max watts.
    pub old_max_watts: Option<u32>,
    /// New cable max watts.
    pub new_max_watts: Option<u32>,
    /// `true` if a new cable anomaly/warning was detected.
    pub warning_raised: bool,
    /// `true` if a previous cable warning cleared.
    pub warning_cleared: bool,
    /// Explanatory message.
    pub reason: String,
}

/// Payload for a port role or status change event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortRoleChangedEvent {
    /// Port identifier.
    pub id: String,
    /// Previous connection status.
    pub old_status: Status,
    /// New connection status.
    pub new_status: Status,
    /// Previous power role.
    pub old_power_role: PowerRole,
    /// New power role.
    pub new_power_role: PowerRole,
    /// Previous data role (`"host"`, `"device"`), if published.
    pub old_data_role: Option<String>,
    /// New data role.
    pub new_data_role: Option<String>,
}

/// Payload for a BOS data-rate assessment transition event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataRateChangedEvent {
    /// Device identifier.
    pub id: String,
    /// Product headline.
    pub headline: String,
    /// Previous BOS rate verdict.
    pub old_verdict: Option<DataRateVerdict>,
    /// New BOS rate verdict.
    pub new_verdict: Option<DataRateVerdict>,
    /// `true` if data rate newly degraded into a warning state.
    pub is_warning: bool,
    /// `true` if data rate warning cleared.
    pub is_restored: bool,
}

/// Compute discrete state transition events between two consecutive snapshots.
pub fn detect_transitions(previous: &Snapshot, current: &Snapshot) -> Vec<StateTransition> {
    let mut transitions = Vec::new();

    let prev_map: HashMap<String, &DeviceSummary> =
        previous.summaries.iter().map(|s| (s.id(), s)).collect();
    let cur_map: HashMap<String, &DeviceSummary> =
        current.summaries.iter().map(|s| (s.id(), s)).collect();

    // 1. Detect arrivals (DeviceConnected) - sorted by id
    let mut arrivals: Vec<_> = cur_map
        .iter()
        .filter(|(id, _)| !prev_map.contains_key(*id))
        .collect();
    arrivals.sort_by_key(|(id, _)| *id);
    for (id, cur_s) in arrivals {
        transitions.push(StateTransition::DeviceConnected(DeviceConnectedEvent {
            id: id.clone(),
            headline: cur_s.headline.clone(),
            subtitle: cur_s.subtitle.clone(),
            category: cur_s.category,
            device_class: cur_s.device_class,
            vendor_id: cur_s.vendor_id,
            product_id: cur_s.product_id,
            link_speed_mbps: cur_s.link_speed_mbps,
            status: cur_s.status,
        }));
    }

    // 2. Detect removals (DeviceDisconnected) - sorted by id
    let mut removals: Vec<_> = prev_map
        .iter()
        .filter(|(id, _)| !cur_map.contains_key(*id))
        .collect();
    removals.sort_by_key(|(id, _)| *id);
    for (id, prev_s) in removals {
        transitions.push(StateTransition::DeviceDisconnected(
            DeviceDisconnectedEvent {
                id: id.clone(),
                headline: prev_s.headline.clone(),
                category: prev_s.category,
                device_class: prev_s.device_class,
            },
        ));
    }

    // 3. Detect property shifts on common devices - sorted by id
    let mut common_ids: Vec<_> = cur_map
        .keys()
        .filter(|id| prev_map.contains_key(*id))
        .collect();
    common_ids.sort();

    for id in common_ids {
        let cur_s = cur_map[id];
        let prev_s = prev_map[id];

        let port_number = cur_s.typec_port.as_ref().map(|p| p.port_number);

        // A. Port connection state & role inversion
        let prev_data_role = get_property(prev_s, "data_role");
        let cur_data_role = get_property(cur_s, "data_role");
        if prev_s.status != cur_s.status
            || prev_s.power.power_role != cur_s.power.power_role
            || prev_data_role != cur_data_role
        {
            transitions.push(StateTransition::PortRoleChanged(PortRoleChangedEvent {
                id: id.clone(),
                old_status: prev_s.status,
                new_status: cur_s.status,
                old_power_role: prev_s.power.power_role,
                new_power_role: cur_s.power.power_role,
                old_data_role: prev_data_role,
                new_data_role: cur_data_role,
            }));
        }

        // B. Link speed shift
        if prev_s.link_speed_mbps != cur_s.link_speed_mbps {
            transitions.push(StateTransition::LinkSpeedChanged(LinkSpeedChangedEvent {
                id: id.clone(),
                headline: cur_s.headline.clone(),
                old_speed_mbps: prev_s.link_speed_mbps,
                new_speed_mbps: cur_s.link_speed_mbps,
            }));
        }

        // C. Power flow change
        if prev_s.power.power_in_mw != cur_s.power.power_in_mw
            || prev_s.power.power_out_mw != cur_s.power.power_out_mw
            || prev_s.power.contract_mw != cur_s.power.contract_mw
        {
            transitions.push(StateTransition::PowerChanged(PowerChangedEvent {
                id: id.clone(),
                headline: cur_s.headline.clone(),
                power_role: cur_s.power.power_role,
                old_power_in_mw: prev_s.power.power_in_mw,
                new_power_in_mw: cur_s.power.power_in_mw,
                old_power_out_mw: prev_s.power.power_out_mw,
                new_power_out_mw: cur_s.power.power_out_mw,
                old_contract_mw: prev_s.power.contract_mw,
                new_contract_mw: cur_s.power.contract_mw,
            }));
        }

        // D. Contract renegotiation
        let prev_active_pdo = prev_s
            .power_delivery
            .as_ref()
            .and_then(|pd| pd.source_capabilities.iter().find(|p| p.is_active));
        let cur_active_pdo = cur_s
            .power_delivery
            .as_ref()
            .and_then(|pd| pd.source_capabilities.iter().find(|p| p.is_active));

        let pdo_changed = match (prev_active_pdo, cur_active_pdo) {
            (Some(p), Some(c)) => {
                p.index != c.index
                    || p.power_mw != c.power_mw
                    || p.voltage_mv != c.voltage_mv
                    || p.current_ma != c.current_ma
                    || p.r#type != c.r#type
            }
            (None, Some(_)) | (Some(_), None) => true,
            (None, None) => {
                cur_s.category == Category::TypeCPort
                    && prev_s.power.contract_mw != cur_s.power.contract_mw
                    && (prev_s.power.contract_mw > 0 || cur_s.power.contract_mw > 0)
            }
        };

        if pdo_changed || get_property(prev_s, "pd_contract") != get_property(cur_s, "pd_contract")
        {
            transitions.push(StateTransition::ContractRenegotiated(
                ContractRenegotiatedEvent {
                    id: id.clone(),
                    port_number,
                    old_power_mw: prev_active_pdo
                        .map(|p| p.power_mw)
                        .unwrap_or(prev_s.power.contract_mw),
                    new_power_mw: cur_active_pdo
                        .map(|p| p.power_mw)
                        .unwrap_or(cur_s.power.contract_mw),
                    old_voltage_mv: prev_active_pdo.map(|p| p.voltage_mv).unwrap_or(0),
                    new_voltage_mv: cur_active_pdo.map(|p| p.voltage_mv).unwrap_or(0),
                    old_current_ma: prev_active_pdo.map(|p| p.current_ma).unwrap_or(0),
                    new_current_ma: cur_active_pdo.map(|p| p.current_ma).unwrap_or(0),
                    old_pdo_index: prev_active_pdo.map(|p| p.index),
                    new_pdo_index: cur_active_pdo.map(|p| p.index),
                    old_pdo_type: prev_active_pdo.map(|p| p.r#type),
                    new_pdo_type: cur_active_pdo.map(|p| p.r#type),
                },
            ));
        }

        // E. Charging diagnostic & bottleneck transitions
        let prev_diag = prev_s.charging_diag.as_ref();
        let cur_diag = cur_s.charging_diag.as_ref();

        let diag_changed = match (prev_diag, cur_diag) {
            (Some(p), Some(c)) => {
                p.bottleneck != c.bottleneck || p.status != c.status || p.is_warning != c.is_warning
            }
            (None, Some(_)) | (Some(_), None) => true,
            (None, None) => false,
        };

        if diag_changed {
            let prev_warn = prev_diag.is_some_and(|d| d.is_warning);
            let cur_warn = cur_diag.is_some_and(|d| d.is_warning);
            let summary = cur_diag
                .map(|d| d.summary.clone())
                .unwrap_or_else(|| "Charging diagnostic cleared".into());
            let action = cur_diag.and_then(|d| {
                if d.recommended_action.is_empty() {
                    None
                } else {
                    Some(d.recommended_action.clone())
                }
            });

            transitions.push(StateTransition::BottleneckChanged(BottleneckChangedEvent {
                id: id.clone(),
                port_number,
                old_bottleneck: prev_diag.map(|d| d.bottleneck),
                new_bottleneck: cur_diag.map(|d| d.bottleneck),
                old_status: prev_diag.map(|d| d.status),
                new_status: cur_diag.map(|d| d.status),
                warning_raised: !prev_warn && cur_warn,
                warning_cleared: prev_warn && !cur_warn,
                root_cause: cur_diag
                    .map(|d| d.root_cause)
                    .unwrap_or(RootCauseCategory::None),
                summary,
                recommended_action: action,
            }));
        }

        // F. Cable warnings & trust shifts
        let prev_cable = prev_s.cable.as_ref();
        let cur_cable = cur_s.cable.as_ref();

        let cable_changed = match (prev_cable, cur_cable) {
            (Some(p), Some(c)) => p.trust != c.trust || p.max_watts != c.max_watts,
            (None, Some(_)) | (Some(_), None) => true,
            (None, None) => false,
        };

        if cable_changed {
            let prev_warn = prev_cable.is_some_and(|c| c.trust.any());
            let cur_warn = cur_cable.is_some_and(|c| c.trust.any());
            let mut reason = if let Some(c) = cur_cable {
                if !c.trust.notes.is_empty() {
                    c.trust.notes.join("; ")
                } else if c.trust.is_suspicious() {
                    "Suspicious cable indicators detected".into()
                } else {
                    format!("Cable rating: {}W", c.max_watts)
                }
            } else {
                "Cable unplugged".into()
            };

            // Preserve evidence changes even when aggregate warning stays true.
            if let (Some(p), Some(c)) = (prev_cable, cur_cable) {
                for (name, old, new) in [
                    ("zero_vid", p.trust.zero_vid, c.trust.zero_vid),
                    ("vid_unknown", p.trust.vid_unknown, c.trust.vid_unknown),
                    (
                        "reserved_bits_set",
                        p.trust.reserved_bits_set,
                        c.trust.reserved_bits_set,
                    ),
                    (
                        "emarker_contract_mismatch",
                        p.trust.emarker_contract_mismatch,
                        c.trust.emarker_contract_mismatch,
                    ),
                    (
                        "unsupported_combination",
                        p.trust.unsupported_combination,
                        c.trust.unsupported_combination,
                    ),
                ] {
                    if old != new {
                        reason.push_str(&format!("; {name}: {old} -> {new}"));
                    }
                }
                for note in &p.trust.notes {
                    if !c.trust.notes.contains(note) {
                        reason.push_str(&format!("; cleared: {note}"));
                    }
                }
            }
            transitions.push(StateTransition::CableWarningChanged(
                CableWarningChangedEvent {
                    id: id.clone(),
                    port_number,
                    old_trust_level: prev_cable.map(|c| c.trust.trust_level),
                    new_trust_level: cur_cable.map(|c| c.trust.trust_level),
                    old_max_watts: prev_cable.map(|c| c.max_watts),
                    new_max_watts: cur_cable.map(|c| c.max_watts),
                    warning_raised: !prev_warn && cur_warn,
                    warning_cleared: prev_warn && !cur_warn,
                    reason,
                },
            ));
        }

        // G. Data-rate assessment shifts
        let prev_rate = prev_s.data_rate.as_ref();
        let cur_rate = cur_s.data_rate.as_ref();

        let rate_changed = match (prev_rate, cur_rate) {
            (Some(p), Some(c)) => {
                p.verdict != c.verdict
                    || (p.verdict == DataRateVerdict::Degraded)
                        != (c.verdict == DataRateVerdict::Degraded)
            }
            (None, Some(_)) | (Some(_), None) => true,
            (None, None) => false,
        };

        if rate_changed {
            let prev_warn = prev_rate.is_some_and(|r| r.verdict == DataRateVerdict::Degraded);
            let cur_warn = cur_rate.is_some_and(|r| r.verdict == DataRateVerdict::Degraded);

            transitions.push(StateTransition::DataRateChanged(DataRateChangedEvent {
                id: id.clone(),
                headline: cur_s.headline.clone(),
                old_verdict: prev_rate.map(|r| r.verdict),
                new_verdict: cur_rate.map(|r| r.verdict),
                is_warning: !prev_warn && cur_warn,
                is_restored: prev_warn && !cur_warn,
            }));
        }
    }

    transitions
}

fn get_property(summary: &DeviceSummary, key: &str) -> Option<String> {
    summary
        .properties
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::{
        Bottleneck, ChargingDiagnostic, DiagnosticConfidence, DiagnosticStatus, RootCauseCategory,
    };
    use crate::summary::{Category, DeviceClass, DeviceSummary, PowerSummary, Status};
    use crate::usb::UsbDevice;

    fn make_test_device(bus_port: &str, speed: u32) -> DeviceSummary {
        DeviceSummary {
            category: Category::UsbDevice,
            device_class: DeviceClass::Storage,
            device_subclass: String::new(),
            status: Status::Connected,
            headline: format!("Storage {bus_port}"),
            subtitle: "SanDisk".into(),
            icon: "drive-removable-media".into(),
            vendor: "SanDisk".into(),
            product: "Ultra".into(),
            vendor_id: 0x0781,
            product_id: 0x5581,
            primary_driver: "usb-storage".into(),
            properties: vec![("data_role".into(), "device".into())],
            link_speed_mbps: speed,
            usb_version: "3.2".into(),
            power: PowerSummary::default(),
            charging_diag: None,
            data_rate: None,
            usb_device: Some(UsbDevice {
                bus_port: bus_port.into(),
                ..Default::default()
            }),
            typec_port: None,
            power_delivery: None,
            cable: None,
        }
    }

    #[test]
    fn ring_buffer_respects_capacity_and_evicts_oldest() {
        let mut history = SnapshotHistory::with_capacity(3);
        assert_eq!(history.capacity(), 3);
        assert_eq!(history.len(), 0);

        for i in 0..5 {
            let snap = Snapshot {
                summaries: vec![make_test_device(&format!("1-{i}"), 5000)],
                ..Default::default()
            };
            history.push(snap, 1000 + i * 100);
        }

        assert_eq!(history.len(), 3);
        assert_eq!(history.total_pushed(), 5);

        // Retained sequence numbers must be 2, 3, 4
        assert_eq!(history.oldest().unwrap().sequence, 2);
        assert_eq!(history.latest().unwrap().sequence, 4);

        let seqs: Vec<u64> = history.entries().map(|e| e.sequence).collect();
        assert_eq!(seqs, vec![2, 3, 4]);
    }

    #[test]
    fn detects_arrival_and_removal_transitions() {
        let mut history = SnapshotHistory::new();

        // Snapshot 1: Device A
        let snap1 = Snapshot {
            summaries: vec![make_test_device("1-1", 5000)],
            ..Default::default()
        };
        let t1 = history.push(snap1, 1000);
        assert!(t1.is_empty(), "Baseline snapshot has no transitions");

        // Snapshot 2: Device A + Device B
        let snap2 = Snapshot {
            summaries: vec![make_test_device("1-1", 5000), make_test_device("1-2", 480)],
            ..Default::default()
        };
        let t2 = history.push(snap2, 2000);
        assert_eq!(t2.len(), 1);
        match &t2[0] {
            StateTransition::DeviceConnected(e) => {
                assert_eq!(e.id, "usb:1-2");
                assert_eq!(e.link_speed_mbps, 480);
            }
            other => panic!("Unexpected transition: {other:?}"),
        }

        // Snapshot 3: Device B only (Device A removed)
        let snap3 = Snapshot {
            summaries: vec![make_test_device("1-2", 480)],
            ..Default::default()
        };
        let t3 = history.push(snap3, 3000);
        assert_eq!(t3.len(), 1);
        match &t3[0] {
            StateTransition::DeviceDisconnected(e) => {
                assert_eq!(e.id, "usb:1-1");
            }
            other => panic!("Unexpected transition: {other:?}"),
        }
    }

    #[test]
    fn detects_link_speed_change() {
        let mut history = SnapshotHistory::new();

        let snap1 = Snapshot {
            summaries: vec![make_test_device("1-1", 480)],
            ..Default::default()
        };
        history.push(snap1, 1000);

        // Same device trains up to 5000 Mbps
        let snap2 = Snapshot {
            summaries: vec![make_test_device("1-1", 5000)],
            ..Default::default()
        };
        let t2 = history.push(snap2, 2000);
        assert_eq!(t2.len(), 1);
        match &t2[0] {
            StateTransition::LinkSpeedChanged(e) => {
                assert_eq!(e.id, "usb:1-1");
                assert_eq!(e.old_speed_mbps, 480);
                assert_eq!(e.new_speed_mbps, 5000);
            }
            other => panic!("Unexpected transition: {other:?}"),
        }
    }

    #[test]
    fn detects_bottleneck_degradation_and_resolution() {
        let mut history = SnapshotHistory::new();

        let mut dev1 = make_test_device("1-1", 5000);
        dev1.charging_diag = Some(ChargingDiagnostic {
            bottleneck: Bottleneck::Fine,
            summary: "Charging at maximum".into(),
            detail: String::new(),
            is_warning: false,
            status: DiagnosticStatus::Healthy,
            root_cause: RootCauseCategory::None,
            confidence: DiagnosticConfidence::High,
            recommended_action: String::new(),
            rule_id: "fine",
        });

        history.push(
            Snapshot {
                summaries: vec![dev1.clone()],
                ..Default::default()
            },
            1000,
        );

        // Degrade to CableLimit
        let mut dev2 = dev1.clone();
        dev2.charging_diag = Some(ChargingDiagnostic {
            bottleneck: Bottleneck::CableLimit,
            summary: "Cable limiting charging speed".into(),
            detail: "Cable rated for 60W but charger delivers 100W".into(),
            is_warning: true,
            status: DiagnosticStatus::Warning,
            root_cause: RootCauseCategory::Cable,
            confidence: DiagnosticConfidence::High,
            recommended_action: "Replace cable with 100W or 240W EPR cable".into(),
            rule_id: "cable_limit",
        });

        let t2 = history.push(
            Snapshot {
                summaries: vec![dev2.clone()],
                ..Default::default()
            },
            2000,
        );
        assert_eq!(t2.len(), 1);
        match &t2[0] {
            StateTransition::BottleneckChanged(e) => {
                assert!(e.warning_raised);
                assert!(!e.warning_cleared);
                assert_eq!(e.new_bottleneck, Some(Bottleneck::CableLimit));
                assert_eq!(e.root_cause, RootCauseCategory::Cable);
            }
            other => panic!("Unexpected transition: {other:?}"),
        }

        // Resolve back to Fine
        let t3 = history.push(
            Snapshot {
                summaries: vec![dev1],
                ..Default::default()
            },
            3000,
        );
        assert_eq!(t3.len(), 1);
        match &t3[0] {
            StateTransition::BottleneckChanged(e) => {
                assert!(!e.warning_raised);
                assert!(e.warning_cleared);
                assert_eq!(e.new_bottleneck, Some(Bottleneck::Fine));
            }
            other => panic!("Unexpected transition: {other:?}"),
        }
    }

    #[test]
    fn timeline_exports_all_events_in_order() {
        let mut history = SnapshotHistory::new();
        history.push(
            Snapshot {
                summaries: vec![make_test_device("1-1", 480)],
                ..Default::default()
            },
            1000,
        );
        history.push(
            Snapshot {
                summaries: vec![make_test_device("1-1", 5000), make_test_device("1-2", 12)],
                ..Default::default()
            },
            2000,
        );

        let timeline = history.timeline();
        assert_eq!(timeline.len(), 2);
        assert_eq!(timeline[0].sequence, 1);
        assert_eq!(timeline[0].timestamp_ms, 2000);
    }
}
