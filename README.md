# usbeehive 🐝

> **What can this USB cable actually do?**

A command-line tool, and a Rust library, that tell you in plain English
what each USB device plugged into your Linux machine can actually do.

**usbeehive is a Linux port of [WhatCable](https://github.com/darrylmorley/whatcable),
a macOS menu-bar app by [Darryl Morley](https://github.com/darrylmorley).**
This port covers all USB devices, not just USB-C, while preserving the
rich USB-C Power Delivery diagnostics from the original.

The Rust rewrite is forked from
[Zetaphor/whatcable-linux](https://github.com/Zetaphor/whatcable-linux)
(originally C++ / CMake).

> **Renamed from `whatcable`.** This project was previously published on
> crates.io as [`whatcable`](https://crates.io/crates/whatcable) — at the
> original author's request, it has been renamed to `usbeehive` as of
> 0.5.0. The old crate is retired (its final release is a redirect to
> here). Use `cargo install usbeehive` going forward.

## What it shows

### All USB devices
- **Identity**: vendor, product name, serial number
- **Speed**: negotiated link speed (1.5 Mbps to 20 Gbps)
- **USB version**: 1.1, 2.0, 3.0, 3.1, 3.2
- **Power allowance**: declared USB bus-power demand, not measured consumption
- **Device type**: HID, Audio, Mass Storage, Hub, etc.
- **Driver**: which kernel driver is handling the device
- **Topology**: hub hierarchy showing what's plugged into what

### USB-C ports (additional detail)
- **Port roles**: data role (host/device), power role (source/sink)
- **Cable e-marker info**: cable speed capability, current rating (3A/5A), active vs passive, cable vendor
- **Charger PDO list**: every voltage / current profile the charger advertises, with the active profile highlighted
- **Charging diagnostics**: identifies bottlenecks — cable limiting speed, charger undersized, etc.
- **Requested operating power** (UCSI): `voltage_now × current_now`, exposed as `negotiatedPowerMW` in JSON output and via `TypeCPowerSupply::negotiated_power_mw()` in the library. These kernel values describe the negotiated operating point, not a power-meter reading.
- **Partner identity**: decoded from PD Discover Identity VDOs

### Mock-data UI component

[`ui/DeviceCard.tsx`](ui/DeviceCard.tsx) is a standalone React/TypeScript
component for `headline`, `subtitle`, and nullable `charging_diag` fields from
the D-Bus `SnapshotJson` shape. It uses caller-provided data only; no live D-Bus
or Rust calls are wired into the component. See [`ui/README.md`](ui/README.md)
for usage and tests, and [`docs/MOCK_DATA.md`](docs/MOCK_DATA.md) for the privacy-
sanitized real CLI and D-Bus fixture captures.

## Install

### From crates.io

```bash
cargo install usbeehive
```

To run the [usbee](https://github.com/abrauchli/usbee) GNOME Shell
extension, install with the D-Bus daemon enabled:

```bash
cargo install usbeehive --features dbus
```

`--features dbus` builds the `usbeehived` daemon, which compiles C code and
needs a C compiler (`cc`) plus the `libudev` development headers:

```bash
# Ubuntu / Debian
sudo apt install gcc libudev-dev
```

The core library and default CLI support Rust 1.85. The optional D-Bus build
requires Rust 1.87 with the currently locked zbus dependency stack. Both
configurations are checked in CI.

### Build from source

Requires Rust 1.85+ ([rustup](https://rustup.rs)) and `libudev` development
headers (for `--watch` hotplug support, on by default).

```bash
# Ubuntu / Debian
sudo apt install libudev-dev pkg-config

# Fedora
sudo dnf install systemd-devel pkgconf-pkg-config

# Arch / Manjaro
sudo pacman -S --needed systemd-libs pkgconf
```

```bash
cargo build --release                                  # default (with --watch)
cargo build --release --no-default-features --features cli,sysfs    # no libudev
sudo install -Dm755 target/release/usbeehive /usr/local/bin/usbeehive
```

### Tests

```bash
cargo test                                # full suite (requires libudev-dev)
cargo test --no-default-features          # pure-decoder subset, no libudev
```

## CLI usage

```bash
usbeehive                                  # human-readable summary of every USB device
usbeehive --json                           # structured JSON output
usbeehive --watch                          # stream updates as devices come and go
usbeehive --raw                            # include raw sysfs attributes
usbeehive --sysfs-root /tmp/fixture-root   # run against a captured tree (testing)
usbeehive --class storage --min-speed 480  # filter by class and negotiated speed
usbeehive --vendor 046d --severity warning # filter by VID and warning level
usbeehive --summary --json                 # aggregate counts and per-port view
usbeehive --format csv                     # export an inventory as CSV
usbeehive --format report                  # human-readable report
usbeehive --altmodes                       # Type-C modes, Billboard and USB4 caveats
usbeehive --altmodes --json                # structured alternate-mode report
usbeehive --save-snapshot before.json      # save full snapshot envelope
usbeehive diff before.json after.json      # compare saved snapshots
usbeehive --watch --history --json         # stream transition events as NDJSON
usbeehive --history --samples 10            # bounded capture without waiting for Ctrl-C
usbeehive --version
usbeehive --help
```

Filters with different names combine with AND; repeating one selector combines
its values with OR. Unknown values do not match a filter requiring that data.
`--min-speed` / `--max-speed` accept Mbps values (`480`, `5G`, `10Gbps`).
`--severity` accepts `ok`, `info`, `warning`, or `critical`. Filtered text is a
flat listing; the no-filter default tree and legacy `--json` array are preserved.
`--summary` accepts `--json`; `--format` accepts `csv`, `json`, or `report`.
`--save-snapshot` writes a serde `Snapshot` object, not the legacy CLI `--json`
array. For live comparison, omit the second `diff` path or use the literal `live`.
Place `--sysfs-root PATH` before a subcommand when testing fixture trees.

`--altmodes` presents the best-effort DisplayPort, Thunderbolt 3 alt-mode,
Billboard, and system-wide USB4/Thunderbolt view. Its report explicitly
distinguishes absent from unrecorded subsystems and does not attribute a
system-wide USB4 router to an individual Type-C port. Live tunnel state and the
active DisplayPort pin assignment are not exposed by the supported sysfs API.

For scripts, use the documented CLI shape directly, for example:

```sh
usbeehive --json | jq '.[] | {headline, vendorId, productId, linkSpeedMbps}'
usbeehive --summary --json | jq '{total, countsByType, pdBottlenecks}'
usbeehive diff before.json after.json --json | jq '.modified[] | {id, link_speed_mbps}'
```

CLI JSON and D-Bus `SnapshotJson` are distinct contracts with different field
names and nested value types; see [`docs/SCHEMA.md`](docs/SCHEMA.md).

## Library usage

The crate has three optional layers, each behind a Cargo feature so
consumers pull in only what they need.

| Feature | Default | Adds |
|---|---|---|
| (none) | always | Data types, USB-PD VDO decoders, diagnostics, summaries — IO-free. |
| `sysfs` | yes | `Sysfs` handle + `DeviceManager` — Linux `/sys` enumeration with injectable root. |
| `watch` | yes | libudev hotplug monitor: `watch::Watcher` + `watch::run_loop`. |
| `cli` | yes | The `usbeehive` binary (clap + JSON / text rendering). |
| `dbus` | no | `usbeehive::dbus` interface module + the `usbeehived` daemon binary publishing `org.usbeehive.Devices5` on the session bus (implies `watch`). |

Library-only consumers can drop the binary deps:

```toml
# Pure decoders, no IO, no libudev:
usbeehive = { version = "0.5", default-features = false }

# Add /sys enumeration:
usbeehive = { version = "0.5", default-features = false, features = ["sysfs"] }

# Add hotplug too:
usbeehive = { version = "0.5", default-features = false, features = ["watch"] }
```

### Decode a Cable VDO

```rust
use usbeehive::pd::{decode_cable_vdo, CableSpeed, CableCurrent};

let v = decode_cable_vdo(2 | (2 << 5) | (3 << 9), false);
assert_eq!(v.speed, CableSpeed::Usb32Gen2);
assert_eq!(v.current_rating, CableCurrent::FiveAmp);
assert_eq!(v.max_watts, 250);
```

### Enumerate the system's USB tree

```rust,no_run
use usbeehive::DeviceManager;

let mut mgr = DeviceManager::new();
mgr.refresh();
for s in mgr.devices() {
    println!("{}: {}", s.headline, s.subtitle);
}
```

### Run a debounced render loop on hotplug events

```rust,no_run
use std::time::Duration;
use usbeehive::{watch::run_loop, DeviceManager};

let mut mgr = DeviceManager::new();
run_loop(Duration::from_millis(500), |_reason| {
    mgr.refresh();
    println!("snapshot has {} devices", mgr.devices().len());
    Ok(())
}).unwrap();
```

### Compare snapshots and retain recent transitions

```rust,no_run
use usbeehive::{compare, history::SnapshotHistory, DeviceManager};

let mut manager = DeviceManager::new();
manager.refresh();
let before = manager.snapshot().clone();
// Plug in or change an adapter before taking the second capture.
manager.refresh();
let after = manager.snapshot().clone();
let diff = compare::compare(&before, &after);
println!("{}", diff.render_text());

let mut history = SnapshotHistory::with_capacity(100);
history.push(before, 1_000); // timestamps supplied by the caller, in milliseconds
for transition in history.push(after, 2_000) {
    println!("{transition}");
}
// history.timeline() includes the timestamp and sequence of each retained event.
```

`Snapshot`, `compare`, `history`, and the diagnostic/trust models are IO-free
and available with `default-features = false`; callers provide snapshots and
timestamps. `DeviceManager` above requires `sysfs`. JSON convenience methods
require `cli` or `dbus`; pure-core consumers can use their own `serde` serializer.
History is a bounded in-memory buffer, not a background recorder: callers push
each capture, the first establishes a baseline, and oldest entries are evicted.

See [`examples/`](examples) for more and [`docs/SCHEMA.md`](docs/SCHEMA.md) for
field contracts, units, nullability, and compatibility rules.

## D-Bus daemon (optional)

Build with the `dbus` feature to get a long-running daemon, `usbeehived`,
that publishes the live snapshot on the session bus. Useful for desktop
applets (KDE / GNOME / tray apps) that want to react to cable plugs and
charging-bottleneck changes without each one re-implementing sysfs
enumeration. The [usbee](https://github.com/abrauchli/usbee) GNOME Shell
extension is one such client.

```bash
cargo build --release --no-default-features --features dbus
./target/release/usbeehived                                # foreground
```

Building the `dbus` feature requires a C compiler (`cc`) and the `libudev`
development headers — on Ubuntu / Debian: `sudo apt install gcc libudev-dev`.

Wire surface — `org.usbeehive.Devices5` at `/org/usbeehive/Devices`:

Each `ListDevices` entry is a 21-field tuple:
`(id, category, device_class, device_subclass, status, headline, subtitle, icon, vendor, product, vendor_id, product_id, primary_driver, properties, port_number, link_speed_mbps, usb_version, power, charging_diag, pdo_list, active_pdo_index)`. See [`src/dbus.rs`](src/dbus.rs) module docs for per-field semantics.

| Member | Kind | Notes |
|---|---|---|
| `ListDevices` | method `() → a(ssssssssssqqsa(ss)ius(uuus)(bsssb)a(usuuuub)i)` | One structured entry per summary. Properties are `(machine_key, value)` pairs; `pdo_list` is the structured source-PDO advertisement. |
| `ListPorts` | method `() → ai` | Type-C `port_number`s currently exposed. |
| `Diagnose` | method `(i) → (bsssb)` | Charging diagnostic for a port — same shape as the per-entry `charging_diag`. `present == false` when none available. |
| `SnapshotJson` | method `() → s` | Full serde `DeviceSummary` array (snake_case), **not** the CLI's camelCase `--json` array. See [schema contracts](docs/SCHEMA.md). |
| `Refresh` | method `() → u` | Force a re-enumeration; returns the new device count. |
| `Version` / `DeviceCount` | properties | Crate version + summary count. |
| `DeviceAdded` / `DeviceRemoved` | signals | Fire when a device or port appears / disappears. |
| `CapabilityDegraded` / `CapabilityRestored` | signals | Fire when a port's charging diagnostic newly raises (or clears) `is_warning` — e.g. a too-thin cable plugged into a beefy charger. |

Quick poke from the shell:

```sh
gdbus call --session --dest org.usbeehive.Devices \
    --object-path /org/usbeehive/Devices \
    --method org.usbeehive.Devices5.ListDevices
```

A minimal Rust client lives in [`examples/dbus_client.rs`](examples/dbus_client.rs).

### Devices5 migration

`Devices4` was retired in favor of `Devices5`, continuing the honest-units
theme: USB-PD wattages on the wire are *negotiated ceilings*, never
measured flow — UCSI reports the contract's operating point, and a sink
that is e.g. battery-charge-limited may request far less than its
contract allows. The break:

- The per-entry `power` tuple grows a field: `(uus)` → `(uuus)`, now
  `(power_in_mw, power_out_mw, contract_mw, power_role)`. `power_in_mw`
  remains the sink's *requested* operating power ("up to N W"); the new
  `contract_mw` is what the active PDO contract *allows* (`0` = no
  contract inferred). `contract_mw > power_in_mw` means the sink is
  limiting its own draw, not the cable or charger.
- New `bottleneck` variant `SinkLimit` (always `is_warning == false`):
  contract healthy, sink requesting < 80% of it — typically a battery
  charge limit or thermal policy. Falls under the existing
  unknown-variant rule for older clients.

Update the interface name and any positional unpacking of the `power`
tuple (`power_role` moves from index 2 to 3); every method/signal is
otherwise unchanged. Adding new
enum variants (`device_class`, `status`, `power_role`, `bottleneck`) is
non-breaking; clients MUST treat any unrecognized string as `Unknown` /
fall back to category-based behavior. Machine-keyed flag properties
(`cable.trust.*`, `transport.*`) are pushed only when their flag fires —
absence is the "off" state. The CHANGELOG has the full break-by-break
migration table.

## How it works

usbeehive reads the following Linux sysfs interfaces. No root access
required for basic info:

| sysfs path | Provides |
|---|---|
| `/sys/bus/usb/devices/` | All USB devices: vendor, product, speed, power, class, interfaces, topology |
| `/sys/class/typec/` | USB-C port state: connection, roles, cable e-marker, partner identity |
| `/sys/class/usb_power_delivery/` | PD negotiation: PDO list from charger, active profile, PPS ranges |
| `/sys/class/power_supply/ucsi-source-psy-*` | Negotiated/requested operating point from `voltage_now × current_now` |
| `/sys/bus/thunderbolt/devices/` | Thunderbolt/USB4 routers, domain security policy, optional link information |

Hotplug uses `libudev` to detect USB, Type-C, power-supply/PD, and Thunderbolt
events. Changes are observable only when the kernel/driver exposes the data
and emits an event; absence of an event is not evidence that nothing changed.

Cable speed and power decoding follow the USB Power Delivery 3.x spec,
ported from the original WhatCable's Swift implementation.

## Caveats

- **USB-C / PD data availability varies by hardware.** The Type-C connector
  class and USB PD sysfs interfaces depend on the kernel driver
  (UCSI, TCPM, platform-specific). Some systems expose full PD negotiation
  data; others expose only basic port info or nothing at all.
- **Cable e-marker info only appears for cables that carry one.** Same as
  the original — most USB-C cables under 60W are unmarked.
- **Cable trust is an evidence-based heuristic, not certification.**
  `KnownGood` means available descriptor data is internally consistent, not
  that copper gauge, thermal behavior, safety, or genuine USB-IF certification
  was independently verified. `LikelyConstrained` describes a capability limit;
  `Inconclusive` means insufficient evidence; `Suspicious` flags a mismatch or
  malformed claim worth investigating, not proof of counterfeiting.
- **A cable bottleneck may be uncertain when operating demand is unavailable.**
  A reported UCSI sink request is compared with the cable rating; without it,
  the selected source PDO or charger advertisement is only a capability ceiling
  and the diagnosis is downgraded to medium/low confidence. A PDO is not a
  wattmeter reading and does not prove a physical cable fault.
- **False positives and negatives are expected.** A legitimate new VID may be
  missing from the local database; unusual firmware can report incomplete or
  contradictory data. A clone can copy a real VID and clean VDOs and escape
  detection. Missing e-marker telemetry is common on UCSI systems and does not
  prove a cable is unmarked. Low requested current alone does not prove poor
  wire quality; battery/thermal policy, charger sharing, ports, adapters, and
  contact problems can all constrain operation. Compare known-good hardware
  and corroborate with a suitable meter before drawing safety conclusions.
- **Vendor name lookup is not exhaustive.** Common vendors are recognized;
  others show the hex VID.

## Credits

Original macOS app: [WhatCable](https://github.com/darrylmorley/whatcable)
by [Darryl Morley](https://github.com/darrylmorley) — and thanks to Darryl
for suggesting the new name when the Rust port outgrew its borrowed one.
The USB Power Delivery decoding logic, charging diagnostics, vendor
database, and plain-English summary approach are derived from the original
macOS app.

Upstream Linux/KDE C++ port: [Zetaphor/whatcable-linux](https://github.com/Zetaphor/whatcable-linux).

## License

[MIT](LICENSE)
