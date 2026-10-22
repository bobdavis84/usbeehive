//! Device filtering for the CLI (`--class`, `--vendor`, `--speed`, …).
//!
//! Operates purely on [`DeviceSummary`] values — no core library type is
//! changed. Semantics:
//!
//! - **Different flags combine with AND**; **repeated values of one flag
//!   combine with OR** (`--class hid --class audio --min-speed 480`
//!   means "(hid OR audio) AND at least 480 Mbps").
//! - Filters never guess. An entry whose relevant data is unknown does not
//!   match a filter on that data: a Type-C port (no USB descriptors) never
//!   matches `--vendor`, and an entry with an unknown (zero) link speed
//!   never matches `--min-speed` / `--max-speed`.

use clap::{Args, ValueEnum};
use usbeehive::summary::{Category, DeviceClass, DeviceSummary, Status};

use crate::severity::{severity_of, Severity};

/// Device-class selector accepted by `--class`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassSel {
    /// One concrete [`DeviceClass`].
    Device(DeviceClass),
    /// Any HID-derived class: keyboard, mouse, input-tablet, gamepad.
    Hid,
    /// Wired or wireless network adapter.
    Network,
    /// Type-C port entries (they carry no device class of their own).
    TypeC,
}

/// Accepted `--class` spellings, in the order shown in error messages.
const CLASS_TABLE: &[(&str, ClassSel)] = &[
    ("hid", ClassSel::Hid),
    ("keyboard", ClassSel::Device(DeviceClass::Keyboard)),
    ("mouse", ClassSel::Device(DeviceClass::Mouse)),
    ("input-tablet", ClassSel::Device(DeviceClass::InputTablet)),
    ("gamepad", ClassSel::Device(DeviceClass::Gamepad)),
    ("storage", ClassSel::Device(DeviceClass::Storage)),
    ("display", ClassSel::Device(DeviceClass::Display)),
    ("audio", ClassSel::Device(DeviceClass::Audio)),
    ("camera", ClassSel::Device(DeviceClass::Camera)),
    ("video-capture", ClassSel::Device(DeviceClass::VideoCapture)),
    ("printer", ClassSel::Device(DeviceClass::Printer)),
    ("phone", ClassSel::Device(DeviceClass::Phone)),
    ("hub", ClassSel::Device(DeviceClass::Hub)),
    ("network", ClassSel::Network),
    ("network-wired", ClassSel::Device(DeviceClass::NetworkWired)),
    (
        "network-wireless",
        ClassSel::Device(DeviceClass::NetworkWireless),
    ),
    ("security-key", ClassSel::Device(DeviceClass::SecurityKey)),
    (
        "smartcard-reader",
        ClassSel::Device(DeviceClass::SmartcardReader),
    ),
    ("bluetooth", ClassSel::Device(DeviceClass::Bluetooth)),
    ("serial", ClassSel::Device(DeviceClass::Serial)),
    ("unknown", ClassSel::Device(DeviceClass::Unknown)),
    ("typec", ClassSel::TypeC),
];

impl ClassSel {
    /// The `--class` spelling of this selector.
    pub fn name(self) -> &'static str {
        CLASS_TABLE
            .iter()
            .find(|(_, c)| *c == self)
            .map(|(n, _)| *n)
            .unwrap_or("?")
    }

    /// Does `s` belong to this class selection?
    pub fn matches(self, s: &DeviceSummary) -> bool {
        match self {
            ClassSel::TypeC => s.category == Category::TypeCPort,
            ClassSel::Hid => matches!(
                s.device_class,
                DeviceClass::Keyboard
                    | DeviceClass::Mouse
                    | DeviceClass::InputTablet
                    | DeviceClass::Gamepad
            ),
            ClassSel::Network => matches!(
                s.device_class,
                DeviceClass::NetworkWired | DeviceClass::NetworkWireless
            ),
            // Type-C ports are always `Unknown`; they are not "unknown devices".
            ClassSel::Device(DeviceClass::Unknown) => {
                s.category != Category::TypeCPort && s.device_class == DeviceClass::Unknown
            }
            ClassSel::Device(DeviceClass::Hub) => {
                s.category == Category::Hub
                    || s.device_class == DeviceClass::Hub
                    || s.usb_device.as_ref().is_some_and(|d| d.device_class == 9)
            }
            ClassSel::Device(c) => s.device_class == c,
        }
    }
}

/// Vendor selector accepted by `--vendor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VendorSel {
    /// USB vendor ID (`046d` or `0x046d`).
    Id(u16),
    /// Case-insensitive substring of the vendor name.
    Name(String),
}

/// Power-state selector accepted by `--charging` (Type-C ports only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ChargeState {
    /// Port is sinking PD power ([`Status::Charging`]).
    Charging,
    /// Port is sourcing PD power to a downstream device ([`Status::Sourcing`]).
    Sourcing,
    /// A partner is attached but no PD power flows ([`Status::Connected`]).
    Idle,
    /// Nothing is attached ([`Status::Empty`]).
    None,
}

impl ChargeState {
    fn matches(self, s: &DeviceSummary) -> bool {
        if s.category != Category::TypeCPort {
            return false;
        }
        let want = match self {
            ChargeState::Charging => Status::Charging,
            ChargeState::Sourcing => Status::Sourcing,
            ChargeState::Idle => Status::Connected,
            ChargeState::None => Status::Empty,
        };
        s.status == want
    }

    fn name(self) -> &'static str {
        match self {
            ChargeState::Charging => "charging",
            ChargeState::Sourcing => "sourcing",
            ChargeState::Idle => "idle",
            ChargeState::None => "none",
        }
    }
}

/// Parse a 1–4 digit hex number, with an optional `0x` prefix.
pub fn parse_hex_u16(s: &str) -> Result<u16, String> {
    let t = s.trim();
    let digits = t
        .strip_prefix("0x")
        .or_else(|| t.strip_prefix("0X"))
        .unwrap_or(t);
    if digits.is_empty() || digits.len() > 4 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "'{s}' is not a 16-bit hex value (expected 1-4 hex digits, e.g. 046d)"
        ));
    }
    u16::from_str_radix(digits, 16).map_err(|e| e.to_string())
}

fn parse_class(s: &str) -> Result<ClassSel, String> {
    let want = s.trim().to_ascii_lowercase();
    CLASS_TABLE
        .iter()
        .find(|(n, _)| *n == want)
        .map(|(_, c)| *c)
        .ok_or_else(|| {
            let names: Vec<&str> = CLASS_TABLE.iter().map(|(n, _)| *n).collect();
            format!(
                "unknown class '{s}' (expected one of: {})",
                names.join(", ")
            )
        })
}

fn parse_vendor(s: &str) -> Result<VendorSel, String> {
    let t = s.trim();
    if t.is_empty() {
        return Err("vendor must not be empty".into());
    }
    if t.starts_with("0x") || t.starts_with("0X") {
        return parse_hex_u16(t).map(VendorSel::Id);
    }
    let looks_like_id = t.len() == 4;
    if looks_like_id {
        if let Ok(id) = parse_hex_u16(t) {
            return Ok(VendorSel::Id(id));
        }
    }
    Ok(VendorSel::Name(t.to_ascii_lowercase()))
}

fn parse_id(s: &str) -> Result<(u16, u16), String> {
    let (v, p) = s
        .split_once(':')
        .ok_or_else(|| format!("'{s}' is not VID:PID (e.g. 046d:c52b)"))?;
    Ok((parse_hex_u16(v)?, parse_hex_u16(p)?))
}

/// Parse a speed such as `480`, `480M`, `5G`, `1.5Mbps` into Mbps.
///
/// A bare number is Mbps. Suffixes `k`, `m`, `g` (optionally followed by
/// `bps`) scale by 0.001, 1 and 1000.
pub fn parse_speed_mbps(s: &str) -> Result<u32, String> {
    let lower = s.trim().to_ascii_lowercase();
    let body = lower.strip_suffix("bps").unwrap_or(&lower);
    let (num, scale) = match body.chars().last() {
        Some('k') => (&body[..body.len() - 1], 0.001),
        Some('m') => (&body[..body.len() - 1], 1.0),
        Some('g') => (&body[..body.len() - 1], 1000.0),
        _ => (body, 1.0),
    };
    let n: f64 = num
        .trim()
        .parse()
        .map_err(|_| format!("'{s}' is not a speed (examples: 480, 480M, 5G, 10Gbps)"))?;
    if !n.is_finite() || n < 0.0 {
        return Err(format!("'{s}' is not a valid speed"));
    }
    let mbps = (n * scale).round();
    if mbps > f64::from(u32::MAX) {
        return Err(format!("'{s}' is too large"));
    }
    Ok(mbps as u32)
}

/// Filter flags. Flatten into the CLI parser with `#[command(flatten)]`.
#[derive(Args, Debug, Clone, Default)]
pub struct FilterArgs {
    /// Only show this device class; repeatable (OR). One of: hid, keyboard,
    /// mouse, input-tablet, gamepad, storage, display, audio, camera,
    /// video-capture, printer, phone, hub, network, network-wired,
    /// network-wireless, security-key, smartcard-reader, bluetooth, serial,
    /// unknown, typec. `typec` selects Type-C port entries.
    #[arg(long = "class", value_name = "CLASS", value_parser = parse_class, help_heading = "Filters")]
    pub classes: Vec<ClassSel>,

    /// Only USB devices from this vendor; repeatable (OR). A 4-digit hex ID
    /// (`046d`, `0x046d`) matches the vendor ID; anything else is a
    /// case-insensitive substring of the vendor name (`logitech`).
    #[arg(long = "vendor", value_name = "HEX|NAME", value_parser = parse_vendor, help_heading = "Filters")]
    pub vendors: Vec<VendorSel>,

    /// Only USB devices with this product ID (hex); repeatable (OR).
    #[arg(long = "product", value_name = "HEX", value_parser = parse_hex_u16, help_heading = "Filters")]
    pub products: Vec<u16>,

    /// Only USB devices with this VID:PID pair (e.g. 046d:c52b); repeatable (OR).
    #[arg(long = "id", value_name = "VID:PID", value_parser = parse_id, help_heading = "Filters")]
    pub ids: Vec<(u16, u16)>,

    /// Only entries linked at or above this speed. Bare numbers are Mbps;
    /// `480M`, `5G`, `10Gbps` also work. Entries with unknown speed are excluded.
    #[arg(long = "min-speed", value_name = "SPEED", value_parser = parse_speed_mbps, help_heading = "Filters")]
    pub min_speed_mbps: Option<u32>,

    /// Only entries linked at or below this speed (same syntax as
    /// `--min-speed`). Entries with unknown speed are excluded.
    #[arg(long = "max-speed", value_name = "SPEED", value_parser = parse_speed_mbps, help_heading = "Filters")]
    pub max_speed_mbps: Option<u32>,

    /// Only Type-C ports in this power state; repeatable (OR). `charging`:
    /// sinking PD power; `sourcing`: supplying a downstream device; `idle`:
    /// partner attached, no PD power flowing; `none`: nothing attached.
    /// USB devices and hubs never match.
    #[arg(
        long = "charging",
        value_name = "STATE",
        value_enum,
        help_heading = "Filters"
    )]
    pub charging: Vec<ChargeState>,

    /// Only the Type-C port with this number (`--port 0` is `port0`);
    /// repeatable (OR).
    #[arg(long = "port", value_name = "N", help_heading = "Filters")]
    pub ports: Vec<i32>,

    /// Only entries at or above this warning severity. `ok` keeps everything;
    /// `warning` keeps slow-cable, degraded-link and suspicious-cable
    /// findings; `critical` keeps only severe diagnostics.
    #[arg(
        long = "severity",
        value_name = "LEVEL",
        value_enum,
        help_heading = "Filters"
    )]
    pub severity: Option<Severity>,
}

impl FilterArgs {
    /// `true` when at least one filter flag was given.
    pub fn is_active(&self) -> bool {
        !(self.classes.is_empty()
            && self.vendors.is_empty()
            && self.products.is_empty()
            && self.ids.is_empty()
            && self.min_speed_mbps.is_none()
            && self.max_speed_mbps.is_none()
            && self.charging.is_empty()
            && self.ports.is_empty()
            && self.severity.is_none())
    }

    /// Cross-flag sanity check (e.g. an empty speed range).
    pub fn validate(&self) -> Result<(), String> {
        match (self.min_speed_mbps, self.max_speed_mbps) {
            (Some(lo), Some(hi)) if lo > hi => Err(format!(
                "--min-speed ({lo} Mbps) is greater than --max-speed ({hi} Mbps)"
            )),
            _ => Ok(()),
        }
    }

    /// Does `s` pass every active filter?
    pub fn matches(&self, s: &DeviceSummary) -> bool {
        self.class_ok(s)
            && self.usb_identity_ok(s)
            && self.speed_ok(s)
            && self.power_state_ok(s)
            && self.port_ok(s)
            && self.severity.is_none_or(|min| severity_of(s) >= min)
    }

    fn class_ok(&self, s: &DeviceSummary) -> bool {
        self.classes.is_empty() || self.classes.iter().any(|c| c.matches(s))
    }

    fn usb_identity_ok(&self, s: &DeviceSummary) -> bool {
        let wants_identity =
            !(self.vendors.is_empty() && self.products.is_empty() && self.ids.is_empty());
        if !wants_identity {
            return true;
        }
        if s.usb_device.is_none() {
            return false;
        }
        let vendor_ok = self.vendors.is_empty()
            || self.vendors.iter().any(|v| match v {
                VendorSel::Id(id) => s.vendor_id == *id,
                VendorSel::Name(n) => s.vendor.to_ascii_lowercase().contains(n),
            });
        let product_ok = self.products.is_empty() || self.products.contains(&s.product_id);
        let id_ok = self.ids.is_empty() || self.ids.contains(&(s.vendor_id, s.product_id));
        vendor_ok && product_ok && id_ok
    }

    fn speed_ok(&self, s: &DeviceSummary) -> bool {
        if self.min_speed_mbps.is_none() && self.max_speed_mbps.is_none() {
            return true;
        }
        if s.link_speed_mbps == 0 {
            return false;
        }
        self.min_speed_mbps.is_none_or(|lo| s.link_speed_mbps >= lo)
            && self.max_speed_mbps.is_none_or(|hi| s.link_speed_mbps <= hi)
    }

    fn power_state_ok(&self, s: &DeviceSummary) -> bool {
        self.charging.is_empty() || self.charging.iter().any(|c| c.matches(s))
    }

    fn port_ok(&self, s: &DeviceSummary) -> bool {
        self.ports.is_empty()
            || s.typec_port
                .as_ref()
                .is_some_and(|p| self.ports.contains(&p.port_number))
    }

    /// Human-readable `key=value` lines describing the active filters, in a
    /// fixed order (used by the `report` format).
    pub fn describe(&self) -> Vec<String> {
        let mut out = Vec::new();
        let join = |v: Vec<String>| v.join("|");
        if !self.classes.is_empty() {
            let names = self.classes.iter().map(|c| c.name().to_string());
            out.push(format!("class={}", join(names.collect())));
        }
        if !self.vendors.is_empty() {
            let v = self.vendors.iter().map(|v| match v {
                VendorSel::Id(id) => format!("{id:04x}"),
                VendorSel::Name(n) => n.clone(),
            });
            out.push(format!("vendor={}", join(v.collect())));
        }
        if !self.products.is_empty() {
            let p = self.products.iter().map(|p| format!("{p:04x}"));
            out.push(format!("product={}", join(p.collect())));
        }
        if !self.ids.is_empty() {
            let i = self.ids.iter().map(|(v, p)| format!("{v:04x}:{p:04x}"));
            out.push(format!("id={}", join(i.collect())));
        }
        if let Some(lo) = self.min_speed_mbps {
            out.push(format!("min-speed={lo}Mbps"));
        }
        if let Some(hi) = self.max_speed_mbps {
            out.push(format!("max-speed={hi}Mbps"));
        }
        if !self.charging.is_empty() {
            let c = self.charging.iter().map(|c| c.name().to_string());
            out.push(format!("charging={}", join(c.collect())));
        }
        if !self.ports.is_empty() {
            let p = self.ports.iter().map(|p| p.to_string());
            out.push(format!("port={}", join(p.collect())));
        }
        if let Some(sev) = self.severity {
            out.push(format!("severity>={}", sev.label()));
        }
        out
    }
}

/// Keep only the summaries that pass `filters`, preserving order.
pub fn apply(filters: &FilterArgs, devices: &[DeviceSummary]) -> Vec<DeviceSummary> {
    devices
        .iter()
        .filter(|s| filters.matches(s))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use usbeehive::typec::TypeCPort;
    use usbeehive::usb::UsbDevice;

    fn usb(vid: u16, pid: u16, vendor: &str, speed: u32, class: u8) -> DeviceSummary {
        DeviceSummary::from_usb_device(&UsbDevice {
            vendor_id: vid,
            product_id: pid,
            manufacturer: vendor.into(),
            product: "Thing".into(),
            speed,
            device_class: class,
            bus_port: format!("1-{pid}"),
            ..Default::default()
        })
    }

    fn port(n: i32, status: Status) -> DeviceSummary {
        let mut s = DeviceSummary::from_typec_port(
            &TypeCPort {
                port_number: n,
                port_name: format!("port{n}"),
                ..Default::default()
            },
            None,
            None,
            None,
        );
        s.status = status;
        s
    }

    #[test]
    fn hex_parsing_accepts_prefix_and_rejects_garbage() {
        assert_eq!(parse_hex_u16("046d"), Ok(0x046d));
        assert_eq!(parse_hex_u16("0x046D"), Ok(0x046d));
        assert_eq!(parse_hex_u16("c"), Ok(0xc));
        assert!(parse_hex_u16("").is_err());
        assert!(parse_hex_u16("12345").is_err());
        assert!(parse_hex_u16("zzzz").is_err());
    }

    #[test]
    fn speed_parsing_handles_units() {
        assert_eq!(parse_speed_mbps("480"), Ok(480));
        assert_eq!(parse_speed_mbps("480M"), Ok(480));
        assert_eq!(parse_speed_mbps("5G"), Ok(5000));
        assert_eq!(parse_speed_mbps("10Gbps"), Ok(10_000));
        assert_eq!(parse_speed_mbps("1.5m"), Ok(2));
        assert_eq!(parse_speed_mbps("0"), Ok(0));
        assert!(parse_speed_mbps("fast").is_err());
        assert!(parse_speed_mbps("-5").is_err());
        assert!(parse_speed_mbps("").is_err());
    }

    #[test]
    fn class_parser_lists_valid_names_on_error() {
        assert_eq!(parse_class("HID"), Ok(ClassSel::Hid));
        let e = parse_class("toaster").unwrap_err();
        assert!(e.contains("storage") && e.contains("typec"), "{e}");
    }

    #[test]
    fn vendor_parser_distinguishes_id_from_name() {
        assert_eq!(parse_vendor("046d"), Ok(VendorSel::Id(0x046d)));
        assert_eq!(parse_vendor("0x46d"), Ok(VendorSel::Id(0x46d)));
        assert_eq!(
            parse_vendor("Logitech"),
            Ok(VendorSel::Name("logitech".into()))
        );
        // 3 hex digits without 0x is a name, not an id.
        assert_eq!(parse_vendor("abc"), Ok(VendorSel::Name("abc".into())));
    }

    #[test]
    fn id_parser_requires_vid_colon_pid() {
        assert_eq!(parse_id("046d:c52b"), Ok((0x046d, 0xc52b)));
        assert!(parse_id("046d").is_err());
        assert!(parse_id("046d:zz").is_err());
    }

    #[test]
    fn invalid_hex_vendor_is_not_a_name() {
        assert!(parse_vendor("0xzzzz").is_err());
    }

    #[test]
    fn empty_filters_match_everything() {
        let f = FilterArgs::default();
        assert!(!f.is_active());
        assert!(f.matches(&usb(1, 1, "A", 480, 0)));
        assert!(f.matches(&port(0, Status::Empty)));
    }

    #[test]
    fn repeated_values_or_and_different_flags_and() {
        let kb = usb(0x046d, 1, "Logitech", 12, 0);
        let hub = usb(0x1a40, 2, "Terminus", 480, 9);
        let f = FilterArgs {
            classes: vec![ClassSel::Device(DeviceClass::Hub), ClassSel::Hid],
            ..Default::default()
        };
        assert!(f.matches(&hub));
        let f2 = FilterArgs {
            vendors: vec![VendorSel::Name("logi".into())],
            classes: f.classes.clone(),
            ..Default::default()
        };
        assert!(!f2.matches(&hub), "AND across flags");
        assert_eq!(f2.matches(&kb), ClassSel::Hid.matches(&kb));
    }

    #[test]
    fn identity_filters_only_match_usb_entries() {
        let p = port(0, Status::Empty);
        let f = FilterArgs {
            vendors: vec![VendorSel::Id(0)],
            ..Default::default()
        };
        assert!(!f.matches(&p));
        let d = usb(0x046d, 0xc52b, "Logitech", 12, 0);
        let f = FilterArgs {
            ids: vec![(0x046d, 0xc52b)],
            ..Default::default()
        };
        assert!(f.matches(&d));
        let f = FilterArgs {
            products: vec![0xc52c],
            ..Default::default()
        };
        assert!(!f.matches(&d));
    }

    #[test]
    fn speed_bounds_exclude_unknown_speed() {
        let unknown = usb(1, 1, "A", 0, 0);
        let slow = usb(1, 2, "A", 12, 0);
        let fast = usb(1, 3, "A", 5000, 0);
        let f = FilterArgs {
            min_speed_mbps: Some(480),
            ..Default::default()
        };
        assert!(!f.matches(&unknown) && !f.matches(&slow) && f.matches(&fast));
        let f = FilterArgs {
            max_speed_mbps: Some(480),
            ..Default::default()
        };
        assert!(!f.matches(&unknown) && f.matches(&slow) && !f.matches(&fast));
    }

    #[test]
    fn speed_range_validation() {
        let f = FilterArgs {
            min_speed_mbps: Some(5000),
            max_speed_mbps: Some(480),
            ..Default::default()
        };
        assert!(f.validate().unwrap_err().contains("greater"));
        assert!(FilterArgs::default().validate().is_ok());
    }

    #[test]
    fn charging_states_map_to_status_for_ports_only() {
        let charging = port(0, Status::Charging);
        let idle = port(1, Status::Connected);
        let none = port(2, Status::Empty);
        let dev = usb(1, 1, "A", 480, 0);
        let only = |c| FilterArgs {
            charging: vec![c],
            ..Default::default()
        };
        assert!(only(ChargeState::Charging).matches(&charging));
        assert!(only(ChargeState::Idle).matches(&idle));
        assert!(only(ChargeState::None).matches(&none));
        assert!(!only(ChargeState::Idle).matches(&charging));
        // A connected USB device is not an "idle port".
        assert!(!only(ChargeState::Idle).matches(&dev));
    }

    #[test]
    fn port_filter_selects_type_c_port_number() {
        let f = FilterArgs {
            ports: vec![1, 3],
            ..Default::default()
        };
        assert!(f.matches(&port(1, Status::Empty)));
        assert!(!f.matches(&port(2, Status::Empty)));
        assert!(!f.matches(&usb(1, 1, "A", 480, 0)));
    }

    #[test]
    fn unknown_class_excludes_type_c_ports() {
        let f = FilterArgs {
            classes: vec![ClassSel::Device(DeviceClass::Unknown)],
            ..Default::default()
        };
        assert!(!f.matches(&port(0, Status::Empty)));
        assert!(f.matches(&usb(1, 1, "A", 480, 0xfe)));
        let f = FilterArgs {
            classes: vec![ClassSel::TypeC],
            ..Default::default()
        };
        assert!(f.matches(&port(0, Status::Empty)));
    }

    #[test]
    fn severity_filter_is_a_minimum() {
        let mut warn = usb(1, 1, "A", 480, 0);
        warn.properties
            .push(("cable.data_speed_limit".into(), "USB 2.0".into()));
        let ok = usb(1, 2, "A", 480, 0);
        let f = FilterArgs {
            severity: Some(Severity::Warning),
            ..Default::default()
        };
        assert!(f.matches(&warn) && !f.matches(&ok));
        let f = FilterArgs {
            severity: Some(Severity::Ok),
            ..Default::default()
        };
        assert!(f.matches(&ok));
        let f = FilterArgs {
            severity: Some(Severity::Critical),
            ..Default::default()
        };
        assert!(!f.matches(&warn));
    }

    #[test]
    fn apply_preserves_order_and_describe_lists_filters() {
        let devs = vec![
            usb(1, 1, "A", 480, 0),
            usb(1, 2, "B", 5000, 0),
            usb(1, 3, "C", 5000, 0),
        ];
        let f = FilterArgs {
            min_speed_mbps: Some(1000),
            vendors: vec![VendorSel::Id(1)],
            ..Default::default()
        };
        let got = apply(&f, &devs);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].product_id, 2);
        assert_eq!(got[1].product_id, 3);
        assert_eq!(f.describe(), vec!["vendor=0001", "min-speed=1000Mbps"]);
    }
}
