//! USB4 / Thunderbolt / alternate-mode awareness (pure, IO-free).
//!
//! Everything here is a function of data already captured in a
//! [`Snapshot`], so it works offline (saved snapshots), under
//! `--no-default-features`, and never touches `/sys`.
//!
//! # Honest absence
//!
//! Alternate-mode and USB4 data is patchy across hardware and kernels. This
//! module therefore never turns "the kernel did not tell us" into a
//! negative claim:
//!
//! * [`Reported::NotApplicable`] — nothing is attached, so the question
//!   does not arise.
//! * [`Reported::NotReported`] — a partner is attached but the kernel
//!   exposed no entry. This does **not** mean the device lacks the feature:
//!   many platforms (notably UCSI) register only a subset of partner modes.
//! * Optional values (`Option<T>`) are `None` when a sysfs attribute was
//!   missing or unparseable — never a defaulted `0`.
//! * [`AltModeReport::caveats`] lists, in plain English, every limitation
//!   that applies to the snapshot being described.
//!
//! # What is *not* available from sysfs
//!
//! * The **active** DisplayPort pin assignment / lane count (the DP
//!   Configure VDO) — only the *advertised* DP Capabilities VDO is exposed.
//! * Live **tunnel** state (which PCIe/DP/USB3 tunnels exist). The closest
//!   signal is the domain security level; see [`TunnelPolicy`].
//! * Which Type-C port a USB4 link is on. The `thunderbolt` bus is not
//!   linked back to `/sys/class/typec`, so USB4 is reported system-wide.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::snapshot::Snapshot;
use crate::thunderbolt::{self, ThunderboltDomain, ThunderboltRouter};
use crate::typec::{TypeCAltMode, TypeCPort};

/// Standard ID of VESA DisplayPort alternate mode.
pub const SVID_DISPLAYPORT: u16 = 0xFF01;
/// Vendor ID of Intel, used as the Thunderbolt 3 alternate-mode SVID.
pub const SVID_THUNDERBOLT: u16 = 0x8087;

// ---------------------------------------------------------------------
// DisplayPort capability decoding
// ---------------------------------------------------------------------

/// Which DisplayPort roles a port advertises (DP Capabilities VDO bits 1:0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DpPortCapability {
    /// `0b00` — reserved by the specification.
    Reserved,
    /// `0b01` — UFP_D capable (DisplayPort sink / display side).
    UfpD,
    /// `0b10` — DFP_D capable (DisplayPort source / GPU side).
    DfpD,
    /// `0b11` — both roles.
    DfpDAndUfpD,
}

impl DpPortCapability {
    /// Stable machine label (`"DFP_D"`, `"UFP_D"`, `"DFP_D+UFP_D"`,
    /// `"reserved"`).
    pub fn label(self) -> &'static str {
        match self {
            DpPortCapability::Reserved => "reserved",
            DpPortCapability::UfpD => "UFP_D",
            DpPortCapability::DfpD => "DFP_D",
            DpPortCapability::DfpDAndUfpD => "DFP_D+UFP_D",
        }
    }
}

/// Decoded DisplayPort Capabilities VDO.
///
/// This is what the partner **advertised**, not what is currently
/// configured. Fields follow the VESA *DisplayPort Alt Mode on USB Type-C*
/// layout as implemented by the Linux kernel's `typec_displayport.h`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DpCapabilities {
    /// The raw VDO the fields below were decoded from.
    pub raw: u32,
    /// Bits 1:0 — supported roles.
    pub port_capability: DpPortCapability,
    /// Bits 5:2 — DP signalling-rate support, **raw**. Left undecoded
    /// because the meaning of the individual bits changed between revisions
    /// of the VESA specification.
    pub signaling_bits: u8,
    /// Bit 6 — `true` when the DP interface is on a Type-C *receptacle*,
    /// `false` when on a *plug* (e.g. a captive adapter cable).
    pub receptacle: bool,
    /// Bit 7 — `true` when USB 2.0 signalling is not used on the
    /// DisplayPort configuration.
    pub usb2_signaling_unused: bool,
    /// Bits 15:8 — supported DFP_D pin assignments as letters (`'A'`..`'F'`),
    /// ascending.
    pub dfp_d_pin_assignments: Vec<char>,
    /// Bits 23:16 — supported UFP_D pin assignments as letters,
    /// ascending.
    pub ufp_d_pin_assignments: Vec<char>,
}

fn activity_label(active: bool, reported: Option<bool>) -> &'static str {
    match reported {
        Some(true) => "active",
        Some(false) => "advertised",
        None if active => "active",
        None => "advertised (active state unavailable)",
    }
}

fn pin_letters(mask: u8) -> Vec<char> {
    (0..6u8)
        .filter(|bit| mask & (1 << bit) != 0)
        .map(|bit| (b'A' + bit) as char)
        .collect()
}

/// Number of DisplayPort lanes a pin assignment dedicates to DP, or `None`
/// for a letter outside `A`..=`F`.
///
/// `A`, `C` and `E` are four-lane DP-only assignments; `B`, `D` and `F`
/// are two-lane DP plus USB 3.x. `A`, `B` and `F` are deprecated in recent
/// revisions of the specification but are still reported when advertised.
pub fn dp_pin_assignment_lanes(letter: char) -> Option<u8> {
    match letter.to_ascii_uppercase() {
        'A' | 'C' | 'E' => Some(4),
        'B' | 'D' | 'F' => Some(2),
        _ => None,
    }
}

/// Decode a DisplayPort Capabilities VDO. Total: every `u32` decodes.
pub fn decode_dp_capabilities(vdo: u32) -> DpCapabilities {
    let port_capability = match vdo & 0b11 {
        0b01 => DpPortCapability::UfpD,
        0b10 => DpPortCapability::DfpD,
        0b11 => DpPortCapability::DfpDAndUfpD,
        _ => DpPortCapability::Reserved,
    };
    DpCapabilities {
        raw: vdo,
        port_capability,
        signaling_bits: ((vdo >> 2) & 0xF) as u8,
        receptacle: vdo & (1 << 6) != 0,
        usb2_signaling_unused: vdo & (1 << 7) != 0,
        dfp_d_pin_assignments: pin_letters(((vdo >> 8) & 0xFF) as u8),
        ufp_d_pin_assignments: pin_letters(((vdo >> 16) & 0xFF) as u8),
    }
}

impl DpCapabilities {
    /// Compact pin-assignment string, e.g. `"DFP_D:C,D UFP_D:C"`. Roles with
    /// no advertised assignment are omitted; empty when neither has any.
    pub fn pin_assignment_summary(&self) -> String {
        let join = |v: &[char]| {
            v.iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(",")
        };
        let mut parts = Vec::new();
        if !self.dfp_d_pin_assignments.is_empty() {
            parts.push(format!("DFP_D:{}", join(&self.dfp_d_pin_assignments)));
        }
        if !self.ufp_d_pin_assignments.is_empty() {
            parts.push(format!("UFP_D:{}", join(&self.ufp_d_pin_assignments)));
        }
        parts.join(" ")
    }

    /// Most DP lanes any advertised pin assignment can carry. `None` when no
    /// assignment is advertised. This is a capability ceiling, not the
    /// active lane count.
    pub fn max_lanes(&self) -> Option<u8> {
        self.dfp_d_pin_assignments
            .iter()
            .chain(&self.ufp_d_pin_assignments)
            .filter_map(|c| dp_pin_assignment_lanes(*c))
            .max()
    }
}

// ---------------------------------------------------------------------
// Honest-absence wrapper
// ---------------------------------------------------------------------

/// A value that distinguishes "not applicable" from "kernel did not
/// report it" from "reported".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "value", rename_all = "snake_case")]
pub enum Reported<T> {
    /// The question doesn't arise (e.g. no partner is attached).
    NotApplicable,
    /// Applicable, but the kernel exposed nothing. Not evidence of absence.
    NotReported,
    /// The kernel reported this.
    Reported(T),
}

impl<T> Reported<T> {
    /// The reported value, if any.
    pub fn value(&self) -> Option<&T> {
        match self {
            Reported::Reported(v) => Some(v),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------
// Per-port alternate modes
// ---------------------------------------------------------------------

/// DisplayPort alternate mode as advertised by a port's partner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayPortInfo {
    /// `true` when the mode has been entered.
    pub active: bool,
    /// Kernel activity attribute, absent when unreadable.
    pub reported_active: Option<bool>,
    /// Mode index within the SVID.
    pub mode: u32,
    /// Raw mode VDO. `None` when the kernel didn't publish a readable one.
    pub vdo: Option<u32>,
    /// Decoded capabilities; `None` exactly when `vdo` is `None`.
    pub capabilities: Option<DpCapabilities>,
    /// Driver-reported selected pin, not inferred from capabilities.
    pub pin_assignment: Option<String>,
}

/// Thunderbolt 3 alternate mode (SVID `0x8087`) as advertised by a partner.
///
/// USB4 is **not** an alternate mode; see [`Usb4Overview`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThunderboltAltModeInfo {
    /// `true` when the mode has been entered.
    pub active: bool,
    /// Kernel activity attribute, absent when unreadable.
    pub reported_active: Option<bool>,
    /// Mode index within the SVID.
    pub mode: u32,
    /// Raw mode VDO, `None` when not readable.
    pub vdo: Option<u32>,
}

/// An alternate mode other than DisplayPort / Thunderbolt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OtherAltMode {
    /// Standard or vendor ID.
    pub svid: u16,
    /// Mode index within the SVID.
    pub mode: u32,
    /// `true` when the mode has been entered.
    pub active: bool,
    /// Kernel activity attribute, absent when unreadable.
    pub reported_active: Option<bool>,
    /// Kernel `description`, if published.
    pub description: Option<String>,
}

/// Alternate-mode picture for one Type-C port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortAltModes {
    /// Kernel port name, e.g. `"port0"`.
    pub port_name: String,
    /// Numeric port index (`-1` if the kernel name was unparseable).
    pub port_number: i32,
    /// `true` when a partner is attached.
    pub partner_connected: bool,
    /// DisplayPort alternate mode.
    pub displayport: Reported<DisplayPortInfo>,
    /// Thunderbolt 3 alternate mode.
    pub thunderbolt: Reported<ThunderboltAltModeInfo>,
    /// Every other advertised mode, in kernel order.
    pub other: Vec<OtherAltMode>,
}

/// Pick the entry to report when a partner lists several modes for one
/// SVID: an active one if present, else the lowest mode index.
fn pick(modes: &[TypeCAltMode], svid: u16) -> Option<&TypeCAltMode> {
    modes
        .iter()
        .filter(|m| m.svid == svid)
        .min_by_key(|m| (!m.active, m.mode))
}

impl PortAltModes {
    /// Derive the alternate-mode picture for one port.
    pub fn for_port(port: &TypeCPort) -> Self {
        let Some(partner) = &port.partner else {
            return PortAltModes {
                port_name: port.port_name.clone(),
                port_number: port.port_number,
                partner_connected: false,
                displayport: Reported::NotApplicable,
                thunderbolt: Reported::NotApplicable,
                other: Vec::new(),
            };
        };
        let displayport = match pick(&partner.altmodes, SVID_DISPLAYPORT) {
            Some(m) => Reported::Reported(DisplayPortInfo {
                active: m.active,
                reported_active: m.reported_active,
                mode: m.mode,
                vdo: m.vdo,
                capabilities: m.vdo.map(decode_dp_capabilities),
                pin_assignment: m.pin_assignment.clone(),
            }),
            None => Reported::NotReported,
        };
        let thunderbolt = match pick(&partner.altmodes, SVID_THUNDERBOLT) {
            Some(m) => Reported::Reported(ThunderboltAltModeInfo {
                active: m.active,
                reported_active: m.reported_active,
                mode: m.mode,
                vdo: m.vdo,
            }),
            None => Reported::NotReported,
        };
        let other = partner
            .altmodes
            .iter()
            .filter(|m| m.svid != SVID_DISPLAYPORT && m.svid != SVID_THUNDERBOLT)
            .map(|m| OtherAltMode {
                svid: m.svid,
                mode: m.mode,
                active: m.active,
                reported_active: m.reported_active,
                description: m.description.clone(),
            })
            .collect();
        PortAltModes {
            port_name: port.port_name.clone(),
            port_number: port.port_number,
            partner_connected: true,
            displayport,
            thunderbolt,
            other,
        }
    }

    /// `true` when a partner is attached and the kernel listed no
    /// alternate modes at all.
    pub fn none_reported(&self) -> bool {
        self.partner_connected
            && matches!(self.displayport, Reported::NotReported)
            && matches!(self.thunderbolt, Reported::NotReported)
            && self.other.is_empty()
    }

    /// Flat key/value rendering for `DeviceSummary::properties`, so the
    /// CLI, JSON and the D-Bus `properties` bag all carry the same facts.
    ///
    /// Keys (all additive; unknown keys must be ignored by consumers):
    ///
    /// | Key | Values |
    /// |---|---|
    /// | `altmode.reported` | `none` — partner attached, kernel listed no modes |
    /// | `altmode.dp` | `active` \| `advertised` |
    /// | `altmode.dp.capability` | `DFP_D` \| `UFP_D` \| `DFP_D+UFP_D` \| `reserved` |
    /// | `altmode.dp.pin_assignments` | e.g. `DFP_D:C,D UFP_D:C`, or `unavailable` |
    /// | `altmode.tbt` | `active` \| `advertised` |
    /// | `altmode.other` | comma list of `svid:state` (e.g. `04e8:advertised`) |
    pub fn to_properties(&self) -> Vec<(String, String)> {
        let state = activity_label;
        let mut out: Vec<(String, String)> = Vec::new();
        if self.none_reported() {
            out.push(("altmode.reported".into(), "none".into()));
        }
        if let Reported::Reported(dp) = &self.displayport {
            out.push((
                "altmode.dp".into(),
                state(dp.active, dp.reported_active).into(),
            ));
            if let Some(pin) = &dp.pin_assignment {
                out.push(("altmode.dp.configured_pin".into(), pin.clone()));
            }
            match &dp.capabilities {
                Some(c) => {
                    out.push((
                        "altmode.dp.capability".into(),
                        c.port_capability.label().into(),
                    ));
                    let pins = c.pin_assignment_summary();
                    out.push((
                        "altmode.dp.pin_assignments".into(),
                        if pins.is_empty() {
                            "unavailable".into()
                        } else {
                            pins
                        },
                    ));
                }
                None => out.push(("altmode.dp.pin_assignments".into(), "unavailable".into())),
            }
        }
        if let Reported::Reported(tb) = &self.thunderbolt {
            out.push((
                "altmode.tbt".into(),
                state(tb.active, tb.reported_active).into(),
            ));
        }
        if !self.other.is_empty() {
            let list = self
                .other
                .iter()
                .map(|m| format!("{:04x}:{}", m.svid, state(m.active, m.reported_active)))
                .collect::<Vec<_>>()
                .join(",");
            out.push(("altmode.other".into(), list));
        }
        out
    }
}

// ---------------------------------------------------------------------
// Billboard
// ---------------------------------------------------------------------

/// Billboard-device view of why an alternate mode is (not) up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BillboardReport {
    /// Bus-port of the Billboard USB device, e.g. `"2-1"`.
    pub bus_port: String,
    /// Advertised alternate-mode SVIDs.
    pub svids: Vec<u16>,
    /// Overall state label (`"Successful"`, `"Unsuccessful"`, …); `None`
    /// when the descriptor lists no modes.
    pub overall_state: Option<String>,
    /// Machine-readable failure reasons (`"no_usb_pd"`, `"no_battery"`).
    pub failure_reasons: Vec<String>,
}

// ---------------------------------------------------------------------
// USB4 / Thunderbolt topology
// ---------------------------------------------------------------------

/// Whether a kernel interface was present when the data was captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BusState {
    /// The interface existed.
    Present,
    /// The interface did not exist (driver not loaded or no hardware).
    Absent,
    /// The snapshot doesn't record this. Treat as "can't say".
    Unknown,
}

/// What a domain's security level implies for tunnelling.
///
/// The kernel doesn't export live tunnel state through sysfs, so the
/// security level is the strongest tunnel-feature signal available. See the
/// kernel's `admin-guide/thunderbolt` document for the definitions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelPolicy {
    /// Raw `security` attribute. `None` when the attribute is absent.
    pub security: Option<String>,
    /// `Some(true)` when PCIe tunnelling is permitted (subject to device
    /// authorization), `Some(false)` when the level disables it, `None`
    /// when the level doesn't settle the question or isn't recognised.
    pub pcie_tunneling: Option<bool>,
    /// Plain-English explanation of the level, including "unrecognised"
    /// and "not reported" cases.
    pub explanation: String,
}

/// Interpret a domain security level. Never guesses for unknown values.
pub fn tunnel_policy(security: Option<&str>) -> TunnelPolicy {
    let (pcie, text): (Option<bool>, String) = match security {
        None => (
            None,
            "Security level not reported by the kernel; tunnelling policy unknown.".into(),
        ),
        Some("none") => (
            Some(true),
            "All devices are authorized automatically; PCIe tunnelling is permitted.".into(),
        ),
        Some("user") => (
            Some(true),
            "PCIe tunnelling is permitted once the user authorizes the device.".into(),
        ),
        Some("secure") => (
            Some(true),
            "PCIe tunnelling is permitted once the user authorizes the device with a challenge key."
                .into(),
        ),
        Some("dponly") => (
            Some(false),
            "Firmware creates DisplayPort and USB tunnels only; no PCIe tunnelling.".into(),
        ),
        Some("usbonly") => (
            None,
            "Firmware creates USB and DisplayPort tunnels for a dock; PCIe links downstream of the dock are removed."
                .into(),
        ),
        Some("nopcie") => (
            Some(false),
            "PCIe tunnelling is disabled on this USB4 system.".into(),
        ),
        Some(other) => (
            None,
            format!("Unrecognised security level \"{other}\"; tunnelling policy unknown."),
        ),
    };
    TunnelPolicy {
        security: security.map(str::to_owned),
        pcie_tunneling: pcie,
        explanation: text,
    }
}

/// One Thunderbolt / USB4 domain and its tunnelling policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainReport {
    /// Directory name, e.g. `"domain0"`.
    pub name: String,
    /// Numeric domain index.
    pub domain: u32,
    /// Tunnelling policy implied by the security level.
    pub tunnel_policy: TunnelPolicy,
    /// IOMMU DMA protection; `None` when not reported.
    pub iommu_dma_protection: Option<bool>,
    /// Whether the kernel can revoke authorization; `None` when not reported.
    pub deauthorization: Option<bool>,
}

impl From<&ThunderboltDomain> for DomainReport {
    fn from(d: &ThunderboltDomain) -> Self {
        DomainReport {
            name: d.name.clone(),
            domain: d.domain,
            tunnel_policy: tunnel_policy(d.security.as_deref()),
            iommu_dma_protection: d.iommu_dma_protection,
            deauthorization: d.deauthorization,
        }
    }
}

/// One router in the USB4 / Thunderbolt tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyNode {
    /// Kernel route string, e.g. `"0-1-2"`.
    pub route: String,
    /// Domain index.
    pub domain: u32,
    /// Hops from the host router (`0` for the host itself).
    pub depth: u32,
    /// `true` for a host router.
    pub is_host: bool,
    /// Kernel-reported generation (`1`..=`4`).
    pub generation: u8,
    /// Readable generation, absent when the legacy numeric field is zero.
    pub reported_generation: Option<u8>,
    /// `"USB4"`, `"Thunderbolt 3"`, `"Thunderbolt 2"`, `"Thunderbolt 1"`, or
    /// `"Thunderbolt (generation N)"` for values above 4.
    pub protocol: String,
    /// Vendor name; `None` when the kernel published none.
    pub vendor_name: Option<String>,
    /// Device name; `None` when the kernel published none.
    pub device_name: Option<String>,
    /// Stable unique ID; `None` when not published.
    pub unique_id: Option<String>,
    /// Authorization state; `None` when not reported.
    pub authorized: Option<bool>,
    /// NVM firmware version; `None` when not reported.
    pub nvm_version: Option<String>,
    /// Receive speed in Gb/s per lane; `None` when not reported.
    pub rx_speed_gbps: Option<u32>,
    /// Receive lane count; `None` when not reported.
    pub rx_lanes: Option<u8>,
    /// Transmit speed in Gb/s per lane; `None` when not reported.
    pub tx_speed_gbps: Option<u32>,
    /// Transmit lane count; `None` when not reported.
    pub tx_lanes: Option<u8>,
    /// `true` when this non-host router's parent isn't in the snapshot, so
    /// the tree position is uncertain.
    pub orphan: bool,
    /// Routers directly downstream of this one.
    pub children: Vec<TopologyNode>,
}

fn protocol_label(generation: u8) -> String {
    match generation {
        0 => "Thunderbolt/USB4 (generation unavailable)".into(),
        4 => "USB4".into(),
        3 => "Thunderbolt 3".into(),
        2 => "Thunderbolt 2".into(),
        1 => "Thunderbolt 1".into(),
        n => format!("Thunderbolt (generation {n})"),
    }
}

fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_owned())
}

/// Parent route of `route` (`"0-1-2"` → `"0-1"`, `"0-1"` → `"0-0"`), or
/// `None` for a host router / unparseable route.
fn parent_route(r: &ThunderboltRouter) -> Option<String> {
    if r.is_host {
        return None;
    }
    let (domain, route) = r.route.split_once('-')?;
    if !route.contains('-') {
        let value = u64::from_str_radix(route, 16).ok()?;
        let shift = (0..8).rev().find(|i| (value >> (i * 8)) != 0)? * 8;
        let parent = if shift == 0 {
            0
        } else {
            value & ((1u64 << shift) - 1)
        };
        return Some(format!("{domain}-{parent:x}"));
    }
    // Accept old saved snapshots which modeled hops as hyphen segments.
    let (head, last) = r.route.rsplit_once('-')?;
    let _: u32 = last.parse().ok()?;
    Some(if head.contains('-') {
        head.to_owned()
    } else {
        format!("{head}-0")
    })
}

fn route_key(route: &str) -> Vec<u32> {
    route
        .split('-')
        .map(|s| s.parse::<u32>().unwrap_or(u32::MAX))
        .collect()
}

fn depth_of(r: &ThunderboltRouter) -> u32 {
    if r.is_host {
        return 0;
    }
    let Some((_, route)) = r.route.split_once('-') else {
        return 0;
    };
    if let Ok(value) = u64::from_str_radix(route, 16) {
        return (64 - value.leading_zeros()).div_ceil(8);
    }
    r.route.split('-').skip(1).count() as u32
}

fn node_from(r: &ThunderboltRouter, orphan: bool) -> TopologyNode {
    TopologyNode {
        route: r.route.clone(),
        domain: r.domain,
        depth: depth_of(r),
        is_host: r.is_host,
        generation: r.generation,
        reported_generation: r
            .reported_generation
            .or((r.generation > 0).then_some(r.generation)),
        protocol: protocol_label(r.generation),
        vendor_name: non_empty(&r.vendor_name),
        device_name: non_empty(&r.device_name),
        unique_id: non_empty(&r.unique_id),
        authorized: r.authorized,
        nvm_version: r.nvm_version.clone(),
        rx_speed_gbps: r.rx_speed_gbps,
        rx_lanes: r.rx_lanes,
        tx_speed_gbps: r.tx_speed_gbps,
        tx_lanes: r.tx_lanes,
        orphan,
        children: Vec::new(),
    }
}

/// Build the router forest (one tree per host router). Routers whose parent
/// is missing become roots flagged [`TopologyNode::orphan`].
pub fn build_topology(routers: &[ThunderboltRouter]) -> Vec<TopologyNode> {
    let by_route: BTreeMap<&str, &ThunderboltRouter> =
        routers.iter().map(|r| (r.route.as_str(), r)).collect();
    let mut children_of: BTreeMap<String, Vec<&ThunderboltRouter>> = BTreeMap::new();
    let mut roots: Vec<(&ThunderboltRouter, bool)> = Vec::new();
    for r in routers {
        match parent_route(r) {
            Some(p) if by_route.contains_key(p.as_str()) => {
                children_of.entry(p).or_default().push(r);
            }
            Some(_) => roots.push((r, true)),
            None => roots.push((r, !r.is_host)),
        }
    }
    fn build(
        r: &ThunderboltRouter,
        orphan: bool,
        children_of: &BTreeMap<String, Vec<&ThunderboltRouter>>,
    ) -> TopologyNode {
        let mut node = node_from(r, orphan);
        if let Some(kids) = children_of.get(&r.route) {
            let mut kids = kids.clone();
            kids.sort_by_key(|k| route_key(&k.route));
            node.children = kids
                .into_iter()
                .map(|k| build(k, false, children_of))
                .collect();
        }
        node
    }
    roots.sort_by_key(|(r, _)| route_key(&r.route));
    roots
        .into_iter()
        .map(|(r, orphan)| build(r, orphan, &children_of))
        .collect()
}

/// Honest statement about live tunnel visibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelVisibility {
    /// Always `false` today: sysfs exposes no per-tunnel state.
    pub available: bool,
    /// Why, and where the nearest signal is.
    pub note: String,
}

/// System-wide USB4 / Thunderbolt picture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usb4Overview {
    /// Whether `/sys/bus/thunderbolt` existed.
    pub thunderbolt_bus: BusState,
    /// Domains and their tunnelling policy.
    pub domains: Vec<DomainReport>,
    /// Router forest.
    pub topology: Vec<TopologyNode>,
    /// `Some(true)` when a USB4 host and a USB4 downstream router are both
    /// present; `Some(false)` when routers were enumerated but no USB4 link
    /// exists; `None` when no router data is available at all. This is
    /// **system-wide** — sysfs can't attribute it to a Type-C port.
    pub usb4_link_active: Option<bool>,
    /// Tunnel-state visibility statement.
    pub tunnels: TunnelVisibility,
}

// ---------------------------------------------------------------------
// Full report
// ---------------------------------------------------------------------

/// Everything usbeehive can say about alternate modes and USB4 for one
/// [`Snapshot`], with explicit statements about what it cannot say.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AltModeReport {
    /// Per Type-C port alternate-mode picture, ordered by port number.
    pub ports: Vec<PortAltModes>,
    /// Billboard devices (USB devices explaining a failed alternate mode).
    pub billboards: Vec<BillboardReport>,
    /// System-wide USB4 / Thunderbolt picture.
    pub usb4: Usb4Overview,
    /// Plain-English limitations that apply to this snapshot.
    pub caveats: Vec<String>,
}

fn bus_state(recorded: Option<bool>, has_data: bool) -> BusState {
    match recorded {
        Some(true) => BusState::Present,
        Some(false) => BusState::Absent,
        None if has_data => BusState::Present,
        None => BusState::Unknown,
    }
}

/// Analyse a snapshot. Pure and total.
pub fn analyze(snapshot: &Snapshot) -> AltModeReport {
    let avail = snapshot.availability;

    let mut ports: Vec<PortAltModes> = snapshot
        .typec_ports
        .iter()
        .map(PortAltModes::for_port)
        .collect();
    ports.sort_by_key(|p| p.port_number);

    let billboards: Vec<BillboardReport> = snapshot
        .usb_devices
        .iter()
        .filter_map(|d| {
            let bb = d.bos.as_ref()?.billboard()?;
            Some(BillboardReport {
                bus_port: d.bus_port.clone(),
                svids: bb.alt_modes.iter().map(|m| m.svid).collect(),
                overall_state: bb.overall_state().map(|s| s.label().to_owned()),
                failure_reasons: bb
                    .failure_reasons()
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            })
        })
        .collect();

    let tb_bus = bus_state(
        avail.map(|a| a.thunderbolt_bus),
        !snapshot.thunderbolt_routers.is_empty() || !snapshot.thunderbolt_domains.is_empty(),
    );
    let has_usb4 = thunderbolt::system_has_usb4_link(&snapshot.thunderbolt_routers);
    let usb4_link_active = if has_usb4 {
        Some(true)
    } else if snapshot.thunderbolt_routers.is_empty()
        || snapshot
            .thunderbolt_routers
            .iter()
            .any(|r| r.generation == 0)
        || snapshot.thunderbolt_routers.iter().any(|r| {
            !r.is_host
                && !snapshot
                    .thunderbolt_routers
                    .iter()
                    .any(|h| h.is_host && h.domain == r.domain)
        })
    {
        None
    } else {
        Some(false)
    };
    let usb4 = Usb4Overview {
        thunderbolt_bus: tb_bus,
        domains: snapshot
            .thunderbolt_domains
            .iter()
            .map(Into::into)
            .collect(),
        topology: build_topology(&snapshot.thunderbolt_routers),
        usb4_link_active,
        tunnels: TunnelVisibility {
            available: false,
            note: "The kernel does not expose live tunnel state through sysfs; \
                   the domain security level is the nearest available signal."
                .into(),
        },
    };

    let mut caveats = Vec::new();
    match avail {
        None => caveats.push(
            "This snapshot does not record which kernel interfaces existed, so an empty \
             section cannot be told apart from unsupported hardware."
                .to_owned(),
        ),
        Some(a) => {
            if !a.typec_class {
                caveats.push(
                    "/sys/class/typec is not present: no Type-C alternate-mode data is \
                     available on this system."
                        .to_owned(),
                );
            }
            if !a.thunderbolt_bus {
                caveats.push(
                    "/sys/bus/thunderbolt is not present: no USB4/Thunderbolt data is \
                     available. This does not by itself mean the hardware lacks USB4 — \
                     the driver may not be loaded."
                        .to_owned(),
                );
            }
        }
    }
    if ports.iter().any(|p| p.none_reported()) {
        caveats.push(
            "Some attached partners listed no alternate modes. That means the kernel \
             reported none, not that the device lacks them; some platforms register \
             only a subset of partner modes."
                .to_owned(),
        );
    }
    caveats.push(
        "Only advertised DisplayPort capabilities are shown; the active pin assignment \
         and lane count are not exposed in sysfs."
            .to_owned(),
    );
    if usb4_link_active.is_some() {
        caveats.push(
            "USB4 link state is system-wide: sysfs does not link Thunderbolt/USB4 \
             routers to a specific Type-C port."
                .to_owned(),
        );
    }

    AltModeReport {
        ports,
        billboards,
        usb4,
        caveats,
    }
}

impl AltModeReport {
    /// Human-readable multi-line rendering.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str("Alternate modes\n");
        if self.ports.is_empty() {
            out.push_str("  (no Type-C ports reported)\n");
        }
        for p in &self.ports {
            out.push_str(&format!("  {}:", p.port_name));
            if !p.partner_connected {
                out.push_str(" no partner attached\n");
                continue;
            }
            if p.none_reported() {
                out.push_str(" none reported by the kernel\n");
                continue;
            }
            out.push('\n');
            if let Reported::Reported(dp) = &p.displayport {
                let state = activity_label(dp.active, dp.reported_active);
                out.push_str(&format!("    DisplayPort: {state}"));
                if let Some(pin) = &dp.pin_assignment {
                    out.push_str(&format!(", configured pin {pin}"));
                }
                match &dp.capabilities {
                    Some(c) => {
                        out.push_str(&format!(", {}", c.port_capability.label()));
                        let pins = c.pin_assignment_summary();
                        if pins.is_empty() {
                            out.push_str(", pin assignments unavailable");
                        } else {
                            out.push_str(&format!(", pin assignments {pins}"));
                        }
                    }
                    None => out.push_str(", capabilities unavailable (no VDO)"),
                }
                out.push('\n');
            }
            if let Reported::Reported(tb) = &p.thunderbolt {
                let state = activity_label(tb.active, tb.reported_active);
                out.push_str(&format!("    Thunderbolt 3 alt mode: {state}\n"));
            }
            for o in &p.other {
                let state = activity_label(o.active, o.reported_active);
                out.push_str(&format!("    SVID {:04x}: {state}\n", o.svid));
            }
        }

        for b in &self.billboards {
            out.push_str(&format!(
                "Billboard {}: {}{}\n",
                b.bus_port,
                b.overall_state.as_deref().unwrap_or("no modes listed"),
                if b.failure_reasons.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", b.failure_reasons.join(", "))
                }
            ));
        }

        out.push_str("USB4 / Thunderbolt\n");
        match self.usb4.thunderbolt_bus {
            BusState::Absent => out.push_str("  subsystem not present\n"),
            BusState::Unknown => out.push_str("  subsystem state not recorded\n"),
            BusState::Present => {
                if self.usb4.topology.is_empty() {
                    out.push_str("  no routers reported\n");
                }
                for n in &self.usb4.topology {
                    render_node(&mut out, n);
                }
                match self.usb4.usb4_link_active {
                    Some(true) => out.push_str("  USB4 link active (system-wide)\n"),
                    Some(false) => out.push_str("  no USB4 link active\n"),
                    None => out.push_str("  USB4 link state unknown (no routers)\n"),
                }
                for d in &self.usb4.domains {
                    out.push_str(&format!("  {}: {}\n", d.name, d.tunnel_policy.explanation));
                }
            }
        }

        if !self.caveats.is_empty() {
            out.push_str("Notes\n");
            for c in &self.caveats {
                out.push_str(&format!("  - {c}\n"));
            }
        }
        out
    }
}

fn render_node(out: &mut String, n: &TopologyNode) {
    let indent = "  ".repeat(n.depth as usize + 1);
    let who = match (&n.vendor_name, &n.device_name) {
        (Some(v), Some(d)) => format!("{v} {d}"),
        (Some(v), None) => v.clone(),
        (None, Some(d)) => d.clone(),
        (None, None) => "unnamed".to_owned(),
    };
    let role = if n.is_host { "host" } else { "device" };
    let link = match (n.tx_speed_gbps, n.tx_lanes) {
        (Some(s), Some(l)) => format!(", {l}x{s} Gb/s"),
        (Some(s), None) => format!(", {s} Gb/s (lanes unknown)"),
        _ => String::new(),
    };
    let orphan = if n.orphan {
        " [parent not in snapshot]"
    } else {
        ""
    };
    out.push_str(&format!(
        "{indent}{} {role} {who} ({}){link}{orphan}\n",
        n.route, n.protocol
    ));
    for c in &n.children {
        render_node(out, c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::SourceAvailability;
    use crate::typec::TypeCPartner;

    fn router(route: &str, is_host: bool, generation: u8) -> ThunderboltRouter {
        ThunderboltRouter {
            route: route.into(),
            domain: route
                .split('-')
                .next()
                .and_then(|d| d.parse().ok())
                .unwrap_or(0),
            is_host,
            generation,
            ..Default::default()
        }
    }

    fn alt(svid: u16, mode: u32, active: bool, vdo: Option<u32>) -> TypeCAltMode {
        TypeCAltMode {
            svid,
            mode,
            active,
            vdo,
            description: None,
            reported_active: Some(active),
            pin_assignment: None,
        }
    }

    fn port_with(altmodes: Vec<TypeCAltMode>) -> TypeCPort {
        TypeCPort {
            port_name: "port0".into(),
            port_number: 0,
            partner: Some(TypeCPartner {
                altmodes,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn dp_vdo_decodes_every_field() {
        // 0x000c0045: UFP_D pins C,D (bits 18,19); DFP none; plug bit clear;
        // port capability 01 (UFP_D); signalling bit 0 set (0b0001).
        let c = decode_dp_capabilities(0x000c_0045);
        assert_eq!(c.port_capability, DpPortCapability::UfpD);
        assert_eq!(c.ufp_d_pin_assignments, vec!['C', 'D']);
        assert!(c.dfp_d_pin_assignments.is_empty());
        assert!(c.receptacle);
        assert_eq!(c.signaling_bits, 0b0001);
    }

    #[test]
    fn dp_receptacle_and_dfp_pins() {
        // DFP_D pins C and E (bits 10, 12 → 0x1400), DFP_D capable (0b10).
        let c = decode_dp_capabilities(0x0000_1402);
        assert_eq!(c.port_capability, DpPortCapability::DfpD);
        assert_eq!(c.dfp_d_pin_assignments, vec!['C', 'E']);
        assert!(!c.receptacle);
        assert_eq!(c.max_lanes(), Some(4));
        assert_eq!(c.pin_assignment_summary(), "DFP_D:C,E");
    }

    #[test]
    fn dp_zero_vdo_is_reserved_with_no_pins() {
        let c = decode_dp_capabilities(0);
        assert_eq!(c.port_capability, DpPortCapability::Reserved);
        assert!(c.pin_assignment_summary().is_empty());
        assert_eq!(c.max_lanes(), None);
    }

    #[test]
    fn dp_both_roles_and_two_lane_assignment() {
        let c = decode_dp_capabilities(0x0008_0803); // DFP pin D, UFP pin D
        assert_eq!(c.port_capability, DpPortCapability::DfpDAndUfpD);
        assert_eq!(c.max_lanes(), Some(2));
        assert_eq!(c.pin_assignment_summary(), "DFP_D:D UFP_D:D");
    }

    #[test]
    fn dp_reserved_pin_bits_are_ignored() {
        // Bits 14/15 and 22/23 are outside A..F and must not be reported.
        let c = decode_dp_capabilities(0x00c0_c000);
        assert!(c.dfp_d_pin_assignments.is_empty());
        assert!(c.ufp_d_pin_assignments.is_empty());
    }

    #[test]
    fn pin_assignment_lane_table() {
        assert_eq!(dp_pin_assignment_lanes('C'), Some(4));
        assert_eq!(dp_pin_assignment_lanes('d'), Some(2));
        assert_eq!(dp_pin_assignment_lanes('Z'), None);
    }

    #[test]
    fn no_partner_is_not_applicable_not_absent() {
        let port = TypeCPort {
            port_name: "port0".into(),
            ..Default::default()
        };
        let p = PortAltModes::for_port(&port);
        assert_eq!(p.displayport, Reported::NotApplicable);
        assert!(!p.none_reported());
        assert!(p.to_properties().is_empty());
    }

    #[test]
    fn partner_without_modes_says_none_reported() {
        let p = PortAltModes::for_port(&port_with(vec![]));
        assert_eq!(p.displayport, Reported::NotReported);
        assert!(p.none_reported());
        assert_eq!(
            p.to_properties(),
            vec![("altmode.reported".to_string(), "none".to_string())]
        );
    }

    #[test]
    fn dp_without_vdo_reports_unavailable_pin_assignments() {
        let p = PortAltModes::for_port(&port_with(vec![alt(SVID_DISPLAYPORT, 1, true, None)]));
        let props = p.to_properties();
        assert!(props.contains(&("altmode.dp".into(), "active".into())));
        assert!(props.contains(&("altmode.dp.pin_assignments".into(), "unavailable".into())));
        let dp = p.displayport.value().unwrap();
        assert!(dp.capabilities.is_none());
    }

    #[test]
    fn dp_with_vdo_reports_capability_and_pins() {
        let p = PortAltModes::for_port(&port_with(vec![alt(
            SVID_DISPLAYPORT,
            1,
            false,
            Some(0x000c_0045),
        )]));
        let props = p.to_properties();
        assert!(props.contains(&("altmode.dp".into(), "advertised".into())));
        assert!(props.contains(&("altmode.dp.pin_assignments".into(), "UFP_D:C,D".into())));
    }

    #[test]
    fn active_mode_wins_over_lower_index_inactive_one() {
        let p = PortAltModes::for_port(&port_with(vec![
            alt(SVID_DISPLAYPORT, 1, false, Some(0x0000_0402)),
            alt(SVID_DISPLAYPORT, 2, true, Some(0x0000_1002)),
        ]));
        let dp = p.displayport.value().unwrap();
        assert_eq!(dp.mode, 2);
        assert!(dp.active);
    }

    #[test]
    fn thunderbolt_and_other_modes_are_listed() {
        let p = PortAltModes::for_port(&port_with(vec![
            alt(SVID_THUNDERBOLT, 3, true, None),
            alt(0x04e8, 1, false, None),
        ]));
        let props = p.to_properties();
        assert!(props.contains(&("altmode.tbt".into(), "active".into())));
        assert!(props.contains(&("altmode.other".into(), "04e8:advertised".into())));
        assert!(!p.none_reported());
    }

    #[test]
    fn tunnel_policy_never_guesses_unknown_levels() {
        let p = tunnel_policy(Some("quantum"));
        assert_eq!(p.pcie_tunneling, None);
        assert!(p.explanation.contains("quantum"));
        assert_eq!(tunnel_policy(None).pcie_tunneling, None);
        assert_eq!(tunnel_policy(Some("none")).pcie_tunneling, Some(true));
        assert_eq!(tunnel_policy(Some("dponly")).pcie_tunneling, Some(false));
        assert_eq!(tunnel_policy(Some("nopcie")).pcie_tunneling, Some(false));
        assert_eq!(tunnel_policy(Some("usbonly")).pcie_tunneling, None);
    }

    #[test]
    fn topology_nests_by_route() {
        let routers = vec![
            router("0-0", true, 4),
            router("0-1", false, 4),
            router("0-1-1", false, 3),
            router("0-3", false, 4),
            router("0-10", false, 4),
        ];
        let forest = build_topology(&routers);
        assert_eq!(forest.len(), 1);
        let host = &forest[0];
        assert_eq!(host.route, "0-0");
        assert_eq!(host.depth, 0);
        assert_eq!(host.protocol, "USB4");
        let kids: Vec<&str> = host.children.iter().map(|c| c.route.as_str()).collect();
        // Numeric, not lexical, ordering: 0-3 before 0-10.
        assert_eq!(kids, vec!["0-1", "0-3", "0-10"]);
        let nested = &host.children[0].children[0];
        assert_eq!(nested.route, "0-1-1");
        assert_eq!(nested.depth, 2);
        assert_eq!(nested.protocol, "Thunderbolt 3");
        assert!(!nested.orphan);
    }

    #[test]
    fn missing_parent_is_flagged_orphan_not_dropped() {
        let forest = build_topology(&[router("0-1-1", false, 4)]);
        assert_eq!(forest.len(), 1);
        assert!(forest[0].orphan);
    }

    #[test]
    fn multiple_domains_yield_multiple_trees() {
        let forest = build_topology(&[
            router("1-0", true, 4),
            router("0-0", true, 3),
            router("1-1", false, 4),
        ]);
        let roots: Vec<&str> = forest.iter().map(|n| n.route.as_str()).collect();
        assert_eq!(roots, vec!["0-0", "1-0"]);
        assert_eq!(forest[1].children.len(), 1);
    }

    #[test]
    fn empty_strings_become_none() {
        let mut r = router("0-0", true, 4);
        r.vendor_name = String::new();
        r.device_name = "Maple Ridge".into();
        let n = &build_topology(&[r])[0];
        assert_eq!(n.vendor_name, None);
        assert_eq!(n.device_name.as_deref(), Some("Maple Ridge"));
        assert_eq!(n.rx_speed_gbps, None);
    }

    #[test]
    fn empty_snapshot_without_availability_is_unknown_not_absent() {
        let r = analyze(&Snapshot::default());
        assert_eq!(r.usb4.thunderbolt_bus, BusState::Unknown);
        assert_eq!(r.usb4.usb4_link_active, None);
        assert!(r.caveats.iter().any(|c| c.contains("does not record")));
    }

    #[test]
    fn absent_buses_are_stated_when_recorded() {
        let snap = Snapshot {
            availability: Some(SourceAvailability {
                usb_bus: true,
                typec_class: false,
                pd_class: false,
                thunderbolt_bus: false,
            }),
            ..Default::default()
        };
        let r = analyze(&snap);
        assert_eq!(r.usb4.thunderbolt_bus, BusState::Absent);
        assert!(r
            .caveats
            .iter()
            .any(|c| c.contains("/sys/class/typec is not present")));
        assert!(r
            .caveats
            .iter()
            .any(|c| c.contains("does not by itself mean")));
        let text = r.to_text();
        assert!(text.contains("subsystem not present"));
    }

    #[test]
    fn usb4_link_is_system_wide_and_caveated() {
        let snap = Snapshot {
            thunderbolt_routers: vec![router("0-0", true, 4), router("0-1", false, 4)],
            availability: Some(SourceAvailability {
                usb_bus: true,
                typec_class: true,
                pd_class: true,
                thunderbolt_bus: true,
            }),
            ..Default::default()
        };
        let r = analyze(&snap);
        assert_eq!(r.usb4.usb4_link_active, Some(true));
        assert!(r.caveats.iter().any(|c| c.contains("system-wide")));
        assert!(!r.usb4.tunnels.available);
        assert!(r.to_text().contains("USB4 link active (system-wide)"));
    }

    #[test]
    fn report_round_trips_through_json() {
        let snap = Snapshot {
            typec_ports: vec![port_with(vec![alt(
                SVID_DISPLAYPORT,
                1,
                true,
                Some(0x000c_0045),
            )])],
            thunderbolt_routers: vec![router("0-0", true, 4)],
            ..Default::default()
        };
        let r = analyze(&snap);
        let json = serde_json::to_string(&r).unwrap();
        let back: AltModeReport = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
        assert!(json.contains("\"state\":\"reported\""));
    }
}
