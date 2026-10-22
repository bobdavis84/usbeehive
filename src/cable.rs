//! Decoded cable e-marker information.
//!
//! [`CableInfo::from_typec_cable`] is the bridge from the raw [`TypeCCable`]
//! sysfs snapshot to a strongly-typed cable view, applying the VDO decoders
//! from [`crate::pd`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::pd::{
    cable_vdo_reserved_bits_set, decode_cable_vdo, decode_id_header, id_header_reserved_bits_set,
    CableCurrent, CableSpeed, ProductType,
};
use crate::trust::CableTrustLevel;
use crate::typec::TypeCCable;
use crate::vendor;

/// Cable trust signals and evaluation results.
///
/// Contains heuristic indicators that hint at a counterfeit, non-compliant,
/// or constrained cable assembly.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct CableTrust {
    /// `true` when the cable's ID Header VDO reports a vendor ID of zero.
    /// USB-IF certified e-markers always carry a registered VID.
    pub zero_vid: bool,
    /// `true` when the vendor ID is non-zero but isn't in the bundled
    /// USB-IF vendor database (i.e. [`vendor::lookup`] fell back to a hex
    /// string).
    pub vid_unknown: bool,
    /// `true` when the raw Cable VDO or ID Header sets bits that USB-PD R3.x defines
    /// as reserved (see [`cable_vdo_reserved_bits_set`] and [`id_header_reserved_bits_set`]).
    pub reserved_bits_set: bool,

    /// Categorical trust assessment level.
    pub trust_level: CableTrustLevel,
    /// `true` when an active operating contract exceeds the cable's advertised current or voltage.
    pub emarker_contract_mismatch: bool,
    /// `true` when unsupported combinations (e.g. EPR voltage on SPR cable) are detected.
    pub unsupported_combination: bool,
    /// Contextual notes explaining trust classification and hedging against false positives.
    pub notes: Vec<String>,
}

impl<'de> Deserialize<'de> for CableTrust {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Default, Deserialize)]
        #[serde(default)]
        struct Wire {
            zero_vid: bool,
            vid_unknown: bool,
            reserved_bits_set: bool,
            trust_level: Option<CableTrustLevel>,
            emarker_contract_mismatch: bool,
            unsupported_combination: bool,
            notes: Vec<String>,
        }
        let w = Wire::deserialize(deserializer)?;
        let inferred = if w.zero_vid
            || w.reserved_bits_set
            || w.emarker_contract_mismatch
            || w.unsupported_combination
        {
            CableTrustLevel::Suspicious
        } else {
            CableTrustLevel::Inconclusive
        };
        Ok(Self {
            zero_vid: w.zero_vid,
            vid_unknown: w.vid_unknown,
            reserved_bits_set: w.reserved_bits_set,
            trust_level: w.trust_level.unwrap_or(inferred),
            emarker_contract_mismatch: w.emarker_contract_mismatch,
            unsupported_combination: w.unsupported_combination,
            notes: w.notes,
        })
    }
}

impl CableTrust {
    /// `true` when any anomaly or limitation heuristic fires.
    pub fn any(&self) -> bool {
        self.zero_vid
            || self.vid_unknown
            || self.reserved_bits_set
            || self.emarker_contract_mismatch
            || self.unsupported_combination
            || self.trust_level == CableTrustLevel::Suspicious
    }

    /// `true` when the trust level indicates a serious anomaly.
    pub fn is_suspicious(&self) -> bool {
        self.trust_level == CableTrustLevel::Suspicious
    }
}

/// Decoded view of a cable's e-marker.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CableInfo {
    /// `true` if the cable is electrically active.
    pub is_active: bool,
    /// `true` if the cable is passive (plain wires + e-marker).
    pub is_passive: bool,
    /// Raw `type` string from sysfs (`"active"` / `"passive"`).
    pub cable_type: String,
    /// Raw `plug_type` string.
    pub plug_type: String,

    /// Decoded link speed, when a Cable VDO is present.
    pub speed: Option<CableSpeed>,
    /// Decoded VBUS current rating.
    pub current_rating: Option<CableCurrent>,
    /// `current_rating × max_vbus_volts`, as a convenience.
    pub max_watts: u32,

    /// Cable vendor ID (low 16 bits of the ID Header VDO).
    pub vendor_id: u16,
    /// Friendly vendor name (or hex fallback) for `vendor_id`.
    pub vendor_name: String,

    /// Trust signals derived from the cable's e-marker. See [`CableTrust`].
    pub trust: CableTrust,

    /// Raw sysfs attributes captured for `--raw` rendering.
    pub raw_attributes: BTreeMap<String, String>,
}

impl CableInfo {
    /// Decode a [`TypeCCable`] snapshot into a [`CableInfo`].
    ///
    /// When the partner advertises an ID Header reporting an active cable,
    /// [`Self::is_active`] is set even if the kernel `type` attribute said
    /// `"passive"` — VDO data is the more authoritative source.
    pub fn from_typec_cable(cable: &TypeCCable) -> CableInfo {
        let mut info = CableInfo {
            cable_type: cable.r#type.clone(),
            plug_type: cable.plug_type.clone(),
            is_active: cable.r#type == "active",
            is_passive: cable.r#type == "passive",
            raw_attributes: cable.raw_attributes.clone(),
            ..Default::default()
        };
        let Some(id) = &cable.identity else {
            info.trust.trust_level = CableTrustLevel::Inconclusive;
            info.trust.notes.push("Cable identity telemetry is unavailable; passive type alone does not establish an absent e-marker or current rating.".into());
            return info;
        };
        info.vendor_id = id.vendor_id;
        info.vendor_name = vendor::lookup(id.vendor_id);
        info.trust.zero_vid = id.vendor_id == 0;
        info.trust.vid_unknown = id.vendor_id != 0 && vendor::is_hex_fallback(&info.vendor_name);
        let Some(&first) = id.vdos.first() else {
            info.trust.trust_level = CableTrustLevel::Inconclusive;
            return info;
        };
        let hdr = decode_id_header(first);
        if id_header_reserved_bits_set(first) {
            info.trust.reserved_bits_set = true;
        }
        let active = hdr.ufp_product_type == Some(ProductType::ActiveCable);
        if let Some(&fourth) = id.vdos.get(3) {
            let v = decode_cable_vdo(fourth, active);
            info.speed = Some(v.speed);
            info.current_rating = Some(v.current_rating);
            info.max_watts = v.max_watts;
            info.is_active = v.is_active;
            info.is_passive = !v.is_active;
            if cable_vdo_reserved_bits_set(fourth, active) {
                info.trust.reserved_bits_set = true;
            }
        }

        // Determine trust_level and populate notes
        if info.trust.zero_vid {
            info.trust.trust_level = CableTrustLevel::Suspicious;
            info.trust.notes.push(
                "E-marker reports Vendor ID 0x0000; USB-IF compliant e-markers must carry a registered vendor ID."
                    .into(),
            );
        }
        if info.trust.reserved_bits_set {
            info.trust.trust_level = CableTrustLevel::Suspicious;
            info.trust.notes.push(
                "E-marker sets reserved VDO bits; indicates non-compliant firmware or cloned microcode."
                    .into(),
            );
        }
        if info.trust.trust_level != CableTrustLevel::Suspicious {
            if id.vdos.get(3).is_none() {
                info.trust.trust_level = CableTrustLevel::Inconclusive;
                info.trust.notes.push(
                    "Cable VDO telemetry is absent from e-marker response; cable capabilities are inconclusive."
                        .into(),
                );
            } else if info.trust.vid_unknown {
                info.trust.trust_level = CableTrustLevel::KnownGood;
                info.trust.notes.push(format!(
                    "Vendor ID 0x{:04X} is not in local database; may be a newly registered USB-IF manufacturer.",
                    info.vendor_id
                ));
            } else {
                info.trust.trust_level = CableTrustLevel::KnownGood;
                info.trust.notes.push(
                    "Valid e-marker with registered USB-IF vendor ID and compliant VDO bit fields."
                        .into(),
                );
            }
        }
        info
    }

    /// Check active operating contract conditions against this cable's ratings and update
    /// trust signals.
    pub fn check_contract(&mut self, active_current_ma: u32, active_voltage_mv: u32) {
        if active_current_ma > 3_200 && self.current_rating == Some(CableCurrent::ThreeAmp) {
            self.trust.emarker_contract_mismatch = true;
            self.trust.trust_level = CableTrustLevel::Suspicious;
            self.trust.notes.push(format!(
                "Active contract current ({:.1}A) exceeds cable's 3A rating.",
                active_current_ma as f64 / 1000.0
            ));
        }
        if active_voltage_mv > 21_000 && self.max_watts > 0 && self.max_watts <= 100 {
            self.trust.unsupported_combination = true;
            self.trust.emarker_contract_mismatch = true;
            self.trust.trust_level = CableTrustLevel::Suspicious;
            self.trust.notes.push(format!(
                "Active contract voltage ({:.1}V) is EPR (>20V), but cable is rated only for SPR (20V max).",
                active_voltage_mv as f64 / 1000.0
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typec::TypeCIdentity;

    fn id(vdos: Vec<u32>, vid: u16) -> TypeCIdentity {
        TypeCIdentity {
            vendor_id: vid,
            product_id: 0,
            vdos,
        }
    }

    #[test]
    fn passive_string_only() {
        let c = TypeCCable {
            r#type: "passive".into(),
            plug_type: "type-c".into(),
            identity: None,
            raw_attributes: BTreeMap::new(),
        };
        let info = CableInfo::from_typec_cable(&c);
        assert!(info.is_passive);
        assert!(info.speed.is_none());
        assert_eq!(info.max_watts, 0);
    }

    #[test]
    fn cable_vdo_drives_active_flag() {
        let id_hdr = 4u32 << 27;
        let cable_vdo = 2u32 | (2 << 5) | (3 << 9);
        let cable = TypeCCable {
            r#type: "passive".into(), // overridden by VDO active flag
            plug_type: String::new(),
            identity: Some(id(vec![id_hdr, 0, 0, cable_vdo], 0x05AC)),
            raw_attributes: BTreeMap::new(),
        };
        let info = CableInfo::from_typec_cable(&cable);
        assert_eq!(info.vendor_name, "Apple");
        assert_eq!(info.speed, Some(CableSpeed::Usb32Gen2));
        assert_eq!(info.current_rating, Some(CableCurrent::FiveAmp));
        assert_eq!(info.max_watts, 250);
        assert!(info.is_active);
        assert!(!info.is_passive);
    }

    #[test]
    fn passive_with_vdo_keeps_passive_flag() {
        let id_hdr = 3u32 << 27; // PassiveCable
        let cable_vdo = 1u32 | (1 << 5); // Gen1, 3A
        let cable = TypeCCable {
            r#type: "passive".into(),
            plug_type: String::new(),
            identity: Some(id(vec![id_hdr, 0, 0, cable_vdo], 0x0BDA)),
            raw_attributes: BTreeMap::new(),
        };
        let info = CableInfo::from_typec_cable(&cable);
        assert_eq!(info.speed, Some(CableSpeed::Usb32Gen1));
        assert_eq!(info.current_rating, Some(CableCurrent::ThreeAmp));
        assert!(info.is_passive);
        assert!(!info.is_active);
        assert_eq!(info.vendor_name, "Realtek");
    }

    #[test]
    fn trust_zero_vid_fires_on_blank_emarker() {
        let cable = TypeCCable {
            r#type: "passive".into(),
            plug_type: String::new(),
            identity: Some(id(vec![3u32 << 27, 0, 0, 1], 0)),
            raw_attributes: BTreeMap::new(),
        };
        let info = CableInfo::from_typec_cable(&cable);
        assert!(info.trust.zero_vid);
        assert!(!info.trust.vid_unknown);
    }

    #[test]
    fn trust_vid_unknown_fires_on_hex_fallback() {
        // 0xDEAD isn't in the bundled vendor DB — lookup falls back to a
        // hex string and vid_unknown fires.
        let cable = TypeCCable {
            r#type: "passive".into(),
            plug_type: String::new(),
            identity: Some(id(vec![3u32 << 27, 0, 0, 1], 0xDEAD)),
            raw_attributes: BTreeMap::new(),
        };
        let info = CableInfo::from_typec_cable(&cable);
        assert!(!info.trust.zero_vid);
        assert!(info.trust.vid_unknown);
    }

    #[test]
    fn trust_reserved_bits_fires_on_dirty_vdo() {
        let dirty = 1u32 | (1 << 5) | (1 << 3); // bit 3 is reserved
        let cable = TypeCCable {
            r#type: "passive".into(),
            plug_type: String::new(),
            identity: Some(id(vec![3u32 << 27, 0, 0, dirty], 0x05AC)),
            raw_attributes: BTreeMap::new(),
        };
        let info = CableInfo::from_typec_cable(&cable);
        assert!(info.trust.reserved_bits_set);
        assert!(!info.trust.vid_unknown); // Apple is in the vendor DB
    }

    #[test]
    fn trust_clean_cable_fires_nothing() {
        let clean = 2u32 | (2 << 5) | (3 << 9);
        let cable = TypeCCable {
            r#type: "passive".into(),
            plug_type: String::new(),
            identity: Some(id(vec![3u32 << 27, 0, 0, clean], 0x05AC)),
            raw_attributes: BTreeMap::new(),
        };
        let info = CableInfo::from_typec_cable(&cable);
        assert!(!info.trust.any());
    }

    #[test]
    fn missing_cable_vdo_keeps_only_vendor() {
        let cable = TypeCCable {
            r#type: "passive".into(),
            plug_type: String::new(),
            identity: Some(id(vec![1u32 << 31], 0x05AC)),
            raw_attributes: BTreeMap::new(),
        };
        let info = CableInfo::from_typec_cable(&cable);
        assert_eq!(info.vendor_name, "Apple");
        assert!(info.speed.is_none());
        assert_eq!(info.max_watts, 0);
    }

    /// `max_watts` and `current_rating` are populated together from the Cable
    /// VDO, so `max_watts > 0` implies `current_rating.is_some()`. The
    /// `Bottleneck::CableNoEMarker` branch in `diagnostic.rs` sits after
    /// `Bottleneck::CableLimit` and is only unmaskable because of that — if a
    /// future change sets `max_watts` from another source without a rating,
    /// this test fails before the diagnostic silently disappears.
    #[test]
    fn max_watts_implies_current_rating_is_set() {
        // With a Cable VDO: both fields land together.
        let with_vdo = TypeCCable {
            r#type: "passive".into(),
            plug_type: String::new(),
            identity: Some(id(vec![3u32 << 27, 0, 0, 1u32 | (1 << 5)], 0x05AC)),
            raw_attributes: BTreeMap::new(),
        };
        let info = CableInfo::from_typec_cable(&with_vdo);
        assert!(info.max_watts > 0);
        assert!(info.current_rating.is_some());

        // Without one: neither field is set.
        let without_vdo = TypeCCable {
            r#type: "passive".into(),
            plug_type: String::new(),
            identity: Some(id(vec![3u32 << 27], 0x05AC)),
            raw_attributes: BTreeMap::new(),
        };
        let info = CableInfo::from_typec_cable(&without_vdo);
        assert_eq!(info.max_watts, 0);
        assert!(info.current_rating.is_none());
    }

    #[test]
    fn trust_classification_levels_in_cable_info() {
        // Clean cable -> KnownGood
        let clean = TypeCCable {
            r#type: "passive".into(),
            plug_type: String::new(),
            identity: Some(id(vec![3u32 << 27, 0, 0, 1u32 | (1 << 5)], 0x05AC)),
            raw_attributes: BTreeMap::new(),
        };
        let info = CableInfo::from_typec_cable(&clean);
        assert_eq!(info.trust.trust_level, CableTrustLevel::KnownGood);
        assert!(!info.trust.is_suspicious());
        assert!(!info.trust.notes.is_empty());

        // Passive without identity -> Inconclusive: missing telemetry is not a rating.
        let passive_no_id = TypeCCable {
            r#type: "passive".into(),
            plug_type: String::new(),
            identity: None,
            raw_attributes: BTreeMap::new(),
        };
        let info_passive = CableInfo::from_typec_cable(&passive_no_id);
        assert_eq!(
            info_passive.trust.trust_level,
            CableTrustLevel::Inconclusive
        );
        assert!(!info_passive.trust.is_suspicious());

        // Active without identity -> Inconclusive
        let active_no_id = TypeCCable {
            r#type: "active".into(),
            plug_type: String::new(),
            identity: None,
            raw_attributes: BTreeMap::new(),
        };
        let info_active = CableInfo::from_typec_cable(&active_no_id);
        assert_eq!(info_active.trust.trust_level, CableTrustLevel::Inconclusive);

        // Blank VID -> Suspicious
        let zero_vid_cable = TypeCCable {
            r#type: "passive".into(),
            plug_type: String::new(),
            identity: Some(id(vec![3u32 << 27, 0, 0, 1], 0)),
            raw_attributes: BTreeMap::new(),
        };
        let info_zero = CableInfo::from_typec_cable(&zero_vid_cable);
        assert_eq!(info_zero.trust.trust_level, CableTrustLevel::Suspicious);
        assert!(info_zero.trust.is_suspicious());
    }

    #[test]
    fn check_contract_detects_5a_on_3a_and_epr_on_spr() {
        // 3A cable (60W max)
        let mut info_3a = CableInfo {
            max_watts: 60,
            current_rating: Some(CableCurrent::ThreeAmp),
            trust: CableTrust {
                trust_level: CableTrustLevel::KnownGood,
                ..Default::default()
            },
            ..Default::default()
        };
        // Operating at 5A (5000 mA)
        info_3a.check_contract(5_000, 20_000);
        assert!(info_3a.trust.emarker_contract_mismatch);
        assert_eq!(info_3a.trust.trust_level, CableTrustLevel::Suspicious);
        assert!(info_3a.trust.is_suspicious());

        // SPR cable (100W max, 20V) operating at EPR 28V
        let mut info_spr = CableInfo {
            max_watts: 100,
            current_rating: Some(CableCurrent::FiveAmp),
            trust: CableTrust {
                trust_level: CableTrustLevel::KnownGood,
                ..Default::default()
            },
            ..Default::default()
        };
        info_spr.check_contract(5_000, 28_000);
        assert!(info_spr.trust.unsupported_combination);
        assert!(info_spr.trust.emarker_contract_mismatch);
        assert_eq!(info_spr.trust.trust_level, CableTrustLevel::Suspicious);
    }

    #[test]
    fn truncated_vdos_missing_cable_vdo_is_inconclusive() {
        let id_hdr = 3u32 << 27; // PassiveCable, clean bits
        let cable = TypeCCable {
            r#type: "passive".into(),
            plug_type: String::new(),
            identity: Some(id(vec![id_hdr], 0x05AC)), // Only 1 VDO (ID Header), Apple VID
            raw_attributes: BTreeMap::new(),
        };
        let info = CableInfo::from_typec_cable(&cable);
        assert_eq!(info.trust.trust_level, CableTrustLevel::Inconclusive);
        assert!(!info.trust.is_suspicious());
        assert!(info
            .trust
            .notes
            .iter()
            .any(|n| n.contains("Cable VDO telemetry is absent")));
    }

    #[test]
    fn truncated_vdos_with_zero_vid_remains_suspicious() {
        let id_hdr = 3u32 << 27;
        let cable = TypeCCable {
            r#type: "passive".into(),
            plug_type: String::new(),
            identity: Some(id(vec![id_hdr], 0x0000)), // Truncated VDOs, but zero VID
            raw_attributes: BTreeMap::new(),
        };
        let info = CableInfo::from_typec_cable(&cable);
        assert_eq!(info.trust.trust_level, CableTrustLevel::Suspicious);
        assert!(info.trust.zero_vid);
    }

    #[test]
    fn check_contract_detects_epr_on_non_emarked_cable() {
        let mut info_unmarked = CableInfo {
            is_passive: true,
            max_watts: 0,
            current_rating: None,
            trust: CableTrust {
                trust_level: CableTrustLevel::LikelyConstrained,
                ..Default::default()
            },
            ..Default::default()
        };
        info_unmarked.check_contract(3_000, 28_000);
        // Unknown rating cannot prove an EPR/SPR mismatch.
        assert!(!info_unmarked.trust.unsupported_combination);
        assert!(!info_unmarked.trust.emarker_contract_mismatch);
        assert!(!info_unmarked.trust.is_suspicious());
    }
}
