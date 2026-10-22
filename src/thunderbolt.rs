//! Thunderbolt / USB4 router data model.
//!
//! Plain-data type corresponding to entries under
//! `/sys/bus/thunderbolt/devices/`. Enumeration lives in `usbeehive-sysfs`.
//!
//! USB4 cannot be detected as a Type-C alternate mode — it's negotiated at
//! the USB-PD enter-mode layer and surfaces through the `thunderbolt`
//! subsystem instead. A router with `generation == 4` is the kernel's
//! signal that the link speaks the USB4 protocol (TBT4 is USB4 with Intel
//! branding); `generation == 3` is TBT3-only.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// One entry under `/sys/bus/thunderbolt/devices/`.
///
/// The kernel publishes every router on every Thunderbolt / USB4 domain
/// here. Host adapters carry a `-0` route suffix (`0-0`, `1-0`, …) and
/// represent the local controller; non-host routers are devices reachable
/// through the host (`0-1`, `0-1-1`, …).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThunderboltRouter {
    /// Absolute sysfs path of the router directory.
    pub sysfs_path: PathBuf,
    /// Kernel-assigned route string (e.g. `"0-0"`, `"0-1"`, `"0-1-1"`).
    pub route: String,
    /// Numeric domain index parsed from the route prefix.
    pub domain: u32,
    /// `true` when this router is a host adapter (route ends with `-0`).
    pub is_host: bool,
    /// Router generation. `1` = original Thunderbolt, `2` = TBT2, `3` = TBT3,
    /// `4` = USB4 (Intel markets this as Thunderbolt 4). `0` when unreadable.
    pub generation: u8,
    /// Actual readable generation; `None` for missing, invalid, or zero.
    /// The legacy `generation` field retains its numeric zero sentinel.
    #[serde(default)]
    pub reported_generation: Option<u8>,
    /// Vendor ID from the `vendor` attribute.
    pub vendor_id: u16,
    /// Device ID from the `device` attribute.
    pub device_id: u16,
    /// `vendor_name` attribute.
    pub vendor_name: String,
    /// `device_name` attribute.
    pub device_name: String,
    /// `unique_id` attribute — stable across reboots.
    pub unique_id: String,
    /// All regular files under the router directory.
    pub raw_attributes: BTreeMap<String, String>,
    /// `authorized` attribute: `Some(true)` once the router has been
    /// authorized for PCIe tunnelling. `None` when the attribute is absent
    /// or unparseable (it is not exposed on every kernel / router type).
    #[serde(default)]
    pub authorized: Option<bool>,
    /// `nvm_version` attribute (e.g. `"61.0"`), `None` when not published.
    #[serde(default)]
    pub nvm_version: Option<String>,
    /// Receive link speed in **Gb/s** per lane from `rx_speed`. `None`
    /// when the attribute is absent (host routers have no upstream link).
    #[serde(default)]
    pub rx_speed_gbps: Option<u32>,
    /// Receive lane count from `rx_lanes`. `None` when not published.
    #[serde(default)]
    pub rx_lanes: Option<u8>,
    /// Transmit link speed in **Gb/s** per lane from `tx_speed`.
    #[serde(default)]
    pub tx_speed_gbps: Option<u32>,
    /// Transmit lane count from `tx_lanes`.
    #[serde(default)]
    pub tx_lanes: Option<u8>,
}

/// One Thunderbolt / USB4 *domain* (`domainN` under
/// `/sys/bus/thunderbolt/devices/`).
///
/// The domain carries the controller-wide security level, which decides
/// which tunnel types the firmware/kernel will set up. The kernel does not
/// export live per-tunnel state through sysfs, so this is the closest
/// "tunnel features" signal available without debugfs.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThunderboltDomain {
    /// Directory name, e.g. `"domain0"`.
    pub name: String,
    /// Numeric domain index parsed from `name`.
    pub domain: u32,
    /// Raw `security` attribute (`none`, `user`, `secure`, `dponly`,
    /// `usbonly`, `nopcie`, …). `None` when absent.
    #[serde(default)]
    pub security: Option<String>,
    /// `iommu_dma_protection`: `Some(true)` when the platform reports
    /// IOMMU-based DMA protection for this domain. `None` when absent.
    #[serde(default)]
    pub iommu_dma_protection: Option<bool>,
    /// `deauthorization`: `Some(true)` when the kernel supports revoking
    /// authorization. `None` when absent.
    #[serde(default)]
    pub deauthorization: Option<bool>,
}

/// Parse a Thunderbolt link-speed attribute such as `"20.0 Gb/s"` into whole
/// Gb/s per lane.
///
/// Returns `None` for malformed or fractional values which cannot be represented
/// exactly as whole Gb/s; never silently truncates a reading.
pub fn parse_speed_gbps(raw: &str) -> Option<u32> {
    let value = raw.trim().strip_suffix("Gb/s").unwrap_or(raw.trim()).trim();
    let (whole, fraction) = value.split_once('.').unwrap_or((value, "0"));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.is_empty()
        || !fraction.bytes().all(|b| b == b'0')
    {
        return None;
    }
    whole.parse().ok()
}

impl ThunderboltRouter {
    /// `true` when this router negotiated the USB4 protocol
    /// (i.e. `generation >= 4`). TBT3-only routers return `false`.
    pub fn supports_usb4(&self) -> bool {
        self.generation >= 4
    }
}

/// Sentinel for "the local system has at least one USB4-capable host
/// adapter AND at least one non-host router has negotiated USB4."
///
/// USB4 is link-local, but the kernel's `thunderbolt` sysfs doesn't link
/// individual routers back to specific Type-C ports — so this is the
/// strongest "USB4 link in use somewhere on this system" signal available
/// without a separate port-mapping table.
pub fn system_has_usb4_link(routers: &[ThunderboltRouter]) -> bool {
    routers.iter().any(|r| {
        !r.is_host
            && r.supports_usb4()
            && routers
                .iter()
                .any(|h| h.is_host && h.domain == r.domain && h.supports_usb4())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn router(route: &str, is_host: bool, generation: u8) -> ThunderboltRouter {
        ThunderboltRouter {
            route: route.into(),
            is_host,
            generation,
            ..Default::default()
        }
    }

    #[test]
    fn usb4_host_in_another_domain_does_not_prove_a_link() {
        let mut host = router("0-0", true, 4);
        host.domain = 0;
        let mut device = router("1-1", false, 4);
        device.domain = 1;
        assert!(!system_has_usb4_link(&[host, device]));
    }

    #[test]
    fn supports_usb4_requires_generation_4() {
        assert!(router("0-0", true, 4).supports_usb4());
        assert!(!router("0-0", true, 3).supports_usb4());
        assert!(!router("0-0", true, 0).supports_usb4());
    }

    #[test]
    fn system_has_usb4_link_needs_both_sides() {
        // Just a USB4 host, no devices → no active link.
        assert!(!system_has_usb4_link(&[router("0-0", true, 4)]));
        // USB4 device but a TBT3-only host → no USB4 link (host can't drive it).
        assert!(!system_has_usb4_link(&[
            router("0-0", true, 3),
            router("0-1", false, 4),
        ]));
        // Both sides USB4 → link active.
        assert!(system_has_usb4_link(&[
            router("0-0", true, 4),
            router("0-1", false, 4),
        ]));
    }

    #[test]
    fn system_has_usb4_link_ignores_tbt3_partner_under_usb4_host() {
        // Backwards-compat dock — TBT3 partner on a USB4 host. Link runs at
        // TBT3, not USB4.
        assert!(!system_has_usb4_link(&[
            router("0-0", true, 4),
            router("0-1", false, 3),
        ]));
    }
}
