# Test Infrastructure & Test Suite Architecture (Milestone M_E2E)

`usbeehive` test infrastructure specification and execution guide for the opaque-box end-to-end (E2E) test suite across Requirements R1 through R7.

---

## 1. Overview & Architectural Principles

The `usbeehive` E2E test suite provides opaque-box verification of CLI and library interfaces under Linux `/sys` conditions. Because hardware USB topologies are non-deterministic and hardware-dependent, the test harness utilizes synthetic, hermetic sysfs trees rooted in isolated temporary directories.

### Core Principles

1. **Opaque-Box Boundary Testing**: Tests execute against external interfaces:
   - **CLI Binary**: Process execution via `std::process::Command` targeting `env!("CARGO_BIN_EXE_usbeehive")` with `--sysfs-root <path>`.
   - **Public Library Interface**: Re-exports in `usbeehive` (`Sysfs::with_root`, `DeviceManager`, `Snapshot`, `SnapshotDiff`, `ChargingDiagnostic`, `CableInfo`, `ThunderboltRouter`).
2. **Progressive Testability**: Tests rely strictly on completed or stable interface contracts defined in `PROJECT.md` and `ORIGINAL_REQUEST.md`.
3. **Hermetic Isolation**: Every test creates its own `TempRoot` scratch directory, operates in complete isolation, and automatically removes all synthetic files on `Drop`.
4. **Deterministic Oracle Grounding**: Test inputs and expected outputs are derived from USB-PD 3.1 specifications, USB 3.2 / USB4 specifications, and explicit requirements in `ORIGINAL_REQUEST.md`.
5. **Zero Mod-Src Pollution**: Test files live strictly in `tests/` (`tests/e2e_suite.rs`, `tests/e2e/`), with zero modifications to production `src/` files.

---

## 2. 4-Tier Test Architecture

The test suite is structured into four distinct tiers:

```
tests/
├── e2e_suite.rs                   # Top-level integration test entrypoint
└── e2e/
    ├── mod.rs                     # Module root
    ├── harness.rs                 # Synthetic tree builders & CLI runners
    ├── tier1_features.rs          # Tier 1: Feature Coverage (R1-R7 Happy Path)
    ├── tier2_boundaries.rs        # Tier 2: Boundary & Corner Cases (R1-R7)
    ├── tier3_combinations.rs      # Tier 3: Cross-Feature Pairwise Interactions
    └── tier4_scenarios.rs         # Tier 4: Real-World Multi-Device Topologies
```

### Tier Breakdown

| Tier | Focus | Test Count | Description |
|---|---|---|---|
| **Tier 1** | Feature Coverage (Happy Path) | 35 (>=5 per area) | Verifies each feature requirement (R1–R7) in isolation with standard, valid inputs. |
| **Tier 2** | Boundary & Corner Cases | 35 (>=5 per area) | Verifies error recovery, empty states, boundary values, malformed inputs, and dirty flags. |
| **Tier 3** | Cross-Feature Combinations | 6 | Verifies pairwise interactions between subsystems (e.g. History + Trust, Diff + Filtering, Alt-Mode + Diagnostics). |
| **Tier 4** | Real-World Application Scenarios | 5 | Verifies complex multi-device topologies (Docks, USB4 daisy chains, undersized chargers, counterfeit cables, hotplug cycles). |
| **Total** | Full Suite | **81 tests** | 100% automated pass with `cargo test`. |

---

## 3. Test Harness Components (`tests/e2e/harness.rs`)

The harness provides high-level builders and assertions on top of `tests/fixture_builder.rs`:

- `TempRoot`: Creates a unique temporary directory `/tmp/wcfix-e2e-<pid>-<counter>` and removes it upon drop.
- `cli()`: Constructs a `std::process::Command` targeting the compiled `usbeehive` binary.
- `run_cli(root, args)`: Executes the CLI against a synthetic sysfs root and captures `(ExitStatus, stdout, stderr)`.
- `run_cli_json(root, args)`: Executes the CLI with `--json` and parses stdout into a `serde_json::Value`.
- `write_keyboard(root, bus_port)`: Creates a standard USB HID keyboard fixture.
- `write_storage(root, bus_port, speed)`: Creates a standard USB Mass Storage device fixture.
- `write_hub(root, bus_port, maxchild)`: Creates a USB hub with downstream ports.
- `write_charger_and_cable(root, port_name, charger_w, cable_w, active_w)`: Assembles a complete Type-C port + partner + cable + PD port profile.
- `write_usb4_topology(root, host_gen, device_gen)`: Assembles host and device Thunderbolt/USB4 routers.

---

## 4. Requirement Mapping (R1–R7)

### R1. JSON Output Schema & Structure
- **T1_R1**: Valid JSON array, standard device properties, Type-C charging output, raw sysfs attributes, empty array on empty sysfs.
- **T2_R1**: Missing optional attributes, non-UTF8 attribute handling, extreme voltage/current values, unparsed descriptors, special characters in product names.

### R2. Diagnostic Reasoning & Actionability
- **T1_R2**: Cable bottleneck warning (100W charger + 60W cable), unconstrained charging (100W + 250W cable), device self-limit (<80% of max), sink draw policy limit, non-e-marked cable pinned at 3A.
- **T2_R2**: Zero-wattage charger, 0mA request handling, charger offering exactly 3A, missing source PDO capabilities, unlinked PD port.

### R3. Historical Snapshot & State Transitions
- **T1_R3**: Sequential snapshot capture, device connection event, device disconnection event, power contract renegotiation transition, link degradation transition.
- **T2_R3**: Transition between identical snapshots (empty diff), transition from empty to populated, rapid hotplug oscillation, port role swap transition, warning resolution transition.

### R4. Snapshot Comparison Tooling
- **T1_R4**: Basic snapshot diff (added/removed devices), power contract difference, link speed difference, cable warning difference, empty diff on identical snapshots.
- **T2_R4**: Diffing completely non-overlapping topologies, diffing with empty previous snapshot, diffing with empty current snapshot, partner unplugged while port remains, multiple simultaneous changes.

### R5. CLI Output Modes & Device Filtering
- **T1_R5**: Tree mode output formatting, list mode flat rendering, category filtering, device class filtering, vendor ID filtering.
- **T2_R5**: Filter with zero matching devices, filter with all matching devices, tree rendering with deep hub chains, raw attributes with missing files, summary counts on empty tree.

### R6. USB4 / Thunderbolt / Alternate-Mode Expansion
- **T1_R6**: USB4 host router identification (`generation >= 4`), USB4 device router identification, active USB4 link verification, Thunderbolt 3 router identification, daisy-chained router hierarchy (`0-0` -> `0-1` -> `0-1-1`).
- **T2_R6**: Missing `/sys/bus/thunderbolt` directory (honest absence), router without generation attribute, router generation 0, Billboard descriptor with unsuccessful alt-mode state, DP altmode without companion USB device.

### R7. Cable Authenticity & Trust Diagnostics
- **T1_R7**: Clean cable with valid vendor ID and VDO, zero VID detection (`vendor_id == 0`), unknown vendor hex fallback, dirty/reserved bits in Cable VDO, 5A contract with 3A cable mismatch.
- **T2_R7**: Passive cable without e-marker, 5A contract with unpopulated e-marker (UCSI limitation), cable VDO with all reserved bits set, EPR wattage on SPR cable, active cable advertising wrong plug type.

---

## 5. Execution Guide

Run the full E2E test suite:
```bash
cargo test --test e2e_suite
```

Run specific tiers:
```bash
cargo test --test e2e_suite tier1_features
cargo test --test e2e_suite tier2_boundaries
cargo test --test e2e_suite tier3_combinations
cargo test --test e2e_suite tier4_scenarios
```

Run specific requirement areas:
```bash
cargo test --test e2e_suite r1_json
cargo test --test e2e_suite r2_diagnostic
cargo test --test e2e_suite r3_history
cargo test --test e2e_suite r4_compare
cargo test --test e2e_suite r5_filter
cargo test --test e2e_suite r6_usb4
cargo test --test e2e_suite r7_trust
```

Run entire crate verification:
```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```
