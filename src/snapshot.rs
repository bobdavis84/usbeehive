//! Pure-core, IO-free snapshot data model.
//!
//! [`Snapshot`] represents a point-in-time view of all discovered USB devices,
//! Type-C ports, Power Delivery ports, Thunderbolt/USB4 routers, and aggregate
//! device summaries.
//!
//! Decoupled entirely from sysfs I/O: safe for use in offline tooling, wasm,
//! network serialization, and pure-computation pipelines under `default-features = false`.

use serde::{Deserialize, Serialize};

use crate::power::PowerDeliveryPort;
use crate::summary::DeviceSummary;
use crate::thunderbolt::{ThunderboltDomain, ThunderboltRouter};
use crate::typec::TypeCPort;
use crate::usb::UsbDevice;

/// Contextual metadata for a [`Snapshot`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotMetadata {
    /// Timestamp in milliseconds since UNIX epoch when the snapshot was captured.
    #[serde(default)]
    pub captured_at_ms: u64,

    /// Monotonic capture sequence number, useful for ring-buffer state history.
    #[serde(default)]
    pub sequence: u64,

    /// Machine identifier, hostname, or data source where the snapshot was captured.
    #[serde(default)]
    pub source: String,
}

impl SnapshotMetadata {
    /// Create new snapshot metadata.
    pub fn new(captured_at_ms: u64, sequence: u64, source: impl Into<String>) -> Self {
        Self {
            captured_at_ms,
            sequence,
            source: source.into(),
        }
    }
}

/// Point-in-time snapshot of the system's USB / Type-C topology.
///
/// Holds discovered USB devices, Type-C ports, USB-PD ports, Thunderbolt/USB4
/// routers, and aggregate device summaries.
///
/// Available unconditionally without filesystem dependencies (`default-features = false`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// USB devices observed in the topology.
    #[serde(default)]
    pub usb_devices: Vec<UsbDevice>,

    /// Type-C ports observed in the topology.
    #[serde(default)]
    pub typec_ports: Vec<TypeCPort>,

    /// USB Power Delivery ports observed in the topology.
    #[serde(default)]
    pub pd_ports: Vec<PowerDeliveryPort>,

    /// Thunderbolt / USB4 routers observed in the topology.
    #[serde(default)]
    pub thunderbolt_routers: Vec<ThunderboltRouter>,

    /// Plain-English and structured summaries for all discovered devices and ports.
    #[serde(default)]
    pub summaries: Vec<DeviceSummary>,

    /// Optional metadata associated with this snapshot capture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<SnapshotMetadata>,

    /// Thunderbolt / USB4 domains (controller-wide security level, which
    /// determines the tunnel types the system will establish).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thunderbolt_domains: Vec<ThunderboltDomain>,

    /// Which kernel interfaces existed when this snapshot was captured.
    /// `None` means "not recorded" (e.g. a snapshot saved by an older
    /// release) — consumers must not read it as "all present" or "all
    /// absent".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub availability: Option<SourceAvailability>,
}

/// Which kernel interfaces were present when a [`Snapshot`] was captured.
///
/// Distinguishes "the subsystem is not there" from "the subsystem is there
/// but nothing is attached", so reports can say *why* a section is empty
/// instead of implying the hardware lacks a feature.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceAvailability {
    /// `/sys/bus/usb/devices` existed.
    #[serde(default)]
    pub usb_bus: bool,
    /// `/sys/class/typec` existed.
    #[serde(default)]
    pub typec_class: bool,
    /// `/sys/class/usb_power_delivery` existed.
    #[serde(default)]
    pub pd_class: bool,
    /// `/sys/bus/thunderbolt/devices` existed.
    #[serde(default)]
    pub thunderbolt_bus: bool,
}

impl Snapshot {
    /// Create a new empty snapshot.
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach metadata to the snapshot.
    pub fn with_metadata(mut self, metadata: SnapshotMetadata) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// `true` when no devices, ports, routers, or summaries are present.
    pub fn is_empty(&self) -> bool {
        self.usb_devices.is_empty()
            && self.typec_ports.is_empty()
            && self.pd_ports.is_empty()
            && self.thunderbolt_routers.is_empty()
            && self.thunderbolt_domains.is_empty()
            && self.summaries.is_empty()
    }

    /// Total number of summaries in this snapshot.
    pub fn len(&self) -> usize {
        self.summaries.len()
    }

    /// Lookup a [`DeviceSummary`] by its canonical identifier (e.g. `"typec:port0"`, `"usb:1-1.4"`).
    pub fn find_summary(&self, id: &str) -> Option<&DeviceSummary> {
        self.summaries.iter().find(|s| s.id() == id)
    }

    /// Lookup a [`UsbDevice`] by its bus-port identifier (e.g. `"1-1.4"`).
    pub fn find_usb_device(&self, bus_port: &str) -> Option<&UsbDevice> {
        self.usb_devices.iter().find(|d| d.bus_port == bus_port)
    }

    /// Lookup a [`TypeCPort`] by its numeric port number.
    pub fn find_typec_port(&self, port_number: i32) -> Option<&TypeCPort> {
        self.typec_ports
            .iter()
            .find(|p| p.port_number == port_number)
    }

    /// Lookup a [`TypeCPort`] by its sysfs name (e.g. `"port0"`).
    pub fn find_typec_port_by_name(&self, name: &str) -> Option<&TypeCPort> {
        self.typec_ports.iter().find(|p| p.port_name == name)
    }

    /// Lookup a [`PowerDeliveryPort`] by its name (e.g. `"pd0"`).
    pub fn find_pd_port(&self, name: &str) -> Option<&PowerDeliveryPort> {
        self.pd_ports.iter().find(|p| p.name == name)
    }

    /// Compare this snapshot with an earlier snapshot using the public comparison engine.
    pub fn compare(&self, previous: &Snapshot) -> crate::compare::SnapshotDiff {
        crate::compare::compare(previous, self)
    }
}

impl From<Vec<DeviceSummary>> for Snapshot {
    fn from(summaries: Vec<DeviceSummary>) -> Self {
        Self {
            summaries,
            ..Default::default()
        }
    }
}

#[cfg(any(feature = "cli", feature = "dbus"))]
impl Snapshot {
    /// Serialize the snapshot to compact JSON string.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Serialize the snapshot to pretty-printed JSON string.
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize a snapshot from JSON.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_empty_and_len() {
        let snap = Snapshot::new();
        assert!(snap.is_empty());
        assert_eq!(snap.len(), 0);
    }

    #[test]
    fn test_snapshot_with_metadata() {
        let meta = SnapshotMetadata::new(1729512000000, 42, "sysfs");
        let snap = Snapshot::new().with_metadata(meta);
        assert!(!snap.is_empty() || snap.metadata.is_some());
        let m = snap.metadata.as_ref().unwrap();
        assert_eq!(m.captured_at_ms, 1729512000000);
        assert_eq!(m.sequence, 42);
        assert_eq!(m.source, "sysfs");
    }

    #[test]
    fn test_snapshot_equality() {
        let snap1 = Snapshot::new();
        let snap2 = Snapshot::new();
        assert_eq!(snap1, snap2);
    }

    #[test]
    fn test_snapshot_json_roundtrip() {
        let mut snap = Snapshot::new();
        snap.metadata = Some(SnapshotMetadata::new(1729512000000, 42, "test-host"));

        let json = serde_json::to_string(&snap).expect("serialize snapshot");
        let parsed: Snapshot = serde_json::from_str(&json).expect("deserialize snapshot");
        assert_eq!(snap, parsed);
    }

    #[test]
    fn test_snapshot_deserialize_legacy_without_metadata() {
        let json = r#"{"usb_devices":[],"typec_ports":[],"pd_ports":[],"thunderbolt_routers":[],"summaries":[]}"#;
        let snap: Snapshot = serde_json::from_str(json).expect("deserialize legacy json");
        assert!(snap.is_empty());
        assert!(snap.metadata.is_none());
    }

    #[test]
    fn test_snapshot_deserialize_empty_object() {
        let json = r#"{}"#;
        let snap: Snapshot = serde_json::from_str(json).expect("deserialize empty json");
        assert!(snap.is_empty());
        assert!(snap.metadata.is_none());
    }
}
