//! E2E Test Harness: Shared test utilities, CLI execution, and synthetic sysfs fixtures.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub use crate::fixture_builder::TempRoot;
use crate::fixture_builder::*;
use usbeehive::{DeviceManager, Snapshot, Sysfs};

/// Construct a Command targeting the compiled `usbeehive` binary.
pub fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_usbeehive"))
}

/// Execute the CLI against a synthetic sysfs root with additional arguments.
/// Returns `(success, stdout, stderr)`.
pub fn run_cli(root: &Path, extra_args: &[&str]) -> (bool, String, String) {
    let mut cmd = cli();
    cmd.arg("--sysfs-root").arg(root);
    for arg in extra_args {
        cmd.arg(arg);
    }
    let out = cmd.output().expect("binary executes successfully");
    let success = out.status.success();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    (success, stdout, stderr)
}

/// Execute the CLI in `--json` mode against a synthetic sysfs root and parse output into JSON.
pub fn run_cli_json(root: &Path, extra_args: &[&str]) -> serde_json::Value {
    let mut args = vec!["--json"];
    args.extend_from_slice(extra_args);
    let (success, stdout, stderr) = run_cli(root, &args);
    assert!(
        success,
        "CLI failed with exit status false.\nStdout: {stdout}\nStderr: {stderr}"
    );
    serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!("Failed to parse CLI stdout as JSON: {e}\nStdout was:\n{stdout}")
    })
}

/// Create a `DeviceManager` pointed at the given synthetic sysfs root and refresh it.
pub fn manager_for(root: &Path) -> DeviceManager {
    let mut mgr = DeviceManager::with_sysfs(Sysfs::with_root(root));
    mgr.refresh();
    mgr
}

/// Create an owned `Snapshot` capturing the state of the given synthetic sysfs root.
pub fn snapshot_for(root: &Path) -> Snapshot {
    let mut mgr = DeviceManager::with_sysfs(Sysfs::with_root(root));
    mgr.refresh();
    mgr.snapshot().clone()
}

/// Create a standard USB Keyboard fixture (Logitech, VID 046d, PID c31c, HID class 0x03).
pub fn write_standard_keyboard(root: &Path, bus_port: &str) {
    let iface = InterfaceFixture {
        number: 0,
        class: 0x03, // HID
        sub_class: 0x01,
        protocol: 0x01,
        driver: "usbhid",
    };
    UsbDeviceFixture {
        bus_port,
        vendor: 0x046d,
        product: 0xc31c,
        product_name: "USB Keyboard",
        manufacturer: "Logitech",
        serial: "KB1001",
        speed_mbps: 12,
        max_power_ma: 100,
        version: "2.00",
        device_class: 0x00,
        bus_num: 1,
        dev_num: 2,
        interfaces: &[iface],
        removable: "removable",
    }
    .write(root);
}

/// Create a standard USB Mouse fixture (Logitech, VID 046d, PID c077, HID class 0x03).
pub fn write_standard_mouse(root: &Path, bus_port: &str) {
    let iface = InterfaceFixture {
        number: 0,
        class: 0x03, // HID
        sub_class: 0x01,
        protocol: 0x02,
        driver: "usbhid",
    };
    UsbDeviceFixture {
        bus_port,
        vendor: 0x046d,
        product: 0xc077,
        product_name: "Optical USB Mouse",
        manufacturer: "Logitech",
        serial: "MS2002",
        speed_mbps: 12,
        max_power_ma: 98,
        version: "2.00",
        device_class: 0x00,
        bus_num: 1,
        dev_num: 3,
        interfaces: &[iface],
        removable: "removable",
    }
    .write(root);
}

/// Create a standard USB Mass Storage device fixture (SanDisk Extreme, VID 0781, PID 5581).
pub fn write_standard_storage(root: &Path, bus_port: &str, speed_mbps: u32) {
    let iface = InterfaceFixture {
        number: 0,
        class: 0x08, // Mass Storage
        sub_class: 0x06,
        protocol: 0x50,
        driver: "uas",
    };
    UsbDeviceFixture {
        bus_port,
        vendor: 0x0781,
        product: 0x5581,
        product_name: "Ultra USB 3.0",
        manufacturer: "SanDisk",
        serial: "SD99887766",
        speed_mbps,
        max_power_ma: 896,
        version: if speed_mbps >= 5000 { "3.20" } else { "2.10" },
        device_class: 0x00,
        bus_num: 2,
        dev_num: 4,
        interfaces: &[iface],
        removable: "removable",
    }
    .write(root);
}

/// Create a standard USB Hub fixture (Genesys Logic, VID 05e3, PID 0610, Hub class 0x09).
pub fn write_standard_hub(root: &Path, bus_port: &str, maxchild: u32) {
    let iface = InterfaceFixture {
        number: 0,
        class: 0x09, // Hub
        sub_class: 0x00,
        protocol: 0x01,
        driver: "hub",
    };
    UsbDeviceFixture {
        bus_port,
        vendor: 0x05e3,
        product: 0x0610,
        product_name: "USB2.0 Hub",
        manufacturer: "GenesysLogic",
        serial: "",
        speed_mbps: 480,
        max_power_ma: 100,
        version: "2.00",
        device_class: 0x09,
        bus_num: 1,
        dev_num: 5,
        interfaces: &[iface],
        removable: "fixed",
    }
    .write(root);
    write_maxchild(root, bus_port, maxchild);
}

/// Create a Type-C Port directory with standard attributes.
pub fn write_standard_typec_port(
    root: &Path,
    port_name: &str,
    data_role: &str,
    power_role: &str,
    is_charging: bool,
) -> PathBuf {
    let mut fields = vec![
        ("data_role", data_role),
        ("power_role", power_role),
        ("port_type", "dual"),
        ("orientation", "normal"),
    ];
    if is_charging {
        fields.push(("usb_power_delivery_revision", "3.1"));
        fields.push(("power_operation_mode", "USB_POWER_DELIVERY"));
    }
    write_typec_port(root, port_name, &fields)
}

/// Create a complete PD charger profile with optional active PDO.
pub fn write_standard_pd_charger(
    root: &Path,
    pd_name: &str,
    parent_port: i32,
    max_watts: u32,
    _active_watts: u32,
) {
    let mut pdos = Vec::new();
    // 5V / 3A (15W) base fixed supply
    pdos.push(PdoFixture {
        voltage_mv: 5000,
        current_ma: 3000,
        power_mw: 15_000,
        kind: "fixed_supply",
        min_voltage_mv: 0,
        max_voltage_mv: 0,
    });

    if max_watts >= 27 {
        pdos.push(PdoFixture {
            voltage_mv: 9000,
            current_ma: 3000,
            power_mw: 27_000,
            kind: "fixed_supply",
            min_voltage_mv: 0,
            max_voltage_mv: 0,
        });
    }

    if max_watts >= 45 {
        pdos.push(PdoFixture {
            voltage_mv: 15_000,
            current_ma: 3000,
            power_mw: 45_000,
            kind: "fixed_supply",
            min_voltage_mv: 0,
            max_voltage_mv: 0,
        });
    }

    if max_watts >= 60 {
        let current = if max_watts >= 100 { 5000 } else { 3000 };
        pdos.push(PdoFixture {
            voltage_mv: 20_000,
            current_ma: current,
            power_mw: max_watts * 1000,
            kind: "fixed_supply",
            min_voltage_mv: 0,
            max_voltage_mv: 0,
        });
    }

    // Write PD directory
    let dir = root.join("class/usb_power_delivery").join(pd_name);
    let caps = dir.join("source-capabilities");
    fs::create_dir_all(&caps).unwrap();
    if parent_port >= 0 {
        write_attr(&dir, "parent_port_number", &parent_port.to_string());
    }
    fs::create_dir_all(caps.join("power")).unwrap();

    for (i, p) in pdos.iter().enumerate() {
        let entry = caps.join(format!("{}:fixed_supply", i + 1));
        fs::create_dir_all(&entry).unwrap();
        write_attr(&entry, "type", p.kind);
        write_attr(&entry, "voltage", &format!("{}mV", p.voltage_mv));
        write_attr(&entry, "maximum_current", &format!("{}mA", p.current_ma));
        write_attr(&entry, "maximum_power", &format!("{}mW", p.power_mw));
    }
}

/// Create a standard Type-C cable with specific wattage rating, vendor ID, and optional dirty reserved bits.
pub fn write_standard_cable(
    root: &Path,
    port_name: &str,
    cable_max_w: u32,
    is_active: bool,
    vendor_id: u16,
    dirty_bits: bool,
) {
    let ptype = if is_active { 4u32 } else { 3u32 };
    let id_hdr = (ptype << 27) | (vendor_id as u32);

    let (speed_bits, cur_bits, vbus_bits) = match cable_max_w {
        240 | 250 => (2u32, 2u32, 3u32), // Gen2, 5A, 50V (250W max)
        100 => (2u32, 2u32, 0u32),       // Gen2, 5A, 20V (100W max)
        60 => (1u32, 1u32, 0u32),        // Gen1, 3A, 20V (60W max)
        _ => (0u32, 1u32, 0u32),         // USB 2.0, 3A, 20V (60W max)
    };

    let mut cable_vdo = speed_bits | (cur_bits << 5) | (vbus_bits << 9);
    if dirty_bits {
        // Set reserved bit 3 and bit 7 defined as reserved in USB-PD Cable VDO
        cable_vdo |= (1 << 3) | (1 << 7);
    }

    let c_type = if is_active { "active" } else { "passive" };
    write_typec_cable(
        root,
        port_name,
        c_type,
        "type-c",
        &[
            ("id_header", id_hdr),
            ("cert_stat", 0),
            ("product", 0x1000),
            ("product_type_vdo1", cable_vdo),
        ],
    );
}

/// Create a Type-C partner with vendor ID and product ID.
pub fn write_standard_partner(root: &Path, port_name: &str, vendor_id: u16, product_id: u32) {
    let id_hdr = (2u32 << 27) | (vendor_id as u32);
    write_typec_partner(
        root,
        port_name,
        "device",
        &[
            ("id_header", id_hdr),
            ("cert_stat", 0),
            ("product", product_id),
        ],
    );
}
