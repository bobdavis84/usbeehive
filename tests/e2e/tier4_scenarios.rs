//! Tier 4: Real-World Application Scenarios (>=5 multi-device topologies).

use crate::e2e::harness::*;
use crate::fixture_builder::*;
use usbeehive::diagnostic::Bottleneck;
use usbeehive::thunderbolt::system_has_usb4_link;

// =========================================================================
// Scenario 1: USB-C Multi-Port Dock Topology
// =========================================================================
#[test]
fn t4_scenario_usb_c_multi_port_dock() {
    let root = TempRoot::new("t4-s1-dock");

    // 1. Upstream Type-C port from Laptop to Dock (100W PD charging)
    write_standard_typec_port(root.path(), "port0", "host [device]", "source [sink]", true);
    write_standard_partner(root.path(), "port0", 0x17ef, 0xa391); // Lenovo ThinkPad Dock VID:PID
    write_standard_cable(root.path(), "port0", 100, false, 0x17ef, false);
    write_standard_pd_charger(root.path(), "pd0", 0, 100, 100);
    link_partner_pd(root.path(), "port0", "pd0");

    // 2. Hub inside dock (port "1-1")
    write_standard_hub(root.path(), "1-1", 4);

    // 3. Downstream devices attached to dock hub:
    // Port 1-1.1: Keyboard
    write_standard_keyboard(root.path(), "1-1.1");
    // Port 1-1.2: Mouse
    write_standard_mouse(root.path(), "1-1.2");
    // Port 1-1.3: USB 3.0 Storage
    write_standard_storage(root.path(), "1-1.3", 5000);

    // Verify via CLI --json
    let json = run_cli_json(root.path(), &[]);
    let arr = json.as_array().expect("array of devices");
    assert_eq!(arr.len(), 5); // 1 Type-C port + 1 Hub + 3 USB peripherals

    let port = arr
        .iter()
        .find(|d| d["category"] == "typec")
        .expect("dock port");
    assert_eq!(port["status"], "Charging");
    assert!(port["charging"].is_object());
    assert_eq!(port["charging"]["isWarning"], false);

    // Verify via CLI tree output
    let (ok, stdout, _) = run_cli(root.path(), &[]);
    assert!(ok);
    assert!(stdout.contains("USB2.0 Hub"));
    assert!(stdout.contains("USB Keyboard"));
    assert!(stdout.contains("Optical USB Mouse"));
    assert!(stdout.contains("Ultra USB 3.0"));
}

// =========================================================================
// Scenario 2: Laptop on Undersized 15W Phone Charger with 60W Cable
// =========================================================================
#[test]
fn t4_scenario_laptop_on_undersized_15w_charger() {
    let root = TempRoot::new("t4-s2-undersized");
    let controller = "USBC000:00";

    // Laptop plugged into a 15W (5V/3A) phone charger using a 60W cable
    write_typec_port_under_ucsi(
        root.path(),
        controller,
        "port0",
        &[
            ("data_role", "host [device]"),
            ("power_role", "source [sink]"),
            ("port_type", "dual"),
            ("orientation", "normal"),
            ("usb_power_delivery_revision", "3.1"),
            ("power_operation_mode", "USB_POWER_DELIVERY"),
        ],
    );
    write_standard_partner(root.path(), "port0", 0x04e8, 0x1000); // Samsung charger
    write_standard_cable(root.path(), "port0", 60, false, 0x04e8, false);
    // 15W charger max (5V / 3A)
    write_standard_pd_charger(root.path(), "pd0", 0, 15, 15);
    link_partner_pd(root.path(), "port0", "pd0");

    // Laptop requests 45W via UCSI power supply (15V @ 3A)
    write_ucsi_source_psy(
        root.path(),
        controller,
        1, // port0 -> port_index 1
        &[
            ("online", "1"),
            ("voltage_now", "15000000"), // 15V in µV
            ("current_now", "3000000"),  // 3A in µA -> 45W requested
            ("voltage_max", "20000000"),
        ],
    );

    let snap = snapshot_for(root.path());
    let port = &snap.summaries[0];
    let diag = port.charging_diag.as_ref().expect("diagnostic present");

    // Charger itself is max 15W (undersized charger limit)
    assert_eq!(diag.bottleneck, Bottleneck::ChargerLimit);
    assert!(diag.is_warning);
    assert!(diag.summary.contains("15W"));

    // CLI list mode text rendering works without panic
    let (ok, stdout, _) = run_cli(root.path(), &["--list"]);
    assert!(ok);
    assert!(stdout.contains("Samsung"));
    assert!(stdout.contains("15W"));
}

// =========================================================================
// Scenario 3: USB4 / Thunderbolt 4 Daisy-Chain Topology
// =========================================================================
#[test]
fn t4_scenario_usb4_daisy_chain() {
    let root = TempRoot::new("t4-s3-daisy");

    // Host PC: Intel Maple Ridge USB4 controller (0-0)
    write_thunderbolt_router(
        root.path(),
        "0-0",
        4,
        &[
            ("vendor_name", "Intel"),
            ("device_name", "Maple Ridge USB4 Host"),
            ("unique_id", "00000000-0000-0000-0000-000000000001"),
        ],
    );

    // Hop 1: CalDigit TS4 Dock (0-1)
    write_thunderbolt_router(
        root.path(),
        "0-1",
        4,
        &[
            ("vendor_name", "CalDigit"),
            ("device_name", "TS4 Thunderbolt 4 Dock"),
            ("unique_id", "00000000-0000-0000-0000-000000000002"),
        ],
    );

    // Hop 2: External NVMe Storage Enclosure; Linux route 0x101 = two hops.
    write_thunderbolt_router(
        root.path(),
        "0-101",
        4,
        &[
            ("vendor_name", "OWC"),
            ("device_name", "Envoy Pro FX NVMe"),
            ("unique_id", "00000000-0000-0000-0000-000000000003"),
        ],
    );

    let snap = snapshot_for(root.path());
    assert_eq!(snap.thunderbolt_routers.len(), 3);
    assert!(system_has_usb4_link(&snap.thunderbolt_routers));

    let host = snap
        .thunderbolt_routers
        .iter()
        .find(|r| r.is_host)
        .expect("host router");
    assert_eq!(host.route, "0-0");
    assert_eq!(host.generation, 4);

    let endpoints: Vec<_> = snap
        .thunderbolt_routers
        .iter()
        .filter(|r| !r.is_host)
        .collect();
    assert_eq!(endpoints.len(), 2);
}

// =========================================================================
// Scenario 4: Counterfeit Cable Bottlenecking High-Power Laptop
// =========================================================================
#[test]
fn t4_scenario_counterfeit_cable_bottleneck() {
    let root = TempRoot::new("t4-s4-counterfeit");

    // 140W USB-PD 3.1 EPR charger
    write_standard_typec_port(root.path(), "port0", "host [device]", "source [sink]", true);
    write_standard_partner(root.path(), "port0", 0x05ac, 0x1000);

    // Cheap counterfeit cable:
    // Reports VID 0x0000, dirty reserved bits, and only rated 60W (3A / 20V)
    write_standard_cable(root.path(), "port0", 60, false, 0x0000, true);

    // Charger offers up to 100W/140W
    write_standard_pd_charger(root.path(), "pd0", 0, 100, 100);
    link_partner_pd(root.path(), "port0", "pd0");

    let snap = snapshot_for(root.path());
    let port = &snap.summaries[0];

    // R2 Bottleneck validation
    let diag = port.charging_diag.as_ref().expect("charging diagnostic");
    assert_eq!(diag.bottleneck, Bottleneck::CableLimit);
    assert!(diag.is_warning);
    assert!(diag.detail.contains("60W"));
    assert!(diag.detail.contains("100W"));

    // R7 Cable trust validation
    let cable = port.cable.as_ref().expect("cable info");
    assert!(cable.trust.zero_vid);
    assert!(cable.trust.reserved_bits_set);
    assert!(cable.trust.any());

    // CLI JSON reflects both warning and trust signals
    let json = run_cli_json(root.path(), &[]);
    let port_json = &json.as_array().unwrap()[0];
    assert_eq!(port_json["charging"]["isWarning"], true);
    assert_eq!(port_json["cable"]["vendorId"], "0x0000");
}

// =========================================================================
// Scenario 5: Hotplug Life Cycle & State Restoration
// =========================================================================
#[test]
fn t4_scenario_hotplug_life_cycle_recovery() {
    let root = TempRoot::new("t4-s5-hotplug");

    // Stage 1: Laptop with internal keyboard
    write_standard_keyboard(root.path(), "1-1");
    let snap_stage1 = snapshot_for(root.path());
    assert_eq!(snap_stage1.usb_devices.len(), 1);

    // Stage 2: External high-speed drive plugged in
    write_standard_storage(root.path(), "2-1", 10_000);
    let snap_stage2 = snapshot_for(root.path());
    let diff_1to2 = snap_stage2.diff(&snap_stage1);
    assert_eq!(diff_1to2.added.len(), 1);
    assert!(diff_1to2.added[0].contains("2-1"));
    assert!(diff_1to2.removed.is_empty());

    // Stage 3: Drive removed
    std::fs::remove_dir_all(root.path().join("bus/usb/devices/2-1")).unwrap();
    let snap_stage3 = snapshot_for(root.path());
    let diff_2to3 = snap_stage3.diff(&snap_stage2);
    assert!(diff_2to3.added.is_empty());
    assert_eq!(diff_2to3.removed.len(), 1);
    assert!(diff_2to3.removed[0].contains("2-1"));

    // Stage 4: Drive reconnected
    write_standard_storage(root.path(), "2-1", 10_000);
    let snap_stage4 = snapshot_for(root.path());
    let diff_3to4 = snap_stage4.diff(&snap_stage3);
    assert_eq!(diff_3to4.added.len(), 1);
    assert!(diff_3to4.added[0].contains("2-1"));

    // Stage 4 relative to Stage 2 must be identical (no diff)
    let diff_2to4 = snap_stage4.diff(&snap_stage2);
    assert!(diff_2to4.is_empty());
}
